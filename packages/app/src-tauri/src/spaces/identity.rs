//! Who I am in a space: member key (the person), device key (this PC), KEM key (to receive keys), node id.
use ed25519_dalek::SigningKey;

use super::ids::{DeviceKey, MemberKey, NodeKey, SpaceId};
use super::limits::NAME_MAX_CHARS;
use super::wire::{Reader, WireError, Writer};
use crate::secure_core::crypto::KemSecret;
use crate::secure_core::sign;
use crate::spaces::keystore::KeyStore;

pub const CERT_DOMAIN: &[u8] = b"pm/spaces/device-cert/v1";

/// Extension tag 1: an optional proof that an account (for example a later sign-in) vouches for this member.
pub const EXT_PROOF: u8 = 1;
pub const EXT_MAX_BYTES: usize = 1024;
pub const PROOF_MAX_BYTES: usize = 512;

/// Extension block = repeated (tag u8, length u16, bytes). A reader skips tags it does not know, so a newer
/// writer can add fields without older readers refusing the cert. Returns None if the block is malformed.
pub fn ext_find(ext: &[u8], want: u8) -> Option<Option<Vec<u8>>> {
    let mut r = Reader::new(ext);
    let mut found = None;
    while r.remaining() > 0 {
        let tag = r.u8().ok()?;
        let n = r.u16().ok()? as usize;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n { v.push(r.u8().ok()?); }
        if tag == want && found.is_none() { found = Some(v); }
    }
    Some(found)
}

/// Builds an extension block holding only the proof (empty when `proof` is None).
pub fn ext_with_proof(proof: Option<&[u8]>) -> Vec<u8> {
    let mut w = Writer::new();
    if let Some(p) = proof {
        let p = &p[..p.len().min(PROOF_MAX_BYTES)];
        w.u8(EXT_PROOF).u16(p.len() as u16).fixed(p);
    }
    w.finish()
}

pub struct SpaceIdentity {
    member: SigningKey,
    device: SigningKey,
    kem: KemSecret,
    node: NodeKey,
}

impl SpaceIdentity {
    pub fn load(keys: &KeyStore, node: NodeKey) -> Result<Self, String> {
        Ok(Self { member: keys.member()?, device: keys.device_sign()?, kem: keys.device_kem()?, node })
    }
    pub fn from_parts(member: SigningKey, device: SigningKey, kem: KemSecret, node: NodeKey) -> Self {
        Self { member, device, kem, node }
    }
    /// Test helper: deterministic identity from one seed byte.
    #[cfg(test)]
    pub fn for_test(seed: u8) -> Self {
        Self::from_parts(
            SigningKey::from_bytes(&[seed; 32]),
            SigningKey::from_bytes(&[seed.wrapping_add(100); 32]),
            KemSecret::from_bytes([seed.wrapping_add(200); 32]),
            NodeKey([seed; 32]),
        )
    }
    pub fn member_key(&self) -> MemberKey { MemberKey(sign::public_of(&self.member)) }
    pub fn device_key(&self) -> DeviceKey { DeviceKey(sign::public_of(&self.device)) }
    pub fn node(&self) -> NodeKey { self.node }
    pub fn kem(&self) -> &KemSecret { &self.kem }
    pub fn sign_as_member(&self, domain: &[u8], msg: &[u8]) -> [u8; 64] { sign::sign(&self.member, domain, msg) }
    pub fn sign_as_device(&self, domain: &[u8], msg: &[u8]) -> [u8; 64] { sign::sign(&self.device, domain, msg) }

    pub fn certify(&self, space: &SpaceId, name: &str, now_ms: u64) -> DeviceCert {
        self.certify_with_proof(space, name, now_ms, None)
    }

    pub fn certify_with_proof(&self, space: &SpaceId, name: &str, now_ms: u64, proof: Option<&[u8]>) -> DeviceCert {
        let mut c = DeviceCert {
            space: *space,
            member: self.member_key(),
            device: self.device_key(),
            node: self.node,
            kem_b64: self.kem.public_b64(),
            name: clean_name(name),
            issued_ms: now_ms,
            ext: ext_with_proof(proof),
            sig: [0u8; 64],
        };
        c.sig = self.sign_as_member(CERT_DOMAIN, &c.signed_part());
        c
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceCert {
    pub space: SpaceId,
    pub member: MemberKey,
    pub device: DeviceKey,
    pub node: NodeKey,
    pub kem_b64: String,
    pub name: String,
    pub issued_ms: u64,
    /// Extension block (see `ext_find`). Covered by the signature, kept byte for byte.
    pub ext: Vec<u8>,
    pub sig: [u8; 64],
}

impl DeviceCert {
    fn signed_part(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(1).fixed(&self.space.0).fixed(&self.member.0).fixed(&self.device.0).fixed(&self.node.0)
            .str(&self.kem_b64).str(&self.name).u64(self.issued_ms).blob(&self.ext);
        w.finish()
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = self.signed_part();
        b.extend_from_slice(&self.sig);
        b
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(b);
        if r.u8()? != 1 { return Err(WireError("cert version")); }
        let c = DeviceCert {
            space: SpaceId(r.fixed()?),
            member: MemberKey(r.fixed()?),
            device: DeviceKey(r.fixed()?),
            node: NodeKey(r.fixed()?),
            kem_b64: r.str(64)?,
            name: r.str(160)?,
            issued_ms: r.u64()?,
            ext: r.blob(EXT_MAX_BYTES)?.to_vec(),
            sig: r.fixed()?,
        };
        r.done()?;
        if ext_find(&c.ext, EXT_PROOF).is_none() { return Err(WireError("cert ext")); }
        Ok(c)
    }
    /// The optional account proof, if the cert carries one. Other extension tags are ignored.
    pub fn proof(&self) -> Option<Vec<u8>> {
        ext_find(&self.ext, EXT_PROOF).flatten().filter(|p| p.len() <= PROOF_MAX_BYTES)
    }
    /// Signature is valid AND the name is already clean (a peer cannot smuggle control characters).
    pub fn verify(&self) -> bool {
        self.name == clean_name(&self.name)
            && sign::verify(&self.member.0, CERT_DOMAIN, &self.signed_part(), &self.sig)
    }
}

/// Trim, drop control and bidi-override characters, collapse spaces, cap at 40 chars; empty becomes "Someone".
pub fn clean_name(raw: &str) -> String {
    let filtered: String = raw
        .chars()
        .filter(|c| !c.is_control() && !matches!(*c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200B}'..='\u{200F}'))
        .collect();
    let collapsed = filtered.split_whitespace().collect::<Vec<_>>().join(" ");
    let capped: String = collapsed.chars().take(NAME_MAX_CHARS).collect();
    let capped = capped.trim().to_string();
    if capped.is_empty() { "Someone".to_string() } else { capped }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cert() -> DeviceCert { SpaceIdentity::for_test(1).certify(&SpaceId([2; 16]), "Sam", 5) }

    #[test]
    fn cert_verifies_and_round_trips() {
        let c = cert();
        assert!(c.verify());
        assert_eq!(DeviceCert::from_bytes(&c.to_bytes()).unwrap(), c);
    }
    #[test]
    fn cert_signed_by_other_member_fails() {
        let mut c = cert();
        c.member = SpaceIdentity::for_test(2).member_key();
        assert!(!c.verify());
    }
    #[test]
    fn changed_name_fails() {
        let mut c = cert();
        c.name = "Eve".into();
        assert!(!c.verify());
    }
    #[test]
    fn unclean_name_in_cert_fails() {
        let id = SpaceIdentity::for_test(1);
        let mut c = cert();
        c.name = "Sam\u{202E}".into();
        c.sig = id.sign_as_member(CERT_DOMAIN, &c.signed_part());
        assert!(!c.verify());
    }
    #[test]
    fn trailing_byte_is_rejected() {
        let mut b = cert().to_bytes();
        b.push(0);
        assert!(DeviceCert::from_bytes(&b).is_err());
    }
    #[test]
    fn clean_name_rules() {
        assert_eq!(clean_name("  Sam   Lee \n"), "Sam Lee");
        assert_eq!(clean_name("\u{202E}"), "Someone");
        assert_eq!(clean_name(&"a".repeat(60)), "a".repeat(40));
    }
    #[test]
    fn proof_is_optional_and_signed() {
        let id = SpaceIdentity::for_test(1);
        assert_eq!(cert().proof(), None);
        let c = id.certify_with_proof(&SpaceId([2; 16]), "Sam", 5, Some(b"badge"));
        assert!(c.verify());
        let back = DeviceCert::from_bytes(&c.to_bytes()).unwrap();
        assert_eq!(back.proof(), Some(b"badge".to_vec()));
        let mut tampered = c.clone();
        tampered.ext = ext_with_proof(Some(b"other"));
        assert!(!tampered.verify());
    }
    #[test]
    fn unknown_extension_tags_are_skipped_not_refused() {
        let id = SpaceIdentity::for_test(1);
        let mut c = cert();
        // a newer writer adds tag 9 (3 bytes) before the proof tag
        let mut ext = vec![9u8, 0, 3, 7, 7, 7];
        ext.extend(ext_with_proof(Some(b"p")));
        c.ext = ext;
        c.sig = id.sign_as_member(CERT_DOMAIN, &c.signed_part());
        let back = DeviceCert::from_bytes(&c.to_bytes()).unwrap();
        assert!(back.verify());
        assert_eq!(back.proof(), Some(b"p".to_vec()));
        // a malformed extension block is refused
        c.ext = vec![9u8, 0, 5, 1];
        c.sig = id.sign_as_member(CERT_DOMAIN, &c.signed_part());
        assert!(DeviceCert::from_bytes(&c.to_bytes()).is_err());
    }
}
