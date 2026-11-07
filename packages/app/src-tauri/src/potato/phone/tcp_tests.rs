//! The server on real sockets: what the door does when bytes really arrive from loopback (as a tunnel's do),
//! the WebSocket, the port, and a body that is too big.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use super::http::{start, start_for_test};
use super::testkit::{kit, Kit};

fn raw(port: u16, request: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(request.as_bytes()).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default())
}

fn get(port: u16, path: &str, extra: &str) -> (u16, String) {
    raw(port, &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{extra}Connection: close\r\n\r\n"))
}

#[test]
fn over_a_real_socket_from_loopback_with_tunnel_headers_and_no_token_is_refused() {
    let k = kit("tcp-fwd", None);
    let server = start_for_test(k.ctx.clone());
    let port = server.addr.port();
    assert_eq!(get(port, "/api/health", "").0, 200);
    for extra in ["Cf-Connecting-Ip: 203.0.113.9\r\n", "X-Forwarded-For: 203.0.113.9\r\n", "X-Forwarded-For: 127.0.0.1\r\nCf-Ray: 1\r\n", ""] {
        for path in ["/api/me", "/api/bots", "/api/bots/nimbus", "/api/changes?since=0&wait_ms=0"] {
            let (status, body) = get(port, path, extra);
            assert_eq!(status, 401, "{path} with {extra:?}: {body}");
        }
        let (status, _) = raw(port, &format!("POST /api/pause-all HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Potato-Phone: 1\r\n{extra}Content-Length: 2\r\n\r\n{{}}"));
        assert_eq!(status, 401, "POST with {extra:?}");
    }
    server.stop();
}

#[test]
fn a_real_pairing_round_trip_over_a_socket() {
    let k = kit("tcp-pair", None);
    let server = start_for_test(k.ctx.clone());
    let port = server.addr.port();
    let code = k.pairing.lock().mint_phone_code("x".into(), crate::mobile_pairing::Caps::all(), None).unwrap().code;
    let body = format!(r#"{{"code":"{code}","deviceName":"Phone"}}"#);
    let (status, text) = raw(port, &format!("POST /api/pair HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Potato-Phone: 1\r\nContent-Length: {}\r\n\r\n{body}", body.len()));
    assert_eq!(status, 200, "{text}");
    let token = serde_json::from_str::<serde_json::Value>(&text).unwrap()["token"].as_str().unwrap().to_string();
    let (status, me) = get(port, "/api/me", &format!("Authorization: Bearer {token}\r\n"));
    assert_eq!(status, 200, "{me}");
    server.stop();
}

#[test]
fn a_body_over_the_limit_is_refused_before_it_is_read() {
    let k = kit("tcp-big", None);
    let server = start_for_test(k.ctx.clone());
    let port = server.addr.port();
    let (status, _) = raw(port, &format!("POST /api/pair HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Potato-Phone: 1\r\nContent-Length: 999999999\r\n\r\n"));
    assert_eq!(status, 413);
    let (status, _) = raw(port, &format!("POST /api/pair HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n"));
    assert_eq!(status, 411);
    server.stop();
}

#[test]
fn a_port_in_use_gives_a_plain_error_and_stopping_gives_the_port_back() {
    let k = kit("tcp-port", None);
    let first = start_for_test(k.ctx.clone());
    let port = first.addr.port();
    let err = start("127.0.0.1".parse().unwrap(), port, k.ctx.clone()).err().expect("the port is taken");
    assert!(err.starts_with("PHONE_PORT_IN_USE:") && err.contains(&port.to_string()), "{err}");
    first.stop();
    std::thread::sleep(Duration::from_millis(300));
    let again = start("127.0.0.1".parse().unwrap(), port, k.ctx.clone()).expect("the port is free again");
    again.stop();
}

// ---- WebSocket -------------------------------------------------------------------------------------------------------

fn ws_open(port: u16, protocols: &str, extra: &str) -> (TcpStream, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let request = format!("GET /api/v1/ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: {protocols}\r\n{extra}\r\n");
    s.write_all(request.as_bytes()).unwrap();
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if s.read(&mut byte).unwrap_or(0) == 0 {
            break;
        }
        head.push(byte[0]);
    }
    (s, String::from_utf8_lossy(&head).into_owned())
}

/// One server frame's opcode and text (unmasked, short).
fn ws_text(s: &mut TcpStream) -> Option<(u8, String)> {
    let mut h = [0_u8; 2];
    s.read_exact(&mut h).ok()?;
    let mut payload = vec![0_u8; (h[1] & 0x7f) as usize];
    s.read_exact(&mut payload).ok()?;
    Some((h[0] & 0x0f, String::from_utf8_lossy(&payload).into_owned()))
}

fn ws_kit(name: &str) -> (Kit, super::http::Server, String) {
    let k = kit(name, None);
    let server = start_for_test(k.ctx.clone());
    let (_, token) = k.pair();
    (k, server, token)
}

#[test]
fn the_websocket_says_changed_at_once_and_on_every_change_and_never_echoes_the_token() {
    let (k, server, token) = ws_kit("ws-ok");
    let (mut s, head) = ws_open(server.addr.port(), &format!("pm.v1, pm-bearer.{token}"), "");
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert!(head.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="), "the accept key for the RFC sample key: {head}");
    assert!(head.contains("Sec-WebSocket-Protocol: pm.v1") && !head.contains(&token), "{head}");
    let (op, first) = ws_text(&mut s).unwrap();
    assert_eq!(op, 1);
    let rev = serde_json::from_str::<serde_json::Value>(&first).unwrap();
    assert_eq!(rev["type"], "changed");
    k.ctx.changes.bump();
    let (_, next) = ws_text(&mut s).unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&next).unwrap()["rev"].as_u64().unwrap() > rev["rev"].as_u64().unwrap());
    server.stop();
}

#[test]
fn the_websocket_needs_a_good_token_and_our_own_origin() {
    let (_k, server, token) = ws_kit("ws-bad");
    let port = server.addr.port();
    for protocols in ["pm.v1", "pm.v1, pm-bearer.nope", &format!("pm-bearer.{token}")] {
        let (_, head) = ws_open(port, protocols, "");
        assert!(head.starts_with("HTTP/1.1 401"), "{protocols}: {head}");
    }
    let (_, head) = ws_open(port, &format!("pm.v1, pm-bearer.{token}"), "Origin: https://evil.example.com\r\n");
    assert!(head.starts_with("HTTP/1.1 403"), "{head}");
    let (_, head) = ws_open(port, &format!("pm.v1, pm-bearer.{token}"), "X-Forwarded-For: 1.2.3.4\r\n");
    assert!(head.starts_with("HTTP/1.1 101"), "tunnel headers do not stop a phone with a token: {head}");
    server.stop();
}

#[test]
fn a_revoked_phone_is_told_and_cut_off_on_its_open_socket() {
    let (k, server, token) = ws_kit("ws-revoke");
    let (mut s, head) = ws_open(server.addr.port(), &format!("pm.v1, pm-bearer.{token}"), "");
    assert!(head.starts_with("HTTP/1.1 101"));
    let _ = ws_text(&mut s);
    let id = k.pairing.lock().phone_devices()[0].id.clone();
    k.pairing.lock().revoke_phone_device(&id).unwrap();
    k.ctx.changes.bump();
    let (_, text) = ws_text(&mut s).unwrap();
    assert_eq!(text, r#"{"type":"signed_out"}"#);
    assert_eq!(ws_text(&mut s).map(|(op, _)| op), Some(8), "then a close frame");
    server.stop();
}
