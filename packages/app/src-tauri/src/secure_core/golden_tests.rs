//! Golden bytes, recorded before the move from team/crypto.rs. They must never change.
use super::crypto as c;

const KEY: [u8; 32] = [7u8; 32];
const NONCE: [u8; 24] = [9u8; 24];
const HEADER: &[u8] = b"pm/golden/header";
const PLAIN: &[u8] = b"hello spaces";

const SEALED_HEX: &str = "be72e28614cf150e9b327b4c091062982af3b79a40619de1a68d185cd326535b822a40dbd28bb5540c75bcd708deaa87578b291d3f17b54bfbeab551d085319abd720f93b571d892a95d25001671bf98e7192508b2f1e33a9c049ed3230822ab2e56a53957745b1f2e9af2dfe804aa6e846e516dfec1af6bff81fc33e5f5ef555b642f3b40409a896626c8101552291ce9f8eb4bb73ec35428762b764c3dc76a52927887e8051459ab51d22725d87144895d37756efa64af5e0be913a7e9a17d15e4d4a51031d00b59cec07be05c61b2552d576189600fcc59d5133e32cbc2690c3d38b09a2d14f06f9c25412eb6494a236990926f6aab13a5bb0d6308a75d81cae6408952748479bb461b31b74d6463d4f3198d15f562da4fb387e23e5ee5d71617460585e5ac80a8c6f4ccfeb8e5e876c775fa13a35bc48079b4513b95c34d1f04318b16946a5821cf564fca4ac5e33dda8e4c153ee5c50c657d93199129d8895d1c403793bcfab9f77fcf1ac5153da867db7795203fea9af10131dd3dd894fd79098b1a8e5cb9c1719b0471758bc1f9470538156c4ed64ffc80097dc92089ab20ef4bbd282c0599a76ff854702f686c76f14bfe3a1a384432cf7dcd8f997d391a24151380e3a5a670c80acf4b67aa3835d60c5fb45d1e9e927e3f2b346432952d77cb07077fd12d67a69ffb564f7ef1b6b24ebc65ce56ac04cb79b263ea7e6988f25c57122ae388f0f92e5c89cab1b4cb81e03981cfecdbbddce53a9066b603e53a1ed1415b64d34d74491cc208a8c3b2f2f6adc4a810b37846a1ec38bec804cde8db17572ac17c49f22f6581d15f3a69b1e53bc1dad70424f7e784b4bee99c12046da80f4fdb3b192c7df672921dfef7d683980974aa6108308302f64e964d5be2b22e1f8d0b1ade659f78984fb690c14b5728bd7276fdd5776d276cd2204c871aa21fa96860ff82588171a6c9f5c0ca82a2ebf5b5fae7a437571a336a3b348be85a678b45bb0940768bd40bbeec124c06b6272d47a4be3a3c588c297026a564ca307de1d2762f2fa0212fb2cba508557d63e5468ae5e3c8e5e5c8eba05bb44993bed57b8438ccb654c3ce8a15b2e3996607c452c91d64f23cebe4a12091b9146d04e7b0ab9a71da848663ba445a257c0d6f85bb3dc52da1025fc09af8bbb747aa5b968760e90c76b4795a293c69216853060ae86d549db15232ba11cd994e21270d1b4d191ea16430ceea33d8f29927205b152eb8fc33a88cc00569c94e972dd6ce19e4f8e63f5e73419fa38eb77547068034d1020248af661f59c414a3e99c6761780558ace296134418bd126723bd4a1a443984e739f2774eb49aa29be2450e2515eec98f473f05d6c934a8387741b3ffed7c4b98d91fad98ac3b0e7005d092f22bbde38cc238132725565e1e7f1d27800e82c7b7483a0800cb2f1b107f3b9ce307737ec76f86fd595f4f7c0d";
const HMAC_HEX: &str = "bab1f4f709b5da415f0036e5fd129ed2f86ef18066e049a44bac78472f09e00a";

fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }

#[test]
#[ignore]
fn golden_print() {
    let ct = c::seal_with_key_and_nonce(&KEY, HEADER, PLAIN, NONCE).unwrap();
    println!("SEALED_HEX={}", hex(&ct));
    println!("HMAC_HEX={}", hex(&c::hmac(b"golden-key", b"golden-message")));
}

#[test]
fn sealed_bytes_are_unchanged() {
    let ct = c::seal_with_key_and_nonce(&KEY, HEADER, PLAIN, NONCE).unwrap();
    assert_eq!(hex(&ct), SEALED_HEX);
}

#[test]
fn golden_ciphertext_still_opens() {
    let ct = c::seal_with_key_and_nonce(&KEY, HEADER, PLAIN, NONCE).unwrap();
    assert_eq!(c::open_with_key(&KEY, &NONCE, HEADER, &ct).unwrap(), PLAIN);
}

#[test]
fn hmac_is_unchanged() {
    assert_eq!(hex(&c::hmac(b"golden-key", b"golden-message")), HMAC_HEX);
    assert!(c::same_mac(&c::hmac(b"a", b"b"), &c::hmac(b"a", b"b")));
    assert!(!c::same_mac(&c::hmac(b"a", b"b"), &c::hmac(b"a", b"c")));
}

#[test]
fn pad_buckets_are_unchanged() {
    assert_eq!(c::PAD_BUCKETS, [1024, 4096, 16384]);
}

#[test]
fn hpke_round_trip_with_fixed_secret() {
    let kem = c::KemSecret::from_bytes([5u8; 32]);
    let (enc, ct) = c::hpke_seal(&kem.public_b64(), b"pm/golden/info", b"key material").unwrap();
    assert_eq!(c::hpke_open(&kem, &enc, b"pm/golden/info", &ct).unwrap(), b"key material");
    assert!(c::hpke_open(&kem, &enc, b"pm/golden/other", &ct).is_err());
}

#[test]
fn kem_public_is_stable_for_fixed_secret() {
    let a = c::KemSecret::from_bytes([5u8; 32]).public_b64();
    let b = c::KemSecret::from_bytes([5u8; 32]).public_b64();
    assert_eq!(a, b);
}
