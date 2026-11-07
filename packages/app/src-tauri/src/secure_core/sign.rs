//! Ed25519 with a domain string, so a signature for one format never verifies as another.
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

/// The signed bytes: len(domain) as one byte, the domain, then the message.
fn framed(domain: &[u8], msg: &[u8]) -> Vec<u8> {
    assert!(domain.starts_with(b"pm/") && domain.len() < 256, "bad signing domain");
    let mut v = Vec::with_capacity(1 + domain.len() + msg.len());
    v.push(domain.len() as u8);
    v.extend_from_slice(domain);
    v.extend_from_slice(msg);
    v
}

pub fn sign(key: &SigningKey, domain: &[u8], msg: &[u8]) -> [u8; 64] {
    key.sign(&framed(domain, msg)).to_bytes()
}

pub fn verify(public: &[u8; 32], domain: &[u8], msg: &[u8], sig: &[u8; 64]) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(public) else { return false };
    // Strict: refuses weak (small-order) keys and non-canonical encodings, which plain `verify` accepts.
    vk.verify_strict(&framed(domain, msg), &Signature::from_bytes(sig)).is_ok()
}

pub fn public_of(key: &SigningKey) -> [u8; 32] {
    key.verifying_key().to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key() -> SigningKey { SigningKey::from_bytes(&[3u8; 32]) }

    #[test]
    fn signature_verifies() {
        let s = sign(&key(), b"pm/test/v1", b"hello");
        assert!(verify(&public_of(&key()), b"pm/test/v1", b"hello", &s));
    }
    #[test]
    fn other_domain_fails() {
        let s = sign(&key(), b"pm/test/v1", b"hello");
        assert!(!verify(&public_of(&key()), b"pm/test/v2", b"hello", &s));
    }
    #[test]
    fn changed_message_fails() {
        let s = sign(&key(), b"pm/test/v1", b"hello");
        assert!(!verify(&public_of(&key()), b"pm/test/v1", b"hellp", &s));
    }
    #[test]
    fn bad_public_key_fails_without_panic() {
        let s = sign(&key(), b"pm/test/v1", b"hello");
        assert!(!verify(&[0xffu8; 32], b"pm/test/v1", b"hello", &s));
    }
    #[test]
    fn a_weak_public_key_cannot_sign_anything() {
        // The identity point as a public key with the all-zero signature (R = identity, S = 0) satisfies the plain
        // Ed25519 equation for every message. `verify_strict` refuses weak keys.
        let mut identity = [0u8; 32];
        identity[0] = 1;
        let mut sig = [0u8; 64];
        sig[0] = 1;
        assert!(!verify(&identity, b"pm/test/v1", b"anything at all", &sig));
    }
    #[test]
    #[should_panic(expected = "bad signing domain")]
    fn domain_must_start_with_pm() {
        let _ = sign(&key(), b"x/test", b"hello");
    }
}
