//! Sign-in for tool servers by link (`mcp_oauth.rs`), against a fake provider on 127.0.0.1 only. No real service, no real
//! account, no real browser: the "browser" is a closure that reads the sign-in address the app would open and then calls the
//! app's one-time listener, the way a browser's redirect would.
//! What is shown: the PKCE and state rules, dynamic registration or "needs a client id", discovery addresses checked like the
//! server's own link, tokens kept only in the credential store, refresh before the end and after a 401, and that no token form
//! reaches a view, a note, an error or an answer.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::bot_settings;
use super::fake_http::{serve, Reply, Req, Server};
use super::mcp;
use super::mcp_oauth;
use super::remote_mcp::{self, RemoteError, Resp, Wire};
use super::remote_mcp_tests::{reply_for, temp, Rig};
use super::sandbox::{FakeSandbox, Sandbox};

#[derive(Default)]
struct Provider {
    base: String,
    registration: bool,
    protected_resource: bool,
    valid_access: String,
    valid_refresh: String,
    challenge: Option<String>,
    registered: Vec<Value>,
    token_forms: Vec<Vec<(String, String)>>,
    mcp_auth: Vec<String>,
    refuse_all: bool,
}

type Shared = Arc<Mutex<Provider>>;

fn decode_form(body: &str) -> Vec<(String, String)> {
    body.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (pct(k), pct(v))
        })
        .collect()
}

fn pct(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            out.push(u8::from_str_radix(&text[i + 1..i + 3], 16).unwrap_or(b'?'));
            i += 3;
        } else if b[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn get<'a>(form: &'a [(String, String)], key: &str) -> &'a str {
    form.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).unwrap_or("")
}

fn provider(registration: bool, protected_resource: bool) -> (Server, Shared) {
    let state: Shared = Arc::new(Mutex::new(Provider { registration, protected_resource, valid_access: "at-1".into(), valid_refresh: "rt-1".into(), ..Default::default() }));
    let shared = state.clone();
    let server = serve(move |req: &Req| {
        let mut p = shared.lock();
        let base = p.base.clone();
        let path = req.path.split('?').next().unwrap_or("").to_string();
        match (req.method.as_str(), path.as_str()) {
            ("GET", "/.well-known/oauth-protected-resource") | ("GET", "/.well-known/oauth-protected-resource/mcp") => {
                if p.protected_resource {
                    Reply::json(200, &json!({ "resource": format!("{base}/mcp"), "authorization_servers": [base] }).to_string())
                } else {
                    Reply::json(404, "{}")
                }
            }
            ("GET", "/.well-known/oauth-authorization-server") => {
                let mut meta = json!({
                    "issuer": base,
                    "authorization_endpoint": format!("{base}/authorize"),
                    "token_endpoint": format!("{base}/token"),
                    "code_challenge_methods_supported": ["S256"],
                });
                if p.registration {
                    meta["registration_endpoint"] = json!(format!("{base}/register"));
                }
                Reply::json(200, &meta.to_string())
            }
            ("POST", "/register") => {
                let v: Value = serde_json::from_str(&req.body).unwrap_or(json!({}));
                p.registered.push(v);
                Reply::json(201, &json!({ "client_id": "client-abc" }).to_string())
            }
            ("POST", "/token") => {
                let form = decode_form(&req.body);
                p.token_forms.push(form.clone());
                match get(&form, "grant_type") {
                    "authorization_code" => {
                        let verified = p.challenge.as_deref().is_some_and(|c| super::mcp_oauth::challenge_of(get(&form, "code_verifier")) == c);
                        if get(&form, "code") == "code-1" && verified {
                            Reply::json(200, &json!({ "access_token": "at-1", "refresh_token": "rt-1", "expires_in": 3600, "token_type": "Bearer" }).to_string())
                        } else {
                            Reply::json(400, &json!({ "error": "invalid_grant" }).to_string())
                        }
                    }
                    "refresh_token" if get(&form, "refresh_token") == p.valid_refresh => {
                        p.valid_access = "at-2".into();
                        p.valid_refresh = "rt-2".into();
                        Reply::json(200, &json!({ "access_token": "at-2", "refresh_token": "rt-2", "expires_in": 3600 }).to_string())
                    }
                    _ => Reply::json(400, &json!({ "error": "invalid_grant" }).to_string()),
                }
            }
            ("POST", "/mcp") => {
                let auth = req.header("authorization").unwrap_or("").to_string();
                p.mcp_auth.push(auth.clone());
                if !p.refuse_all && auth == format!("Bearer {}", p.valid_access) {
                    reply_for(req, false)
                } else {
                    Reply::json(401, "{}")
                }
            }
            _ => Reply::json(404, "{}"),
        }
    });
    state.lock().base = server.base();
    (server, state)
}

/// What the person's browser would do after the sign-in page: come back to the app's listener.
fn browser(state: Shared, then: Vec<(&'static str, &'static str)>) -> impl Fn(&str) -> Result<(), String> {
    move |auth_url: &str| {
        let query = auth_url.split_once('?').map_or("", |(_, q)| q);
        let q = decode_form(query);
        assert_eq!(get(&q, "response_type"), "code");
        assert_eq!(get(&q, "code_challenge_method"), "S256");
        assert!(get(&q, "redirect_uri").starts_with("http://127.0.0.1:") && get(&q, "redirect_uri").ends_with("/callback"));
        assert!(get(&q, "resource").ends_with("/mcp"));
        assert!(!get(&q, "state").is_empty() && !get(&q, "code_challenge").is_empty());
        state.lock().challenge = Some(get(&q, "code_challenge").to_string());
        let redirect = get(&q, "redirect_uri").to_string();
        let real_state = get(&q, "state").to_string();
        let steps: Vec<(String, String)> = then.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        std::thread::spawn(move || {
            for (code, st) in steps {
                let st = if st.is_empty() { real_state.clone() } else { st };
                let _ = ureq::get(&format!("{redirect}?code={code}&state={st}")).call();
            }
        });
        Ok(())
    }
}

fn good(state: &Shared) -> impl Fn(&str) -> Result<(), String> {
    browser(state.clone(), vec![("code-1", "")])
}

fn live() -> remote_mcp::LiveWire {
    remote_mcp::LiveWire::new(true)
}

fn add(bot: &str, root: &std::path::Path, base: &str) {
    mcp::add_remote(root, bot, "tracker", &format!("{base}/mcp"), true, None).unwrap();
}

fn sign_in(bot: &str, root: &std::path::Path, state: &Shared, client_id: Option<&str>) -> super::store::StoreResult<()> {
    mcp::oauth_sign_in(root, bot, "tracker", client_id, &live(), Duration::from_secs(10), &good(state))
}

// ---- the pieces -----------------------------------------------------------------------------------

#[test]
fn the_challenge_is_the_s256_of_the_verifier_as_the_standard_shows() {
    assert_eq!(mcp_oauth::challenge_of("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
}

// ---- the whole sign-in ------------------------------------------------------------------------------

#[test]
fn a_server_that_registers_apps_is_signed_in_with_pkce_and_the_tokens_stay_in_the_credential_store() {
    let bot = "oauth2";
    let (server, state) = provider(true, true);
    let root = temp("oauth-full");
    add(bot, &root, &server.base());
    sign_in(bot, &root, &state, None).unwrap();

    // The app registered itself as a public client with its one-time return address.
    let reg = state.lock().registered.clone();
    assert_eq!(reg.len(), 1);
    assert_eq!(reg[0]["token_endpoint_auth_method"], "none");
    assert!(reg[0]["redirect_uris"][0].as_str().unwrap().starts_with("http://127.0.0.1:"));
    // The code was exchanged with the verifier that matches the challenge (the fake refuses otherwise).
    let forms = state.lock().token_forms.clone();
    assert_eq!(get(&forms[0], "grant_type"), "authorization_code");
    assert_eq!(get(&forms[0], "client_id"), "client-abc");
    assert!(get(&forms[0], "code_verifier").len() >= 43);

    let saved = bot_settings::load(&root, bot).unwrap().mcp_servers.remove(0);
    assert!(saved.oauth);
    let kept = mcp_oauth::load(bot, "tracker").unwrap();
    assert_eq!((kept.access_token.as_str(), kept.refresh_token.as_deref(), kept.client_id.as_str()), ("at-1", Some("rt-1"), "client-abc"));
    // Not in the settings file, not in the page's view.
    let file = std::fs::read_to_string(bot_settings::settings_dir(&root).join(format!("{bot}.json"))).unwrap();
    assert!(!file.contains("at-1") && !file.contains("rt-1"));
    let view = serde_json::to_string(&mcp::overview(&root, bot, &FakeSandbox::new()).unwrap()).unwrap();
    assert!(!view.contains("at-1") && !view.contains("rt-1") && view.contains("\"signedIn\":true"));

    // Calls carry the access token.
    let wire = live();
    let answer = mcp::call_remote(bot, &saved, &wire, "get_issue", &json!({})).unwrap();
    assert_eq!(answer["content"][0]["text"], "called get_issue with Bearer at-1");
}

#[test]
fn a_server_with_no_description_pages_gets_the_standard_addresses() {
    let bot = "oauth3";
    let (server, state) = provider(true, false);
    let root = temp("oauth-std");
    add(bot, &root, &server.base());
    sign_in(bot, &root, &state, None).unwrap();
    assert!(mcp_oauth::load(bot, "tracker").is_some());
}

#[test]
fn a_server_that_cannot_register_needs_a_client_id_and_the_browser_is_not_opened_without_one() {
    let bot = "oauth4";
    let (server, state) = provider(false, true);
    let root = temp("oauth-noid");
    add(bot, &root, &server.base());
    let opened = Arc::new(Mutex::new(false));
    let flag = opened.clone();
    let open = move |_: &str| -> Result<(), String> {
        *flag.lock() = true;
        Ok(())
    };
    let err = mcp::oauth_sign_in(&root, bot, "tracker", None, &live(), Duration::from_secs(1), &open).unwrap_err();
    assert_eq!(err.code, "MCP_OAUTH_NEEDS_CLIENT_ID");
    assert!(err.message.contains("client id"));
    assert!(!*opened.lock());
    assert!(mcp_oauth::load(bot, "tracker").is_none());
    assert!(!bot_settings::load(&root, bot).unwrap().mcp_servers[0].oauth);
    // With the person's own client id there is no registration and that id is used.
    sign_in(bot, &root, &state, Some("my-client-id")).unwrap();
    assert!(state.lock().registered.is_empty());
    assert_eq!(get(&state.lock().token_forms[0], "client_id"), "my-client-id");
    assert_eq!(mcp_oauth::load(bot, "tracker").unwrap().client_id, "my-client-id");
    // A client id with a space or a control character is refused.
    assert_eq!(mcp::oauth_sign_in(&root, bot, "tracker", Some("bad id"), &live(), Duration::from_secs(1), &open).unwrap_err().code, "MCP_OAUTH_NEEDS_CLIENT_ID");
}

#[test]
fn an_answer_with_the_wrong_state_is_ignored_and_the_right_one_still_works() {
    let bot = "oauth5";
    let (server, state) = provider(true, true);
    let root = temp("oauth-state");
    add(bot, &root, &server.base());
    let open = browser(state.clone(), vec![("code-evil", "forged-state"), ("code-1", "")]);
    mcp::oauth_sign_in(&root, bot, "tracker", None, &live(), Duration::from_secs(10), &open).unwrap();
    let forms = state.lock().token_forms.clone();
    assert_eq!(forms.len(), 1, "the forged answer never reached the token address");
    assert_eq!(get(&forms[0], "code"), "code-1");
}

#[test]
fn a_sign_in_that_is_not_finished_ends_and_stores_nothing() {
    let bot = "oauth6";
    let (server, _state) = provider(true, true);
    let root = temp("oauth-timeout");
    add(bot, &root, &server.base());
    let open = |_: &str| -> Result<(), String> { Ok(()) };
    let err = mcp::oauth_sign_in(&root, bot, "tracker", None, &live(), Duration::from_millis(300), &open).unwrap_err();
    assert_eq!(err.code, "MCP_OAUTH_TIMEOUT");
    assert!(mcp_oauth::load(bot, "tracker").is_none());
}

#[test]
fn a_sign_in_the_person_refuses_ends_with_a_plain_line() {
    let bot = "oauth7";
    let (server, state) = provider(true, true);
    let root = temp("oauth-denied");
    add(bot, &root, &server.base());
    let inner = browser(state.clone(), vec![]);
    let open = move |url: &str| -> Result<(), String> {
        inner(url)?;
        let q = decode_form(url.split_once('?').unwrap().1);
        let back = format!("{}?error=access_denied&state={}", get(&q, "redirect_uri"), get(&q, "state"));
        std::thread::spawn(move || {
            let _ = ureq::get(&back).call();
        });
        Ok(())
    };
    let err = mcp::oauth_sign_in(&root, bot, "tracker", None, &live(), Duration::from_secs(10), &open).unwrap_err();
    assert_eq!(err.code, "MCP_OAUTH_DENIED");
    assert!(mcp_oauth::load(bot, "tracker").is_none());
}

#[test]
fn a_server_that_will_not_do_pkce_s256_is_not_signed_in_to() {
    let weak = serve(|req| {
        if req.path.contains("oauth-authorization-server") {
            Reply::json(200, &json!({ "authorization_endpoint": "http://127.0.0.1:1/a", "token_endpoint": "http://127.0.0.1:1/t", "code_challenge_methods_supported": ["plain"] }).to_string())
        } else {
            Reply::json(404, "{}")
        }
    });
    let err = mcp_oauth::discover(&live(), &format!("{}/mcp", weak.base()), true).unwrap_err();
    assert_eq!(err.code, "MCP_OAUTH_UNSUPPORTED");
}

// ---- refresh --------------------------------------------------------------------------------------

fn expire(bot: &str, stored_for: &str, ms: i64) {
    let mut s = mcp_oauth::load(bot, stored_for).unwrap();
    s.expires_at_ms = ms;
    crate::secrets::backend().set(&mcp_oauth::secret_name(bot, stored_for), &serde_json::to_string(&s).unwrap()).unwrap();
}

#[test]
fn a_token_about_to_end_is_renewed_before_the_call_and_the_new_one_is_kept() {
    let bot = "oauth9";
    let (server, state) = provider(true, true);
    let root = temp("oauth-refresh");
    add(bot, &root, &server.base());
    sign_in(bot, &root, &state, None).unwrap();
    expire(bot, "tracker", 1); // long ago
    let saved = bot_settings::load(&root, bot).unwrap().mcp_servers.remove(0);
    let answer = mcp::call_remote(bot, &saved, &live(), "get_issue", &json!({})).unwrap();
    assert_eq!(answer["content"][0]["text"], "called get_issue with Bearer at-2");
    let forms = state.lock().token_forms.clone();
    assert_eq!(get(&forms[1], "grant_type"), "refresh_token");
    assert_eq!(get(&forms[1], "refresh_token"), "rt-1");
    assert_eq!(get(&forms[1], "client_id"), "client-abc");
    let kept = mcp_oauth::load(bot, "tracker").unwrap();
    assert_eq!((kept.access_token.as_str(), kept.refresh_token.as_deref()), ("at-2", Some("rt-2")));
    assert!(kept.expires_at_ms > 1_000_000_000_000, "a new end time was saved");
}

#[test]
fn a_token_that_is_still_good_is_not_renewed() {
    let bot = "oauth10";
    let (server, state) = provider(true, true);
    let root = temp("oauth-keep");
    add(bot, &root, &server.base());
    sign_in(bot, &root, &state, None).unwrap();
    let saved = bot_settings::load(&root, bot).unwrap().mcp_servers.remove(0);
    mcp::call_remote(bot, &saved, &live(), "get_issue", &json!({})).unwrap();
    mcp::call_remote(bot, &saved, &live(), "get_issue", &json!({})).unwrap();
    assert_eq!(state.lock().token_forms.len(), 1, "only the sign-in used the token address");
}

#[test]
fn a_401_renews_the_token_once_and_retries() {
    let bot = "oauth11";
    let (server, state) = provider(true, true);
    let root = temp("oauth-401");
    add(bot, &root, &server.base());
    sign_in(bot, &root, &state, None).unwrap();
    // The server stopped accepting at-1 (revoked, rotated) though it has not "expired" here.
    state.lock().valid_access = "revoked".into();
    let saved = bot_settings::load(&root, bot).unwrap().mcp_servers.remove(0);
    let answer = mcp::call_remote(bot, &saved, &live(), "get_issue", &json!({})).unwrap();
    assert_eq!(answer["content"][0]["text"], "called get_issue with Bearer at-2");
    assert_eq!(state.lock().token_forms.iter().filter(|f| get(f, "grant_type") == "refresh_token").count(), 1, "renewed once");
}

#[test]
fn a_401_that_a_new_token_does_not_fix_ends_with_a_sign_in_line_and_no_loop() {
    let bot = "oauth-loop";
    let (server, state) = provider(true, true);
    let root = temp("oauth-401b");
    add(bot, &root, &server.base());
    sign_in(bot, &root, &state, None).unwrap();
    // The server refuses every token: renewing once does not help, and the call stops with a plain line.
    state.lock().refuse_all = true;
    let saved = bot_settings::load(&root, bot).unwrap().mcp_servers.remove(0);
    let err = mcp::call_remote(bot, &saved, &live(), "get_issue", &json!({})).unwrap_err();
    assert_eq!(err.code, "MCP_NEEDS_SIGN_IN");
    assert_eq!(state.lock().token_forms.iter().filter(|f| get(f, "grant_type") == "refresh_token").count(), 1, "renewed once, not in a loop");
    let note = mcp::remote_message(bot, &saved, &err);
    assert!(!note.contains("at-") && !note.contains("rt-"));
}

#[test]
fn a_refresh_the_server_refuses_asks_for_a_new_sign_in_without_naming_a_token() {
    let bot = "oauth13";
    let (server, state) = provider(true, true);
    let root = temp("oauth-dead");
    add(bot, &root, &server.base());
    sign_in(bot, &root, &state, None).unwrap();
    expire(bot, "tracker", 1);
    state.lock().valid_refresh = "something-else".into();
    let saved = bot_settings::load(&root, bot).unwrap().mcp_servers.remove(0);
    let err = mcp::call_remote(bot, &saved, &live(), "get_issue", &json!({})).unwrap_err();
    assert_eq!(err.code, "MCP_NEEDS_SIGN_IN");
    let note = mcp::remote_message(bot, &saved, &err);
    assert!(!note.contains("at-1") && !note.contains("rt-1"));
}

// ---- discovery addresses are checked ------------------------------------------------------------------

#[derive(Default)]
struct Pages {
    pages: Mutex<VecDeque<(String, Resp)>>,
    asked: Mutex<Vec<String>>,
}

impl Wire for Pages {
    fn post(&self, url: &str, _h: &[(String, String)], _b: &str, _w: Option<i64>) -> Result<Resp, RemoteError> {
        self.asked.lock().push(format!("POST {url}"));
        Ok(Resp { status: 404, ..Default::default() })
    }
    fn get(&self, url: &str, _h: &[(String, String)]) -> Result<Resp, RemoteError> {
        self.asked.lock().push(format!("GET {url}"));
        let found = self.pages.lock().iter().find(|(u, _)| url.contains(u.as_str())).map(|(_, r)| r.clone());
        Ok(found.unwrap_or(Resp { status: 404, ..Default::default() }))
    }
}

fn page(url: &str, body: Value) -> (String, Resp) {
    (url.to_string(), Resp { status: 200, body: body.to_string(), ..Default::default() })
}

#[test]
fn an_address_found_on_a_discovery_page_is_refused_when_it_points_inside_or_is_not_https() {
    for (field, value) in [
        ("token_endpoint", "http://169.254.169.254/token"),
        ("token_endpoint", "https://10.0.0.1/token"),
        ("authorization_endpoint", "https://127.0.0.1/authorize"),
        ("authorization_endpoint", "https://localhost/authorize"),
        ("registration_endpoint", "https://192.168.1.1/register"),
        ("registration_endpoint", "ftp://mcp.example.com/register"),
        ("token_endpoint", "file:///etc/passwd"),
    ] {
        let mut meta = json!({ "authorization_endpoint": "https://auth.example.com/authorize", "token_endpoint": "https://auth.example.com/token", "registration_endpoint": "https://auth.example.com/register" });
        meta[field] = json!(value);
        let wire = Pages::default();
        wire.pages.lock().push_back(page("/.well-known/oauth-authorization-server", meta));
        let err = mcp_oauth::discover(&wire, "https://mcp.example.com/mcp", false).unwrap_err();
        assert!(err.code == "MCP_URL_PRIVATE" || err.code == "MCP_URL_SCHEME", "{field} {value}: {}", err.code);
    }
}

#[test]
fn a_resource_that_names_an_inside_sign_in_server_is_refused_before_it_is_asked() {
    for issuer in ["https://10.0.0.5", "http://127.0.0.1:8080", "https://169.254.169.254", "https://intranet.corp"] {
        let wire = Pages::default();
        wire.pages.lock().push_back(page("/.well-known/oauth-protected-resource", json!({ "authorization_servers": [issuer] })));
        let err = mcp_oauth::discover(&wire, "https://mcp.example.com/mcp", false).unwrap_err();
        assert!(err.code.starts_with("MCP_URL"), "{issuer}: {}", err.code);
        assert!(wire.asked.lock().iter().all(|a| !a.contains(issuer.trim_start_matches("https://").trim_start_matches("http://"))), "{issuer} was never asked");
    }
}

#[test]
fn a_public_sign_in_server_found_on_a_page_is_accepted() {
    let wire = Pages::default();
    wire.pages.lock().push_back(page("/.well-known/oauth-protected-resource", json!({ "authorization_servers": ["https://auth.example.com"] })));
    wire.pages.lock().push_back(page("auth.example.com/.well-known/oauth-authorization-server", json!({ "authorization_endpoint": "https://auth.example.com/authorize", "token_endpoint": "https://auth.example.com/token", "registration_endpoint": "https://auth.example.com/register" })));
    let found = mcp_oauth::discover(&wire, "https://mcp.example.com/mcp", false).unwrap();
    assert_eq!(found.authorization, "https://auth.example.com/authorize");
    assert_eq!(found.registration.as_deref(), Some("https://auth.example.com/register"));
}

// ---- signing out, removing, scrubbing ---------------------------------------------------------------------

#[test]
fn signing_out_and_removing_forget_the_tokens() {
    let bot = "oauth17";
    let (server, state) = provider(true, true);
    let root = temp("oauth-out");
    add(bot, &root, &server.base());
    sign_in(bot, &root, &state, None).unwrap();
    mcp::oauth_sign_out(&root, bot, "tracker").unwrap();
    assert!(mcp_oauth::load(bot, "tracker").is_none());
    assert!(!bot_settings::load(&root, bot).unwrap().mcp_servers[0].oauth);
    sign_in(bot, &root, &state, None).unwrap();
    let sandbox = FakeSandbox::new();
    sandbox.start().unwrap();
    mcp::remove_server(&root, bot, "tracker", &sandbox).unwrap();
    assert!(mcp_oauth::load(bot, "tracker").is_none());
    assert_eq!(mcp::oauth_sign_out(&root, bot, "tracker").unwrap_err().code, "MCP_NOT_FOUND");
}

#[test]
fn a_stdio_server_or_a_missing_one_cannot_be_signed_in_to() {
    let bot = "oauth18";
    let root = temp("oauth-bad");
    let sandbox = FakeSandbox::new();
    mcp::add_server(&root, bot, mcp::NewServer { name: "plain".into(), command: "npx".into(), args: vec![], env: Default::default() }, &sandbox).unwrap();
    let open = |_: &str| -> Result<(), String> { Ok(()) };
    assert_eq!(mcp::oauth_sign_in(&root, bot, "plain", None, &live(), Duration::from_millis(100), &open).unwrap_err().code, "MCP_NOT_FOUND");
    assert_eq!(mcp::oauth_sign_in(&root, bot, "nobody", None, &live(), Duration::from_millis(100), &open).unwrap_err().code, "MCP_NOT_FOUND");
}

#[test]
fn through_the_gate_a_signed_in_answer_never_carries_the_token_even_when_the_server_echoes_it() {
    let bot = "oauth19";
    let (server, state) = provider(true, true);
    let r = Rig::new("oauth-gate");
    add(bot, &r.root, &server.base());
    sign_in(bot, &r.root, &state, None).unwrap();
    r.full(bot);
    let answer = r.call(&r.ctx(bot), "mcp_call_tool", json!({ "server": "tracker", "tool": "get_issue", "arguments": {} })).unwrap();
    // The fake server echoes the Authorization header into its answer.
    assert!(!answer.to_string().contains("at-1"), "{answer}");
    assert!(answer["text"].as_str().unwrap().contains("[hidden]"));
    // The step list and the tools list are free of it too.
    let list = r.call(&r.ctx(bot), "mcp_list_tools", json!({})).unwrap();
    assert!(!list.to_string().contains("at-1"));
}

#[test]
fn the_sign_in_never_leaves_the_machine_for_an_address_the_server_chose_after_it_is_signed_in() {
    let bot = "oauth20";
    // The token address is stored; a refresh posts only there, and a redirect from it to another origin does not take the form's
    // secrets along (the refresh token is in the body, so a redirect to another origin is refused outright for a private place).
    let (server, state) = provider(true, true);
    let root = temp("oauth-redir");
    add(bot, &root, &server.base());
    sign_in(bot, &root, &state, None).unwrap();
    let mut stored = mcp_oauth::load(bot, "tracker").unwrap();
    stored.token_endpoint = "https://169.254.169.254/token".into();
    stored.expires_at_ms = 1;
    crate::secrets::backend().set(&mcp_oauth::secret_name(bot, "tracker"), &serde_json::to_string(&stored).unwrap()).unwrap();
    let saved = bot_settings::load(&root, bot).unwrap().mcp_servers.remove(0);
    let wire = remote_mcp::LiveWire::new(false);
    let err = mcp_oauth::access_token(&wire, bot, &saved, 2, false).unwrap_err();
    assert!(err.code == "MCP_URL_PRIVATE" || err.code == "MCP_URL_SCHEME", "{}", err.code);
}
