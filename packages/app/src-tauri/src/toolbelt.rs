//! MCP host locations declared by TUI profiles. The install feature is gone.

pub fn host_from_toml(_text: &str) -> Option<crate::tui::declarative::schema::McpHostTable> {
    None
}

pub mod store {
    pub fn secret_values() -> Vec<String> {
        Vec::new()
    }

    pub fn migrate_on_startup() {}
}
