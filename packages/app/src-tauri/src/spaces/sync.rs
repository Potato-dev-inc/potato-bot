//! Sync between two members over one stream. Pure: no I/O, no clock. The service drives it.
use std::collections::{BTreeMap, BTreeSet};

use super::frame::Frame;
use super::ids::{DeviceKey, NodeKey, SpaceId};
use super::join::{hint_from_bytes, hint_to_bytes};
use super::limits::{MAX_ENTRY_BYTES, MAX_FRAME_BYTES, MAX_WANT_PER_ROUND};
use super::net::PeerHint;
use super::roster::ControlOp;
use super::wire::{Reader, WireError, Writer};

pub const HELLO: u8 = 0x01;
pub const CONTROL_OPS: u8 = 0x02;
pub const WANT: u8 = 0x03;
pub const ENTRIES: u8 = 0x04;
pub const PUSH: u8 = 0x05;
pub const PRESENCE: u8 = 0x06;
pub const ADDR_BOOK: u8 = 0x07;
pub const BYE: u8 = 0x08;

/// Most devices a Hello lists.
const HELLO_HEADS_MAX: usize = 120;
/// Control ops in one frame.
const OPS_PER_FRAME: usize = 100;
const OP_MAX_BYTES: usize = 60_000;
/// Devices in one Want.
const WANT_DEVICES_MAX: usize = 64;
/// Unknown frames in a row before the stream is closed.
const UNKNOWN_IN_ROW_MAX: u32 = 100;
const ENTRIES_FRAME_BUDGET: usize = MAX_FRAME_BYTES - 64;

/// What the session needs to read. Implemented by the service over SpaceStore + Roster.
pub trait SyncView {
    fn space(&self) -> SpaceId;
    fn control_seq(&self) -> u64;
    fn control_head(&self) -> [u8; 32];
    fn control_hash_at(&self, seq: u64) -> Option<[u8; 32]>;
    fn control_ops_after(&self, seq: u64) -> Vec<ControlOp>;
    fn heads(&self) -> Vec<(DeviceKey, u64, u64)>;
    fn floor(&self, device: &DeviceKey) -> u64;
    fn knows_device(&self, device: &DeviceKey) -> bool;
    fn missing(&self, device: &DeviceKey, from: u64, to: u64, max: usize) -> Vec<u64>;
    fn entries_for(&self, device: &DeviceKey, seqs: &[u64]) -> (Vec<Vec<u8>>, Vec<u64>);
    fn my_hint(&self) -> PeerHint;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    Send(u8, Vec<u8>),
    Ingest(Vec<Vec<u8>>),
    Gone(Vec<(DeviceKey, u64)>),
    ApplyControl(Vec<ControlOp>),
    Hint(PeerHint),
    /// Close the stream with this reason. The service writes `Bye(code)` first. Code 0 = the peer said bye.
    Close(u8),
}

pub struct SyncSession {
    pub peer: NodeKey,
    said_hello: bool,
    heard_hello: bool,
    /// The peer's control log is longer than mine: wait for its ops before asking for entries.
    awaiting_control: bool,
    their_heads: BTreeMap<DeviceKey, (u64, u64)>,
    asked: BTreeSet<(DeviceKey, u64)>,
    /// Asked for and not sent back. Not asked again on this stream (another peer may have them).
    skipped: BTreeSet<(DeviceKey, u64)>,
    /// Devices the peer answered for (an entry or an honoured Gone) since the last Want.
    answered: BTreeSet<DeviceKey>,
    unknown_in_row: u32,
}

impl SyncSession {
    pub fn new(peer: NodeKey) -> Self {
        Self {
            peer,
            said_hello: false,
            heard_hello: false,
            awaiting_control: false,
            their_heads: BTreeMap::new(),
            asked: BTreeSet::new(),
            skipped: BTreeSet::new(),
            answered: BTreeSet::new(),
            unknown_in_row: 0,
        }
    }

    pub fn start(&mut self, v: &dyn SyncView) -> Vec<Out> {
        self.said_hello = true;
        vec![Out::Send(HELLO, encode_hello(v))]
    }

    pub fn on_frame(&mut self, v: &dyn SyncView, f: &Frame) -> Vec<Out> {
        if !self.heard_hello {
            return if f.kind == HELLO { self.on_hello(v, &f.body) } else { vec![Out::Close(3)] };
        }
        match f.kind {
            HELLO => {
                self.unknown_in_row = 0;
                Vec::new()
            }
            CONTROL_OPS => {
                self.unknown_in_row = 0;
                self.on_control_ops(&f.body)
            }
            WANT => {
                self.unknown_in_row = 0;
                self.on_want(v, &f.body)
            }
            ENTRIES => {
                self.unknown_in_row = 0;
                self.on_entries(v, &f.body)
            }
            PUSH => {
                self.unknown_in_row = 0;
                self.on_push(&f.body)
            }
            PRESENCE | ADDR_BOOK => {
                self.unknown_in_row = 0;
                Vec::new()
            }
            BYE => vec![Out::Close(0)],
            _ => {
                self.unknown_in_row += 1;
                if self.unknown_in_row > UNKNOWN_IN_ROW_MAX { vec![Out::Close(4)] } else { Vec::new() }
            }
        }
    }

    fn on_hello(&mut self, v: &dyn SyncView, body: &[u8]) -> Vec<Out> {
        let Ok(h) = decode_hello(body) else { return vec![Out::Close(3)] };
        if h.space != v.space() { return vec![Out::Close(3)]; }
        self.heard_hello = true;
        for (d, low, high) in &h.heads { self.their_heads.insert(*d, (*low, *high)); }
        // Only the peer's own address is kept: a hint for any other node is not what that node said about itself.
        let mut out = Vec::new();
        if h.hint.node == self.peer { out.push(Out::Hint(h.hint.clone())); }
        let mine = v.control_seq();
        if h.control_seq <= mine {
            let same_prefix = h.control_seq == 0 || v.control_hash_at(h.control_seq) == Some(h.control_head);
            if !same_prefix { return vec![Out::Close(2)]; }
            if h.control_seq < mine {
                let ops = v.control_ops_after(h.control_seq);
                for chunk in ops.chunks(OPS_PER_FRAME) { out.push(Out::Send(CONTROL_OPS, encode_ops(chunk))); }
            }
        } else {
            self.awaiting_control = true;
            return out;
        }
        out.extend(self.next_round(v));
        out
    }

    fn on_control_ops(&mut self, body: &[u8]) -> Vec<Out> {
        let Ok(mut ops) = decode_ops(body) else { return vec![Out::Close(2)] };
        ops.sort_by_key(|o| o.seq);
        self.awaiting_control = false;
        vec![Out::ApplyControl(ops)]
    }

    fn on_want(&mut self, v: &dyn SyncView, body: &[u8]) -> Vec<Out> {
        let Ok(asks) = decode_want(body) else { return vec![Out::Close(4)] };
        let (mut have, mut gone) = (Vec::new(), Vec::new());
        for (device, seqs) in asks {
            let (mut got, gone_seqs) = v.entries_for(&device, &seqs);
            have.append(&mut got);
            gone.extend(gone_seqs.into_iter().map(|s| (device, s)));
        }
        encode_entries(&have, &gone).into_iter().map(|b| Out::Send(ENTRIES, b)).collect()
    }

    fn on_entries(&mut self, v: &dyn SyncView, body: &[u8]) -> Vec<Out> {
        let Ok((entries, gone, last)) = decode_entries(body) else { return vec![Out::Close(4)] };
        // A Gone is honoured only for a seq this session asked for, of a device that is on the roster: a peer
        // cannot make this PC write placeholders for seqs it never asked about.
        let gone: Vec<(DeviceKey, u64)> =
            gone.into_iter().filter(|(d, s)| self.asked.contains(&(*d, *s)) && v.knows_device(d)).collect();
        for (d, s) in &gone {
            self.asked.remove(&(*d, *s));
            self.skipped.insert((*d, *s));
            self.answered.insert(*d);
        }
        let mut out = Vec::new();
        if !gone.is_empty() { out.push(Out::Gone(gone)); }
        if !entries.is_empty() {
            for b in &entries {
                if let Some((d, s)) = header_of(b) {
                    self.asked.remove(&(d, s));
                    self.answered.insert(d);
                    // Never asked again on this stream, even if the ingest refuses it.
                    self.skipped.insert((d, s));
                }
            }
            out.push(Out::Ingest(entries));
        }
        if last {
            // The reply is complete: what is still asked for, the peer does not have.
            let rest: Vec<_> = self.asked.iter().cloned().collect();
            // A device the peer answered nothing for: its claimed head was false, so it is not believed again
            // (until a Push shows a real entry). Otherwise a huge claimed head would fill every round.
            for (d, _) in &rest {
                if !self.answered.contains(d) { self.their_heads.remove(d); }
            }
            self.skipped.extend(rest);
            self.asked.clear();
            self.answered.clear();
        }
        out
    }

    fn on_push(&mut self, body: &[u8]) -> Vec<Out> {
        let mut r = Reader::new(body);
        let Ok(bytes) = r.blob(MAX_ENTRY_BYTES) else { return vec![Out::Close(4)] };
        if let Some((d, s)) = header_of(bytes) {
            let e = self.their_heads.entry(d).or_insert((1, s));
            e.1 = e.1.max(s);
        }
        vec![Out::Ingest(vec![bytes.to_vec()])]
    }

    /// Called by the service after it ingested an Entries or Push frame or applied control ops.
    /// Sends one Want when something the peer says it has is missing here.
    pub fn next_round(&mut self, v: &dyn SyncView) -> Vec<Out> {
        if !self.heard_hello || self.awaiting_control || !self.asked.is_empty() { return Vec::new(); }
        let mut gaps: Vec<(u64, DeviceKey, u64, u64)> = Vec::new();
        for (d, (low, high)) in &self.their_heads {
            if !v.knows_device(d) { continue; }
            let from = (*low).max(v.floor(d) + 1);
            if *high >= from { gaps.push((*high - from + 1, *d, from, *high)); }
        }
        gaps.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut budget = MAX_WANT_PER_ROUND;
        let mut asks: Vec<(DeviceKey, Vec<u64>)> = Vec::new();
        for (_, d, from, high) in gaps {
            if budget == 0 || asks.len() >= WANT_DEVICES_MAX { break; }
            let extra = self.skipped.iter().filter(|(sd, _)| *sd == d).count();
            let seqs: Vec<u64> = v
                .missing(&d, from, high, budget + extra)
                .into_iter()
                .filter(|s| !self.skipped.contains(&(d, *s)))
                .take(budget)
                .collect();
            if seqs.is_empty() { continue; }
            budget -= seqs.len();
            asks.push((d, seqs));
        }
        if asks.is_empty() { return Vec::new(); }
        for (d, seqs) in &asks {
            for s in seqs { self.asked.insert((*d, *s)); }
        }
        vec![Out::Send(WANT, encode_want(&asks))]
    }

    pub fn push_frame(entry_bytes: &[u8]) -> Out {
        let mut w = Writer::new();
        w.blob(entry_bytes);
        Out::Send(PUSH, w.finish())
    }

    /// Heard the peer's Hello and nothing is outstanding.
    pub fn is_synced(&self) -> bool { self.heard_hello && !self.awaiting_control && self.asked.is_empty() }

    /// The highest seq the peer is known to have for a device (from its Hello and Pushes).
    pub fn their_high(&self, device: &DeviceKey) -> Option<u64> { self.their_heads.get(device).map(|h| h.1) }
}

/// (author device, seq) of an entry, read from its header only.
fn header_of(bytes: &[u8]) -> Option<(DeviceKey, u64)> {
    super::log::Entry::from_bytes(bytes).ok().map(|e| (e.header.device, e.header.seq))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub space: SpaceId,
    pub control_seq: u64,
    pub control_head: [u8; 32],
    pub heads: Vec<(DeviceKey, u64, u64)>,
    pub hint: PeerHint,
}

pub fn encode_hello(v: &dyn SyncView) -> Vec<u8> {
    let mut heads = v.heads();
    if heads.len() > HELLO_HEADS_MAX {
        // Drop the devices with the smallest gap.
        let gap = |h: &(DeviceKey, u64, u64)| (h.2 + 1).saturating_sub(h.1);
        heads.sort_by(|a, b| gap(b).cmp(&gap(a)).then(a.0.cmp(&b.0)));
        heads.truncate(HELLO_HEADS_MAX);
    }
    let mut w = Writer::new();
    w.u8(1).fixed(&v.space().0).u64(v.control_seq()).fixed(&v.control_head()).u16(heads.len() as u16);
    for (d, low, high) in &heads { w.fixed(&d.0).u64(*low).u64(*high); }
    hint_to_bytes(&v.my_hint(), &mut w);
    w.finish()
}

pub fn decode_hello(body: &[u8]) -> Result<Hello, WireError> {
    let mut r = Reader::new(body);
    if r.u8()? != 1 { return Err(WireError("bad version")); }
    let space = SpaceId(r.fixed()?);
    let control_seq = r.u64()?;
    let control_head = r.fixed()?;
    let n = r.u16()? as usize;
    if n > HELLO_HEADS_MAX { return Err(WireError("too many heads")); }
    let mut heads = Vec::with_capacity(n);
    for _ in 0..n { heads.push((DeviceKey(r.fixed()?), r.u64()?, r.u64()?)); }
    let hint = hint_from_bytes(&mut r)?;
    r.skip_rest();
    Ok(Hello { space, control_seq, control_head, heads, hint })
}

pub fn encode_ops(ops: &[ControlOp]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u16(ops.len() as u16);
    for o in ops { w.blob(&o.to_bytes()); }
    w.finish()
}

fn decode_ops(body: &[u8]) -> Result<Vec<ControlOp>, WireError> {
    let mut r = Reader::new(body);
    let n = r.u16()? as usize;
    if n > OPS_PER_FRAME { return Err(WireError("too many ops")); }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(ControlOp::from_bytes(r.blob(OP_MAX_BYTES)?).map_err(|_| WireError("bad op"))?);
    }
    r.skip_rest();
    Ok(out)
}

pub fn encode_want(asks: &[(DeviceKey, Vec<u64>)]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u16(asks.len() as u16);
    for (d, seqs) in asks {
        w.fixed(&d.0).u16(seqs.len() as u16);
        for s in seqs { w.u64(*s); }
    }
    w.finish()
}

/// Rejects more than 64 devices and more than 500 seqs in all.
pub fn decode_want(body: &[u8]) -> Result<Vec<(DeviceKey, Vec<u64>)>, WireError> {
    let mut r = Reader::new(body);
    let n = r.u16()? as usize;
    if n > WANT_DEVICES_MAX { return Err(WireError("too many devices")); }
    let mut total = 0usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let d = DeviceKey(r.fixed()?);
        let k = r.u16()? as usize;
        total += k;
        if total > MAX_WANT_PER_ROUND { return Err(WireError("too many seqs")); }
        let mut seqs = Vec::with_capacity(k);
        for _ in 0..k { seqs.push(r.u64()?); }
        out.push((d, seqs));
    }
    r.skip_rest();
    Ok(out)
}

/// Splits into frames of at most MAX_FRAME_BYTES - 64. Always returns at least one frame; the last one carries
/// `last = 1` and the gone list, so the asker knows the reply is complete.
pub fn encode_entries(entries: &[Vec<u8>], gone: &[(DeviceKey, u64)]) -> Vec<Vec<u8>> {
    let mut groups: Vec<Vec<&Vec<u8>>> = vec![Vec::new()];
    let mut size = 0usize;
    // 2 (n) + 2 (g) + 1 (last) + the gone list
    let fixed = 5 + gone.len() * 40;
    for e in entries {
        let cur = groups.last().map(|g| g.len()).unwrap_or(0);
        if cur > 0 && (size + e.len() + 4 + fixed > ENTRIES_FRAME_BUDGET || cur >= u16::MAX as usize) {
            groups.push(Vec::new());
            size = 0;
        }
        size += e.len() + 4;
        groups.last_mut().expect("one group").push(e);
    }
    let count = groups.len();
    groups
        .into_iter()
        .enumerate()
        .map(|(i, g)| {
            let is_last = i + 1 == count;
            let mut w = Writer::new();
            w.u16(g.len() as u16);
            for e in g { w.blob(e); }
            let gl: &[(DeviceKey, u64)] = if is_last { gone } else { &[] };
            w.u16(gl.len() as u16);
            for (d, s) in gl { w.fixed(&d.0).u64(*s); }
            w.u8(is_last as u8);
            w.finish()
        })
        .collect()
}

pub type EntriesBody = (Vec<Vec<u8>>, Vec<(DeviceKey, u64)>, bool);

/// (entries, gone, last).
pub fn decode_entries(body: &[u8]) -> Result<EntriesBody, WireError> {
    let mut r = Reader::new(body);
    let n = r.u16()? as usize;
    let mut entries = Vec::new();
    for _ in 0..n { entries.push(r.blob(MAX_ENTRY_BYTES)?.to_vec()); }
    let g = r.u16()? as usize;
    if g > MAX_WANT_PER_ROUND { return Err(WireError("too many gone")); }
    let mut gone = Vec::with_capacity(g);
    for _ in 0..g { gone.push((DeviceKey(r.fixed()?), r.u64()?)); }
    let last = r.u8()? != 0;
    r.skip_rest();
    Ok((entries, gone, last))
}
