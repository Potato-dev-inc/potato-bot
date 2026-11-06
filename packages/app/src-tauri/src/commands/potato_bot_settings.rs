//! The bot panel's per-bot settings (Advanced tab): the bot's own talk brain, its instructions with their history,
//! its tool switches, Rename, Delete and "Export chat". Desktop only, like the rest of Potato Bot: none of these is
//! reachable through the bridge, the phone or MCP. Errors are one string, `CODE: plain sentence`.
//!
//! The engine is in `potato::service`; the per-bot file is in `potato::bot_settings`. Rename changes the companion's
//! name (a bot IS a companion), through the companion service, so the name stays one name everywhere.

use serde::Serialize;
use tauri::State;

use super::session_chat::SessionChatState;
use crate::companion::manage::Patch;
use crate::potato::bot_settings::{self, BotOptions, ToolSwitches};
use crate::potato::live;
use crate::potato::risk::Level;
use crate::potato::service::Service;
use crate::potato::store::{BrainKind, StoreError};
use crate::session_chat::export_save::PageKind;
use std::sync::Arc;

fn service() -> Result<Arc<Service>, String> {
    live::service().ok_or_else(|| "POTATO_NOT_READY: Potato Bot is not ready yet. Try again in a moment.".to_string())
}

fn text(error: StoreError) -> String {
    error.to_string()
}

/// Runs one service call off the UI thread (a stop waits for the brain to leave).
async fn run<T: Send + 'static>(call: impl FnOnce(&Service) -> Result<T, StoreError> + Send + 'static) -> Result<T, String> {
    let service = service()?;
    tauri::async_runtime::spawn_blocking(move || call(&service).map_err(text)).await.map_err(|e| format!("INTERNAL: {e}"))?
}

/// What the Advanced tab shows: the brain (`null` = the default), the tool switches, whether the sandbox Labs
/// switch is on, the instructions in use and their versions (newest first).
#[tauri::command]
pub async fn potato_bot_options_get(bot_id: String) -> Result<BotOptions, String> {
    run(move |s| s.bot_options(&bot_id)).await
}

/// Picks the bot's talk brain. `null` goes back to the default in Settings.
#[tauri::command]
pub async fn potato_bot_brain_set(bot_id: String, brain: Option<BrainKind>) -> Result<BotOptions, String> {
    run(move |s| {
        s.set_talk_brain(&bot_id, brain)?;
        s.bot_options(&bot_id)
    })
    .await
}

/// Saves the bot's own OpenAI-compatible provider (base URL and model). `null` goes back to the default in
/// Settings. Its key is set separately under the credential store name `potato_provider_key_<bot>`.
#[tauri::command]
pub async fn potato_bot_provider_set(bot_id: String, custom: Option<crate::potato::store::CustomProvider>) -> Result<BotOptions, String> {
    run(move |s| {
        s.set_provider(&bot_id, custom)?;
        s.bot_options(&bot_id)
    })
    .await
}

#[tauri::command]
pub async fn potato_bot_tools_set(bot_id: String, tools: ToolSwitches) -> Result<BotOptions, String> {
    run(move |s| {
        s.set_tools(&bot_id, tools)?;
        s.bot_options(&bot_id)
    })
    .await
}

/// Sets how much the bot asks before it acts in its computer: `ask_first`, `ask_when_risky` or `full`. Only this command
/// writes the level; nothing the bot can call reaches it.
#[tauri::command]
pub async fn potato_bot_level_set(bot_id: String, level: Level) -> Result<BotOptions, String> {
    run(move |s| {
        s.set_level(&bot_id, level)?;
        s.bot_options(&bot_id)
    })
    .await
}

/// Sets the limit on the sites the bot's web tools open (`enabled`, and the list of sites). Only this command writes it; nothing the
/// bot can call reaches it. An invalid site answers a plain sentence and saves nothing.
#[tauri::command]
pub async fn potato_bot_sites_set(bot_id: String, enabled: bool, sites: Vec<String>) -> Result<BotOptions, String> {
    run(move |s| {
        s.set_site_limit(&bot_id, enabled, &sites)?;
        s.bot_options(&bot_id)
    })
    .await
}

/// Removes one saved "Always allow" rule (Settings > Advanced). The next command of that class asks again, at once.
#[tauri::command]
pub async fn potato_bot_always_remove(bot_id: String, class: String) -> Result<BotOptions, String> {
    run(move |s| {
        s.remove_always_rule(&bot_id, &class)?;
        s.bot_options(&bot_id)
    })
    .await
}

/// Saves the instructions as a new version (nothing is added when the text is the one in use).
#[tauri::command]
pub async fn potato_bot_instructions_save(bot_id: String, text: String) -> Result<BotOptions, String> {
    run(move |s| {
        s.set_instructions(&bot_id, &text)?;
        s.bot_options(&bot_id)
    })
    .await
}

/// Restores the version saved at `at` (epoch ms, as listed) by adding it again as the newest one.
#[tauri::command]
pub async fn potato_bot_instructions_restore(bot_id: String, at: i64) -> Result<BotOptions, String> {
    run(move |s| {
        s.restore_instructions(&bot_id, at)?;
        s.bot_options(&bot_id)
    })
    .await
}

/// Renames the bot. The name is the companion's name, so the change is made there.
#[tauri::command]
pub async fn potato_bot_rename(bot_id: String, name: String) -> Result<(), String> {
    let name = bot_settings::clean_name(&name).map_err(text)?;
    let check = bot_id.clone();
    run(move |s| s.require_bot_exists(&check)).await?;
    let companions = crate::companion::live::service().ok_or_else(|| "COMPANION_NOT_READY: Companions are not ready yet. Try again in a moment.".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        companions
            .update(&bot_id, Patch { name: Some(name), ..Default::default() })
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("INTERNAL: {e}"))?
}

/// Deletes the bot. `typed_name` must be the bot's name exactly (the page asks the person to type it; the engine
/// checks it again). Its chat, settings and brain folder move into the removed folder, and its brain is stopped.
/// With `delete_companion`, the companion profile goes too, so it frees one of the three companion slots.
#[tauri::command]
pub async fn potato_bot_delete(bot_id: String, typed_name: String, delete_companion: Option<bool>) -> Result<(), String> {
    let also = delete_companion.unwrap_or(false);
    let id = bot_id.clone();
    run(move |s| s.delete_bot(&bot_id, &typed_name)).await?;
    if also {
        let companions = crate::companion::live::service()
            .ok_or_else(|| "COMPANION_NOT_READY: Companions are not ready yet. Try again in a moment.".to_string())?;
        tauri::async_runtime::spawn_blocking(move || companions.delete(&id).map_err(|e| e.to_string()))
            .await
            .map_err(|e| format!("INTERNAL: {e}"))??;
    }
    Ok(())
}

/// "Export chat": the bot's chat as one Markdown file. Returns the text (for the preview) and the one-time token
/// that `chat_export_save` writes, so the save goes through the same dialog path as the pane exports.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BotExport {
    pub markdown: String,
    pub token: String,
}

#[tauri::command]
pub async fn potato_bot_export_markdown(chat: State<'_, SessionChatState>, bot_id: String) -> Result<BotExport, String> {
    let key = format!("potato:{bot_id}");
    let markdown = run(move |s| s.export_markdown(&bot_id)).await?;
    let token = chat.pages().keep(&key, PageKind::Markdown, &markdown)?;
    Ok(BotExport { markdown, token })
}

/// Undo on a card that says the bot changed one of its own settings (name, instructions, persona or model): puts the old
/// value back. The owner's click only; the bot has no tool for it. A change can be undone once.
#[tauri::command]
pub async fn potato_setting_undo(bot_id: String, change_id: String) -> Result<(), String> {
    run(move |s| s.undo_setting(&bot_id, &change_id)).await
}
