use super::identity::SpaceIdentity;
use super::ids::{BotId, ChannelId, NodeKey, SpaceId};
use super::roster::*;
use crate::secure_core::grant::seal_key_to;

const NOW: u64 = 1_000_000;
const KEY: [u8; 32] = [9; 32];

fn owner() -> SpaceIdentity { SpaceIdentity::for_test(1) }
fn sam() -> SpaceIdentity { SpaceIdentity::for_test(2) }
fn space() -> SpaceId { SpaceId([7; 16]) }

fn genesis() -> (Roster, ControlOp) {
    Roster::genesis(&owner(), space(), "Club", "Owner", ChannelId([1; 8]), &KEY, NOW).unwrap()
}

fn fresh() -> Roster { genesis().0 }

fn add_member_body(r: &Roster, who: &SpaceIdentity) -> OpBody {
    let cert = who.certify(&r.space, "Sam", NOW);
    let grant = seal_key_to(cert.device.0, &cert.kem_b64, &epoch_info(&r.space, r.epoch), &KEY).unwrap();
    OpBody::AddMember { cert, epoch: r.epoch, grant }
}

/// Builds an owner op with `next_op` and applies it.
fn step(r: &mut Roster, body: OpBody) -> Result<ControlOp, RosterError> {
    let op = r.next_op(&owner(), body, NOW)?;
    r.apply(&op)?;
    Ok(op)
}

fn with_sam() -> Roster {
    let mut r = fresh();
    let b = add_member_body(&r, &sam());
    step(&mut r, b).unwrap();
    r
}

fn err(r: Result<ControlOp, RosterError>) -> String { r.unwrap_err().0 }

#[test]
fn genesis_replays() {
    let (r, op) = genesis();
    let back = Roster::replay(space(), &[op]).unwrap();
    assert_eq!(back.members.len(), 1);
    assert_eq!(back.members[&owner().member_key()].role, Role::Owner);
    assert_eq!(back.channels.len(), 1);
    assert_eq!(back.channels[0].name, "general");
    assert_eq!((back.epoch, back.seq), (1, 1));
    assert_eq!(back.head, r.head);
    assert_eq!(back.kind, SpaceKind::Private);
    assert_eq!(back.bot_rules, BotRules { per_person: 1, channels: BotChannels::All });
}

#[test]
fn op_bytes_round_trip() {
    let (_, op) = genesis();
    assert_eq!(ControlOp::from_bytes(&op.to_bytes()).unwrap(), op);
}

#[test]
fn owner_can_open_epoch_one() {
    assert_eq!(fresh().open_epoch_key(&owner(), 1), Some(KEY));
}

#[test]
fn add_member_then_sam_opens_the_key() {
    let r = with_sam();
    assert_eq!(r.members.len(), 2);
    assert_eq!(r.open_epoch_key(&sam(), 1), Some(KEY));
}

fn raw_op(r: &Roster, signer: &SpaceIdentity, prev: [u8; 32], body: OpBody) -> ControlOp {
    let mut op = ControlOp {
        space: r.space, seq: r.seq + 1, prev, created_ms: NOW, body,
        signer: signer.member_key(), sig: [0u8; 64],
    };
    op.sig = signer.sign_as_member(CONTROL_DOMAIN, &op.signed_part());
    op
}

fn add_channel(name: &str, id: u8) -> OpBody { OpBody::AddChannel { channel: ChannelId([id; 8]), name: name.into() } }

#[test]
fn op_signed_by_member_is_rejected() {
    let mut r = with_sam();
    let op = raw_op(&r, &sam(), r.head, add_channel("x", 5));
    assert_eq!(r.apply(&op).unwrap_err().0, "not the owner");
}

#[test]
fn broken_chain_is_rejected() {
    let mut r = fresh();
    let op = raw_op(&r, &owner(), [1; 32], add_channel("x", 5));
    assert_eq!(r.apply(&op).unwrap_err().0, "broken chain");
}

#[test]
fn replayed_op_is_rejected() {
    let mut r = fresh();
    let op = step(&mut r, add_channel("x", 5)).unwrap();
    assert_eq!(r.apply(&op).unwrap_err().0, "wrong seq");
}

#[test]
fn flipped_signature_byte_is_rejected() {
    let mut r = fresh();
    let mut op = r.next_op(&owner(), add_channel("x", 5), NOW).unwrap();
    op.sig[0] ^= 1;
    assert_eq!(r.apply(&op).unwrap_err().0, "bad signature");
}

#[test]
fn fifty_first_member_is_rejected() {
    let mut r = fresh();
    for seed in 2..=50u8 {
        let b = add_member_body(&r, &SpaceIdentity::for_test(seed));
        step(&mut r, b).unwrap();
    }
    assert_eq!(r.members.len(), 50);
    let b = add_member_body(&r, &SpaceIdentity::for_test(51));
    assert_eq!(err(r.next_op(&owner(), b, NOW)), "space full");
}

#[test]
fn twenty_first_channel_is_rejected() {
    let mut r = fresh();
    for i in 1..=19u8 { step(&mut r, add_channel(&format!("c{i}"), 10 + i)).unwrap(); }
    assert_eq!(err(r.next_op(&owner(), add_channel("c20", 40), NOW)), "too many channels");
}

fn remove_sam() -> OpBody { OpBody::RemoveMember { member: sam().member_key(), reason: RemoveReason::Removed } }

#[test]
fn remove_requires_rotate_next() {
    let mut r = with_sam();
    step(&mut r, remove_sam()).unwrap();
    assert_eq!(err(r.next_op(&owner(), add_channel("x", 5), NOW)), "rotate first");
}

#[test]
fn rotate_must_cover_exactly_current_devices() {
    let mut r = with_sam();
    let both = r.grants_for_all(2, &[8; 32]).unwrap();
    step(&mut r, remove_sam()).unwrap();
    assert_eq!(err(r.next_op(&owner(), OpBody::Rotate { epoch: 2, grants: both }, NOW)), "grants do not match devices");
}

#[test]
fn removed_member_cannot_open_new_epoch() {
    let mut r = with_sam();
    step(&mut r, remove_sam()).unwrap();
    let grants = r.grants_for_all(2, &[8; 32]).unwrap();
    step(&mut r, OpBody::Rotate { epoch: 2, grants }).unwrap();
    assert_eq!(r.open_epoch_key(&sam(), 2), None);
    assert_eq!(r.open_epoch_key(&owner(), 2), Some([8; 32]));
    assert_eq!(r.gone[&sam().member_key()], 2);
}

#[test]
fn cannot_remove_owner() {
    let r = fresh();
    let b = OpBody::RemoveMember { member: owner().member_key(), reason: RemoveReason::Removed };
    assert_eq!(err(r.next_op(&owner(), b, NOW)), "cannot remove owner");
}

#[test]
fn cannot_archive_first_channel() {
    let r = fresh();
    let b = OpBody::ArchiveChannel { channel: r.channels[0].id };
    assert_eq!(err(r.next_op(&owner(), b, NOW)), "cannot archive the first channel");
}

#[test]
fn channel_names_are_cleaned() {
    assert_eq!(clean_channel_name("  Book Club! "), Some("book-club".into()));
    assert_eq!(clean_channel_name("---"), None);
}

#[test]
fn unknown_op_type_is_rejected() {
    let mut b = genesis().1.to_bytes();
    b[69] = 99;
    assert!(ControlOp::from_bytes(&b).is_err());
}

#[test]
fn node_in_use_is_rejected() {
    let r = fresh();
    // a new member whose node id is the owner's node
    let other = SpaceIdentity::from_parts(
        ed25519_dalek::SigningKey::from_bytes(&[60; 32]),
        ed25519_dalek::SigningKey::from_bytes(&[61; 32]),
        crate::secure_core::crypto::KemSecret::from_bytes([62; 32]),
        owner().node(),
    );
    let b = add_member_body(&r, &other);
    assert_eq!(err(r.next_op(&owner(), b, NOW)), "node in use");
}

// ---- private-space format: kind, bots, rules ----

fn bot_for(who: &SpaceIdentity, id: u8, name: &str) -> OpBody { new_bot_claim(who, &space(), BotId([id; 8]), name) }

#[test]
fn genesis_kind_is_kept_and_unknown_kind_is_refused() {
    // body = str name ("Club": 2 + 4 bytes), then the kind byte; the body starts after 70 + 4 header bytes
    let mut b = genesis().1.to_bytes();
    let at = 70 + 4 + 2 + 4;
    assert_eq!(b[at], 0);
    b[at] = 1;
    assert_eq!(ControlOp::from_bytes(&b).unwrap_err().0, "unsupported space kind");
}

#[test]
fn default_is_one_bot_per_person_in_all_channels() {
    let mut r = with_sam();
    step(&mut r, bot_for(&sam(), 1, "Sam的fish")).unwrap();
    assert_eq!(err(r.next_op(&owner(), bot_for(&sam(), 2, "Sam的two"), NOW)), "too many bots");
    // the owner is a person too
    step(&mut r, bot_for(&owner(), 3, "Owner的fish")).unwrap();
    assert!(r.bot_may_speak(&BotId([1; 8]), &r.channels[0].id));
}

#[test]
fn bot_ops_round_trip_in_bytes() {
    let mut r = with_sam();
    let rules = BotRules { per_person: 3, channels: BotChannels::Only(vec![ChannelId([1; 8])]) };
    let ops = [
        step(&mut r, bot_for(&sam(), 1, "Sam的fish")).unwrap(),
        step(&mut r, OpBody::SetBotRules { rules }).unwrap(),
        step(&mut r, OpBody::RemoveBot { bot: BotId([1; 8]) }).unwrap(),
    ];
    for op in ops {
        assert_eq!(ControlOp::from_bytes(&op.to_bytes()).unwrap(), op);
    }
}

#[test]
fn bot_claim_must_be_signed_by_its_member() {
    let r = with_sam();
    // the owner tries to attach a bot to Sam's key without Sam's signature
    let OpBody::AddBot { bot, name, sig, .. } = bot_for(&owner(), 1, "Fake") else { unreachable!() };
    let b = OpBody::AddBot { bot, member: sam().member_key(), name, sig };
    assert_eq!(err(r.next_op(&owner(), b, NOW)), "bad bot signature");
    // a stranger's claim is refused even if validly signed
    let stranger = SpaceIdentity::for_test(77);
    assert_eq!(err(r.next_op(&owner(), bot_for(&stranger, 2, "Out"), NOW)), "not a member");
}

#[test]
fn bot_name_clash_ignores_case_and_lookalikes() {
    let mut r = with_sam();
    step(&mut r, OpBody::SetBotRules { rules: BotRules { per_person: 10, channels: BotChannels::All } }).unwrap();
    step(&mut r, bot_for(&sam(), 1, "Ren的fish")).unwrap();
    let clashes = ["ren的FISH", "Ren的f1sh", "R\u{435}n的fish", "Ren 的 fish", "\u{ff32}\u{ff45}\u{ff4e}的fish", "Ren的flsh"];
    for (i, clash) in clashes.iter().enumerate() {
        assert_eq!(err(r.next_op(&owner(), bot_for(&sam(), 10 + i as u8, clash), NOW)), "bot name taken", "{clash}");
    }
    assert_eq!(bot_name_key("vvhale"), bot_name_key("Whale"));
    assert_eq!(bot_name_key("burn"), bot_name_key("bum"));
    step(&mut r, bot_for(&sam(), 2, "Ren的fash")).unwrap();
}

#[test]
fn bot_names_must_be_clean_and_not_empty() {
    let r = with_sam();
    for bad in ["", "   ", "Someone", "Sam\u{202E}x", " padded "] {
        assert_eq!(err(r.next_op(&owner(), bot_for(&sam(), 1, bad), NOW)), "bad bot name", "{bad:?}");
    }
}

#[test]
fn removing_a_bot_frees_its_name_and_its_place() {
    let mut r = with_sam();
    step(&mut r, bot_for(&sam(), 1, "Fish")).unwrap();
    assert_eq!(err(r.next_op(&owner(), bot_for(&sam(), 2, "Eel"), NOW)), "too many bots");
    step(&mut r, OpBody::SetBotRules { rules: BotRules { per_person: 2, channels: BotChannels::All } }).unwrap();
    assert_eq!(err(r.next_op(&owner(), bot_for(&sam(), 2, "fish"), NOW)), "bot name taken");
    step(&mut r, OpBody::RemoveBot { bot: BotId([1; 8]) }).unwrap();
    step(&mut r, bot_for(&sam(), 2, "fish")).unwrap();
    assert_eq!(err(r.next_op(&owner(), OpBody::RemoveBot { bot: BotId([1; 8]) }, NOW)), "no such bot");
}

#[test]
fn bot_id_cannot_be_reused() {
    let mut r = with_sam();
    step(&mut r, bot_for(&sam(), 1, "Fish")).unwrap();
    assert_eq!(err(r.next_op(&owner(), bot_for(&owner(), 1, "Other"), NOW)), "bot exists");
}

#[test]
fn rules_zero_to_ten_and_no_lowering_below_what_exists() {
    let mut r = with_sam();
    let rules = |n: u32| OpBody::SetBotRules { rules: BotRules { per_person: n, channels: BotChannels::All } };
    assert_eq!(err(r.next_op(&owner(), rules(11), NOW)), "bad bot rules");
    step(&mut r, rules(0)).unwrap();
    assert_eq!(err(r.next_op(&owner(), bot_for(&sam(), 1, "Fish"), NOW)), "too many bots");
    step(&mut r, rules(10)).unwrap();
    step(&mut r, bot_for(&sam(), 1, "Fish")).unwrap();
    step(&mut r, bot_for(&sam(), 2, "Eel")).unwrap();
    assert_eq!(err(r.next_op(&owner(), rules(1), NOW)), "members have more bots");
    step(&mut r, OpBody::RemoveBot { bot: BotId([2; 8]) }).unwrap();
    step(&mut r, rules(1)).unwrap();
}

#[test]
fn channel_rules_limit_where_a_bot_may_speak() {
    let mut r = with_sam();
    step(&mut r, add_channel("random", 5)).unwrap();
    step(&mut r, bot_for(&sam(), 1, "Fish")).unwrap();
    let (gen, rnd) = (ChannelId([1; 8]), ChannelId([5; 8]));
    assert!(r.bot_may_speak(&BotId([1; 8]), &rnd));
    let only = |v: Vec<ChannelId>| OpBody::SetBotRules { rules: BotRules { per_person: 1, channels: BotChannels::Only(v) } };
    step(&mut r, only(vec![rnd])).unwrap();
    assert!(r.bot_may_speak(&BotId([1; 8]), &rnd));
    assert!(!r.bot_may_speak(&BotId([1; 8]), &gen));
    assert!(!r.bot_may_speak(&BotId([9; 8]), &rnd), "unknown bot");
    // unsorted, duplicate or unknown channel lists are refused
    assert_eq!(err(r.next_op(&owner(), only(vec![rnd, gen]), NOW)), "bad bot rules");
    assert_eq!(err(r.next_op(&owner(), only(vec![gen, gen]), NOW)), "bad bot rules");
    assert_eq!(err(r.next_op(&owner(), only(vec![ChannelId([99; 8])]), NOW)), "no such channel");
}

#[test]
fn removing_a_member_removes_their_bots() {
    let mut r = with_sam();
    step(&mut r, bot_for(&sam(), 1, "Fish")).unwrap();
    step(&mut r, remove_sam()).unwrap();
    assert!(r.bots.is_empty());
    assert!(!r.bot_may_speak(&BotId([1; 8]), &r.channels[0].id));
}

#[test]
fn replay_reproduces_bots_and_rules() {
    let (mut r, first) = genesis();
    let mut log = vec![first];
    let bodies = [
        add_member_body(&r, &sam()),
        bot_for(&sam(), 1, "Fish"),
        OpBody::SetBotRules { rules: BotRules { per_person: 4, channels: BotChannels::All } },
    ];
    for body in bodies {
        log.push(step(&mut r, body).unwrap());
    }
    let back = Roster::replay(space(), &log).unwrap();
    assert_eq!(back.bots, r.bots);
    assert_eq!(back.bot_rules, r.bot_rules);
    assert_eq!(back.head, r.head);
}

#[test]
fn mailbox_ops_follow_their_rules() {
    let mut r = with_sam();
    let n = |b: u8| NodeKey([b; 32]);
    step(&mut r, OpBody::AddMailbox { node: n(40) }).unwrap();
    assert_eq!(err(r.next_op(&owner(), OpBody::AddMailbox { node: n(40) }, NOW)), "mailbox exists");
    assert_eq!(err(r.next_op(&owner(), OpBody::AddMailbox { node: n(1) }, NOW)), "node in use");
    step(&mut r, OpBody::AddMailbox { node: n(41) }).unwrap();
    assert_eq!(err(r.next_op(&owner(), OpBody::AddMailbox { node: n(42) }, NOW)), "too many mailboxes");
    step(&mut r, OpBody::RemoveMailbox { node: n(40) }).unwrap();
    assert_eq!(err(r.next_op(&owner(), OpBody::RemoveMailbox { node: n(40) }, NOW)), "no such mailbox");
}

#[test]
fn removed_device_is_kept_in_past() {
    let mut r = with_sam();
    let dev = sam().device_key();
    assert_eq!(r.author(&dev), Some((sam().member_key(), None)));
    step(&mut r, remove_sam()).unwrap();
    assert_eq!(r.author(&dev), Some((sam().member_key(), Some(2))));
    assert_eq!(r.author(&SpaceIdentity::for_test(9).device_key()), None);
}

#[test]
fn a_bot_id_that_was_ever_on_the_roster_is_never_given_out_again() {
    let mut r = with_sam();
    step(&mut r, bot_for(&sam(), 1, "Sam的fish")).unwrap();
    step(&mut r, OpBody::RemoveBot { bot: BotId([1; 8]) }).unwrap();
    // the same id again, even claimed by another member or by the same one, is refused: old lines stay Sam's
    assert_eq!(err(r.next_op(&owner(), bot_for(&sam(), 1, "Sam的fish"), NOW)), "bot id used");
    assert_eq!(err(r.next_op(&owner(), bot_for(&owner(), 1, "Owner的fish"), NOW)), "bot id used");
    assert_eq!(r.bot_owners.get(&BotId([1; 8])), Some(&sam().member_key()));
}
