//! Capability notes a TUI profile can declare. The full agent-context settings surface is gone;
//! the trait keeps this small record for the built-in and declarative profiles.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentCapability {
    CodebaseReasoning,
    Implementation,
    Review,
    Debugging,
    TerminalOps,
    UiOrchestration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelDetection {
    Unknown,
    CliBanner,
    Configuration,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentContextProfile {
    pub agent_type: crate::tui::AgentId,
    pub label: String,
    pub default_model: Option<String>,
    pub model_detection: ModelDetection,
    pub smartness: u8,
    pub strengths: Vec<AgentCapability>,
    pub context_notes: Vec<String>,
    pub best_for: Vec<String>,
    pub planned_sidebar_actions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ModelRef {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct AgentModelInspection {
    pub last_user_model: Option<ModelRef>,
    pub session_model: Option<ModelRef>,
    pub detected_model: Option<String>,
}

/// No built-in table in this build: declarative profiles still describe themselves.
pub fn context_profile(_id: &str) -> Option<AgentContextProfile> {
    None
}

pub fn registered_context_agent(id: &str) -> Result<Option<String>, ()> {
    Ok(Some(id.to_string()))
}

pub fn inspect_agent_model_with_registry(
    _registry: Option<&std::sync::Arc<parking_lot::Mutex<crate::pty::PaneRegistry>>>,
    _pane_id: &str,
    _agent: String,
    _buffer: &str,
) -> AgentModelInspection {
    AgentModelInspection::default()
}

/// The label people read for a built-in agent ("Claude", never the brand name "Claude Code").
fn builtin_label(agent: crate::pty::agents::AgentType) -> &'static str {
    use crate::pty::agents::AgentType;
    match agent {
        AgentType::Claude => "Claude",
        AgentType::Codex => "Codex",
        AgentType::Opencode => "OpenCode",
        AgentType::OpencodeNative => "OpenCode (API)",
        AgentType::Cmd => "Command Prompt",
        AgentType::Powershell => "PowerShell",
        AgentType::Bash => "Bash",
        AgentType::Cursor => "Cursor",
        AgentType::CursorAgent => "Cursor Agent",
    }
}

pub fn get_agent_context_profile(agent: crate::pty::agents::AgentType) -> &'static AgentContextProfile {
    static PROFILES: std::sync::Mutex<Vec<(crate::pty::agents::AgentType, &'static AgentContextProfile)>> = std::sync::Mutex::new(Vec::new());
    let mut known = PROFILES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((_, profile)) = known.iter().find(|(kind, _)| *kind == agent) {
        return profile;
    }
    // At most one small record per built-in agent is ever made, so it lives for the whole run.
    let profile: &'static AgentContextProfile = Box::leak(Box::new(AgentContextProfile {
        agent_type: crate::tui::AgentId::new("agent"),
        label: builtin_label(agent).to_string(),
        default_model: None,
        model_detection: ModelDetection::Unknown,
        smartness: 5,
        strengths: Vec::new(),
        context_notes: Vec::new(),
        best_for: Vec::new(),
        planned_sidebar_actions: Vec::new(),
    }));
    known.push((agent, profile));
    profile
}
