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

/// Picks the model the bot uses on the Go plan. `null` goes back to the default in Settings.
#[tauri::command]
pub async fn potato_bot_go_model_set(bot_id: String, model: Option<String>) -> Result<BotOptions, String> {
    run(move |s| {
        s.set_go_model(&bot_id, model)?;
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

// ---- moving bots in and out (bot file, copy, account folder, ChatGPT or Claude drafts) ------------------------------------

use crate::potato::service::portable::{AccountPreview, Applied, BotInfo, Choices, Preview};

/// The avatar and the voice the bot's companion has now, looked up here so the page does not have to send them.
fn companion_info(bot_id: &str) -> BotInfo {
    let view = crate::companion::live::service().and_then(|c| c.views(false).into_iter().find(|v| v.id == bot_id));
    match view {
        Some(v) => BotInfo { name: v.name, avatar: v.avatar, persona: v.persona, talk_persona: String::new() },
        None => BotInfo::default(),
    }
}

/// "Export bot": writes the bot's file at `path` (the page's save dialog chose it) and returns the path. `memory`: the notes come too.
#[tauri::command]
pub async fn potato_bot_export_save(bot_id: String, memory: bool, path: String) -> Result<String, String> {
    let target = std::path::PathBuf::from(&path);
    if target.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() != Some("json") {
        return Err("POTATO_EXPORT_PATH: Save the bot as a .json file.".to_string());
    }
    let info = companion_info(&bot_id);
    let (file, _name) = run(move |s| s.export_bot_text(&bot_id, info, memory)).await?;
    std::fs::write(&target, file).map_err(|e| format!("POTATO_EXPORT_IO: Could not save the file: {e}."))?;
    Ok(target.display().to_string())
}

/// The name the save dialog starts with.
#[tauri::command]
pub async fn potato_bot_export_name(bot_id: String) -> Result<String, String> {
    let info = companion_info(&bot_id);
    run(move |s| s.export_bot_text(&bot_id, info, false).map(|(_, name)| name)).await
}

/// What a bot file holds: the preview. Nothing is made or written.
#[tauri::command]
pub async fn potato_bot_file_inspect(path: String) -> Result<Preview, String> {
    run(move |s| s.inspect_bot_file(std::path::Path::new(&path))).await
}

/// Puts the ticked parts of a bot file into a bot the page just made.
#[tauri::command]
pub async fn potato_bot_file_apply(bot_id: String, path: String, choices: Choices) -> Result<Applied, String> {
    run(move |s| s.apply_bot_file(&bot_id, std::path::Path::new(&path), choices)).await
}

/// "Duplicate": puts a bot's instructions, skills, routines (paused) and memory into a second bot the page just made.
#[tauri::command]
pub async fn potato_bot_copy_into(from_id: String, to_id: String) -> Result<Applied, String> {
    run(move |s| s.copy_bot_into(&from_id, &to_id)).await
}

/// Whole-account copy: writes a folder inside `parent` and returns its path.
#[tauri::command]
pub async fn potato_account_export(parent: String, chats: bool) -> Result<String, String> {
    let names: std::collections::HashMap<String, BotInfo> = crate::companion::live::service()
        .map(|c| c.views(false).into_iter().map(|v| (v.id.clone(), BotInfo { name: v.name, avatar: v.avatar, persona: v.persona, talk_persona: String::new() })).collect())
        .unwrap_or_default();
    let dir = run(move |s| s.account_export(std::path::Path::new(&parent), &|id| names.get(id).cloned().unwrap_or_default(), chats)).await?;
    Ok(dir.display().to_string())
}

#[tauri::command]
pub async fn potato_account_inspect(dir: String) -> Result<AccountPreview, String> {
    run(move |s| s.account_inspect(std::path::Path::new(&dir))).await
}

/// One bot of a copy folder (`id` is its folder name) into a bot the page just made: its parts and its chat.
#[tauri::command]
pub async fn potato_account_restore_bot(dir: String, id: String, bot_id: String) -> Result<Applied, String> {
    run(move |s| s.account_restore_bot(std::path::Path::new(&dir), &id, &bot_id)).await
}

/// A file from ChatGPT or Claude as text (at most 2 MB; a zip is refused), so the page can look for the instructions in it.
#[tauri::command]
pub async fn potato_import_read_text(path: String) -> Result<String, String> {
    run(move |_| crate::potato::service::portable::read_capped(std::path::Path::new(&path))).await
}

/// The reviewed drafts from ChatGPT or Claude become the bot's instructions and memory notes. Same cleaning as a file.
#[tauri::command]
pub async fn potato_import_drafts_save(bot_id: String, instructions: String, memories: Vec<String>) -> Result<Applied, String> {
    run(move |s| s.save_drafts(&bot_id, &instructions, &memories)).await
}

// ---- the owner's own agents ("Through my agents"): add, approve, remove, pick -------------------------------------

/// The bot's approved agents and the one it talks through.
#[tauri::command]
pub async fn potato_agents_get(bot_id: String) -> Result<crate::potato::service::AgentsView, String> {
    run(move |s| s.agents_view(&bot_id)).await
}

/// Checks a pasted definition and answers the summary to read, or the mistake with its line. Saves nothing.
#[tauri::command]
pub async fn potato_agent_check(definition: String) -> Result<crate::potato::custom_agents::Summary, String> {
    let service = service()?;
    service.agent_check(&definition).map_err(|e| format!("POTATO_AGENT_INVALID: {e}"))
}

/// The owner approved the summary they read (`fingerprint`). Only this command adds an agent; nothing the bot can call reaches it.
#[tauri::command]
pub async fn potato_agent_add(bot_id: String, definition: String, fingerprint: String) -> Result<crate::potato::service::AgentsView, String> {
    run(move |s| s.agent_add(&bot_id, &definition, &fingerprint)).await
}

#[tauri::command]
pub async fn potato_agent_remove(bot_id: String, agent_id: String) -> Result<crate::potato::service::AgentsView, String> {
    run(move |s| s.agent_remove(&bot_id, &agent_id)).await
}

/// Picks the approved agent a bot talks through (`null`: none).
#[tauri::command]
pub async fn potato_agent_pick(bot_id: String, agent_id: Option<String>) -> Result<crate::potato::service::AgentsView, String> {
    run(move |s| s.agent_pick(&bot_id, agent_id.as_deref())).await
}
