//! Spaces on the phone: the route table's abilities, the list / messages / read / send happy paths against real
//! `SpacesService` nodes on the in-memory net, the refusals (no `type`, read-only channel, Labs off, unknown ids), the
//! change feed waking on a space event, and that this file calls nothing but read and send.

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{json, Value};

use super::api::{need, need_for};
use super::live::space_observer_for;
use super::testkit::{req, Kit, kit};
use crate::mobile_pairing::{Cap, Caps};
use crate::spaces::service_tests::{eventually, general, pair, say, texts, Node};

struct Rig {
    k: Kit,
    a: Node,
    b: Node,
    space: String,
    channel: String,
    token: String,
}

/// Two joined nodes; the phone is paired to node `a`'s PC (the owner), Spaces is on.
async fn rig(name: &str) -> Rig {
    let (_hub, _clk, a, b, space) = pair().await;
    let k = kit(name, None);
    k.env.spaces_on.store(true, Ordering::SeqCst);
    *k.env.spaces.lock() = Some(a.svc.clone());
    a.svc.set_observer(space_observer_for(k.ctx.changes.clone()));
    let (_, token) = k.pair();
    let channel = general(&a, &space).to_hex();
    Rig { k, a, b, space: space.to_hex(), channel, token }
}

impl Rig {
    fn get(&self, path: &str) -> (u16, Value) {
        let r = self.k.authed(&self.token, "GET", path, "");
        (r.status, r.body_json())
    }
    fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let r = self.k.authed(&self.token, "POST", path, &body.to_string());
        (r.status, r.body_json())
    }
    fn base(&self) -> String {
        format!("/api/spaces/{}/channels/{}", self.space, self.channel)
    }
}

#[test]
fn the_routes_need_the_right_abilities_and_anything_else_needs_control() {
    let seg = |s: &'static str| s.split('/').collect::<Vec<_>>();
    assert_eq!(need("GET", &seg("spaces")), Some(Cap::Watch));
    assert_eq!(need("GET", &seg("spaces/s/channels/c/messages")), Some(Cap::Watch));
    assert_eq!(need("POST", &seg("spaces/s/channels/c/read")), Some(Cap::Watch));
    assert_eq!(need("POST", &seg("spaces/s/channels/c/send")), Some(Cap::Type));
    // Nothing else under /spaces is a phone action: it fails closed to `control` (and then 404s in the route).
    for (m, p) in [
        ("POST", "spaces"),
        ("DELETE", "spaces/s"),
        ("POST", "spaces/s/invite"),
        ("POST", "spaces/s/leave"),
        ("POST", "spaces/s/channels"),
        ("POST", "spaces/s/channels/c/archive"),
        ("POST", "spaces/s/members/m/remove"),
        ("POST", "spaces/s/bots"),
        ("GET", "spaces/s/channels/c/messages/extra"),
    ] {
        assert_eq!(need(m, &seg(p)), Some(Cap::Control), "{m} {p}");
    }
    let r = req("POST", "/api/spaces/s/channels/c/send", &[], r#"{"text":"hi"}"#);
    assert_eq!(need_for(&r, &seg("spaces/s/channels/c/send")), Some(Cap::Type));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_phone_without_type_cannot_send_but_can_read() {
    let r = rig("sp-caps").await;
    let (_, watch_token) = r.k.pair_with(Caps::none().with(Cap::Watch, true));
    let send = r.k.authed(&watch_token, "POST", &format!("{}/send", r.base()), r#"{"text":"hi"}"#);
    assert_eq!(send.status, 403, "{}", send.body_json());
    assert!(texts(&r.a, &crate::spaces::ids::SpaceId::from_hex(&r.space).unwrap()).is_empty(), "nothing was sent");
    assert_eq!(r.k.authed(&watch_token, "GET", "/api/spaces", "").status, 200);
    // A phone with nothing (not even watch) cannot read the list either.
    let (_, none_token) = r.k.pair_with(Caps::none());
    assert_eq!(r.k.authed(&none_token, "GET", "/api/spaces", "").status, 403);
    // The `X-Potato-Phone` header is still needed on a POST.
    let bearer = format!("Bearer {}", r.token);
    let bare = crate::potato::phone::gate::dispatch(
        &r.k.ctx,
        &{
            let mut q = req("POST", &format!("{}/send", r.base()), &[("authorization", bearer.as_str())], r#"{"text":"hi"}"#);
            q.headers.retain(|(k, _)| k != "x-potato-phone");
            q
        },
    );
    assert_eq!(bare.status, 403);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn me_says_spaces_only_when_it_is_on_and_everything_is_409_when_labs_is_off() {
    let r = rig("sp-labs").await;
    let (status, me) = r.get("/api/me");
    assert_eq!((status, me["spaces"].as_bool()), (200, Some(true)));
    r.k.env.spaces_on.store(false, Ordering::SeqCst);
    let (_, me) = r.get("/api/me");
    assert!(me.get("spaces").is_none(), "omitted when off: {me}");
    for (status, v) in [
        r.get("/api/spaces"),
        r.get(&format!("{}/messages", r.base())),
        r.post(&format!("{}/read", r.base()), json!({})),
        r.post(&format!("{}/send", r.base()), json!({ "text": "hi" })),
    ] {
        assert_eq!((status, v["error"].as_str()), (409, Some("spaces_off")), "{v}");
        assert!(v["message"].as_str().is_some_and(|m| !m.is_empty()));
    }
    let id = crate::spaces::ids::SpaceId::from_hex(&r.space).unwrap();
    assert!(texts(&r.a, &id).is_empty(), "nothing was sent while Labs was off");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_list_has_spaces_channels_unread_and_can_write() {
    let r = rig("sp-list").await;
    let space = crate::spaces::ids::SpaceId::from_hex(&r.space).unwrap();
    say(&r.b, &space, "hello from b").await;
    eventually("a got it", || texts(&r.a, &space).len() == 1).await;
    let (status, v) = r.get("/api/spaces");
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["online"], true);
    let s = &v["spaces"][0];
    assert_eq!((s["id"].as_str(), s["name"].as_str(), s["unread"].as_u64()), (Some(r.space.as_str()), Some("Book club"), Some(1)));
    let c = &s["channels"][0];
    assert_eq!((c["id"].as_str(), c["name"].as_str(), c["unread"].as_u64(), c["canWrite"].as_bool()), (Some(r.channel.as_str()), Some("general"), Some(1), Some(true)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn messages_come_oldest_first_in_pages_with_the_fields_mapped() {
    let r = rig("sp-msgs").await;
    let space = crate::spaces::ids::SpaceId::from_hex(&r.space).unwrap();
    for t in ["one", "two", "three"] {
        say(&r.b, &space, t).await;
    }
    eventually("a got all", || texts(&r.a, &space).len() == 3).await;
    let (status, v) = r.get(&format!("{}/messages?limit=2", r.base()));
    assert_eq!(status, 200, "{v}");
    let texts_of = |v: &Value| v["messages"].as_array().unwrap().iter().map(|m| m["text"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(texts_of(&v), ["two", "three"], "the newest page, oldest first");
    assert_eq!((v["hasMore"].as_bool(), v["canWrite"].as_bool()), (Some(true), Some(true)));
    let m = &v["messages"][0];
    assert_eq!((m["mine"].as_bool(), m["bot"].as_bool(), m["state"].as_str()), (Some(false), Some(false), Some("sent")));
    assert!(m["id"].as_str().is_some_and(|s| !s.is_empty()) && m["atMs"].as_u64().is_some() && m["files"].as_array().unwrap().is_empty());
    assert!(m["authorName"].as_str().is_some_and(|n| !n.is_empty()), "an author name is there");
    let before = m["id"].as_str().unwrap();
    let (_, older) = r.get(&format!("{}/messages?limit=2&before={before}", r.base()));
    assert_eq!(texts_of(&older), ["one"]);
    assert_eq!(older["hasMore"], false);
    // limit is clamped to 1..=100 and a junk `before` is a plain 404, not a crash.
    assert_eq!(r.get(&format!("{}/messages?limit=100000", r.base())).0, 200);
    assert_eq!(r.get(&format!("{}/messages?before=zz", r.base())).0, 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn read_clears_the_unread_count() {
    let r = rig("sp-read").await;
    let space = crate::spaces::ids::SpaceId::from_hex(&r.space).unwrap();
    say(&r.b, &space, "ping").await;
    eventually("a got it", || texts(&r.a, &space).len() == 1).await;
    assert_eq!(r.get("/api/spaces").1["spaces"][0]["unread"], 1);
    let (status, v) = r.post(&format!("{}/read", r.base()), json!({}));
    assert_eq!((status, v["ok"].as_bool()), (200, Some(true)));
    assert_eq!(r.get("/api/spaces").1["spaces"][0]["unread"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_send_is_the_desktop_send_and_reaches_the_other_member() {
    let r = rig("sp-send").await;
    let space = crate::spaces::ids::SpaceId::from_hex(&r.space).unwrap();
    let (status, v) = r.post(&format!("{}/send", r.base()), json!({ "text": "  hi from my phone  " }));
    assert_eq!((status, v["ok"].as_bool()), (200, Some(true)), "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    assert!(!id.is_empty());
    eventually("b got it", || texts(&r.b, &space) == ["hi from my phone"]).await;
    // It is this PC's own person talking: the message reads back as mine, with the id the send gave.
    let (_, page) = r.get(&format!("{}/messages", r.base()));
    let last = page["messages"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((last["id"].as_str(), last["mine"].as_bool(), last["text"].as_str()), (Some(id.as_str()), Some(true), Some("hi from my phone")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bad_text_unknown_ids_and_read_only_channels_are_refused_with_plain_words() {
    let r = rig("sp-refuse").await;
    let space = crate::spaces::ids::SpaceId::from_hex(&r.space).unwrap();
    let send = format!("{}/send", r.base());
    for (body, status, code) in [
        (json!({ "text": "   " }), 400, "empty"),
        (json!({}), 400, "empty"),
        (json!({ "text": "x".repeat(8001) }), 400, "too_long"),
        (json!({ "text": "x".repeat(4001) }), 400, "too_long"),
    ] {
        let (s, v) = r.post(&send, body);
        assert_eq!((s, v["error"].as_str()), (status, Some(code)), "{v}");
        assert!(v["message"].as_str().is_some_and(|m| !m.is_empty()));
    }
    let bad = format!("/api/spaces/{}/channels/{}/send", "00".repeat(32), r.channel);
    assert_eq!(r.post(&bad, json!({ "text": "hi" })).0, 404);
    let bad = format!("/api/spaces/{}/channels/{}/send", r.space, "00".repeat(16));
    assert_eq!(r.post(&bad, json!({ "text": "hi" })).0, 404);
    assert_eq!(r.get(&format!("/api/spaces/{}/channels/nope/messages", r.space)).0, 404);
    assert_eq!(r.get("/api/spaces/nope/channels/nope/messages").0, 404);
    // A channel the owner closed can be read but not written.
    let detail = r.a.svc.channel_create(&r.space, "old").await.unwrap();
    let old = detail["channels"].as_array().unwrap().iter().find(|c| c["name"] == "old").unwrap()["id"].as_str().unwrap().to_string();
    say(&r.a, &space, "still here").await;
    r.a.svc.channel_archive(&r.space, &old).await.unwrap();
    let (s, v) = r.post(&format!("/api/spaces/{}/channels/{old}/send", r.space), json!({ "text": "hi" }));
    assert_eq!((s, v["error"].as_str()), (403, Some("read_only")), "{v}");
    let (s, v) = r.get(&format!("/api/spaces/{}/channels/{old}/messages", r.space));
    assert_eq!((s, v["canWrite"].as_bool()), (200, Some(false)), "{v}");
    let (_, list) = r.get("/api/spaces");
    assert!(list["spaces"][0]["channels"].as_array().unwrap().iter().all(|c| c["id"] != old.as_str()), "a closed channel is not listed");
    // Nothing reached the log from the refused sends.
    assert_eq!(texts(&r.a, &space), ["still here"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_space_event_wakes_the_change_feed() {
    let r = rig("sp-feed").await;
    let space = crate::spaces::ids::SpaceId::from_hex(&r.space).unwrap();
    // A message arriving from another member bumps the revision.
    let before = r.k.ctx.changes.rev();
    say(&r.b, &space, "wake up").await;
    eventually("the feed moved", || r.k.ctx.changes.rev() > before).await;
    // And a long poll that is waiting wakes at once, well before its wait is over.
    let since = r.k.ctx.changes.rev();
    let ctx = r.k.ctx.clone();
    let poll = std::thread::spawn(move || {
        let t = std::time::Instant::now();
        let rev = ctx.changes.wait(since, Duration::from_secs(20));
        (rev, t.elapsed())
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    say(&r.b, &space, "second").await;
    let (rev, took) = poll.join().unwrap();
    assert!(rev > since && took < Duration::from_secs(10), "woke after {took:?}");
    // The route itself: GET /api/changes with the old revision answers with a newer one without waiting out its time.
    let (status, v) = r.get(&format!("/api/changes?since={}&wait_ms=3000", since));
    assert_eq!(status, 200);
    assert!(v["rev"].as_u64().unwrap() > since);
}

/// The phone is read and send only: this file may not call anything that administers a space or a bot.
#[test]
fn the_phone_routes_call_only_read_and_send() {
    let source = include_str!("spaces_routes.rs");
    for banned in [
        ".create_invite(", ".cancel_invite(", ".start_join(", ".answer_join(", ".remove_member(", ".leave(", ".forget(", ".block(",
        ".channel_create(", ".channel_rename(", ".channel_archive(", ".create_space(", ".react(", ".delete(", "send_with_bot", "send_now",
        "bots_in_detail", "bot_port", "offer_file",
    ] {
        assert!(!source.contains(banned), "the phone routes must not call {banned}");
    }
    assert!(source.contains(".send_text("), "a phone message goes through the desktop's own send");
}
