//! Saving a chat page the app already built (the bot's "Export chat" path).

use super::*;

#[tauri::command]
pub async fn chat_export_save(chat: State<'_, SessionChatState>, token: String, path: String) -> Result<String, String> {
    chat.pages()
        .save(&token, &path)
        .map(|path| path.display().to_string())
        .map_err(|error| error.to_string())
}
