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
use crate::potato::sandbox::screen::{Audience, Mode, ScreenError, LINK_TTL_MS};
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
        // Spaces on the phone: read and send only (no invites, kicks, bots or leaving from a phone).
        ("GET", ["spaces"]) | ("GET", ["spaces", _, "channels", _, "messages"]) | ("POST", ["spaces", _, "channels", _, "read"]) => Some(Cap::Watch),
        ("POST", ["spaces", _, "channels", _, "send"]) => Some(Cap::Type),
        // A screen link: watching needs `watch`; the body's `control` mode needs more (`need_for`, and `type` in the route).
        ("POST", ["bots", _, "screen"]) => Some(Cap::Watch),
        // Handing the screen back and keeping a taken screen are control actions.
        ("POST", ["bots", _, "screen", "release"]) | ("POST", ["bots", _, "screen", "keep"]) => Some(Cap::Control),
        _ => Some(Cap::Control),
    }
}

/// The ability a request needs: `need`, raised to `control` when a screen link asks for `control` mode.
pub fn need_for(req: &Request, seg: &[&str]) -> Option<Cap> {
    let base = need(req.method.as_str(), seg);
    if let ("POST", ["bots", _, "screen"]) = (req.method.as_str(), seg) {
        let mode = serde_json::from_slice::<Value>(&req.body).ok().and_then(|v| v.get("mode").and_then(Value::as_str).map(str::to_string));
        if mode.as_deref() != Some("view") {
            // "control", and anything we cannot read as "view", is held to the stricter ability (fail closed).
            return Some(Cap::Control);
        }
    }
    base
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

/// Whether the bot uses the sandbox computer: Potato Bot and the Sandbox switch are on, a sandbox is set up, and the bot's
/// own Computer tool switch is on (the same conditions under which its brain is told it has a computer).
fn has_computer(s: &Service, bot: &str) -> bool {
    s.sandbox_offered() && crate::potato::bot_settings::load(s.root(), bot).map(|b| b.tools.computer).unwrap_or(false)
}

pub fn bot_json(b: &BotView, computer: bool) -> Value {
    let (state, text) = state_of(b);
    let mut v = json!({
        "id": b.id, "name": b.name, "state": state, "stateText": text,
        "lastLine": b.last_line, "lastAt": b.last_at, "unread": b.unread,
        "openCards": b.open_cards, "typing": b.typing,
    });
    if computer {
        v["computer"] = json!(true);
    }
    v
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
            super::control::clear(&ctx.env.screen_hub(), &who.device_id);
            ctx.changes.bump();
            match revoked {
                Ok(_) => Response::json(200, &json!({ "ok": true })),
                Err(_) => err(500, "internal", "Could not sign this phone out. Try again."),
            }
        }
        ("GET", ["changes"]) => changes(ctx, req),
        (_, ["spaces", ..]) => super::spaces_routes::route(ctx, req, seg),
        ("GET", ["bots"]) => bots(ctx),
        ("GET", ["bots", id]) => bot(ctx, req, id),
        ("POST", ["bots", id, "read"]) => read(ctx, id),
        ("POST", ["bots", id, "send"]) => send(ctx, req, id),
        ("POST", ["bots", id, "cards", card, "answer"]) => answer(ctx, req, id, card),
        ("POST", ["bots", id, "stop"]) => stop(ctx, id),
        ("POST", ["bots", id, "screen"]) => screen_link(ctx, req, id, who),
        ("POST", ["bots", _, "screen", "release"]) => screen_release(ctx, who),
        ("POST", ["bots", _, "screen", "keep"]) => screen_keep(ctx, who),
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
    let mut v = json!({
        "deviceId": who.device_id, "name": name, "pcName": ctx.env.pc_name(),
        "caps": who.caps.list(), "groups": ctx.env.groups_on(), "paused": ctx.env.paused(),
        "candidates": ctx.env.candidates(),
    });
    if ctx.env.spaces_on() {
        v["spaces"] = json!(true);
    }
    Response::json(200, &v)
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
            "bots": state.bots.iter().filter(|b| !b.archived).map(|b| bot_json(b, has_computer(&s, &b.id))).collect::<Vec<_>>(),
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
            "bot": bot_json(view, has_computer(&s, id)),
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
        let computer = crate::potato::attachments::running_computer(&s, id);
        match crate::potato::attachments::prepare(&root, id, text, &files, stamp, computer.as_deref()) {
            Err(e) => return store_err(e),
            Ok(prepared) => {
                let sent = s.send_with_images(id, &prepared.message, &prepared.images);
                if sent.is_err() {
                    crate::potato::attachments::forget(&root, id, &prepared.saved);
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

// ---- the bot's computer screen ---------------------------------------------------------------------------------

/// A screen error as the phone API says it: `{ error: code, message }`, the code in lower case, no secret in either.
fn screen_err(e: &ScreenError) -> Response {
    let status = match e {
        ScreenError::Off => 409,
        other => other.status(),
    };
    err(status, &e.code().to_ascii_lowercase(), &e.message())
}

/// `POST /api/bots/{id}/screen` `{ "mode": "view" | "control" }`: a link to this bot's computer screen for THIS phone.
/// The same hub, claims, one hour, lease and "the person said yes at the PC" rules as the desktop's Computer panel; the
/// link is only good for this phone, on this server (`/screen/<sealed>/embed.html`). The VNC password is in the `#`
/// fragment of `src` and nowhere else.
fn screen_link(ctx: &Ctx, req: &Request, id: &str, who: &Caller) -> Response {
    let v = match body(req) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(mode) = v.get("mode").and_then(Value::as_str).and_then(Mode::parse) else {
        return err(400, "bad_request", "mode is view or control.");
    };
    // Control is typing and clicking on the PC's computer: it needs both abilities (the gate already asked for `control`).
    if mode == Mode::Control && !(who.caps.has(Cap::Control) && who.caps.has(Cap::Type)) {
        return err(403, "not_allowed", "This phone is not allowed to do that (control). You can change it on your PC: Settings > Phone.");
    }
    let s = match service(ctx) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if let Err(e) = s.require_bot_exists(id) {
        return store_err(e);
    }
    if !has_computer(&s, id) {
        return err(409, "no_computer", "This bot does not use a computer.");
    }
    let hub = ctx.env.screen_hub();
    let audience = Audience::Device(who.device_id.clone());
    if mode == Mode::Control {
        // Refuse before anything is taken: the switch, a running sandbox, and the person's yes at the PC.
        if let Err(e) = hub.ensure_running() {
            return screen_err(&e);
        }
        if !hub.phone_may_control(&who.device_id) {
            return screen_err(&ScreenError::PhoneNeedsDesktopYes);
        }
        // The person takes the screen exactly as the PC's "Take over" does (the bot is displaced, not fought). When the
        // person already holds it (at the PC), nothing is taken again: that would end their own control link.
        if hub.state().phase != "you" {
            if let Err(e) = hub.take_over() {
                return screen_err(&e);
            }
            ctx.changes.bump();
        }
    }
    let query = if mode == Mode::Control { "?path=websockify&control=1" } else { "?path=websockify" };
    match hub.link_at(id, mode, &audience, super::screen_routes::ROUTE_ROOT, query) {
        Ok(src) => Response::json(200, &json!({ "src": src.into_inner(), "mode": mode, "expiresAtMs": hub.now_ms() + LINK_TTL_MS })),
        Err(e) => screen_err(&e),
    }
}

/// `POST /api/bots/{id}/screen/release`: "I'm done": the phone that was allowed to take control gives the screen back
/// (what the PC's "Hand back" does; the bot may take a lease again).
fn screen_release(ctx: &Ctx, who: &Caller) -> Response {
    let hub = ctx.env.screen_hub();
    if !hub.phone_may_control(&who.device_id) {
        return screen_err(&ScreenError::PhoneNeedsDesktopYes);
    }
    match hub.hand_back() {
        Ok(state) => {
            if let Ok(s) = service(ctx) {
                s.login_cards_after_hand_back(None);
            }
            ctx.changes.bump();
            Response::json(200, &json!({ "ok": true, "phase": state.phase }))
        }
        Err(e) => screen_err(&e),
    }
}

/// `POST /api/bots/{id}/screen/keep`: the person is still working: the lease lasts 15 more minutes.
fn screen_keep(ctx: &Ctx, who: &Caller) -> Response {
    let hub = ctx.env.screen_hub();
    if !hub.phone_may_control(&who.device_id) {
        return screen_err(&ScreenError::PhoneNeedsDesktopYes);
    }
    match hub.keep() {
        Ok(()) => Response::json(200, &json!({ "ok": true })),
        Err(e) => screen_err(&e),
    }
}
