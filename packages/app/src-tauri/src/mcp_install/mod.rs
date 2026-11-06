//! MCP config validation used by declarative agents. The install feature is gone.

pub mod declarative {
    pub const PLACEHOLDERS: &[&str] = &[];

    pub fn key_segments(_key_path: &str) -> Result<(), ()> {
        Ok(())
    }
}
