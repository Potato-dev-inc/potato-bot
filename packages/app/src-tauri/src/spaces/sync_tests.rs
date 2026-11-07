use std::collections::VecDeque;

use super::frame::Frame;
use super::identity::SpaceIdentity;
use super::ids::{ChannelId, DeviceKey, NodeKey, SpaceId};
use super::ingest::{ingest_entry, EpochKeys, Ingest};
use super::log::*;
use super::net::PeerHint;
use super::roster::{epoch_info, ControlOp, OpBody, Roster};
use super::store::SpaceStore;
use super::sync::*;

const NOW: u64 = 10_000_000_000;
const KEY: [u8; 32] = [9; 32];
const CH: ChannelId = ChannelId([1; 8]);

fn owner() -> SpaceIdentity { SpaceIdentity::for_test(1) }
fn sam() -> SpaceIdentity { SpaceIdentity::for_test(2) }
fn space() -> SpaceId { SpaceId([7; 16]) }

fn keys() -> EpochKeys {
    let mut k = EpochKeys::default();
    k.0.insert(1, KEY);
    k
}

/// Control log of `n` ops (n >= 2): genesis, Sam joins, then channels.
fn ops(space: SpaceId, n: usize, tag: u8) -> Vec<ControlOp> {
    let (mut r, g) = Roster::genesis(&owner(), space, "Club", "Owner", CH, &KEY, NOW).unwrap();
    let mut out = vec![g];
    let cert = sam().certify(&space, "Sam", NOW);
    let grant = crate::secure_core::grant::seal_key_to(cert.device.0, &cert.kem_b64, &epoch_info(&space, 1), &KEY).unwrap();
    let mut bodies = vec![OpBody::AddMember { cert, epoch: 1, grant }];
    for i in 0..n.saturating_sub(2) {
        bodies.push(OpBody::AddChannel { channel: ChannelId([tag.wrapping_add(i as u8 + 10); 8]), name: format!("room{tag}{i}") });
    }
    for b in bodies.into_iter().take(n - 1) {
        let op = r.next_op(&owner(), b, NOW).unwrap();
        r.apply(&op).unwrap();
        out.push(op);
    }
    out
}

/// One control log for the whole test run: two builds of it differ (the key grant is sealed with fresh randomness).
fn base_ops() -> &'static Vec<ControlOp> {
    static OPS: std::sync::OnceLock<Vec<ControlOp>> = std::sync::OnceLock::new();
    OPS.get_or_init(|| ops(space(), 4, 0))
}

struct FakeView {
    store: SpaceStore,
    roster: Roster,
    hint: PeerHint,
    extra_heads: Vec<(DeviceKey, u64, u64)>,
}

impl FakeView {
    fn new(space: SpaceId, ops: &[ControlOp]) -> Self {
        let store = SpaceStore::open_memory().unwrap();
        let mut roster = Roster::replay(space, &ops[..1]).unwrap();
        store.control_append(&ops[0]).unwrap();
        for op in &ops[1..] {
            roster.apply(op).unwrap();
            store.control_append(op).unwrap();
        }
        Self { store, roster, hint: PeerHint { node: NodeKey([5; 32]), relay: String::new(), lan: vec![] }, extra_heads: vec![] }
    }
    fn standard() -> Self { Self::new(space(), &base_ops()[..2]) }
    fn put(&self, e: &Entry) -> Ingest { ingest_entry(&self.store, &self.roster, &keys(), &e.to_bytes(), NOW) }
    fn count(&self) -> i64 { self.store.conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0)).unwrap() }
}

impl SyncView for FakeView {
    fn space(&self) -> SpaceId { self.roster.space }
    fn control_seq(&self) -> u64 { self.roster.seq }
    fn control_head(&self) -> [u8; 32] { self.roster.head }
    fn control_hash_at(&self, seq: u64) -> Option<[u8; 32]> { self.store.control_hash_at(seq).unwrap() }
    fn control_ops_after(&self, seq: u64) -> Vec<ControlOp> { self.store.control_ops_after(seq).unwrap() }
    fn heads(&self) -> Vec<(DeviceKey, u64, u64)> {
        let mut h = self.store.heads().unwrap();
        h.extend(self.extra_heads.clone());
        h
    }
    fn floor(&self, d: &DeviceKey) -> u64 { self.store.author_floor(d).unwrap() }
    fn knows_device(&self, d: &DeviceKey) -> bool { self.roster.author(d).is_some() }
    fn missing(&self, d: &DeviceKey, from: u64, to: u64, max: usize) -> Vec<u64> { self.store.missing(d, from, to, max).unwrap() }
    fn entries_for(&self, d: &DeviceKey, seqs: &[u64]) -> (Vec<Vec<u8>>, Vec<u64>) { self.store.entries_for(d, seqs).unwrap() }
    fn my_hint(&self) -> PeerHint { self.hint.clone() }
}

fn text(who: &SpaceIdentity, seq: u64, t: &str) -> Entry {
    let h = EntryHeader {
        space: space(), channel: CH, device: who.device_key(), seq, lamport: seq, created_ms: NOW - 10_000_000 + seq * 7_000, epoch: 1, kind: KIND_TEXT,
        refers: None, bot: None,
    };
    Entry::seal(who, h, &Body::Text { text: t.into(), mentions: vec![] }, &KEY).unwrap()
}

fn fill(v: &FakeView, who: &SpaceIdentity, from: u64, to: u64) {
    for s in from..=to {
        assert!(matches!(v.put(&text(who, s, &format!("m{s}"))), Ingest::Stored { .. }));
    }
}

struct Peer {
    view: FakeView,
    sess: SyncSession,
    inbox: VecDeque<Frame>,
    sent: Vec<(u8, Vec<u8>)>,
    outs: Vec<Out>,
    closed: Option<u8>,
}

impl Peer {
    fn new(view: FakeView, other: u8) -> Self {
        Self { view, sess: SyncSession::new(NodeKey([other; 32])), inbox: VecDeque::new(), sent: vec![], outs: vec![], closed: None }
    }
}

/// Executes outputs the way the service does. Frames to send go to `to_peer`.
fn exec(p: &mut Peer, outs: Vec<Out>, to_peer: &mut Vec<Frame>) {
    for o in outs {
        p.outs.push(o.clone());
        match o {
            Out::Send(k, b) => {
                p.sent.push((k, b.clone()));
                to_peer.push(Frame { kind: k, body: b });
            }
            Out::Ingest(list) => {
                for b in list { ingest_entry(&p.view.store, &p.view.roster, &keys(), &b, NOW); }
            }
            Out::Gone(list) => {
                for (d, s) in list { p.view.store.insert_gone_placeholder(&d, s).unwrap(); }
            }
            Out::ApplyControl(ops) => {
                for op in ops {
                    if p.view.roster.apply(&op).is_err() { p.closed = Some(2); break; }
                    p.view.store.control_append(&op).unwrap();
                }
            }
            Out::Hint(_) => {}
            Out::Close(c) => p.closed = Some(c),
        }
    }
}

/// Wires two sessions back to back until nothing moves.
fn run(a: &mut Peer, b: &mut Peer) {
    let mut to_b = Vec::new();
    let mut to_a = Vec::new();
    let oa = a.sess.start(&a.view);
    exec(a, oa, &mut to_b);
    let ob = b.sess.start(&b.view);
    exec(b, ob, &mut to_a);
    for _ in 0..10_000 {
        a.inbox.extend(to_a.drain(..));
        b.inbox.extend(to_b.drain(..));
        if a.inbox.is_empty() && b.inbox.is_empty() { return; }
        while let Some(f) = a.inbox.pop_front() {
            if a.closed.is_some() { break; }
            let o = a.sess.on_frame(&a.view, &f);
            exec(a, o, &mut to_b);
            if matches!(f.kind, CONTROL_OPS | ENTRIES | PUSH) {
                let o = a.sess.next_round(&a.view);
                exec(a, o, &mut to_b);
            }
        }
        while let Some(f) = b.inbox.pop_front() {
            if b.closed.is_some() { break; }
            let o = b.sess.on_frame(&b.view, &f);
            exec(b, o, &mut to_a);
            if matches!(f.kind, CONTROL_OPS | ENTRIES | PUSH) {
                let o = b.sess.next_round(&b.view);
                exec(b, o, &mut to_a);
            }
        }
    }
    panic!("sync did not end");
}

fn wants(p: &Peer) -> Vec<Vec<(DeviceKey, Vec<u64>)>> {
    p.sent.iter().filter(|(k, _)| *k == WANT).map(|(_, b)| decode_want(b).unwrap()).collect()
}

fn hello_of(v: &FakeView) -> Frame { Frame { kind: HELLO, body: encode_hello(v) } }

#[test]
fn start_sends_hello_first() {
    let v = FakeView::standard();
    let mut s = SyncSession::new(NodeKey([2; 32]));
    let out = s.start(&v);
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], Out::Send(k, _) if *k == HELLO));
}

#[test]
fn frame_before_hello_closes() {
    let v = FakeView::standard();
    let mut s = SyncSession::new(NodeKey([2; 32]));
    let f = Frame { kind: WANT, body: encode_want(&[]) };
    assert_eq!(s.on_frame(&v, &f), vec![Out::Close(3)]);
}

#[test]
fn wrong_space_closes() {
    let v = FakeView::standard();
    let other = FakeView::new(SpaceId([8; 16]), &ops(SpaceId([8; 16]), 2, 0));
    let mut s = SyncSession::new(NodeKey([2; 32]));
    assert!(s.on_frame(&v, &hello_of(&other)).contains(&Out::Close(3)));
}

#[test]
fn equal_stores_need_nothing() {
    let (a, b) = (FakeView::standard(), FakeView::standard());
    fill(&a, &sam(), 1, 5);
    fill(&b, &sam(), 1, 5);
    let (mut a, mut b) = (Peer::new(a, 2), Peer::new(b, 1));
    run(&mut a, &mut b);
    assert!(wants(&a).is_empty() && wants(&b).is_empty());
    assert!(a.sess.is_synced() && b.sess.is_synced());
}

#[test]
fn b_gets_what_a_has() {
    let (a, b) = (FakeView::standard(), FakeView::standard());
    fill(&a, &sam(), 1, 10);
    let (mut a, mut b) = (Peer::new(a, 2), Peer::new(b, 1));
    run(&mut a, &mut b);
    let w = wants(&b);
    assert_eq!(w.len(), 1);
    assert_eq!(w[0][0].1, (1..=10).collect::<Vec<u64>>());
    assert_eq!(b.view.count(), 10);
    assert!(wants(&a).is_empty());
    assert!(a.sess.is_synced() && b.sess.is_synced());
}

#[test]
fn both_ways_at_once() {
    let (a, b) = (FakeView::standard(), FakeView::standard());
    fill(&a, &sam(), 1, 5);
    fill(&b, &owner(), 1, 3);
    let (mut a, mut b) = (Peer::new(a, 2), Peer::new(b, 1));
    run(&mut a, &mut b);
    assert_eq!(a.view.count(), 8);
    assert_eq!(b.view.count(), 8);
}

#[test]
fn want_is_capped_at_500() {
    let (a, b) = (FakeView::standard(), FakeView::standard());
    fill(&a, &sam(), 1, 1200);
    let (mut a, mut b) = (Peer::new(a, 2), Peer::new(b, 1));
    run(&mut a, &mut b);
    let w = wants(&b);
    assert_eq!(w.len(), 3);
    assert_eq!(w[0][0].1.len(), 500);
    assert_eq!(w[1][0].1.len(), 500);
    assert_eq!(w[2][0].1.len(), 200);
    assert_eq!(b.view.count(), 1200);
}

#[test]
fn entries_frames_stay_under_one_mib() {
    let entries: Vec<Vec<u8>> = (0..30).map(|i| vec![i as u8; 60 * 1024]).collect();
    let frames = encode_entries(&entries, &[(DeviceKey([3; 32]), 4)]);
    assert!(frames.len() >= 2);
    assert!(frames.iter().all(|f| f.len() < 1_048_576));
    let mut total = 0;
    for (i, f) in frames.iter().enumerate() {
        let (e, g, last) = decode_entries(f).unwrap();
        total += e.len();
        assert_eq!(last, i + 1 == frames.len());
        assert_eq!(g.len(), if last { 1 } else { 0 });
    }
    assert_eq!(total, 30);
}

#[test]
fn behind_control_log_gets_ops() {
    let full = base_ops().clone();
    let a = FakeView::new(space(), &full);
    let b = FakeView::new(space(), &full[..2]);
    let (mut a, mut b) = (Peer::new(a, 2), Peer::new(b, 1));
    run(&mut a, &mut b);
    let sent_ops: Vec<_> = a.sent.iter().filter(|(k, _)| *k == CONTROL_OPS).collect();
    assert_eq!(sent_ops.len(), 1);
    let applied: Vec<u64> = b
        .outs
        .iter()
        .filter_map(|o| if let Out::ApplyControl(ops) = o { Some(ops.iter().map(|o| o.seq).collect::<Vec<_>>()) } else { None })
        .flatten()
        .collect();
    assert_eq!(applied, vec![3, 4]);
    assert_eq!(b.view.roster.seq, 4);
    assert!(a.sess.is_synced() && b.sess.is_synced());
}

#[test]
fn forked_control_log_closes() {
    let a = FakeView::standard();
    // Same genesis and length, other second op.
    let g = base_ops()[0].clone();
    let r = Roster::replay(space(), &[g.clone()]).unwrap();
    let op2 = r.next_op(&owner(), OpBody::AddChannel { channel: ChannelId([77; 8]), name: "other".into() }, NOW).unwrap();
    let b = FakeView::new(space(), &[g, op2]);
    let (mut a, mut b) = (Peer::new(a, 2), Peer::new(b, 1));
    run(&mut a, &mut b);
    assert_eq!(a.closed, Some(2));
    assert_eq!(b.closed, Some(2));
}

#[test]
fn push_updates_their_head() {
    let (a, b) = (FakeView::standard(), FakeView::standard());
    fill(&a, &sam(), 1, 10);
    fill(&b, &sam(), 1, 10);
    let mut s = SyncSession::new(NodeKey([1; 32]));
    s.on_frame(&b, &hello_of(&a));
    assert_eq!(s.their_high(&sam().device_key()), Some(10));
    let e = text(&sam(), 11, "m11");
    let mut w = super::wire::Writer::new();
    w.blob(&e.to_bytes());
    let out = s.on_frame(&b, &Frame { kind: PUSH, body: w.finish() });
    assert_eq!(out, vec![Out::Ingest(vec![e.to_bytes()])]);
    assert_eq!(s.their_high(&sam().device_key()), Some(11));
}

#[test]
fn gone_seqs_are_reported() {
    let (a, b) = (FakeView::standard(), FakeView::standard());
    a.put(&text(&sam(), 1, "m1"));
    a.store.insert_gone_placeholder(&sam().device_key(), 2).unwrap();
    a.put(&text(&sam(), 3, "m3"));
    let (mut a, mut b) = (Peer::new(a, 2), Peer::new(b, 1));
    run(&mut a, &mut b);
    assert!(b.outs.contains(&Out::Gone(vec![(sam().device_key(), 2)])));
    assert!(b.view.store.is_gone(&sam().device_key(), 2).unwrap());
    assert_eq!(wants(&b).len(), 1);
    assert_eq!(b.view.count(), 3);
}

#[test]
fn want_over_500_is_rejected_by_decoder() {
    let body = encode_want(&[(DeviceKey([3; 32]), (1..=501).collect())]);
    assert!(decode_want(&body).is_err());
    let ok = encode_want(&[(DeviceKey([3; 32]), (1..=500).collect())]);
    assert!(decode_want(&ok).is_ok());
}

#[test]
fn unknown_device_is_not_asked() {
    let mut a = FakeView::standard();
    a.extra_heads.push((DeviceKey([99; 32]), 1, 50));
    let b = FakeView::standard();
    let (mut a, mut b) = (Peer::new(a, 2), Peer::new(b, 1));
    run(&mut a, &mut b);
    assert!(wants(&b).is_empty());
    assert!(b.sess.is_synced());
}

#[test]
fn hundred_unknown_frames_close() {
    let v = FakeView::standard();
    let mut s = SyncSession::new(NodeKey([2; 32]));
    s.on_frame(&v, &hello_of(&v));
    let f = Frame { kind: 0x40, body: vec![] };
    for _ in 0..100 {
        assert!(s.on_frame(&v, &f).is_empty());
    }
    assert_eq!(s.on_frame(&v, &f), vec![Out::Close(4)]);
}

fn entries_frame(entries: &[Vec<u8>], gone: &[(DeviceKey, u64)], last: bool) -> Frame {
    let mut w = super::wire::Writer::new();
    w.u16(entries.len() as u16);
    for e in entries { w.blob(e); }
    w.u16(gone.len() as u16);
    for (d, s) in gone { w.fixed(&d.0).u64(*s); }
    w.u8(last as u8);
    Frame { kind: ENTRIES, body: w.finish() }
}

#[test]
fn a_head_that_is_never_answered_is_dropped() {
    let mut liar = FakeView::standard();
    liar.extra_heads.push((owner().device_key(), 1, 1u64 << 62));
    let me = FakeView::standard();
    let mut s = SyncSession::new(NodeKey([1; 32]));
    let first = s.on_frame(&me, &hello_of(&liar));
    assert!(first.iter().any(|o| matches!(o, Out::Send(k, _) if *k == WANT)));
    // An empty, complete reply: the peer does not have what it claimed.
    let out = s.on_frame(&me, &entries_frame(&[], &[], true));
    assert!(out.is_empty());
    assert!(s.next_round(&me).is_empty(), "the same liar must not fill the budget again");
    assert_eq!(s.their_high(&owner().device_key()), None);
}

#[test]
fn gone_that_was_not_asked_is_ignored() {
    let me = FakeView::standard();
    let mut s = SyncSession::new(NodeKey([1; 32]));
    s.on_frame(&me, &hello_of(&FakeView::standard()));
    let out = s.on_frame(&me, &entries_frame(&[], &[(sam().device_key(), 1u64 << 40)], true));
    assert!(!out.iter().any(|o| matches!(o, Out::Gone(_))));
}

#[test]
fn gone_that_was_asked_is_kept() {
    let mut a = FakeView::standard();
    a.extra_heads.push((sam().device_key(), 1, 3));
    let me = FakeView::standard();
    let mut s = SyncSession::new(NodeKey([1; 32]));
    s.on_frame(&me, &hello_of(&a));
    let out = s.on_frame(&me, &entries_frame(&[], &[(sam().device_key(), 2)], false));
    assert!(out.contains(&Out::Gone(vec![(sam().device_key(), 2)])));
}

#[test]
fn a_hello_hint_for_another_node_is_not_kept() {
    let me = FakeView::standard();
    // The view's own hint names node [5; 32].
    let mut from_stranger = SyncSession::new(NodeKey([1; 32]));
    let out = from_stranger.on_frame(&me, &hello_of(&FakeView::standard()));
    assert!(!out.iter().any(|o| matches!(o, Out::Hint(_))), "{out:?}");
    let mut from_owner_of_hint = SyncSession::new(NodeKey([5; 32]));
    let out = from_owner_of_hint.on_frame(&me, &hello_of(&FakeView::standard()));
    assert!(out.iter().any(|o| matches!(o, Out::Hint(h) if h.node == NodeKey([5; 32]))));
}
