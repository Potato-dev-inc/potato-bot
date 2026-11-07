//! Id types. All are random bytes or public keys, shown as lowercase hex.
use crate::secure_core::crypto::random_bytes;

pub fn to_hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }

pub fn from_hex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let s = s.trim();
    if s.len() != N * 2 || !s.bytes().all(|c| c.is_ascii_hexdigit()) { return None; }
    let mut out = [0u8; N];
    for i in 0..N { out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?; }
    Some(out)
}

macro_rules! id_type {
    ($name:ident, $n:expr) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub [u8; $n]);
        impl $name {
            pub fn to_hex(&self) -> String { to_hex(&self.0) }
            pub fn from_hex(s: &str) -> Option<Self> { from_hex::<$n>(s).map(Self) }
        }
    };
}

id_type!(SpaceId, 16);
id_type!(ChannelId, 8);
id_type!(InviteId, 8);
id_type!(BotId, 8);
id_type!(EntryId, 32);
id_type!(NodeKey, 32);
id_type!(MemberKey, 32);
id_type!(DeviceKey, 32);

impl SpaceId { pub fn random() -> Self { Self(random_bytes::<16>()) } }
impl ChannelId { pub fn random() -> Self { Self(random_bytes::<8>()) } }
impl InviteId { pub fn random() -> Self { Self(random_bytes::<8>()) } }
impl BotId { pub fn random() -> Self { Self(random_bytes::<8>()) } }

impl NodeKey {
    /// First 8 hex chars, shown as XXXX-XXXX in the UI (like pairing's display_code style).
    pub fn short(&self) -> String {
        let h = self.to_hex().to_uppercase();
        format!("{}-{}", &h[0..4], &h[4..8])
    }
}

/// Keystore "team" name for a space: epoch keys and invite secrets live under it.
pub fn space_tag(space: &SpaceId) -> String { format!("space:{}", space.to_hex()) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hex_round_trip() {
        let s = SpaceId([0xab; 16]);
        assert_eq!(s.to_hex(), "ab".repeat(16));
        assert_eq!(SpaceId::from_hex(&s.to_hex()), Some(s));
    }
    #[test]
    fn bad_hex_is_none() {
        assert_eq!(ChannelId::from_hex("zz00000000000000"), None);
        assert_eq!(ChannelId::from_hex("00"), None);
    }
    #[test]
    fn random_ids_differ() {
        assert_ne!(SpaceId::random(), SpaceId::random());
    }
    #[test]
    fn space_tag_format() {
        assert_eq!(space_tag(&SpaceId([1; 16])), format!("space:{}", "01".repeat(16)));
    }
    #[test]
    fn node_short_is_two_groups_of_four() {
        assert_eq!(NodeKey([0x4f; 32]).short(), "4F4F-4F4F");
    }
}
