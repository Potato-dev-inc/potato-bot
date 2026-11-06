//! Tauri commands of Potato Bot's routines (phase P9a). Desktop only: a routine is made and changed by the person
//! at this computer, never through the bridge, the phone or MCP. The list, create, update, pause, resume, delete and
//! "run now" call `potato::routines`; the 30 s tick lives in `potato::live`.
//!
//! Errors are one string, `CODE: plain sentence`, like the other Potato Bot commands.

use crate::potato::live;
use crate::potato::routines::{self, RoutineInput, RoutineView};
use crate::potato::service::SendOutcome;
use crate::potato::trash;
use crate::potato::store::StoreError;
use crate::potato::waits::{self, WaitView};

fn text(error: StoreError) -> String {
    error.to_string()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn root() -> Result<std::path::PathBuf, String> {
    live::service().map(|s| s.root().to_path_buf()).ok_or_else(|| "POTATO_NOT_READY: Potato Bot is not ready yet. Try again in a moment.".to_string())
}

/// The routines of one bot, or of every bot when `bot_id` is missing.
#[tauri::command]
pub async fn potato_routines_list(bot_id: Option<String>) -> Result<Vec<RoutineView>, String> {
    let root = root()?;
    tauri::async_runtime::spawn_blocking(move || routines::list(&root, bot_id.as_deref()).map_err(text))
        .await
        .map_err(|e| format!("INTERNAL: {e}"))?
}

/// The waits a bot has open (`wait_until`): what it waits for, how often it looks, how many looks so far.
#[tauri::command]
pub async fn potato_waits_list(bot_id: Option<String>) -> Result<Vec<WaitView>, String> {
    let root = root()?;
    tauri::async_runtime::spawn_blocking(move || waits::list(&root, bot_id.as_deref()).map_err(text))
        .await
        .map_err(|e| format!("INTERNAL: {e}"))?
}

/// The owner gives up a wait. The bot is not told: nothing wakes it.
#[tauri::command]
pub async fn potato_wait_cancel(wait_id: String) -> Result<(), String> {
    let root = root()?;
    tauri::async_runtime::spawn_blocking(move || waits::cancel(&root, None, &wait_id).map(|_| ()).map_err(text))
        .await
        .map_err(|e| format!("INTERNAL: {e}"))?
}

#[tauri::command]
pub async fn potato_routine_create(bot_id: String, input: RoutineInput) -> Result<RoutineView, String> {
    let root = root()?;
    tauri::async_runtime::spawn_blocking(move || routines::create(&root, &bot_id, &input, now_ms()).map_err(text))
        .await
        .map_err(|e| format!("INTERNAL: {e}"))?
}

#[tauri::command]
pub async fn potato_routine_update(routine_id: String, input: RoutineInput) -> Result<RoutineView, String> {
    let root = root()?;
    tauri::async_runtime::spawn_blocking(move || routines::update(&root, &routine_id, &input, now_ms()).map_err(text))
        .await
        .map_err(|e| format!("INTERNAL: {e}"))?
}

#[tauri::command]
pub async fn potato_routine_pause(routine_id: String, paused: bool) -> Result<RoutineView, String> {
    let root = root()?;
    tauri::async_runtime::spawn_blocking(move || routines::set_paused(&root, &routine_id, paused, now_ms()).map_err(text))
        .await
        .map_err(|e| format!("INTERNAL: {e}"))?
}

/// Deletes one routine at once: it goes to Recently deleted (7 days). Returns the trash id, for "Undo".
#[tauri::command]
pub async fn potato_routine_delete(routine_id: String) -> Result<String, String> {
    let root = root()?;
    tauri::async_runtime::spawn_blocking(move || trash::trash_routine(&root, &routine_id, now_ms()).map(|item| item.id).map_err(text))
        .await
        .map_err(|e| format!("INTERNAL: {e}"))?
}

/// "Run now": the routine's prompt goes to its bot at once. The schedule does not move.
#[tauri::command]
pub async fn potato_routine_run_now(routine_id: String) -> Result<SendOutcome, String> {
    let service = live::service().ok_or_else(|| "POTATO_NOT_READY: Potato Bot is not ready yet. Try again in a moment.".to_string())?;
    tauri::async_runtime::spawn_blocking(move || routines::run_now(&service, &routine_id, now_ms()).map_err(text))
        .await
        .map_err(|e| format!("INTERNAL: {e}"))?
}

/// The next few runs of a cron, for the preview in the New routine form (the Rust side owns the cron reader, so the
/// page does not copy it). Times are instants; the page writes them in the computer's local clock. A cron that would
/// not be saved answers with the plain-English reason.
#[tauri::command]
pub async fn potato_routine_cron_preview(expr: String, count: Option<usize>) -> Result<Vec<i64>, String> {
    routines::cron_preview(&expr, now_ms(), count.unwrap_or(3)).map_err(text)
}

/// The last runs of every routine of one bot, newest first (10 per routine): when, ok / failed / skipped with a plain
/// sentence, and the id of the bot's reply message in the thread when there is one.
#[tauri::command]
pub async fn potato_routine_runs(bot_id: String) -> Result<Vec<routines::RunView>, String> {
    let root = root()?;
    tauri::async_runtime::spawn_blocking(move || routines::runs_of(&root, &bot_id).map_err(text))
        .await
        .map_err(|e| format!("INTERNAL: {e}"))?
}
