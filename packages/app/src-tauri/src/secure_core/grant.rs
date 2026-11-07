//! Seal a 32-byte key (for example a space epoch key) to devices by their KEM public key (HPKE).
use super::crypto::{hpke_open, hpke_seal, CryptoError, KemSecret};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedKey {
    /// Which device can open it (its signing public key; used only to find the right grant).
    pub device: [u8; 32],
    pub enc: Vec<u8>,
    pub ct: Vec<u8>,
}

pub fn seal_key_to(device: [u8; 32], kem_public_b64: &str, info: &[u8], key: &[u8; 32])
    -> Result<SealedKey, CryptoError>
{
    let (enc, ct) = hpke_seal(kem_public_b64, info, key)?;
    Ok(SealedKey { device, enc, ct })
}

pub fn seal_key_to_many(recipients: &[([u8; 32], String)], info: &[u8], key: &[u8; 32])
    -> Result<Vec<SealedKey>, CryptoError>
{
    recipients.iter().map(|(d, kem)| seal_key_to(*d, kem, info, key)).collect()
}

pub fn open_sealed_key(sealed: &SealedKey, kem: &KemSecret, info: &[u8]) -> Option<[u8; 32]> {
    let plain = hpke_open(kem, &sealed.enc, info, &sealed.ct).ok()?;
    plain.as_slice().try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_key_opens_with_the_right_secret() {
        let kem = KemSecret::from_bytes([1u8; 32]);
        let s = seal_key_to([9u8; 32], &kem.public_b64(), b"pm/spaces/epoch/v1", &[4u8; 32]).unwrap();
        assert_eq!(open_sealed_key(&s, &kem, b"pm/spaces/epoch/v1"), Some([4u8; 32]));
    }
    #[test]
    fn wrong_info_does_not_open() {
        let kem = KemSecret::from_bytes([1u8; 32]);
        let s = seal_key_to([9u8; 32], &kem.public_b64(), b"pm/a", &[4u8; 32]).unwrap();
        assert_eq!(open_sealed_key(&s, &kem, b"pm/b"), None);
    }
    #[test]
    fn wrong_secret_does_not_open() {
        let kem = KemSecret::from_bytes([1u8; 32]);
        let other = KemSecret::from_bytes([2u8; 32]);
        let s = seal_key_to([9u8; 32], &kem.public_b64(), b"pm/a", &[4u8; 32]).unwrap();
        assert_eq!(open_sealed_key(&s, &other, b"pm/a"), None);
    }
    #[test]
    fn many_gives_one_grant_per_device_in_order() {
        let a = KemSecret::from_bytes([1u8; 32]);
        let b = KemSecret::from_bytes([2u8; 32]);
        let list = vec![([1u8; 32], a.public_b64()), ([2u8; 32], b.public_b64())];
        let g = seal_key_to_many(&list, b"pm/a", &[4u8; 32]).unwrap();
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].device, [1u8; 32]);
        assert_eq!(open_sealed_key(&g[1], &b, b"pm/a"), Some([4u8; 32]));
    }
}
