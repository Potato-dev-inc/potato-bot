//! Spaces on the phone: read and send, nothing else. `GET /api/spaces`, `GET /api/spaces/{sid}/channels/{cid}/messages`,
//! `POST .../read`, `POST .../send`.
//!
//! The phone is this PC's own person, exactly like the desktop composer: a send goes through `SpacesService::send_text`,
//! the same call the desktop's `space_send` command makes (same signing, same size and rate limits, same hand-off to this
//! PC's bots, whose rules live below that call and are not touched here). Nothing here invites, removes, blocks, leaves,
//! forgets, creates a channel or touches a bot. Behind Labs `spaces_enabled`: off, every route is `409 spaces_off`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::gate::Ctx;
use super::http::{Request, Response};
use crate::spaces::service::SpacesService;

const MAX_TEXT: usize = 8000;
const DEFAULT_PAGE: usize = 50;
const MAX_PAGE: usize = 100;

fn err(status: u16, code: &str, message: &str) -> Response {
    Response::error(status, code, message)
}

/// The service when Spaces is on (`None`: on, but not started), or the response to give instead.
fn spaces_on(ctx: &Ctx) -> Result<Option<Arc<SpacesService>>, Response> {
    if !ctx.env.spaces_on() {
        return Err(err(409, "spaces_off", "Spaces is off on your PC. Turn it on in Settings, under Labs."));
    }
    Ok(ctx.env.spaces())
}

fn running(ctx: &Ctx) -> Result<Arc<SpacesService>, Response> {
    spaces_on(ctx)?.ok_or_else(|| err(503, "not_ready", "Spaces is not ready on your PC yet. Try again in a moment."))
}

/// A service message as the phone API says it. "Not known" is a 404; the rest is what the person should read.
fn service_err(message: &str) -> Response {
    if message.ends_with("is not known.") || message == "That message is gone." {
        err(404, "not_found", message)
    } else if message.starts_with("You're sending messages too fast") || message.starts_with("You've sent a lot of messages") {
        err(429, "rate_limited", message)
    } else if message.contains("too long") {
        err(400, "too_long", message)
    } else if message.starts_with("Write something") {
        err(400, "empty", message)
    } else if message.contains("closed") || message.contains("archived") || message.contains("any more") {
        err(403, "read_only", message)
    } else {
        err(400, "bad_request", message)
    }
}

pub fn route(ctx: &Ctx, req: &Request, seg: &[&str]) -> Response {
    match (req.method.as_str(), seg) {
        ("GET", ["spaces"]) => list(ctx),
        ("GET", ["spaces", sid, "channels", cid, "messages"]) => messages(ctx, req, sid, cid),
        ("POST", ["spaces", sid, "channels", cid, "read"]) => read(ctx, sid, cid),
        ("POST", ["spaces", sid, "channels", cid, "send"]) => send(ctx, req, sid, cid),
        _ => err(404, "not_found", "There is no such route."),
    }
}

/// Whether this PC's person can write in the channel: the space is theirs to talk in and the channel is open.
/// `None`: no such channel.
fn can_write(detail: &Value, cid: &str) -> Option<bool> {
    let ch = detail["channels"].as_array()?.iter().find(|c| c["id"] == cid)?;
    Some(detail["state"] == "active" && ch["archived"] != true)
}

fn list(ctx: &Ctx) -> Response {
    let svc = match spaces_on(ctx) {
        Ok(Some(s)) => s,
        Ok(None) => return Response::json(200, &json!({ "online": false, "spaces": [] })),
        Err(r) => return r,
    };
    let view = svc.view();
    let mut spaces = Vec::new();
    for sp in view["spaces"].as_array().into_iter().flatten() {
        let Some(sid) = sp["id"].as_str() else { continue };
        let Ok(detail) = svc.detail(sid) else { continue };
        let channels: Vec<Value> = detail["channels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["archived"] != true)
            .map(|c| {
                let cid = c["id"].as_str().unwrap_or("");
                json!({ "id": cid, "name": c["name"], "unread": c["unread"], "canWrite": can_write(&detail, cid).unwrap_or(false) })
            })
            .collect();
        spaces.push(json!({ "id": sid, "name": sp["name"], "unread": sp["unread"], "channels": channels }));
    }
    Response::json(200, &json!({ "online": view["online"].as_bool().unwrap_or(false), "spaces": spaces }))
}

fn file_list(m: &Value) -> Vec<Value> {
    match m["file"].as_object() {
        Some(f) => vec![json!({ "name": f.get("name"), "size": f.get("size") })],
        None => Vec::new(),
    }
}

fn message_out(m: &Value) -> Value {
    let has_file = m["file"].is_object();
    let bot = m["isFish"].as_bool().unwrap_or(false);
    json!({
        // A bot line is never "mine" (the service already says so; this keeps it true if that ever slips).
        "id": m["id"], "authorName": m["authorName"], "mine": !bot && m["isMine"].as_bool().unwrap_or(false),
        "bot": bot, "ownerName": m["ownerName"], "ownedByMe": m["ownedByMe"].as_bool().unwrap_or(false),
        // A file message's text upstream is just the file's name; the phone gets it in `files`.
        "text": if has_file { json!("") } else { m["text"].clone() },
        "atMs": m["createdAtMs"], "files": file_list(m),
        // The service stores a message before it says so, so everything it shows is sent.
        "state": "sent",
    })
}

fn messages(ctx: &Ctx, req: &Request, sid: &str, cid: &str) -> Response {
    let svc = match running(ctx) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let limit = req.query_param("limit").and_then(|v| v.parse::<usize>().ok()).unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let before = req.query_param("before").filter(|b| !b.is_empty());
    let detail = match svc.detail(sid) {
        Ok(d) => d,
        Err(e) => return service_err(&e),
    };
    let Some(writable) = can_write(&detail, cid) else { return err(404, "not_found", "That channel is not known.") };
    // One more than asked for tells whether there is an older page.
    let page = match svc.messages(sid, cid, before.as_deref(), (limit + 1) as u32) {
        Ok(Value::Array(rows)) => rows,
        Ok(_) => Vec::new(),
        Err(e) => return service_err(&e),
    };
    let has_more = page.len() > limit;
    let skip = page.len().saturating_sub(limit);
    let out: Vec<Value> = page
        .iter()
        .skip(skip)
        .filter(|m| m["deleted"].is_null() && m["blocked"] != true)
        .map(message_out)
        .collect();
    Response::json(200, &json!({ "messages": out, "canWrite": writable, "hasMore": has_more }))
}

fn read(ctx: &Ctx, sid: &str, cid: &str) -> Response {
    let svc = match running(ctx) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match svc.mark_read(sid, cid) {
        Ok(()) => Response::json(200, &json!({ "ok": true })),
        Err(e) => service_err(&e),
    }
}

/// Runs the service send (`async`, but ready at once) on the app's runtime and waits: the phone server's threads are plain
/// threads.
fn run_send(svc: Arc<SpacesService>, sid: String, cid: String, text: String) -> Result<Value, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    tauri::async_runtime::spawn(async move {
        let _ = tx.send(svc.send_text(&sid, &cid, &text, None).await);
    });
    rx.recv_timeout(Duration::from_secs(20)).unwrap_or_else(|_| Err("That took too long. Try again.".to_string()))
}

fn send(ctx: &Ctx, req: &Request, sid: &str, cid: &str) -> Response {
    let svc = match running(ctx) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let v: Value = match serde_json::from_slice(&req.body) {
        Ok(v) => v,
        Err(_) => return err(400, "bad_request", "That is not the JSON this server expects."),
    };
    let text = v.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if text.is_empty() {
        return err(400, "empty", "Write something first.");
    }
    if text.chars().count() > MAX_TEXT {
        return err(400, "too_long", "That message is too long.");
    }
    // The same rule the desktop composer shows: a closed channel or a space you left cannot be written in.
    let detail = match svc.detail(sid) {
        Ok(d) => d,
        Err(e) => return service_err(&e),
    };
    match can_write(&detail, cid) {
        None => return err(404, "not_found", "That channel is not known."),
        Some(false) => return err(403, "read_only", "You can only read this channel."),
        Some(true) => {}
    }
    match run_send(svc, sid.to_string(), cid.to_string(), text) {
        Ok(m) => {
            ctx.changes.bump();
            Response::json(200, &json!({ "ok": true, "id": m["id"] }))
        }
        Err(e) => service_err(&e),
    }
}
