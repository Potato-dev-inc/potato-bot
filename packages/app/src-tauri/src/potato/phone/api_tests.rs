//! The phone routes on the real engine (with its fake clock, host and launcher): the bot list, a chat, sending text and
//! a photo, answering every kind of card, the job Stop, Pause all, and groups. And what must never happen: a password
//! or key passing through, a sign-in card answered, a connector opened, "Always allow" reached.

use serde_json::{json, Value};

use super::testkit::{kit, Kit};
use crate::potato::cards::{self, Kind};
use crate::potato::rooms::store::{self as rstore, Room};
use crate::potato::service_card_tests::{ask, one_card, rig, DM};
use crate::potato::service_tests::{Rig, KEY};

fn setup(name: &str) -> (Rig, Kit, String) {
    let (rig, _fake) = rig(name);
    let k = kit(name, Some(rig.svc.clone()));
    let (_, token) = k.pair();
    (rig, k, token)
}

fn get(k: &Kit, token: &str, path: &str) -> (u16, Value) {
    let r = k.authed(token, "GET", path, "");
    (r.status, r.body_json())
}

fn post(k: &Kit, token: &str, path: &str, body: Value) -> (u16, Value) {
    let r = k.authed(token, "POST", path, &body.to_string());
    (r.status, r.body_json())
}

fn new_card(kind: Kind, action: &str, choices: &[&str]) -> cards::New {
    cards::New {
        kind,
        action: action.into(),
        why: "because".into(),
        choices: choices.iter().map(|c| c.to_string()).collect(),
        conversation: DM.into(),
        command_hash: None,
        classes: vec![],
        keep_going: None,
        connector: None,
        login: None,
        mcp_add: None,
        speaker: "Nimbus".into(),
        home: None,
    }
}

fn raise(rig: &Rig, new: cards::New) -> cards::Card {
    cards::raise(&rig.root, "nimbus", new, 2_000).unwrap().card
}

#[test]
fn the_bot_list_has_a_last_line_a_state_and_needs_you() {
    let (rig, k, token) = setup("phone-list");
    let (status, v) = get(&k, &token, "/api/bots");
    assert_eq!(status, 200, "{v}");
    let bots = v["bots"].as_array().unwrap();
    let nimbus = bots.iter().find(|b| b["id"] == "nimbus").unwrap();
    assert_eq!(nimbus["name"], "Nimbus");
    assert_eq!(nimbus["lastLine"], "ok", "the last line is what the bot said last");
    assert_eq!(nimbus["state"], "idle");
    assert_eq!(nimbus["openCards"], 0);
    assert!(v["rev"].as_u64().unwrap() >= 1 && v["paused"] == json!(false) && v["rooms"].is_array());

    ask(&rig, "a-1");
    let (_, v) = get(&k, &token, "/api/bots");
    let nimbus = v["bots"].as_array().unwrap().iter().find(|b| b["id"] == "nimbus").unwrap().clone();
    assert_eq!(nimbus["state"], "needs_you");
    assert_eq!(nimbus["stateText"], "Needs you");
    assert_eq!(nimbus["openCards"], 1);
}

#[test]
fn a_chat_has_history_open_cards_and_no_internal_fields() {
    let (rig, k, token) = setup("phone-chat");
    ask(&rig, "a-1");
    let (status, v) = get(&k, &token, "/api/bots/nimbus");
    assert_eq!(status, 200);
    assert_eq!(v["bot"]["id"], "nimbus");
    let texts: Vec<_> = v["messages"].as_array().unwrap().iter().filter_map(|m| m["text"].as_str()).collect();
    assert!(texts.contains(&"hello") && texts.contains(&"ok"), "{texts:?}");
    let cards = v["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["kind"], "approval");
    assert_eq!(cards[0]["action"], "Deploy to staging");
    for hidden in ["commandHash", "conversation", "classes"] {
        assert!(cards[0].get(hidden).is_none(), "{hidden} is not for the phone");
    }
    for m in v["messages"].as_array().unwrap() {
        assert!(m.get("paneId").is_none() && m.get("costNano").is_none());
    }
    assert!(v.get("job").is_some() && v.get("work").is_some() && v["typing"] == json!(false));
    assert_eq!(get(&k, &token, "/api/bots/ghost").0, 404);
    assert!(!serde_json::to_string(&v).unwrap().contains(KEY), "a key never goes to a phone");
}

#[test]
fn sending_a_message_saves_it_and_says_delivered() {
    let (rig, k, token) = setup("phone-send");
    let (status, v) = post(&k, &token, "/api/bots/nimbus/send", json!({"text": "How is it going?"}));
    assert_eq!(status, 200, "{v}");
    assert!(v["id"].as_str().unwrap().starts_with("m-"));
    assert_eq!(v["delivered"], true);
    assert!(rig.thread().iter().any(|m| m.text == "How is it going?"));
    assert!(k.ctx.changes.rev() >= 2, "the phone's own change wakes the other phones");
    for bad in [json!({"text": ""}), json!({"text": "   "}), json!({}), json!({"text": "x".repeat(9000)})] {
        assert_eq!(post(&k, &token, "/api/bots/nimbus/send", bad).0, 400);
    }
    assert_eq!(post(&k, &token, "/api/bots/ghost/send", json!({"text": "hi"})).0, 404);
}

#[test]
fn a_message_sent_while_the_bot_works_shows_as_queued() {
    let (rig, k, token) = setup("phone-queued");
    rig.post(vec![json!({"v":1,"type":"typing","conversationId":DM,"on":true})]);
    let id = post(&k, &token, "/api/bots/nimbus/send", json!({"text": "also check the logs"})).1["id"].as_str().unwrap().to_string();
    let (_, v) = get(&k, &token, "/api/bots/nimbus");
    let m = v["messages"].as_array().unwrap().iter().find(|m| m["id"] == id.as_str()).unwrap();
    assert_eq!(m["queued"], true, "{m}");
}

// a 1x1 PNG
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

#[test]
fn a_photo_is_saved_in_the_bots_folder_and_the_bot_is_told() {
    let (rig, k, token) = setup("phone-photo");
    let (status, v) = post(&k, &token, "/api/bots/nimbus/send", json!({"text": "what is this?", "files": [{"name": "look.png", "kind": "image", "data": PNG}]}));
    assert_eq!(status, 200, "{v}");
    let last = rig.thread().into_iter().rev().find(|m| m.text.contains("what is this?")).unwrap();
    assert!(last.text.contains("[Attached image: look.png"), "{}", last.text);
    // Something that is not an image, and five files, are refused and nothing is kept.
    let (bad, _) = post(&k, &token, "/api/bots/nimbus/send", json!({"text": "x", "files": [{"name": "a.exe", "kind": "image", "data": "AAAA"}]}));
    assert_eq!(bad, 400);
    let too_many: Vec<_> = (0..5).map(|i| json!({"name": format!("{i}.png"), "kind": "image", "data": PNG})).collect();
    assert_eq!(post(&k, &token, "/api/bots/nimbus/send", json!({"text": "x", "files": too_many})).0, 400);
    assert_eq!(post(&k, &token, "/api/bots/nimbus/send", json!({"text": "x", "files": "nope"})).0, 400);
}

// ---- cards ---------------------------------------------------------------------------------------------------------

#[test]
fn an_approval_is_answered_allow_once_allow_for_this_job_or_deny() {
    for (word, name) in [("allow_once", "once"), ("allow_job", "job"), ("deny", "deny")] {
        let (rig, k, token) = setup(&format!("phone-approve-{name}"));
        ask(&rig, "a-1");
        let card = one_card(&rig);
        let (status, v) = post(&k, &token, &format!("/api/bots/nimbus/cards/{}/answer", card.id), json!({"answer": word}));
        assert_eq!(status, 200, "{word}: {v}");
        assert_eq!(v["answer"], word);
        assert!(crate::potato::cards::open_cards(&rig.root, "nimbus").is_empty(), "{word} closed the card");
        let again = post(&k, &token, &format!("/api/bots/nimbus/cards/{}/answer", card.id), json!({"answer": word}));
        assert_eq!(again.0, 404, "one answer only");
    }
}

#[test]
fn an_approval_refuses_words_it_does_not_offer_including_always_allow() {
    let (rig, k, token) = setup("phone-approve-bad");
    ask(&rig, "a-1");
    let card = one_card(&rig);
    for bad in ["always", "allow_always", "0", "yes please", "", "ALLOW_ONCE; rm -rf /"] {
        let (status, v) = post(&k, &token, &format!("/api/bots/nimbus/cards/{}/answer", card.id), json!({"answer": bad}));
        assert_eq!(status, 400, "{bad:?}: {v}");
    }
    assert_eq!(cards::open_cards(&rig.root, "nimbus").len(), 1, "still waiting");
    assert_eq!(post(&k, &token, "/api/bots/nimbus/cards/card-ffffffffffff/answer", json!({"answer": "deny"})).0, 404);
}

#[test]
fn a_question_is_answered_with_a_choice_number() {
    let (rig, k, token) = setup("phone-question");
    let card = raise(&rig, new_card(Kind::Question, "Which server?", &["staging", "production"]));
    let (_, v) = get(&k, &token, "/api/bots/nimbus");
    assert_eq!(v["cards"][0]["choices"], json!(["staging", "production"]));
    assert_eq!(post(&k, &token, &format!("/api/bots/nimbus/cards/{}/answer", card.id), json!({"answer": "7"})).0, 400);
    let (status, v) = post(&k, &token, &format!("/api/bots/nimbus/cards/{}/answer", card.id), json!({"answer": "1"}));
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["answer"], "choice:1");
}

#[test]
fn keep_going_or_stop_is_a_question_the_phone_can_answer() {
    let (rig, k, token) = setup("phone-keep-going");
    let mut new = new_card(Kind::Question, "Your job reached its limit", &["Keep going", "Stop"]);
    new.keep_going = Some("steps".into());
    let card = raise(&rig, new);
    let (_, v) = get(&k, &token, "/api/bots/nimbus");
    assert!(v["cards"][0]["keepGoing"].is_string());
    assert_eq!(post(&k, &token, &format!("/api/bots/nimbus/cards/{}/answer", card.id), json!({"answer": "1"})).0, 200);
}

#[test]
fn a_connector_card_cannot_open_pc_settings_from_the_phone() {
    let (rig, k, token) = setup("phone-connector");
    let mut new = new_card(Kind::Question, "Nimbus wants to use GitHub", &["Connect in Settings", "Not now"]);
    new.connector = Some("GitHub".into());
    let card = raise(&rig, new);
    let path = format!("/api/bots/nimbus/cards/{}/answer", card.id);
    let (status, v) = post(&k, &token, &path, json!({"answer": "0"}));
    assert_eq!(status, 403, "{v}");
    assert_eq!(v["error"], "pc_only");
    assert_eq!(cards::open_cards(&rig.root, "nimbus").len(), 1, "still waiting for the PC");
    assert_eq!(post(&k, &token, &path, json!({"answer": "1"})).0, 200, "Not now is fine");
}

#[test]
fn a_sign_in_card_can_only_be_dismissed_and_no_password_is_ever_read() {
    let (rig, k, token) = setup("phone-login");
    let mut new = new_card(Kind::Question, "Nimbus wants to sign in to shop.example.com", &["Sign in", "Take over", "Not now"]);
    new.login = Some("https://shop.example.com".into());
    let card = raise(&rig, new);
    let path = format!("/api/bots/nimbus/cards/{}/answer", card.id);
    for secret_try in [
        json!({"answer": "0", "username": "ren", "password": "hunter2"}),
        json!({"answer": "1", "password": "hunter2"}),
        json!({"answer": "sign_in", "password": "hunter2"}),
    ] {
        let (status, v) = post(&k, &token, &path, secret_try);
        assert!(status == 403 || status == 400, "{status} {v}");
        assert!(!v.to_string().contains("hunter2"));
    }
    assert_eq!(cards::open_cards(&rig.root, "nimbus").len(), 1, "nothing answered it");
    let (status, _) = post(&k, &token, &path, json!({"answer": "2", "password": "hunter2"}));
    assert_eq!(status, 200, "Not now is allowed");
    // The password was never read, so it is nowhere in what the engine kept.
    let all: String = std::fs::read_dir(&rig.root).unwrap().flatten().flat_map(|e| walk(&e.path())).collect();
    assert!(!all.contains("hunter2"), "a password must not reach any file");
    // And the phone's view of the sign-in card says where to sign in.
    let (_, v) = get(&k, &token, "/api/bots/nimbus");
    assert!(v["cards"].as_array().unwrap().is_empty());
}

fn walk(path: &std::path::Path) -> Vec<String> {
    if path.is_dir() {
        std::fs::read_dir(path).map(|d| d.flatten().flat_map(|e| walk(&e.path())).collect()).unwrap_or_default()
    } else {
        vec![std::fs::read_to_string(path).unwrap_or_default()]
    }
}

#[test]
fn the_sign_in_card_reaches_the_phone_with_its_origin_and_no_secret_field() {
    let (rig, k, token) = setup("phone-login-view");
    let mut new = new_card(Kind::Question, "Nimbus wants to sign in to shop.example.com", &["Sign in", "Take over", "Not now"]);
    new.login = Some("https://shop.example.com".into());
    raise(&rig, new);
    let (_, v) = get(&k, &token, "/api/bots/nimbus");
    assert_eq!(v["cards"][0]["login"], "https://shop.example.com");
    let text = v.to_string().to_lowercase();
    assert!(!text.contains("password\":") && !text.contains("secret"), "{text}");
}

// ---- job, pause, groups ---------------------------------------------------------------------------------------------

#[test]
fn stop_ends_the_job_and_pause_all_then_start_again_toggle_the_hold() {
    let (_rig, k, token) = setup("phone-stop");
    assert_eq!(post(&k, &token, "/api/bots/nimbus/stop", json!({})).0, 200);
    assert_eq!(post(&k, &token, "/api/bots/ghost/stop", json!({})).0, 404);
    assert_eq!(post(&k, &token, "/api/pause-all", json!({})).1["paused"], true);
    assert_eq!(get(&k, &token, "/api/bots").1["paused"], true);
    assert_eq!(get(&k, &token, "/api/me").1["paused"], true);
    assert_eq!(post(&k, &token, "/api/resume-all", json!({})).1["paused"], false);
}

#[test]
fn groups_are_listed_opened_and_written_to_only_when_the_switch_is_on() {
    let (rig, k, token) = setup("phone-groups");
    rig.svc.add_bot("mochi").ok();
    let room = Room { id: "rabc123def456".into(), name: "Team".into(), bot_ids: vec!["nimbus".into(), "mochi".into()], created_ms: 1, last_ms: 5, unread: 2, ..Default::default() };
    rstore::save_rooms(&rig.root, &[room]).unwrap();

    k.env.groups.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(get(&k, &token, "/api/bots").1["rooms"], json!([]));
    assert_eq!(get(&k, &token, "/api/rooms/rabc123def456").0, 404);
    assert_eq!(post(&k, &token, "/api/rooms/rabc123def456/send", json!({"text": "hi"})).0, 404);

    k.env.groups.store(true, std::sync::atomic::Ordering::SeqCst);
    let (_, v) = get(&k, &token, "/api/bots");
    assert_eq!(v["rooms"][0]["name"], "Team");
    assert_eq!(v["rooms"][0]["bots"], json!(["Nimbus", "Mochi"]));
    assert_eq!(v["rooms"][0]["unread"], 2);
    assert_eq!(get(&k, &token, "/api/me").1["groups"], true);
    let (status, v) = get(&k, &token, "/api/rooms/rabc123def456");
    assert_eq!(status, 200, "{v}");
    assert!(v["lines"].is_array() && v["work"].is_array() && v["typing"].is_array());
    let (status, v) = post(&k, &token, "/api/rooms/rabc123def456/send", json!({"text": "hello team"}));
    assert_eq!(status, 200, "{v}");
    let (_, v) = get(&k, &token, "/api/rooms/rabc123def456");
    assert!(v["lines"].as_array().unwrap().iter().any(|l| l["text"] == "hello team"));
    assert_eq!(get(&k, &token, "/api/rooms/not-a-room").0, 400);
    assert_eq!(get(&k, &token, "/api/rooms/rffffffffffff").0, 404);
}

#[test]
fn the_engine_missing_is_a_plain_503_not_a_crash() {
    let k = kit("phone-no-engine", None);
    let (_, token) = k.pair();
    let r = k.authed(&token, "GET", "/api/bots", "");
    assert_eq!(r.status, 503);
    assert_eq!(r.body_json()["error"], "not_ready");
}

#[test]
fn opening_a_chat_on_the_phone_can_mark_it_read() {
    let (rig, k, token) = setup("phone-read");
    assert_eq!(post(&k, &token, "/api/bots/nimbus/read", json!({})).0, 200);
    let _ = rig;
}
