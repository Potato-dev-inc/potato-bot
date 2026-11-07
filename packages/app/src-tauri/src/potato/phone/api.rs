//! The phone API, version 1: every route a signed-in phone may call. Plain JSON, camelCase, errors as
//! `{ "error": "code", "message": "plain sentence" }`. Documented in `docs/PHONE-API.md`.
//!
//! No page logic lives in the server: it hands out what the desktop shows (cut down to what a phone needs) and
//! passes a phone's actions to the same engine calls the desktop uses. A secret never passes through here:
//! sign-in cards can only be dismissed, never answered with a password; "Always allow" is not offered.

use std::time::Duration;

use serde_json::{json, Value};

use super::gate::{Caller, Ctx};
use super::http::Request;
use super::http::Response;
use crate::mobile_pairing::Cap;
use crate::potato::cards::{Card, Kind};
use crate::potato::rooms::store as rstore;
use crate::potato::service::{BotView, BrainState, Service};
use crate::potato::store::{self, Message, StoreError};

const MAX_TEXT: usize = 8000;

/// The ability a route needs. `None`: any signed-in phone. Unknown routes need `control` (fail closed).
pub fn need(method: &str, seg: &[&str]) -> Option<Cap> {
    match (method, seg) {
        ("GET", ["me"]) | ("DELETE", ["me"]) | ("GET", ["changes"]) => None,
        ("POST", ["push", "presence"]) | ("DELETE", ["push", "subscribe"]) => None,
        ("GET", ["bots"]) | ("GET", ["bots", _]) | ("GET", ["rooms", _]) | ("GET", ["push", "key"]) | ("POST", ["push", "subscribe"]) => Some(Cap::Watch),
        ("POST", ["bots", _, "read"]) => Some(Cap::Watch),
        ("POST", ["bots", _, "send"]) | ("POST", ["rooms", _, "send"]) => Some(Cap::Type),
        ("POST", ["bots", _, "cards", _, "answer"]) => Some(Cap::Answer),
        _ => Some(Cap::Control),
    }
}

fn err(status: u16, code: &str, message: &str) -> Response {
    Response::error(status, code, message)
}

fn store_err(e: StoreError) -> Response {
    let status = if e.code.contains("NOT_FOUND") { 404 } else { 400 };
    Response::json(status, &json!({ "error": e.code.to_ascii_lowercase(), "message": e.message }))
}

fn body(req: &Request) -> Result<Value, Response> {
    if req.body.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(&req.body).map_err(|_| err(400, "bad_request", "That is not the JSON this server expects."))
}

fn service(ctx: &Ctx) -> Result<std::sync::Arc<Service>, Response> {
    ctx.env.service().ok_or_else(|| err(503, "not_ready", "Potato Bot is not ready yet. Try again in a moment."))
}

// ---- shapes ---------------------------------------------------------------------------------------------------

fn state_of(b: &BotView) -> (&'static str, &'static str) {
    if b.open_cards > 0 {
        return ("needs_you", "Needs you");
    }
    let running = b.work.as_ref().is_some_and(|w| w.ended_ms.is_none() && !w.failed);
    if b.typing || running || matches!(b.brain_state, BrainState::Starting) {
        return ("working", "Working");
    }
    match b.brain_state {
        BrainState::Off => ("offline", "Off"),
        BrainState::NeedsKey => ("problem", "Needs a key"),
        BrainState::NeedsNode => ("problem", "Needs Node.js"),
        BrainState::MissingProgram => ("problem", "Broken install"),
        BrainState::DailyLimit => ("problem", "Daily limit reached"),
        BrainState::Stopped => ("problem", "Stopped"),
        _ if b.last_error.is_some() => ("problem", "Had an error"),
        _ => ("idle", "Ready"),
    }
}

pub fn bot_json(b: &BotView) -> Value {
    let (state, text) = state_of(b);
    json!({
        "id": b.id, "name": b.name, "state": state, "stateText": text,
        "lastLine": b.last_line, "lastAt": b.last_at, "unread": b.unread,
        "openCards": b.open_cards, "typing": b.typing,
    })
}

fn message_json(m: &Message) -> Value {
    let mut v = serde_json::to_value(m).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        for key in ["paneId", "costNano"] {
            o.remove(key);
        }
    }
    v
}

fn card_json(c: &Card) -> Value {
    let mut v = serde_json::to_value(c).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        for key in ["commandHash", "conversation", "job", "classes"] {
            o.remove(key);
        }
    }
    v
}

fn room_json(s: &Service, r: &rstore::Room) -> Value {
    json!({
        "id": r.id, "name": r.name, "bots": r.bot_ids.iter().map(|b| s.bot_name(b)).collect::<Vec<_>>(),
        "lastMs": r.last_ms, "unread": r.unread, "typing": s.room_typing(&r.id),
    })
}

// ---- routes ---------------------------------------------------------------------------------------------------

pub fn route(ctx: &Ctx, req: &Request, seg: &[&str], who: &Caller) -> Response {
    match (req.method.as_str(), seg) {
        ("GET", ["me"]) => me(ctx, who),
        ("DELETE", ["me"]) => {
            let revoked = ctx.pairing.lock().revoke_phone_device(&who.device_id);
            ctx.changes.bump();
            match revoked {
                Ok(_) => Response::json(200, &json!({ "ok": true })),
                Err(_) => err(500, "internal", "Could not sign this phone out. Try again."),
            }
        }
        ("GET", ["changes"]) => changes(ctx, req),
        ("GET", ["bots"]) => bots(ctx),
        ("GET", ["bots", id]) => bot(ctx, req, id),
        ("POST", ["bots", id, "read"]) => read(ctx, id),
        ("POST", ["bots", id, "send"]) => send(ctx, req, id),
        ("POST", ["bots", id, "cards", card, "answer"]) => answer(ctx, req, id, card),
        ("POST", ["bots", id, "stop"]) => stop(ctx, id),
        ("POST", ["pause-all"]) => {
            let hold = ctx.env.pause_all();
            ctx.changes.bump();
            Response::json(200, &json!({ "paused": hold.held }))
        }
        ("POST", ["resume-all"]) => match ctx.env.resume_all() {
            Ok(hold) => {
                ctx.changes.bump();
                Response::json(200, &json!({ "paused": hold.held }))
            }
            Err(e) => err(500, "internal", &e),
        },
        ("GET", ["rooms", id]) => room(ctx, id),
        ("POST", ["rooms", id, "send"]) => room_send(ctx, req, id),
        ("GET", ["push", "key"]) | ("POST", ["push", "subscribe"]) | ("DELETE", ["push", "subscribe"]) | ("POST", ["push", "presence"]) => super::push_routes::route(ctx, req, seg, who),
        _ => err(404, "not_found", "There is no such route."),
    }
}

fn me(ctx: &Ctx, who: &Caller) -> Response {
    let name = ctx.pairing.lock().device_name(&who.device_id).unwrap_or_default();
    Response::json(
        200,
        &json!({
            "deviceId": who.device_id, "name": name, "pcName": ctx.env.pc_name(),
            "caps": who.caps.list(), "groups": ctx.env.groups_on(), "paused": ctx.env.paused(),
            "candidates": ctx.env.candidates(),
        }),
    )
}

fn changes(ctx: &Ctx, req: &Request) -> Response {
    let since = req.query_param("since").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let wait = req.query_param("wait_ms").and_then(|v| v.parse::<u64>().ok()).unwrap_or(20_000).min(25_000);
    let rev = ctx.changes.wait(since, Duration::from_millis(wait));
    Response::json(200, &json!({ "rev": rev }))
}

fn bots(ctx: &Ctx) -> Response {
    let s = match service(ctx) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let state = match s.state() {
        Ok(state) => state,
        Err(e) => return store_err(e),
    };
    let mut rooms = Vec::new();
    if ctx.env.groups_on() {
        if let Ok(list) = rstore::load_rooms(s.root()) {
            rooms = list.iter().map(|r| room_json(&s, r)).collect();
        }
    }
    Response::json(
        200,
        &json!({
            "rev": ctx.changes.rev(), "paused": ctx.env.paused(),
            "bots": state.bots.iter().filter(|b| !b.archived).map(bot_json).collect::<Vec<_>>(),
            "rooms": rooms,
        }),
    )
}

fn bot(ctx: &Ctx, req: &Request, id: &str) -> Response {
    let s = match service(ctx) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let limit = req.query_param("limit").and_then(|v| v.parse::<usize>().ok()).unwrap_or(200).clamp(1, 500);
    let state = match s.state() {
        Ok(state) => state,
        Err(e) => return store_err(e),
    };
    let Some(view) = state.bots.iter().find(|b| b.id == id) else {
        return err(404, "potato_bot_not_found", "No bot has that name.");
    };
    let messages = match s.messages(id, limit) {
        Ok(m) => m,
        Err(e) => return store_err(e),
    };
    let cards = s.open_cards(id).unwrap_or_default();
    let job = s.job_view(id).ok().flatten();
    Response::json(
        200,
        &json!({
            "rev": ctx.changes.rev(),
            "bot": bot_json(view),
            "messages": messages.iter().map(message_json).collect::<Vec<_>>(),
            "cards": cards.iter().map(card_json).collect::<Vec<_>>(),
            "job": job.map(|j| serde_json::to_value(j).unwrap_or(Value::Null)),
            "work": view.work.as_ref().map(|w| serde_json::to_value(w).unwrap_or(Value::Null)),
            "typing": view.typing,
        }),
    )
}

fn read(ctx: &Ctx, id: &str) -> Response {
    let s = match service(ctx) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match s.mark_read(id) {
        Ok(()) => Response::json(200, &json!({ "ok": true })),
        Err(e) => store_err(e),
    }
}

fn send(ctx: &Ctx, req: &Request, id: &str) -> Response {
    let s = match service(ctx) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let v = match body(req) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let text = v.get("text").and_then(Value::as_str).unwrap_or("");
    if text.chars().count() > MAX_TEXT {
        return err(400, "too_long", "That message is too long.");
    }
    let files: Vec<crate::potato::attachments::Incoming> = match v.get("files") {
        Some(f) if !f.is_null() => match serde_json::from_value(f.clone()) {
            Ok(files) => files,
            Err(_) => return err(400, "bad_request", "Those files are not in the shape the server expects."),
        },
        _ => Vec::new(),
    };
    if text.trim().is_empty() && files.is_empty() {
        return err(400, "empty", "Write something first.");
    }
    let outcome = if files.is_empty() {
        s.send(id, text)
    } else {
        let root = s.root().to_path_buf();
        if !store::load_bots(&root).map(|b| b.iter().any(|b| b.id == id)).unwrap_or(false) {
            return err(404, "potato_bot_not_found", "No bot has that name.");
        }
        let stamp = ctx.env.now_ms() as i64;
        match crate::potato::attachments::save_all(&root, id, &files, stamp) {
            Err(e) => return store_err(e),
            Ok(saved) => {
                let sent = crate::potato::attachments::compose(text, &saved).and_then(|m| s.send(id, &m));
                if sent.is_err() {
                    crate::potato::attachments::forget(&root, id, &saved);
                }
                sent
            }
        }
    };
    match outcome {
        Ok(o) => {
            ctx.changes.bump();
            Response::json(200, &json!({ "id": o.id, "delivered": o.delivered, "state": serde_json::to_value(o.state).unwrap_or(Value::Null) }))
        }
        Err(e) => store_err(e),
    }
}

/// What a phone may answer. Secrets and standing changes stay on the PC:
/// - a sign-in card can only be dismissed (its last button, "Not now");
/// - a connector card's first button opens PC settings, so it is not answerable here;
/// - "Always allow" is never offered (it is a different command and not routed).
fn answer(ctx: &Ctx, req: &Request, bot: &str, card_id: &str) -> Response {
    let s = match service(ctx) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let v = match body(req) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let given = v.get("answer").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let Some(card) = s.open_cards(bot).unwrap_or_default().into_iter().find(|c| c.id == card_id) else {
        return err(404, "card_not_open", "That card is not waiting any more.");
    };
    if card.kind == Kind::Question {
        let index = given.parse::<usize>().ok();
        if card.login.is_some() && index != card.choices.len().checked_sub(1) {
            return err(403, "pc_only", "Sign in on your PC.");
        }
        if card.connector.is_some() && index == Some(0) {
            return err(403, "pc_only", "Connect this on your PC.");
        }
    }
    match s.answer_card(bot, card_id, &given) {
        Ok(o) => {
            ctx.changes.bump();
            Response::json(200, &json!({ "ok": true, "answer": o.answer }))
        }
        Err(e) => store_err(e),
    }
}

fn stop(ctx: &Ctx, id: &str) -> Response {
    let s = match service(ctx) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if let Err(e) = s.require_bot_exists(id) {
        return store_err(e);
    }
    s.stop_bot_by_person(id);
    ctx.changes.bump();
    Response::json(200, &json!({ "ok": true }))
}

fn room(ctx: &Ctx, id: &str) -> Response {
    if !ctx.env.groups_on() {
        return err(404, "groups_off", "Bot groups are off on the PC.");
    }
    let s = match service(ctx) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if !store::valid_room_id(id) {
        return err(400, "bad_request", "That is not a group.");
    }
    let Some(room) = rstore::load_rooms(s.root()).ok().and_then(|rooms| rooms.into_iter().find(|r| r.id == id)) else {
        return err(404, "not_found", "No group has that name.");
    };
    let lines = match s.room_lines(id, 200) {
        Ok(l) => l,
        Err(e) => return err(400, "room_error", &format!("{e:?}")),
    };
    Response::json(
        200,
        &json!({
            "rev": ctx.changes.rev(), "room": room_json(&s, &room), "lines": lines,
            "work": s.room_work(id).into_iter().map(|(bot, run)| json!({ "bot": bot, "run": run })).collect::<Vec<_>>(),
            "typing": s.room_typing(id),
        }),
    )
}

fn room_send(ctx: &Ctx, req: &Request, id: &str) -> Response {
    if !ctx.env.groups_on() {
        return err(404, "groups_off", "Bot groups are off on the PC.");
    }
    let s = match service(ctx) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if !store::valid_room_id(id) {
        return err(400, "bad_request", "That is not a group.");
    }
    let v = match body(req) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let text = v.get("text").and_then(Value::as_str).unwrap_or("");
    if text.trim().is_empty() || text.chars().count() > MAX_TEXT {
        return err(400, "bad_request", "Write something short first.");
    }
    match s.send_room(id, text) {
        Ok(_) => {
            ctx.changes.bump();
            Response::json(200, &json!({ "ok": true }))
        }
        Err(e) => err(400, "room_error", &format!("{e:?}")),
    }
}
