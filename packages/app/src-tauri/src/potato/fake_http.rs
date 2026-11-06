//! A tiny HTTP server on 127.0.0.1 for tests (a fake tool server, a fake sign-in server). One request per connection.
//! Every request it received is kept, so a test can look at the headers that really crossed the wire.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

use parking_lot::Mutex;

#[derive(Clone, Debug, Default)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Req {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Reply {
        Reply { status, headers: vec![("Content-Type".into(), "application/json".into())], body: body.to_string() }
    }
    pub fn sse(body: &str) -> Reply {
        Reply { status: 200, headers: vec![("Content-Type".into(), "text/event-stream".into())], body: body.to_string() }
    }
    pub fn redirect(status: u16, to: &str) -> Reply {
        Reply { status, headers: vec![("Location".into(), to.to_string())], body: String::new() }
    }
    pub fn with(mut self, name: &str, value: &str) -> Reply {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

pub struct Server {
    pub port: u16,
    pub seen: Arc<Mutex<Vec<Req>>>,
}

impl Server {
    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
    pub fn requests(&self) -> Vec<Req> {
        self.seen.lock().clone()
    }
}

/// Starts the server on a thread that lives for the rest of the test process.
pub fn serve(handler: impl Fn(&Req) -> Reply + Send + Sync + 'static) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Arc<Mutex<Vec<Req>>> = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let handler = Arc::new(handler);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let log = log.clone();
            let handler = handler.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                let mut headers = Vec::new();
                let mut length = 0usize;
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                        break;
                    }
                    if let Some((n, v)) = h.split_once(':') {
                        if n.eq_ignore_ascii_case("content-length") {
                            length = v.trim().parse().unwrap_or(0);
                        }
                        headers.push((n.trim().to_string(), v.trim().to_string()));
                    }
                }
                let mut body = vec![0u8; length];
                let _ = reader.read_exact(&mut body);
                let req = Req { method, path, headers, body: String::from_utf8_lossy(&body).to_string() };
                log.lock().push(req.clone());
                let reply = handler(&req);
                let mut out = format!("HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n", reply.status, reply.body.len());
                for (n, v) in &reply.headers {
                    out.push_str(&format!("{n}: {v}\r\n"));
                }
                out.push_str("\r\n");
                out.push_str(&reply.body);
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            });
        }
    });
    Server { port, seen }
}
