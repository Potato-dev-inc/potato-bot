//! The phone's sandbox screen: `POST /api/bots/{id}/screen` (what the link says and what refuses it), the viewer and the
//! websocket under `/screen/<sealed>/` on real sockets (a fake guest websockify, the real viewer folder, a clock the test
//! moves), the bot list's `computer` flag, and the revocation of a pairing while a relay is open. No app handle, no
//! sandbox, no WSL.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::api::{need, need_for};
use super::http::{start_for_test, Server};
use super::testkit::{kit_with_hub, req, Kit};
use crate::mobile_pairing::{Cap, Caps};
use crate::potato::bot_settings::ToolSwitches;
use crate::potato::sandbox::screen::{ScreenHub, ScreenSource, LINK_TTL_MS};
use crate::potato::sandbox::secrets::ScreenSecrets;
use crate::potato::sandbox::{SbxResult, ScreenTarget};
use crate::potato::service_card_tests::rig;
use crate::potato::service_tests::Rig;

const T0: i64 = 9_000_000;

fn viewer_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("potato-sandbox").join("viewer")
}

// ---- the fake guest ------------------------------------------------------------------------------

struct Guest {
    port: u16,
    heads: Arc<Mutex<Vec<String>>>,
}

fn spawn_guest() -> Guest {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind guest");
    let port = listener.local_addr().unwrap().port();
    let heads = Arc::new(Mutex::new(Vec::new()));
    let seen = heads.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let seen = seen.clone();
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(byte[0]);
                }
                seen.lock().push(String::from_utf8_lossy(&head).into_owned());
                let _ = stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: test\r\n\r\n");
                let mut buf = [0u8; 1024];
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            if stream.write_all(&buf[..n]).is_err() {
                                return;
                            }
                        }
                    }
                }
            });
        }
    });
    Guest { port, heads }
}

struct Source {
    port: u16,
    generation: AtomicU64,
    secrets: Arc<ScreenSecrets>,
    running: AtomicBool,
}

impl ScreenSource for Source {
    fn name(&self) -> String {
        "pm-sbx-main".to_string()
    }

    fn target(&self) -> SbxResult<ScreenTarget> {
        let passwords = self.running.load(Ordering::SeqCst).then(|| self.secrets.clone());
        Ok(ScreenTarget { view_port: self.port, generation: self.generation.load(Ordering::SeqCst), passwords })
    }
}

// ---- the rig --------------------------------------------------------------------------------------

struct Screen {
    _rig: Rig,
    k: Kit,
    source: Arc<Source>,
    clock: Arc<AtomicI64>,
    guest: Guest,
    server: Server,
}

impl Drop for Screen {
    fn drop(&mut self) {
        self.server.stop();
    }
}

fn screen(name: &str) -> Screen {
    let (rig, _fake) = rig(name);
    let clock = Arc::new(AtomicI64::new(T0));
    let tick = clock.clone();
    let hub = Arc::new(ScreenHub::with_clock(Box::new(move || tick.load(Ordering::SeqCst))));
    let guest = spawn_guest();
    let source = Arc::new(Source { port: guest.port, generation: AtomicU64::new(1), secrets: Arc::new(ScreenSecrets::generate()), running: AtomicBool::new(true) });
    hub.set_source(Some(source.clone()));
    hub.set_enabled(true);
    let k = kit_with_hub(name, Some(rig.svc.clone()), hub);
    *k.env.viewer.lock() = Some(viewer_dir());
    let server = start_for_test(k.ctx.clone());
    Screen { _rig: rig, k, source, clock, guest, server }
}

impl Screen {
    fn port(&self) -> u16 {
        self.server.addr.port()
    }

    fn hub(&self) -> &Arc<ScreenHub> {
        &self.k.env.hub
    }

    /// A paired phone with these abilities: its id and token.
    fn phone(&self, caps: Caps) -> (String, String) {
        self.k.pair_with(caps)
    }

    fn post(&self, token: &str, bot: &str, mode: &str) -> (u16, Value) {
        let r = self.k.authed(token, "POST", &format!("/api/bots/{bot}/screen"), &json!({ "mode": mode }).to_string());
        (r.status, r.body_json())
    }

    /// The `src` of a good answer, split at the `#`.
    fn src(&self, token: &str, mode: &str) -> (String, String) {
        let (status, v) = self.post(token, "nimbus", mode);
        assert_eq!(status, 200, "{v}");
        let src = v["src"].as_str().unwrap().to_string();
        let (path, fragment) = src.split_once('#').expect("the password is in a fragment");
        (path.to_string(), fragment.to_string())
    }

    fn get(&self, path: &str, extra: &[&str]) -> Reply {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port())).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut text = format!("GET {path} HTTP/1.1\r\n");
        if !extra.iter().any(|h| h.to_ascii_lowercase().starts_with("host:")) {
            text.push_str(&format!("Host: 127.0.0.1:{}\r\n", self.port()));
        }
        for h in extra {
            text.push_str(h);
            text.push_str("\r\n");
        }
        text.push_str("Connection: close\r\n\r\n");
        stream.write_all(text.as_bytes()).unwrap();
        let mut raw = Vec::new();
        let _ = stream.read_to_end(&mut raw);
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4).unwrap_or(raw.len());
        let head = String::from_utf8_lossy(&raw[..split]).into_owned();
        let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        Reply { status, head, body: String::from_utf8_lossy(&raw[split..]).into_owned() }
    }

    /// A websocket client: sends the upgrade, returns the stream and the handshake answer.
    fn socket(&self, sealed_path: &str, extra: &[&str]) -> (TcpStream, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port())).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut text = format!("GET {sealed_path}/websockify HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n", self.port());
        text.push_str("Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: binary\r\n");
        for h in extra {
            text.push_str(h);
            text.push_str("\r\n");
        }
        text.push_str("\r\n");
        stream.write_all(text.as_bytes()).unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).unwrap_or(0) == 0 {
                break;
            }
            head.push(byte[0]);
        }
        (stream, String::from_utf8_lossy(&head).into_owned())
    }
}

struct Reply {
    status: u16,
    head: String,
    body: String,
}

fn echo(stream: &mut TcpStream, text: &str) -> bool {
    if stream.write_all(text.as_bytes()).is_err() {
        return false;
    }
    let mut got = vec![0u8; text.len()];
    stream.read_exact(&mut got).is_ok() && got == text.as_bytes()
}

/// Waits for the relay to close: true when the next read ends (EOF or reset) within `limit`.
fn closes_within(stream: &mut TcpStream, limit: Duration) -> bool {
    let start = Instant::now();
    stream.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let mut buf = [0u8; 64];
    while start.elapsed() < limit {
        match stream.read(&mut buf) {
            Ok(0) => return true,
            Ok(_) => {}
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => return true,
        }
    }
    false
}

fn watch_only() -> Caps {
    Caps::none().with(Cap::Watch, true)
}

fn control_caps() -> Caps {
    Caps::all()
}

// ---- the abilities -------------------------------------------------------------------------------

#[test]
fn the_screen_routes_need_the_right_abilities() {
    assert_eq!(need("POST", &["bots", "nimbus", "screen"]), Some(Cap::Watch));
    assert_eq!(need("POST", &["bots", "nimbus", "screen", "release"]), Some(Cap::Control));
    assert_eq!(need("POST", &["bots", "nimbus", "screen", "keep"]), Some(Cap::Control));
    // anything else under a bot still fails closed
    assert_eq!(need("POST", &["bots", "nimbus", "screen", "other"]), Some(Cap::Control));
    assert_eq!(need("GET", &["bots", "nimbus", "screen"]), Some(Cap::Control));
    assert_eq!(need("DELETE", &["bots", "nimbus", "screen"]), Some(Cap::Control));
    // a link for control is held to control; for view, to watch; a body we cannot read is held to control
    let seg = ["bots", "nimbus", "screen"];
    assert_eq!(need_for(&req("POST", "/api/bots/nimbus/screen", &[], r#"{"mode":"view"}"#), &seg), Some(Cap::Watch));
    assert_eq!(need_for(&req("POST", "/api/bots/nimbus/screen", &[], r#"{"mode":"control"}"#), &seg), Some(Cap::Control));
    assert_eq!(need_for(&req("POST", "/api/bots/nimbus/screen", &[], "{}"), &seg), Some(Cap::Control));
    assert_eq!(need_for(&req("POST", "/api/bots/nimbus/screen", &[], "not json"), &seg), Some(Cap::Control));
}

#[test]
fn view_works_with_watch_and_the_password_is_only_in_the_fragment() {
    let s = screen("scr-view");
    let (_, token) = s.phone(watch_only());
    let (status, v) = s.post(&token, "nimbus", "view");
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["mode"], "view");
    assert_eq!(v["expiresAtMs"], T0 + LINK_TTL_MS);
    let src = v["src"].as_str().unwrap();
    assert!(src.starts_with("/screen/"), "same origin, relative: {src}");
    let (path, fragment) = src.split_once('#').expect("a fragment");
    assert!(path.contains("/embed.html?path=websockify") && !path.contains("control=1"), "{path}");
    let password = s.source.secrets.view().expose().to_string();
    assert_eq!(fragment, format!("vnc={password}"));
    assert!(!path.contains(&password), "the password is never before the #");
    assert!(!v.to_string().replace(src, "").contains(&password), "and nowhere else in the answer");
    assert!(!password.is_empty() && password != s.source.secrets.control().expose());
}

#[test]
fn control_is_refused_without_the_control_ability_before_anything_is_taken() {
    let s = screen("scr-nocontrol");
    s.hub().allow_phone_control("anyone");
    // watch + type, but not control: the gate refuses (a 403 in the API's own words)
    let (id, token) = s.phone(Caps::none().with(Cap::Watch, true).with(Cap::Type, true));
    s.hub().allow_phone_control(&id);
    let (status, v) = s.post(&token, "nimbus", "control");
    assert_eq!((status, v["error"].as_str()), (403, Some("not_allowed")), "{v}");
    // watch + control, but not type: the route refuses
    let (id, token) = s.phone(Caps::none().with(Cap::Watch, true).with(Cap::Control, true));
    s.hub().allow_phone_control(&id);
    let (status, v) = s.post(&token, "nimbus", "control");
    assert_eq!((status, v["error"].as_str()), (403, Some("not_allowed")), "{v}");
    assert_eq!(s.hub().state().phase, "free", "nothing was taken");
    // a view-only phone cannot hand back or keep either
    let (_, view) = s.phone(watch_only());
    assert_eq!(s.k.authed(&view, "POST", "/api/bots/nimbus/screen/release", "{}").status, 403);
    assert_eq!(s.k.authed(&view, "POST", "/api/bots/nimbus/screen/keep", "{}").status, 403);
}

#[test]
fn control_is_refused_without_the_pcs_yes_and_takes_nothing() {
    let s = screen("scr-noyes");
    let (_, token) = s.phone(control_caps());
    let (status, v) = s.post(&token, "nimbus", "control");
    assert_eq!((status, v["error"].as_str()), (403, Some("phone_needs_desktop_yes")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("yes on your PC"));
    assert_eq!(s.hub().state().phase, "free", "the bot was not displaced");
    // and the view link is fine for the same phone
    assert_eq!(s.post(&token, "nimbus", "view").0, 200);
}

#[test]
fn control_with_the_yes_takes_the_screen_like_the_pcs_take_over() {
    let s = screen("scr-control");
    let (id, token) = s.phone(control_caps());
    s.hub().allow_phone_control(&id);
    let (path, fragment) = s.src(&token, "control");
    assert!(path.contains("/embed.html?path=websockify&control=1"), "{path}");
    assert_eq!(fragment, format!("vnc={}", s.source.secrets.control().expose()));
    assert_eq!(s.hub().state().phase, "you", "the person holds the screen");
    // a second request while the person holds it does not take it again (that would end the first link)
    let sealed = path.trim_start_matches("/screen/").split('/').next().unwrap().to_string();
    let first = s.get(&format!("/screen/{sealed}/embed.html"), &[]);
    assert_eq!(first.status, 200);
    let _ = s.src(&token, "control");
    assert_eq!(s.get(&format!("/screen/{sealed}/embed.html"), &[]).status, 200, "the first control link still lives");
    // handing back ends control links
    let r = s.k.authed(&token, "POST", "/api/bots/nimbus/screen/release", "{}");
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(s.hub().state().phase, "free");
    assert_eq!(s.get(&format!("/screen/{sealed}/embed.html"), &[]).status, 403, "SCREEN_LEASE_LOST");
}

#[test]
fn the_link_errors_are_plain() {
    let s = screen("scr-errors");
    let (_, token) = s.phone(control_caps());
    let (status, v) = s.post(&token, "nobody", "view");
    assert_eq!((status, v["error"].as_str()), (404, Some("potato_bot_not_found")), "{v}");
    let (status, v) = s.post(&token, "nimbus", "sideways");
    assert_eq!((status, v["error"].as_str()), (400, Some("bad_request")), "{v}");
    // the bot's Computer switch is off: it has no computer
    s._rig.svc.set_tools("nimbus", ToolSwitches { computer: false, ..ToolSwitches::default() }).unwrap();
    let (status, v) = s.post(&token, "nimbus", "view");
    assert_eq!((status, v["error"].as_str()), (409, Some("no_computer")), "{v}");
    s._rig.svc.set_tools("nimbus", ToolSwitches::default()).unwrap();
    // the sandbox is not running
    s.source.running.store(false, Ordering::SeqCst);
    let (status, v) = s.post(&token, "nimbus", "view");
    assert_eq!((status, v["error"].as_str()), (409, Some("screen_not_running")), "{v}");
    s.source.running.store(true, Ordering::SeqCst);
    // the Labs switch is off
    s.hub().set_enabled(false);
    let (status, v) = s.post(&token, "nimbus", "view");
    assert_eq!((status, v["error"].as_str()), (409, Some("screen_off")), "{v}");
    // nothing in any error carries a password or a link
    let all = [s.source.secrets.view().expose().to_string(), s.source.secrets.control().expose().to_string()];
    for password in all {
        assert!(!v.to_string().contains(&password));
    }
}

#[test]
fn the_bot_list_says_which_bots_use_the_computer() {
    let s = screen("scr-list");
    let (_, token) = s.phone(watch_only());
    let list = s.k.authed(&token, "GET", "/api/bots", "").body_json();
    let nimbus = list["bots"].as_array().unwrap().iter().find(|b| b["id"] == "nimbus").unwrap().clone();
    assert_eq!(nimbus["computer"], json!(true));
    let one = s.k.authed(&token, "GET", "/api/bots/nimbus", "").body_json();
    assert_eq!(one["bot"]["computer"], json!(true));
    // the bot's own Computer switch off: no flag
    s._rig.svc.set_tools("nimbus", ToolSwitches { computer: false, ..ToolSwitches::default() }).unwrap();
    let list = s.k.authed(&token, "GET", "/api/bots", "").body_json();
    let nimbus = list["bots"].as_array().unwrap().iter().find(|b| b["id"] == "nimbus").unwrap().clone();
    assert!(nimbus.get("computer").is_none(), "{nimbus}");
    // no sandbox wired in at all: no flag
    let plain = Rig::ready("scr-list-plain");
    let k = kit_with_hub("scr-list-plain", Some(plain.svc.clone()), Arc::new(ScreenHub::new()));
    let (_, token) = k.pair();
    let list = k.authed(&token, "GET", "/api/bots", "").body_json();
    let nimbus = list["bots"].as_array().unwrap().iter().find(|b| b["id"] == "nimbus").unwrap().clone();
    assert!(nimbus.get("computer").is_none(), "{nimbus}");
}

// ---- the viewer ----------------------------------------------------------------------------------

#[test]
fn the_viewer_page_is_served_with_the_phone_pages_own_origin_and_the_hardening_headers() {
    let s = screen("scr-page");
    let (_, token) = s.phone(watch_only());
    let (path, _) = s.src(&token, "view");
    let page = s.get(path.split('?').next().unwrap(), &[]);
    assert_eq!(page.status, 200, "{}", page.head);
    let origin = format!("http://127.0.0.1:{}", s.port());
    assert!(page.body.contains(&format!(r#"name="pm-sbx-parent-origins" content="{origin}""#)), "the parent is this phone page's origin");
    assert!(!page.body.contains(r#"content="*""#) && !page.head.contains("frame-ancestors *"));
    let head = page.head.to_ascii_lowercase();
    assert!(head.contains(&format!("content-security-policy: frame-ancestors 'self' {origin}")), "{}", page.head);
    for line in ["cache-control: no-store, no-transform", "x-content-type-options: nosniff", "referrer-policy: no-referrer"] {
        assert!(head.contains(line), "{line} in {}", page.head);
    }
    // through a tunnel the page is https on the tunnel's name
    s.k.env.hosts.lock().tunnel = Some("quiet-lake.trycloudflare.com".into());
    let tunneled = s.get(path.split('?').next().unwrap(), &["Host: quiet-lake.trycloudflare.com", "X-Forwarded-Proto: https", "Cf-Connecting-Ip: 203.0.113.9"]);
    assert_eq!(tunneled.status, 200, "{}", tunneled.head);
    assert!(tunneled.body.contains(r#"content="https://quiet-lake.trycloudflare.com""#), "{}", tunneled.body.lines().find(|l| l.contains("parent-origins")).unwrap_or(""));
    // a viewer script and nothing outside the viewer
    let sealed_dir = path.rsplit_once('/').unwrap().0.to_string();
    assert_eq!(s.get(&format!("{sealed_dir}/mobile-keyboard.js"), &[]).status, 200);
    assert_eq!(s.get(&format!("{sealed_dir}/../secret.js"), &[]).status, 404);
    assert_eq!(s.get(&format!("{sealed_dir}/index.html"), &[]).status, 404);
    // the answers carry no password
    assert!(!page.body.contains(s.source.secrets.view().expose()));
}

#[test]
fn a_bad_expired_stale_or_foreign_link_gets_no_page() {
    let s = screen("scr-badlinks");
    let (id_a, token_a) = s.phone(watch_only());
    let (_, token_b) = s.phone(watch_only());
    let (path_a, _) = s.src(&token_a, "view");
    let (path_b, _) = s.src(&token_b, "view");
    let dir_a = path_a.rsplit_once('/').unwrap().0.to_string();
    let dir_b = path_b.rsplit_once('/').unwrap().0.to_string();
    // good
    assert_eq!(s.get(&format!("{dir_a}/embed.html"), &[]).status, 200);
    // not a link at all, or a changed one
    assert_eq!(s.get("/screen/garbage/embed.html", &[]).status, 404);
    assert_eq!(s.get("/screen/", &[]).status, 404);
    let tampered = format!("{dir_a}x/embed.html");
    assert_eq!(s.get(&tampered, &[]).status, 404);
    // the app's own link (made for the desktop) is no good on the phone server
    let app = s.hub().link("nimbus", crate::potato::sandbox::screen::Mode::View, &crate::potato::sandbox::screen::Audience::App, "").unwrap().into_inner();
    let app_sealed = app.split("/screen/").nth(1).unwrap().split('/').next().unwrap().to_string();
    assert_eq!(s.get(&format!("/screen/{app_sealed}/embed.html"), &[]).status, 404);
    // each link names ONE phone (its audience), and that phone's pairing is what is checked: signing A out kills A's
    // link and leaves B's alone
    assert_eq!(s.get(&format!("{dir_b}/embed.html"), &[]).status, 200);
    assert!(s.k.pairing.lock().revoke_phone_device(&id_a).unwrap());
    assert_eq!(s.get(&format!("{dir_a}/embed.html"), &[]).status, 403, "A was signed out");
    assert_eq!(s.get(&format!("{dir_b}/embed.html"), &[]).status, 200, "B is untouched");
    // a restart of the sandbox kills the link
    s.source.generation.fetch_add(1, Ordering::SeqCst);
    assert_eq!(s.get(&format!("{dir_b}/embed.html"), &[]).status, 410);
    s.source.generation.fetch_sub(1, Ordering::SeqCst);
    // an hour later it has run out
    s.clock.fetch_add(LINK_TTL_MS + 1, Ordering::SeqCst);
    assert_eq!(s.get(&format!("{dir_b}/embed.html"), &[]).status, 404);
    s.clock.fetch_sub(LINK_TTL_MS + 1, Ordering::SeqCst);
    // with the Labs switch off the route does not exist
    s.hub().set_enabled(false);
    assert_eq!(s.get(&format!("{dir_b}/embed.html"), &[]).status, 404);
}

#[test]
fn a_phone_whose_pairing_ran_out_is_refused_the_page() {
    let s = screen("scr-lostwatch");
    let (id, token) = s.phone(watch_only());
    let (path, _) = s.src(&token, "view");
    let dir = path.rsplit_once('/').unwrap().0.to_string();
    assert_eq!(s.get(&format!("{dir}/embed.html"), &[]).status, 200);
    s.k.pairing.lock().set_device_expiry_for_test(&id, 1);
    assert_eq!(s.get(&format!("{dir}/embed.html"), &[]).status, 403);
}

#[test]
fn the_door_rules_still_apply_to_the_viewer() {
    let s = screen("scr-door");
    let (_, token) = s.phone(watch_only());
    let (path, _) = s.src(&token, "view");
    let page = path.split('?').next().unwrap().to_string();
    // a foreign Host (DNS rebinding) is 421, a foreign Origin is 403
    assert_eq!(s.get(&page, &["Host: evil.example.com"]).status, 421);
    assert_eq!(s.get(&page, &["Origin: https://evil.example.com"]).status, 403);
    // its own origin is fine
    assert_eq!(s.get(&page, &[&format!("Origin: http://127.0.0.1:{}", s.port())]).status, 200);
    // only reading: no POST to a viewer file
    let mut stream = TcpStream::connect(("127.0.0.1", s.port())).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    stream.write_all(format!("POST {page} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", s.port()).as_bytes()).unwrap();
    let mut out = String::new();
    let _ = stream.read_to_string(&mut out);
    assert!(out.starts_with("HTTP/1.1 405"), "{out}");
    // a screen link is not an API token: the API still wants the bearer
    let sealed_dir = page.rsplit_once('/').unwrap().0.to_string();
    assert_eq!(s.get(&format!("/api{sealed_dir}/embed.html"), &[]).status, 401);
}

// ---- the websocket -------------------------------------------------------------------------------

#[test]
fn the_websocket_is_relayed_and_only_the_handshake_goes_to_the_guest() {
    let s = screen("scr-ws");
    let (_, token) = s.phone(watch_only());
    let (path, _) = s.src(&token, "view");
    let dir = path.rsplit_once('/').unwrap().0.to_string();
    let (mut ws, head) = s.socket(&dir, &["Authorization: Bearer nope", "Cookie: a=b", &format!("Origin: http://127.0.0.1:{}", s.port()), "X-Forwarded-For: 203.0.113.9"]);
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert!(echo(&mut ws, "hello guest"));
    let heads = s.guest.heads.lock().clone();
    assert_eq!(heads.len(), 1);
    let sent = heads[0].to_ascii_lowercase();
    for banned in ["authorization", "cookie", "origin", "x-forwarded", "bearer", "nope"] {
        assert!(!sent.contains(banned), "{banned} reached the guest: {sent}");
    }
    assert!(sent.starts_with("get /websockify http/1.1") && sent.contains("sec-websocket-key"), "{sent}");
    // an idle screen is not cut by the server's request timeout, and a view relay survives the lease
    let _ = s.hub().take_over();
    let _ = s.hub().hand_back();
    assert!(echo(&mut ws, "still here"));
    // no upgrade, no websocket
    assert_eq!(s.get(&format!("{dir}/websockify"), &[]).status, 400);
}

#[test]
fn the_websocket_closes_when_the_pairing_is_revoked() {
    let s = screen("scr-ws-revoke");
    let (id, token) = s.phone(watch_only());
    let (path, _) = s.src(&token, "view");
    let dir = path.rsplit_once('/').unwrap().0.to_string();
    let (mut ws, head) = s.socket(&dir, &[]);
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert!(echo(&mut ws, "alive"));
    assert!(s.k.pairing.lock().revoke_phone_device(&id).unwrap());
    assert!(closes_within(&mut ws, Duration::from_secs(4)), "the relay must close within a few seconds of the sign-out");
    // and a new socket with the same link is refused
    let (_, head) = s.socket(&dir, &[]);
    assert!(head.starts_with("HTTP/1.1 403"), "{head}");
}

#[test]
fn a_control_websocket_closes_when_the_yes_is_taken_back_or_the_pairing_runs_out() {
    let s = screen("scr-ws-control");
    let (id, token) = s.phone(control_caps());
    s.hub().allow_phone_control(&id);
    let (path, _) = s.src(&token, "control");
    let dir = path.rsplit_once('/').unwrap().0.to_string();
    // the person's yes taken back at the PC
    let (mut ws, head) = s.socket(&dir, &[]);
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert!(echo(&mut ws, "typing"));
    s.hub().revoke_phone_control(&id);
    assert!(closes_within(&mut ws, Duration::from_secs(4)), "closes when the yes is taken back");
    // the pairing runs out
    s.hub().allow_phone_control(&id);
    let (mut ws, head) = s.socket(&dir, &[]);
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    s.k.pairing.lock().set_device_expiry_for_test(&id, 1);
    assert!(closes_within(&mut ws, Duration::from_secs(4)), "closes when the pairing is no longer good");
}

#[test]
fn a_websocket_with_a_foreign_origin_or_link_is_refused() {
    let s = screen("scr-ws-bad");
    let (_, token) = s.phone(watch_only());
    let (path, _) = s.src(&token, "view");
    let dir = path.rsplit_once('/').unwrap().0.to_string();
    let (_, head) = s.socket(&dir, &["Origin: https://evil.example.com"]);
    assert!(head.starts_with("HTTP/1.1 403"), "{head}");
    let (_, head) = s.socket("/screen/garbage", &[]);
    assert!(head.starts_with("HTTP/1.1 404"), "{head}");
    assert!(s.guest.heads.lock().is_empty(), "nothing reached the guest");
}

#[test]
fn the_pcs_switch_gives_the_yes_and_taking_it_back_closes_a_live_control_relay() {
    use super::control;
    let s = screen("scr-pc-switch");
    let (id, token) = s.phone(control_caps());
    // Default off: the phone's control request is refused until the person says yes at the PC.
    let (status, v) = s.post(&token, "nimbus", "control");
    assert_eq!((status, v["error"].as_str()), (403, Some("phone_needs_desktop_yes")), "{v}");
    // The PC's switch: saved with the pairing, and the hub has the yes at once.
    control::set(&s.k.pairing, s.hub(), &id, true).unwrap();
    assert!(s.hub().phone_may_control(&id));
    assert!(s.k.pairing.lock().phone_control_ids().contains(&id));
    let (path, _) = s.src(&token, "control");
    let dir = path.rsplit_once('/').unwrap().0.to_string();
    let (mut ws, head) = s.socket(&dir, &[]);
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert!(echo(&mut ws, "typing"));
    // Switch off: the relay's own per-second recheck closes it, and a new request is refused again.
    control::set(&s.k.pairing, s.hub(), &id, false).unwrap();
    assert!(closes_within(&mut ws, Duration::from_secs(4)), "the control relay closes when the PC takes the yes back");
    assert!(!s.k.pairing.lock().phone_control_ids().contains(&id));
    let (status, v) = s.post(&token, "nimbus", "control");
    assert_eq!((status, v["error"].as_str()), (403, Some("phone_needs_desktop_yes")), "{v}");
}
