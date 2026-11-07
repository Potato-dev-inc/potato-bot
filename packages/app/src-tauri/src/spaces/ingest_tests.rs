use super::identity::SpaceIdentity;
use super::ids::{BotId, ChannelId, EntryId, SpaceId};
use super::ingest::*;
use super::log::*;
use super::roster::{epoch_info, new_bot_claim, OpBody, Roster};
use super::store::SpaceStore;

const NOW: u64 = 10_000_000_000;
const KEY: [u8; 32] = [9; 32];
const DAY: u64 = 86_400_000;
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

fn hdr(who: &SpaceIdentity, kind: u8, seq: u64, lamport: u64) -> EntryHeader {
    EntryHeader {
        space: space(), channel: CH, device: who.device_key(), seq, lamport, created_ms: NOW, epoch: 1, kind,
        refers: None, bot: None,
    }
}

fn text(who: &SpaceIdentity, seq: u64, lamport: u64, t: &str) -> Entry {
    Entry::seal(who, hdr(who, KIND_TEXT, seq, lamport), &Body::Text { text: t.into(), mentions: vec![] }, &KEY).unwrap()
}

fn delete(who: &SpaceIdentity, seq: u64, lamport: u64, target: EntryId) -> Entry {
    let mut h = hdr(who, KIND_DELETE, seq, lamport);
    h.refers = Some(target);
    Entry::seal(who, h, &Body::Delete, &KEY).unwrap()
}

fn react(who: &SpaceIdentity, seq: u64, lamport: u64, target: EntryId, on: bool) -> Entry {
    let mut h = hdr(who, KIND_REACT, seq, lamport);
    h.refers = Some(target);
    Entry::seal(who, h, &Body::React { code: "heart".into(), on }, &KEY).unwrap()
}

fn put(s: &SpaceStore, r: &Roster, k: &EpochKeys, e: &Entry) -> Ingest { ingest_entry(s, r, k, &e.to_bytes(), NOW) }

fn page(s: &SpaceStore) -> Vec<super::store::MessageRow> { s.messages_page(&CH, None, 50).unwrap() }

#[test]
fn text_is_stored_and_shown() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    let e = text(&sam(), 1, 1, "hi");
    assert_eq!(put(&s, &r, &keys(), &e), Ingest::Stored { id: e.id(), shown: true });
    let rows = page(&s);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].text, "hi");
    assert_eq!(rows[0].member, sam().member_key());
}

#[test]
fn same_bytes_twice_is_duplicate() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    let e = text(&sam(), 1, 1, "hi");
    put(&s, &r, &keys(), &e);
    assert_eq!(put(&s, &r, &keys(), &e), Ingest::Duplicate);
}

#[test]
fn seq_reuse_is_rejected() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    put(&s, &r, &keys(), &text(&sam(), 1, 1, "one"));
    assert_eq!(put(&s, &r, &keys(), &text(&sam(), 1, 2, "two")), Ingest::Rejected("seq reused"));
}

#[test]
fn heads_and_missing() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    for seq in [1, 2, 5] { put(&s, &r, &keys(), &text(&sam(), seq, seq, "x")); }
    assert_eq!(s.heads().unwrap(), vec![(sam().device_key(), 1, 5)]);
    assert_eq!(s.missing(&sam().device_key(), 1, 5, 10).unwrap(), vec![3, 4]);
    assert_eq!(s.missing(&sam().device_key(), 1, 5, 1).unwrap(), vec![3]);
    assert_eq!(s.next_seq(&sam().device_key()).unwrap(), 6);
}

#[test]
fn missing_skips_below_floor() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    for seq in [1, 2, 5] { put(&s, &r, &keys(), &text(&sam(), seq, seq, "x")); }
    s.raise_floor(&sam().device_key(), 3).unwrap();
    assert_eq!(s.missing(&sam().device_key(), 1, 5, 10).unwrap(), vec![4]);
}

#[test]
fn old_entry_raises_floor() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    let mut h = hdr(&sam(), KIND_TEXT, 7, 1);
    h.created_ms = NOW - 91 * DAY;
    let e = Entry::seal(&sam(), h, &Body::Text { text: "old".into(), mentions: vec![] }, &KEY).unwrap();
    assert_eq!(put(&s, &r, &keys(), &e), Ingest::Old);
    assert_eq!(s.author_floor(&sam().device_key()).unwrap(), 7);
    assert!(page(&s).is_empty());
}

#[test]
fn delete_by_author_hides_and_drops_bytes() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    let t = text(&sam(), 1, 1, "secret");
    put(&s, &r, &keys(), &t);
    put(&s, &r, &keys(), &delete(&sam(), 2, 2, t.id()));
    let m = s.message(&t.id()).unwrap().unwrap();
    assert_eq!(m.deleted_by, Some(sam().member_key()));
    assert_eq!(m.text, "");
    let (have, gone) = s.entries_for(&sam().device_key(), &[1]).unwrap();
    assert!(have.is_empty());
    assert_eq!(gone, vec![1]);
    assert!(s.is_gone(&sam().device_key(), 1).unwrap());
}

#[test]
fn delete_by_other_member_is_ignored() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    let t = text(&owner(), 1, 1, "mine");
    put(&s, &r, &keys(), &t);
    put(&s, &r, &keys(), &delete(&sam(), 1, 2, t.id()));
    let m = s.message(&t.id()).unwrap().unwrap();
    assert_eq!(m.deleted_by, None);
    assert_eq!(m.text, "mine");
}

#[test]
fn delete_by_owner_works_on_anyone() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    let t = text(&sam(), 1, 1, "oops");
    put(&s, &r, &keys(), &t);
    put(&s, &r, &keys(), &delete(&owner(), 1, 2, t.id()));
    assert_eq!(s.message(&t.id()).unwrap().unwrap().deleted_by, Some(owner().member_key()));
}

#[test]
fn delete_before_target_applies_when_target_arrives() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    let t = text(&sam(), 1, 1, "late");
    put(&s, &r, &keys(), &delete(&sam(), 2, 2, t.id()));
    put(&s, &r, &keys(), &t);
    let m = s.message(&t.id()).unwrap().unwrap();
    assert_eq!(m.deleted_by, Some(sam().member_key()));
    assert_eq!(m.text, "");
}

#[test]
fn reaction_last_lamport_wins() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    let t = text(&owner(), 1, 1, "x");
    put(&s, &r, &keys(), &t);
    put(&s, &r, &keys(), &react(&sam(), 1, 5, t.id(), true));
    put(&s, &r, &keys(), &react(&sam(), 2, 4, t.id(), false));
    let got = s.reactions_for(&[t.id()]).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].1.as_str(), got[0].2), ("heart", 1));
    assert_eq!(got[0].3, vec![sam().member_key()]);
}

#[test]
fn entry_for_unknown_epoch_is_stored_but_not_shown() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    let e = text(&sam(), 1, 1, "hi");
    assert_eq!(put(&s, &r, &EpochKeys::default(), &e), Ingest::Stored { id: e.id(), shown: false });
    assert!(page(&s).is_empty());
    assert!(s.has_entry(&e.id()).unwrap());
}

#[test]
fn entries_for_returns_gone_list() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    let t1 = text(&sam(), 1, 1, "a");
    let t2 = text(&sam(), 2, 2, "b");
    put(&s, &r, &keys(), &t1);
    put(&s, &r, &keys(), &t2);
    put(&s, &r, &keys(), &delete(&sam(), 3, 3, t1.id()));
    let (have, gone) = s.entries_for(&sam().device_key(), &[1, 2]).unwrap();
    assert_eq!(have, vec![t2.to_bytes()]);
    assert_eq!(gone, vec![1]);
}

#[test]
fn real_entry_replaces_gone_placeholder() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    s.insert_gone_placeholder(&sam().device_key(), 3).unwrap();
    assert!(s.is_gone(&sam().device_key(), 3).unwrap());
    let (have, gone) = s.entries_for(&sam().device_key(), &[3]).unwrap();
    assert!(have.is_empty());
    assert_eq!(gone, vec![3]);
    let e = text(&sam(), 3, 1, "back");
    assert!(matches!(put(&s, &r, &keys(), &e), Ingest::Stored { .. }));
    assert!(!s.is_gone(&sam().device_key(), 3).unwrap());
}

#[test]
fn lamport_advances() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    put(&s, &r, &keys(), &text(&sam(), 1, 50, "x"));
    assert_eq!(s.next_lamport().unwrap(), 51);
}

#[test]
fn unread_counts_others_only() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    put(&s, &r, &keys(), &text(&sam(), 1, 1, "a"));
    put(&s, &r, &keys(), &text(&sam(), 2, 2, "b"));
    put(&s, &r, &keys(), &text(&owner(), 1, 3, "mine"));
    s.mark_read(&CH, 1).unwrap();
    assert_eq!(s.unread(&CH, &owner().member_key()).unwrap(), 1);
}

#[test]
fn bot_text_is_stored_under_its_owner_with_the_bot_id() {
    let (s, mut r) = (SpaceStore::open_memory().unwrap(), roster());
    let bot = BotId([5; 8]);
    let op = r.next_op(&owner(), new_bot_claim(&sam(), &space(), bot, "Fishy"), NOW).unwrap();
    r.apply(&op).unwrap();
    let mut h = hdr(&sam(), KIND_BOT, 1, 1);
    h.bot = Some(bot);
    let body = Body::BotText { text: "hello".into(), called_by: None, hops: 0 };
    let e = Entry::seal(&sam(), h, &body, &KEY).unwrap();
    assert_eq!(put(&s, &r, &keys(), &e), Ingest::Stored { id: e.id(), shown: true });
    let m = s.message(&e.id()).unwrap().unwrap();
    assert_eq!((m.bot, m.member, m.kind), (Some(bot), sam().member_key(), KIND_BOT));
    // a bot that is not on the roster is refused
    let mut h2 = hdr(&sam(), KIND_BOT, 2, 2);
    h2.bot = Some(BotId([6; 8]));
    let e2 = Entry::seal(&sam(), h2, &body, &KEY).unwrap();
    assert_eq!(put(&s, &r, &keys(), &e2), Ingest::Rejected("bot not on the roster"));
}

#[test]
fn a_failed_ingest_leaves_nothing_behind() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    let wrong = {
        let mut k = EpochKeys::default();
        k.0.insert(1, [1; 32]);
        k
    };
    let e = text(&sam(), 1, 1, "hi");
    assert_eq!(put(&s, &r, &wrong, &e), Ingest::Rejected("cannot open"));
    assert!(!s.has_entry(&e.id()).unwrap());
    assert!(s.heads().unwrap().is_empty());
}

#[test]
fn a_lamport_far_ahead_of_the_clock_is_rejected() {
    let (s, r) = (SpaceStore::open_memory().unwrap(), roster());
    let far = text(&sam(), 1, 1u64 << 62, "far");
    assert_eq!(put(&s, &r, &keys(), &far), Ingest::Rejected("lamport too far ahead"));
    assert_eq!(s.next_lamport().unwrap(), 1, "the clock did not move");
    let near = text(&sam(), 1, 1_000_000, "near");
    assert!(matches!(put(&s, &r, &keys(), &near), Ingest::Stored { .. }));
}
