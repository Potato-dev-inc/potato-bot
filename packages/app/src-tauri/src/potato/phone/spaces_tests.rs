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

// ---- evidence: who sent what, on two PCs (ignored; writes JSON for screenshot pages) ----------------------------------

struct EvSink {
    got: std::sync::Mutex<Vec<crate::spaces::bot_bridge::Delivery>>,
    list: Vec<crate::spaces::bot_bridge::Candidate>,
}
impl EvSink {
    fn new(id: &str) -> std::sync::Arc<EvSink> {
        std::sync::Arc::new(EvSink {
            got: std::sync::Mutex::new(Vec::new()),
            list: vec![crate::spaces::bot_bridge::Candidate { id: id.into(), name: id.into() }],
        })
    }
    fn count(&self) -> usize { self.got.lock().unwrap().len() }
    fn last(&self) -> crate::spaces::bot_bridge::Delivery { self.got.lock().unwrap().last().cloned().expect("a delivery") }
}
impl crate::spaces::bot_bridge::BotSink for EvSink {
    fn deliver(&self, d: &crate::spaces::bot_bridge::Delivery) -> Result<(), crate::spaces::bot_bridge::Refusal> {
        self.got.lock().unwrap().push(d.clone());
        Ok(())
    }
    fn candidates(&self) -> Vec<crate::spaces::bot_bridge::Candidate> { self.list.clone() }
}
impl crate::spaces::diary::DiaryHome for EvSink {
    fn vault_of(&self, _: &str) -> Option<std::path::PathBuf> { None }
}

/// Two people (Ren owns the space, Sam joined) and their bots (Hoot, Pip) talk in #general. Each PC's desktop payloads and phone
/// API payloads are written to `$EVIDENCE_DIR/{ren,sam}.json`. Run:
/// `EVIDENCE_DIR=... cargo test --lib evidence_names_two_people_two_bots -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "writes evidence JSON to $EVIDENCE_DIR; run by hand"]
async fn evidence_names_two_people_two_bots() {
    use crate::spaces::bot_bridge::conversation_of;
    use crate::spaces::memnet::MemoryHub;
    use crate::spaces::service_tests::{clock, join, make};

    let dir = std::path::PathBuf::from(std::env::var("EVIDENCE_DIR").expect("set EVIDENCE_DIR"));
    std::fs::create_dir_all(&dir).unwrap();

    let hub = MemoryHub::new();
    let clk = clock();
    let a = make(&hub, 1, &clk, None, None);
    a.svc.set_display_name("Ren").unwrap();
    a.svc.start_with(false).await.unwrap();
    let b = make(&hub, 2, &clk, None, None);
    b.svc.set_display_name("Sam").unwrap();
    b.svc.start_with(false).await.unwrap();
    let hex = a.svc.create_space("Weekend hike").await.unwrap();
    join(&a, &b, &hex).await;
    eventually("a link", || a.svc.link_count() == 1 && b.svc.link_count() == 1).await;
    let space = crate::spaces::ids::SpaceId::from_hex(&hex).unwrap();

    let (sink_a, sink_b) = (EvSink::new("hoot"), EvSink::new("pip"));
    a.svc.set_bot_port(Some(sink_a.clone()));
    b.svc.set_bot_port(Some(sink_b.clone()));
    a.svc.bot_add(&hex, "hoot", "Hoot").unwrap();
    b.svc.bot_add(&hex, "pip", "Pip").unwrap();
    eventually("both bots on both PCs", || {
        a.svc.with_open(&space, |sp| sp.roster.bots.len()) == Some(2) && b.svc.with_open(&space, |sp| sp.roster.bots.len()) == Some(2)
    })
    .await;
    // the roster must show Pip as Sam's on Sam's PC before Sam can call it
    eventually("pip is Sam's here", || b.svc.bot_list(&hex).unwrap()["bots"].as_array().unwrap().iter().any(|x| x["name"] == "Pip" && x["mine"] == true)).await;

    let ch = general(&a, &space).to_hex();
    let step = |clk: &std::sync::Arc<std::sync::atomic::AtomicU64>| { clk.fetch_add(2 * 60_000, Ordering::SeqCst); };
    let mut want = 0usize;

    // 1. Ren asks Hoot
    step(&clk);
    a.svc.send_text(&hex, &ch, "Morning Sam! I'd love a hike this Saturday. @Hoot what's a good easy trail near us?", None).await.unwrap();
    want += 1;
    eventually("hoot is called", || sink_a.count() == 1).await;
    // 2. Hoot answers (the real reply path)
    step(&clk);
    let d = sink_a.last();
    a.svc
        .bot_reply(&d.bot_local, &d.id, &conversation_of(&d.space, &d.channel),
            "Try the Ridge Loop. It is about 6 km, takes around two hours, is mostly shaded and has a lookout halfway round.")
        .unwrap();
    want += 1;
    eventually("both have 2", || texts(&a, &space).len() == want && texts(&b, &space).len() == want).await;
    // 3. Sam calls Pip
    step(&clk);
    b.svc.send_text(&hex, &ch, "The Ridge Loop sounds perfect. I'll bring sandwiches. @Pip what else should we pack?", None).await.unwrap();
    want += 1;
    eventually("pip is called", || sink_b.count() == 1).await;
    // 4. Pip answers
    step(&clk);
    let d = sink_b.last();
    b.svc
        .bot_reply(&d.bot_local, &d.id, &conversation_of(&d.space, &d.channel),
            "Water (one litre each), a light jacket, sunscreen and a few snacks. Start early, it gets warm by midday.")
        .unwrap();
    want += 1;
    eventually("both have 4", || texts(&a, &space).len() == want && texts(&b, &space).len() == want).await;
    // 5. Sam
    step(&clk);
    b.svc.send_text(&hex, &ch, "Thanks both! Is 8am at the trailhead okay for you, Ren?", None).await.unwrap();
    want += 1;
    eventually("both have 5", || texts(&a, &space).len() == want && texts(&b, &space).len() == want).await;
    // 6. Ren
    step(&clk);
    a.svc.send_text(&hex, &ch, "8am works for me. I'll bring a first-aid kit. See you Saturday!", None).await.unwrap();
    want += 1;
    eventually("both have 6", || texts(&a, &space).len() == want && texts(&b, &space).len() == want).await;

    // ---- the payloads, per PC ----
    for (who, n) in [("ren", &a), ("sam", &b)] {
        let svc = &n.svc;
        let k = kit(&format!("ev-{who}"), None);
        k.env.spaces_on.store(true, Ordering::SeqCst);
        *k.env.spaces.lock() = Some(svc.clone());
        let (device_id, token) = k.pair();
        let get = |path: &str| {
            let r = k.authed(&token, "GET", path, "");
            assert_eq!(r.status, 200, "{path}: {}", r.body_json());
            r.body_json()
        };
        let phone_spaces = get("/api/spaces");
        let phone_msgs = get(&format!("/api/spaces/{hex}/channels/{ch}/messages?limit=50"));
        let phone_me = get("/api/me");
        let doc = json!({
            "who": who,
            "spaceId": hex, "channelId": ch,
            "desktop": {
                "spaces_display_name": { "displayName": svc.display_name() },
                "spaces_view": svc.view(),
                "space_detail": svc.detail(&hex).unwrap(),
                "space_messages": svc.messages(&hex, &ch, None, 200).unwrap(),
                "space_bot_list": svc.bot_list(&hex).unwrap(),
            },
            "phone": { "me": phone_me, "spaces": phone_spaces, "messages": phone_msgs, "token": token, "deviceId": device_id },
        });
        std::fs::write(dir.join(format!("{who}.json")), serde_json::to_string_pretty(&doc).unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&k.dir);
    }
    a.svc.stop().await;
    b.svc.stop().await;
}
