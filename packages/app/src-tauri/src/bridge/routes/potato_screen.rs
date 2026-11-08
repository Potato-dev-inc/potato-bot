//! `/potato/sandbox/screen/<sealed>/...`: the sandbox screen viewer (spec "screen.rs"), and
//! `POST /potato/sandbox/screen-link`: how a paired phone gets a link.
//!
//! - `GET .../embed.html` (and the viewer's own scripts and `novnc/`): the files of `packages/potato-sandbox/viewer/`.
//!   The page gets its parent-origin allowlist filled in (`screen::parent_origins`) and is sent with
//!   `Cache-Control: no-store, no-transform`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer` and a
//!   `frame-ancestors` policy for those same origins.
//! - `GET .../websockify` with `Upgrade: websocket`: relayed byte for byte to the guest's websockify on
//!   `127.0.0.1:<view port>`. Only the handshake headers the websocket needs are forwarded (no cookie, no bearer, no
//!   local secret, no `Origin`). While the relay is open it is checked every second (`ScreenHub::revalidate`, and for a
//!   phone that its pairing still allows watching) and closed the moment the link stops being good: the lease was lost,
//!   the generation moved on, the link ran out, a Labs switch went off.
//! - `POST /potato/sandbox/screen-link` `{ "botId", "mode": "view"|"control" }`: a paired phone's way to a link. The
//!   desktop gets its links through the Tauri command, never through the bridge: a local program (an agent in a pane
//!   calls the bridge too) must not be able to ask for a screen password.
//!
//! Who may call: `gate` runs from `bridge/http.rs::gate_with`. A request that does not need a pairing token (a loopback
//! peer with no forwarder header, no foreign `Host`, no foreign `Origin`) is the app's own webview; anything else must be
//! a link made for a paired device that may still watch. Then the sealed link decides what it reaches
//! (`ScreenHub::open`). The VNC password is never in a path or a query and never on this side of the viewer: it is in the
//! URL fragment of the iframe address, which no request carries. Nothing here logs.

use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::mobile_pairing::{self, Cap, Caller};
use crate::potato::sandbox::screen::{self, Audience, Mode, ScreenHub};
use crate::potato::sandbox::screen_serve::{self, not_found, respond_json, StillAllowed};

/// Whether these segments are under `/potato/sandbox/screen/<sealed>/`.
pub(in crate::bridge) fn is_screen_path(segments: &[&str]) -> bool {
    screen::route_parts(segments).is_some()
}

/// The caller kind for a screen request, or `None` when the request is not about the screen.
pub(in crate::bridge) fn gate(hub: &ScreenHub, headers: &str, peer_loopback: bool, segments: &[&str]) -> Option<Result<Caller, (u16, Value)>> {
    let (sealed, _) = screen::route_parts(segments)?;
    if !hub.enabled() {
        return Some(Err(not_found()));
    }
    if !mobile_pairing::requires_pairing_token(headers, peer_loopback) {
        return Some(Ok(Caller::Local));
    }
    // From outside: only a link made for a paired phone that may still watch.
    let Some(Audience::Device(device_id)) = hub.peek_audience(sealed) else { return Some(Err(not_found())) };
    Some(match mobile_pairing::stream_caps(&device_id) {
        Some(caps) if caps.has(Cap::Watch) => Ok(Caller::Device { device_id, caps }),
        _ => Err((403, json!({ "code": "DEVICE_NOT_ALLOWED", "error": "This phone may not watch right now." }))),
    })
}

fn audience_of(caller: &Caller) -> Option<Audience> {
    match caller {
        Caller::Local => Some(Audience::App),
        Caller::Device { device_id, .. } => Some(Audience::Device(device_id.clone())),
        Caller::Public | Caller::Brain { .. } => None,
    }
}

/// Whether the phone behind `caller` may still do what the link is for (watch; for control also type).
fn caller_still_allowed(caller: &Caller, mode: Mode) -> bool {
    match caller {
        Caller::Local => true,
        Caller::Device { device_id, .. } => mobile_pairing::stream_caps(device_id).is_some_and(|caps| {
            caps.has(Cap::Watch) && (mode == Mode::View || caps.has(Cap::Type))
        }),
        Caller::Public | Caller::Brain { .. } => false,
    }
}

/// One request under `/potato/sandbox/screen/<sealed>/`. `caller` is what `gate` decided. `viewer` is the folder with
/// `embed.html` and `novnc/`. The serving and the relay are `screen_serve` (shared with the phone server).
pub(in crate::bridge) fn serve(
    stream: &mut TcpStream,
    hub: &Arc<ScreenHub>,
    viewer: Option<&Path>,
    method: &str,
    headers: &str,
    segments: &[&str],
    caller: &Caller,
) -> Result<(), String> {
    let Some((sealed, rest)) = screen::route_parts(segments) else { return respond_json(stream, 404, &not_found().1) };
    let Some(audience) = audience_of(caller) else { return respond_json(stream, 403, &json!({ "error": "not allowed" })) };
    let origins = screen::parent_origins(headers, matches!(caller, Caller::Device { .. }));
    let caller_for_check = caller.clone();
    let still: StillAllowed = Arc::new(move |mode| caller_still_allowed(&caller_for_check, mode));
    screen_serve::serve_sealed(stream, hub, viewer, method, headers, sealed, rest, &audience, &origins, still)
}
