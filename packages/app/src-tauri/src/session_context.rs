use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DEFAULT_STANDBY_POLL_MS: u64 = 1_500;
pub const DEFAULT_STANDBY_MAX_MS: u64 = 10 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneRole {
    Implementer,
    Reviewer,
    Shell,
    Orchestrator,
    Observer,
}

impl PaneRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Implementer => "implementer",
            Self::Reviewer => "reviewer",
            Self::Shell => "shell",
            Self::Orchestrator => "orchestrator",
            Self::Observer => "observer",
        }
    }

    #[allow(dead_code)]
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "implementer" => Ok(Self::Implementer),
            "reviewer" => Ok(Self::Reviewer),
            "shell" => Ok(Self::Shell),
            "orchestrator" => Ok(Self::Orchestrator),
            "observer" => Ok(Self::Observer),
            _ => Err(format!("invalid pane role: {value}")),
        }
    }
}

#[allow(dead_code)]
pub fn allowed_pane_roles() -> Vec<PaneRole> {
    vec![
        PaneRole::Implementer,
        PaneRole::Reviewer,
        PaneRole::Shell,
        PaneRole::Orchestrator,
        PaneRole::Observer,
    ]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneDigest {
    pub pane_id: String,
    pub summary: String,
    pub source: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTimelineEvent {
    pub timestamp_ms: i64,
    pub actor: String,
    pub event_type: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockConflictProjection {
    pub resource_id: String,
    pub requested_owner_id: String,
    pub existing_owner_id: String,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestratorStateProjection {
    pub standby_poll_ms: u64,
    pub standby_max_ms: u64,
}

impl Default for OrchestratorStateProjection {
    fn default() -> Self {
        Self {
            standby_poll_ms: DEFAULT_STANDBY_POLL_MS,
            standby_max_ms: DEFAULT_STANDBY_MAX_MS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionContextProjection {
    pub current_goal: Option<String>,
    pub pane_roles: BTreeMap<String, PaneRole>,
    pub pane_digests: BTreeMap<String, PaneDigest>,
    pub timeline: Vec<SessionTimelineEvent>,
    pub lock_conflicts: Vec<LockConflictProjection>,
    pub orchestrator: OrchestratorStateProjection,
}

impl Default for SessionContextProjection {
    fn default() -> Self {
        Self {
            current_goal: None,
            pane_roles: BTreeMap::new(),
            pane_digests: BTreeMap::new(),
            timeline: Vec::new(),
            lock_conflicts: Vec::new(),
            orchestrator: OrchestratorStateProjection::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_session_context_current_goal() {
        let context = SessionContextProjection {
            current_goal: Some("Finish Rust migration".to_string()),
            ..SessionContextProjection::default()
        };
        let value = serde_json::to_value(context).unwrap();
        assert_eq!(value["current_goal"], json!("Finish Rust migration"));
    }

    #[test]
    fn serializes_each_allowed_pane_role() {
        let roles = allowed_pane_roles();
        let values = roles
            .iter()
            .map(|role| serde_json::to_value(role).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            values,
            vec![
                json!("implementer"),
                json!("reviewer"),
                json!("shell"),
                json!("orchestrator"),
                json!("observer")
            ]
        );
    }

    #[test]
    fn rejects_invalid_pane_role() {
        assert!(PaneRole::parse("driver").is_err());
    }

    #[test]
    fn serializes_pane_digest() {
        let digest = PaneDigest {
            pane_id: "pane-1".to_string(),
            summary: "Tests are running".to_string(),
            source: "manual".to_string(),
            updated_at_ms: 42,
        };
        let value = serde_json::to_value(digest).unwrap();
        assert_eq!(value["summary"], json!("Tests are running"));
    }

    #[test]
    fn default_orchestrator_state_matches_sidebar_policy() {
        let state = OrchestratorStateProjection::default();
        assert_eq!(state.standby_poll_ms, 1_500);
        assert_eq!(state.standby_max_ms, 600_000);
    }
}
