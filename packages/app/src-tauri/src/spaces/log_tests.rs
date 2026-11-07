use sha2::{Digest, Sha256};

use super::identity::SpaceIdentity;
use super::ids::{BotId, ChannelId, EntryId, MemberKey, SpaceId};
use super::log::*;
use super::roster::{epoch_info, new_bot_claim, BotChannels, BotRules, OpBody, RemoveReason, Roster};
use crate::secure_core::crypto::seal_with_key;

const NOW: u64 = 10_000_000_000;
const KEY: [u8; 32] = [9; 32];
const DAY: u64 = 86_400_000;
const CH: ChannelId = ChannelId([1; 8]);
const BOT: BotId = BotId([5; 8]);

fn owner() -> SpaceIdentity { SpaceIdentity::for_test(1) }
fn sam() -> SpaceIdentity { SpaceIdentity::for_test(2) }
fn space() -> SpaceId { SpaceId([7; 16]) }

fn step(r: &mut Roster, body: OpBody) {
    let op = r.next_op(&owner(), body, NOW).unwrap();
    r.apply(&op).unwrap();
}

/// Owner (seed 1) and Sam (seed 2), epoch 1, no bots.
fn roster() -> Roster {
    let (mut r, _) = Roster::genesis(&owner(), space(), "Club", "Owner", CH, &KEY, NOW).unwrap();
    let cert = sam().certify(&space(), "Sam", NOW);
    let grant = crate::secure_core::grant::seal_key_to(cert.device.0, &cert.kem_b64, &epoch_info(&space(), 1), &KEY).unwrap();
    step(&mut r, OpBody::AddMember { cert, epoch: 1, grant });
    r
}

/// The same, with Sam's bot on the roster.
fn roster_with_bot() -> Roster {
    let mut r = roster();
    step(&mut r, new_bot_claim(&sam(), &space(), BOT, "Fishy"));
    r
}

fn hdr(who: &SpaceIdentity, kind: u8) -> EntryHeader {
    EntryHeader {
        space: space(), channel: CH, device: who.device_key(), seq: 1, lamport: 1, created_ms: NOW, epoch: 1, kind,
        refers: None, bot: None,
    }
}

fn text(who: &SpaceIdentity, t: &str) -> Entry {
    Entry::seal(who, hdr(who, KIND_TEXT), &Body::Text { text: t.into(), mentions: vec![] }, &KEY).unwrap()
}

fn bot_entry(who: &SpaceIdentity, bot: Option<BotId>, channel: ChannelId) -> Entry {
    let mut h = hdr(who, KIND_BOT);
    h.bot = bot;
    h.channel = channel;
    Entry::seal(who, h, &Body::BotText { text: "hello".into(), called_by: Some(owner().member_key()), hops: 0 }, &KEY).unwrap()
}

fn resign(who: &SpaceIdentity, e: &mut Entry) { e.sig = who.sign_as_device(ENTRY_DOMAIN, &e.signed_part()); }

#[test]
fn text_entry_round_trips() {
    let e = text(&sam(), "hi");
    assert_eq!(Entry::from_bytes(&e.to_bytes()).unwrap(), e);
    assert_eq!(e.open(&KEY).unwrap(), Body::Text { text: "hi".into(), mentions: vec![] });
}

#[test]
fn entry_layout_offsets() {
    let b = text(&sam(), "hi").to_bytes();
    assert_eq!(&b[0..4], b"PMSE");
    assert_eq!(b[4], 1);
    assert_eq!(&b[29..61], &sam().device_key().0);
    assert_eq!(b[89], 1);
    assert_eq!(&b[90..122], &[0u8; 32]);
    assert_eq!(&b[122..130], &[0u8; 8]);
    assert_eq!(HEADER_LEN, 130);
    let bot = bot_entry(&sam(), Some(BOT), CH).to_bytes();
    assert_eq!(&bot[122..130], &BOT.0);
}

#[test]
fn id_is_sha256_of_bytes() {
    let e = text(&sam(), "hi");
    let want: [u8; 32] = Sha256::digest(e.to_bytes()).into();
    assert_eq!(e.id(), EntryId(want));
}

#[test]
fn flipped_byte_in_header_fails_open() {
    let mut e = text(&sam(), "hi");
    e.header.lamport ^= 1;
    resign(&sam(), &mut e);
    assert!(e.signature_ok());
    assert_eq!(e.open(&KEY), Err(LogError("cannot open")));
}

#[test]
fn flipped_byte_breaks_signature() {
    let mut e = text(&sam(), "hi");
    e.sealed[0] ^= 1;
    assert!(!e.signature_ok());
    assert_eq!(check(&roster(), &e, NOW), Verdict::Reject("bad signature"));
}

#[test]
fn wrong_key_cannot_open() {
    assert!(text(&sam(), "hi").open(&[8; 32]).is_err());
}

#[test]
fn stranger_author_is_rejected() {
    let e = text(&SpaceIdentity::for_test(9), "hi");
    assert_eq!(check(&roster(), &e, NOW), Verdict::Reject("unknown author"));
}

#[test]
fn removed_author_old_epoch_is_ok_new_epoch_rejected() {
    let mut r = roster();
    step(&mut r, OpBody::RemoveMember { member: sam().member_key(), reason: RemoveReason::Removed });
    let grants = r.grants_for_all(2, &[8; 32]).unwrap();
    step(&mut r, OpBody::Rotate { epoch: 2, grants });
    let old = text(&sam(), "hi");
    assert_eq!(check(&r, &old, NOW), Verdict::Ok { member: sam().member_key() });
    let mut h = hdr(&sam(), KIND_TEXT);
    h.epoch = 2;
    let new = Entry::seal(&sam(), h, &Body::Text { text: "hi".into(), mentions: vec![] }, &[8; 32]).unwrap();
    assert_eq!(check(&r, &new, NOW), Verdict::Reject("removed author"));
}

#[test]
fn future_entry_is_rejected() {
    let mut h = hdr(&sam(), KIND_TEXT);
    h.created_ms = NOW + 11 * 60 * 1000;
    let e = Entry::seal(&sam(), h, &Body::Text { text: "hi".into(), mentions: vec![] }, &KEY).unwrap();
    assert_eq!(check(&roster(), &e, NOW), Verdict::Reject("from the future"));
}

#[test]
fn ninety_one_day_old_entry_is_old() {
    let mut h = hdr(&sam(), KIND_TEXT);
    h.created_ms = NOW - 91 * DAY;
    let e = Entry::seal(&sam(), h, &Body::Text { text: "hi".into(), mentions: vec![] }, &KEY).unwrap();
    assert_eq!(check(&roster(), &e, NOW), Verdict::Old);
}

#[test]
fn react_without_target_is_rejected() {
    let e = Entry::seal(&sam(), hdr(&sam(), KIND_REACT), &Body::React { code: "heart".into(), on: true }, &KEY).unwrap();
    assert_eq!(check(&roster(), &e, NOW), Verdict::Reject("missing target"));
}

#[test]
fn oversized_entry_is_refused() {
    // 3700 four-byte characters (14 800 bytes) and 20 mentions still fit one entry.
    let big = "\u{1F600}".repeat(3700);
    let mentions = vec![MemberKey([3; 32]); 20];
    let e = Entry::seal(&sam(), hdr(&sam(), KIND_TEXT), &Body::Text { text: big, mentions }, &KEY).unwrap();
    assert!(e.to_bytes().len() <= 65_536);
    assert_eq!(Entry::from_bytes(&vec![0u8; 65_537]), Err(LogError("too big")));
    // More text than the byte cap is refused when sealing.
    let too_much = "\u{1F600}".repeat(4000);
    assert!(Entry::seal(&sam(), hdr(&sam(), KIND_TEXT), &Body::Text { text: too_much, mentions: vec![] }, &KEY).is_err());
}

#[test]
fn reserved_kind_is_stored_not_shown() {
    let h = hdr(&sam(), 12);
    let (nonce, sealed) = seal_with_key(&KEY, &h.to_bytes(), &[1, 2, 3]).unwrap();
    let mut e = Entry { header: h, nonce, sealed, sig: [0u8; 64] };
    resign(&sam(), &mut e);
    assert_eq!(e.open(&KEY).unwrap(), Body::Reserved(12));
    assert_eq!(check(&roster(), &e, NOW), Verdict::Ok { member: sam().member_key() });
}

#[test]
fn kind_64_is_invalid() {
    let mut b = text(&sam(), "hi").to_bytes();
    b[89] = 64;
    assert!(Entry::from_bytes(&b).is_err());
}

#[test]
fn clean_text_rules() {
    assert_eq!(clean_text("a\u{202E}b\u{0007}\n\tc   "), "ab\n\tc");
    assert_eq!(clean_text(&"x".repeat(5000)).chars().count(), 4000);
}

#[test]
fn reaction_code_must_be_known() {
    assert!(encode_body(&Body::React { code: "fire".into(), on: true }).is_err());
    assert!(encode_body(&Body::React { code: "heart".into(), on: true }).is_ok());
}

#[test]
fn unclean_text_in_a_body_is_refused_by_the_reader() {
    let mut w = super::wire::Writer::new();
    w.u8(1).str("a\u{202E}b").u8(0);
    assert_eq!(decode_body(KIND_TEXT, &w.finish()), Err(LogError("unclean text")));
}

#[test]
fn bot_text_round_trips_and_names_its_bot() {
    let e = bot_entry(&sam(), Some(BOT), CH);
    let back = Entry::from_bytes(&e.to_bytes()).unwrap();
    assert_eq!(back.header.bot, Some(BOT));
    assert_eq!(
        back.open(&KEY).unwrap(),
        Body::BotText { text: "hello".into(), called_by: Some(owner().member_key()), hops: 0 }
    );
    assert_eq!(check(&roster_with_bot(), &e, NOW), Verdict::Ok { member: sam().member_key() });
}

#[test]
fn bot_text_from_a_bot_not_on_the_roster_is_rejected() {
    let e = bot_entry(&sam(), Some(BOT), CH);
    assert_eq!(check(&roster(), &e, NOW), Verdict::Reject("bot not on the roster"));
    let e = bot_entry(&sam(), None, CH);
    assert_eq!(check(&roster_with_bot(), &e, NOW), Verdict::Reject("missing bot"));
}

#[test]
fn bot_text_signed_by_another_members_device_is_rejected() {
    let e = bot_entry(&owner(), Some(BOT), CH);
    assert_eq!(check(&roster_with_bot(), &e, NOW), Verdict::Reject("bot is not yours"));
}

#[test]
fn bot_text_from_a_bot_removed_later_is_still_accepted() {
    let mut r = roster_with_bot();
    step(&mut r, OpBody::RemoveBot { bot: BOT });
    let e = bot_entry(&sam(), Some(BOT), CH);
    assert_eq!(check(&r, &e, NOW), Verdict::Ok { member: sam().member_key() });
    // Someone else still cannot use the removed bot's id.
    assert_eq!(check(&r, &bot_entry(&owner(), Some(BOT), CH), NOW), Verdict::Reject("bot is not yours"));
}

#[test]
fn bot_text_in_a_channel_closed_later_is_still_accepted() {
    let mut r = roster_with_bot();
    step(&mut r, OpBody::AddChannel { channel: ChannelId([2; 8]), name: "ideas".into() });
    step(&mut r, OpBody::ArchiveChannel { channel: ChannelId([2; 8]) });
    let e = bot_entry(&sam(), Some(BOT), ChannelId([2; 8]));
    assert_eq!(check(&r, &e, NOW), Verdict::Ok { member: sam().member_key() });
}

#[test]
fn bot_rules_changed_later_do_not_reject_old_bot_text() {
    let mut r = roster_with_bot();
    step(&mut r, OpBody::AddChannel { channel: ChannelId([2; 8]), name: "ideas".into() });
    step(&mut r, OpBody::SetBotRules { rules: BotRules { per_person: 1, channels: BotChannels::Only(vec![ChannelId([2; 8])]) } });
    assert_eq!(check(&r, &bot_entry(&sam(), Some(BOT), CH), NOW), Verdict::Ok { member: sam().member_key() });
    // The send-time filter is the roster's.
    assert!(!r.bot_may_speak(&BOT, &CH));
    assert!(r.bot_may_speak(&BOT, &ChannelId([2; 8])));
}

#[test]
fn a_bot_id_on_a_text_entry_is_rejected() {
    let mut h = hdr(&sam(), KIND_TEXT);
    h.bot = Some(BOT);
    let e = Entry::seal(&sam(), h, &Body::Text { text: "hi".into(), mentions: vec![] }, &KEY).unwrap();
    assert_eq!(check(&roster_with_bot(), &e, NOW), Verdict::Reject("bot id on a non-bot entry"));
}

#[test]
fn a_bot_answering_a_bot_is_off() {
    assert!(encode_body(&Body::BotText { text: "x".into(), called_by: None, hops: 1 }).is_err());
    let mut w = super::wire::Writer::new();
    w.u8(1).str("x").fixed(&[0u8; 32]).u8(1);
    assert_eq!(decode_body(KIND_BOT, &w.finish()), Err(LogError("a bot may not answer a bot")));
}

#[test]
fn the_message_id_survives_a_round_trip_for_diary_lines() {
    let e = bot_entry(&sam(), Some(BOT), CH);
    assert_eq!(Entry::from_bytes(&e.to_bytes()).unwrap().id(), e.id());
}

#[test]
fn removed_member_cannot_date_an_entry_after_the_removal() {
    let mut r = roster();
    step(&mut r, OpBody::RemoveMember { member: sam().member_key(), reason: RemoveReason::Removed });
    let later = NOW + 2 * DAY;
    // Old epoch (1), so the epoch rule alone lets it through; the removal time must stop it.
    let mut h = hdr(&sam(), KIND_TEXT);
    h.created_ms = NOW + DAY;
    let forged = Entry::seal(&sam(), h, &Body::Text { text: "hi".into(), mentions: vec![] }, &KEY).unwrap();
    assert_eq!(check(&r, &forged, later), Verdict::Reject("after removal"));
    // Before the removal, or within the clock skew of it, is fine.
    let mut h = hdr(&sam(), KIND_TEXT);
    h.created_ms = NOW - DAY;
    let before = Entry::seal(&sam(), h, &Body::Text { text: "hi".into(), mentions: vec![] }, &KEY).unwrap();
    assert_eq!(check(&r, &before, later), Verdict::Ok { member: sam().member_key() });
    let mut h = hdr(&sam(), KIND_TEXT);
    h.created_ms = NOW + FUTURE_SKEW_MS;
    let skewed = Entry::seal(&sam(), h, &Body::Text { text: "hi".into(), mentions: vec![] }, &KEY).unwrap();
    assert_eq!(check(&r, &skewed, later), Verdict::Ok { member: sam().member_key() });
}

#[test]
fn bot_claim_and_drop_round_trip_carry_no_header_bot_and_pass_the_check() {
    let claim = Body::BotClaim { bot: BOT, name: "Sam的fish".into(), sig: [7u8; 64] };
    let e = Entry::seal(&sam(), hdr(&sam(), KIND_BOT_CLAIM), &claim, &KEY).unwrap();
    let back = Entry::from_bytes(&e.to_bytes()).unwrap();
    assert_eq!(back.open(&KEY).unwrap(), claim);
    assert_eq!(check(&roster(), &e, NOW), Verdict::Ok { member: sam().member_key() });
    let drop = Body::BotDrop { bot: BOT };
    let d = Entry::seal(&sam(), hdr(&sam(), KIND_BOT_DROP), &drop, &KEY).unwrap();
    assert_eq!(Entry::from_bytes(&d.to_bytes()).unwrap().open(&KEY).unwrap(), drop);
    // an empty or too long name does not seal; a bot id in the header of a claim is refused
    assert!(Entry::seal(&sam(), hdr(&sam(), KIND_BOT_CLAIM), &Body::BotClaim { bot: BOT, name: String::new(), sig: [0; 64] }, &KEY).is_err());
    assert!(Entry::seal(&sam(), hdr(&sam(), KIND_BOT_CLAIM), &Body::BotClaim { bot: BOT, name: "x".repeat(41), sig: [0; 64] }, &KEY).is_err());
    let mut h = hdr(&sam(), KIND_BOT_CLAIM);
    h.bot = Some(BOT);
    let with_bot = Entry::seal(&sam(), h, &claim, &KEY).unwrap();
    assert_eq!(check(&roster(), &with_bot, NOW), Verdict::Reject("bot id on a non-bot entry"));
}
