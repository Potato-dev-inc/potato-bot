//! Pairing for the phone web app (`potato::phone`): a code that works once for ten minutes, a per-phone
//! token, a list of phones and revoke.
//!
//! - One pending code at a time; a new one replaces the old one.
//! - Single use: a good redeem burns the code. Five wrong guesses against a live code burn it too (the person
//!   makes a new one on the PC), so a guesser gets five tries in ten minutes, not a search.
//! - A code can be a *re-link*: it gives an already paired phone a new token and keeps its id, name and what it
//!   may do. The phone's address changed (a tunnel restart, a new network); the pairing did not.
//! - The token is 32 random bytes; only its SHA-256 is stored. The token is the whole secret: nothing here
//!   knows an address, so a later stable address needs no change to pairing.
//! - `hello` keys: a phone checks that the machine at an address is its PC by asking for an HMAC of a fresh
//!   nonce under a key only that PC can know (the pairing code before pairing, the token's hash after).

use super::*;
use subtle::ConstantTimeEq;

pub const CODE_TTL_SECS: u64 = 600;
pub const MAX_WRONG_GUESSES: u8 = 5;
const CODE_LEN: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneCode {
    pub code: String,
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneGrant {
    pub device_id: String,
    pub token: String,
    pub name: String,
    pub relinked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairFail {
    /// No live code, or it does not match.
    BadCode,
    /// The code ran out (ten minutes).
    Expired,
    /// Too many wrong guesses: this code is dead.
    Burned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRow {
    pub id: String,
    pub name: String,
    pub paired_at: u64,
    pub last_seen: Option<u64>,
    pub expired: bool,
    pub caps: Vec<Cap>,
    /// The person's yes, at the PC, to this phone controlling the bots' computer.
    pub control_computer: bool,
}

fn normalize_code(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase()
}

fn new_code() -> String {
    use rand::RngCore;
    const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = [0_u8; CODE_LEN];
    OsRng.fill_bytes(&mut bytes);
    // 256 is a multiple of 32, so the modulo adds no bias.
    bytes.iter().map(|b| CHARSET[(*b as usize) % CHARSET.len()] as char).collect()
}

fn clean_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).take(60).collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        "Phone".to_string()
    } else {
        cleaned
    }
}

impl MobilePairingStore {
    /// Makes the one pending code. `relink`: a paired device's id to give a new token to (it must exist).
    pub fn mint_phone_code(&mut self, address: String, caps: Caps, relink: Option<String>) -> Result<PhoneCode, String> {
        if let Some(id) = &relink {
            if !self.devices.iter().any(|d| &d.id == id) {
                return Err("That phone is not paired any more.".to_string());
            }
        }
        let code = new_code();
        let expires_at = now_secs() + CODE_TTL_SECS;
        self.pending = Some(PendingPairing { code: code.clone(), expires_at, bridge_url: address, caps, relink, failures: 0 });
        let _ = self.persist();
        Ok(PhoneCode { code, expires_at })
    }

    /// Spends a code. Single use: success clears it. Every failure leaves the caller one of three plain answers.
    pub fn redeem_phone_code(&mut self, code: &str, device_name: &str) -> Result<PhoneGrant, PairFail> {
        self.redeem_phone_code_at(code, device_name, now_secs())
    }

    pub fn redeem_phone_code_at(&mut self, code: &str, device_name: &str, now: u64) -> Result<PhoneGrant, PairFail> {
        let given = normalize_code(code);
        let Some(pending) = self.pending.as_mut() else { return Err(PairFail::BadCode) };
        if now > pending.expires_at {
            self.pending = None;
            let _ = self.persist();
            return Err(PairFail::Expired);
        }
        let same: bool = pending.code.as_bytes().ct_eq(given.as_bytes()).into();
        if !same {
            pending.failures += 1;
            let burned = pending.failures >= MAX_WRONG_GUESSES;
            if burned {
                self.pending = None;
                let _ = self.persist();
                return Err(PairFail::Burned);
            }
            return Err(PairFail::BadCode);
        }
        let (caps, relink) = (pending.caps, pending.relink.clone());
        self.pending = None;

        use rand::RngCore;
        let mut raw = [0_u8; 32];
        OsRng.fill_bytes(&mut raw);
        let token = encode_b64(&raw);
        let token_hash_b64 = encode_b64(Sha256::digest(raw).as_slice());
        let exp = now + DEVICE_IDLE_EXPIRY_SECS;

        let (device_id, name, relinked, caps) = match relink.and_then(|id| self.devices.iter().position(|d| d.id == id)) {
            Some(index) => {
                let device = &mut self.devices[index];
                device.token_hash_b64 = token_hash_b64;
                device.exp = Some(exp);
                device.last_seen = Some(now);
                (device.id.clone(), device.name.clone(), true, device.caps)
            }
            None => {
                let device_id = uuid::Uuid::new_v4().to_string();
                let name = clean_name(device_name);
                self.devices.push(PersistedDevice {
                    id: device_id.clone(),
                    name: name.clone(),
                    token_hash_b64,
                    public_key_b64: String::new(),
                    paired_at: now,
                    caps,
                    exp: Some(exp),
                    last_seen: Some(now),
                    control_computer: false,
                });
                (device_id, name, false, caps)
            }
        };
        let _ = self.persist();
        DEVICE_VIEW.write().insert(device_id.clone(), (caps, Some(exp)));
        if !relinked {
            events::emit(SecurityEvent::DevicePaired { device_id: device_id.clone(), caps: caps.list() });
        }
        Ok(PhoneGrant { device_id, token, name, relinked })
    }

    /// The key a phone and this PC both know before pairing: SHA-256 of the live pairing code. `None`: no live code.
    pub fn pending_hello_key(&self) -> Option<Vec<u8>> {
        let pending = self.pending.as_ref().filter(|p| now_secs() <= p.expires_at)?;
        Some(Sha256::digest(pending.code.as_bytes()).to_vec())
    }

    /// The key for a paired phone: the SHA-256 of its token's bytes, which is what is stored.
    pub fn device_hello_key(&self, device_id: &str) -> Option<Vec<u8>> {
        let now = now_secs();
        let device = self.devices.iter().find(|d| d.id == device_id && !device_expired(d, now))?;
        decode_b64(&device.token_hash_b64).ok()
    }

    pub fn phone_devices(&self) -> Vec<DeviceRow> {
        let now = now_secs();
        self.devices
            .iter()
            .map(|d| DeviceRow {
                id: d.id.clone(),
                name: d.name.clone(),
                paired_at: d.paired_at,
                last_seen: d.last_seen,
                expired: device_expired(d, now),
                caps: d.caps.list(),
                control_computer: d.control_computer,
            })
            .collect()
    }

    /// Saves the per-phone "can control the bot's computer" yes. `Ok(false)`: no such phone. Only the PC calls this.
    pub fn set_phone_control(&mut self, device_id: &str, on: bool) -> Result<bool, String> {
        let Some(device) = self.devices.iter_mut().find(|d| d.id == device_id) else { return Ok(false) };
        if device.control_computer != on {
            device.control_computer = on;
            self.persist()?;
        }
        Ok(true)
    }

    /// The phones that have the person's yes saved (those whose pairing is still good).
    pub fn phone_control_ids(&self) -> Vec<String> {
        let now = now_secs();
        self.devices.iter().filter(|d| d.control_computer && !device_expired(d, now)).map(|d| d.id.clone()).collect()
    }

    pub fn device_name(&self, device_id: &str) -> Option<String> {
        self.devices.iter().find(|d| d.id == device_id).map(|d| d.name.clone())
    }

    /// Signs a phone out at once: its token stops working on the next request and its open streams close.
    pub fn revoke_phone_device(&mut self, device_id: &str) -> Result<bool, String> {
        let before = self.devices.len();
        self.devices.retain(|d| d.id != device_id);
        if self.devices.len() == before {
            return Ok(false);
        }
        self.persist()?;
        DEVICE_VIEW.write().remove(device_id);
        events::emit(SecurityEvent::DeviceRevoked { device_id: device_id.to_string() });
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> MobilePairingStore {
        let dir = std::env::temp_dir().join(format!("pm-phone-pair-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        MobilePairingStore::load_or_create(dir.join("pairing.json")).unwrap()
    }

    #[test]
    fn a_code_works_once() {
        let mut s = store("once");
        let c = s.mint_phone_code("http://x".into(), Caps::all(), None).unwrap();
        let grant = s.redeem_phone_code(&c.code, "My phone").unwrap();
        assert_eq!(grant.name, "My phone");
        assert_eq!(s.redeem_phone_code(&c.code, "again"), Err(PairFail::BadCode), "the second use finds no code");
        assert_eq!(s.phone_devices().len(), 1);
    }

    #[test]
    fn a_code_runs_out_after_ten_minutes() {
        let mut s = store("ttl");
        let c = s.mint_phone_code("x".into(), Caps::all(), None).unwrap();
        assert_eq!(c.expires_at - now_secs(), CODE_TTL_SECS);
        assert_eq!(s.redeem_phone_code_at(&c.code, "p", c.expires_at + 1), Err(PairFail::Expired));
        assert_eq!(s.redeem_phone_code_at(&c.code, "p", c.expires_at), Err(PairFail::BadCode), "the code is gone once it was seen expired");
        let c2 = s.mint_phone_code("x".into(), Caps::all(), None).unwrap();
        assert!(s.redeem_phone_code_at(&c2.code, "p", c2.expires_at).is_ok(), "the last second still counts");
    }

    #[test]
    fn five_wrong_guesses_burn_the_code_even_for_the_right_answer_after() {
        let mut s = store("burn");
        let c = s.mint_phone_code("x".into(), Caps::all(), None).unwrap();
        for _ in 0..(MAX_WRONG_GUESSES - 1) {
            assert_eq!(s.redeem_phone_code("AAAAAAAA", "p"), Err(PairFail::BadCode));
        }
        assert_eq!(s.redeem_phone_code("AAAAAAAA", "p"), Err(PairFail::Burned));
        assert_eq!(s.redeem_phone_code(&c.code, "p"), Err(PairFail::BadCode), "the real code no longer works");
        assert!(s.phone_devices().is_empty());
    }

    #[test]
    fn a_new_code_replaces_the_old_one_and_the_code_is_typed_forgivingly() {
        let mut s = store("replace");
        let a = s.mint_phone_code("x".into(), Caps::all(), None).unwrap();
        let b = s.mint_phone_code("x".into(), Caps::all(), None).unwrap();
        assert_ne!(a.code, b.code);
        assert_eq!(s.redeem_phone_code(&a.code, "p"), Err(PairFail::BadCode));
        let spaced = format!("{}-{}", b.code[..4].to_lowercase(), &b.code[4..]);
        assert!(s.redeem_phone_code(&spaced, "p").is_ok(), "case, dash and spaces do not matter");
    }

    #[test]
    fn the_token_checks_out_and_only_its_hash_is_stored() {
        let mut s = store("token");
        let c = s.mint_phone_code("x".into(), Caps::all(), None).unwrap();
        let g = s.redeem_phone_code(&c.code, "p").unwrap();
        assert!(matches!(s.check_token(&g.token), TokenCheck::Valid { .. }));
        let file = std::fs::read_to_string(&s.path).unwrap();
        assert!(!file.contains(&g.token), "the token itself is never written");
        assert!(matches!(s.check_token("AAAA"), TokenCheck::Unknown));
    }

    #[test]
    fn revoking_signs_the_phone_out_at_once() {
        let mut s = store("revoke");
        let c = s.mint_phone_code("x".into(), Caps::all(), None).unwrap();
        let g = s.redeem_phone_code(&c.code, "p").unwrap();
        assert!(s.revoke_phone_device(&g.device_id).unwrap());
        assert!(matches!(s.check_token(&g.token), TokenCheck::Unknown));
        assert!(stream_caps(&g.device_id).is_none());
        assert!(!s.revoke_phone_device(&g.device_id).unwrap());
    }

    #[test]
    fn a_relink_keeps_the_device_and_changes_the_token() {
        let mut s = store("relink");
        let c = s.mint_phone_code("x".into(), Caps::none().with(Cap::Watch, true), None).unwrap();
        let first = s.redeem_phone_code(&c.code, "Ren's phone").unwrap();
        let c2 = s.mint_phone_code("y".into(), Caps::all(), Some(first.device_id.clone())).unwrap();
        let second = s.redeem_phone_code(&c2.code, "ignored name").unwrap();
        assert!(second.relinked);
        assert_eq!(second.device_id, first.device_id);
        assert_eq!(second.name, "Ren's phone");
        assert_ne!(second.token, first.token);
        assert!(matches!(s.check_token(&first.token), TokenCheck::Unknown), "the old token stops working");
        match s.check_token(&second.token) {
            TokenCheck::Valid { caps, .. } => assert!(caps.has(Cap::Watch) && !caps.has(Cap::Control), "caps stay what they were"),
            other => panic!("{other:?}"),
        }
        assert_eq!(s.phone_devices().len(), 1);
        assert!(s.mint_phone_code("y".into(), Caps::all(), Some("nobody".into())).is_err());
    }

    #[test]
    fn hello_keys_exist_only_for_a_live_code_and_a_paired_phone() {
        let mut s = store("hello");
        assert!(s.pending_hello_key().is_none());
        let c = s.mint_phone_code("x".into(), Caps::all(), None).unwrap();
        assert_eq!(s.pending_hello_key().unwrap(), Sha256::digest(c.code.as_bytes()).to_vec());
        let g = s.redeem_phone_code(&c.code, "p").unwrap();
        assert!(s.pending_hello_key().is_none(), "the used code has no key");
        let raw = decode_b64(&g.token).unwrap();
        assert_eq!(s.device_hello_key(&g.device_id).unwrap(), Sha256::digest(&raw).to_vec());
        assert!(s.device_hello_key("nobody").is_none());
    }

    #[test]
    fn a_relink_code_is_not_kept_on_disk() {
        let mut s = store("relink-disk");
        let c = s.mint_phone_code("x".into(), Caps::all(), None).unwrap();
        let g = s.redeem_phone_code(&c.code, "p").unwrap();
        let c2 = s.mint_phone_code("x".into(), Caps::all(), Some(g.device_id)).unwrap();
        let file = std::fs::read_to_string(&s.path).unwrap();
        assert!(!file.contains(&c2.code));
    }
}
