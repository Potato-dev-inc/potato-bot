//! Who hears a line in a group, and the item they are given. Pure: no files.

use serde_json::json;

use super::fanout::*;
use super::store::{Room, RoomFrom, RoomLine, RoomMode, Speaker};
use crate::potato::protocol::Tools;

const ROOM: &str = "r0123456789ab";

fn room(bots: &[&str]) -> Room {
    Room { id: ROOM.into(), name: "Weekend".into(), bot_ids: bots.iter().map(|b| b.to_string()).collect(), created_ms: 1, last_ms: 1, unread: 0, ..Default::default() }
}

fn person_line() -> RoomLine {
    RoomLine { id: "l000000000001".into(), from: RoomFrom { kind: Speaker::Person, id: "owner".into(), name: "You".into() }, text: "hi".into(), at: 10, hop: 0, in_reply_to: None, file: None, handoff: None }
}

fn bot_line(bot: &str, hop: u8) -> RoomLine {
    RoomLine { id: format!("l00000000000{hop}"), from: RoomFrom { kind: Speaker::Bot, id: bot.into(), name: bot.into() }, text: "yo".into(), at: 20, hop, in_reply_to: Some("l000000000001".into()), file: None, handoff: None }
}

fn on(_: &str) -> bool {
    true
}

fn under(_: &str) -> bool {
    false
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn person_line_goes_to_all_bots() {
    let (go, skip) = targets(&room(&["a", "b", "c"]), &person_line(), &on, &under, false);
    assert_eq!(go, ids(&["a", "b", "c"]));
    assert!(skip.is_empty());
}

#[test]
fn bot_line_skips_sender() {
    let (go, skip) = targets(&room(&["a", "b", "c"]), &bot_line("b", 1), &on, &under, false);
    assert_eq!(go, ids(&["a", "c"]));
    assert_eq!(skip, vec![("b".to_string(), Skip::Sender)]);
}

#[test]
fn hop_limit_stops_fanout() {
    let (go, skip) = targets(&room(&["a", "b", "c"]), &bot_line("b", MAX_BOT_HOPS), &on, &under, false);
    assert!(go.is_empty());
    assert_eq!(skip, vec![("a".to_string(), Skip::HopLimit), ("b".to_string(), Skip::Sender), ("c".to_string(), Skip::HopLimit)]);
    // one hop earlier still goes on
    assert_eq!(targets(&room(&["a", "b"]), &bot_line("b", MAX_BOT_HOPS - 1), &on, &under, false).0, ids(&["a"]));
}

#[test]
fn bot_off_skipped() {
    let (go, skip) = targets(&room(&["a", "b"]), &person_line(), &|b| b != "b", &under, false);
    assert_eq!(go, ids(&["a"]));
    assert_eq!(skip, vec![("b".to_string(), Skip::BotOff)]);
}

#[test]
fn daily_limit_skipped() {
    // a is over its limit: skipped, and b still gets the line
    let (go, skip) = targets(&room(&["a", "b"]), &person_line(), &on, &|b| b == "a", false);
    assert_eq!(go, ids(&["b"]));
    assert_eq!(skip, vec![("a".to_string(), Skip::DailyLimit)]);
}

#[test]
fn held_skips_all() {
    let (go, skip) = targets(&room(&["a", "b"]), &person_line(), &on, &under, true);
    assert!(go.is_empty());
    assert_eq!(skip, vec![("a".to_string(), Skip::Held), ("b".to_string(), Skip::Held)]);
}

fn roster() -> Vec<RoomFrom> {
    vec![
        RoomFrom { kind: Speaker::Person, id: "owner".into(), name: "You".into() },
        RoomFrom { kind: Speaker::Bot, id: "a".into(), name: "Ann".into() },
        RoomFrom { kind: Speaker::Bot, id: "b".into(), name: "Bo".into() },
    ]
}

fn lines(n: usize) -> Vec<RoomLine> {
    (0..n).map(|i| RoomLine { id: format!("l{i:012x}"), text: format!("line {i}"), at: i as i64, ..person_line() }).collect()
}

#[test]
fn inbox_item_shape() {
    let r = roster();
    let item = inbox_item("l000000000001", &room(&["a", "b"]), &r[1], &r, &lines(1), &person_line(), Tools::Full, false);
    assert_eq!(item["v"], 1);
    assert_eq!(item["type"], "room", "the key is `type`, not `kind`");
    assert!(item.get("kind").is_none());
    assert_eq!(item["id"], "l000000000001");
    assert_eq!(item["roomId"], ROOM);
    assert_eq!(item["roomName"], "Weekend");
    assert_eq!(item["conversationId"], format!("app:room:{ROOM}"));
    assert_eq!(item["me"], json!({"kind":"bot","id":"a","name":"Ann"}));
    assert_eq!(item["roster"][0]["kind"], "person", "the person first");
    assert_eq!(item["roster"].as_array().unwrap().len(), 3);
    assert_eq!(item["recent"].as_array().unwrap().len(), 1);
    assert_eq!(item["line"]["inReplyTo"], json!(null));
    assert_eq!(item["line"]["at"], 10);
    assert_eq!(item["tools"], "full");
    assert_eq!(item["sandboxOnly"], false);
}

#[test]
fn inbox_item_caps_recent() {
    let r = roster();
    let all = lines(30);
    let item = inbox_item("x", &room(&["a", "b"]), &r[1], &r, &all, &person_line(), Tools::Full, false);
    let recent = item["recent"].as_array().unwrap();
    assert_eq!(recent.len(), RECENT_FOR_BRAIN);
    assert_eq!(recent[0]["text"], "line 10", "the newest 20, oldest first");
    assert_eq!(recent[19]["text"], "line 29");
}

#[test]
fn room_line_from_other_person_is_sandbox_only() {
    let r = roster();
    let item = inbox_item("x", &room(&["a", "b"]), &r[1], &r, &[], &person_line(), Tools::NoMemory, true);
    assert_eq!(item["sandboxOnly"], true);
    assert_eq!(item["tools"], "no_memory");
}

// ---- group settings ----

#[test]
fn a_bot_switched_off_in_the_group_hears_nothing_and_the_others_still_do() {
    let mut r = room(&["a", "b", "c"]);
    r.off_bots = ids(&["b"]);
    let (go, skip) = targets(&r, &person_line(), &on, &under, false);
    assert_eq!(go, ids(&["a", "c"]));
    assert_eq!(skip, vec![("b".to_string(), Skip::Muted)]);
    // a bot's line too: the switched-off bot is not woken by a hand-off either
    let (go, _) = targets(&r, &bot_line("a", 1), &on, &under, false);
    assert_eq!(go, ids(&["c"]));
}

#[test]
fn the_item_carries_the_mode_the_instructions_and_the_folder_path() {
    let r = roster();
    let mut group = room(&["a", "b"]);
    let plain = inbox_item("l000000000001", &group, &r[1], &r, &[], &person_line(), Tools::Full, false);
    assert_eq!((plain["mode"].clone(), plain["instructions"].clone(), plain["folder"].clone()), (json!("listen"), json!(""), json!(format!("/home/pm/groups/{ROOM}"))));
    group.mode = RoomMode::Mentions;
    group.instructions = "Answer in French.".into();
    group.folder = "trip".into();
    let item = inbox_item("l000000000001", &group, &r[1], &r, &[], &person_line(), Tools::Full, false);
    assert_eq!((item["mode"].clone(), item["instructions"].clone(), item["folder"].clone()), (json!("mentions"), json!("Answer in French."), json!("/home/pm/groups/trip")));
}

#[test]
fn ten_bots_all_get_the_persons_line_and_the_sender_is_still_skipped() {
    let names: Vec<String> = (0..10).map(|i| format!("b{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let r = room(&refs);
    let (go, skip) = targets(&r, &person_line(), &on, &under, false);
    assert_eq!((go.len(), skip.len()), (10, 0));
    let (go, skip) = targets(&r, &bot_line("b3", 1), &on, &under, false);
    assert_eq!((go.len(), skip), (9, vec![("b3".to_string(), Skip::Sender)]));
}
