//! Sign-in for tool servers by link (OAuth 2.1 with PKCE), done on this PC.
//!
//! - Discovery: the server's protected-resource page names its sign-in server; that server's metadata names the sign-in,
//!   token and (maybe) registration addresses. A server with neither page gets the standard `/authorize`, `/token` and
//!   `/register` on its own origin. Every address read from a page is checked like the server's own link (https, no private
//!   address unless Local was ticked for the server), so a server cannot send this app to an inside address.
//! - Registration: when the server offers dynamic client registration the app registers itself (a public client, no secret,
//!   PKCE S256). Otherwise the person must give a client id; there is no client id of ours and no hosted app.
//! - The browser: the app opens the sign-in address in the person's browser. The answer comes back to a one-time listener
//!   on `127.0.0.1` at a random port, and is accepted only with the `state` this app made.
//! - The tokens go to the credential store under `potato_mcp_oauth_<bot>_<server>` and nowhere else. The access token is
//!   refreshed shortly before it expires, and once more when the server says 401.
//!
//! Written for Potato from RFC 6749, 7636, 7591, 8414, 8707 and 9728; no code copied.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::bot_settings::McpServer;
use super::mcp::percent_encode;
use super::remote_mcp::{check_endpoint, get_following, post_following, RemoteError, Wire};
use crate::secrets;

type Res<T> = Result<T, RemoteError>;

/// How long the person has to finish in the browser.
pub const SIGN_IN_SECS: u64 = 300;
/// A token this close to its end is replaced before it is used.
const REFRESH_MARGIN_MS: i64 = 60_000;
const DEFAULT_LIFETIME_SECS: i64 = 3600;

pub fn secret_name(bot: &str, server: &str) -> String {
    format!("potato_mcp_oauth_{bot}_{server}")
}

/// What is kept for one signed-in server.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at_ms: i64,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub token_endpoint: String,
}

pub fn load(bot: &str, server: &str) -> Option<Stored> {
    secrets::backend().get(&secret_name(bot, server)).ok().flatten().and_then(|t| serde_json::from_str(&t).ok())
}

fn save(bot: &str, server: &str, stored: &Stored) -> Res<()> {
    let backend = secrets::backend();
    if !backend.available() {
        return Err(RemoteError::new("MCP_NO_STORE", "The credential store is not available, so the sign-in cannot be kept."));
    }
    let text = serde_json::to_string(stored).map_err(|_| RemoteError::new("MCP_NO_STORE", "The sign-in could not be saved."))?;
    backend.set(&secret_name(bot, server), &text).map_err(|_| RemoteError::new("MCP_NO_STORE", "The sign-in could not be saved in the credential store."))
}

pub fn forget(bot: &str, server: &str) {
    let _ = secrets::backend().delete(&secret_name(bot, server));
}

/// Every secret string of a signed-in server, for scrubbing answers and notes.
pub fn secret_values(bot: &str, server: &str) -> Vec<String> {
    let Some(s) = load(bot, server) else { return Vec::new() };
    let mut out = vec![s.access_token];
    out.extend(s.refresh_token);
    out.extend(s.client_secret);
    out
}

// ---- small helpers --------------------------------------------------------------------------------

fn url_b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random_text(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    url_b64(&buf)
}

/// `S256`: the challenge is the URL-safe base64 of the verifier's SHA-256.
pub fn challenge_of(verifier: &str) -> String {
    url_b64(&Sha256::digest(verifier.as_bytes()))
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v))).collect::<Vec<_>>().join("&")
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() && bytes.get(i + 1).zip(bytes.get(i + 2)).is_some_and(|(a, b)| a.is_ascii_hexdigit() && b.is_ascii_hexdigit()) => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("00");
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b'?'));
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn query_of(target: &str) -> Vec<(String, String)> {
    let query = target.split_once('?').map_or("", |(_, q)| q);
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

fn origin_and_path(url: &str) -> (String, String) {
    let after = url.find("://").map(|i| i + 3).unwrap_or(0);
    let end = url[after..].find(['/', '?']).map(|i| i + after).unwrap_or(url.len());
    let path = url[end..].split('?').next().unwrap_or("").trim_end_matches('/');
    (url[..end].to_string(), path.to_string())
}

fn json_headers() -> Vec<(String, String)> {
    vec![("Accept".to_string(), "application/json".to_string())]
}

/// A page read for discovery: `None` when the server has none (404 and the like).
fn get_json(wire: &dyn Wire, url: &str, local: bool) -> Res<Option<Value>> {
    let (resp, _) = get_following(wire, url, local, json_headers())?;
    if !(200..300).contains(&resp.status) {
        return Ok(None);
    }
    Ok(serde_json::from_str(resp.body.trim()).ok())
}

// ---- discovery ------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub authorization: String,
    pub token: String,
    pub registration: Option<String>,
}

fn field(v: &Value, key: &str, local: bool) -> Res<Option<String>> {
    match v.get(key).and_then(Value::as_str) {
        None => Ok(None),
        // An address on a page is checked as the server's own link is.
        Some(url) => check_endpoint(url, local).map(Some),
    }
}

/// Finds the sign-in addresses for a server link.
pub fn discover(wire: &dyn Wire, server_url: &str, local: bool) -> Res<Endpoints> {
    let (origin, path) = origin_and_path(server_url);
    // Which sign-in server the resource names (RFC 9728); none named means its own origin.
    let mut issuer = origin.clone();
    for candidate in [format!("{origin}/.well-known/oauth-protected-resource{path}"), format!("{origin}/.well-known/oauth-protected-resource")] {
        if let Some(meta) = get_json(wire, &candidate, local)? {
            if let Some(first) = meta.get("authorization_servers").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str) {
                issuer = check_endpoint(first, local)?.trim_end_matches('/').to_string();
            }
            break;
        }
    }
    // The sign-in server's own description (RFC 8414), or the OpenID one.
    let (i_origin, i_path) = origin_and_path(&issuer);
    for candidate in [
        format!("{i_origin}/.well-known/oauth-authorization-server{i_path}"),
        format!("{i_origin}/.well-known/openid-configuration{i_path}"),
        format!("{issuer}/.well-known/openid-configuration"),
    ] {
        let Some(meta) = get_json(wire, &candidate, local)? else { continue };
        if let Some(methods) = meta.get("code_challenge_methods_supported").and_then(Value::as_array) {
            if !methods.iter().any(|m| m == "S256") {
                return Err(RemoteError::new("MCP_OAUTH_UNSUPPORTED", "This server's sign-in does not support the safe PKCE method (S256), so it was not used."));
            }
        }
        let authorization = field(&meta, "authorization_endpoint", local)?;
        let token = field(&meta, "token_endpoint", local)?;
        if let (Some(authorization), Some(token)) = (authorization, token) {
            return Ok(Endpoints { authorization, token, registration: field(&meta, "registration_endpoint", local)? });
        }
    }
    // The standard places, when the server publishes no description.
    Ok(Endpoints {
        authorization: check_endpoint(&format!("{i_origin}/authorize"), local)?,
        token: check_endpoint(&format!("{i_origin}/token"), local)?,
        registration: Some(check_endpoint(&format!("{i_origin}/register"), local)?),
    })
}

/// Registers this app as a public client. `None`: the server cannot register clients.
fn register(wire: &dyn Wire, endpoint: &str, redirect: &str, local: bool) -> Res<Option<(String, Option<String>)>> {
    let body = json!({
        "client_name": "Potato Bot",
        "redirect_uris": [redirect],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    })
    .to_string();
    let headers = vec![("Content-Type".to_string(), "application/json".to_string()), ("Accept".to_string(), "application/json".to_string())];
    let (resp, _) = post_following(wire, endpoint, local, headers, &body, None)?;
    if !(200..300).contains(&resp.status) {
        return Ok(None);
    }
    let v: Value = serde_json::from_str(resp.body.trim()).map_err(|_| RemoteError::new("MCP_OAUTH_FAILED", "The server's registration answer could not be read."))?;
    let Some(id) = v.get("client_id").and_then(Value::as_str).filter(|s| !s.is_empty()) else { return Ok(None) };
    Ok(Some((id.to_string(), v.get("client_secret").and_then(Value::as_str).map(str::to_string))))
}

// ---- the browser step -----------------------------------------------------------------------------

fn reply_page(stream: &mut TcpStream, status: &str, text: &str) {
    let body = format!("<!doctype html><meta charset=utf-8><title>Potato Bot</title><body style=\"font:16px system-ui;margin:3em\"><p>{text}</p>");
    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}", body.len());
    let _ = stream.flush();
}

/// Waits for the browser to come back to the one-time address. Returns the `code`. A request with a different `state`, a
/// different path, or no code is answered and ignored, and the wait goes on until the time is up.
fn wait_for_code(listener: &TcpListener, state: &str, limit: Duration) -> Res<String> {
    listener.set_nonblocking(true).map_err(|_| RemoteError::new("MCP_OAUTH_FAILED", "The sign-in could not listen for the browser."))?;
    let started = Instant::now();
    loop {
        if started.elapsed() > limit {
            return Err(RemoteError::new("MCP_OAUTH_TIMEOUT", "The sign-in was not finished in time. Press Sign in to try again."));
        }
        let (mut stream, _) = match listener.accept() {
            Ok(pair) => pair,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
                continue;
            }
            Err(_) => return Err(RemoteError::new("MCP_OAUTH_FAILED", "The sign-in could not listen for the browser.")),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).unwrap_or(0) == 0 {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let path = target.split('?').next().unwrap_or("");
        if method != "GET" || path != "/callback" {
            reply_page(&mut stream, "404 Not Found", "Nothing here.");
            continue;
        }
        let query = query_of(target);
        let get = |k: &str| query.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        if get("state") != Some(state) {
            reply_page(&mut stream, "400 Bad Request", "This sign-in answer is not for this request, so it was ignored.");
            continue;
        }
        if let Some(error) = get("error") {
            reply_page(&mut stream, "200 OK", "The sign-in was cancelled. You can close this tab.");
            let why: String = error.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(40).collect();
            return Err(RemoteError::new("MCP_OAUTH_DENIED", format!("The sign-in was not allowed ({why}).")));
        }
        let Some(code) = get("code").filter(|c| !c.is_empty() && c.len() < 4096) else {
            reply_page(&mut stream, "400 Bad Request", "This sign-in answer had no code.");
            continue;
        };
        reply_page(&mut stream, "200 OK", "You are signed in. You can close this tab and go back to Potato Bot.");
        return Ok(code.to_string());
    }
}

// ---- tokens ---------------------------------------------------------------------------------------

fn parse_token_answer(body: &str, now_ms: i64) -> Res<(String, Option<String>, i64)> {
    let v: Value = serde_json::from_str(body.trim()).map_err(|_| RemoteError::new("MCP_OAUTH_FAILED", "The server's token answer could not be read."))?;
    let Some(access) = v.get("access_token").and_then(Value::as_str).filter(|t| !t.is_empty() && t.len() < 8192) else {
        return Err(RemoteError::new("MCP_OAUTH_FAILED", "The server did not give a token."));
    };
    let secs = v.get("expires_in").and_then(Value::as_i64).filter(|s| *s > 0).unwrap_or(DEFAULT_LIFETIME_SECS);
    Ok((access.to_string(), v.get("refresh_token").and_then(Value::as_str).map(str::to_string), now_ms + secs * 1000))
}

fn token_post(wire: &dyn Wire, endpoint: &str, local: bool, body: &str) -> Res<super::remote_mcp::Resp> {
    let headers = vec![("Content-Type".to_string(), "application/x-www-form-urlencoded".to_string()), ("Accept".to_string(), "application/json".to_string())];
    post_following(wire, endpoint, local, headers, body, None).map(|(r, _)| r)
}

/// The whole sign-in. `open` shows the address in the person's browser. Tokens are kept in the credential store; the
/// caller marks the server signed in. `client_id`: given by the person when the server cannot register clients.
#[allow(clippy::too_many_arguments)]
pub fn sign_in(
    wire: &dyn Wire,
    bot: &str,
    server: &McpServer,
    client_id: Option<&str>,
    now_ms: i64,
    limit: Duration,
    open: &dyn Fn(&str) -> Result<(), String>,
) -> Res<()> {
    if server.url.is_empty() {
        return Err(RemoteError::new("MCP_OAUTH_UNSUPPORTED", "Only a tool server by link can be signed in to."));
    }
    let local = server.local;
    let endpoints = discover(wire, &server.url, local)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| RemoteError::new("MCP_OAUTH_FAILED", "The sign-in could not listen for the browser."))?;
    let port = listener.local_addr().map_err(|_| RemoteError::new("MCP_OAUTH_FAILED", "The sign-in could not listen for the browser."))?.port();
    let redirect = format!("http://127.0.0.1:{port}/callback");

    let (id, secret) = match client_id.map(str::trim).filter(|c| !c.is_empty()) {
        Some(given) => {
            if given.len() > 512 || given.chars().any(|c| c.is_control() || c.is_whitespace()) {
                return Err(RemoteError::new("MCP_OAUTH_NEEDS_CLIENT_ID", "That client id is not valid."));
            }
            (given.to_string(), None)
        }
        None => match &endpoints.registration {
            Some(reg) => register(wire, reg, &redirect, local)?.ok_or_else(|| needs_client_id())?,
            None => return Err(needs_client_id()),
        },
    };

    let verifier = random_text(48);
    let state = random_text(24);
    let challenge = challenge_of(&verifier);
    let sep = if endpoints.authorization.contains('?') { '&' } else { '?' };
    let auth_url = format!(
        "{}{sep}{}",
        endpoints.authorization,
        form(&[
            ("response_type", "code"),
            ("client_id", &id),
            ("redirect_uri", &redirect),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
            ("resource", &server.url),
        ])
    );
    open(&auth_url).map_err(|_| RemoteError::new("MCP_OAUTH_FAILED", "The browser could not be opened. Open Potato Bot's sign-in again."))?;
    let code = wait_for_code(&listener, &state, limit)?;
    drop(listener);

    let mut pairs = vec![
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("client_id", id.as_str()),
        ("code_verifier", verifier.as_str()),
        ("resource", server.url.as_str()),
    ];
    if let Some(s) = &secret {
        pairs.push(("client_secret", s));
    }
    let resp = token_post(wire, &endpoints.token, local, &form(&pairs))?;
    if !(200..300).contains(&resp.status) {
        return Err(RemoteError::new("MCP_OAUTH_FAILED", "The server did not accept the sign-in."));
    }
    let (access, refresh, expires) = parse_token_answer(&resp.body, now_ms)?;
    save(bot, &server.name, &Stored { access_token: access, refresh_token: refresh, expires_at_ms: expires, client_id: id, client_secret: secret, token_endpoint: endpoints.token })
}

fn needs_client_id() -> RemoteError {
    RemoteError::new("MCP_OAUTH_NEEDS_CLIENT_ID", "This server does not sign apps in by itself. It needs a client id from the service: create one there (with the redirect address http://127.0.0.1) and paste it here.")
}

/// The access token to use now, refreshed first when it is about to end (or when `force`: the server just said 401).
/// `Ok(None)`: the server was never signed in to.
pub fn access_token(wire: &dyn Wire, bot: &str, server: &McpServer, now_ms: i64, force: bool) -> Res<Option<String>> {
    let Some(mut stored) = load(bot, &server.name) else { return Ok(None) };
    if !force && stored.expires_at_ms - now_ms > REFRESH_MARGIN_MS {
        return Ok(Some(stored.access_token));
    }
    let Some(refresh) = stored.refresh_token.clone() else {
        // No way to renew: use it as it is until the server refuses, then ask for a new sign-in.
        return if force { Err(sign_in_again()) } else { Ok(Some(stored.access_token)) };
    };
    let mut pairs = vec![("grant_type", "refresh_token"), ("refresh_token", refresh.as_str()), ("client_id", stored.client_id.as_str()), ("resource", server.url.as_str())];
    if let Some(s) = &stored.client_secret {
        pairs.push(("client_secret", s));
    }
    let resp = token_post(wire, &stored.token_endpoint, server.local, &form(&pairs))?;
    if !(200..300).contains(&resp.status) {
        return Err(sign_in_again());
    }
    let (access, new_refresh, expires) = parse_token_answer(&resp.body, now_ms).map_err(|_| sign_in_again())?;
    stored.access_token = access.clone();
    if new_refresh.is_some() {
        stored.refresh_token = new_refresh;
    }
    stored.expires_at_ms = expires;
    save(bot, &server.name, &stored)?;
    Ok(Some(access))
}

fn sign_in_again() -> RemoteError {
    RemoteError::new("MCP_NEEDS_SIGN_IN", "The sign-in ended and could not be renewed. Press Sign in again in Settings > Connectors.")
}
