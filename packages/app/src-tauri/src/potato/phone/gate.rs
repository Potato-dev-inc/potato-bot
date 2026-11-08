//! Every request to the phone server passes through `dispatch`, in this order:
//!
//! 1. `Host` must be a name we answer to (an IP, `localhost`, `*.local`, this computer, the person's own address,
//!    the running tunnel). Anything else is 421: a web page that reaches us by DNS rebinding carries its own name.
//! 2. `Origin`, when there is one, must be this server's own origin or one of its own addresses; otherwise 403.
//!    Only those get CORS headers (the app on the LAN address talking to the tunnel). Never `*`.
//! 3. Anything that changes something needs the header `X-Potato-Phone: 1`: a web page on another site cannot send
//!    it without a CORS preflight, and the preflight is refused.
//! 4. Only the app files, `/api/health`, `/api/hello` and `/api/pair` are open. Every other path needs a device
//!    token (`Authorization: Bearer`), whoever the caller is and wherever the request came from. Loopback is
//!    NOT trust here: the tunnel program and any local forwarder arrive from 127.0.0.1. Forwarder headers are
//!    never a reason to trust a request; they only put it on a tighter rate limit.
//! 5. The token's abilities (watch, type, answer, control) must cover the route.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::http::{Request, Response};
use super::net;
use super::proof;
use super::rate::Limiter;
use crate::mobile_pairing::{Caps, PairFail, SharedPairingStore, TokenCheck};

/// A counter that goes up whenever anything a phone shows may have changed, and lets a request wait for that.
pub struct Changes {
    rev: Mutex<u64>,
    cv: Condvar,
}

impl Changes {
    pub fn new() -> Changes {
        Changes { rev: Mutex::new(1), cv: Condvar::new() }
    }

    pub fn bump(&self) {
        *self.rev.lock().unwrap() += 1;
        self.cv.notify_all();
    }

    pub fn rev(&self) -> u64 {
        *self.rev.lock().unwrap()
    }

    /// The revision once it is above `since`, or after `wait` whatever it is.
    pub fn wait(&self, since: u64, wait: Duration) -> u64 {
        let guard = self.rev.lock().unwrap();
        let (guard, _) = self.cv.wait_timeout_while(guard, wait, |rev| *rev <= since).unwrap();
        *guard
    }
}

impl Default for Changes {
    fn default() -> Self {
        Changes::new()
    }
}

/// What the server needs from the app around it. The real one is `live::LiveEnv`; tests bring their own.
pub trait Env: Send + Sync {
    fn service(&self) -> Option<Arc<crate::potato::service::Service>>;
    fn groups_on(&self) -> bool;
    /// Whether the Labs switch for Spaces is on. Off: the Spaces routes answer `409 spaces_off`.
    fn spaces_on(&self) -> bool {
        false
    }
    /// The running Spaces service, when Spaces is on and has started.
    fn spaces(&self) -> Option<Arc<crate::spaces::service::SpacesService>> {
        None
    }
    fn pc_name(&self) -> String;
    /// Every origin a phone may try, best first.
    fn candidates(&self) -> Vec<String>;
    fn hosts(&self) -> net::Hosts;
    fn now_ms(&self) -> u64;
    fn pause_all(&self) -> crate::potato::hold::Hold;
    fn resume_all(&self) -> Result<crate::potato::hold::Hold, String>;
    fn paused(&self) -> bool;
    /// Web push, when this build has it ready.
    fn push(&self) -> Option<Arc<crate::push::hub::PushHub>> {
        None
    }
    /// The app's one sandbox screen hub (sealed links, lease, the person's yes). Tests bring their own.
    fn screen_hub(&self) -> Arc<crate::potato::sandbox::screen::ScreenHub> {
        crate::potato::sandbox::screen::hub().clone()
    }
    /// The folder with the screen viewer (`embed.html`, `novnc/`), when this install has it.
    fn viewer_dir(&self) -> Option<PathBuf> {
        crate::potato::sandbox::screen_live::viewer_dir()
    }
}

pub struct Ctx {
    pub pairing: SharedPairingStore,
    pub env: Arc<dyn Env>,
    pub limiter: Limiter,
    pub changes: Arc<Changes>,
    /// The folder with `index.html`, `app.js`, ...
    pub assets: Option<PathBuf>,
}

/// Who a rate limit is counted against. A tunnel hides the phone's address behind loopback, so when the
/// connecting-IP header is there it is used as a label only (it can be forged by a LAN caller, which is why it is
/// never trusted for anything but this).
pub fn client_key(req: &Request) -> String {
    match req.header("cf-connecting-ip").or_else(|| req.header("x-forwarded-for")) {
        Some(ip) => format!("fwd:{}", ip.split(',').next().unwrap_or("").trim()),
        None => format!("peer:{}", req.peer),
    }
}

fn json_ok(value: Value) -> Response {
    Response::json(200, &value)
}

pub(super) fn security_headers(mut r: Response, html: bool) -> Response {
    r = r
        .with("Cache-Control", "no-store")
        .with("X-Content-Type-Options", "nosniff")
        .with("Referrer-Policy", "no-referrer")
        .with("X-Frame-Options", "DENY");
    if html {
        r = r.with(
            "Content-Security-Policy",
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self' http: https: ws: wss:; manifest-src 'self'; worker-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        );
    }
    r
}

/// Host and Origin rules (1 and 2). `Ok(cors)`: the origin to echo in `Access-Control-Allow-Origin`, if any.
pub fn check_host_and_origin(ctx: &Ctx, req: &Request, is_api: bool) -> Result<Option<String>, Response> {
    let Some(host) = req.header("host") else {
        return Err(Response::error(400, "bad_request", "No Host header."));
    };
    if !ctx.env.hosts().allows(host) {
        return Err(Response::error(421, "bad_host", "This address is not one this phone server answers to."));
    }
    let Some(origin) = req.header("origin") else { return Ok(None) };
    let Some(norm) = net::origin_of(origin) else {
        return if is_api { Err(Response::error(403, "bad_origin", "This page may not use the phone server.")) } else { Ok(None) };
    };
    let authority = norm.split_once("://").map(|(_, a)| a).unwrap_or("");
    if authority == host.trim().to_ascii_lowercase() {
        return Ok(None); // our own page
    }
    let known = ctx.env.candidates().iter().filter_map(|c| net::origin_of(c)).any(|c| c == norm);
    if known {
        return Ok(Some(norm));
    }
    if is_api {
        Err(Response::error(403, "bad_origin", "This page may not use the phone server."))
    } else {
        Ok(None)
    }
}

fn with_cors(mut r: Response, cors: &Option<String>) -> Response {
    if let Some(origin) = cors {
        r = r
            .with("Access-Control-Allow-Origin", origin)
            .with("Vary", "Origin")
            .with("Access-Control-Allow-Headers", "Authorization, Content-Type, X-Potato-Phone")
            .with("Access-Control-Allow-Methods", "GET, POST, DELETE, OPTIONS")
            .with("Access-Control-Max-Age", "600");
    }
    r
}

/// `/api/v1/x` and `/api/x` are the same route.
fn api_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/api")?;
    let rest = rest.strip_prefix("/v1").unwrap_or(rest);
    (rest.is_empty() || rest.starts_with('/')).then(|| rest.to_string())
}

pub fn dispatch(ctx: &Ctx, req: &Request) -> Response {
    let api = api_path(&req.path);
    let cors = match check_host_and_origin(ctx, req, api.is_some()) {
        Ok(cors) => cors,
        Err(r) => return security_headers(r, false),
    };
    let response = match api {
        Some(path) => dispatch_api(ctx, req, &path, &cors),
        None => serve_static(ctx, req),
    };
    with_cors(security_headers(response, req.path == "/" || req.path.ends_with(".html")), &cors)
}

fn dispatch_api(ctx: &Ctx, req: &Request, path: &str, cors: &Option<String>) -> Response {
    if req.method == "OPTIONS" {
        // A preflight: answered only when the origin check above let a known origin through.
        return if cors.is_some() { Response::bytes(204, "text/plain", Vec::new()) } else { Response::error(403, "bad_origin", "This page may not use the phone server.") };
    }
    let changes_state = req.method != "GET" && req.method != "HEAD";
    if changes_state && req.header("x-potato-phone") != Some("1") {
        return Response::error(403, "bad_request", "That request is missing its phone header.");
    }
    let now = ctx.env.now_ms();
    let fwd = req.forwarded();
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match (req.method.as_str(), segments.as_slice()) {
        ("GET", ["health"]) => return json_ok(json!({ "ok": true, "app": "potato-phone", "v": 1 })),
        ("POST", ["hello"]) => return hello(ctx, req, fwd, now),
        ("POST", ["pair"]) => return pair(ctx, req, fwd, now),
        _ => {}
    }
    // Everything else: a device token, from anyone, from anywhere.
    let (limit, window) = if fwd { (30, 60_000) } else { (120, 60_000) };
    let Some(token) = bearer(req) else {
        // No token is cheap to refuse, but a flood of wrong ones is counted.
        return Response::error(401, "unpaired", "This phone is not paired. Scan the QR on your PC.");
    };
    if !ctx.limiter.allow(&format!("auth:{}", client_key(req)), limit, window, now) {
        return Response::error(429, "rate_limited", "Too many tries. Wait a minute.");
    }
    let check = ctx.pairing.lock().check_token(&token);
    let (device_id, caps) = match check {
        TokenCheck::Valid { device_id, caps } => (device_id, caps),
        TokenCheck::Expired { .. } => return Response::error(401, "unpaired", "This phone was signed out after 30 days. Scan the QR on your PC again."),
        TokenCheck::Unknown => return Response::error(401, "unpaired", "This phone is not paired. Scan the QR on your PC."),
    };
    let need = super::api::need_for(req, &segments);
    if let Some(cap) = need {
        if !caps.has(cap) {
            return Response::error(403, "not_allowed", &format!("This phone is not allowed to do that ({}). You can change it on your PC: Settings > Phone.", cap.as_str()));
        }
    }
    super::api::route(ctx, req, &segments, &Caller { device_id, caps })
}

pub struct Caller {
    pub device_id: String,
    pub caps: Caps,
}

fn bearer(req: &Request) -> Option<String> {
    let value = req.header("authorization")?;
    let token = value.strip_prefix("Bearer ").or_else(|| value.strip_prefix("bearer "))?.trim();
    (!token.is_empty() && token.len() <= 200).then(|| token.to_string())
}

fn parse_body(req: &Request) -> Result<Value, Response> {
    serde_json::from_slice(&req.body).map_err(|_| Response::error(400, "bad_request", "That is not the JSON this server expects."))
}

fn hello(ctx: &Ctx, req: &Request, fwd: bool, now: u64) -> Response {
    let (limit, window) = if fwd { (20, 60_000) } else { (90, 60_000) };
    if !ctx.limiter.allow("hello", limit, window, now) || !ctx.limiter.allow(&format!("hello:{}", client_key(req)), limit, window, now) {
        return Response::error(429, "rate_limited", "Too many tries. Wait a minute.");
    }
    let body = match parse_body(req) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let nonce = body.get("nonce").and_then(Value::as_str).unwrap_or("");
    if !proof::nonce_ok(nonce) {
        return Response::error(400, "bad_request", "Send a fresh random nonce (16 to 64 letters or digits).");
    }
    let kind = body.get("kind").and_then(Value::as_str).unwrap_or("");
    let key = match kind {
        "pair" => ctx.pairing.lock().pending_hello_key(),
        "device" => body.get("deviceId").and_then(Value::as_str).and_then(|id| ctx.pairing.lock().device_hello_key(id)),
        _ => return Response::error(400, "bad_request", "kind is pair or device."),
    };
    // No such code or phone: answer anyway, with a proof under a throwaway key, so the answer tells a guesser nothing.
    let key = key.unwrap_or_else(|| {
        use rand::RngCore;
        let mut k = vec![0_u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut k);
        k
    });
    json_ok(json!({ "proof": proof::proof(&key, nonce), "pcName": ctx.env.pc_name() }))
}

fn pair(ctx: &Ctx, req: &Request, fwd: bool, now: u64) -> Response {
    // A tight limit on the open route, tighter through a tunnel. One shared budget, so many callers do not add up.
    let (limit, window) = if fwd { (4, 60_000) } else { (10, 60_000) };
    if !ctx.limiter.allow("pair", limit, window, now) || !ctx.limiter.allow(&format!("pair:{}", client_key(req)), limit, window, now) {
        return Response::error(429, "rate_limited", "Too many tries. Wait a minute, then try again.");
    }
    let body = match parse_body(req) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let code = body.get("code").and_then(Value::as_str).unwrap_or("");
    let name = body.get("deviceName").and_then(Value::as_str).unwrap_or("Phone");
    if code.is_empty() || code.len() > 32 {
        return Response::error(400, "bad_code", "That code is not right. Make a new one on your PC.");
    }
    let outcome = ctx.pairing.lock().redeem_phone_code(code, name);
    match outcome {
        Ok(grant) => {
            crate::audit::record(
                crate::audit::Kind::Pairing,
                crate::audit::Who::Device(Some(grant.device_id.clone())),
                if grant.relinked { "A phone linked again with this computer" } else { "A phone paired with this computer" },
                json!({ "deviceId": grant.device_id }),
            );
            ctx.changes.bump();
            json_ok(json!({ "deviceId": grant.device_id, "token": grant.token, "pcName": ctx.env.pc_name(), "candidates": ctx.env.candidates() }))
        }
        Err(PairFail::BadCode) => Response::error(400, "bad_code", "That code is not right. Check it, or make a new one on your PC."),
        Err(PairFail::Expired) => Response::error(410, "expired", "That code ran out. Make a new one on your PC."),
        Err(PairFail::Burned) => Response::error(400, "burned", "Too many wrong tries. Make a new code on your PC."),
    }
}

// ---- the app's own files ----------------------------------------------------------------------------------------

fn content_type(name: &str) -> Option<&'static str> {
    Some(match name.rsplit_once('.')?.1 {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        _ => return None,
    })
}

fn serve_static(ctx: &Ctx, req: &Request) -> Response {
    if req.method != "GET" && req.method != "HEAD" {
        return Response::error(405, "bad_method", "Use GET.");
    }
    let Some(root) = &ctx.assets else {
        return Response::error(503, "no_app", "The phone app files are not installed.");
    };
    let rel = req.path.trim_start_matches('/');
    // Only plain file names under the app folder: no folders walked up, no hidden files, no tools or tests.
    let name = if rel.is_empty() { "index.html" } else { rel };
    let plain = !name.contains(['\\', ':']) && name.split('/').all(|p| !p.is_empty() && !p.starts_with('.')) && !name.starts_with("tools/");
    let file = if plain { content_type(name).map(|_| root.join(name)) } else { None };
    match file.and_then(|f| std::fs::read(&f).ok().map(|b| (f, b))) {
        Some((file, bytes)) => {
            let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let mut r = Response::bytes(200, content_type(name).unwrap_or("application/octet-stream"), bytes);
            if name == "sw.js" {
                r = r.with("Service-Worker-Allowed", "/");
            }
            r
        }
        // A path like /pair/ABC that is not a file: the app opens and reads its own fragment.
        None if !rel.contains('.') => match std::fs::read(root.join("index.html")) {
            Ok(bytes) => Response::bytes(200, "text/html; charset=utf-8", bytes),
            Err(_) => Response::error(404, "not_found", "Not found."),
        },
        None => Response::error(404, "not_found", "Not found."),
    }
}
