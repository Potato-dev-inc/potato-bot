//! App-level commands for the Potato Bot shell.

use super::*;

/// Whether this build shows every Labs switch as on (a development build).
#[tauri::command]
pub fn dev_labs_override_active() -> bool {
    settings_store::dev_labs_override_active()
}

/// The bridge address of this app profile, when it wrote a port file.
#[tauri::command]
pub fn profile_bridge_url() -> Option<String> {
    crate::app_paths::profile()?;
    let text = std::fs::read_to_string(crate::app_paths::bridge_port_file()).ok()?;
    crate::bridge::port_file::parse_port(&text).map(|port| format!("http://127.0.0.1:{port}"))
}
