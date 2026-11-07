//! Web push for a phone: `GET /push/key`, `POST /push/subscribe`, `DELETE /push/subscribe`, `POST /push/presence`.
//! The PC sends the pushes itself with its own VAPID key (RFC 8030/8291/8292); there is no hosted service.
//! A subscription belongs to the device whose token made the request, never to a device named in the body.

use serde_json::{json, Value};

use super::gate::{Caller, Ctx};
use super::http::{Request, Response};
use crate::push::store::Subscription;

pub fn route(ctx: &Ctx, req: &Request, seg: &[&str], who: &Caller) -> Response {
    let Some(hub) = ctx.env.push() else {
        return Response::error(503, "push_unavailable", "Notifications are not ready on the PC yet.");
    };
    let now = ctx.env.now_ms();
    match (req.method.as_str(), seg) {
        ("GET", ["push", "key"]) => Response::json(200, &json!({ "publicKey": hub.vapid().public_key_b64url() })),
        ("POST", ["push", "subscribe"]) => {
            let v: Value = match serde_json::from_slice(&req.body) {
                Ok(v) => v,
                Err(_) => return Response::error(400, "bad_request", "That is not a push subscription."),
            };
            let text = |value: Option<&Value>| value.and_then(Value::as_str).unwrap_or("").to_string();
            let sub = Subscription {
                endpoint: text(v.get("endpoint")),
                p256dh: text(v.pointer("/keys/p256dh")),
                auth: text(v.pointer("/keys/auth")),
                created_at: now / 1000,
            };
            match hub.store().lock().upsert(&who.device_id, sub) {
                Ok(()) => Response::json(200, &json!({ "ok": true })),
                Err(why) => Response::error(400, "invalid_subscription", &format!("That push address is not accepted: {why}.")),
            }
        }
        ("DELETE", ["push", "subscribe"]) => {
            let removed = hub.store().lock().remove(&who.device_id).unwrap_or(false);
            Response::json(200, &json!({ "ok": true, "removed": removed }))
        }
        ("POST", ["push", "presence"]) => {
            let visible = serde_json::from_slice::<Value>(&req.body).ok().and_then(|v| v.get("visible").and_then(Value::as_bool)).unwrap_or(false);
            hub.set_visible(&who.device_id, visible, now);
            Response::json(200, &json!({ "ok": true }))
        }
        _ => Response::error(404, "not_found", "There is no such route."),
    }
}
