//! Presence: ephemeral, never stored in the log or the database. 90-second freshness.
use std::collections::BTreeMap;

use super::ids::MemberKey;
use super::wire::{Reader, WireError, Writer};

pub const FRESH_MS: u64 = 90_000;
pub const SEND_EVERY_MS: u64 = 30_000;
/// Most entries a Presence frame may relay.
pub const RELAY_MAX: usize = 50;
/// A report may be this far ahead of my clock before it is treated as "now".
pub const FUTURE_SLACK_MS: u64 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence { Online, Away, Offline }

impl Presence {
    pub fn name(self) -> &'static str {
        match self {
            Presence::Online => "online",
            Presence::Away => "away",
            Presence::Offline => "offline",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "online" => Some(Presence::Online),
            "away" => Some(Presence::Away),
            _ => None,
        }
    }
    fn code(self) -> u8 {
        match self {
            Presence::Online => 1,
            Presence::Away => 2,
            Presence::Offline => 0,
        }
    }
    fn from_code(c: u8) -> Option<Self> {
        match c {
            1 => Some(Presence::Online),
            2 => Some(Presence::Away),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct PresenceTable { seen: BTreeMap<MemberKey, (Presence, u64)> }

impl PresenceTable {
    pub fn new() -> Self { Self::default() }

    /// Keep the newest report per member; a report more than 10 s ahead of `now_ms` counts as `now_ms`.
    pub fn heard(&mut self, member: MemberKey, state: Presence, seen_ms: u64, now_ms: u64) {
        if state == Presence::Offline { return; }
        let seen = if seen_ms > now_ms.saturating_add(FUTURE_SLACK_MS) { now_ms } else { seen_ms };
        match self.seen.get(&member) {
            Some((_, old)) if *old > seen => {}
            _ => { self.seen.insert(member, (state, seen)); }
        }
    }

    /// Offline when nothing fresh is known (nothing heard, or older than 90 s).
    pub fn state_of(&self, member: &MemberKey, now_ms: u64) -> Presence {
        match self.seen.get(member) {
            Some((s, at)) if now_ms.saturating_sub(*at) <= FRESH_MS => *s,
            _ => Presence::Offline,
        }
    }

    /// What I relay: fresh entries only, at most 50.
    pub fn relay_list(&self, now_ms: u64) -> Vec<(MemberKey, Presence, u64)> {
        self.seen
            .iter()
            .filter(|(_, (_, at))| now_ms.saturating_sub(*at) <= FRESH_MS)
            .take(RELAY_MAX)
            .map(|(m, (s, at))| (*m, *s, *at))
            .collect()
    }

    /// Drop stale entries and entries of people no longer in the space.
    pub fn retain(&mut self, now_ms: u64, keep: impl Fn(&MemberKey) -> bool) {
        self.seen.retain(|m, (_, at)| now_ms.saturating_sub(*at) <= FRESH_MS && keep(m));
    }
}

/// Body: `u8` my state (1 online, 2 away), `u16` n (<= 50), n x (`[32]` member, `u8` state, `u64` seen_ms).
pub fn encode(mine: Presence, relay: &[(MemberKey, Presence, u64)]) -> Vec<u8> {
    let relay = &relay[..relay.len().min(RELAY_MAX)];
    let mut w = Writer::new();
    w.u8(mine.code()).u16(relay.len() as u16);
    for (m, s, at) in relay { w.fixed(&m.0).u8(s.code()).u64(*at); }
    w.finish()
}

pub fn decode(body: &[u8]) -> Result<(Presence, Vec<(MemberKey, Presence, u64)>), WireError> {
    let mut r = Reader::new(body);
    let mine = Presence::from_code(r.u8()?).ok_or(WireError("bad state"))?;
    let n = r.u16()? as usize;
    if n > RELAY_MAX { return Err(WireError("too many entries")); }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let m = MemberKey(r.fixed::<32>()?);
        let s = Presence::from_code(r.u8()?).ok_or(WireError("bad state"))?;
        out.push((m, s, r.u64()?));
    }
    r.done()?;
    Ok((mine, out))
}
