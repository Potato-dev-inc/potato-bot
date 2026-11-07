//! Joining a space. Joiner: dial owner, send JoinRequest, wait. Owner: check, ask the person, welcome.
use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;

use super::frame::{read_frame, write_frame, Frame};
use super::identity::{ext_find, ext_with_proof, DeviceCert, SpaceIdentity, EXT_MAX_BYTES, EXT_PROOF, PROOF_MAX_BYTES};
use super::ids::{InviteId, NodeKey, SpaceId};
use super::invite::{join_proof, proof_matches, Check, InviteRecord, InviteTicket};
use super::limits::*;
use super::net::{NetError, PeerHint, SpaceNet, SpaceStream};
use super::roster::{epoch_info, ControlOp, OpBody, Roster, RosterError};
use super::wire::{Reader, WireError, Writer};
use super::words::SECRET_BYTES;
use crate::secure_core::grant::seal_key_to;

pub const JOIN_REQUEST: u8 = 0x11;
pub const JOIN_WAIT: u8 = 0x12;
pub const JOIN_WELCOME: u8 = 0x13;
pub const JOIN_REFUSED: u8 = 0x14;

/// Biggest certificate a request may carry (the cert itself holds a name, a key and an extension block).
const CERT_MAX_BYTES: usize = 2048;
/// Control ops in one welcome frame.
const OPS_PER_FRAME: usize = 100;
/// One control op, at most (same cap as the roster uses for an op body).
const OP_MAX_BYTES: usize = 60_000;
const HINTS_MAX: usize = 12;
const LAN_MAX: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefuseReason { WrongWords = 1, Gone = 2, No = 3, LockedOut = 4, Busy = 5, Full = 6, NoAnswer = 7, Bad = 8 }

pub fn refuse_reason(b: u8) -> RefuseReason {
    match b {
        1 => RefuseReason::WrongWords,
        2 => RefuseReason::Gone,
        3 => RefuseReason::No,
        4 => RefuseReason::LockedOut,
        5 => RefuseReason::Busy,
        6 => RefuseReason::Full,
        7 => RefuseReason::NoAnswer,
        _ => RefuseReason::Bad,
    }
}

/// The request. `ext` is an optional extension block after the known fields (same layout as a cert's: tag, length,
/// bytes). Tag 1 is an account proof that is not trusted for anything yet (shown as a badge later). A reader skips
/// tags and trailing fields it does not know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinRequestFrame {
    pub space: SpaceId,
    pub invite: InviteId,
    pub cert: DeviceCert,
    pub proof: [u8; 32],
    pub ext: Vec<u8>,
}

impl JoinRequestFrame {
    pub fn new(space: SpaceId, invite: InviteId, cert: DeviceCert, proof: [u8; 32], account_proof: Option<&[u8]>) -> Self {
        Self { space, invite, cert, proof, ext: ext_with_proof(account_proof) }
    }
    pub fn account_proof(&self) -> Option<Vec<u8>> {
        ext_find(&self.ext, EXT_PROOF).flatten().filter(|p| p.len() <= PROOF_MAX_BYTES)
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(1).fixed(&self.space.0).fixed(&self.invite.0).blob(&self.cert.to_bytes()).fixed(&self.proof).blob(&self.ext);
        w.finish()
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, ()> {
        let mut r = Reader::new(b);
        let e = |_: WireError| ();
        if r.u8().map_err(e)? != 1 { return Err(()); }
        let space = SpaceId(r.fixed().map_err(e)?);
        let invite = InviteId(r.fixed().map_err(e)?);
        let cert = DeviceCert::from_bytes(r.blob(CERT_MAX_BYTES).map_err(e)?).map_err(e)?;
        let proof = r.fixed().map_err(e)?;
        // Fields a newer writer adds come after the known ones; an older request simply ends here.
        let ext = if r.remaining() > 0 { r.blob(EXT_MAX_BYTES).map_err(e)?.to_vec() } else { Vec::new() };
        if ext_find(&ext, EXT_PROOF).is_none() { return Err(()); }
        r.skip_rest();
        Ok(Self { space, invite, cert, proof, ext })
    }
}

pub fn hint_to_bytes(h: &PeerHint, w: &mut Writer) {
    w.fixed(&h.node.0).str(&h.relay).u8(h.lan.len().min(LAN_MAX) as u8);
    for a in h.lan.iter().take(LAN_MAX) { w.str(a); }
}

pub fn hint_from_bytes(r: &mut Reader) -> Result<PeerHint, WireError> {
    let node = NodeKey(r.fixed()?);
    let relay = r.str(200)?;
    let k = r.u8()? as usize;
    if k > LAN_MAX { return Err(WireError("too many addresses")); }
    let mut lan = Vec::new();
    for _ in 0..k { lan.push(r.str(64)?); }
    Ok(PeerHint { node, relay, lan })
}

/// Strangers' failed joins, across all spaces on this PC. In memory, at most 256 nodes (oldest dropped).
pub struct Lockouts {
    /// node -> (strikes, locked until, last strike)
    by_node: BTreeMap<NodeKey, (u32, u64, u64)>,
    /// invite -> no try before this time (set by a wrong try)
    by_invite: BTreeMap<InviteId, u64>,
    minute_start: u64,
    minute_count: u32,
}

const LOCKOUT_NODES_MAX: usize = 256;

impl Lockouts {
    pub fn new() -> Self { Self { by_node: BTreeMap::new(), by_invite: BTreeMap::new(), minute_start: 0, minute_count: 0 } }

    pub fn locked(&self, node: &NodeKey, now_ms: u64) -> bool {
        matches!(self.by_node.get(node), Some((s, until, _)) if *s >= JOIN_LOCKOUT_STRIKES && now_ms < *until)
    }

    pub fn strike(&mut self, node: NodeKey, now_ms: u64) {
        if !self.by_node.contains_key(&node) && self.by_node.len() >= LOCKOUT_NODES_MAX {
            if let Some(oldest) = self.by_node.iter().min_by_key(|(_, v)| v.2).map(|(k, _)| *k) {
                self.by_node.remove(&oldest);
            }
        }
        let e = self.by_node.entry(node).or_insert((0, 0, now_ms));
        // A lockout that already ended starts the count again.
        if e.0 >= JOIN_LOCKOUT_STRIKES && now_ms >= e.1 { *e = (0, 0, now_ms); }
        e.0 += 1;
        e.2 = now_ms;
        if e.0 >= JOIN_LOCKOUT_STRIKES { e.1 = now_ms + JOIN_LOCKOUT_MS; }
    }

    pub fn len(&self) -> usize { self.by_node.len() }

    /// True while a wrong try on this invite is still being waited out.
    pub fn invite_paused(&self, invite: &InviteId, now_ms: u64) -> bool {
        matches!(self.by_invite.get(invite), Some(until) if now_ms < *until)
    }

    /// A wrong try on `invite`: no other try on it for INVITE_TRY_GAP_MS. At most 64 invites are remembered.
    pub fn pause_invite(&mut self, invite: InviteId, now_ms: u64) {
        self.by_invite.retain(|_, until| now_ms < *until);
        if self.by_invite.len() >= 64 && !self.by_invite.contains_key(&invite) {
            if let Some(first) = self.by_invite.keys().next().copied() { self.by_invite.remove(&first); }
        }
        self.by_invite.insert(invite, now_ms.saturating_add(INVITE_TRY_GAP_MS));
    }

    pub fn clear(&mut self, node: &NodeKey) { self.by_node.remove(node); }

    /// Counts this request; true when over JOINS_PER_MINUTE in the current minute.
    pub fn busy(&mut self, now_ms: u64) -> bool {
        if now_ms.saturating_sub(self.minute_start) >= 60_000 {
            self.minute_start = now_ms;
            self.minute_count = 0;
        }
        self.minute_count += 1;
        self.minute_count > JOINS_PER_MINUTE
    }
}

impl Default for Lockouts {
    fn default() -> Self { Self::new() }
}

pub enum Verdict { Wait { request_id: i64, answer_by_ms: u64 }, Refuse(RefuseReason, u8) }
pub enum Answer { LetIn { ops: Vec<ControlOp>, hints: Vec<PeerHint> }, No, NoAnswer }

#[async_trait]
pub trait JoinHost: Send + Sync {
    /// Runs the rules in "owner_check order"; persists invite state and the request.
    fn check(&self, req: &JoinRequestFrame, joiner: NodeKey, now_ms: u64) -> Verdict;
    /// Resolves when the owner clicks, or after JOIN_WAIT_MS with NoAnswer.
    async fn wait_answer(&self, request_id: i64) -> Answer;
}

/// Owner side, called by the service when a stranger's first frame is JoinRequest.
pub async fn serve_join(mut s: SpaceStream, joiner: NodeKey, first: Frame, host: &dyn JoinHost, now_ms: u64)
    -> Result<(), NetError>
{
    let Ok(req) = JoinRequestFrame::from_bytes(&first.body) else {
        let _ = write_frame(&mut s.send, JOIN_REFUSED, &[RefuseReason::Bad as u8, 0]).await;
        return Ok(());
    };
    match host.check(&req, joiner, now_ms) {
        Verdict::Refuse(r, left) => {
            let _ = write_frame(&mut s.send, JOIN_REFUSED, &[r as u8, left]).await;
        }
        Verdict::Wait { request_id, answer_by_ms } => {
            write_frame(&mut s.send, JOIN_WAIT, &answer_by_ms.to_be_bytes()).await.map_err(|_| NetError::Closed)?;
            let answer = tokio::time::timeout(Duration::from_millis(JOIN_WAIT_MS), host.wait_answer(request_id))
                .await
                .unwrap_or(Answer::NoAnswer);
            match answer {
                Answer::LetIn { ops, hints } => send_welcome(&mut s, &ops, &hints).await?,
                Answer::No => {
                    let _ = write_frame(&mut s.send, JOIN_REFUSED, &[RefuseReason::No as u8, 0]).await;
                }
                Answer::NoAnswer => {
                    let _ = write_frame(&mut s.send, JOIN_REFUSED, &[RefuseReason::NoAnswer as u8, 0]).await;
                }
            }
        }
    }
    Ok(())
}

/// One welcome frame body: u16 n, n x blob op, u8 k, k x hint.
pub fn welcome_body(raw_ops: &[Vec<u8>], hints: &[PeerHint]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u16(raw_ops.len() as u16);
    for o in raw_ops { w.blob(o); }
    let hints = &hints[..hints.len().min(HINTS_MAX)];
    w.u8(hints.len() as u8);
    for h in hints { hint_to_bytes(h, &mut w); }
    w.finish()
}

async fn send_welcome(s: &mut SpaceStream, ops: &[ControlOp], hints: &[PeerHint]) -> Result<(), NetError> {
    let raw: Vec<Vec<u8>> = ops.iter().map(|o| o.to_bytes()).collect();
    // Chunks of up to OPS_PER_FRAME ops, and never more bytes than one frame holds. Hints ride on the last frame.
    let mut chunks: Vec<Vec<Vec<u8>>> = vec![Vec::new()];
    let mut size = 0usize;
    for o in raw {
        let cur = chunks.last().map(|c| c.len()).unwrap_or(0);
        if cur >= OPS_PER_FRAME || (size + o.len() + 4 > MAX_FRAME_BYTES - 4096 && cur > 0) {
            chunks.push(Vec::new());
            size = 0;
        }
        size += o.len() + 4;
        chunks.last_mut().expect("one chunk").push(o);
    }
    let last = chunks.len() - 1;
    for (i, c) in chunks.iter().enumerate() {
        let body = welcome_body(c, if i == last { hints } else { &[] });
        write_frame(&mut s.send, JOIN_WELCOME, &body).await.map_err(|_| NetError::Closed)?;
    }
    Ok(())
}

/// Most control ops, and most bytes, a welcome may carry in all. A real control log is far smaller.
pub const WELCOME_OPS_MAX: usize = 5_000;
pub const WELCOME_BYTES_MAX: usize = 32 * 1024 * 1024;

/// Running total of a welcome, so a stream that never ends cannot make the joiner hold or replay without limit.
pub struct WelcomeBudget { ops: usize, bytes: usize }

impl WelcomeBudget {
    pub fn new() -> Self { Self { ops: 0, bytes: 0 } }
    pub fn take(&mut self, ops: usize, bytes: usize) -> Result<(), JoinError> {
        self.ops = self.ops.saturating_add(ops);
        self.bytes = self.bytes.saturating_add(bytes);
        if self.ops > WELCOME_OPS_MAX || self.bytes > WELCOME_BYTES_MAX {
            return Err(JoinError::BadWelcome("welcome too large".into()));
        }
        Ok(())
    }
}

impl Default for WelcomeBudget {
    fn default() -> Self { Self::new() }
}

pub enum JoinOutcome {
    /// `ops` is the whole control log the owner sent (the new member stores it so it can serve it on).
    Joined { roster: Roster, epoch_key: [u8; 32], hints: Vec<PeerHint>, ops: Vec<ControlOp> },
    Refused(RefuseReason, u8),
}

// Written by hand: the derived Debug would print the space key.
impl std::fmt::Debug for JoinOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JoinOutcome::Joined { roster, hints, ops, .. } => f
                .debug_struct("Joined")
                .field("space", &roster.space)
                .field("epoch", &roster.epoch)
                .field("epoch_key", &"<hidden>")
                .field("hints", &hints.len())
                .field("ops", &ops.len())
                .finish(),
            JoinOutcome::Refused(r, left) => f.debug_tuple("Refused").field(r).field(left).finish(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum JoinError { Unreachable, BadWelcome(String), Closed }

/// Joiner side. `on_wait` is called with the answer-by time when the owner's PC says "waiting".
pub async fn run_joiner(
    net: &dyn SpaceNet, ticket: &InviteTicket, secret: &[u8; SECRET_BYTES], me: &SpaceIdentity, my_name: &str,
    now_ms: u64, account_proof: Option<&[u8]>, on_wait: &(dyn Fn(u64) + Send + Sync),
) -> Result<JoinOutcome, JoinError> {
    let hint = PeerHint { node: ticket.owner_node, relay: ticket.relay.clone(), lan: ticket.lan.clone() };
    let link = net.dial(&hint).await.map_err(|_| JoinError::Unreachable)?;
    let mut s = link.open().await.map_err(|_| JoinError::Closed)?;
    let cert = me.certify_with_proof(&ticket.space, my_name, now_ms, account_proof);
    let proof = join_proof(secret, &ticket.space, &ticket.invite, &me.node(), &me.member_key());
    let req = JoinRequestFrame::new(ticket.space, ticket.invite, cert.clone(), proof, account_proof);
    write_frame(&mut s.send, JOIN_REQUEST, &req.to_bytes()).await.map_err(|_| JoinError::Closed)?;
    let mut ops: Vec<ControlOp> = Vec::new();
    let mut budget = WelcomeBudget::new();
    loop {
        let f = tokio::time::timeout(Duration::from_millis(JOIN_WAIT_MS + 60_000), read_frame(&mut s.recv))
            .await
            .map_err(|_| JoinError::Closed)?
            .map_err(|_| JoinError::Closed)?;
        match f.kind {
            JOIN_WAIT => {
                let b: [u8; 8] = f.body.get(..8).and_then(|x| x.try_into().ok()).ok_or(JoinError::Closed)?;
                on_wait(u64::from_be_bytes(b));
            }
            JOIN_REFUSED => {
                let reason = refuse_reason(*f.body.first().unwrap_or(&8));
                return Ok(JoinOutcome::Refused(reason, *f.body.get(1).unwrap_or(&0)));
            }
            JOIN_WELCOME => {
                let (mut part, hints) = decode_welcome(&f.body).map_err(JoinError::BadWelcome)?;
                budget.take(part.len(), f.body.len())?;
                // The roster is replayed (every signature checked) only when my own AddMember has arrived,
                // not after every frame.
                let mine = part.iter().any(|o| matches!(&o.body, OpBody::AddMember { cert: c, .. } if *c == cert));
                ops.append(&mut part);
                if mine {
                    if let Some(done) = accept_welcome(ticket, me, &cert, &ops, hints)? { return Ok(done); }
                }
            }
            _ => return Err(JoinError::BadWelcome("unexpected frame".into())),
        }
    }
}

pub fn decode_welcome(body: &[u8]) -> Result<(Vec<ControlOp>, Vec<PeerHint>), String> {
    let bad = |e: WireError| e.0.to_string();
    let mut r = Reader::new(body);
    let n = r.u16().map_err(bad)? as usize;
    if n > OPS_PER_FRAME { return Err("too many ops".into()); }
    let mut ops = Vec::with_capacity(n);
    for _ in 0..n {
        let b = r.blob(OP_MAX_BYTES).map_err(bad)?;
        ops.push(ControlOp::from_bytes(b).map_err(|e| e.0)?);
    }
    let k = r.u8().map_err(bad)? as usize;
    if k > HINTS_MAX { return Err("too many hints".into()); }
    let mut hints = Vec::with_capacity(k);
    for _ in 0..k { hints.push(hint_from_bytes(&mut r).map_err(bad)?); }
    r.skip_rest();
    Ok((ops, hints))
}

/// Ok(None) = not complete yet (my AddMember not seen). Checks, in order:
/// 1 `Roster::replay(ticket.space, ops)` succeeds;
/// 2 the owner has a device whose node == ticket.owner_node;
/// 3 there is a member whose device cert equals `my_cert` exactly;
/// 4 `roster.open_epoch_key(me, roster.epoch)` is Some.
pub fn accept_welcome(
    ticket: &InviteTicket, me: &SpaceIdentity, my_cert: &DeviceCert, ops: &[ControlOp], hints: Vec<PeerHint>,
) -> Result<Option<JoinOutcome>, JoinError> {
    let bad = |m: &str| JoinError::BadWelcome(m.to_string());
    if ops.len() > WELCOME_OPS_MAX { return Err(bad("welcome too large")); }
    let roster = Roster::replay(ticket.space, ops).map_err(|e| JoinError::BadWelcome(e.0))?;
    let owner_ok = roster
        .members
        .get(&roster.owner)
        .map(|m| m.devices.iter().any(|d| d.node == ticket.owner_node))
        .unwrap_or(false);
    if !owner_ok { return Err(bad("wrong owner")); }
    let me_in = roster.members.values().any(|m| m.devices.iter().any(|d| d == my_cert));
    if !me_in { return Ok(None); }
    let epoch_key = roster.open_epoch_key(me, roster.epoch).ok_or_else(|| bad("no key for me"))?;
    Ok(Some(JoinOutcome::Joined { roster, epoch_key, hints, ops: ops.to_vec() }))
}

/// The owner-side rules, in order. `invite` is None when no record exists for the request's invite id.
pub fn owner_check(
    roster: &Roster, invite: Option<&mut InviteRecord>, secret: Option<[u8; SECRET_BYTES]>, lockouts: &mut Lockouts,
    req: &JoinRequestFrame, joiner: NodeKey, now_ms: u64,
) -> Result<(), (RefuseReason, u8)> {
    // A locked-out node is answered first and does not use up the shared per-minute budget.
    if lockouts.locked(&joiner, now_ms) { return Err((RefuseReason::LockedOut, 0)); }
    // One wrong try pauses the whole invite (any node): fresh node keys cannot burn its five tries in a burst.
    // Also before the budget, so a flood against a paused invite costs the real joiners nothing.
    if lockouts.invite_paused(&req.invite, now_ms) { return Err((RefuseReason::Busy, 0)); }
    if lockouts.busy(now_ms) { return Err((RefuseReason::Busy, 0)); }
    if req.space != roster.space || !req.cert.verify() || req.cert.space != roster.space || req.cert.node != joiner {
        lockouts.strike(joiner, now_ms);
        return Err((RefuseReason::Bad, 0));
    }
    if roster.members.len() >= MAX_MEMBERS { return Err((RefuseReason::Full, 0)); }
    let (Some(invite), Some(secret)) = (invite, secret) else {
        lockouts.strike(joiner, now_ms);
        return Err((RefuseReason::Gone, 0));
    };
    if invite.id != req.invite {
        lockouts.strike(joiner, now_ms);
        return Err((RefuseReason::Gone, 0));
    }
    let expected = join_proof(&secret, &req.space, &req.invite, &joiner, &req.cert.member);
    let proof_ok = proof_matches(&expected, &req.proof);
    match invite.check(proof_ok, joiner, now_ms) {
        Check::Accept => {}
        Check::WrongWords { tries_left } => {
            lockouts.strike(joiner, now_ms);
            lockouts.pause_invite(req.invite, now_ms);
            return Err((RefuseReason::WrongWords, tries_left as u8));
        }
        Check::Gone => {
            lockouts.strike(joiner, now_ms);
            return Err((RefuseReason::Gone, 0));
        }
    }
    if roster.members.contains_key(&req.cert.member) { return Err((RefuseReason::Bad, 0)); }
    lockouts.clear(&joiner);
    Ok(())
}

/// The owner lets a person in: an `AddMember` op with the exact cert from the request and a grant of the current key.
pub fn owner_let_in(
    roster: &Roster, owner: &SpaceIdentity, cert: &DeviceCert, epoch_key: &[u8; 32], now_ms: u64,
) -> Result<ControlOp, RosterError> {
    let grant = seal_key_to(cert.device.0, &cert.kem_b64, &epoch_info(&roster.space, roster.epoch), epoch_key)
        .map_err(|_| RosterError("seal failed".into()))?;
    roster.next_op(owner, OpBody::AddMember { cert: cert.clone(), epoch: roster.epoch, grant }, now_ms)
}
