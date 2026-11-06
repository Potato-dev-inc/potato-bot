//! One-click update (Tauri updater v2). The page asks `app_update_check`; a press on Update calls `app_update_install`.
//!
//! - Nothing is installed without a press. A check only reads `latest.json` (the GitHub release) and keeps the answer.
//! - The download is checked against the public key in `tauri.conf.json` before anything else happens; a file that does not
//!   verify is refused and the app goes on as it was.
//! - If a bot's job is running, the install is refused until the page says the person was warned (`confirm_running`).
//! - Only then the brains are asked to leave (a job is saved on disk and picks up again after the restart), the sandbox's
//!   holder is released, and the installer runs. The installer is the MSI of the next version: same upgrade code, so it
//!   replaces the old one in place. The app's data folder and the sandbox distro are not touched by it.
//! - Any failure is a plain sentence; the version that is running keeps working.

use std::sync::Mutex;

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};

static FOUND: Mutex<Option<Update>> = Mutex::new(None);

#[derive(Serialize, Debug, PartialEq, Eq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOffer {
    pub current_version: String,
    pub version: String,
    pub notes: Option<String>,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBusy {
    /// Names of the bots whose job is running.
    pub bots: Vec<String>,
}

/// A plain sentence for what went wrong (the page shows it as it is). Pure, so tests can pin the words.
pub fn plain_update_error(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("signature") || lower.contains("minisign") || lower.contains("verif") {
        "The update did not pass its safety check, so it was not installed. Your current version keeps working.".to_string()
    } else if lower.contains("dns") || lower.contains("connect") || lower.contains("network") || lower.contains("timed out") || lower.contains("error sending request") || lower.contains("unreachable") {
        "Could not reach the update server. Check your internet and try again. Your current version keeps working.".to_string()
    } else if lower.contains("404") || lower.contains("not found") || lower.contains("status") {
        "No update information was found right now. Try again later.".to_string()
    } else {
        "The update could not be installed. Your current version keeps working.".to_string()
    }
}

/// Bots whose job is running now (read from the saved job files).
pub fn running_job_bots(root: &std::path::Path) -> Vec<String> {
    crate::potato::store::load_bots(root)
        .unwrap_or_default()
        .into_iter()
        .filter(|bot| crate::potato::jobs::load(root, &bot.id).is_some_and(|job| job.is_running()))
        .map(|bot| bot.id)
        .collect()
}

#[tauri::command]
pub async fn app_update_check(app: AppHandle) -> Result<Option<UpdateOffer>, String> {
    let updater = app.updater().map_err(|e| plain_update_error(&e.to_string()))?;
    match updater.check().await {
        Ok(Some(update)) => {
            let offer = UpdateOffer { current_version: update.current_version.clone(), version: update.version.clone(), notes: update.body.clone().filter(|n| !n.trim().is_empty()) };
            *FOUND.lock().unwrap_or_else(|p| p.into_inner()) = Some(update);
            Ok(Some(offer))
        }
        Ok(None) => {
            *FOUND.lock().unwrap_or_else(|p| p.into_inner()) = None;
            Ok(None)
        }
        Err(e) => Err(plain_update_error(&e.to_string())),
    }
}

/// Which bots have a running job: the page warns before it asks to install.
#[tauri::command]
pub fn app_update_busy() -> UpdateBusy {
    UpdateBusy { bots: running_job_bots(&crate::potato::live::root()) }
}

/// The press on Update: download and verify, then stop the brains and the sandbox's holder cleanly, then install.
#[tauri::command]
pub async fn app_update_install(app: AppHandle, confirm_running: bool) -> Result<(), String> {
    let busy = running_job_bots(&crate::potato::live::root());
    if !busy.is_empty() && !confirm_running {
        return Err(format!("UPDATE_JOB_RUNNING: {} is working on a job. Confirm to update anyway; the job is saved and carries on after the restart.", busy.join(", ")));
    }
    let update = FOUND.lock().unwrap_or_else(|p| p.into_inner()).take();
    let update = match update {
        Some(update) => update,
        None => {
            let updater = app.updater().map_err(|e| plain_update_error(&e.to_string()))?;
            updater.check().await.map_err(|e| plain_update_error(&e.to_string()))?.ok_or_else(|| "There is no update to install.".to_string())?
        }
    };
    // Download first and check the signature; nothing has been stopped yet, so a failure here changes nothing.
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| plain_update_error(&e.to_string()))?;
    stop_for_update(&app);
    update.install(bytes).map_err(|e| plain_update_error(&e.to_string()))?;
    // On Windows the installer ends this process; if it is still here, it is not going to run.
    Ok(())
}

/// What the app's own exit does for the bots and the sandbox, done before the installer closes the app (it ends the
/// process without the usual exit event).
fn stop_for_update(app: &AppHandle) {
    use tauri::Manager;
    crate::standby::shutdown();
    crate::potato::live::stop_all(std::time::Duration::from_secs(2));
    crate::potato::sandbox_host::sleep();
    if let Some(state) = app.try_state::<crate::commands::AppState>() {
        crate::pty::registry_kill_all_and_wait(&state.registry);
    }
}

/// With `PM_UPDATE_PROBE=<file>` set: at start-up, check, download and verify (never install) and write what happened
/// to the file. It is how a build is shown to find, fetch and verify an update; off unless the variable is set.
pub fn startup_probe(app: AppHandle) {
    let Some(file) = std::env::var_os("PM_UPDATE_PROBE").filter(|f| !f.is_empty()) else { return };
    tauri::async_runtime::spawn(async move {
        let report = match app_update_check(app.clone()).await {
            Ok(Some(offer)) => {
                let found = FOUND.lock().unwrap_or_else(|p| p.into_inner()).take();
                match found {
                    Some(update) => match update.download(|_, _| {}, || {}).await {
                        Ok(bytes) => serde_json::json!({ "result": "verified", "current": offer.current_version, "version": offer.version, "notes": offer.notes, "bytes": bytes.len() }),
                        Err(e) => serde_json::json!({ "result": "refused", "version": offer.version, "plain": plain_update_error(&e.to_string()), "raw": e.to_string() }),
                    },
                    None => serde_json::json!({ "result": "no update kept" }),
                }
            }
            Ok(None) => serde_json::json!({ "result": "none" }),
            Err(plain) => serde_json::json!({ "result": "check failed", "plain": plain }),
        };
        let _ = std::fs::write(file, report.to_string());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bad_signature_is_said_as_a_safety_check_and_nothing_is_installed() {
        let s = plain_update_error("the signature could not be verified: Invalid signature");
        assert!(s.contains("safety check") && s.contains("keeps working"), "{s}");
    }

    #[test]
    fn a_network_failure_is_said_in_plain_words() {
        for raw in ["error sending request for url (https://github.com/x): dns error", "operation timed out", "Could not connect"] {
            assert!(plain_update_error(raw).contains("Could not reach the update server"), "{raw}");
        }
    }

    #[test]
    fn any_other_failure_keeps_the_old_version_working() {
        assert!(plain_update_error("weird").contains("keeps working"));
        assert!(plain_update_error("HTTP status client error (404 Not Found)").contains("No update information"));
    }

    #[test]
    fn the_raw_error_text_is_never_shown() {
        let s = plain_update_error("C:\\Users\\someone\\secret path failed: signature mismatch");
        assert!(!s.contains("someone") && !s.contains("C:"), "{s}");
    }

    #[test]
    fn a_running_job_is_found_in_the_saved_files_and_a_finished_one_is_not() {
        let dir = std::env::temp_dir().join(format!("pm-update-jobs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(running_job_bots(&dir).is_empty(), "no bots, no jobs");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
