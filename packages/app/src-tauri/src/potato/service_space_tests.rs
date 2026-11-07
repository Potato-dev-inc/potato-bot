//! Bots in Spaces, Potato's side: the `space` item, the trust of a turn, what a space turn may ask for, and that nothing of a
//! space conversation is ever written to a thread. Fakes only.
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::hold::{self, Hold};
use super::protocol::parse_event;
use super::sandbox::lease::ScreenLease;
use super::sandbox::FakeSandbox;
use super::sandbox_calls::Port;
use super::service::{Host, SpacePort};
use super::service_tests::Rig;
use super::store::{self, BrainKind, Settings};
use crate::spaces::bot_bridge::{Delivery, Line, Refusal};
use crate::spaces::ids::{BotId, ChannelId, SpaceId};

const SPACE: [u8; 16] = [0xab; 16];
const CHANNEL: [u8; 8] = [0xcd; 8];

fn conversation() -> String {
    store::app_space_conversation(&SpaceId(SPACE).to_hex(), &ChannelId(CHANNEL).to_hex())
}

fn line(id: &str, text: &str) -> Line {
    Line { id: id.into(), from_kind: "person", from_id: "11".repeat(32), from_name: "Sam".into(), text: text.into(), at: 1_000, in_reply_to: None }
}

fn delivery(id: &str, owner: bool) -> Delivery {
    Delivery {
        id: id.into(),
        bot_local: "nimbus".into(),
        bot: BotId([7; 8]),
        bot_name: "Ren的fish".into(),
        space: SpaceId(SPACE),
        space_name: "Book club".into(),
        channel: ChannelId(CHANNEL),
        channel_name: "general".into(),
        line: line(&format!("{id}-line"), "@Ren的fish hello"),
        recent: if owner { Vec::new() } else { vec![line("l0", "earlier talk")] },
        owner_turn: owner,
        memory: true,
    }
}

#[derive(Default)]
struct Port2 {
    replies: Mutex<Vec<(String, String, String, String)>>,
    ended: Mutex<Vec<(String, String)>>,
}

impl SpacePort for Port2 {
    fn reply(&self, bot: &str, delivery: &str, conversation: &str, text: &str) {
        self.replies.lock().push((bot.into(), delivery.into(), conversation.into(), text.into()));
    }
    fn ended(&self, bot: &str, delivery: &str) {
        self.ended.lock().push((bot.into(), delivery.into()));
    }
}

fn mk(name: &str) -> (Rig, Arc<Port2>) {
    let rig = Rig::new(name);
    let port = Arc::new(Port2::default());
    rig.svc.set_space_port(Some(port.clone()));
    (rig, port)
}

fn with_computer(name: &str) -> (Rig, Arc<Port2>) {
    let (rig, port) = mk(name);
    rig.svc.set_spawner(Arc::new(|job| job()));
    rig.svc.set_sandbox_port(Some(Port { sandbox: Arc::new(FakeSandbox::new()), lease: Arc::new(Mutex::new(ScreenLease::new())) }));
    rig.svc.set_sandbox_enabled(true);
    (rig, port)
}

fn space_items(rig: &Rig) -> Vec<Value> {
    rig.inbox(0).0.into_iter().filter(|i| i["type"] == "space").collect()
}

fn post(rig: &Rig, event: Value) {
    let parsed = parse_event(&event).expect("the event parses");
    rig.svc.ingest("nimbus", vec![parsed]);
}

fn reply(id: &str, text: &str) -> Value {
    json!({"v":1,"type":"reply","conversationId":conversation(),"inReplyTo":id,"segments":[text]})
}

fn results(rig: &Rig) -> Vec<Value> {
    rig.inbox(0).0.into_iter().filter(|i| i["type"] == "sandbox_result").collect()
}

fn call(id: &str, tool: &str, args: Value) -> Value {
    json!({"v":1,"type":"sandbox_call","callId":id,"conversationId":conversation(),"tool":tool,"args":args,"image":false})
}

/// No file below the potato folder is named after a space conversation, and the bot's own thread is empty.
fn nothing_saved(rig: &Rig) {
    let thread = rig.svc.messages("nimbus", 50).unwrap();
    assert!(thread.is_empty(), "the bot's own thread is untouched: {thread:?}");
    let dir = rig.root.join("conversations");
    let mut names = Vec::new();
    fn walk(d: &std::path::Path, out: &mut Vec<String>) {
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten() {
                out.push(e.path().to_string_lossy().to_string());
                if e.path().is_dir() { walk(&e.path(), out); }
            }
        }
    }
    walk(&dir, &mut names);
    assert!(names.iter().all(|n| !n.contains("space")), "{names:?}");
    assert!(rstore_files(rig).is_empty(), "no group file either");
}

fn rstore_files(rig: &Rig) -> Vec<String> {
    let rooms = rig.root.join("rooms");
    std::fs::read_dir(rooms).map(|rd| rd.flatten().map(|e| e.path().to_string_lossy().to_string()).collect()).unwrap_or_default()
}

// ---- the item ------------------------------------------------------------------------------------------------------

#[test]
fn an_owner_line_becomes_a_space_item_with_the_wire_fields() {
    let (rig, _) = mk("sp_owner");
    rig.svc.space_deliver(&delivery("d1", true)).unwrap();
    assert_eq!(rig.spawned(), 1, "the brain is asked for");
    let items = space_items(&rig);
    assert_eq!(items.len(), 1);
    let i = &items[0];
    assert_eq!((i["v"].clone(), i["type"].clone(), i["id"].clone()), (json!(1), json!("space"), json!("d1")));
    assert_eq!(i["conversationId"], conversation().as_str());
    assert_eq!(i["channelId"], "cd".repeat(8).as_str());
    assert_eq!(i["spaceId"], "ab".repeat(16).as_str());
    assert_eq!((i["spaceName"].clone(), i["channelName"].clone()), (json!("Book club"), json!("general")));
    assert_eq!(i["me"], json!({"kind":"bot","id":"nimbus","name":"Ren的fish"}));
    assert_eq!(i["line"]["from"]["kind"], "person");
    assert_eq!((i["line"]["hop"].clone(), i["line"]["id"].clone(), i["line"]["text"].clone()), (json!(0), json!("d1-line"), json!("@Ren的fish hello")));
    assert_eq!(i["recent"].as_array().unwrap().len(), 0);
    assert_eq!((i["turn"].clone(), i["memory"].clone()), (json!("owner"), json!(true)));
    assert_eq!(i["capabilities"], json!({"sandbox": false}));
    assert!(i.get("tools").is_none() && i.get("sandboxOnly").is_none());
    nothing_saved(&rig);
}

#[test]
fn a_guest_line_carries_the_recent_talk_and_no_computer() {
    let (rig, _) = with_computer("sp_guest");
    assert!(rig.svc.sandbox_offered());
    rig.svc.space_deliver(&delivery("d1", false)).unwrap();
    let i = &space_items(&rig)[0];
    assert_eq!(i["turn"], "guest");
    assert_eq!(i["recent"].as_array().unwrap().len(), 1);
    assert_eq!(i["capabilities"], json!({"sandbox": false}), "a guest turn never has the computer");
    // an owner turn gets it when it is wired in
    let (rig2, _) = with_computer("sp_owner_pc");
    rig2.svc.space_deliver(&delivery("d2", true)).unwrap();
    assert_eq!(space_items(&rig2)[0]["capabilities"], json!({"sandbox": true}));
}

#[test]
fn memory_off_is_carried() {
    let (rig, _) = mk("sp_mem");
    let mut d = delivery("d1", false);
    d.memory = false;
    rig.svc.space_deliver(&d).unwrap();
    assert_eq!(space_items(&rig)[0]["memory"], false);
}

#[test]
fn text_is_redacted_on_the_way_in() {
    let (rig, _) = mk("sp_redact");
    let mut d = delivery("d1", false);
    d.line.text = "my key is sk-abcdefghijklmnopqrstuvwxyz0123".into();
    rig.svc.space_deliver(&d).unwrap();
    assert!(!space_items(&rig)[0]["line"]["text"].as_str().unwrap().contains("abcdefghijklmnop"));
}

// ---- the checks of a group line ------------------------------------------------------------------------------------

#[test]
fn the_checks_a_group_line_has_are_the_same() {
    // switched off
    let (rig, _) = mk("sp_off");
    rig.brains.set_enabled(false);
    assert_eq!(rig.svc.space_deliver(&delivery("d1", true)), Err(Refusal::Unavailable));
    assert_eq!(rig.spawned(), 0);
    // paused (Stop everything)
    let (rig, _) = mk("sp_held");
    hold::save(&rig.root, &Hold { held: true, ..Hold::default() }).unwrap();
    assert_eq!(rig.svc.space_deliver(&delivery("d1", true)), Err(Refusal::Unavailable));
    // a bot that does not exist
    let (rig, _) = mk("sp_nobot");
    let mut d = delivery("d1", true);
    d.bot_local = "ghost".into();
    assert_eq!(rig.svc.space_deliver(&d), Err(Refusal::Unavailable));
    // no model key: it cannot start
    let (rig, _) = mk("sp_nokey");
    rig.host.secrets.lock().clear();
    assert_eq!(rig.svc.space_deliver(&delivery("d1", true)), Err(Refusal::Unavailable));
    assert!(rig.inbox(0).0.is_empty());
}

#[test]
fn a_bot_on_my_own_agents_is_refused_and_not_offered() {
    let (rig, _) = mk("sp_agents");
    let settings = Settings { brain: BrainKind::Agents, ..store::load_settings(&rig.root).unwrap() };
    store::save_settings(&rig.root, &settings).unwrap();
    assert_eq!(rig.svc.space_deliver(&delivery("d1", true)), Err(Refusal::NotAllowed));
    assert!(rig.svc.space_candidates().is_empty(), "the Add bot list does not offer it");
    let (rig, _) = mk("sp_agents_ok");
    let names: Vec<String> = rig.svc.space_candidates().into_iter().map(|c| c.id).collect();
    assert_eq!(names, vec!["nimbus".to_string()]);
}

#[test]
fn the_days_money_limit_stops_it() {
    let (rig, _) = mk("sp_limit");
    let mut settings = store::load_settings(&rig.root).unwrap();
    settings.daily_limit_cents = 1;
    store::save_settings(&rig.root, &settings).unwrap();
    let day = rig.host.local_day(rig.host.now_ms());
    super::cost::charge_with(&rig.root, &day, "nimbus", "app", "deepseek-chat", super::cost::Tokens { prompt: 50_000_000, completion: 50_000_000, cache_hit: 0, cache_miss: 50_000_000 }, settings.daily_limit()).unwrap();
    assert_eq!(rig.svc.space_deliver(&delivery("d1", true)), Err(Refusal::Limit));
    assert!(space_items(&rig).is_empty());
}

#[test]
fn one_kind_of_turn_at_a_time_in_a_channel() {
    let (rig, _) = mk("sp_busy");
    rig.svc.space_deliver(&delivery("d1", false)).unwrap();
    // another guest line is fine, an owner line waits its turn: the brain names the conversation, not the turn
    rig.svc.space_deliver(&delivery("d2", false)).unwrap();
    assert_eq!(rig.svc.space_deliver(&delivery("d3", true)), Err(Refusal::Busy));
    assert_eq!(space_items(&rig).len(), 2);
}

// ---- the answer ----------------------------------------------------------------------------------------------------

#[test]
fn an_answer_goes_to_the_space_port_and_nowhere_else() {
    let (rig, port) = mk("sp_reply");
    rig.svc.space_deliver(&delivery("d1", false)).unwrap();
    rig.hello();
    post(&rig, json!({"v":1,"type":"typing","conversationId":conversation(),"on":true}));
    post(&rig, json!({"v":1,"type":"reply","conversationId":conversation(),"inReplyTo":"d1","segments":["one","two"],"kaomoji":"(^_^)"}));
    assert_eq!(port.replies.lock().clone(), vec![("nimbus".to_string(), "d1".to_string(), conversation(), "one\n\ntwo (^_^)".to_string())]);
    nothing_saved(&rig);
    let view = &rig.svc.state().unwrap().bots[0];
    assert!(!view.typing, "the turn is over");
    assert_eq!(view.unread, 0);
    // answered once: a repeat is ignored
    post(&rig, reply("d1", "again"));
    assert_eq!(port.replies.lock().len(), 1);
}

#[test]
fn a_mid_turn_note_is_dropped_and_the_turn_stays_open() {
    let (rig, port) = mk("sp_note");
    rig.svc.space_deliver(&delivery("d1", false)).unwrap();
    post(&rig, json!({"v":1,"type":"reply","conversationId":conversation(),"inReplyTo":"d1","segments":["working"],"note":true}));
    assert!(port.replies.lock().is_empty());
    nothing_saved(&rig);
    post(&rig, reply("d1", "done"));
    assert_eq!(port.replies.lock().len(), 1);
}

#[test]
fn an_answer_for_a_line_it_was_not_given_or_in_another_conversation_is_ignored() {
    let (rig, port) = mk("sp_stray");
    // before any delivery
    post(&rig, reply("d1", "from nowhere"));
    rig.svc.space_deliver(&delivery("d1", false)).unwrap();
    post(&rig, reply("d-made-up", "no such line"));
    let other = format!("app:space:{}:{}", "ef".repeat(16), "cd".repeat(8));
    post(&rig, json!({"v":1,"type":"reply","conversationId":other,"inReplyTo":"d1","segments":["wrong space"]}));
    assert!(port.replies.lock().is_empty());
    nothing_saved(&rig);
    post(&rig, reply("d1", "right"));
    assert_eq!(port.replies.lock().len(), 1);
}

#[test]
fn another_bot_cannot_answer_for_this_one() {
    let (rig, port) = mk("sp_other_bot");
    store::add_bot(&rig.root, "kiwi", "fish", 3).unwrap();
    rig.svc.space_deliver(&delivery("d1", false)).unwrap();
    rig.svc.ingest("kiwi", vec![parse_event(&reply("d1", "let me answer")).unwrap()]);
    assert!(port.replies.lock().is_empty());
}

#[test]
fn a_pass_and_an_error_end_the_turn_without_a_note_or_a_stuck_line() {
    let (rig, port) = mk("sp_end");
    rig.svc.space_deliver(&delivery("d1", false)).unwrap();
    post(&rig, json!({"v":1,"type":"pass","conversationId":conversation(),"inReplyTo":"d1","reason":"no_reply"}));
    assert_eq!(port.ended.lock().clone(), vec![("nimbus".to_string(), "d1".to_string())]);
    post(&rig, reply("d1", "too late"));
    assert!(port.replies.lock().is_empty());
    // an item the brain could not read comes back as an error with the same ids
    rig.svc.space_deliver(&delivery("d2", false)).unwrap();
    post(&rig, json!({"v":1,"type":"error","code":"INTERNAL","conversationId":conversation(),"inReplyTo":"d2"}));
    assert_eq!(port.ended.lock().len(), 2);
    post(&rig, reply("d2", "too late"));
    assert!(port.replies.lock().is_empty());
    nothing_saved(&rig);
    // and a new line is taken again: nothing is stuck
    rig.svc.space_deliver(&delivery("d3", true)).unwrap();
    assert_eq!(space_items(&rig).len(), 3);
}

#[test]
fn usage_is_charged_to_the_bot_as_for_a_group() {
    let (rig, _) = mk("sp_usage");
    rig.svc.space_deliver(&delivery("d1", false)).unwrap();
    rig.hello();
    post(&rig, reply("d1", "hi"));
    let before = rig.svc.state().unwrap().bots[0].used_today_nano;
    post(&rig, json!({"v":1,"type":"usage","model":"deepseek-chat","promptTokens":10000,"completionTokens":2000,"cacheHitTokens":0,"cacheMissTokens":10000}));
    let after = rig.svc.state().unwrap().bots[0].used_today_nano;
    assert!(after > before, "{before} -> {after}");
    nothing_saved(&rig);
}

// ---- trust and tools -----------------------------------------------------------------------------------------------

#[test]
fn a_guest_turn_has_no_computer_no_helper_and_no_tool_at_all() {
    let (rig, _) = with_computer("sp_guest_tools");
    rig.svc.space_deliver(&delivery("d1", false)).unwrap();
    rig.hello();
    for (i, tool) in ["computer_observe", "run_terminal", "read_file", "web_fetch", "mcp_call_tool", "scratchpad_add", "read_history", "hand_off", "request_approval"].iter().enumerate() {
        post(&rig, call(&format!("c{i}"), tool, json!({"command":"echo hi"})));
    }
    let r = results(&rig);
    assert_eq!(r.len(), 9);
    for item in &r {
        assert_eq!(item["ok"], false, "{item}");
        assert_eq!(item["error"]["code"], "TOOL_NOT_ALLOWED", "{item}");
    }
    // a helper request is dropped, and no card is written anywhere
    post(&rig, json!({"v":1,"type":"helper_request","requestId":"h1","conversationId":conversation(),"task":"do things","project":null}));
    let updates: Vec<Value> = rig.inbox(0).0.into_iter().filter(|i| i["type"] == "helper_update").collect();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["status"], "refused");
    nothing_saved(&rig);
}

#[test]
fn an_owner_turn_has_the_computers_tools_and_nothing_that_writes_a_chat() {
    let (rig, _) = with_computer("sp_owner_tools");
    rig.svc.space_deliver(&delivery("d1", true)).unwrap();
    rig.hello();
    post(&rig, call("ok1", "computer_observe", json!({})));
    for (i, tool) in ["send_file", "ask_person_for_screen", "request_approval", "ask_user", "request_connector", "scratchpad_add", "read_history", "hand_off", "message_bot", "job_start", "render_plot", "add_mcp_server"].iter().enumerate() {
        post(&rig, call(&format!("n{i}"), tool, json!({})));
    }
    let r = results(&rig);
    let by = |id: &str| r.iter().find(|x| x["callId"] == id).cloned().unwrap_or_else(|| panic!("no result for {id}"));
    assert_eq!(by("ok1")["ok"], true, "the computer is for the owner's turn");
    for i in 0..12 {
        let item = by(&format!("n{i}"));
        assert_eq!(item["ok"], false, "{item}");
        assert_eq!(item["error"]["message"], "Do this in your private chat with me.");
    }
    nothing_saved(&rig);
}

#[test]
fn a_late_tool_call_after_the_turn_ended_has_no_trust() {
    let (rig, _) = with_computer("sp_late");
    rig.svc.space_deliver(&delivery("d1", true)).unwrap();
    rig.hello();
    post(&rig, reply("d1", "done"));
    post(&rig, call("late", "computer_observe", json!({})));
    assert!(results(&rig).is_empty(), "the conversation is no longer one the bot owes an answer for");
}

#[test]
fn trust_is_the_spaces_verdict_never_the_brains() {
    let (rig, _) = with_computer("sp_trust");
    rig.svc.space_deliver(&delivery("d1", false)).unwrap();
    rig.hello();
    // a brain that says it is the owner in the call changes nothing
    post(&rig, json!({"v":1,"type":"sandbox_call","callId":"x","conversationId":conversation(),"tool":"computer_observe","args":{"trust":"owner_pc","owner":true},"image":false}));
    assert_eq!(results(&rig)[0]["ok"], false);
}

// ---- what the real brain names -----------------------------------------------------------------------------------------

/// The real brain answers with `inReplyTo` = the id of the LINE it answers (the wire says so), not the delivery's own id.
/// Found by `space_loop_tests` (real brain): before the fix every real answer was dropped and the turn never ended.
#[test]
fn the_brain_names_the_line_it_answers_and_that_ends_the_delivery() {
    let (rig, port) = mk("sp_line_id");
    rig.hello();
    rig.svc.space_deliver(&delivery("d1", true)).unwrap();
    post(&rig, reply("d1-line", "Tea at six."));
    assert_eq!(*port.replies.lock(), vec![("nimbus".to_string(), "d1".to_string(), conversation(), "Tea at six.".to_string())]);
    // a pass and a failure name the line too
    rig.svc.space_deliver(&delivery("d2", true)).unwrap();
    post(&rig, json!({"v":1,"type":"pass","conversationId":conversation(),"inReplyTo":"d2-line","reason":"no_reply"}));
    rig.svc.space_deliver(&delivery("d3", true)).unwrap();
    post(&rig, json!({"v":1,"type":"error","code":"INTERNAL","conversationId":conversation(),"inReplyTo":"d3-line"}));
    assert_eq!(*port.ended.lock(), vec![("nimbus".to_string(), "d2".to_string()), ("nimbus".to_string(), "d3".to_string())]);
    // a second answer to the same line is nobody's
    post(&rig, reply("d1-line", "again"));
    assert_eq!(port.replies.lock().len(), 1);
}
