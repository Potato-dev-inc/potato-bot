//! Loop and burn detector: "this agent seems stuck".
//!
//! Consumes what the activity ticker (`pane_activity`) already reads once a second: the visible
//! screen tail of a pane whose screen or status changed, plus its status. Nothing runs in the PTY
//! reader and nothing runs for a silent pane. From the stream of tails it derives per-turn
//! records (a turn is `running` -> `idle`/`error`) and looks for four agent-agnostic signals:
//!
//! * `repeated_error`: the same error-like line (near-duplicate, numbers/paths/timestamps ignored)
//!   in N consecutive turns;
//! * `repeated_action`: the same tool/action lines in the same order for N consecutive turns,
//!   with no progress marker anywhere in them;
//! * `no_progress`: 2N turns inside ten minutes, each doing tool work or hitting errors, none
//!   showing a progress marker;
//! * `stalled`: a pane `running` continuously for X minutes without a single new distinct line.
//!
//! Only lines that are NEW since the previous observation count (a multiset difference of
//! normalised lines), so an old error that merely stays on screen never repeats. A signal is
//! raised once per episode (hysteresis): it re-arms only after the detection has cleared.
//! Generic code never matches agent names; error-likeness comes from the profile
//! (`TuiProfile::is_error_line`: a generic default plus TOML `[tui] error_regex`).
//!
//! Near-duplicate matching uses the `strsim` crate (MIT).

use crate::pty::status::PaneStatus;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LoopKind {
    RepeatedError,
    RepeatedAction,
    NoProgress,
    Stalled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoopSignal {
    pub pane_id: String,
    pub kind: LoopKind,
    /// At most three screen lines that show the problem.
    pub evidence_lines: Vec<String>,
    /// Turns involved (for `stalled`: minutes without new output).
    pub turns: u32,
    /// Epoch milliseconds when the pattern started.
    pub since: u64,
    /// Plain sentence tail for the card: "the same error 4 times".
    pub summary: String,
}

/// What the guard tells the outside world.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Event {
    Raised(LoopSignal),
    Cleared(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    /// Consecutive turns before a repeat counts (conservative default 4; clamped 3..=10).
    pub turns: u32,
    /// Minutes of `running` without new output before `stalled` fires (clamped 5..=240).
    pub stalled_minutes: u32,
    /// Register the global panic shortcut (Ctrl+Alt+Shift+P). Off by default.
    pub panic_shortcut: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { enabled: true, turns: 4, stalled_minutes: 20, panic_shortcut: false }
    }
}

impl Settings {
    pub fn clamped(mut self) -> Self {
        self.turns = self.turns.clamp(3, 10);
        self.stalled_minutes = self.stalled_minutes.clamp(5, 240);
        self
    }
}

// ---------------------------------------------------------------------------------------------
// Line classification

/// Generic error-likeness; profiles extend it (`TuiProfile::is_error_line`).
static ERROR_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(error|errors|failed|failure|exception|traceback|panicked|fatal|cannot|can't|could not|couldn't|unable to|not found|undefined|permission denied|access denied|timed out|timeout|enoent|eacces|segmentation fault|unresolved|unexpected)\b|\bTS\d{4}\b|\bE\d{4}\b",
    )
    .expect("error regex")
});
/// Lines that mention errors without being one ("0 failed", "no errors", "fixed the error").
static NOT_ERROR_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b0 (errors?|failed|failures?)\b|\bno (errors?|failures?|issues)\b|\bwithout (errors?|failures?)\b|\b(fixed|resolved|fixes|fixing)\b")
        .expect("not error regex")
});

/// The generic error-like test every profile starts from.
pub fn default_error_line(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty() && line.chars().count() <= 400 && ERROR_RE.is_match(line) && !NOT_ERROR_RE.is_match(line)
}

// ---------------------------------------------------------------------------------------------
// Per-pane tracking

#[derive(Clone, Default)]
#[allow(dead_code)]
struct Turn {
    started: u64,
    /// (normalised key, original line), one per distinct key.
    errors: Vec<(String, String)>,
    /// (normalised key with paths, original line), consecutive repeats collapsed.
    actions: Vec<(String, String)>,
    progress: bool,
}

#[derive(Clone)]
#[allow(dead_code)]
struct TurnRecord {
    turn: Turn,
    ended: u64,
}

struct Active {
    signal: LoopSignal,
    dismissed: bool,
}

#[derive(Default)]
#[allow(dead_code)]
struct PaneTrack {
    seeded: bool,
    prev: HashMap<String, usize>,
    status: Option<PaneStatus>,
    turn: Turn,
    turns: VecDeque<TurnRecord>,
    running_since: Option<u64>,
    last_new_at: u64,
    seen_this_run: HashSet<String>,
    last_lines: Vec<String>,
    active: Option<Active>,
}

/// The detector. Pure: time is passed in, so tests drive it with a fake clock.
#[derive(Default)]
pub struct Guard {
    pub settings: Settings,
    panes: HashMap<String, PaneTrack>,
}

impl Guard {
    pub fn new(settings: Settings) -> Self {
        Self { settings: settings.clamped(), panes: HashMap::new() }
    }

    pub fn all_active(&self) -> Vec<LoopSignal> {
        self.panes.values().filter_map(|t| t.active.as_ref()).filter(|a| !a.dismissed).map(|a| a.signal.clone()).collect()
    }

}

// ---------------------------------------------------------------------------------------------
// Process-wide instance and wiring

type Sink = Box<dyn Fn(&Event) + Send + Sync>;

static GUARD: Lazy<Mutex<Guard>> = Lazy::new(|| Mutex::new(Guard::new(Settings::default())));
static SINK: Lazy<Mutex<Option<Sink>>> = Lazy::new(|| Mutex::new(None));

pub fn all_active() -> Vec<LoopSignal> {
    GUARD.lock().all_active()
}

fn settings_path() -> std::path::PathBuf {
    crate::app_paths::app_data_dir().join("loop_guard.json")
}

/// Load saved settings and route events to the UI (Tauri event + SSE). Call once at startup.
pub fn init(app: tauri::AppHandle) {
    if let Some(saved) = std::fs::read_to_string(settings_path()).ok().and_then(|t| serde_json::from_str::<Settings>(&t).ok()) {
        GUARD.lock().settings = saved.clamped();
    }
    *SINK.lock() = Some(Box::new(move |event| {
        use tauri::Emitter;
        match event {
            Event::Raised(signal) => {
                let _ = app.emit("loop://signal", signal);
                crate::bridge::push_loop_signal_sse(Some(signal), &signal.pane_id);
            }
            Event::Cleared(pane_id) => {
                let _ = app.emit("loop://clear", serde_json::json!({ "pane_id": pane_id }));
                crate::bridge::push_loop_signal_sse(None, pane_id);
            }
        }
    }));
}
