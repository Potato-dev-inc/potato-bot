//! Top-level request routing for the local bridge: health, and nothing else.

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;
use tauri::AppHandle;

use super::errors::RouteResult;
use crate::pty::PaneRegistry;

#[derive(Debug, Serialize)]
struct Health {
    ok: bool,
    version: &'static str,
    build_id: &'static str,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn route(
    method: &str,
    segments: &[&str],
    _query: &str,
    _body: &[u8],
    _registry: &Arc<Mutex<PaneRegistry>>,
    _app: &AppHandle,
    _headers: &str,
    _peer_loopback: bool,
    _peer: Option<crate::companion::peer::Peer>,
) -> RouteResult {
    if segments == ["health"] && method == "GET" {
        return Ok((
            200,
            serde_json::to_value(Health {
                ok: true,
                version: env!("CARGO_PKG_VERSION"),
                build_id: env!("PM_BUILD_ID"),
            })
            .unwrap(),
        ));
    }
    Err((404, json!({ "error": "not found" })))
}
