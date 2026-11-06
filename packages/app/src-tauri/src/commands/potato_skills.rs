//! Tauri commands of the bot panel's Skills tab (Potato Bot): list, edit and delete a bot's skills. Desktop only, like the
//! rest of Potato Bot; none of these is reachable through the bridge, the phone or MCP, and the bot cannot call them. The
//! store is `potato::skills`. Errors are one string, `CODE: plain sentence`. Each command runs off the UI thread.

use crate::potato::live;
use crate::potato::skills::{self, Skill};
use crate::potato::store::StoreError;

async fn run<T: Send + 'static>(call: impl FnOnce(&std::path::Path) -> Result<T, StoreError> + Send + 'static) -> Result<T, String> {
    let service = live::service().ok_or_else(|| "POTATO_NOT_READY: Potato Bot is not ready yet. Try again in a moment.".to_string())?;
    let root = service.root().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || call(&root).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("INTERNAL: {e}"))?
}

/// The bot's skills, by name, each with its text.
#[tauri::command]
pub async fn potato_skill_list(bot_id: String) -> Result<Vec<Skill>, String> {
    run(move |root| skills::list(root, &bot_id)).await
}

/// Changes a skill's description and steps. The name stays.
#[tauri::command]
pub async fn potato_skill_save(bot_id: String, slug: String, description: String, body: String) -> Result<Skill, String> {
    run(move |root| skills::save(root, &bot_id, &slug, &description, &body)).await
}

/// Deletes a skill: the file moves to the removed folder, nothing is erased. The page asks first.
#[tauri::command]
pub async fn potato_skill_delete(bot_id: String, slug: String) -> Result<Skill, String> {
    run(move |root| skills::delete(root, &bot_id, &slug)).await
}
