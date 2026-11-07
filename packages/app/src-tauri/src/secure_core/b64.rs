//! base64url (no padding) helpers. Copied unchanged from `pm-relay-protocol` (`b64`, `unb64`, `unb64_32`), which
//! Potato does not have. `KemSecret::public_b64` output depends on this exact engine, so it must not change.
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;

pub fn b64(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

pub fn unb64(text: &str) -> Option<Vec<u8>> {
    B64.decode(text.trim()).ok()
}

/// A 32-byte value from base64url (public keys).
pub fn unb64_32(text: &str) -> Option<[u8; 32]> {
    unb64(text)?.try_into().ok()
}
