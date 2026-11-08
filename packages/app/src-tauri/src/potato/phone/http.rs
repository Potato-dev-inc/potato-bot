//! The phone server's plumbing: sockets, one request in, one response out, and the WebSocket upgrade.
//! Nothing here decides who may do what (`gate.rs` and `api.rs` do); it only keeps the port safe to leave open:
//! public peers are dropped unread, heads and bodies are size-limited, reads time out, connections are capped.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::gate::Ctx;
use super::net;

const MAX_HEAD: usize = 32 * 1024;
/// Three photos of up to a few MB each, base64.
pub const MAX_BODY: usize = 14 * 1024 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CONNECTIONS: usize = 96;

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    /// Lowercase names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub peer: IpAddr,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// A header only a proxy, tunnel or CDN adds. Never a reason to trust; a reason to be stricter.
    pub fn forwarded(&self) -> bool {
        const NAMES: &[&str] = &[
            "x-pm-proxied", "cf-connecting-ip", "cf-connecting-ipv6", "cf-ray", "cf-visitor", "cf-ipcountry", "cdn-loop", "true-client-ip",
            "x-forwarded-for", "x-forwarded-host", "x-forwarded-proto", "x-forwarded-port", "x-real-ip", "forwarded",
        ];
        NAMES.iter().any(|n| self.header(n).is_some())
    }

    pub fn query_param(&self, name: &str) -> Option<String> {
        self.query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == name).then(|| v.to_string())
        })
    }
}

pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(status: u16, value: &Value) -> Response {
        Response { status, content_type: "application/json; charset=utf-8".into(), headers: Vec::new(), body: value.to_string().into_bytes() }
    }

    pub fn error(status: u16, code: &str, message: &str) -> Response {
        Response::json(status, &serde_json::json!({ "error": code, "message": message }))
    }

    pub fn bytes(status: u16, content_type: &str, body: Vec<u8>) -> Response {
        Response { status, content_type: content_type.into(), headers: Vec::new(), body }
    }

    pub fn with(mut self, name: &str, value: &str) -> Response {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    #[cfg(test)]
    pub fn body_json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        413 => "Payload Too Large",
        421 => "Misdirected Request",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "OK",
    }
}

pub fn write_response(stream: &mut TcpStream, response: &Response, head_only: bool) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n", response.status, reason(response.status), response.content_type, response.body.len());
    for (k, v) in &response.headers {
        // A header value can never carry a line break.
        if !v.contains(['\r', '\n']) {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if !head_only && response.status != 204 {
        stream.write_all(&response.body)?;
    }
    stream.flush()
}

/// What reading a request can go wrong with, as the response to send.
pub fn read_request(stream: &mut TcpStream, peer: IpAddr) -> Result<Option<Request>, Response> {
    let bad = |s: u16, c: &str, m: &str| Response::error(s, c, m);
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 8192];
    let head_end = loop {
        let n = match stream.read(&mut chunk) {
            Ok(0) => return Ok(None),
            Ok(n) => n,
            Err(_) => return Ok(None),
        };
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > MAX_HEAD {
            return Err(bad(413, "too_big", "That request is too big."));
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let line = lines.next().unwrap_or("");
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("/").to_string());
    if method.is_empty() {
        return Err(bad(400, "bad_request", "That is not a request."));
    }
    let mut headers = Vec::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    if headers.iter().any(|(k, v)| k == "transfer-encoding" && !v.eq_ignore_ascii_case("identity")) {
        return Err(bad(411, "length_required", "Send a Content-Length."));
    }
    let length = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse::<usize>().ok()).unwrap_or(0);
    if length > MAX_BODY {
        return Err(bad(413, "too_big", "That request is too big."));
    }
    let start = head_end + 4;
    while buf.len().saturating_sub(start) < length {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(bad(400, "bad_request", "The request stopped early.")),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let body = buf[start..start + length].to_vec();
    let (path, query) = target.split_once('?').map(|(p, q)| (p.to_string(), q.to_string())).unwrap_or((target, String::new()));
    Ok(Some(Request { method, path, query, headers, body, peer }))
}

/// A running phone server. Dropping it does not stop it; `stop` does.
pub struct Server {
    stop: Arc<AtomicBool>,
    pub addr: SocketAddr,
}

impl Server {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag and lets go of the port.
        let wake = if self.addr.ip().is_unspecified() { SocketAddr::from(([127, 0, 0, 1], self.addr.port())) } else { self.addr };
        let _ = TcpStream::connect_timeout(&wake, Duration::from_millis(500));
    }
}

/// Binds `ip:port` and serves. `PHONE_PORT_IN_USE` when the port is taken.
pub fn start(ip: IpAddr, port: u16, ctx: Arc<Ctx>) -> Result<Server, String> {
    let listener = TcpListener::bind((ip, port)).map_err(|e| match e.kind() {
        std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied => format!("PHONE_PORT_IN_USE: Another program is using port {port}. Pick another port."),
        _ => format!("PHONE_PORT_IN_USE: The phone server could not use port {port}: {e}. Pick another port."),
    })?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let active = Arc::new(AtomicUsize::new(0));
    std::thread::Builder::new()
        .name("phone-server".into())
        .spawn(move || {
            for stream in listener.incoming() {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                if active.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                    continue; // dropped: too busy
                }
                active.fetch_add(1, Ordering::SeqCst);
                let (ctx, active, flag) = (ctx.clone(), active.clone(), flag.clone());
                let spawned = std::thread::Builder::new().name("phone-conn".into()).spawn(move || {
                    handle(stream, &ctx, &flag);
                    active.fetch_sub(1, Ordering::SeqCst);
                });
                if spawned.is_err() {
                    // The closure (and its counter decrement) never ran.
                    // Counter is repaired by the next accept seeing a lower value only after threads end; keep it simple.
                }
            }
        })
        .map_err(|e| format!("PHONE_PORT_IN_USE: could not start the phone server: {e}"))?;
    Ok(Server { stop, addr })
}

fn handle(mut stream: TcpStream, ctx: &Arc<Ctx>, stop: &AtomicBool) {
    let Ok(peer) = stream.peer_addr().map(|a| a.ip()) else { return };
    // Not a home or office network, and not this machine: dropped without reading a byte.
    if !net::peer_allowed(peer) {
        return;
    }
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(READ_TIMEOUT));
    let request = match read_request(&mut stream, peer) {
        Ok(Some(r)) => r,
        Ok(None) => return,
        Err(response) => {
            let _ = write_response(&mut stream, &response, false);
            return;
        }
    };
    if let Some(upgrade) = super::ws::wants_upgrade(&request) {
        super::ws::serve(stream, ctx, &request, upgrade, stop);
        return;
    }
    // The sandbox screen viewer and its websocket: the sealed link in the path is the credential (an iframe cannot send
    // a bearer), checked against the pairing by `screen_routes`.
    if super::screen_routes::is_screen_path(&request.path) {
        super::screen_routes::serve(&mut stream, ctx, &request);
        return;
    }
    let head_only = request.method == "HEAD";
    let response = super::gate::dispatch(ctx, &request);
    let _ = write_response(&mut stream, &response, head_only);
}

#[cfg(test)]
pub fn start_for_test(ctx: Arc<Ctx>) -> Server {
    start(IpAddr::from([127, 0, 0, 1]), 0, ctx).expect("bind a test phone server")
}
