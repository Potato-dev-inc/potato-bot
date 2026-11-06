//! Agent registry listing for the bot's companion and helper pickers.

use super::*;

#[tauri::command]
pub fn agents_list() -> Value {
    serde_json::to_value(crate::agent_run_mode::decorate_listing(crate::tui::agent_types_listing()))
        .unwrap_or(Value::Null)
}
