//! The Search chats page's one command (Potato Bot): words in every bot's chat and every group's thread, newest first.
//! Desktop only, like the rest of Potato Bot; it reads and writes nothing. It runs off the UI thread, so a long history
//! never freezes the window. Errors are one string, `CODE: plain sentence`.

use crate::potato::live;
use crate::potato::search::Results;

#[tauri::command]
pub async fn potato_search_chats(query: String) -> Result<Results, String> {
    let service = live::service().ok_or_else(|| "POTATO_NOT_READY: Potato Bot is not ready yet. Try again in a moment.".to_string())?;
    tauri::async_runtime::spawn_blocking(move || service.search_chats(&query).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("INTERNAL: {e}"))?
}
