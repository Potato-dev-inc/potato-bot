//! The "is this really my PC" answer. A phone sends a fresh nonce; the PC answers with an HMAC of it under a key
//! only the PC (and the phone it paired with) can know. A machine that is merely open at the same address cannot.

use hmac::{Hmac, Mac};
use sha2::Sha256;

const LABEL: &str = "potato-phone-hello|";

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Lowercase hex of HMAC-SHA256(key, "potato-phone-hello|" + nonce).
pub fn proof(key: &[u8], nonce: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac takes any key length");
    mac.update(LABEL.as_bytes());
    mac.update(nonce.as_bytes());
    hex(&mac.finalize().into_bytes())
}

/// A nonce is 16 to 64 letters or digits.
pub fn nonce_ok(nonce: &str) -> bool {
    (16..=64).contains(&nonce.len()) && nonce.bytes().all(|b| b.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_is_stable_and_depends_on_key_and_nonce() {
        let a = proof(b"key", "0123456789abcdef");
        assert_eq!(a, proof(b"key", "0123456789abcdef"));
        assert_ne!(a, proof(b"other", "0123456789abcdef"));
        assert_ne!(a, proof(b"key", "0123456789abcdee"));
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn matches_the_known_hmac_sha256_vector() {
        // RFC 4231 test case 2 uses the key "Jefe"; here the label is part of the message, so only the primitive is checked.
        let mut mac = Hmac::<Sha256>::new_from_slice(b"Jefe").unwrap();
        mac.update(b"what do ya want for nothing?");
        assert_eq!(hex(&mac.finalize().into_bytes()), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn nonce_rules() {
        assert!(nonce_ok("0123456789abcdef"));
        assert!(!nonce_ok("short"));
        assert!(!nonce_ok(&"a".repeat(65)));
        assert!(!nonce_ok("0123456789abcde!"));
    }
}
