//! Groups in the engine: the person's line fans out to every member bot, a bot's reply is saved to the room and goes
//! to the others, and the hop limit, the hold, the day's limit, a note and a crash each behave. Fakes only.

use serde_json::{json, Value};

use super::hold::{self, Hold};
use super::protocol::parse_event;
use super::rooms::fanout::{MAX_BOT_HOPS, NOTE_HOP};
use super::rooms::store::{self as rstore, Room, RoomLine, Speaker};
use super::service_tests::Rig;
use super::store::{self, BrainKind, Settings};

const ROOM: &str = "r0123456789ab";

/// Two bots, nimbus and mochi, in one group. A third bot, kiwi, exists but is not in it.
fn pair(name: &str) -> Rig {
    let rig = Rig::new(name);
    store::add_bot(&rig.root, "mochi", "potato", 2).unwrap();
    store::add_bot(&rig.root, "kiwi", "fish", 3).unwrap();
    rstore::save_rooms(&rig.root, &[Room { id: ROOM.into(), name: "Weekend".into(), bot_ids: vec!["nimbus".into(), "mochi".into()], created_ms: 1, last_ms: 1, unread: 0, ..Default::default() }]).unwrap();
    rig
}

fn conversation() -> String {
    store::app_room_conversation(ROOM)
}

fn post(rig: &Rig, bot: &str, event: Value) {
    rig.svc.ingest(bot, vec![parse_event(&event).expect("the event parses")]);
}

fn reply(rig: &Rig, bot: &str, to: &str, text: &str) {
    post(rig, bot, json!({"v":1,"type":"reply","conversationId":conversation(),"inReplyTo":to,"segments":[text]}));
}

/// The room items a bot's brain would fetch now.
fn items(rig: &Rig, bot: &str) -> Vec<Value> {
    let body = rig.svc.inbox_poll(bot, 0, 0);
    body["items"].as_array().cloned().unwrap_or_default().into_iter().filter(|i| i["type"] == "room").collect()
}

fn lines(rig: &Rig) -> Vec<RoomLine> {
    rstore::read_lines(&rig.root, ROOM, 500)
}

/// The lines the person would read (the app's own notes about bots left out).
fn spoken(rig: &Rig) -> Vec<RoomLine> {
    lines(rig).into_iter().filter(|l| l.hop != NOTE_HOP).collect()
}

#[test]
fn send_room_saves_and_fans_out() {
    let rig = pair("g7_send");
    let line = rig.svc.send_room(ROOM, "  @everyone hi  ").unwrap();
    assert_eq!((line.hop, line.from.kind, line.text.as_str()), (0, Speaker::Person, "@everyone hi"));
    // one line saved
    let saved = rig.svc.room_lines(ROOM, 50).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].id, line.id);
    assert_eq!(saved[0].from.id, "owner");
    // both brains were asked for, and each has one room item for that line
    assert_eq!(rig.spawned(), 2);
    for bot in ["nimbus", "mochi"] {
        let got = items(&rig, bot);
        assert_eq!(got.len(), 1, "{bot}");
        assert_eq!(got[0]["type"], "room");
        assert_eq!(got[0]["id"], line.id.as_str());
        assert_eq!(got[0]["conversationId"], conversation());
        assert_eq!(got[0]["me"]["id"], bot);
        assert_eq!(got[0]["roster"][0]["kind"], "person");
        assert_eq!(got[0]["roster"].as_array().unwrap().len(), 3);
        assert_eq!(got[0]["line"]["text"], "@everyone hi");
        assert_eq!(got[0]["tools"], "full");
        assert_eq!(got[0]["sandboxOnly"], false);
        assert_eq!(got[0]["capabilities"]["sandbox"], false, "room items carry capabilities as a send does");
        assert_eq!(got[0]["recent"].as_array().unwrap().len(), 0);
    }
    // nothing was written into either bot's own thread
    assert!(rig.svc.messages("nimbus", 50).unwrap().is_empty());
    assert!(rig.svc.messages("mochi", 50).unwrap().is_empty());
}

#[test]
fn send_room_refuses_what_a_send_refuses() {
    let rig = pair("g7_refuse");
    assert_eq!(rig.svc.send_room(ROOM, "   ").unwrap_err().code, "POTATO_EMPTY");
    assert_eq!(rig.svc.send_room("r9999999999ab", "hi").unwrap_err().code, "POTATO_ROOM_GONE");
    assert_eq!(rig.svc.send_room("../x", "hi").unwrap_err().code, "POTATO_ROOM_ID");
    rig.brains.set_enabled(false);
    assert_eq!(rig.svc.send_room(ROOM, "hi").unwrap_err().code, "POTATO_OFF");
    assert!(lines(&rig).is_empty());
}

#[test]
fn room_reply_saved_with_hop_and_fanned() {
    let rig = pair("g7_reply");
    let first = rig.svc.send_room(ROOM, "hi both").unwrap();
    reply(&rig, "nimbus", &first.id, "hello from nimbus");
    let all = spoken(&rig);
    assert_eq!(all.len(), 2);
    let r = &all[1];
    assert_eq!((r.hop, r.from.kind, r.from.id.as_str(), r.text.as_str()), (1, Speaker::Bot, "nimbus", "hello from nimbus"));
    assert_eq!(r.in_reply_to.as_deref(), Some(first.id.as_str()));
    // mochi got the person's line and then nimbus's; nimbus did not get its own
    let mochi = items(&rig, "mochi");
    assert_eq!(mochi.len(), 2);
    assert_eq!(mochi[1]["line"]["id"], r.id.as_str());
    assert_eq!(mochi[1]["line"]["hop"], 1);
    assert_eq!(mochi[1]["recent"][0]["id"], first.id.as_str());
    assert_eq!(items(&rig, "nimbus").len(), 1, "a bot never gets its own line");
    // nothing went into a bot's own thread
    assert!(rig.svc.messages("nimbus", 50).unwrap().is_empty());
}

#[test]
fn room_reply_chain_stops_at_hop_3() {
    let rig = pair("g7_chain");
    let l0 = rig.svc.send_room(ROOM, "talk to each other").unwrap();
    reply(&rig, "nimbus", &l0.id, "one");
    let l1 = spoken(&rig)[1].clone();
    reply(&rig, "mochi", &l1.id, "two");
    let l2 = spoken(&rig)[2].clone();
    assert_eq!(l2.hop, 2);
    reply(&rig, "nimbus", &l2.id, "three");
    let all = spoken(&rig);
    assert_eq!(all.len(), 4);
    assert_eq!(all[3].hop, MAX_BOT_HOPS, "the hop-3 line is saved");
    // but not passed on: mochi has the person's line and nimbus's first, nimbus has the person's and mochi's
    let ids = |bot: &str| items(&rig, bot).into_iter().map(|i| i["line"]["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(ids("mochi"), vec![l0.id.clone(), l1.id.clone()]);
    assert_eq!(ids("nimbus"), vec![l0.id.clone(), l2.id.clone()]);
    // and an answer to the hop-3 line, which nobody was given, is ignored
    reply(&rig, "mochi", &all[3].id, "four");
    assert_eq!(spoken(&rig).len(), 4);
}

#[test]
fn a_reply_to_a_line_the_bot_was_not_given_or_already_answered_is_ignored() {
    let rig = pair("g7_stray");
    let l0 = rig.svc.send_room(ROOM, "hi").unwrap();
    reply(&rig, "nimbus", "l00000000dead", "no such line");
    assert_eq!(spoken(&rig).len(), 1);
    reply(&rig, "nimbus", &l0.id, "first");
    reply(&rig, "nimbus", &l0.id, "second answer to the same line");
    assert_eq!(spoken(&rig).len(), 2, "one answer per line, so a repeat cannot restart the hop count");
}

#[test]
fn an_event_from_a_bot_outside_the_group_is_refused() {
    let rig = pair("g7_outsider");
    let l0 = rig.svc.send_room(ROOM, "hi").unwrap();
    // kiwi is a bot, but not a member
    reply(&rig, "kiwi", &l0.id, "let me in");
    post(&rig, "kiwi", json!({"v":1,"type":"pass","conversationId":conversation(),"inReplyTo":l0.id,"reason":"low_score"}));
    post(&rig, "kiwi", json!({"v":1,"type":"typing","conversationId":conversation(),"on":true}));
    assert_eq!(spoken(&rig).len(), 1);
    assert!(rig.svc.room_typing(ROOM).is_empty());
    assert!(items(&rig, "mochi").len() == 1, "nothing was passed on");
    // a member's pass for a conversation that is not a group changes nothing and does not panic
    post(&rig, "nimbus", json!({"v":1,"type":"pass","conversationId":"app:dm:nimbus","inReplyTo":"x","reason":"low_score"}));
}

#[test]
fn a_note_is_saved_to_the_room_but_not_passed_on() {
    let rig = pair("g7_note");
    let l0 = rig.svc.send_room(ROOM, "do the long thing").unwrap();
    post(&rig, "nimbus", json!({"v":1,"type":"reply","conversationId":conversation(),"inReplyTo":l0.id,"segments":["working on it"],"note":true}));
    let all = spoken(&rig);
    assert_eq!(all.len(), 2, "the note is in the room");
    assert_eq!(all[1].text, "working on it");
    assert_eq!(items(&rig, "mochi").len(), 1, "but mochi is not given it");
    // the turn is still running: the final reply is accepted and does go on
    reply(&rig, "nimbus", &l0.id, "done");
    assert_eq!(spoken(&rig).len(), 3);
    assert_eq!(items(&rig, "mochi").len(), 2);
}

#[test]
fn a_pass_ends_the_turn_and_saves_nothing() {
    let rig = pair("g7_pass");
    let l0 = rig.svc.send_room(ROOM, "anyone?").unwrap();
    post(&rig, "mochi", json!({"v":1,"type":"typing","conversationId":conversation(),"on":true}));
    assert_eq!(rig.svc.room_typing(ROOM), vec!["Mochi".to_string()]);
    post(&rig, "mochi", json!({"v":1,"type":"pass","conversationId":conversation(),"inReplyTo":l0.id,"reason":"not_first_responder"}));
    assert!(rig.svc.room_typing(ROOM).is_empty());
    assert_eq!(spoken(&rig).len(), 1);
    // the line is answered: a late reply to it is ignored
    reply(&rig, "mochi", &l0.id, "too late");
    assert_eq!(spoken(&rig).len(), 1);
}

#[test]
fn held_skips_all() {
    let rig = pair("g7_held");
    let l0 = rig.svc.send_room(ROOM, "hello").unwrap();
    hold::save(&rig.root, &Hold { held: true, ..Hold::default() }).unwrap();
    // the person cannot send while bots are paused
    assert_eq!(rig.svc.send_room(ROOM, "again").unwrap_err().code, "POTATO_PAUSED");
    // a reply that was already on its way is saved, but goes to nobody
    reply(&rig, "nimbus", &l0.id, "late but saved");
    assert_eq!(spoken(&rig).len(), 2);
    assert_eq!(items(&rig, "mochi").len(), 1);
}

#[test]
fn daily_limit_skipped() {
    let rig = pair("g7_limit");
    rig.svc.set_settings(Settings { brain: BrainKind::Deepseek, daily_limit_cents: 50, ..Settings::default() }).unwrap();
    // mochi has used 55 cents today; nimbus nothing
    rig.svc.ingest("mochi", vec![parse_event(&json!({"v":1,"type":"usage","model":"deepseek-flash","promptTokens":0,"completionTokens":500_000,"cacheHitTokens":0,"cacheMissTokens":0})).unwrap()]);
    let l0 = rig.svc.send_room(ROOM, "who is here").unwrap();
    assert_eq!(items(&rig, "nimbus").len(), 1, "the bot under its limit still gets the line");
    assert!(items(&rig, "mochi").is_empty(), "the bot over its limit does not");
    // the person is told why, in the room
    let notes: Vec<RoomLine> = lines(&rig).into_iter().filter(|l| l.hop == NOTE_HOP).collect();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].from.id, "mochi");
    assert!(notes[0].text.contains("used today's limit"), "{}", notes[0].text);
    // the others carry on: nimbus answers and nobody else is blocked
    reply(&rig, "nimbus", &l0.id, "just me");
    assert_eq!(spoken(&rig).len(), 2);
    assert!(items(&rig, "mochi").is_empty());
    // the notes never reach a brain as earlier talk
    let l1 = rig.svc.send_room(ROOM, "again").unwrap();
    let again = items(&rig, "nimbus");
    let last = again.iter().find(|i| i["line"]["id"] == l1.id.as_str()).unwrap();
    assert!(last["recent"].as_array().unwrap().iter().all(|l| l["hop"] != json!(NOTE_HOP)));
}

#[test]
fn usage_is_still_charged_to_the_bot_that_made_it() {
    let rig = pair("g7_usage");
    rig.svc.send_room(ROOM, "hi").unwrap();
    rig.svc.ingest("nimbus", vec![parse_event(&json!({"v":1,"type":"usage","model":"deepseek-flash","promptTokens":0,"completionTokens":9_091,"cacheHitTokens":0,"cacheMissTokens":0})).unwrap()]);
    let report = rig.svc.usage_report().unwrap();
    let cost_of = |bot: &str| report.bots.iter().find(|b| b.id == bot).map(|b| b.today.cost_nano).unwrap_or(0);
    assert!(cost_of("nimbus") > 0, "nimbus was charged");
    assert_eq!(cost_of("mochi"), 0, "mochi was not");
}

#[test]
fn a_crashed_bot_leaves_a_note_and_the_room_goes_on() {
    let rig = pair("g7_crash");
    let l0 = rig.svc.send_room(ROOM, "hello").unwrap();
    // nimbus fetched the line (so it is not simply re-queued), typed, then died
    let _ = items(&rig, "nimbus");
    post(&rig, "nimbus", json!({"v":1,"type":"typing","conversationId":conversation(),"on":true}));
    assert_eq!(rig.svc.room_typing(ROOM), vec!["Nimbus".to_string()]);
    rig.child(0).exit_with(1);
    rig.svc.tick();
    assert!(rig.svc.room_typing(ROOM).is_empty(), "nobody is shown as typing");
    let notes: Vec<RoomLine> = lines(&rig).into_iter().filter(|l| l.hop == NOTE_HOP).collect();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].text.contains("brain stopped before it answered"), "{}", notes[0].text);
    // mochi is unaffected
    reply(&rig, "mochi", &l0.id, "I am here");
    assert_eq!(spoken(&rig).len(), 2);
    // nothing was written into nimbus's own thread for the room
    assert!(rig.svc.messages("nimbus", 50).unwrap().is_empty());
}

#[test]
fn an_error_on_a_room_turn_is_a_note_in_the_room_and_ends_the_turn() {
    let rig = pair("g7_error");
    let l0 = rig.svc.send_room(ROOM, "hi").unwrap();
    post(&rig, "mochi", json!({"v":1,"type":"error","code":"PROVIDER_AUTH","conversationId":conversation(),"inReplyTo":l0.id}));
    let notes: Vec<RoomLine> = lines(&rig).into_iter().filter(|l| l.hop == NOTE_HOP).collect();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].from.id, "mochi");
    assert!(rig.svc.messages("mochi", 50).unwrap().is_empty(), "not in the bot's own thread");
    // the turn is over: a reply to that line is ignored
    reply(&rig, "mochi", &l0.id, "late");
    assert_eq!(spoken(&rig).len(), 1);
}

/// The live bug: a "hi" with no session running must start the brains and be waiting in each inbox, in the shape the
/// brain's parser reads (`personItem` in room-wire.json is the same shape).
#[test]
fn a_hi_with_no_live_session_starts_the_bots_and_reaches_their_inboxes() {
    let rig = pair("g7_hi_cold");
    assert_eq!(rig.spawned(), 0, "nothing is running yet");
    let line = rig.svc.send_room(ROOM, "hi").unwrap();
    assert_eq!(rig.spawned(), 2, "the keeper started both brains");
    let fixture: Value = serde_json::from_str(include_str!("../../../../potato-brain/test/fixtures/room-wire.json")).unwrap();
    let want = &fixture["personItem"];
    for bot in ["nimbus", "mochi"] {
        let got = items(&rig, bot);
        assert_eq!(got.len(), 1, "{bot} has the line in its inbox");
        let item = &got[0];
        assert_eq!(item["id"], line.id.as_str());
        assert_eq!(item["line"]["from"], want["line"]["from"], "the owner is named You with the id owner");
        assert_eq!(item["line"]["inReplyTo"], Value::Null);
        assert_eq!(item["roster"][0], want["roster"][0]);
        assert_eq!(item["roster"].as_array().unwrap().len(), 3);
        assert_eq!(item["me"]["kind"], "bot");
        // every field the brain's parser requires is there with the type it needs
        for key in ["v", "type", "id", "roomId", "roomName", "conversationId", "me", "roster", "recent", "line", "tools", "sandboxOnly", "capabilities"] {
            assert!(item.get(key).is_some(), "{bot}: missing {key}");
        }
    }
}

/// A bot that cannot start leaves a line in the group, so the person is never left with silence.
#[test]
fn a_bot_that_cannot_start_leaves_a_room_note() {
    let rig = pair("g7_nostart");
    rig.launcher.fail_spawn.store(true, std::sync::atomic::Ordering::SeqCst);
    rig.svc.send_room(ROOM, "hi").unwrap();
    for _ in 0..8 {
        rig.advance(10_000);
        rig.svc.tick();
    }
    let notes: Vec<RoomLine> = lines(&rig).into_iter().filter(|l| l.hop == NOTE_HOP).collect();
    assert!(notes.iter().any(|n| n.from.id == "nimbus" && n.text.contains("brain stopped")), "{notes:?}");
    assert!(notes.iter().any(|n| n.from.id == "mochi" && n.text.contains("brain stopped")), "{notes:?}");
}

// ---- hand_off: a task from one bot to another, as a card ------------------------------------------------------------------

fn call_as(rig: &Rig, bot: &str, conversation: &str, id: &str, tool: &str, args: Value) -> Value {
    rig.svc.set_spawner(std::sync::Arc::new(|job| job()));
    rig.svc.ingest(bot, vec![parse_event(&json!({"v":1,"type":"sandbox_call","callId":id,"conversationId":conversation,"tool":tool,"args":args,"image":false})).unwrap()]);
    let body = rig.svc.inbox_poll(bot, 0, 0);
    body["items"].as_array().cloned().unwrap_or_default().into_iter().find(|i| i["type"] == "sandbox_result" && i["callId"] == id).unwrap_or_else(|| panic!("no answer to {id}"))
}

fn code_of(item: &Value) -> String {
    item["error"]["code"].as_str().unwrap_or("").to_string()
}

fn hand_off(rig: &Rig, bot: &str, id: &str, to: &str, task: &str, in_reply_to: &str) -> Value {
    call_as(rig, bot, &conversation(), id, "hand_off", json!({"to": to, "task": task, "inReplyTo": in_reply_to}))
}

fn handoffs(rig: &Rig) -> Vec<RoomLine> {
    lines(rig).into_iter().filter(|l| l.handoff.is_some()).collect()
}

#[test]
fn a_hand_off_saves_a_card_and_wakes_only_the_bot_it_names() {
    let rig = pair("g_handoff");
    let mut rooms = rstore::load_rooms(&rig.root).unwrap();
    rooms[0].bot_ids.push("kiwi".into());
    rstore::save_rooms(&rig.root, &rooms).unwrap();
    let first = rig.svc.send_room(ROOM, "check the weather and plan the trip").unwrap();
    let answer = hand_off(&rig, "nimbus", "h-1", "Mochi", "  look up the weather  ", &first.id);
    assert_eq!(answer["ok"], json!(true), "{answer:?}");
    let cards = handoffs(&rig);
    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    assert_eq!((card.hop, card.from.id.as_str(), card.text.as_str()), (1, "nimbus", "@Mochi look up the weather"));
    assert_eq!(card.in_reply_to.as_deref(), Some(first.id.as_str()));
    let h = card.handoff.as_ref().unwrap();
    assert_eq!((h.to_id.as_str(), h.to_name.as_str(), h.task.as_str(), h.passed), ("mochi", "Mochi", "look up the weather", true));
    // mochi got the person's line and then the hand-off; kiwi only the person's line; nimbus never its own
    let mochi = items(&rig, "mochi");
    assert_eq!(mochi.len(), 2);
    assert_eq!(mochi[1]["line"]["id"], card.id.as_str());
    assert_eq!(mochi[1]["line"]["hop"], 1);
    assert_eq!(items(&rig, "kiwi").len(), 1, "a hand-off goes to the bot it names, not to the whole group");
    assert_eq!(items(&rig, "nimbus").len(), 1);
}

#[test]
fn a_chain_of_hand_offs_stops_at_the_hop_limit_and_the_card_says_so() {
    let rig = pair("g_handoff_hops");
    let l0 = rig.svc.send_room(ROOM, "go").unwrap();
    assert_eq!(hand_off(&rig, "nimbus", "h-1", "mochi", "step one", &l0.id)["ok"], json!(true));
    let h1 = handoffs(&rig)[0].clone();
    assert_eq!(hand_off(&rig, "mochi", "h-2", "nimbus", "step two", &h1.id)["ok"], json!(true));
    let h2 = handoffs(&rig)[1].clone();
    assert_eq!(h2.hop, 2);
    // the third step would be hop 3: the card is saved as not passed on, the bot is told, and nobody is woken
    let before = items(&rig, "mochi").len();
    let answer = hand_off(&rig, "nimbus", "h-3", "mochi", "step three", &h2.id);
    assert_eq!((answer["ok"].clone(), code_of(&answer)), (json!(false), "HOP_LIMIT".to_string()), "{answer:?}");
    let last = handoffs(&rig).pop().unwrap();
    assert_eq!((last.hop, last.handoff.as_ref().unwrap().passed), (MAX_BOT_HOPS, false));
    assert_eq!(items(&rig, "mochi").len(), before, "nothing is delivered past the limit");
}

#[test]
fn a_hand_off_is_refused_for_a_made_up_line_an_unknown_bot_yourself_a_switched_off_bot_and_a_private_chat() {
    let rig = pair("g_handoff_refuse");
    let l0 = rig.svc.send_room(ROOM, "go").unwrap();
    let c = |a: &Value| (a["ok"].clone(), code_of(a));
    // a line id the bot was never given
    assert_eq!(c(&hand_off(&rig, "nimbus", "r-1", "mochi", "x", "l00000000dead")), (json!(false), "NOT_ANSWERING".to_string()));
    // a name that is not in the group, and the bot itself
    assert_eq!(c(&hand_off(&rig, "nimbus", "r-2", "kiwi", "x", &l0.id)), (json!(false), "NO_SUCH_BOT".to_string()));
    assert_eq!(c(&hand_off(&rig, "nimbus", "r-3", "nimbus", "x", &l0.id)), (json!(false), "NO_SUCH_BOT".to_string()));
    assert_eq!(c(&hand_off(&rig, "nimbus", "r-4", "mochi", "   ", &l0.id)), (json!(false), "BAD_ARGS".to_string()));
    // a bot switched off in this group
    let mut rooms = rstore::load_rooms(&rig.root).unwrap();
    rooms[0].off_bots.push("mochi".into());
    rstore::save_rooms(&rig.root, &rooms).unwrap();
    assert_eq!(c(&hand_off(&rig, "nimbus", "r-5", "mochi", "x", &l0.id)), (json!(false), "BOT_OFF".to_string()));
    // the private chat has no hand_off
    let dm = call_as(&rig, "nimbus", "app:dm:nimbus", "r-6", "hand_off", json!({"to": "mochi", "task": "x", "inReplyTo": l0.id}));
    assert_eq!(c(&dm), (json!(false), "TOOL_NOT_ALLOWED".to_string()));
    assert!(handoffs(&rig).is_empty(), "no refused call left a card");
}

#[test]
fn a_hand_off_cannot_be_made_twice_from_one_answered_line() {
    let rig = pair("g_handoff_once");
    let l0 = rig.svc.send_room(ROOM, "go").unwrap();
    // the bot answers; the line is no longer one it is working on
    reply(&rig, "nimbus", &l0.id, "done");
    let answer = hand_off(&rig, "nimbus", "o-1", "mochi", "late", &l0.id);
    assert_eq!(code_of(&answer), "NOT_ANSWERING", "{answer:?}");
}
