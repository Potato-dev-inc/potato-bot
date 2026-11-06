//! Recent-event queries over the global log: filtered summaries and a pane's last event.

use crate::events::{PaneId, SystemEvent};
use crate::opencode::status::LastEventSummary;

pub fn last_pane_event_summary(pane_id: &str) -> Option<LastEventSummary> {
    // Scans from the end of the log: every worker-status call asks this, and the log only grows.
    let entry = super::find_last_entry(
        |entry| matches!(event_pane_id(&entry.payload), Some(id) if id.0 == pane_id),
    )
    .ok()??;
    Some(LastEventSummary {
        event_type: event_type_name(&entry.payload).to_string(),
        timestamp_ms: entry.timestamp_ms,
    })
}

fn event_pane_id(payload: &SystemEvent) -> Option<PaneId> {
    match payload {
        SystemEvent::PaneSpawned { pane_id, .. }
        | SystemEvent::PaneKilled { pane_id, .. }
        | SystemEvent::PaneInputWritten { pane_id, .. }
        | SystemEvent::PaneOutputObserved { pane_id, .. }
        | SystemEvent::PaneStatusChanged { pane_id, .. }
        | SystemEvent::PaneRoleSet { pane_id, .. }
        | SystemEvent::PaneDigestUpdated { pane_id, .. }
        | SystemEvent::AgentObservation { pane_id, .. }
        | SystemEvent::OpenCodeKeySwap { pane_id, .. }
        | SystemEvent::PaneModelSwitched { pane_id, .. }
        | SystemEvent::PaneTuiReattached { pane_id, .. } => Some(pane_id.clone()),
        SystemEvent::DelegationPrepared { target_pane_id, .. } => target_pane_id.clone(),
        _ => None,
    }
}

pub(crate) fn event_type_name(payload: &SystemEvent) -> &'static str {
    match payload {
        SystemEvent::PaneSpawned { .. } => "PaneSpawned",
        SystemEvent::PaneKilled { .. } => "PaneKilled",
        SystemEvent::PaneInputWritten { .. } => "PaneInputWritten",
        SystemEvent::PaneOutputObserved { .. } => "PaneOutputObserved",
        SystemEvent::PaneStatusChanged { .. } => "PaneStatusChanged",
        SystemEvent::McpToolCalled { .. } => "McpToolCalled",
        SystemEvent::McpToolCompleted { .. } => "McpToolCompleted",
        SystemEvent::TaskCreated { .. } => "TaskCreated",
        SystemEvent::TaskClaimed { .. } => "TaskClaimed",
        SystemEvent::TaskLeaseRenewed { .. } => "TaskLeaseRenewed",
        SystemEvent::TaskStatusUpdated { .. } => "TaskStatusUpdated",
        SystemEvent::TaskCompleted { .. } => "TaskCompleted",
        SystemEvent::TaskBlocked { .. } => "TaskBlocked",
        SystemEvent::ReviewerAssigned { .. } => "ReviewerAssigned",
        SystemEvent::ResourceLockAcquired { .. } => "ResourceLockAcquired",
        SystemEvent::ResourceLockConflict { .. } => "ResourceLockConflict",
        SystemEvent::ResourceLockReleased { .. } => "ResourceLockReleased",
        SystemEvent::ResourceLockExpired { .. } => "ResourceLockExpired",
        SystemEvent::AgentObservation { .. } => "AgentObservation",
        SystemEvent::SessionGoalUpdated { .. } => "SessionGoalUpdated",
        SystemEvent::PaneRoleSet { .. } => "PaneRoleSet",
        SystemEvent::PaneDigestUpdated { .. } => "PaneDigestUpdated",
        SystemEvent::DelegationPrepared { .. } => "DelegationPrepared",
        SystemEvent::OrchestratorStandbyPolicyUpdated { .. } => "OrchestratorStandbyPolicyUpdated",
        SystemEvent::OpenCodeKeySwap { .. } => "OpenCodeKeySwap",
        SystemEvent::PaneModelSwitched { .. } => "PaneModelSwitched",
        SystemEvent::PaneTuiReattached { .. } => "PaneTuiReattached",
    }
}
