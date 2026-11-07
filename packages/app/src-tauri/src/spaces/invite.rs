//! Space invites: link + 8 words. 24 h, one use, 5 wrong tries.
use super::identity::clean_name;
use super::ids::{InviteId, MemberKey, NodeKey, SpaceId};
use super::limits::{INVITE_MAX_STRIKES, INVITE_TTL_MS};
use super::wire::{base32_decode, base32_encode, Reader, Writer};
use super::words::{self, WordsError, SECRET_BYTES};
use crate::secure_core::crypto::{hmac, random_bytes, same_mac};

pub const LINK_PREFIX: &str = "pms1:";
pub const PROOF_DOMAIN: &[u8] = b"pm/spaces/join/v1";
pub const CLOCK_SKEW_MS: u64 = 10 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InviteError { NotALink, Words(WordsError), Expired }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteTicket {
    pub space: SpaceId,
    pub invite: InviteId,
    pub owner_node: NodeKey,
    pub expires_ms: u64,
    pub relay: String,
    pub lan: Vec<String>,
    pub space_name: String,
}

impl InviteTicket {
    pub fn to_link(&self) -> String {
        let mut w = Writer::new();
        w.u8(1).fixed(&self.space.0).fixed(&self.invite.0).fixed(&self.owner_node.0).u64(self.expires_ms)
            .str(&self.relay).u8(self.lan.len().min(4) as u8);
        for a in self.lan.iter().take(4) { w.str(a); }
        w.str(&self.space_name);
        format!("{LINK_PREFIX}{}", base32_encode(&w.finish()))
    }

    /// Accepts the link, or the QR text (link + "#" + words). Leading/trailing spaces are ignored.
    pub fn parse(text: &str) -> Result<(InviteTicket, Option<[u8; SECRET_BYTES]>), InviteError> {
        let t = text.trim();
        let rest = t.strip_prefix(LINK_PREFIX).ok_or(InviteError::NotALink)?;
        let (b32, words_part) = match rest.split_once('#') { Some((a, b)) => (a, Some(b)), None => (rest, None) };
        let bytes = base32_decode(b32).ok_or(InviteError::NotALink)?;
        let mut r = Reader::new(&bytes);
        let bad = |_| InviteError::NotALink;
        if r.u8().map_err(bad)? != 1 { return Err(InviteError::NotALink); }
        let space = SpaceId(r.fixed().map_err(bad)?);
        let invite = InviteId(r.fixed().map_err(bad)?);
        let owner_node = NodeKey(r.fixed().map_err(bad)?);
        let expires_ms = r.u64().map_err(bad)?;
        let relay = r.str(200).map_err(bad)?;
        let k = r.u8().map_err(bad)?;
        if k > 4 { return Err(InviteError::NotALink); }
        let mut lan = Vec::new();
        for _ in 0..k { lan.push(r.str(64).map_err(bad)?); }
        let space_name = clean_name(&r.str(160).map_err(bad)?);
        // Fields added by a newer version come after the known ones and are skipped, not refused.
        r.skip_rest();
        let secret = match words_part {
            Some(w) => Some(words::parse_typed(w).map_err(InviteError::Words)?),
            None => None,
        };
        Ok((InviteTicket { space, invite, owner_node, expires_ms, relay, lan, space_name }, secret))
    }

    /// Joiner side clock check (10 minutes of skew allowed).
    pub fn check_not_expired(&self, now_ms: u64) -> Result<(), InviteError> {
        if now_ms > self.expires_ms.saturating_add(CLOCK_SKEW_MS) { Err(InviteError::Expired) } else { Ok(()) }
    }
}

pub fn qr_text(ticket: &InviteTicket, secret: &[u8; SECRET_BYTES]) -> String {
    format!("{}#{}", ticket.to_link(), words::encode(secret).join("-"))
}

pub struct NewInvite { pub ticket: InviteTicket, pub secret: [u8; SECRET_BYTES] }

pub fn new_invite(space: SpaceId, owner_node: NodeKey, relay: &str, lan: &[String], space_name: &str, now_ms: u64)
    -> NewInvite
{
    NewInvite {
        ticket: InviteTicket {
            space, invite: InviteId::random(), owner_node, expires_ms: now_ms + INVITE_TTL_MS,
            relay: relay.to_string(), lan: lan.iter().take(4).cloned().collect(), space_name: clean_name(space_name),
        },
        secret: random_bytes::<SECRET_BYTES>(),
    }
}

pub fn join_proof(secret: &[u8; SECRET_BYTES], space: &SpaceId, invite: &InviteId, joiner: &NodeKey,
                  member: &MemberKey) -> [u8; 32] {
    let mut m = PROOF_DOMAIN.to_vec();
    m.extend_from_slice(&space.0);
    m.extend_from_slice(&invite.0);
    m.extend_from_slice(&joiner.0);
    m.extend_from_slice(&member.0);
    hmac(secret, &m)
}

pub fn proof_matches(expected: &[u8; 32], got: &[u8; 32]) -> bool { same_mac(expected, got) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteState { Open, Pending(NodeKey), Used, Dead, Cancelled }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteRecord { pub id: InviteId, pub created_ms: u64, pub expires_ms: u64, pub strikes: u32, pub state: InviteState }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check { Accept, WrongWords { tries_left: u32 }, Gone }

impl InviteRecord {
    pub fn new(id: InviteId, created_ms: u64) -> Self {
        Self { id, created_ms, expires_ms: created_ms + INVITE_TTL_MS, strikes: 0, state: InviteState::Open }
    }
    pub fn is_open(&self, now_ms: u64) -> bool {
        matches!(self.state, InviteState::Open | InviteState::Pending(_)) && now_ms < self.expires_ms
    }
    /// The whole rule set. The caller persists `self` after every call.
    pub fn check(&mut self, proof_ok: bool, joiner: NodeKey, now_ms: u64) -> Check {
        match self.state {
            InviteState::Used | InviteState::Dead | InviteState::Cancelled => return Check::Gone,
            _ => {}
        }
        if now_ms >= self.expires_ms { self.state = InviteState::Dead; return Check::Gone; }
        if let InviteState::Pending(j) = self.state {
            if j != joiner { return Check::Gone; }
        }
        if proof_ok {
            self.state = InviteState::Pending(joiner);
            return Check::Accept;
        }
        self.strikes += 1;
        if self.strikes >= INVITE_MAX_STRIKES { self.state = InviteState::Dead; return Check::Gone; }
        Check::WrongWords { tries_left: INVITE_MAX_STRIKES - self.strikes }
    }
    /// Owner answered (either way): the invite is used (plan decision P-3).
    pub fn answered(&mut self) { self.state = InviteState::Used; }
    pub fn cancel(&mut self) { self.state = InviteState::Cancelled; }
    /// No answer within JOIN_WAIT_MS.
    pub fn timed_out(&mut self) { self.state = InviteState::Dead; }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket() -> InviteTicket {
        InviteTicket {
            space: SpaceId([1; 16]),
            invite: InviteId([2; 8]),
            owner_node: NodeKey([3; 32]),
            expires_ms: 5000,
            relay: "https://r.example".into(),
            lan: vec!["192.168.1.5:4000".into(), "10.0.0.2:4000".into()],
            space_name: "Book club".into(),
        }
    }
    const A: NodeKey = NodeKey([0xa; 32]);
    const B: NodeKey = NodeKey([0xb; 32]);

    #[test]
    fn link_round_trips() {
        let t = ticket();
        assert_eq!(InviteTicket::parse(&t.to_link()).unwrap(), (t, None));
    }
    #[test]
    fn qr_text_carries_the_words() {
        let t = ticket();
        assert_eq!(InviteTicket::parse(&qr_text(&t, &[3; 11])).unwrap(), (t, Some([3; 11])));
    }
    #[test]
    fn link_starts_with_prefix_and_is_lowercase() {
        let l = ticket().to_link();
        let rest = l.strip_prefix("pms1:").unwrap();
        assert!(rest.bytes().all(|c| c.is_ascii_lowercase() || (b'2'..=b'7').contains(&c)));
    }
    #[test]
    fn not_a_link_is_rejected() {
        let wrong_version = format!("pms1:{}", base32_encode(&[2]));
        for bad in ["hello", "pms1:!!!", wrong_version.as_str()] {
            assert_eq!(InviteTicket::parse(bad), Err(InviteError::NotALink), "{bad}");
        }
    }
    #[test]
    fn unknown_trailing_fields_in_a_link_are_skipped() {
        let l = ticket().to_link();
        let mut bytes = base32_decode(l.strip_prefix("pms1:").unwrap()).unwrap();
        bytes.extend_from_slice(&[9, 9, 9]);
        let newer = format!("pms1:{}", base32_encode(&bytes));
        assert_eq!(InviteTicket::parse(&newer).unwrap().0, ticket());
    }
    #[test]
    fn expiry_is_24_hours() {
        let n = new_invite(SpaceId([1; 16]), NodeKey([3; 32]), "", &[], "Club", 1000);
        assert_eq!(n.ticket.expires_ms, 1000 + 86_400_000);
    }
    #[test]
    fn joiner_clock_allows_ten_minutes_skew() {
        let t = InviteTicket { expires_ms: 1000, ..ticket() };
        assert_eq!(t.check_not_expired(1000 + 599_999), Ok(()));
        assert_eq!(t.check_not_expired(1000 + 600_001), Err(InviteError::Expired));
    }
    fn rec() -> InviteRecord { InviteRecord::new(InviteId([2; 8]), 1000) }
    #[test]
    fn right_words_accept_and_pend() {
        let mut r = rec();
        assert_eq!(r.check(true, A, 2000), Check::Accept);
        assert_eq!(r.state, InviteState::Pending(A));
    }
    #[test]
    fn one_use_other_joiner_gets_gone() {
        let mut r = rec();
        r.check(true, A, 2000);
        assert_eq!(r.check(true, B, 2000), Check::Gone);
    }
    #[test]
    fn same_joiner_can_reconnect_while_pending() {
        let mut r = rec();
        r.check(true, A, 2000);
        assert_eq!(r.check(true, A, 2001), Check::Accept);
    }
    #[test]
    fn fifth_wrong_try_kills_the_invite() {
        let mut r = rec();
        for left in [4, 3, 2, 1] {
            assert_eq!(r.check(false, A, 2000), Check::WrongWords { tries_left: left });
        }
        assert_eq!(r.check(false, A, 2000), Check::Gone);
        assert_eq!(r.state, InviteState::Dead);
    }
    #[test]
    fn dead_invite_stays_dead_even_with_right_words() {
        let mut r = rec();
        for _ in 0..5 { r.check(false, A, 2000); }
        assert_eq!(r.check(true, A, 2000), Check::Gone);
    }
    #[test]
    fn expired_invite_is_gone() {
        let mut r = rec();
        let at = r.expires_ms;
        assert_eq!(r.check(true, A, at), Check::Gone);
        assert_eq!(r.state, InviteState::Dead);
    }
    #[test]
    fn answered_invite_is_used() {
        let mut r = rec();
        r.check(true, A, 2000);
        r.answered();
        assert_eq!(r.check(true, A, 2000), Check::Gone);
    }
    #[test]
    fn proof_binds_the_joiner_node() {
        let s = [5u8; 11];
        let (sp, inv, m) = (SpaceId([1; 16]), InviteId([2; 8]), MemberKey([7; 32]));
        let a = join_proof(&s, &sp, &inv, &A, &m);
        let b = join_proof(&s, &sp, &inv, &B, &m);
        assert_ne!(a, b);
        assert!(proof_matches(&a, &a));
        assert!(!proof_matches(&a, &b));
    }
    #[test]
    fn parse_reports_bad_word_in_qr() {
        let t = ticket();
        let w = words::encode(&[0; 11]);
        let text = format!("{}#{}-zzzz-{}", t.to_link(), w[0], w[2..].join("-"));
        assert_eq!(InviteTicket::parse(&text), Err(InviteError::Words(WordsError::Unknown(2, "zzzz".into()))));
    }
}
