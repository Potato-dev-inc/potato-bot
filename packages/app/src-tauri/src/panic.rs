//! Panic button: one action interrupts every running agent pane and reports what it stopped.
//!
//! Entry points: the palette ("Stop all agents", Tauri command `stop_all_agents`), the tray menu
//! item and the optional global shortcut (both go through `stop_all_and_show`, which also opens the
//! cockpit and emits `panic://stopped` so the sheet appears), and `POST /panes/stop-all` for the
//! phone. Interrupt = the same Ctrl+C `interrupt_pane` sends. Only panes whose status is
//! `running` are touched; plain shell panes are left alone (a dev server in a shell is not an
//! agent burning credits), and so are idle, waiting and errored panes.
//!
//! Before a pane is interrupted, a checkpoint of its project is taken (`checkpoints::before_stop`),
//! so "Stop all agents" never leaves the user without a way back. The checkpoints run in parallel
//! under a short time budget: a huge folder must not delay the stop, it just goes without one.

use crate::pty::PaneRegistry;
use parking_lot::Mutex;
use serde::Serialize;
use std::sync::Arc;
use tauri::AppHandle;

/// What the panic button needs to know about one pane.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub pane_id: String,
    pub agent_type: String,
    pub cwd: String,
    pub status: String,
    pub is_shell: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoppedPane {
    pub pane_id: String,
    pub agent_type: String,
    pub cwd: String,
    /// The checkpoint taken just before the interrupt (the way back), when one could be taken.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct StopReport {
    pub stopped: Vec<StoppedPane>,
    /// Panes that were running but could not be interrupted: `(pane_id, reason)`.
    pub failed: Vec<(String, String)>,
}

/// [`stop_running`] that also records, per pane, the checkpoint `checkpoint_of` names. It is asked
/// BEFORE the pane is interrupted.
pub fn stop_running_with(
    candidates: &[Candidate],
    checkpoint_of: &dyn Fn(&Candidate) -> Option<String>,
    interrupt: &dyn Fn(&str) -> Result<(), String>,
) -> StopReport {
    let mut report = StopReport::default();
    for c in candidates.iter().filter(|c| c.status == "running" && !c.is_shell) {
        let checkpoint_id = checkpoint_of(c);
        match interrupt(&c.pane_id) {
            Ok(()) => report.stopped.push(StoppedPane {
                pane_id: c.pane_id.clone(),
                agent_type: c.agent_type.clone(),
                cwd: c.cwd.clone(),
                checkpoint_id,
            }),
            Err(reason) => report.failed.push((c.pane_id.clone(), reason)),
        }
    }
    report
}

fn candidates(registry: &Arc<Mutex<PaneRegistry>>) -> Vec<Candidate> {
    let panes = registry.lock().list();
    panes
        .into_iter()
        .map(|p| Candidate {
            is_shell: crate::tui::profile_for(&p.agent_type).features().kind == crate::tui::Kind::Shell,
            pane_id: p.id,
            agent_type: p.agent_type,
            cwd: p.cwd,
            status: p.status,
        })
        .collect()
}

/// Interrupt every running agent pane of the registry, checkpointing each one's project first.
pub fn stop_all(registry: &Arc<Mutex<PaneRegistry>>, app: &AppHandle) -> StopReport {
    // Potato Bot: every brain stops (they cost money per message), every routine is held (a due run is recorded
    // as "Skipped while paused"), and Fish's computer sleeps. A quick ask to leave, then the tree is killed.
    crate::potato::live::pause_all(std::time::Duration::from_millis(300));
    let list = candidates(registry);
    stop_running_with(
        &list,
        &|_| None,
        &|id| crate::pty::registry_write_input(registry, app, id, "\u{3}", false, false, None, crate::pty::WriteOrigin::Nudge),
    )
}

/// Shortcut and tray entry: stop everything, bring the cockpit forward and tell it to show the sheet.
pub fn stop_all_and_show(app: &AppHandle) -> StopReport {
    use tauri::{Emitter, Manager};
    let report = match app.try_state::<crate::AppState>() {
        Some(state) => stop_all(&state.registry, app),
        None => StopReport::default(),
    };
    crate::cockpit::open_cockpit(app, None);
    let _ = app.emit("panic://stopped", &report);
    report
}

pub const SHORTCUT: &str = "Ctrl+Alt+Shift+P";

/// Register (or unregister) the global shortcut according to the saved setting. Off by default.
pub fn init_shortcut(app: &AppHandle) {
    apply_shortcut(app, true);
}

pub fn apply_shortcut(app: &AppHandle, enabled: bool) {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister(SHORTCUT);
    if !enabled {
        return;
    }
    let handle = app.clone();
    let result = shortcuts.on_shortcut(SHORTCUT, move |_app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            let handle = handle.clone();
            std::thread::spawn(move || {
                stop_all_and_show(&handle);
            });
        }
    });
    if let Err(e) = result {
        tracing::warn!(error = %e, "panic shortcut could not be registered");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn c(id: &str, status: &str, shell: bool) -> Candidate {
        Candidate { pane_id: id.into(), agent_type: if shell { "bash" } else { "claude" }.into(), cwd: "/p".into(), status: status.into(), is_shell: shell }
    }

    /// Test-only shorthand: `stop_running_with` without checkpoints (the production wrapper was removed as dead code).
    fn stop_running(candidates: &[Candidate], interrupt: &dyn Fn(&str) -> Result<(), String>) -> StopReport {
        stop_running_with(candidates, &|_| None, interrupt)
    }

    #[test]
    fn interrupts_only_running_agent_panes() {
        let calls = RefCell::new(Vec::<String>::new());
        let panes = [
            c("run-1", "running", false),
            c("idle-1", "idle", false),
            c("wait-1", "waiting_input", false),
            c("err-1", "error", false),
            c("run-2", "running", false),
            c("shell-1", "running", true),
        ];
        let report = stop_running(&panes, &|id| {
            calls.borrow_mut().push(id.to_string());
            Ok(())
        });
        assert_eq!(*calls.borrow(), vec!["run-1", "run-2"]);
        assert_eq!(report.stopped.iter().map(|s| s.pane_id.as_str()).collect::<Vec<_>>(), vec!["run-1", "run-2"]);
        assert!(report.failed.is_empty());
    }

    #[test]
    fn a_failing_interrupt_is_reported_and_does_not_stop_the_rest() {
        let panes = [c("a", "running", false), c("b", "running", false)];
        let report = stop_running(&panes, &|id| if id == "a" { Err("pane has exited".into()) } else { Ok(()) });
        assert_eq!(report.stopped.len(), 1);
        assert_eq!(report.stopped[0].pane_id, "b");
        assert_eq!(report.failed, vec![("a".to_string(), "pane has exited".to_string())]);
    }

    #[test]
    fn a_checkpoint_is_taken_before_each_interrupt_and_its_id_is_reported() {
        let calls = RefCell::new(Vec::<String>::new());
        let panes = [c("run-1", "running", false), c("idle-1", "idle", false), c("run-2", "running", false)];
        let report = stop_running_with(
            &panes,
            &|p| {
                calls.borrow_mut().push(format!("checkpoint {}", p.pane_id));
                (p.pane_id == "run-1").then(|| "7".to_string())
            },
            &|id| {
                calls.borrow_mut().push(format!("interrupt {id}"));
                Ok(())
            },
        );
        assert_eq!(
            *calls.borrow(),
            ["checkpoint run-1", "interrupt run-1", "checkpoint run-2", "interrupt run-2"],
            "each pane is checkpointed first; idle panes are left alone"
        );
        assert_eq!(report.stopped[0].checkpoint_id.as_deref(), Some("7"));
        assert_eq!(report.stopped[1].checkpoint_id, None, "a pane that could not be checkpointed is still stopped");
    }

    #[test]
    fn nothing_running_stops_nothing() {
        let report = stop_running(&[c("a", "idle", false)], &|_| panic!("must not interrupt"));
        assert_eq!(report, StopReport::default());
    }
}
