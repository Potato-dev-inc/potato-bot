//! Rate limits (D5.5): per-author entry limits decided from stored data, the per-stream frame meter, the lockout cap,
//! and the cowork M1-M5 lessons that need a running service.
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::frame::{read_frame, write_frame};
use super::identity::SpaceIdentity;
use super::ids::{ChannelId, NodeKey, SpaceId};
use super::ingest::{ingest_entry, EpochKeys, Ingest};
use super::join::Lockouts;
use super::log::{Body, Entry, EntryHeader, KIND_TEXT};
use super::memnet::MemoryHub;
use super::ratelimit::{LinkMeter, Meter};
use super::roster::{epoch_info, OpBody, Roster};
use super::service_tests::{clock, eventually, general, make, node, pair, say, texts, trio, Node};
use super::store::SpaceStore;
use super::sync;
use super::words::{self, SECRET_BYTES};

const NOW: u64 = 10_000_000_000;
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

/// Sam's text number `seq`, written at `created_ms` by Sam's own clock.
fn sam_text(seq: u64, created_ms: u64) -> Entry {
    let who = sam();
    let h = EntryHeader {
        space: space(), channel: CH, device: who.device_key(), seq, lamport: seq, created_ms, epoch: 1, kind: KIND_TEXT,
        refers: None, bot: None,
    };
    Entry::seal(&who, h, &Body::Text { text: format!("m{seq}"), mentions: vec![] }, &KEY).unwrap()
}

fn put(s: &SpaceStore, r: &Roster, e: &Entry) -> Ingest { ingest_entry(s, r, &keys(), &e.to_bytes(), NOW + 3_700_000) }

fn ten_in_ten_seconds() -> (SpaceStore, Roster) {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    for i in 0..10u64 {
        assert!(matches!(put(&s, &r, &sam_text(i + 1, NOW + i * 1000)), Ingest::Stored { .. }));
    }
    (s, r)
}

#[test]
fn eleventh_entry_in_ten_seconds_is_dropped() {
    let (s, r) = ten_in_ten_seconds();
    let eleventh = sam_text(11, NOW + 9_500);
    assert_eq!(put(&s, &r, &eleventh), Ingest::Rejected("too fast"));
    // Not stored, so the service has nothing to forward.
    assert!(!s.has_entry(&eleventh.id()).unwrap());
    assert!(!s.has_seq(&sam().device_key(), 11).unwrap());
}

#[test]
fn burst_window_slides() {
    let (s, r) = ten_in_ten_seconds();
    assert!(matches!(put(&s, &r, &sam_text(11, NOW + 10_100)), Ingest::Stored { .. }));
}

#[test]
fn six_hundred_first_in_an_hour_is_dropped() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    // 600 entries, one every 5.9 s: never more than 2 in any 10 s, 600 inside the hour.
    for i in 0..600u64 {
        assert!(matches!(put(&s, &r, &sam_text(i + 1, NOW + i * 5_900)), Ingest::Stored { .. }), "entry {i}");
    }
    assert_eq!(put(&s, &r, &sam_text(601, NOW + 600 * 5_900)), Ingest::Rejected("too fast"));
}

#[tokio::test]
async fn my_own_send_gets_the_plain_text() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    let hex = a.svc.create_space("Book club").await.unwrap();
    let ch = general(&a, &SpaceId::from_hex(&hex).unwrap()).to_hex();
    for i in 0..10 {
        a.svc.send_text(&hex, &ch, &format!("m{i}"), None).await.unwrap();
    }
    assert_eq!(
        a.svc.send_text(&hex, &ch, "m10", None).await.unwrap_err(),
        "You're sending messages too fast. Wait a few seconds."
    );
    // Ten seconds later the window has moved on.
    clk.fetch_add(10_100, Ordering::SeqCst);
    a.svc.send_text(&hex, &ch, "m10", None).await.unwrap();
}

#[test]
fn link_meter_closes_after_three_seconds_over_the_cap() {
    let mut m = LinkMeter::new();
    let base = 10_000u64;
    let mut closed_in = None;
    'outer: for sec in 0..6u64 {
        for i in 0..250u64 {
            if m.frame(base + sec * 1000 + i) == Meter::Close {
                closed_in = Some(sec + 1);
                break 'outer;
            }
        }
    }
    assert_eq!(closed_in, Some(4));
    // A calm stream is never closed.
    let mut calm = LinkMeter::new();
    for sec in 0..20u64 {
        for i in 0..150u64 {
            assert_eq!(calm.frame(base + sec * 1000 + i), Meter::Ok);
        }
    }
}

#[tokio::test]
async fn flooding_stream_is_closed() {
    let (_hub, clk, a, b, space) = pair().await;
    let t0 = clk.load(Ordering::SeqCst);
    let hello = b.svc.hello_bytes(&space).unwrap();
    let link = b.svc.link_to(&a.key).unwrap();
    let mut stream = link.open().await.unwrap();
    write_frame(&mut stream.send, sync::HELLO, &hello).await.unwrap();
    for sec in 0..4u64 {
        clk.store(t0 + sec * 1000, Ordering::SeqCst);
        for _ in 0..250 {
            // Once A has closed the stream, further writes fail: that is the point.
            let _ = write_frame(&mut stream.send, sync::PRESENCE, &[]).await;
        }
        // Let A read this second's frames before the clock moves on.
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    let bye = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let f = read_frame(&mut stream.recv).await.unwrap();
            if f.kind == sync::BYE { return f; }
        }
    })
    .await
    .unwrap();
    assert_eq!(bye.body, vec![4]);
}

#[test]
fn lockout_map_is_capped_at_256() {
    let mut l = Lockouts::new();
    for i in 0..300u32 {
        let mut k = [0u8; 32];
        k[..4].copy_from_slice(&i.to_be_bytes());
        l.strike(NodeKey(k), 1_000 + i as u64);
    }
    assert_eq!(l.len(), 256);
}

#[tokio::test]
async fn duplicate_push_is_not_forwarded() {
    let (hub, _clk, a, b, c, space) = trio().await;
    hub.cut(a.key, c.key, true);
    eventually("a and c are apart", || a.svc.link_count() == 1 && c.svc.link_count() == 1).await;
    say(&a, &space, "x").await;
    eventually("c gets x through b", || texts(&c, &space).contains(&"x".to_string())).await;
    let bytes: Vec<u8> = a
        .svc
        .with_open(&space, |sp| sp.store.conn.query_row("SELECT bytes FROM entries WHERE kind = 1", [], |r| r.get(0)).unwrap())
        .unwrap();
    let (b_pushes, a_pushes) = (b.svc.pushes_written(), a.svc.pushes_written());
    a.svc.forward(&space, None, &bytes);
    a.svc.forward(&space, None, &bytes);
    eventually("a wrote both pushes", || a.svc.pushes_written() >= a_pushes + 2).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(b.svc.pushes_written(), b_pushes);
}

fn strikes(n: &Node, space: &SpaceId) -> u32 {
    n.svc.with_open(space, |sp| sp.store.invites().unwrap().first().map(|i| i.strikes).unwrap_or(0)).unwrap()
}

#[tokio::test]
async fn strikes_survive_restart() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    let stranger = node(&hub, 2, &clk).await;
    let hex = a.svc.create_space("Book club").await.unwrap();
    let space = SpaceId::from_hex(&hex).unwrap();
    let invite = a.svc.create_invite(&hex).await.unwrap();
    let link = invite["link"].as_str().unwrap().to_string();
    let wrong = words::encode(&[0xABu8; SECRET_BYTES]).join(" ");
    for n in 1..=3u32 {
        stranger.svc.start_join(&link, &wrong).await.unwrap();
        eventually("a strike", || strikes(&a, &space) == n).await;
        // A wrong try pauses the invite for a while, whoever knocks next.
        clk.fetch_add(super::limits::INVITE_TRY_GAP_MS + 1, std::sync::atomic::Ordering::SeqCst);
    }
    // The owner restarts: the strikes are in its database, the in-memory lockout is gone.
    a.svc.stop().await;
    let Node { keystore, dir, .. } = a;
    let a2 = make(&hub, 1, &clk, Some(keystore), Some(dir));
    a2.svc.start_with(false).await.unwrap();
    assert_eq!(strikes(&a2, &space), 3);
    for n in 4..=5u32 {
        stranger.svc.start_join(&link, &wrong).await.unwrap();
        eventually("a strike", || strikes(&a2, &space) == n).await;
        clk.fetch_add(super::limits::INVITE_TRY_GAP_MS + 1, std::sync::atomic::Ordering::SeqCst);
    }
    assert_eq!(a2.svc.view()["invites"][0]["state"], "dead");
}

#[tokio::test]
async fn rate_limit_survives_restart() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    let hex = a.svc.create_space("Book club").await.unwrap();
    let space = SpaceId::from_hex(&hex).unwrap();
    let ch = general(&a, &space).to_hex();
    for i in 0..10 {
        a.svc.send_text(&hex, &ch, &format!("m{i}"), None).await.unwrap();
    }
    a.svc.stop().await;
    let Node { keystore, dir, .. } = a;
    let a2 = make(&hub, 1, &clk, Some(keystore), Some(dir));
    a2.svc.start_with(false).await.unwrap();
    assert_eq!(
        a2.svc.send_text(&hex, &ch, "m10", None).await.unwrap_err(),
        "You're sending messages too fast. Wait a few seconds."
    );
}

#[test]
fn catch_up_after_everything_was_pruned() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    for i in 1..=3u64 {
        assert!(matches!(put(&s, &r, &sam_text(i, NOW + i)), Ingest::Stored { .. }));
    }
    // Retention removed everything of Sam's and raised the floor to 40.
    let dev = sam().device_key();
    s.conn.execute("DELETE FROM entries WHERE device = ?1", rusqlite::params![dev.0.to_vec()]).unwrap();
    s.conn.execute("DELETE FROM messages", []).unwrap();
    s.raise_floor(&dev, 40).unwrap();
    let next = sam_text(41, NOW + 20_000);
    assert!(matches!(put(&s, &r, &next), Ingest::Stored { .. }));
    assert!(s.has_seq(&dev, 41).unwrap());
    // Nothing below the floor is asked for again.
    assert!(s.missing(&dev, 1, 41, 500).unwrap().is_empty());
}
