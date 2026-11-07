use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

use super::frame::read_frame;
use super::identity::{DeviceCert, SpaceIdentity};
use super::ids::{ChannelId, InviteId, NodeKey, SpaceId};
use super::invite::{new_invite, InviteRecord, InviteState, InviteTicket};
use super::join::*;
use super::limits::*;
use super::memnet::{MemNet, MemoryHub};
use super::net::{PeerHint, SpaceNet, SpacePermissions};
use super::roster::*;
use super::words::SECRET_BYTES;
use crate::secure_core::grant::seal_key_to;

const NOW: u64 = 1_000_000;
const KEY: [u8; 32] = [9; 32];

fn owner() -> SpaceIdentity { SpaceIdentity::for_test(1) }
fn space() -> SpaceId { SpaceId([7; 16]) }

fn genesis() -> (Roster, ControlOp) {
    Roster::genesis(&owner(), space(), "Club", "Owner", ChannelId([1; 8]), &KEY, NOW).unwrap()
}

enum Decision { LetIn, No, NoAnswer }

/// The owner's side for tests: an in-memory roster and invite, a decision the test sends.
struct ManualHost {
    roster: Mutex<Roster>,
    log: Mutex<Vec<ControlOp>>,
    invite: Mutex<InviteRecord>,
    secret: [u8; SECRET_BYTES],
    lockouts: Mutex<Lockouts>,
    certs: Mutex<HashMap<i64, DeviceCert>>,
    next: AtomicI64,
    decisions: AsyncMutex<mpsc::UnboundedReceiver<Decision>>,
}

struct Rig {
    host: Arc<ManualHost>,
    ticket: InviteTicket,
    secret: [u8; SECRET_BYTES],
    decide: mpsc::UnboundedSender<Decision>,
    hub: Arc<MemoryHub>,
    owner_net: Arc<MemNet>,
}

#[async_trait]
impl JoinHost for ManualHost {
    fn check(&self, req: &JoinRequestFrame, joiner: NodeKey, now_ms: u64) -> Verdict {
        let roster = self.roster.lock().unwrap();
        let mut invite = self.invite.lock().unwrap();
        let mut lockouts = self.lockouts.lock().unwrap();
        let res = owner_check(&roster, Some(&mut invite), Some(self.secret), &mut lockouts, req, joiner, now_ms);
        match res {
            Err((r, left)) => Verdict::Refuse(r, left),
            Ok(()) => {
                let id = self.next.fetch_add(1, Ordering::SeqCst);
                self.certs.lock().unwrap().insert(id, req.cert.clone());
                Verdict::Wait { request_id: id, answer_by_ms: now_ms + JOIN_WAIT_MS }
            }
        }
    }

    async fn wait_answer(&self, request_id: i64) -> Answer {
        let d = self.decisions.lock().await.recv().await;
        self.invite.lock().unwrap().answered();
        match d {
            Some(Decision::LetIn) => {
                let cert = self.certs.lock().unwrap().get(&request_id).cloned().unwrap();
                let mut roster = self.roster.lock().unwrap();
                let op = owner_let_in(&roster, &owner(), &cert, &KEY, NOW).unwrap();
                roster.apply(&op).unwrap();
                let mut log = self.log.lock().unwrap();
                log.push(op);
                Answer::LetIn { ops: log.clone(), hints: vec![PeerHint { node: NodeKey([1; 32]), relay: String::new(), lan: vec![] }] }
            }
            Some(Decision::No) => Answer::No,
            _ => Answer::NoAnswer,
        }
    }
}

/// An owner with one open invite, listening on a memory net. Joiners dial `ticket.owner_node`.
fn rig() -> Rig {
    let (roster, g) = genesis();
    let owner_key = owner().node();
    let n = new_invite(space(), owner_key, "", &[], "Club", NOW);
    let (tx, rx) = mpsc::unbounded_channel();
    let host = Arc::new(ManualHost {
        roster: Mutex::new(roster),
        log: Mutex::new(vec![g]),
        invite: Mutex::new(InviteRecord::new(n.ticket.invite, NOW)),
        secret: n.secret,
        lockouts: Mutex::new(Lockouts::new()),
        certs: Mutex::new(HashMap::new()),
        next: AtomicI64::new(1),
        decisions: AsyncMutex::new(rx),
    });
    let hub = MemoryHub::new();
    let perms = SpacePermissions::new();
    perms.set_open_invites(1);
    let owner_net = hub.node(owner_key, perms);
    let on = owner_net.clone();
    let h = host.clone();
    tokio::spawn(async move {
        while let Some(link) = on.accept().await {
            let h = h.clone();
            tokio::spawn(async move {
                let joiner = link.peer();
                let Ok(mut s) = link.accept_stream().await else { return };
                let Ok(first) = read_frame(&mut s.recv).await else { return };
                let _ = serve_join(s, joiner, first, h.as_ref(), NOW).await;
            });
        }
    });
    Rig { host, ticket: n.ticket, secret: n.secret, decide: tx, hub, owner_net }
}

/// A joiner whose memory-net key is its identity's node.
fn joiner(rig: &Rig, seed: u8) -> (SpaceIdentity, Arc<MemNet>) {
    let id = SpaceIdentity::for_test(seed);
    let net = rig.hub.node(id.node(), SpacePermissions::new());
    (id, net)
}

async fn join(rig: &Rig, id: &SpaceIdentity, net: &MemNet, secret: &[u8; SECRET_BYTES]) -> Result<JoinOutcome, JoinError> {
    run_joiner(net, &rig.ticket, secret, id, "Sam", NOW, None, &|_| {}).await
}

fn refused(r: Result<JoinOutcome, JoinError>) -> (RefuseReason, u8) {
    match r.unwrap() {
        JoinOutcome::Refused(a, b) => (a, b),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn wrong() -> [u8; SECRET_BYTES] { [1; SECRET_BYTES] }

#[tokio::test]
async fn right_words_and_let_in_joins() {
    let rig = rig();
    let (id, net) = joiner(&rig, 2);
    rig.decide.send(Decision::LetIn).unwrap();
    match join(&rig, &id, &net, &rig.secret).await.unwrap() {
        JoinOutcome::Joined { roster, epoch_key, hints, .. } => {
            assert_eq!(roster.members.len(), 2);
            assert_eq!(epoch_key, KEY);
            assert_eq!(roster.open_epoch_key(&id, roster.epoch), Some(KEY));
            assert_eq!(hints.len(), 1);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(rig.owner_net.me(), Some(rig.ticket.owner_node));
}

#[tokio::test]
async fn owner_says_no() {
    let rig = rig();
    let (id, net) = joiner(&rig, 2);
    rig.decide.send(Decision::No).unwrap();
    assert_eq!(refused(join(&rig, &id, &net, &rig.secret).await), (RefuseReason::No, 0));
    assert_eq!(rig.host.invite.lock().unwrap().state, InviteState::Used);
}

#[tokio::test]
async fn wrong_words_report_tries_left() {
    let rig = rig();
    let (id, net) = joiner(&rig, 2);
    assert_eq!(refused(join(&rig, &id, &net, &wrong()).await), (RefuseReason::WrongWords, 4));
}

/// A request from joiner `seed` with `proof`, run through the owner rules at `now`.
fn try_at(
    roster: &Roster, rec: &mut InviteRecord, secret: [u8; SECRET_BYTES], l: &mut Lockouts, seed: u8, right: bool, now: u64,
) -> Result<(), (RefuseReason, u8)> {
    let me = SpaceIdentity::for_test(seed);
    let cert = me.certify(&space(), "Sam", NOW);
    let proof = if right {
        super::invite::join_proof(&secret, &space(), &rec.id, &me.node(), &me.member_key())
    } else {
        [0u8; 32]
    };
    let req = JoinRequestFrame::new(space(), rec.id, cert, proof, None);
    owner_check(roster, Some(rec), Some(secret), l, &req, me.node(), now)
}

#[test]
fn fresh_node_keys_cannot_burn_an_invite_in_a_burst() {
    let (roster, _) = genesis();
    let n = new_invite(space(), owner().node(), "", &[], "Club", NOW);
    let mut rec = InviteRecord::new(n.ticket.invite, NOW);
    let mut l = Lockouts::new();
    let got: Vec<_> = (2..=9u8).map(|seed| try_at(&roster, &mut rec, n.secret, &mut l, seed, false, NOW)).collect();
    assert_eq!(got[0], Err((RefuseReason::WrongWords, 4)));
    assert!(got[1..].iter().all(|r| *r == Err((RefuseReason::Busy, 0))), "{got:?}");
    assert_eq!(rec.strikes, 1);
    assert_eq!(rec.state, InviteState::Open);
    // The real joiner, from another node, gets in once the pause is over.
    assert_eq!(try_at(&roster, &mut rec, n.secret, &mut l, 20, true, NOW + INVITE_TRY_GAP_MS), Ok(()));
}

#[test]
fn five_spaced_wrong_tries_still_kill_the_invite() {
    let (roster, _) = genesis();
    let n = new_invite(space(), owner().node(), "", &[], "Club", NOW);
    let mut rec = InviteRecord::new(n.ticket.invite, NOW);
    let mut l = Lockouts::new();
    let got: Vec<_> = (0..5u64)
        .map(|i| try_at(&roster, &mut rec, n.secret, &mut l, 2 + i as u8, false, NOW + i * INVITE_TRY_GAP_MS))
        .collect();
    assert_eq!(
        got,
        vec![
            Err((RefuseReason::WrongWords, 4)),
            Err((RefuseReason::WrongWords, 3)),
            Err((RefuseReason::WrongWords, 2)),
            Err((RefuseReason::WrongWords, 1)),
            Err((RefuseReason::Gone, 0)),
        ]
    );
    let late = NOW + 6 * INVITE_TRY_GAP_MS;
    assert_eq!(try_at(&roster, &mut rec, n.secret, &mut l, 30, true, late), Err((RefuseReason::Gone, 0)));
}

#[test]
fn a_locked_out_node_does_not_use_up_the_busy_budget() {
    let (roster, _) = genesis();
    let n = new_invite(space(), owner().node(), "", &[], "Club", NOW);
    let mut rec = InviteRecord::new(n.ticket.invite, NOW);
    let mut l = Lockouts::new();
    let bad = SpaceIdentity::for_test(2).node();
    for _ in 0..3 { l.strike(bad, NOW); }
    for _ in 0..30 {
        assert_eq!(try_at(&roster, &mut rec, n.secret, &mut l, 2, true, NOW), Err((RefuseReason::LockedOut, 0)));
    }
    assert_eq!(try_at(&roster, &mut rec, n.secret, &mut l, 3, true, NOW), Ok(()));
}

#[tokio::test]
async fn three_failed_joins_lock_the_node_out() {
    let rig = rig();
    let (id, net) = joiner(&rig, 2);
    // The invite pauses after one wrong try, so the strikes are given to the node directly (this test is about the
    // node lockout, not the invite).
    assert_eq!(refused(join(&rig, &id, &net, &wrong()).await).0, RefuseReason::WrongWords);
    for _ in 0..2 { rig.host.lockouts.lock().unwrap().strike(id.node(), NOW); }
    assert_eq!(refused(join(&rig, &id, &net, &rig.secret).await), (RefuseReason::LockedOut, 0));
}

#[test]
fn lockout_ends_after_ten_minutes() {
    let mut l = Lockouts::new();
    let n = NodeKey([5; 32]);
    for _ in 0..3 { l.strike(n, 1000); }
    assert!(l.locked(&n, 1000 + 599_999));
    assert!(!l.locked(&n, 1000 + 600_001));
}

#[test]
fn busy_after_twenty_requests_in_a_minute() {
    let mut l = Lockouts::new();
    let results: Vec<bool> = (0..21).map(|_| l.busy(5_000)).collect();
    assert!(results[..20].iter().all(|b| !b));
    assert!(results[20]);
    assert!(!l.busy(5_000 + 60_000));
}

#[tokio::test]
async fn cert_for_another_node_is_bad() {
    let rig = rig();
    let id = SpaceIdentity::for_test(3);
    // The link is authenticated as another node than the one inside the certificate.
    let net = rig.hub.node(NodeKey([77; 32]), SpacePermissions::new());
    assert_eq!(refused(join(&rig, &id, &net, &rig.secret).await), (RefuseReason::Bad, 0));
}

#[test]
fn full_space_refuses_without_using_the_invite() {
    let (mut roster, _) = genesis();
    for seed in 2..=50u8 {
        let who = SpaceIdentity::for_test(seed);
        let cert = who.certify(&roster.space, "Sam", NOW);
        let grant = seal_key_to(cert.device.0, &cert.kem_b64, &epoch_info(&roster.space, roster.epoch), &KEY).unwrap();
        let op = roster.next_op(&owner(), OpBody::AddMember { cert, epoch: roster.epoch, grant }, NOW).unwrap();
        roster.apply(&op).unwrap();
    }
    assert_eq!(roster.members.len(), MAX_MEMBERS);
    let n = new_invite(space(), owner().node(), "", &[], "Club", NOW);
    let mut rec = InviteRecord::new(n.ticket.invite, NOW);
    let me = SpaceIdentity::for_test(60);
    let cert = me.certify(&space(), "New", NOW);
    let proof = super::invite::join_proof(&n.secret, &space(), &n.ticket.invite, &me.node(), &me.member_key());
    let req = JoinRequestFrame::new(space(), n.ticket.invite, cert, proof, None);
    let mut l = Lockouts::new();
    let r = owner_check(&roster, Some(&mut rec), Some(n.secret), &mut l, &req, me.node(), NOW);
    assert_eq!(r, Err((RefuseReason::Full, 0)));
    assert_eq!(rec.state, InviteState::Open);
}

/// Owner log with one extra member (`seed`), plus that member's cert.
fn log_with(seed: u8) -> (Vec<ControlOp>, SpaceIdentity, DeviceCert) {
    let (mut roster, g) = genesis();
    let who = SpaceIdentity::for_test(seed);
    let cert = who.certify(&space(), "Sam", NOW);
    let op = owner_let_in(&roster, &owner(), &cert, &KEY, NOW).unwrap();
    roster.apply(&op).unwrap();
    (vec![g, op], who, cert)
}

fn ticket_for(owner_node: NodeKey) -> InviteTicket {
    InviteTicket {
        space: space(), invite: InviteId([3; 8]), owner_node, expires_ms: NOW + 1000, relay: String::new(),
        lan: vec![], space_name: "Club".into(),
    }
}

#[test]
fn welcome_from_wrong_owner_node_is_rejected() {
    let (ops, who, cert) = log_with(2);
    let ticket = ticket_for(NodeKey([99; 32]));
    let r = accept_welcome(&ticket, &who, &cert, &ops, vec![]);
    assert!(matches!(r, Err(JoinError::BadWelcome(_))));
}

#[test]
fn good_welcome_is_accepted_and_partial_is_not_done_yet() {
    let (ops, who, cert) = log_with(2);
    let ticket = ticket_for(owner().node());
    assert!(matches!(accept_welcome(&ticket, &who, &cert, &ops[..1], vec![]), Ok(None)));
    assert!(matches!(accept_welcome(&ticket, &who, &cert, &ops, vec![]), Ok(Some(JoinOutcome::Joined { .. }))));
}

#[test]
fn tampered_welcome_op_is_rejected() {
    let (ops, who, cert) = log_with(2);
    let mut raw: Vec<Vec<u8>> = ops.iter().map(|o| o.to_bytes()).collect();
    *raw[1].last_mut().unwrap() ^= 1; // the signature is the last 64 bytes
    let (decoded, hints) = decode_welcome(&welcome_body(&raw, &[])).unwrap();
    let r = accept_welcome(&ticket_for(owner().node()), &who, &cert, &decoded, hints);
    assert!(matches!(r, Err(JoinError::BadWelcome(_))));
}

#[test]
fn welcome_with_cut_off_bytes_is_rejected() {
    let (ops, _, _) = log_with(2);
    let raw: Vec<Vec<u8>> = ops.iter().map(|o| o.to_bytes()).collect();
    let body = welcome_body(&raw, &[]);
    assert!(decode_welcome(&body[..body.len() - 10]).is_err());
}

#[tokio::test]
async fn no_answer_in_thirty_minutes() {
    let rig = rig();
    let (id, net) = joiner(&rig, 2);
    rig.decide.send(Decision::NoAnswer).unwrap();
    assert_eq!(refused(join(&rig, &id, &net, &rig.secret).await), (RefuseReason::NoAnswer, 0));
}

#[test]
fn join_request_round_trips_with_an_optional_proof() {
    let me = SpaceIdentity::for_test(2);
    let cert = me.certify(&space(), "Sam", NOW);
    let plain = JoinRequestFrame::new(space(), InviteId([3; 8]), cert.clone(), [4; 32], None);
    assert_eq!(JoinRequestFrame::from_bytes(&plain.to_bytes()).unwrap(), plain);
    assert_eq!(plain.account_proof(), None);
    let with = JoinRequestFrame::new(space(), InviteId([3; 8]), cert, [4; 32], Some(b"badge"));
    let back = JoinRequestFrame::from_bytes(&with.to_bytes()).unwrap();
    assert_eq!(back.account_proof(), Some(b"badge".to_vec()));
}

#[test]
fn join_request_skips_unknown_fields_and_reads_old_requests() {
    let me = SpaceIdentity::for_test(2);
    let cert = me.certify(&space(), "Sam", NOW);
    let req = JoinRequestFrame::new(space(), InviteId([3; 8]), cert, [4; 32], Some(b"p"));
    // a newer writer adds bytes at the end: skipped
    let mut newer = req.to_bytes();
    newer.extend_from_slice(&[9, 9, 9]);
    assert_eq!(JoinRequestFrame::from_bytes(&newer).unwrap(), req);
    // an older writer sent no extension block at all: still read, no proof
    let full = req.to_bytes();
    let older = &full[..full.len() - (4 + req.ext.len())];
    let back = JoinRequestFrame::from_bytes(older).unwrap();
    assert_eq!(back.account_proof(), None);
    // a malformed extension block is refused
    let mut broken = req.clone();
    broken.ext = vec![1, 0, 9, 1];
    assert!(JoinRequestFrame::from_bytes(&broken.to_bytes()).is_err());
}

#[test]
fn peer_hint_round_trips() {
    let h = PeerHint { node: NodeKey([4; 32]), relay: "https://r.example".into(), lan: vec!["10.0.0.2:1".into()] };
    let mut w = super::wire::Writer::new();
    hint_to_bytes(&h, &mut w);
    let b = w.finish();
    assert_eq!(hint_from_bytes(&mut super::wire::Reader::new(&b)).unwrap(), h);
}

#[test]
fn a_welcome_with_too_many_ops_is_refused_before_any_replay() {
    let (_, g) = genesis();
    let ops = vec![g; WELCOME_OPS_MAX + 1];
    let me = SpaceIdentity::for_test(2);
    let cert = me.certify(&space(), "Sam", NOW);
    let r = accept_welcome(&ticket_for(owner().node()), &me, &cert, &ops, vec![]);
    assert!(matches!(&r, Err(JoinError::BadWelcome(m)) if m == "welcome too large"), "{r:?}");
}

#[test]
fn the_welcome_budget_stops_ops_and_bytes() {
    let mut b = WelcomeBudget::new();
    assert!(b.take(100, 1000).is_ok());
    assert!(b.take(WELCOME_OPS_MAX - 100, 0).is_ok());
    assert!(matches!(b.take(1, 0), Err(JoinError::BadWelcome(m)) if m == "welcome too large"));
    let mut b = WelcomeBudget::new();
    assert!(b.take(1, WELCOME_BYTES_MAX).is_ok());
    assert!(matches!(b.take(0, 1), Err(JoinError::BadWelcome(m)) if m == "welcome too large"));
}

#[test]
fn joined_outcome_debug_does_not_print_the_key() {
    let (roster, _) = genesis();
    let out = JoinOutcome::Joined { roster, epoch_key: [0xAB; 32], hints: vec![], ops: vec![] };
    let text = format!("{out:?}");
    assert!(!text.contains("171"), "{text}");
    assert!(!text.contains("0xab") && !text.contains("0xAB") && !text.contains("ab, ab"), "{text}");
}
