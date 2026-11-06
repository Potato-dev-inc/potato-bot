//! The wire between the app and the brain, checked against one shared file: the brain's own test
//! (`packages/potato-brain/test/rooms-wire.test.ts`) reads the same `room-wire.json`. Here the item the app writes
//! must equal the file's item, and every event in the file (written by the brain) must parse as the right event.

use serde_json::Value;

use super::fanout::inbox_item;
use super::store::{Room, RoomFrom, RoomLine, Speaker};
use crate::potato::protocol::{parse_event, with_capabilities, ErrorCode, Event, Tools};

const FIXTURE: &str = include_str!("../../../../../potato-brain/test/fixtures/room-wire.json");

fn member(kind: Speaker, id: &str, name: &str) -> RoomFrom {
    RoomFrom { kind, id: id.into(), name: name.into() }
}

#[test]
fn the_item_the_app_writes_is_the_item_the_brain_reads() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let room = Room { id: "r0123456789ab".into(), name: "Weekend".into(), bot_ids: vec!["nimbus".into(), "mochi".into()], created_ms: 1, last_ms: 1, unread: 0 };
    let you = member(Speaker::Person, "owner", "You");
    let nimbus = member(Speaker::Bot, "nimbus", "Nimbus");
    let mochi = member(Speaker::Bot, "mochi", "Mochi");
    let recent = RoomLine { id: "l0000000000a0".into(), from: mochi.clone(), text: "morning".into(), at: 1000, hop: 1, in_reply_to: Some("l0000000000a9".into()), file: None };
    let line = RoomLine { id: "l0000000000a1".into(), from: you.clone(), text: "@Nimbus hi".into(), at: 2000, hop: 0, in_reply_to: None, file: None };
    let item = inbox_item("l0000000000a1", &room, &nimbus, &[you, nimbus.clone(), mochi], &[recent], &line, Tools::Full, false);
    let item = with_capabilities(item, true);
    assert_eq!(item, fixture["item"]);
}

/// The item for the line that went unanswered in the live test: a bare "hi" from the owner (named "You") to a group
/// of two bots, no earlier lines, no computer. The brain's parser and the real brain read the same object.
#[test]
fn the_item_for_a_bare_hi_from_you_is_the_item_the_brain_reads() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let room = Room { id: "r4bb2b9763480".into(), name: "test group".into(), bot_ids: vec!["blank".into(), "blank-2".into()], created_ms: 1, last_ms: 1, unread: 0 };
    let you = member(Speaker::Person, "owner", "You");
    let blank = member(Speaker::Bot, "blank", "Blank");
    let blank2 = member(Speaker::Bot, "blank-2", "Blank 2");
    let line = RoomLine { id: "l80d6960566e2".into(), from: you.clone(), text: "hi".into(), at: 1793885003778, hop: 0, in_reply_to: None, file: None };
    let item = inbox_item("l80d6960566e2", &room, &blank2, &[you, blank, blank2.clone()], &[], &line, Tools::Full, false);
    assert_eq!(with_capabilities(item, false), fixture["personItem"]);
}

#[test]
fn the_events_the_brain_writes_are_read_by_the_app() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let events: Vec<Event> = fixture["events"].as_array().unwrap().iter().map(|e| parse_event(e).expect("every event in the file parses")).collect();
    let conversation = "app:room:r0123456789ab".to_string();
    assert_eq!(events[0], Event::Typing { conversation_id: conversation.clone(), on: true });
    assert_eq!(
        events[1],
        Event::Reply {
            conversation_id: conversation.clone(),
            in_reply_to: "l0000000000a1".into(),
            segments: vec!["hello".into(), "there".into()],
            kaomoji: Some("(^_^)".into()),
            tone: Some("genki".into()),
            note: false,
        }
    );
    assert_eq!(events[2], Event::Pass { conversation_id: conversation.clone(), in_reply_to: "l0000000000a1".into(), reason: "low_score".into() });
    assert_eq!(events[3], Event::Error { code: ErrorCode::ProviderLimit, conversation_id: Some(conversation), in_reply_to: Some("l0000000000a1".into()), steps: 0 });
}
