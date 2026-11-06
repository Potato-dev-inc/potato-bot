//! Tauri commands of the Files section of the Computer tab (Potato Bot): browse the bot's home folder, look at a file,
//! download a file to Windows, upload a file from Windows. Desktop only; the work is `potato::files_browser`, which talks to the
//! computer only through the one fixed files script. Nothing here is reachable by a bot.
//!
//! Errors are one string, `CODE: plain sentence`, like the other Potato Bot commands. Downloads take the path the person picked
//! in the native save dialog, uploads the path they picked in the open dialog. Every call runs off the UI thread.

use std::path::Path;
use std::sync::Arc;

use crate::potato::files_browser::{self, FilePreview, Listing};
use crate::potato::sandbox::Sandbox;
use crate::potato::store::{self, StoreError};
use crate::potato::{live, sandbox_host};

async fn blocking<T: Send + 'static>(bot_id: String, call: impl FnOnce(Arc<dyn Sandbox>) -> Result<T, StoreError> + Send + 'static) -> Result<T, String> {
    let sandbox = sandbox_host::manager()?.sandbox().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if !store::load_bots(&live::root()).map_err(|e| e.to_string())?.iter().any(|b| b.id == bot_id) {
            return Err("POTATO_BOT_NOT_FOUND: That bot is not known.".to_string());
        }
        call(sandbox).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("INTERNAL: {e}"))?
}

/// The folder at `path` (absent or empty: the bot's home).
#[tauri::command]
pub async fn potato_files_list(bot_id: String, path: Option<String>) -> Result<Listing, String> {
    blocking(bot_id, move |sandbox| files_browser::list(sandbox.as_ref(), path.as_deref().unwrap_or(""))).await
}

/// The start of a text file, a small picture, or just the name and size.
#[tauri::command]
pub async fn potato_files_preview(bot_id: String, path: String) -> Result<FilePreview, String> {
    blocking(bot_id, move |sandbox| files_browser::preview(sandbox.as_ref(), &path)).await
}

/// Download: copies the file to `dest` (a path from the save dialog). Answers the number of bytes saved.
#[tauri::command]
pub async fn potato_files_download(bot_id: String, path: String, dest: String) -> Result<u64, String> {
    blocking(bot_id, move |sandbox| {
        let (_, bytes) = files_browser::download(sandbox.as_ref(), &path)?;
        files_browser::save_download(Path::new(&dest), &bytes)?;
        Ok(bytes.len() as u64)
    })
    .await
}

/// Upload: puts the file at `src` (a path from the open dialog) in the folder `dir` of the computer. Answers where it is.
#[tauri::command]
pub async fn potato_files_upload(bot_id: String, dir: String, src: String) -> Result<String, String> {
    blocking(bot_id, move |sandbox| {
        let (name, bytes) = files_browser::read_upload(Path::new(&src))?;
        files_browser::upload(sandbox.as_ref(), &dir, &name, bytes)
    })
    .await
}
