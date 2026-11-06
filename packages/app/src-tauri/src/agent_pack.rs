//! MCP host and skill locations declared by TUI profiles. The installers are gone; the profile
//! trait keeps the hooks so a profile can still declare a table, and this build returns none.

pub struct SkillsHostTable;

pub fn skills_host_from_toml(_text: &str) -> Option<SkillsHostTable> {
    None
}
