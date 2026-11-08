//! `GET /screen/<sealed>/...` on the phone server: the sandbox screen viewer and its websocket, for the iframe a paired
//! phone's page opens from the `src` that `POST /api/bots/{id}/screen` gave it.
//!
//! An iframe and a websocket cannot send the `Authorization` header, so here (and only here) the sealed link in the path
//! is the credential, exactly as on the bridge. It is made for ONE paired phone (`Audience::Device`), by the hub, and
//! every request still passes:
//!
//! 1. the same Host and Origin rules as the API (`gate::check_host_and_origin`, strict): a foreign `Host` is 421, a
//!    foreign `Origin` is 403;
//! 2. the hub is on, and the link names a phone (an app link, a bad or an expired tag is 404, no hint which);
//! 3. that phone is still paired and may still watch (`Caps` read from the pairing store, so a revoked phone is 403);
//! 4. `screen_serve::serve_sealed`: tag, clock, audience (another device's link is refused), sandbox generation and,
//!    for control, the person's lease and the PC's yes. An open websocket is checked again every second, and also that
//!    the phone's pairing still allows watching (and, for control, typing and control), and is closed when any check fails.
//!
//! The viewer page is told to post its status only to this page's own origin (the origin the request came in on, which
//! step 1 vouched for), never `*`. Nothing here logs; the VNC password is in the `#` fragment the phone page built, which no
//! request carries.

use std::net::TcpStream;
use std::sync::Arc;

use super::gate::{check_host_and_origin, client_key, security_headers, Ctx};
use super::http::{write_response, Request, Response};
use crate::mobile_pairing::Cap;
use crate::potato::sandbox::screen::{Audience, Mode};
use crate::potato::sandbox::screen_serve::{self, StillAllowed};

/// Where the phone server serves the viewer (the `src` of a screen link starts with it).
pub const ROUTE_ROOT: &str = "/screen";

/// A request for a viewer file or the websocket is not a `/api` route and not an app file.
pub fn is_screen_path(path: &str) -> bool {
    path.strip_prefix(ROUTE_ROOT).is_some_and(|rest| rest.starts_with('/'))
}

fn refuse(stream: &mut TcpStream, r: Response) {
    let r = security_headers(r, false);
    let _ = write_response(stream, &r, false);
}

/// The origin of the page this request came in on: scheme from the tunnel's forwarded headers (a quick tunnel is https),
/// host from the `Host` header the gate already checked.
fn page_origin(req: &Request) -> Option<String> {
    let host = req.header("host")?.trim().to_ascii_lowercase();
    if host.is_empty() || !host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']')) {
        return None;
    }
    let proto = req.header("x-forwarded-proto").and_then(|p| p.split(',').next()).map(|p| p.trim().to_ascii_lowercase());
    let https = match proto.as_deref() {
        Some(p) => p == "https",
        None => req.header("cf-visitor").is_some_and(|v| v.contains("https")) || req.forwarded(),
    };
    Some(format!("{}://{host}", if https { "https" } else { "http" }))
}

/// The request as the header text the shared relay reads (`Upgrade`, `Sec-WebSocket-*`, ...).
fn headers_text(req: &Request) -> String {
    let mut out = String::new();
    for (k, v) in &req.headers {
        if !k.contains(['\r', '\n']) && !v.contains(['\r', '\n']) {
            out.push_str(&format!("{k}: {v}\r\n"));
        }
    }
    out
}

fn not_found() -> Response {
    Response::error(404, "not_found", "Not found.")
}

/// Whether the phone may still do what a link of this mode is for.
fn allows(caps: crate::mobile_pairing::Caps, mode: Mode) -> bool {
    caps.has(Cap::Watch) && (mode == Mode::View || (caps.has(Cap::Type) && caps.has(Cap::Control)))
}

/// Answers one request under `/screen/`. Writes to `stream` itself (a websocket keeps the socket open).
pub fn serve(stream: &mut TcpStream, ctx: &Arc<Ctx>, req: &Request) {
    // 1. Host and Origin, strictly: the viewer is as sensitive as the API.
    if let Err(r) = check_host_and_origin(ctx, req, true) {
        return refuse(stream, r);
    }
    let now = ctx.env.now_ms();
    let (limit, window) = if req.forwarded() { (300, 60_000) } else { (600, 60_000) };
    if !ctx.limiter.allow(&format!("screen:{}", client_key(req)), limit, window, now) {
        return refuse(stream, Response::error(429, "rate_limited", "Too many tries. Wait a minute."));
    }
    let segments: Vec<&str> = req.path.split('/').filter(|s| !s.is_empty()).collect();
    let (Some(sealed), rest) = (segments.get(1).copied(), segments.get(2..).unwrap_or(&[])) else { return refuse(stream, not_found()) };

    // 2. The hub is on and the link is a phone's.
    let hub = ctx.env.screen_hub();
    if !hub.enabled() {
        return refuse(stream, not_found());
    }
    let Some(Audience::Device(device_id)) = hub.peek_audience(sealed) else { return refuse(stream, not_found()) };

    // 3. That phone is still paired and may still watch.
    let caps = ctx.pairing.lock().device_caps(&device_id);
    if !caps.is_some_and(|caps| caps.has(Cap::Watch)) {
        return refuse(stream, Response::error(403, "not_allowed", "This phone may not watch right now."));
    }

    // 4. The shared serving: the tag, the clock, the audience, the generation, the lease; the relay is rechecked.
    let Some(origin) = page_origin(req) else { return refuse(stream, Response::error(400, "bad_request", "No usable Host header.")) };
    let pairing = ctx.pairing.clone();
    let id = device_id.clone();
    let still: StillAllowed = Arc::new(move |mode| pairing.lock().device_caps(&id).is_some_and(|caps| allows(caps, mode)));
    let viewer = ctx.env.viewer_dir();
    let _ = screen_serve::serve_sealed(stream, &hub, viewer.as_deref(), &req.method, &headers_text(req), sealed, rest, &Audience::Device(device_id), &[origin], still);
}
