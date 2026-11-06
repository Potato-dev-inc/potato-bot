//! Tauri commands of Potato Bot (spec sections 2, 3 and 5). Desktop only: the page, the settings and the
//! brain's start and stop are never reachable through the bridge, the phone or MCP. The brain itself
//! reaches the app only through `/potato/brain/*` (`potato::auth`).
//!
//! Errors are one string, `CODE: plain sentence`, like the companion commands. The key is set through
//! `secret_set("deepseek_api_key", ..)` (masked reads only); nothing here returns it. `potato://changed`
//! (payload `{ botId }`) tells the page to read again.

use std::sync::Arc;

use serde_json::Value;
use tauri::{AppHandle, State};

use crate::potato::helpers::HelperStartOutcome;
use crate::potato::live;
use crate::potato::cards::Card;
use crate::potato::service::{AnswerOutcome, ForgetOutcome, LoginOrigin, PotatoState, SendOutcome, Service};
use crate::potato::store::{Bot, Message, Settings, StoreError};
use super::AppState;

fn service() -> Result<Arc<Service>, String> {
    live::service().ok_or_else(|| "POTATO_NOT_READY: Potato Bot is not ready yet. Try again in a moment.".to_string())
}

fn text(error: StoreError) -> String {
    error.to_string()
}

/// Runs one service call off the UI thread (a send may start the brain; a stop waits for it to leave).
async fn run<T: Send + 'static>(call: impl FnOnce(&Service) -> Result<T, StoreError> + Send + 'static) -> Result<T, String> {
    let service = service()?;
    tauri::async_runtime::spawn_blocking(move || call(&service).map_err(text)).await.map_err(|e| format!("INTERNAL: {e}"))?
}

#[tauri::command]
pub async fn potato_state() -> Result<PotatoState, String> {
    run(|s| s.state()).await
}

#[tauri::command]
pub async fn potato_messages(bot_id: String, limit: Option<usize>) -> Result<Vec<Message>, String> {
    run(move |s| s.messages(&bot_id, limit.unwrap_or(200))).await
}

/// One message from the person. Saved first; then handed to the bot's brain, which starts if it must.
#[tauri::command]
pub async fn potato_send(bot_id: String, text: String) -> Result<SendOutcome, String> {
    run(move |s| s.send(&bot_id, &text)).await
}

/// Turns a companion into a Potato Bot (the persona is the Settings default).
#[tauri::command]
pub async fn potato_bot_add(bot_id: String) -> Result<Bot, String> {
    run(move |s| s.add_bot(&bot_id)).await
}

#[tauri::command]
pub async fn potato_set_persona(bot_id: String, pack: String) -> Result<Bot, String> {
    run(move |s| s.set_persona(&bot_id, &pack)).await
}

/// "Start again": forgets the crashes and starts the brain.
#[tauri::command]
pub async fn potato_bot_start(bot_id: String) -> Result<(), String> {
    run(move |s| s.start_bot(&bot_id)).await
}

/// The page's Stop button: asks the brain to leave. The next message starts it again.
#[tauri::command]
pub async fn potato_bot_stop(bot_id: String) -> Result<(), String> {
    run(move |s| {
        s.stop_bot_by_person(&bot_id);
        Ok(())
    })
    .await
}

/// The list's Archive and Restore: hides a bot's chat or brings it back. Nothing is deleted.
#[tauri::command]
pub async fn potato_bot_archive(bot_id: String, archived: bool) -> Result<(), String> {
    run(move |s| s.set_archived(&bot_id, archived)).await
}

#[tauri::command]
pub async fn potato_mark_read(bot_id: String) -> Result<(), String> {
    run(move |s| s.mark_read(&bot_id)).await
}

/// The owner reacts to a message (or takes the reaction back). Only the app calls this; the bot has no such tool.
#[tauri::command]
pub async fn potato_toggle_reaction(bot_id: String, message_id: String, emoji: String) -> Result<Vec<String>, String> {
    run(move |s| s.toggle_reaction(&bot_id, &message_id, &emoji)).await
}

/// "Forget talk memory": the brain's folder is moved aside, never deleted.
#[tauri::command]
pub async fn potato_forget_talk_memory(bot_id: String) -> Result<(), String> {
    run(move |s| s.forget_talk_memory(&bot_id)).await
}

#[tauri::command]
pub async fn potato_settings_get() -> Result<Settings, String> {
    run(|s| s.settings()).await
}

#[tauri::command]
pub async fn potato_settings_set(settings: Settings) -> Result<Settings, String> {
    run(move |s| s.set_settings(settings)).await
}

/// The cleaned lines of the brain's log, for the "why did it stop" view. Never message text or a key.
#[tauri::command]
pub async fn potato_log(bot_id: String) -> Result<Vec<String>, String> {
    Ok(service()?.log_lines(&bot_id))
}

/// "Change folder": the names of the recent projects a helper may work in. Names only, never paths.
#[tauri::command]
pub async fn potato_helper_folders(bot_id: String) -> Result<Vec<String>, String> {
    run(move |s| s.helper_folders(&bot_id)).await
}

/// Start on a helper card. `folder` is a project NAME (from "Change folder"); without it the card's own folder
/// is used. Nothing runs without this call, and the call needs an open card.
#[tauri::command]
pub async fn potato_helper_start(bot_id: String, request_id: String, folder: Option<String>) -> Result<HelperStartOutcome, String> {
    run(move |s| s.helper_start(&bot_id, &request_id, folder.as_deref())).await
}

/// Not now on a helper card.
#[tauri::command]
pub async fn potato_helper_decline(bot_id: String, request_id: String) -> Result<(), String> {
    run(move |s| s.helper_decline(&bot_id, &request_id)).await
}

/// A button on a decision card (an approval, a question, a Keep going). `answer`: `allow_once`, `allow_job` or `deny` for an
/// approval, a choice number from 0 for a question. One answer only: a second one, an answer to a cancelled card or to a
/// card id that does not exist is an error. This command is the only way a card is answered.
#[tauri::command]
pub async fn potato_answer_card(bot_id: String, card_id: String, answer: String) -> Result<AnswerOutcome, String> {
    run(move |s| s.answer_card(&bot_id, &card_id, &answer)).await
}

/// "Always allow" on an approval card. The one command that saves a rule: this bot may run commands of the card's one class
/// without asking again. Refused for the destroying classes (rm, force-push), for a card with two classes and for a
/// question. Nothing the bot can call reaches this; the plain `potato_answer_card` refuses the same words.
#[tauri::command]
pub async fn potato_answer_card_always(bot_id: String, card_id: String) -> Result<AnswerOutcome, String> {
    run(move |s| s.answer_card_always(&bot_id, &card_id)).await
}

/// "Sign in" on a sign-in card. The user name and the password come from the card's two fields and go to the page in the
/// bot's browser through the host; the card is answered only if that worked. Nothing here logs, stores or returns either one.
/// The password is wrapped in a wiped-on-drop holder the moment the service gets it.
#[tauri::command]
pub async fn potato_submit_login(bot_id: String, card_id: String, username: String, password: String) -> Result<AnswerOutcome, String> {
    run(move |s| s.submit_login(&bot_id, &card_id, username, password)).await
}

/// "It worked, continue" on a sign-in card the page tool could not read: the person saw the page signed in.
#[tauri::command]
pub async fn potato_confirm_login(bot_id: String, card_id: String) -> Result<AnswerOutcome, String> {
    run(move |s| s.confirm_login(&bot_id, &card_id)).await
}

/// The origin the bot's browser really has in front, for a sign-in card to show beside the site the bot named.
#[tauri::command]
pub async fn potato_login_origin(bot_id: String, card_id: String) -> Result<LoginOrigin, String> {
    run(move |s| s.login_origin(&bot_id, &card_id)).await
}

/// "Sign out" on a sign-in card: that site's cookies and stored data leave the bot's browser.
#[tauri::command]
pub async fn potato_forget_site(bot_id: String, card_id: String) -> Result<ForgetOutcome, String> {
    run(move |s| s.forget_site(&bot_id, &card_id)).await
}

/// The bot's job for the job bar: goal, step, start, spend and state. `null` when it has none.
#[tauri::command]
pub async fn potato_job_get(bot_id: String) -> Result<Option<crate::potato::jobs::View>, String> {
    run(move |s| s.job_view(&bot_id)).await
}

/// The cards of a bot that still wait for an answer, oldest first.
#[tauri::command]
pub async fn potato_open_cards(bot_id: String) -> Result<Vec<Card>, String> {
    run(move |s| s.open_cards(&bot_id)).await
}

/// Settings > Potato Bot > Check key: one tiny turn. Answers "Works." or one plain sentence. The key is read
/// inside the app and goes to the model maker only.
#[tauri::command]
pub async fn potato_check_key() -> Result<String, String> {
    run(|s| s.check_key()).await
}

/// "Install Node.js" on the card: a visible PowerShell window the person watches.
#[tauri::command(async)]
pub fn potato_install_node(_state: State<'_, AppState>, _app: AppHandle) -> Result<Value, String> {
    #[cfg(windows)]
    {
        // Visible on purpose: the person answers winget's prompts there.
        let mut command = crate::proc_util::visible_command("powershell.exe");
        command.args([
            "-NoLogo",
            "-NoExit",
            "-Command",
            "winget install --id OpenJS.NodeJS.LTS -e --accept-package-agreements --accept-source-agreements",
        ]);
        command
            .spawn()
            .map_err(|e| format!("NODE_INSTALL_FAILED: {e}"))?;
        Ok(serde_json::json!({ "started": true }))
    }
    #[cfg(not(windows))]
    {
        Err("NODE_INSTALL_UNSUPPORTED: install Node.js 20 or newer with your package manager.".to_string())
    }
}

/// The files the bot's job changed with its file tools, each with Restore. `null` when the bot has no job.
#[tauri::command]
pub async fn potato_job_files(bot_id: String) -> Result<Option<crate::potato::file_undo::View>, String> {
    run(move |s| s.job_files(&bot_id)).await
}

/// Restore on one file of a job: the old content goes back (or a file the job made is removed). The owner's click only.
#[tauri::command]
pub async fn potato_job_file_restore(bot_id: String, job_id: String, file_id: String) -> Result<crate::potato::file_undo::Entry, String> {
    run(move |s| s.restore_job_file(&bot_id, &job_id, &file_id)).await
}
