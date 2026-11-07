//! `GET /api/v1/ws`: the same "something changed" signal as the long poll, over a WebSocket, for a later native
//! app. Quick tunnels do not carry Server-Sent Events but do carry WebSockets.
//!
//! Auth is the device token, offered as the subprotocol `pm-bearer.<token>` next to `pm.v1` (a browser
//! cannot set a header on a WebSocket). The server answers `pm.v1`, so the token is never echoed. The
//! messages are text frames of JSON: `{"type":"changed","rev":N}` (the first one at once, then on every change)
//! and `{"type":"signed_out"}` before it closes when the phone is revoked.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::gate::{self, Ctx};
use super::http::{write_response, Request, Response};
use crate::mobile_pairing::{Cap, TokenCheck};

pub struct Upgrade {
    key: String,
}

const PATHS: &[&str] = &["/api/v1/ws", "/api/ws"];

pub fn wants_upgrade(req: &Request) -> Option<Upgrade> {
    if req.method != "GET" || !PATHS.contains(&req.path.as_str()) {
        return None;
    }
    let upgrade = req.header("upgrade")?;
    if !upgrade.eq_ignore_ascii_case("websocket") {
        return None;
    }
    Some(Upgrade { key: req.header("sec-websocket-key")?.to_string() })
}

/// The token from the offered subprotocols (`pm-bearer.<token>`).
fn offered_token(req: &Request) -> Option<String> {
    let offered = req.header("sec-websocket-protocol")?;
    let has_version = offered.split(',').any(|p| p.trim() == "pm.v1");
    let token = offered.split(',').find_map(|p| p.trim().strip_prefix("pm-bearer.").map(str::to_string))?;
    has_version.then_some(token)
}

fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0x80 | opcode];
    match payload.len() {
        n if n < 126 => out.push(n as u8),
        n if n <= 0xffff => {
            out.push(126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(payload);
    out
}

pub fn serve(mut stream: TcpStream, ctx: &Arc<Ctx>, req: &Request, upgrade: Upgrade, stop: &AtomicBool) {
    let refuse = |stream: &mut TcpStream, r: Response| {
        let _ = write_response(stream, &r, false);
    };
    if let Err(r) = gate::check_host_and_origin(ctx, req, true) {
        return refuse(&mut stream, r);
    }
    if !ctx.limiter.allow(&format!("auth:{}", gate::client_key(req)), 60, 60_000, ctx.env.now_ms()) {
        return refuse(&mut stream, Response::error(429, "rate_limited", "Too many tries. Wait a minute."));
    }
    let Some(token) = offered_token(req) else {
        return refuse(&mut stream, Response::error(401, "unpaired", "This phone is not paired."));
    };
    let device_id = match ctx.pairing.lock().check_token(&token) {
        TokenCheck::Valid { device_id, caps } if caps.has(Cap::Watch) => device_id,
        TokenCheck::Valid { .. } => return refuse(&mut stream, Response::error(403, "not_allowed", "This phone may not watch.")),
        _ => return refuse(&mut stream, Response::error(401, "unpaired", "This phone is not paired.")),
    };
    let accept = tungstenite::handshake::derive_accept_key(upgrade.key.as_bytes());
    let head = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: pm.v1\r\n\r\n");
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    let _ = stream.set_read_timeout(None);

    // A reader thread only notices the phone going away (a close frame or the end of the stream).
    let closed = Arc::new(AtomicBool::new(false));
    if let Ok(mut reader) = stream.try_clone() {
        let closed = closed.clone();
        std::thread::spawn(move || {
            let mut buf = [0_u8; 512];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) if n >= 1 && buf[0] & 0x0f == 0x8 => break,
                    Ok(_) => {}
                }
            }
            closed.store(true, Ordering::SeqCst);
        });
    }

    let mut seen = 0_u64;
    let mut first = true;
    loop {
        if stop.load(Ordering::SeqCst) || closed.load(Ordering::SeqCst) {
            break;
        }
        let rev = if first { ctx.changes.rev() } else { ctx.changes.wait(seen, Duration::from_secs(10)) };
        // A phone that was signed out is told before anything else, and cut off.
        if crate::mobile_pairing::stream_caps(&device_id).is_none() {
            let _ = stream.write_all(&frame(0x1, br#"{"type":"signed_out"}"#));
            let _ = stream.write_all(&frame(0x8, &[]));
            break;
        }
        let message = if first || rev > seen {
            first = false;
            seen = rev;
            frame(0x1, serde_json::json!({ "type": "changed", "rev": rev }).to_string().as_bytes())
        } else {
            frame(0x9, &[]) // a ping, so a dead phone is found
        };
        if stream.write_all(&message).is_err() {
            break;
        }
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}
