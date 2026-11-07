use std::cell::RefCell;

use rusqlite::params;

use super::identity::SpaceIdentity;
use super::ids::{ChannelId, SpaceId};
use super::ingest::{ingest_entry, EpochKeys, Ingest};
use super::limits::{AUTHOR_KEEP_BYTES, KEEP_BYTES, KEEP_MS, TRIM_TO_BYTES};
use super::log::*;
use super::retention::*;
use super::roster::{epoch_info, OpBody, Roster};
use super::store::SpaceStore;

const NOW: u64 = 10_000_000_000;
const DAY: u64 = 86_400_000;
const MB: i64 = 1024 * 1024;
const KEY: [u8; 32] = [9; 32];
const CH: ChannelId = ChannelId([1; 8]);

fn owner() -> SpaceIdentity { SpaceIdentity::for_test(1) }
fn sam() -> SpaceIdentity { SpaceIdentity::for_test(2) }
fn space() -> SpaceId { SpaceId([7; 16]) }

fn roster() -> Roster {
    let (mut r, _) = Roster::genesis(&owner(), space(), "Club", "Owner", CH, &KEY, NOW).unwrap();
    let cert = sam().certify(&space(), "Sam", NOW);
    let grant = crate::secure_core::grant::seal_key_to(cert.device.0, &cert.kem_b64, &epoch_info(&space(), 1), &KEY).unwrap();
    let op = r.next_op(&owner(), OpBody::AddMember { cert, epoch: 1, grant }, NOW).unwrap();
    r.apply(&op).unwrap();
    r
}

fn keys() -> EpochKeys {
    let mut k = EpochKeys::default();
    k.0.insert(1, KEY);
    k
}

fn entry(seq: u64, created_ms: u64) -> Entry {
    let who = sam();
    let h = EntryHeader {
        space: space(), channel: CH, device: who.device_key(), seq, lamport: seq, created_ms, epoch: 1, kind: KIND_TEXT,
        refers: None, bot: None,
    };
    Entry::seal(&who, h, &Body::Text { text: format!("m{seq}"), mentions: vec![] }, &KEY).unwrap()
}

/// Stored past the receive checks, the way an entry that was fine when it arrived becomes old later.
fn put_raw(s: &SpaceStore, e: &Entry) {
    s.insert_entry(e, &e.id(), &e.to_bytes(), NOW).unwrap();
}

fn put_message_row(s: &SpaceStore, e: &Entry) {
    s.conn
        .execute(
            "INSERT INTO messages (id, channel, member, device, bot, lamport, created_ms, kind, text, reply_to, deleted_by, \
             deleted_ms, file_json) VALUES (?1, ?2, x'00', ?3, NULL, 1, ?4, 1, 'hello', NULL, NULL, NULL, NULL)",
            params![e.id().0.to_vec(), CH.0.to_vec(), e.header.device.0.to_vec(), e.header.created_ms as i64],
        )
        .unwrap();
}

fn count(s: &SpaceStore, table: &str) -> i64 {
    s.conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
}

/// A row with a made-up size and no bytes: the size rules count the `size` column.
fn sized_row(s: &SpaceStore, seq: u64, created_ms: u64, size: i64) {
    let device = sam().device_key();
    let mut id = [0u8; 32];
    id[..8].copy_from_slice(&seq.to_be_bytes());
    s.conn
        .execute(
            "INSERT INTO entries (id, device, seq, channel, lamport, created_ms, received_ms, kind, ref_id, epoch, bytes, size, gone) \
             VALUES (?1, ?2, ?3, ?4, ?3, ?5, 0, 1, NULL, 1, x'', ?6, 0)",
            params![id.to_vec(), device.0.to_vec(), seq as i64, CH.0.to_vec(), created_ms as i64, size],
        )
        .unwrap();
    s.conn
        .execute(
            "INSERT INTO authors (device, floor, high) VALUES (?1, 0, ?2) ON CONFLICT(device) DO UPDATE SET high = MAX(high, excluded.high)",
            params![device.0.to_vec(), seq as i64],
        )
        .unwrap();
}

/// Like `sized_row`, for any author.
fn sized_row_by(s: &SpaceStore, who: &SpaceIdentity, seq: u64, created_ms: u64, size: i64) {
    let device = who.device_key();
    let mut id = [0u8; 32];
    id[..8].copy_from_slice(&seq.to_be_bytes());
    id[8..16].copy_from_slice(&device.0[..8]);
    s.conn
        .execute(
            "INSERT INTO entries (id, device, seq, channel, lamport, created_ms, received_ms, kind, ref_id, epoch, bytes, size, gone)              VALUES (?1, ?2, ?3, ?4, ?3, ?5, 0, 1, NULL, 1, x'', ?6, 0)",
            params![id.to_vec(), device.0.to_vec(), seq as i64, CH.0.to_vec(), created_ms as i64, size],
        )
        .unwrap();
    s.conn
        .execute(
            "INSERT INTO authors (device, floor, high) VALUES (?1, 0, ?2) ON CONFLICT(device) DO UPDATE SET high = MAX(high, excluded.high)",
            params![device.0.to_vec(), seq as i64],
        )
        .unwrap();
}

fn rows_of(s: &SpaceStore, who: &SpaceIdentity) -> i64 {
    s.conn
        .query_row("SELECT COUNT(*) FROM entries WHERE device = ?1", params![who.device_key().0.to_vec()], |r| r.get(0))
        .unwrap()
}

fn cached_file(s: &SpaceStore, n: u8, size: i64) {
    s.conn
        .execute(
            "INSERT INTO files (sha, name, size, have, offer, cached_ms) VALUES (?1, 'f', ?2, 1, ?3, ?4)",
            params![vec![n; 32], size, vec![n; 32], n as i64],
        )
        .unwrap();
}

fn nothing(_: &[u8; 32]) {}

#[test]
fn entries_older_than_90_days_go() {
    let s = SpaceStore::open_memory().unwrap();
    for seq in 1..=3 { put_raw(&s, &entry(seq, NOW - 91 * DAY)); }
    put_raw(&s, &entry(4, NOW - DAY));
    let r = prune(&s, NOW, &nothing).unwrap();
    assert_eq!(r.old_entries, 3);
    assert_eq!(count(&s, "entries"), 1);
    let sam = sam().device_key();
    assert_eq!(s.author_floor(&sam).unwrap(), 3);
    assert_eq!(s.missing(&sam, 1, 4, 10).unwrap(), Vec::<u64>::new());
}

#[test]
fn messages_cascade_with_entries() {
    let s = SpaceStore::open_memory().unwrap();
    let old = entry(1, NOW - 91 * DAY);
    put_raw(&s, &old);
    put_message_row(&s, &old);
    assert_eq!(count(&s, "messages"), 1);
    prune(&s, NOW, &nothing).unwrap();
    assert_eq!(count(&s, "messages"), 0);
}

#[test]
fn size_rule_trims_to_180_mb() {
    let s = SpaceStore::open_memory().unwrap();
    // Five authors, 50 MB each: the biggest are cut to an equal share, each from its own oldest end.
    let who: Vec<SpaceIdentity> = (2..=6).map(SpaceIdentity::for_test).collect();
    for w in &who {
        for seq in 1..=50u64 { sized_row_by(&s, w, seq, NOW - DAY + seq, MB); }
    }
    assert!(used_bytes(&s).unwrap() > KEEP_BYTES);
    let r = prune(&s, NOW, &nothing).unwrap();
    assert!(used_bytes(&s).unwrap() <= TRIM_TO_BYTES);
    assert_eq!(r.size_entries, 70);
    assert_eq!(count(&s, "entries"), 180);
    for w in &who {
        assert_eq!(rows_of(&s, w), 36);
        assert_eq!(s.author_floor(&w.device_key()).unwrap(), 14);
    }
    assert_eq!(r.bytes_after, used_bytes(&s).unwrap());
}

/// Review finding: ratelimit counts by the author's own created_ms, so one member could post ~230 MB of
/// backdated entries in a day. Oldest-first eviction then threw out everyone else's older messages.
#[test]
fn one_member_cannot_push_out_everyone_elses_history() {
    let s = SpaceStore::open_memory().unwrap();
    let flooder = sam();
    let quiet = SpaceIdentity::for_test(3);
    for seq in 1..=20u64 { sized_row_by(&s, &quiet, seq, NOW - 60 * DAY + seq, MB / 10); }
    // 230 MB, all stamped older than the quiet member's messages.
    for seq in 1..=230u64 { sized_row_by(&s, &flooder, seq, NOW - 80 * DAY + seq, MB); }
    assert!(used_bytes(&s).unwrap() > KEEP_BYTES);
    prune(&s, NOW, &nothing).unwrap();
    assert_eq!(rows_of(&s, &quiet), 20, "other members' history must survive");
    assert_eq!(s.author_floor(&quiet.device_key()).unwrap(), 0);
    assert!(rows_of(&s, &flooder) as u64 * MB as u64 <= AUTHOR_KEEP_BYTES);
    // The flooder loses their own oldest first.
    let lowest: i64 = s
        .conn
        .query_row("SELECT MIN(seq) FROM entries WHERE device = ?1", params![flooder.device_key().0.to_vec()], |r| r.get(0))
        .unwrap();
    assert!(lowest > 100);
}

#[test]
fn tombstones_and_placeholders_survive_eviction() {
    let s = SpaceStore::open_memory().unwrap();
    let flooder = sam();
    for seq in 1..=230u64 { sized_row_by(&s, &flooder, seq, NOW - DAY + seq, MB); }
    // A delete entry (kind 4) and a gone placeholder, both from the flooder.
    s.conn.execute("UPDATE entries SET kind = 4, size = 100 WHERE seq = 3", []).unwrap();
    s.insert_gone_placeholder(&flooder.device_key(), 231).unwrap();
    prune(&s, NOW, &nothing).unwrap();
    let kept: i64 = s.conn.query_row("SELECT COUNT(*) FROM entries WHERE seq = 3 AND kind = 4", [], |r| r.get(0)).unwrap();
    assert_eq!(kept, 1);
    assert!(s.is_gone(&flooder.device_key(), 231).unwrap());
}

#[test]
fn files_go_before_messages() {
    let s = SpaceStore::open_memory().unwrap();
    for seq in 1..=150u64 { sized_row(&s, seq, NOW - DAY + seq, MB); }
    for n in 1..=6u8 { cached_file(&s, n, 10 * MB); }
    assert!(used_bytes(&s).unwrap() > KEEP_BYTES);
    let gone: RefCell<Vec<[u8; 32]>> = RefCell::new(Vec::new());
    let r = prune(&s, NOW, &|sha| gone.borrow_mut().push(*sha)).unwrap();
    assert!(used_bytes(&s).unwrap() <= TRIM_TO_BYTES);
    assert_eq!(count(&s, "entries"), 150);
    assert_eq!(r.size_entries, 0);
    assert_eq!(r.files, 3);
    // Oldest cached first.
    assert_eq!(*gone.borrow(), vec![[1u8; 32], [2u8; 32], [3u8; 32]]);
    let still: i64 = s.conn.query_row("SELECT COUNT(*) FROM files WHERE have = 1", [], |r| r.get(0)).unwrap();
    assert_eq!(still, 3);
}

#[test]
fn nothing_happens_under_limits() {
    let s = SpaceStore::open_memory().unwrap();
    let (r, k) = (roster(), keys());
    for seq in 1..=10 {
        let e = entry(seq, NOW - DAY);
        assert!(matches!(ingest_entry(&s, &r, &k, &e.to_bytes(), NOW), Ingest::Stored { .. }));
    }
    let report = prune(&s, NOW, &nothing).unwrap();
    assert_eq!((report.old_entries, report.size_entries, report.files), (0, 0, 0));
    assert_eq!(count(&s, "entries"), 10);
    assert_eq!(report.bytes_after, used_bytes(&s).unwrap());
}

#[test]
fn pruned_entry_is_not_reingested() {
    let s = SpaceStore::open_memory().unwrap();
    let (r, k) = (roster(), keys());
    // Valid when it arrived, 89 days old; time passes; it is now over 90 days old.
    let e = entry(1, NOW - 89 * DAY);
    assert!(matches!(ingest_entry(&s, &r, &k, &e.to_bytes(), NOW), Ingest::Stored { .. }));
    let later = NOW + 2 * DAY;
    assert_eq!(prune(&s, later, &nothing).unwrap().old_entries, 1);
    assert_eq!(count(&s, "entries"), 0);
    assert_eq!(ingest_entry(&s, &r, &k, &e.to_bytes(), later), Ingest::Old);
    assert!(!s.has_seq(&sam().device_key(), 1).unwrap());
    assert!(KEEP_MS < 91 * DAY);
}

#[test]
fn reactions_to_deleted_targets_go() {
    let s = SpaceStore::open_memory().unwrap();
    let old = entry(1, NOW - 91 * DAY);
    let fresh = entry(2, NOW - DAY);
    put_raw(&s, &old);
    put_raw(&s, &fresh);
    for target in [&old, &fresh] {
        s.conn
            .execute(
                "INSERT INTO reactions (target, member, emoji, is_on, lamport, entry) VALUES (?1, x'01', 'heart', 1, 1, x'02')",
                params![target.id().0.to_vec()],
            )
            .unwrap();
    }
    assert_eq!(count(&s, "reactions"), 2);
    prune(&s, NOW, &nothing).unwrap();
    assert_eq!(count(&s, "reactions"), 1);
}
