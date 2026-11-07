//! Tauri commands for Spaces. Every one is gated by Labs `spaces_enabled`. Agents, MCP and the phone have no route here.
use std::sync::Arc;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use super::service::{BotPatch, SpacesService};

pub const OFF: &str = "Spaces is off. Turn it on in Settings, under Labs.";
const NOT_ONLINE: &str = "Spaces can't go online right now. Check your internet connection.";

/// Labs check first. Then the service that the runtime started when the switch went on. Commands never start it.
/// The notifiers are set on every call: a new service after a switch off and on gets them too.
fn ready(app: &AppHandle) -> Result<Arc<SpacesService>, String> {
    let public = crate::settings_store::read_public_settings(app);
    if !super::labs_on(&public) {
        return Err(OFF.into());
    }
    let svc = super::runtime::running_service().ok_or_else(|| NOT_ONLINE.to_string())?;
    let a = app.clone();
    svc.set_notifier(Arc::new(move |v: &Value| {
        let _ = a.emit("spaces://changed", v);
    }));
    let b = app.clone();
    svc.set_message_notifier(Arc::new(move |v: &Value| {
        let _ = b.emit("spaces://message", v);
    }));
    Ok(svc)
}

#[tauri::command]
pub async fn spaces_view(app: AppHandle) -> Result<Value, String> {
    Ok(ready(&app)?.view())
}

#[tauri::command]
pub async fn space_create(app: AppHandle, name: String) -> Result<Value, String> {
    let svc = ready(&app)?;
    let id = svc.create_space(&name).await?;
    Ok(json!({ "space": id, "view": svc.view() }))
}

#[tauri::command]
pub async fn space_invite_create(app: AppHandle, space: String) -> Result<Value, String> {
    ready(&app)?.create_invite(&space).await
}

#[tauri::command]
pub async fn space_invite_cancel(app: AppHandle, space: String, invite: String) -> Result<Value, String> {
    let svc = ready(&app)?;
    svc.cancel_invite(&space, &invite)?;
    Ok(svc.view())
}

#[tauri::command]
pub async fn space_join(app: AppHandle, text: String, words: String) -> Result<Value, String> {
    let svc = ready(&app)?;
    svc.start_join(&text, &words).await?;
    Ok(svc.view())
}

#[tauri::command]
pub async fn space_join_answer(app: AppHandle, space: String, request: i64, allow: bool) -> Result<Value, String> {
    let svc = ready(&app)?;
    svc.answer_join(&space, request, allow).await?;
    Ok(svc.view())
}

#[tauri::command]
pub async fn space_join_cancel(app: AppHandle, space: String) -> Result<Value, String> {
    let svc = ready(&app)?;
    svc.cancel_join(&space);
    Ok(svc.view())
}

#[tauri::command]
pub async fn space_detail(app: AppHandle, space: String) -> Result<Value, String> {
    ready(&app)?.detail(&space)
}

#[tauri::command]
pub async fn space_messages(app: AppHandle, space: String, channel: String, before: Option<String>, limit: u32)
    -> Result<Value, String>
{
    ready(&app)?.messages(&space, &channel, before.as_deref(), limit)
}

#[tauri::command]
pub async fn space_send(app: AppHandle, space: String, channel: String, text: String, reply: Option<String>)
    -> Result<Value, String>
{
    let svc = ready(&app)?;
    svc.send_text(&space, &channel, &text, reply.as_deref()).await
}

#[tauri::command]
pub async fn space_read(app: AppHandle, space: String, channel: String) -> Result<Value, String> {
    let svc = ready(&app)?;
    svc.mark_read(&space, &channel)?;
    svc.detail(&space)
}

#[tauri::command]
pub async fn space_channel_create(app: AppHandle, space: String, name: String) -> Result<Value, String> {
    ready(&app)?.channel_create(&space, &name).await
}

#[tauri::command]
pub async fn space_channel_rename(app: AppHandle, space: String, channel: String, name: String) -> Result<Value, String> {
    ready(&app)?.channel_rename(&space, &channel, &name).await
}

#[tauri::command]
pub async fn space_channel_archive(app: AppHandle, space: String, channel: String) -> Result<Value, String> {
    ready(&app)?.channel_archive(&space, &channel).await
}

#[tauri::command]
pub async fn space_remove_member(app: AppHandle, space: String, member: String) -> Result<Value, String> {
    let svc = ready(&app)?;
    svc.remove_member(&space, &member).await?;
    svc.detail(&space)
}

#[tauri::command]
pub async fn space_leave(app: AppHandle, space: String) -> Result<Value, String> {
    let svc = ready(&app)?;
    svc.leave(&space).await?;
    svc.detail(&space)
}

#[tauri::command]
pub async fn space_forget(app: AppHandle, space: String) -> Result<Value, String> {
    let svc = ready(&app)?;
    svc.forget(&space)?;
    Ok(svc.view())
}

#[tauri::command]
pub async fn space_block(app: AppHandle, space: String, member: String, on: bool) -> Result<Value, String> {
    ready(&app)?.block(&space, &member, on)
}

#[tauri::command]
pub async fn space_react(app: AppHandle, space: String, target: String, code: String, on: bool)
    -> Result<Value, String>
{
    ready(&app)?.react(&space, &target, &code, on).await
}

#[tauri::command]
pub async fn space_delete(app: AppHandle, space: String, target: String) -> Result<Value, String> {
    ready(&app)?.delete(&space, &target).await
}

#[tauri::command]
pub async fn space_file_offer(app: AppHandle, space: String, channel: String, path: String) -> Result<Value, String> {
    ready(&app)?.offer_file(&space, &channel, &path).await
}

/// Starts a fetch when a person clicks `Get file`. Returns when the file is here (or says why it is not).
#[tauri::command]
pub async fn space_file_get(app: AppHandle, space: String, sha: String) -> Result<Value, String> {
    ready(&app)?.get_file(&space, &sha).await?;
    Ok(json!({ "have": true }))
}

#[tauri::command]
pub async fn space_file_save(app: AppHandle, space: String, sha: String) -> Result<Value, String> {
    let svc = ready(&app)?;
    let downloads = app.path().download_dir().map_err(|_| "Can't find your Downloads folder.".to_string())?;
    svc.save_file(&space, &sha, &downloads)
}

/// The window reports `online` or `away`. Nothing is stored; peers see it for 90 seconds.
#[tauri::command]
pub async fn space_presence(app: AppHandle, state: String) -> Result<(), String> {
    ready(&app)?.set_my_presence(&state)
}

#[tauri::command]
pub async fn space_show_online(app: AppHandle, on: bool) -> Result<Value, String> {
    let svc = ready(&app)?;
    svc.set_show_online(on)?;
    Ok(svc.view())
}

// ---- bots in a space (JSON is camelCase; see `SpacesService::bot_list` for the fields) ----------------------------

#[tauri::command]
pub async fn space_bot_list(app: AppHandle, space: String) -> Result<Value, String> {
    ready(&app)?.bot_list(&space)
}

#[tauri::command]
pub async fn space_bot_candidates(app: AppHandle, space: String) -> Result<Value, String> {
    ready(&app)?.bot_candidates(&space)
}

#[tauri::command]
pub async fn space_bot_add(app: AppHandle, space: String, potato_bot: String, name: String) -> Result<Value, String> {
    ready(&app)?.bot_add(&space, &potato_bot, &name)
}

#[tauri::command]
pub async fn space_bot_remove(app: AppHandle, space: String, bot: String) -> Result<Value, String> {
    ready(&app)?.bot_remove(&space, &bot)
}

/// Space owner: bots per person (0 to 10) and the channels bots may speak in (`null`: all).
#[tauri::command]
pub async fn space_bot_rules(app: AppHandle, space: String, per_person: u32, channels: Option<Vec<String>>) -> Result<Value, String> {
    ready(&app)?.bot_rules(&space, per_person, channels)
}

/// This PC's own settings for a bot: `{ mode, memory, dayCap, silenced, muted }`, any of them.
#[tauri::command]
pub async fn space_bot_local(app: AppHandle, space: String, bot: String, patch: BotPatch) -> Result<Value, String> {
    ready(&app)?.bot_local(&space, &bot, &patch)
}
