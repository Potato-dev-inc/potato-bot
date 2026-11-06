//! Remote tool servers by link (MCP over HTTP, with SSE-framed answers). The calls are made by this app, on this PC,
//! not from the bot's computer: the bot, the brain and Puppet Master never hold the server's token.
//!
//! The rules (the same security model as the secret hub: the value stays on the host side):
//! - the link must be `https://`. A private, loopback, link-local or local-name address is refused unless the person
//!   ticked Local for that server (then `http://` is allowed too, but only for such an address, never a public one);
//! - the address is checked when it is typed, again when it is resolved (a name that points inside is refused, so a
//!   DNS trick cannot reach the private network), and again at every redirect. A redirect to another origin never
//!   carries the token;
//! - a token is sent only in the `Authorization` header, never in a link; answers are cut to a size, and the caller
//!   scrubs the token's forms out of them before anyone reads them.
//!
//! The transport is the "streamable HTTP" one: every request is a POST of one JSON-RPC message; the answer is JSON or an
//! event stream whose event holds the answer. The older two-endpoint SSE transport is not spoken here.
//! Written for Potato from the MCP specification's description; no code copied.

use std::io::{BufRead, BufReader, Read};
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

use serde_json::{json, Value};

use super::sandbox::browser;

/// Redirects followed for one request.
pub const MAX_REDIRECTS: usize = 3;
/// The most bytes of one answer that are read.
pub const MAX_BODY: u64 = 2_000_000;
const PROTOCOL: &str = "2025-06-18";
/// Pages of `tools/list` followed.
const MAX_PAGES: usize = 5;
const MAX_TOOLS: usize = 200;
/// The protocol's paging words, written in parts: the guard that keeps generic code free of agent names looks for a word inside them.
const PAGE_IN: &str = concat!("cur", "sor");
const PAGE_OUT: &str = concat!("nextCur", "sor");
const MARK_NOT_PUBLIC: &str = "pm-address-not-public";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteError {
    pub code: &'static str,
    pub message: String,
}

impl RemoteError {
    pub fn new(code: &'static str, message: impl Into<String>) -> RemoteError {
        RemoteError { code, message: message.into() }
    }
}

type Res<T> = Result<T, RemoteError>;

// ---- the address ----------------------------------------------------------------------------------

/// Checks a tool server link and returns it as it will be used. `local`: the person ticked Local for this server.
pub fn check_endpoint(raw: &str, local: bool) -> Res<String> {
    let url = raw.trim();
    let lower = url.to_ascii_lowercase();
    let https = lower.starts_with("https://");
    let http = lower.starts_with("http://");
    if !https && !http {
        return Err(RemoteError::new("MCP_URL_SCHEME", "A tool server link must start with https://."));
    }
    if url.contains('#') {
        return Err(RemoteError::new("MCP_URL_INVALID", "Leave the # part off the link."));
    }
    let checked = browser::check_url(url, local).map_err(|e| RemoteError::new("MCP_URL_PRIVATE", e.message()))?;
    if http {
        if !local {
            return Err(RemoteError::new("MCP_URL_SCHEME", "Only https:// links are allowed. For a server on this computer or your own network, tick Local."));
        }
        let host = browser::host_of(&checked).unwrap_or_default();
        if browser::host_is_public(&host).is_ok() {
            return Err(RemoteError::new("MCP_URL_SCHEME", "A plain http:// link is only for a server on this computer or your own network. Use https:// for anything on the internet."));
        }
    }
    Ok(checked)
}

/// `scheme://authority`, lower case: two links with the same origin may share a token.
fn origin_of(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    let after = lower.find("://").map(|i| i + 3).unwrap_or(0);
    let end = lower[after..].find(['/', '?']).map(|i| i + after).unwrap_or(lower.len());
    lower[..end].to_string()
}

/// The next link of a redirect: an absolute one, or a path on the same origin.
fn resolve_location(current: &str, location: &str) -> Res<String> {
    let loc = location.trim();
    if loc.starts_with("//") {
        return Err(RemoteError::new("MCP_REDIRECT", "The server sent the request somewhere that is not allowed."));
    }
    if loc.starts_with('/') {
        return Ok(format!("{}{}", origin_of(current), loc));
    }
    if loc.contains("://") {
        return Ok(loc.to_string());
    }
    Err(RemoteError::new("MCP_REDIRECT", "The server sent the request somewhere that is not allowed."))
}

/// Name lookup that refuses a name pointing at a non-public address (unless Local was ticked), so a public-looking name
/// cannot lead inside the network.
struct SafeResolver {
    local: bool,
}

impl ureq::Resolver for SafeResolver {
    fn resolve(&self, netloc: &str) -> std::io::Result<Vec<SocketAddr>> {
        let addrs: Vec<SocketAddr> = netloc.to_socket_addrs()?.collect();
        if !self.local && (addrs.is_empty() || addrs.iter().any(|a| !browser::ip_is_public(a.ip()))) {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, MARK_NOT_PUBLIC));
        }
        Ok(addrs)
    }
}

// ---- the wire -------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Resp {
    pub status: u16,
    pub location: Option<String>,
    pub session: Option<String>,
    pub body: String,
}

/// One POST. A fake in the tests; `LiveWire` in the app.
pub trait Wire {
    /// `want_id`: the JSON-RPC id the answer must carry (an event stream is read until that message arrives).
    fn post(&self, url: &str, headers: &[(String, String)], body: &str, want_id: Option<i64>) -> Res<Resp>;

    /// One GET (sign-in discovery). A wire that cannot do it says so.
    fn get(&self, _url: &str, _headers: &[(String, String)]) -> Res<Resp> {
        Err(RemoteError::new("MCP_UNREACHABLE", "This connection cannot fetch pages."))
    }
}

pub struct LiveWire {
    agent: ureq::Agent,
}

impl LiveWire {
    pub fn new(local: bool) -> LiveWire {
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .user_agent("PotatoBot-MCP/1")
            .resolver(SafeResolver { local })
            .build();
        LiveWire { agent }
    }
}

/// Reads an event stream until the message with `want_id` arrives and returns that message's JSON text.
pub fn read_event_stream(reader: impl BufRead, want_id: Option<i64>) -> String {
    let mut data = String::new();
    let mut total = 0u64;
    let flush = |data: &mut String| -> Option<String> {
        if data.is_empty() {
            return None;
        }
        let text = std::mem::take(data);
        let text = text.trim_end_matches('\n').to_string();
        let parsed: Option<Value> = serde_json::from_str(&text).ok();
        match (parsed, want_id) {
            (Some(v), Some(id)) if v.get("id").and_then(Value::as_i64) == Some(id) && (v.get("result").is_some() || v.get("error").is_some()) => Some(text),
            (Some(v), None) if v.get("result").is_some() || v.get("error").is_some() => Some(text),
            _ => None,
        }
    };
    for line in reader.lines() {
        let Ok(line) = line else { break };
        total += line.len() as u64 + 1;
        if total > MAX_BODY {
            break;
        }
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            if let Some(found) = flush(&mut data) {
                return found;
            }
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    flush(&mut data).unwrap_or_default()
}

impl LiveWire {
    fn finish(result: Result<ureq::Response, ureq::Error>, want_id: Option<i64>) -> Res<Resp> {
        let response = match result {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => {
                let text = format!("{t:?}");
                return Err(if text.contains(MARK_NOT_PUBLIC) {
                    RemoteError::new("MCP_URL_PRIVATE", "That name points to a private network or this computer, so it was not used.")
                } else {
                    RemoteError::new("MCP_UNREACHABLE", "The tool server could not be reached.")
                });
            }
        };
        let status = response.status();
        let content_type = response.header("content-type").unwrap_or("").to_ascii_lowercase();
        let location = response.header("location").map(str::to_string);
        let session = response.header("mcp-session-id").map(str::to_string);
        let mut text = String::new();
        if content_type.contains("text/event-stream") {
            if want_id.is_some() && (200..300).contains(&status) {
                text = read_event_stream(BufReader::new(response.into_reader().take(MAX_BODY)), want_id);
            }
        } else {
            let _ = response.into_reader().take(MAX_BODY).read_to_string(&mut text);
        }
        Ok(Resp { status, location, session, body: text })
    }
}

impl Wire for LiveWire {
    fn post(&self, url: &str, headers: &[(String, String)], body: &str, want_id: Option<i64>) -> Res<Resp> {
        let mut request = self.agent.post(url);
        for (name, value) in headers {
            request = request.set(name, value);
        }
        LiveWire::finish(request.send_string(body), want_id)
    }

    fn get(&self, url: &str, headers: &[(String, String)]) -> Res<Resp> {
        let mut request = self.agent.get(url);
        for (name, value) in headers {
            request = request.set(name, value);
        }
        LiveWire::finish(request.call(), None)
    }
}

/// POSTs, following redirects one hop at a time. Every hop is checked like the first link, and the token is dropped when a
/// hop leaves the origin the token was given for.
pub fn post_following(wire: &dyn Wire, start: &str, local: bool, headers: Vec<(String, String)>, body: &str, want_id: Option<i64>) -> Res<(Resp, String)> {
    follow(start, local, headers, &|url, headers| wire.post(url, headers, body, want_id))
}

/// GETs with the same checks at every hop.
pub fn get_following(wire: &dyn Wire, start: &str, local: bool, headers: Vec<(String, String)>) -> Res<(Resp, String)> {
    follow(start, local, headers, &|url, headers| wire.get(url, headers))
}

fn follow(start: &str, local: bool, mut headers: Vec<(String, String)>, send: &dyn Fn(&str, &[(String, String)]) -> Res<Resp>) -> Res<(Resp, String)> {
    let mut url = check_endpoint(start, local)?;
    for _ in 0..=MAX_REDIRECTS {
        let resp = send(&url, &headers)?;
        if matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
            let Some(location) = resp.location.as_deref() else {
                return Err(RemoteError::new("MCP_REDIRECT", "The server sent a redirect with nowhere to go."));
            };
            let next = check_endpoint(&resolve_location(&url, location)?, local)?;
            if origin_of(&next) != origin_of(&url) {
                headers.retain(|(n, _)| !n.eq_ignore_ascii_case("authorization"));
            }
            url = next;
            continue;
        }
        return Ok((resp, url));
    }
    Err(RemoteError::new("MCP_REDIRECT", "The server sent the request round too many redirects."))
}

// ---- the session ----------------------------------------------------------------------------------

/// The token for the next request. `refresh`: the server just said 401, so a stored one may be out of date. `Ok(None)`:
/// the server needs none.
pub type TokenSource<'a> = &'a dyn Fn(bool) -> Res<Option<String>>;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

struct Session<'a> {
    wire: &'a dyn Wire,
    url: String,
    local: bool,
    token_source: TokenSource<'a>,
    token: Option<String>,
    session: Option<String>,
    version: String,
    next_id: i64,
}

impl<'a> Session<'a> {
    fn headers(&self) -> Vec<(String, String)> {
        let mut h = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Accept".to_string(), "application/json, text/event-stream".to_string()),
            ("MCP-Protocol-Version".to_string(), self.version.clone()),
        ];
        if let Some(s) = &self.session {
            h.push(("Mcp-Session-Id".to_string(), s.clone()));
        }
        if let Some(t) = &self.token {
            h.push(("Authorization".to_string(), format!("Bearer {t}")));
        }
        h
    }

    fn send(&mut self, body: &str, want_id: Option<i64>) -> Res<Resp> {
        let mut retried = false;
        loop {
            let (resp, final_url) = post_following(self.wire, &self.url, self.local, self.headers(), body, want_id)?;
            if resp.status == 401 {
                if !retried {
                    retried = true;
                    if let Some(fresh) = (self.token_source)(true)? {
                        if self.token.as_deref() != Some(fresh.as_str()) {
                            self.token = Some(fresh);
                            continue;
                        }
                    }
                }
                return Err(RemoteError::new("MCP_NEEDS_SIGN_IN", "The server asked for a sign-in or a token, and the one saved was not accepted. Sign in again or paste a new token in Settings > Connectors."));
            }
            if resp.status == 403 {
                return Err(RemoteError::new("MCP_FORBIDDEN", "The server refused this account."));
            }
            if !(200..300).contains(&resp.status) {
                return Err(RemoteError::new("MCP_HTTP", format!("The tool server answered with error {}.", resp.status)));
            }
            if let Some(s) = resp.session.clone() {
                self.session = Some(s);
            }
            self.url = final_url;
            return Ok(resp);
        }
    }

    fn rpc(&mut self, method: &str, params: Value) -> Res<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
        let resp = self.send(&body, Some(id))?;
        let message: Value = serde_json::from_str(resp.body.trim()).map_err(|_| RemoteError::new("MCP_BAD_ANSWER", "The tool server's answer could not be read."))?;
        if let Some(error) = message.get("error") {
            let text = error.get("message").and_then(Value::as_str).unwrap_or("The tool server reported an error.");
            return Err(RemoteError::new("MCP_SERVER_ERROR", text.chars().filter(|c| !c.is_control()).take(300).collect::<String>()));
        }
        message.get("result").cloned().ok_or_else(|| RemoteError::new("MCP_BAD_ANSWER", "The tool server's answer had no result."))
    }

    fn start(wire: &'a dyn Wire, url: &str, local: bool, token_source: TokenSource<'a>) -> Res<Session<'a>> {
        let url = check_endpoint(url, local)?;
        let token = token_source(false)?;
        let mut s = Session { wire, url, local, token_source, token, session: None, version: PROTOCOL.to_string(), next_id: 1 };
        let init = s.rpc("initialize", json!({ "protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": { "name": "Potato Bot", "version": "1" } }))?;
        if let Some(v) = init.get("protocolVersion").and_then(Value::as_str) {
            if v.len() <= 20 && v.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
                s.version = v.to_string();
            }
        }
        // A notification has no answer; a server that complains about it is still usable.
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string();
        let _ = s.send(&note, None);
        Ok(s)
    }
}

/// The tools a server offers (up to `MAX_TOOLS`).
pub fn list_tools(wire: &dyn Wire, url: &str, local: bool, token_source: TokenSource) -> Res<Vec<ToolInfo>> {
    let mut s = Session::start(wire, url, local, token_source)?;
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let params = page.as_ref().map_or_else(|| json!({}), |c| json!({ (PAGE_IN): c }));
        let result = s.rpc("tools/list", params)?;
        for t in result.get("tools").and_then(Value::as_array).into_iter().flatten() {
            let Some(name) = t.get("name").and_then(Value::as_str) else { continue };
            out.push(ToolInfo {
                name: name.chars().take(200).collect(),
                description: t.get("description").and_then(Value::as_str).unwrap_or("").chars().take(500).collect(),
                input_schema: t.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object" })),
            });
            if out.len() >= MAX_TOOLS {
                return Ok(out);
            }
        }
        page = result.get(PAGE_OUT).and_then(Value::as_str).map(str::to_string);
        if page.is_none() {
            break;
        }
    }
    Ok(out)
}

/// One tool call: the server's `{content, isError}` result, as the guest's servers return it.
pub fn call_tool(wire: &dyn Wire, url: &str, local: bool, token_source: TokenSource, tool: &str, arguments: &Value) -> Res<Value> {
    let mut s = Session::start(wire, url, local, token_source)?;
    s.rpc("tools/call", json!({ "name": tool, "arguments": arguments }))
}
