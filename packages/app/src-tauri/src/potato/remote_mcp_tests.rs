//! Tool servers by link (`remote_mcp.rs`, and `mcp.rs` / `sandbox_calls.rs` around it). The SSRF cases (address forms, redirects,
//! names that point inside), the speaking of the protocol against a local fake server over real sockets, and the rule that a
//! token is read on the host, sent only to its own origin, kept out of every file and view, and scrubbed from every answer.
//! Tokens here are fake strings; no real account or server is used.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::bot_settings;
use super::fake_http::{serve, Reply, Req, Server};
use super::mcp;
use super::protocol::Trust;
use super::remote_mcp::{self, check_endpoint, post_following, RemoteError, Resp, Wire};
use super::sandbox::lease::ScreenLease;
use super::sandbox::{FakeSandbox, Recorded, Sandbox};
use super::sandbox_calls::{CallError, Calls, Context, Gates, Port};
use super::service_tests::FakeHost;
use super::store;

const TOKEN: &str = "fake-token-5e1c0b9a42";

pub(super) fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-potato-remote-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_token(_refresh: bool) -> Result<Option<String>, RemoteError> {
    Ok(None)
}

fn with_token(_refresh: bool) -> Result<Option<String>, RemoteError> {
    Ok(Some(TOKEN.to_string()))
}

// ---- a fake tool server over real sockets ---------------------------------------------------------

pub(super) fn reply_for(req: &Req, sse: bool) -> Reply {
    if req.method != "POST" {
        return Reply::json(405, "{}");
    }
    let v: Value = serde_json::from_str(&req.body).unwrap_or(json!({}));
    let method = v["method"].as_str().unwrap_or("").to_string();
    let result = match method.as_str() {
        "initialize" => json!({ "protocolVersion": "2025-06-18", "capabilities": { "tools": {} }, "serverInfo": { "name": "fake", "version": "1" } }),
        "notifications/initialized" => return Reply { status: 202, headers: vec![], body: String::new() },
        "tools/list" => json!({ "tools": [
            { "name": "get_issue", "description": "Read one issue", "inputSchema": { "type": "object" } },
            { "name": "create_issue", "description": "Make an issue", "inputSchema": { "type": "object" } },
        ] }),
        "tools/call" => {
            let who = req.header("authorization").unwrap_or("no header");
            json!({ "content": [{ "type": "text", "text": format!("called {} with {who}", v["params"]["name"].as_str().unwrap_or("?")) }], "isError": false })
        }
        _ => return Reply::json(200, &json!({ "jsonrpc": "2.0", "id": v["id"], "error": { "code": -32601, "message": "no such method" } }).to_string()),
    };
    let message = json!({ "jsonrpc": "2.0", "id": v["id"], "result": result }).to_string();
    let reply = if sse { Reply::sse(&format!("event: message\ndata: {message}\n\n")) } else { Reply::json(200, &message) };
    if method == "initialize" {
        reply.with("Mcp-Session-Id", "sess-1")
    } else {
        reply
    }
}

fn mcp_server(sse: bool) -> Server {
    serve(move |req| reply_for(req, sse))
}

fn live() -> remote_mcp::LiveWire {
    remote_mcp::LiveWire::new(true)
}

// ---- the address -----------------------------------------------------------------------------------

#[test]
fn only_public_https_links_are_accepted_without_the_local_tick() {
    assert!(check_endpoint("https://mcp.example.com/mcp", false).is_ok());
    assert!(check_endpoint("  https://mcp.example.com:8443/a/b?x=1  ", false).is_ok());
    for bad in [
        "http://mcp.example.com/mcp",
        "ftp://mcp.example.com/mcp",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "mcp.example.com/mcp",
        "https://localhost/mcp",
        "https://LOCALHOST:8080/mcp",
        "https://127.0.0.1/mcp",
        "https://127.1/mcp",
        "https://0x7f.0.0.1/mcp",
        "https://2130706433/mcp",
        "https://017700000001/mcp",
        "https://0.0.0.0/mcp",
        "https://[::1]/mcp",
        "https://[::ffff:127.0.0.1]/mcp",
        "https://[::ffff:10.0.0.1]/mcp",
        "https://[fe80::1]/mcp",
        "https://[fd00::1]/mcp",
        "https://10.0.0.5/mcp",
        "https://172.16.4.4/mcp",
        "https://192.168.1.10/mcp",
        "https://169.254.169.254/latest/meta-data",
        "https://100.64.0.1/mcp",
        "https://printer.local/mcp",
        "https://nas.lan/mcp",
        "https://wiki.internal/mcp",
        "https://intranet/mcp",
        "https://app.127.0.0.1.nip.io/mcp",
        "https://user:pass@mcp.example.com/mcp",
        "https://mcp.example.com/mcp#frag",
        "https://",
        "",
    ] {
        assert!(check_endpoint(bad, false).is_err(), "{bad} must be refused");
    }
}

#[test]
fn the_local_tick_opens_this_computer_and_the_own_network_but_plain_http_stays_off_the_internet() {
    for ok in ["http://127.0.0.1:8080/mcp", "https://127.0.0.1:8443/mcp", "http://localhost:3000/mcp", "http://192.168.1.20/mcp", "http://nas.lan/mcp", "https://mcp.example.com/mcp"] {
        assert!(check_endpoint(ok, true).is_ok(), "{ok} is allowed with Local");
    }
    // A public address over plain http would send the token in the clear across the internet: never, Local or not.
    assert_eq!(check_endpoint("http://mcp.example.com/mcp", true).unwrap_err().code, "MCP_URL_SCHEME");
    assert_eq!(check_endpoint("http://8.8.8.8/mcp", true).unwrap_err().code, "MCP_URL_SCHEME");
    // Local does not turn off the other checks.
    assert!(check_endpoint("ftp://127.0.0.1/mcp", true).is_err());
    assert!(check_endpoint("http://user:pw@127.0.0.1/mcp", true).is_err());
}

// ---- redirects (a scripted wire, no network) -------------------------------------------------------

#[derive(Default)]
struct Script {
    replies: Mutex<VecDeque<Resp>>,
    seen: Mutex<Vec<(String, Vec<(String, String)>)>>,
}

impl Script {
    fn with(replies: Vec<Resp>) -> Script {
        Script { replies: Mutex::new(replies.into()), seen: Mutex::default() }
    }
    fn urls(&self) -> Vec<String> {
        self.seen.lock().iter().map(|(u, _)| u.clone()).collect()
    }
    fn has_auth(&self, at: usize) -> bool {
        self.seen.lock()[at].1.iter().any(|(n, _)| n.eq_ignore_ascii_case("authorization"))
    }
}

impl Wire for Script {
    fn post(&self, url: &str, headers: &[(String, String)], _body: &str, _want: Option<i64>) -> Result<Resp, RemoteError> {
        self.seen.lock().push((url.to_string(), headers.to_vec()));
        Ok(self.replies.lock().pop_front().unwrap_or(Resp { status: 200, body: "{}".into(), ..Default::default() }))
    }
}

fn redirect(status: u16, to: &str) -> Resp {
    Resp { status, location: Some(to.to_string()), ..Default::default() }
}

fn auth() -> Vec<(String, String)> {
    vec![("Authorization".to_string(), format!("Bearer {TOKEN}"))]
}

#[test]
fn a_redirect_into_a_private_or_local_place_is_refused_before_it_is_followed() {
    for target in [
        "http://127.0.0.1:9/x",
        "https://127.0.0.1/x",
        "https://169.254.169.254/latest/meta-data/iam",
        "https://10.1.2.3/x",
        "https://192.168.0.1/",
        "https://[::1]/",
        "https://localhost/x",
        "https://2130706433/",
        "http://mcp.example.com/plain",
        "ftp://mcp.example.com/x",
        "file:///etc/passwd",
        "//evil.example/x",
        "elsewhere",
        "https://app.10.0.0.1.nip.io/x",
    ] {
        for status in [301, 302, 303, 307, 308] {
            let wire = Script::with(vec![redirect(status, target)]);
            let err = post_following(&wire, "https://mcp.example.com/mcp", false, auth(), "{}", None).unwrap_err();
            assert!(err.code.starts_with("MCP_"), "{target}: {}", err.code);
            assert_eq!(wire.urls().len(), 1, "{target} was never requested");
        }
    }
}

#[test]
fn a_redirect_to_a_private_place_is_refused_even_after_a_good_hop() {
    let wire = Script::with(vec![redirect(307, "https://mcp2.example.com/mcp"), redirect(307, "https://169.254.169.254/latest")]);
    let err = post_following(&wire, "https://mcp.example.com/mcp", false, auth(), "{}", None).unwrap_err();
    assert_eq!(err.code, "MCP_URL_PRIVATE");
    assert_eq!(wire.urls(), vec!["https://mcp.example.com/mcp", "https://mcp2.example.com/mcp"]);
}

#[test]
fn the_token_goes_only_where_it_was_given_and_never_to_another_origin() {
    let wire = Script::with(vec![redirect(307, "/v2/mcp"), redirect(307, "https://other.example.org/mcp"), Resp { status: 200, body: "{}".into(), ..Default::default() }]);
    let (resp, last) = post_following(&wire, "https://mcp.example.com/mcp", false, auth(), "{}", None).unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(last, "https://other.example.org/mcp");
    assert_eq!(wire.urls(), vec!["https://mcp.example.com/mcp", "https://mcp.example.com/v2/mcp", "https://other.example.org/mcp"]);
    assert!(wire.has_auth(0));
    assert!(wire.has_auth(1), "the same origin keeps it");
    assert!(!wire.has_auth(2), "another origin never gets it");
}

#[test]
fn a_loop_of_redirects_ends() {
    let wire = Script::with((0..10).map(|_| redirect(302, "/again")).collect());
    let err = post_following(&wire, "https://mcp.example.com/mcp", false, auth(), "{}", None).unwrap_err();
    assert_eq!(err.code, "MCP_REDIRECT");
    assert_eq!(wire.urls().len(), remote_mcp::MAX_REDIRECTS + 1);
}

// ---- names that point inside, and redirects over real sockets --------------------------------------

#[test]
fn a_name_or_address_that_resolves_inside_is_refused_at_connect_time_without_the_tick() {
    let server = mcp_server(false);
    let strict = remote_mcp::LiveWire::new(false);
    // The checks of the link are skipped here on purpose: this is the last line, the connection itself (a name that points
    // at 127.0.0.1 after the link was approved).
    for url in [format!("http://127.0.0.1:{}/mcp", server.port), format!("http://localhost:{}/mcp", server.port)] {
        let err = strict.post(&url, &[], "{}", None).unwrap_err();
        assert_eq!(err.code, "MCP_URL_PRIVATE", "{url}");
    }
    assert!(server.requests().is_empty(), "nothing reached the server");
    // With the tick the same connection is made.
    let resp = live().post(&format!("http://127.0.0.1:{}/mcp", server.port), &[], "{}", None).unwrap();
    assert_eq!(resp.status, 200);
}

#[test]
fn over_real_sockets_a_redirect_to_another_origin_leaves_the_token_behind() {
    let target = serve(|_| Reply::json(200, "{}"));
    let to = format!("{}/landing", target.base());
    let first = serve(move |_| Reply::redirect(307, &to));
    let (resp, _) = post_following(&live(), &format!("{}/mcp", first.base()), true, auth(), "{}", None).unwrap();
    assert_eq!(resp.status, 200);
    assert!(first.requests()[0].header("authorization").is_some());
    let landed = target.requests();
    assert_eq!(landed.len(), 1);
    assert!(landed[0].header("authorization").is_none(), "the other origin never saw the token");
}

#[test]
fn over_real_sockets_a_redirect_to_a_non_web_address_is_refused() {
    for target in ["file:///c:/windows/win.ini", "ftp://127.0.0.1/x", "gopher://127.0.0.1/x"] {
        let to = target.to_string();
        let first = serve(move |_| Reply::redirect(302, &to));
        let err = post_following(&live(), &format!("{}/mcp", first.base()), true, auth(), "{}", None).unwrap_err();
        assert_eq!(err.code, "MCP_URL_SCHEME", "{target}");
    }
}

// ---- the protocol ----------------------------------------------------------------------------------

#[test]
fn a_session_starts_lists_and_calls_whether_the_answer_is_json_or_an_event_stream() {
    for sse in [false, true] {
        let server = mcp_server(sse);
        let url = format!("{}/mcp", server.base());
        let tools = remote_mcp::list_tools(&live(), &url, true, &no_token).unwrap();
        assert_eq!(tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["get_issue", "create_issue"]);
        assert_eq!(tools[0].description, "Read one issue");
        let out = remote_mcp::call_tool(&live(), &url, true, &no_token, "get_issue", &json!({ "id": 4 })).unwrap();
        assert_eq!(out["content"][0]["text"], "called get_issue with no header");
        assert_eq!(out["isError"], false);
    }
}

#[test]
fn the_session_id_and_the_token_are_sent_on_every_later_request_and_only_as_headers() {
    let server = mcp_server(false);
    let url = format!("{}/mcp", server.base());
    let out = remote_mcp::call_tool(&live(), &url, true, &with_token, "get_issue", &json!({})).unwrap();
    assert_eq!(out["content"][0]["text"], format!("called get_issue with Bearer {TOKEN}"));
    let seen = server.requests();
    let methods: Vec<String> = seen.iter().map(|r| serde_json::from_str::<Value>(&r.body).unwrap()["method"].as_str().unwrap().to_string()).collect();
    assert_eq!(methods, vec!["initialize", "notifications/initialized", "tools/call"]);
    assert!(seen[0].header("mcp-session-id").is_none());
    assert_eq!(seen[1].header("mcp-session-id"), Some("sess-1"));
    assert_eq!(seen[2].header("mcp-session-id"), Some("sess-1"));
    for r in &seen {
        assert_eq!(r.header("authorization"), Some(format!("Bearer {TOKEN}").as_str()));
        assert!(!r.path.contains(TOKEN) && !r.body.contains(TOKEN), "never in the link or the body");
        assert!(r.header("accept").unwrap().contains("text/event-stream"));
    }
}

#[test]
fn a_server_that_wants_a_sign_in_gets_a_plain_answer_that_holds_no_token() {
    let server = serve(|_| Reply::json(401, "{}").with("WWW-Authenticate", "Bearer realm=\"x\""));
    let err = remote_mcp::list_tools(&live(), &format!("{}/mcp", server.base()), true, &with_token).unwrap_err();
    assert_eq!(err.code, "MCP_NEEDS_SIGN_IN");
    assert!(!err.message.contains(TOKEN));
    // One retry with a fresh token only when the source gives a different one; the same one is not sent twice.
    assert_eq!(server.requests().len(), 1 + 0, "the same token is not retried");
}

#[test]
fn a_401_is_retried_once_with_a_refreshed_token() {
    let server = serve(|req| match req.header("authorization") {
        Some("Bearer fresh-token-1") => reply_for(req, false),
        _ => Reply::json(401, "{}"),
    });
    let source = |refresh: bool| -> Result<Option<String>, RemoteError> { Ok(Some(if refresh { "fresh-token-1".to_string() } else { "old-token-0".to_string() })) };
    let tools = remote_mcp::list_tools(&live(), &format!("{}/mcp", server.base()), true, &source).unwrap();
    assert_eq!(tools.len(), 2);
}

#[test]
fn server_errors_and_bad_answers_are_plain_sentences() {
    let server = serve(|_| Reply::json(500, "boom"));
    assert_eq!(remote_mcp::list_tools(&live(), &format!("{}/mcp", server.base()), true, &no_token).unwrap_err().code, "MCP_HTTP");
    let junk = serve(|_| Reply::json(200, "not json"));
    assert_eq!(remote_mcp::list_tools(&live(), &format!("{}/mcp", junk.base()), true, &no_token).unwrap_err().code, "MCP_BAD_ANSWER");
    let dead = serve(|_| Reply::json(200, "{}"));
    let url = format!("{}/mcp", dead.base());
    assert_eq!(remote_mcp::list_tools(&live(), &url, true, &no_token).unwrap_err().code, "MCP_BAD_ANSWER", "an answer with no result");
}

#[test]
fn an_event_stream_is_read_until_the_wanted_message_and_no_further() {
    let text = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\ndata: {\"id\":8,\"result\":{}}\n\n";
    let got = remote_mcp::read_event_stream(std::io::Cursor::new(text.as_bytes()), Some(7));
    assert_eq!(serde_json::from_str::<Value>(&got).unwrap()["result"]["ok"], true);
    assert_eq!(remote_mcp::read_event_stream(std::io::Cursor::new(text.as_bytes()), Some(99)), "", "never arrived");
    // A message split over two data lines is joined.
    let split = "data: {\"jsonrpc\":\"2.0\",\ndata: \"id\":3,\"result\":{}}\n\n";
    assert!(remote_mcp::read_event_stream(std::io::Cursor::new(split.as_bytes()), Some(3)).contains("\"id\":3"));
}

// ---- the settings, the credential store, the views -------------------------------------------------

fn running() -> FakeSandbox {
    let sandbox = FakeSandbox::new();
    sandbox.start().unwrap();
    sandbox
}

#[test]
fn adding_by_link_keeps_the_token_in_the_credential_store_and_out_of_every_file_and_view() {
    let root = temp("add");
    let sandbox = running();
    mcp::add_remote(&root, "rb1", "tracker", "https://mcp.example.com/mcp", false, Some(format!("  {TOKEN}  "))).unwrap();
    let settings = bot_settings::load(&root, "rb1").unwrap();
    let server = &settings.mcp_servers[0];
    assert_eq!((server.url.as_str(), server.local, server.enabled, server.connector), ("https://mcp.example.com/mcp", false, true, true));
    assert_eq!(server.env_keys, vec!["TOKEN".to_string()]);
    let on_disk = std::fs::read_to_string(bot_settings::settings_dir(&root).join("rb1.json")).unwrap();
    assert!(!on_disk.contains(TOKEN));
    let view = serde_json::to_string(&mcp::overview(&root, "rb1", &sandbox).unwrap()).unwrap();
    assert!(!view.contains(TOKEN) && view.contains("https://mcp.example.com/mcp"));
    let kept = crate::secrets::backend().get(&mcp::env_secret("rb1", "tracker")).unwrap().unwrap();
    assert!(kept.contains(TOKEN));
    // It never joins the computer's list, plain or secret: the computer has no part in it.
    assert!(mcp::guest_config(&root).unwrap().as_array().unwrap().is_empty());
    mcp::push(&root, &sandbox).unwrap();
    assert!(sandbox.last_secret_config().map_or(true, |c| c.as_array().unwrap().is_empty()));
    assert!(!sandbox.calls().iter().any(|c| matches!(c, Recorded::SecretConfig(names) if names.contains(&"tracker".to_string()))));
}

#[test]
fn adding_by_link_refuses_a_bad_name_link_or_token_and_a_second_server_of_the_same_name() {
    let root = temp("add-bad");
    let add = |name: &str, url: &str, local: bool, token: Option<&str>| mcp::add_remote(&root, "rb2", name, url, local, token.map(str::to_string));
    assert_eq!(add("Bad Name", "https://mcp.example.com/mcp", false, None).unwrap_err().code, "MCP_BAD_NAME");
    assert_eq!(add("computer", "https://mcp.example.com/mcp", false, None).unwrap_err().code, "MCP_BAD_NAME");
    assert_eq!(add("ok", "http://mcp.example.com/mcp", false, None).unwrap_err().code, "MCP_URL_SCHEME");
    assert_eq!(add("ok", "https://127.0.0.1/mcp", false, None).unwrap_err().code, "MCP_URL_PRIVATE");
    assert_eq!(add("ok", "https://mcp.example.com/mcp", false, Some("line\nbreak")).unwrap_err().code, "MCP_BAD_ENV");
    assert!(bot_settings::load(&root, "rb2").unwrap().mcp_servers.is_empty(), "a refused add leaves nothing");
    add("ok", "https://mcp.example.com/mcp", false, None).unwrap();
    assert_eq!(add("ok", "https://other.example.com/mcp", false, None).unwrap_err().code, "MCP_NAME_TAKEN");
    assert!(crate::secrets::backend().get(&mcp::env_secret("rb2", "ok")).unwrap().is_none(), "no token, nothing stored");
    add("local1", "http://127.0.0.1:9/mcp", true, None).unwrap();
    assert!(bot_settings::load(&root, "rb2").unwrap().mcp_servers[1].local);
}

#[test]
fn removing_a_server_by_link_deletes_its_token() {
    let root = temp("remove");
    let sandbox = running();
    mcp::add_remote(&root, "rb3", "tracker", "https://mcp.example.com/mcp", false, Some(TOKEN.to_string())).unwrap();
    mcp::remove_server(&root, "rb3", "tracker", &sandbox).unwrap();
    assert!(crate::secrets::backend().get(&mcp::env_secret("rb3", "tracker")).unwrap().is_none());
}

#[test]
fn test_asks_the_server_from_this_pc_and_does_not_need_the_computer() {
    let root = temp("test");
    let server = mcp_server(false);
    let sandbox = FakeSandbox::new(); // stopped
    mcp::add_remote(&root, "rb4", "tracker", &format!("{}/mcp", server.base()), true, Some(TOKEN.to_string())).unwrap();
    let result = mcp::test_server(&root, "rb4", "tracker", &sandbox).unwrap();
    assert!(result.ok, "{:?}", result.error);
    assert_eq!(result.tools, vec!["get_issue".to_string(), "create_issue".to_string()]);
    // A server that fails: the sentence carries no token form.
    let down = serve(move |_| Reply::json(401, "{}"));
    mcp::add_remote(&root, "rb4", "down", &format!("{}/mcp", down.base()), true, Some(TOKEN.to_string())).unwrap();
    let failed = mcp::test_server(&root, "rb4", "down", &sandbox).unwrap();
    assert!(!failed.ok);
    assert!(!serde_json::to_string(&failed).unwrap().contains(TOKEN));
}

// ---- through the gate -------------------------------------------------------------------------------

pub(super) struct Rig {
    pub(super) calls: Calls,
    pub(super) port: Port,
    pub(super) root: PathBuf,
}

impl Rig {
    pub(super) fn new(name: &str) -> Rig {
        let root = temp(name);
        let host = Arc::new(FakeHost {
            now: AtomicI64::new(1_000),
            secrets: Mutex::new(Default::default()),
            changes: Mutex::new(Vec::new()),
            resources: Mutex::new(None),
            url: Mutex::new(None),
        });
        let fake = Arc::new(FakeSandbox::new());
        fake.start().unwrap();
        let port = Port { sandbox: fake, lease: Arc::new(Mutex::new(ScreenLease::new())) };
        Rig { calls: Calls::new(root.clone(), host), port, root }
    }

    pub(super) fn ctx(&self, bot: &str) -> Context {
        // A bot with a record and a folder, so a card can be raised for it.
        if store::load_bots(&self.root).map(|b| b.iter().all(|x| x.id != bot)).unwrap_or(true) {
            store::add_bot(&self.root, bot, "fish", 1).unwrap();
            store::ensure_brain_dirs(&self.root, bot).unwrap();
        }
        Context { bot: bot.to_string(), conversation: format!("app:dm:{bot}"), trust: Trust::OwnerPc, want_image: false }
    }

    pub(super) fn call(&self, ctx: &Context, tool: &str, args: Value) -> Result<Value, CallError> {
        self.calls.execute(Some(&self.port), Gates { bot_on: true, sandbox_on: true }, ctx, tool, &args)
    }

    pub(super) fn full(&self, bot: &str) {
        bot_settings::update(&self.root, bot, |s| {
            s.level = super::risk::Level::Full;
            Ok(())
        })
        .unwrap();
    }

    fn open_cards(&self, bot: &str) -> usize {
        super::cards::open_cards(&self.root, bot).len()
    }
}

#[test]
fn a_call_to_a_server_by_link_goes_through_the_card_gate_like_a_connector() {
    let r = Rig::new("gate-card");
    let server = mcp_server(false);
    mcp::add_remote(&r.root, "rg1", "tracker", &format!("{}/mcp", server.base()), true, Some(TOKEN.to_string())).unwrap();
    let ctx = r.ctx("rg1");
    // A change asks the owner first and does not reach the server.
    let asked = r.call(&ctx, "mcp_call_tool", json!({ "server": "tracker", "tool": "create_issue", "arguments": {} })).unwrap_err();
    assert_eq!(asked.code, "APPROVAL_PENDING");
    assert_eq!(r.open_cards("rg1"), 1);
    // A read asks too, until Always allow reads is on for that connector.
    let read = r.call(&ctx, "mcp_call_tool", json!({ "server": "tracker", "tool": "get_issue", "arguments": {} })).unwrap_err();
    assert_eq!(read.code, "APPROVAL_PENDING");
    assert!(server.requests().is_empty(), "nothing was sent before the owner said yes");
    mcp::set_always_allow_reads(&r.root, "rg1", "tracker", true).unwrap();
    let answer = r.call(&ctx, "mcp_call_tool", json!({ "server": "tracker", "tool": "get_issue", "arguments": {} })).unwrap();
    assert_eq!(answer["ok"], true);
    assert!(!answer.to_string().contains(TOKEN), "the server echoed its header, and the token was taken out");
    assert!(answer["text"].as_str().unwrap().contains("[hidden]"));
}

#[test]
fn a_server_by_link_is_for_the_owners_private_chat_only() {
    let r = Rig::new("gate-room");
    let server = mcp_server(false);
    mcp::add_remote(&r.root, "rg2", "tracker", &format!("{}/mcp", server.base()), true, Some(TOKEN.to_string())).unwrap();
    r.full("rg2");
    let call = |ctx: &Context| r.call(ctx, "mcp_call_tool", json!({ "server": "tracker", "tool": "get_issue", "arguments": {} }));
    let mut room = r.ctx("rg2");
    room.conversation = store::app_room_conversation("r0123456789ab");
    assert_eq!(call(&room).unwrap_err().code, "TOOL_NOT_ALLOWED");
    let mut stranger = r.ctx("rg2");
    stranger.trust = Trust::Others;
    assert!(call(&stranger).is_err());
    // Another bot has no such server and cannot borrow this one's token.
    let _ = call(&r.ctx("rg2-other"));
    assert!(server.requests().is_empty(), "none of those reached the server");
    // The owner's own chat works.
    assert_eq!(call(&r.ctx("rg2")).unwrap()["ok"], true);
    // Switched off: refused.
    mcp::set_enabled(&r.root, "rg2", "tracker", false, r.port.sandbox.as_ref()).unwrap();
    assert_eq!(call(&r.ctx("rg2")).unwrap_err().code, "CONNECTOR_NOT_CONNECTED");
}

#[test]
fn the_tools_list_shows_the_bots_servers_by_link_to_the_owner_only() {
    let r = Rig::new("gate-list");
    let server = mcp_server(false);
    mcp::add_remote(&r.root, "rg3", "tracker", &format!("{}/mcp", server.base()), true, None).unwrap();
    let list = r.call(&r.ctx("rg3"), "mcp_list_tools", json!({})).unwrap();
    let entry = list["servers"].as_array().unwrap().iter().find(|s| s["name"] == "tracker").unwrap();
    assert_eq!(entry["status"], "ready");
    assert_eq!(entry["tools"].as_array().unwrap().len(), 2);
    r.calls.end_run(Some(&r.port), "rg3", "app:dm:rg3");
    let mut room = r.ctx("rg3");
    room.conversation = store::app_room_conversation("r0123456789ab");
    let seen = r.call(&room, "mcp_list_tools", json!({})).unwrap();
    assert!(seen["servers"].as_array().unwrap().iter().all(|s| s["name"] != "tracker"));
    r.calls.end_run(Some(&r.port), "rg3", &room.conversation);
    // A server that fails shows as failed with a sentence, not as an error of the whole list.
    let down = serve(|_| Reply::json(500, "no"));
    mcp::add_remote(&r.root, "rg3", "broken", &format!("{}/mcp", down.base()), true, None).unwrap();
    let list = r.call(&r.ctx("rg3"), "mcp_list_tools", json!({})).unwrap();
    let broken = list["servers"].as_array().unwrap().iter().find(|s| s["name"] == "broken").unwrap();
    assert_eq!(broken["status"], "failed");
}

#[test]
fn a_server_by_link_does_not_take_the_screen() {
    let r = Rig::new("gate-screen");
    let server = mcp_server(false);
    mcp::add_remote(&r.root, "rg4", "tracker", &format!("{}/mcp", server.base()), true, None).unwrap();
    r.full("rg4");
    // The person holds the screen: a screen call is turned away, a call to a server by link is not.
    let _ = r.port.lease.lock().take_over(1_000);
    let call = r.call(&r.ctx("rg4"), "mcp_call_tool", json!({ "server": "tracker", "tool": "get_issue", "arguments": {} }));
    assert!(call.is_ok(), "{call:?}");
}

// ---- per-bot tool lists ------------------------------------------------------------------------------

fn lists(root: &std::path::Path, bot: &str, name: &str, allow: &[&str], block: &[&str]) -> store::StoreResult<()> {
    mcp::set_tool_lists(root, bot, name, allow.iter().map(|s| s.to_string()).collect(), block.iter().map(|s| s.to_string()).collect())
}

#[test]
fn a_tool_is_allowed_when_it_is_not_blocked_and_is_listed_if_there_is_an_allow_list() {
    let mut s = bot_settings::McpServer::default();
    assert!(s.tool_allowed("anything"), "no lists: everything");
    s.allow_tools = vec!["get_issue".into(), "create_issue".into()];
    assert!(s.tool_allowed("get_issue") && !s.tool_allowed("delete_repo"));
    s.block_tools = vec!["create_issue".into()];
    assert!(s.tool_allowed("get_issue") && !s.tool_allowed("create_issue"), "block beats allow");
    s.allow_tools.clear();
    assert!(s.tool_allowed("delete_repo") && !s.tool_allowed("create_issue"), "block only");
    s.allow_tools = vec!["get_issue".into()];
    assert!(!s.tool_allowed("Get_Issue"));
}

#[test]
fn setting_the_lists_checks_names_and_keeps_them_out_of_nothing_but_the_settings_file() {
    let root = temp("lists-set");
    mcp::add_remote(&root, "tl1", "tracker", "https://mcp.example.com/mcp", false, None).unwrap();
    lists(&root, "tl1", "tracker", &["get_issue", " get_issue ", "list_issues"], &["delete_issue"]).unwrap();
    let server = bot_settings::load(&root, "tl1").unwrap().mcp_servers.remove(0);
    assert_eq!(server.allow_tools, vec!["get_issue".to_string(), "list_issues".to_string()], "trimmed and without repeats");
    assert_eq!(server.block_tools, vec!["delete_issue".to_string()]);
    assert_eq!(lists(&root, "tl1", "tracker", &[""], &[]).unwrap_err().code, "MCP_BAD_TOOL");
    assert_eq!(lists(&root, "tl1", "tracker", &["a\nb"], &[]).unwrap_err().code, "MCP_BAD_TOOL");
    assert_eq!(lists(&root, "tl1", "tracker", &[&"x".repeat(201)], &[]).unwrap_err().code, "MCP_BAD_TOOL");
    assert_eq!(lists(&root, "tl1", "nobody", &[], &[]).unwrap_err().code, "MCP_NOT_FOUND");
    // A refused change leaves the saved lists as they were.
    assert_eq!(bot_settings::load(&root, "tl1").unwrap().mcp_servers[0].allow_tools.len(), 2);
    lists(&root, "tl1", "tracker", &[], &[]).unwrap();
    assert!(bot_settings::load(&root, "tl1").unwrap().mcp_servers[0].allow_tools.is_empty());
}

#[test]
fn the_overview_shows_the_lists_so_the_page_can_draw_them() {
    let root = temp("lists-view");
    let sandbox = running();
    mcp::add_remote(&root, "tl2", "tracker", "https://mcp.example.com/mcp", false, None).unwrap();
    lists(&root, "tl2", "tracker", &["get_issue"], &["delete_issue"]).unwrap();
    let view = mcp::overview(&root, "tl2", &sandbox).unwrap();
    assert_eq!(view.servers[0].allow_tools, vec!["get_issue".to_string()]);
    assert_eq!(view.servers[0].block_tools, vec!["delete_issue".to_string()]);
}

#[test]
fn a_tool_off_the_list_is_refused_by_the_host_before_any_card_or_request_even_at_the_full_level() {
    let r = Rig::new("lists-gate");
    let server = mcp_server(false);
    mcp::add_remote(&r.root, "tl3", "tracker", &format!("{}/mcp", server.base()), true, Some(TOKEN.to_string())).unwrap();
    r.full("tl3");
    let call = |tool: &str| r.call(&r.ctx("tl3"), "mcp_call_tool", json!({ "server": "tracker", "tool": tool, "arguments": {} }));
    assert_eq!(call("create_issue").unwrap()["ok"], true, "no list yet: allowed");
    let before = server.requests().len();
    lists(&r.root, "tl3", "tracker", &["get_issue"], &[]).unwrap();
    // Checked at call time: the change holds for the very next call.
    let refused = call("create_issue").unwrap_err();
    assert_eq!(refused.code, "TOOL_NOT_ALLOWED");
    assert!(refused.message.contains("switched this tool off"));
    assert_eq!(server.requests().len(), before, "nothing was sent");
    assert_eq!(r.open_cards("tl3"), 0, "no card is made for a tool that is off");
    assert_eq!(call("get_issue").unwrap()["ok"], true);
    // A block beats the allow list.
    lists(&r.root, "tl3", "tracker", &["get_issue", "create_issue"], &["create_issue"]).unwrap();
    assert_eq!(call("create_issue").unwrap_err().code, "TOOL_NOT_ALLOWED");
    // The list is also applied on a lowercase or oddly spaced name: only the exact name is allowed.
    assert_eq!(call("Get_Issue").unwrap_err().code, "TOOL_NOT_ALLOWED");
}

#[test]
fn a_tool_off_the_list_is_refused_for_a_server_in_the_computer_too() {
    let r = Rig::new("lists-guest");
    let tokens = std::collections::BTreeMap::from([("GITHUB_PERSONAL_ACCESS_TOKEN".to_string(), TOKEN.to_string())]);
    mcp::connect(&r.root, "tl4", "github", tokens, r.port.sandbox.as_ref()).unwrap();
    r.full("tl4");
    lists(&r.root, "tl4", "github", &[], &["create_issue"]).unwrap();
    let blocked = r.call(&r.ctx("tl4"), "mcp_call_tool", json!({ "server": "github", "tool": "create_issue", "arguments": {} })).unwrap_err();
    assert_eq!(blocked.code, "TOOL_NOT_ALLOWED");
    assert_eq!(r.open_cards("tl4"), 0);
}

#[test]
fn the_tools_list_the_bot_reads_leaves_out_what_it_may_not_call() {
    let r = Rig::new("lists-list");
    let server = mcp_server(false);
    mcp::add_remote(&r.root, "tl5", "tracker", &format!("{}/mcp", server.base()), true, None).unwrap();
    lists(&r.root, "tl5", "tracker", &[], &["create_issue"]).unwrap();
    let list = r.call(&r.ctx("tl5"), "mcp_list_tools", json!({})).unwrap();
    let tools: Vec<&str> = list["servers"].as_array().unwrap().iter().find(|s| s["name"] == "tracker").unwrap()["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(tools, vec!["get_issue"]);
    // The same filter on a list that came from the computer, for a server this bot has; one it does not have is left alone.
    let settings = bot_settings::load(&r.root, "tl5").unwrap();
    let mut reply = json!({ "servers": [
        { "name": "tracker", "tools": [{ "name": "get_issue" }, { "name": "create_issue" }, "other"] },
        { "name": "elsewhere", "tools": [{ "name": "create_issue" }] },
    ] });
    mcp::filter_listed_tools(&settings, &mut reply);
    assert_eq!(reply["servers"][0]["tools"], json!([{ "name": "get_issue" }, "other"]));
    assert_eq!(reply["servers"][1]["tools"], json!([{ "name": "create_issue" }]));
}
