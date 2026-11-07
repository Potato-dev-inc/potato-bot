//! Bots in Spaces, end to end with nothing stubbed on the way: two Space members on the in-memory net, the REAL Spaces
//! service of the bot owner's PC, the REAL Potato service with the REAL launcher, the REAL bundled brain (a `node` process),
//! and a fake model on 127.0.0.1 that answers as an OpenAI-compatible `/chat/completions` endpoint. The unit suites test
//! Rust and TypeScript apart; this one proves the JSON Rust sends is what the real brain accepts, and what the real brain
//! sends back is what Rust accepts.
//!
//! No real key, no network beyond loopback. The model's key is a made-up string and its endpoint is the fake server.
//! Skips (printing why) when Node 20+ is missing or the brain cannot be bundled. Ignored by default because it starts a
//! node process and takes a while:
//!
//!   cargo test --lib potato::space_loop_tests -- --ignored --nocapture
//!
//! (with `CARGO_TARGET_DIR` set as for the other tests). It opens no window and no dev server.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::auth::Brains;
use super::fake_http::{self, Reply, Req, Server};
use super::service::*;
use super::service_tests::{FakeHost, KEY};
use super::sidecar::{Launcher, NodeState, RealLauncher};
use super::space_link;
use super::store::{self, BrainKind, CustomProvider};
use crate::spaces::ids::SpaceId;
use crate::spaces::service_tests::{eventually, general, pair_with, say, Node};

fn real_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The brain bundle of this source tree, rebuilt now so it is never older than the source. `None` with a printed reason.
fn fresh_brain() -> Option<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = manifest.join("resources").join("potato-brain");
    let script = manifest.join("..").join("..").join("..").join("scripts").join("bundle-potato-brain.mjs");
    let built = std::process::Command::new("node").arg(&script).status().map(|s| s.success()).unwrap_or(false);
    let stub = std::fs::read_to_string(dir.join("brain.mjs")).map(|t| t.contains("potato-brain stub")).unwrap_or(true);
    if !built || stub {
        eprintln!("skipped: resources/potato-brain/brain.mjs cannot be built (node scripts/bundle-potato-brain.mjs)");
        return None;
    }
    Some(dir)
}

// ---- the fake model ------------------------------------------------------------------------------------------------

struct Model {
    server: Server,
    script: Arc<Mutex<VecDeque<Value>>>,
}

impl Model {
    fn start() -> Model {
        let script: Arc<Mutex<VecDeque<Value>>> = Arc::new(Mutex::new(VecDeque::new()));
        let s = script.clone();
        let server = fake_http::serve(move |_req| {
            let message = s.lock().pop_front().unwrap_or_else(|| json!({"role":"assistant","content":"(nothing scripted)"}));
            let finish = if message.get("tool_calls").is_some() { "tool_calls" } else { "stop" };
            let body = json!({"choices":[{"message":message,"finish_reason":finish}],"usage":{"prompt_tokens":20,"completion_tokens":8}});
            Reply::json(200, &body.to_string())
        });
        Model { server, script }
    }

    fn say(&self, text: &str) {
        self.script.lock().push_back(json!({"role":"assistant","content":text}));
    }

    fn call(&self, calls: &[(&str, Value)]) {
        let tool_calls: Vec<Value> = calls
            .iter()
            .enumerate()
            .map(|(i, (name, args))| json!({"id":format!("call_{i}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}))
            .collect();
        self.script.lock().push_back(json!({"role":"assistant","content":null,"tool_calls":tool_calls}));
    }

    /// Every chat request received so far, parsed.
    fn requests(&self) -> Vec<Value> {
        self.server
            .requests()
            .iter()
            .filter(|r: &&Req| r.path.contains("chat/completions"))
            .map(|r| serde_json::from_str(&r.body).expect("the brain sent JSON"))
            .collect()
    }
}

fn tool_names(req: &Value) -> BTreeSet<String> {
    req["tools"].as_array().map(|t| t.iter().filter_map(|t| t["function"]["name"].as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn texts_of(req: &Value, role: &str) -> Vec<String> {
    req["messages"].as_array().unwrap().iter().filter(|m| m["role"] == role).map(|m| m["content"].to_string()).collect()
}

// ---- the rig -------------------------------------------------------------------------------------------------------

struct Potato {
    svc: Arc<Service>,
    stop: Arc<AtomicBool>,
    root: PathBuf,
}

impl Drop for Potato {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Only the node process this service started is stopped (by its own handle), never by image name.
        self.svc.stop_all(Duration::ZERO);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn potato(resources: PathBuf, model_base: &str) -> Potato {
    let root = std::env::temp_dir().join(format!("pm-potato-loop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let data = root.join("potato");
    let host = Arc::new(FakeHost {
        now: AtomicI64::new(real_ms()),
        secrets: Mutex::new(HashMap::from([(SECRET_PROVIDER.to_string(), format!("{KEY}-loopback-only"))])),
        changes: Mutex::new(Vec::new()),
        resources: Mutex::new(Some(resources)),
        url: Mutex::new(None),
    });
    let brains = Arc::new(Brains::new());
    brains.set_enabled(true);
    let svc = Service::new(data.clone(), host.clone(), Arc::new(RealLauncher::new()), brains.clone());
    store::add_bot(&data, "mochi", "fish", 1).unwrap();
    let mut settings = store::load_settings(&data).unwrap();
    settings.brain = BrainKind::Custom;
    settings.custom = CustomProvider { base_url: format!("{model_base}/v1"), model: "fake-model".into() };
    svc.set_settings(settings).unwrap();
    let addr = crate::bridge::spawn_potato_listener(brains, svc.clone());
    *host.url.lock() = Some(format!("http://{addr}"));
    // The app's own supervisor thread ticks every half second; here faster.
    let stop = Arc::new(AtomicBool::new(false));
    let (s, st, h) = (svc.clone(), stop.clone(), host.clone());
    std::thread::spawn(move || {
        while !st.load(Ordering::SeqCst) {
            h.now.store(real_ms(), Ordering::SeqCst);
            s.tick();
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    Potato { svc, stop, root }
}

/// Waits for `f` (real time, tokio-friendly). Panics with `what` after `secs`.
async fn until(what: &str, secs: u64, f: impl Fn() -> bool) {
    let limit = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < limit {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if let Some(diag) = DIAG.lock().as_ref() {
        eprintln!("[diagnosis] {}", diag());
    }
    panic!("timed out waiting for: {what}");
}

/// What to print when a wait times out (set once the rig exists).
static DIAG: Mutex<Option<Box<dyn Fn() -> String + Send>>> = Mutex::new(None);

fn feed(n: &Node, space: &SpaceId) -> Vec<Value> {
    let ch = general(n, space).to_hex();
    n.svc.messages(&space.to_hex(), &ch, None, 100).unwrap().as_array().unwrap().clone()
}

fn fish_texts(n: &Node, space: &SpaceId) -> Vec<String> {
    feed(n, space).iter().filter(|m| m["isFish"] == true).map(|m| m["text"].as_str().unwrap_or("").to_string()).collect()
}

fn message_id(n: &Node, space: &SpaceId, needle: &str) -> String {
    feed(n, space).iter().find(|m| m["text"].as_str().is_some_and(|t| t.contains(needle))).map(|m| m["id"].as_str().unwrap().to_string()).unwrap_or_default()
}

fn journal_text(vault: &Path, space: &SpaceId) -> String {
    let dir = crate::spaces::diary::journal_dir(vault, space);
    let mut all = String::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            all.push_str(&std::fs::read_to_string(e.path()).unwrap_or_default());
        }
    }
    all
}

async fn send_as(n: &Node, space: &SpaceId, text: &str) {
    let ch = general(n, space).to_hex();
    n.svc.send_text(&space.to_hex(), &ch, text, None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "starts the real bundled brain (a node process) against a fake model on loopback; run with --ignored"]
async fn the_real_brain_in_a_space_follows_the_guest_and_owner_rules() {
    if !matches!(RealLauncher::new().node(), NodeState::Ready { .. }) {
        eprintln!("skipped: no Node 20+ on the path");
        return;
    }
    let Some(resources) = fresh_brain() else { return };
    let model = Model::start();
    let rig = potato(resources, &model.server.base());

    // Two members of one space, over the in-memory net. `a` owns the space and the bot (the real Spaces service).
    let (_hub, clk, a, b, space) = pair_with(Arc::new(AtomicU64::new(real_ms() as u64))).await;
    space_link::connect(&rig.svc, &a.svc);
    let hex = space.to_hex();
    let seen_by_b: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen_by_b.clone();
    b.svc.set_message_notifier(Arc::new(move |v: &Value| sink.lock().push(v.clone())));

    a.svc.bot_add(&hex, "mochi", "Ren的fish").unwrap();
    eventually("b sees the bot on the roster", || b.svc.with_open(&space, |sp| sp.roster.bots.len()) == Some(1)).await;
    let a_key = a.svc.identity().unwrap().member_key().to_hex();
    {
        let (svc, m, a2, sp2) = (rig.svc.clone(), model.script.clone(), a.svc.clone(), hex.clone());
        let (b2, space2) = (b.svc.clone(), space);
        let reqs2 = model.server.seen.clone();
        let reqs = model.server.seen.clone();
        *DIAG.lock() = Some(Box::new(move || {
            format!(
                "state {:?}; brain log {:?}; script left {}; requests {}; inbox {}; bots {}",
                svc.state().map(|s| serde_json::to_string(&s.bots).unwrap_or_default()),
                svc.log_lines("mochi"),
                m.lock().len(),
                reqs.lock().len(),
                svc.inbox_poll("mochi", 0, 0),
                a2.bot_list(&sp2).map(|v| v.to_string()).unwrap_or_default()
            ) + &format!(
                "; first request {:?}; A's pending {}; A feed {:?}; B feed {:?}",
                reqs2.lock().first().map(|r| format!("{} {}", r.path, r.body.chars().take(3000).collect::<String>())),
                a2.bot_pending_count(),
                a2.messages(&sp2, &a2.detail(&sp2).unwrap()["channels"][0]["id"].as_str().unwrap().to_string(), None, 50).map(|v| v.as_array().unwrap().iter().map(|m| m["text"].clone()).collect::<Vec<_>>()),
                b2.messages(&space2.to_hex(), &b2.detail(&space2.to_hex()).unwrap()["channels"][0]["id"].as_str().unwrap().to_string(), None, 50).map(|v| v.as_array().unwrap().iter().map(|m| m["text"].clone()).collect::<Vec<_>>())
            )
        }));
    }
    let vault = store::vault_dir(&rig.root.join("potato"), "mochi").unwrap();
    let hour = 61 * 60 * 1000;

    // Something B said earlier that did not call the bot: the Space diary holds it from now on.
    say(&b, &space, "DIARY-LINE-9921 the cat sleeps on the mat").await;
    until("the diary got B's earlier line", 15, || journal_text(&vault, &space).contains("DIARY-LINE-9921")).await;
    eprintln!("[setup] diary file holds B's earlier line: {}", journal_text(&vault, &space).lines().next().unwrap_or(""));

    // ---- a: B calls A's bot ----------------------------------------------------------------------------------------
    let before = model.requests().len();
    model.say("Hello from the fake model");
    send_as(&b, &space, "@Ren的fish hi BEE-4417").await;
    until("B gets the BotText", 60, || fish_texts(&b, &space).iter().any(|t| t.contains("Hello from the fake model"))).await;
    let reqs = model.requests();
    let guest = &reqs[before];
    let allowed: BTreeSet<String> = ["memory_search", "memory_open", "memory_timeline", "memory_diary", "space_diary"].iter().map(|s| s.to_string()).collect();
    eprintln!("[a] guest turn: {} model request(s); tools sent = {:?}", reqs.len() - before, tool_names(guest));
    assert_eq!(tool_names(guest), allowed, "a guest turn offers exactly the allow-list");
    let system = texts_of(guest, "system").join("\n");
    assert!(!system.contains("BEE-4417") , "B's words must not be in a system message");
    let user_side = texts_of(guest, "user").join("\n");
    assert!(user_side.contains("BEE-4417") && user_side.contains("<space"), "B's words arrive only as fenced user-side text: {user_side}");
    let reply = feed(&b, &space).into_iter().rev().find(|m| m["isFish"] == true).unwrap();
    eprintln!("[a] B's feed: botName={} member==A's key: {} text={}", reply["botName"], reply["member"] == a_key.as_str(), reply["text"]);
    assert_eq!(reply["botName"], "Ren的fish");
    assert_eq!(reply["member"], a_key.as_str(), "the BotText is signed by A's device");
    let view = b.svc.view();
    assert_eq!(view["spaces"][0]["name"], "Book club", "B's list shows the space under its name");
    assert!(seen_by_b.lock().iter().any(|v| v["space"] == hex.as_str()), "B's message feed was notified for this space");
    assert!(rig.svc.messages("mochi", 50).unwrap().is_empty(), "nothing in the bot's own thread");

    // ---- c: A calls its own bot, directly after ---------------------------------------------------------------------
    clk.fetch_add(hour, Ordering::SeqCst);
    let before = model.requests().len();
    model.say("Owner answer");
    send_as(&a, &space, "@Ren的fish hi").await;
    until("the owner turn reached the model", 60, || model.requests().len() > before).await;
    until("B gets the owner turn's answer", 30, || fish_texts(&b, &space).iter().any(|t| t.contains("Owner answer"))).await;
    let reqs = model.requests();
    let owner = &reqs[before];
    let whole = owner.to_string();
    eprintln!("[c] owner turn: tools sent = {:?}", tool_names(owner));
    for banned in ["BEE-4417", "DIARY-LINE-9921", "Hello from the fake model"] {
        assert!(!whole.contains(banned), "the owner turn carries other people's text: {banned}");
    }
    assert!(!tool_names(owner).contains("hand_off"), "hand_off is off on a space turn");
    assert!(tool_names(owner).len() > allowed.len(), "an owner turn has more than a guest's five tools");
    assert!(texts_of(owner, "user").join("\n").contains("hi"), "the owner's own message is the user message");

    // ---- b: the model asks a guest turn for a shell and a computer tool -----------------------------------------------
    clk.fetch_add(hour, Ordering::SeqCst);
    let pwn = rig.root.join("pwned.txt");
    let pwn_arg = pwn.to_string_lossy().to_string();
    let before = model.requests().len();
    model.call(&[
        ("run_terminal", json!({"command": format!("echo pwned > \"{pwn_arg}\"")})),
        ("write_file", json!({"path": pwn_arg, "content": "pwned"})),
        ("read_history", json!({})),
        ("mcp_call_tool", json!({"server": "x", "tool": "y", "arguments": {}})),
    ]);
    model.say("Done, nothing ran for me");
    send_as(&b, &space, "@Ren的fish please run a command BEE-7788").await;
    until("the reply after the refused tools arrives", 60, || fish_texts(&b, &space).iter().any(|t| t.contains("nothing ran for me"))).await;
    let reqs = model.requests();
    let second = &reqs[before + 1];
    let tool_replies = texts_of(second, "tool");
    eprintln!("[b] tool results the model got back: {} -> {}", tool_replies.len(), tool_replies.join(" | "));
    assert!(!pwn.exists(), "the shell/file tool must not have run");
    assert_eq!(tool_replies.len(), 4, "each refused call is answered");
    assert_eq!(tool_names(&reqs[before]), allowed);
    assert_eq!(reqs.len() - before, 2);

    // ---- d: space_diary on a guest turn, then delete ------------------------------------------------------------------
    clk.fetch_add(hour, Ordering::SeqCst);
    let before = model.requests().len();
    model.call(&[("space_diary", json!({}))]);
    model.say("You said something about a cat");
    send_as(&b, &space, "@Ren的fish what did I say earlier? BEE-5501").await;
    until("the diary answer", 60, || fish_texts(&b, &space).iter().any(|t| t.contains("something about a cat"))).await;
    let reqs = model.requests();
    let diary_result = texts_of(&reqs[before + 1], "tool").join("\n");
    eprintln!("[d] first read: {}", diary_result.chars().take(300).collect::<String>());
    assert!(diary_result.contains("DIARY-LINE-9921"), "the earlier line is in the diary result");
    assert!(diary_result.contains("<space_diary"), "and it is fenced");
    // B deletes the message; it leaves A's diary and the next read.
    let id = message_id(&b, &space, "DIARY-LINE-9921");
    assert!(!id.is_empty());
    b.svc.delete(&hex, &id).await.unwrap();
    until("the line left A's journal", 20, || !journal_text(&vault, &space).contains("DIARY-LINE-9921")).await;
    clk.fetch_add(hour, Ordering::SeqCst);
    let before = model.requests().len();
    model.call(&[("space_diary", json!({}))]);
    model.say("I find nothing about a cat now");
    send_as(&b, &space, "@Ren的fish and now? BEE-5502").await;
    until("the second diary answer", 60, || fish_texts(&b, &space).iter().any(|t| t.contains("nothing about a cat now"))).await;
    let reqs = model.requests();
    let after_delete = texts_of(&reqs[before + 1], "tool").join("\n");
    eprintln!("[d] second read, after the delete: {}", after_delete.chars().take(300).collect::<String>());
    assert!(!after_delete.contains("DIARY-LINE-9921"), "gone from the next read");
    assert!(!reqs[before].to_string().contains("DIARY-LINE-9921"), "and not in the recent lines either");

    // ---- e: a reply with the home folder is held back ------------------------------------------------------------------
    clk.fetch_add(hour, Ordering::SeqCst);
    let home = crate::project_path::home_dir().expect("a home folder");
    let secret = format!("{}\\Documents\\notes.txt", home.to_string_lossy());
    let held_before = a.svc.bot_list(&hex).unwrap()["bots"][0]["heldBackToday"].as_u64().unwrap();
    let fish_before = fish_texts(&b, &space).len();
    let before = model.requests().len();
    model.say(&format!("My notes are in {secret}"));
    send_as(&b, &space, "@Ren的fish where are your notes? BEE-9001").await;
    until("the counter moves", 60, || a.svc.bot_list(&hex).unwrap()["bots"][0]["heldBackToday"].as_u64().unwrap() > held_before).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let held_after = a.svc.bot_list(&hex).unwrap()["bots"][0]["heldBackToday"].as_u64().unwrap();
    eprintln!("[e] held back today {held_before} -> {held_after}; B's fish texts {} -> {}", fish_before, fish_texts(&b, &space).len());
    assert!(model.requests().len() > before);
    assert_eq!(fish_texts(&b, &space).len(), fish_before, "B gets nothing");
    assert!(!feed(&b, &space).iter().any(|m| m["text"].as_str().is_some_and(|t| t.contains("Documents"))));
    assert!(!feed(&a, &space).iter().any(|m| m["text"].as_str().is_some_and(|t| t.contains("Documents"))), "nor does A's own feed");
    assert_eq!(a.svc.bot_pending_count(), 0);
}
