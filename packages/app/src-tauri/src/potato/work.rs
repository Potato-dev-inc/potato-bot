//! The live step list of a turn ("Looked at the screen", "Model stopped answering, retrying"). The brain sends one
//! `progress` event per tool start and end (the tool's name and a short safe line, never arguments or results);
//! this keeps the last few for the thread to show while the turn runs. Nothing here is saved to disk.

use serde::Serialize;

use super::protocol::{ProgressPhase, StepInfo};

/// Steps kept per turn; the oldest is dropped past this.
const MAX_STEPS: usize = 40;
/// Log lines kept per step (the thread shows the last three).
const MAX_LOG: usize = 3;
/// One log line, in characters.
const LOG_CHARS: usize = 80;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkStep {
    pub id: String,
    pub tool: String,
    /// `running`, `done`, `error` or `retry` (a model call being asked again).
    pub status: &'static str,
    pub started_ms: i64,
    pub ended_ms: Option<i64>,
    pub log: Vec<String>,
    /// A plain one-line description of what the step did ("Ran: git status"), up to 120 characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The full command or path, for the opened row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full: Option<String>,
    /// The model's own short reason for the call, when it gave one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The last lines of the output (at most 10), once the step ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail: Option<String>,
    /// The error line of a step that failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit: Option<i64>,
}

/// Lines kept of an output tail, and the characters of the whole tail.
const TAIL_LINES: usize = 10;
const TAIL_CHARS: usize = 800;

/// One scrubbed line: whitespace folded, cut to `max` characters with an ellipsis.
fn one_line(text: &str, max: usize) -> Option<String> {
    let safe = crate::companion::view::scrub(text, None);
    let flat = safe.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return None;
    }
    if flat.chars().count() <= max {
        return Some(flat);
    }
    Some(flat.chars().take(max.saturating_sub(1)).collect::<String>() + "\u{2026}")
}

/// A scrubbed block of text, lines kept: the last `lines` lines, at most `max` characters.
fn block(text: &str, lines: usize, max: usize) -> Option<String> {
    let safe = crate::companion::view::scrub(text, None);
    let all: Vec<&str> = safe.lines().map(str::trim_end).collect();
    let from = all.len().saturating_sub(lines);
    let joined = all[from..].join("\n");
    let joined = joined.trim().to_string();
    if joined.is_empty() {
        return None;
    }
    let count = joined.chars().count();
    Some(if count <= max { joined } else { joined.chars().skip(count - max).collect() })
}

fn new_step(id: &str, tool: &str, status: &'static str, now: i64) -> WorkStep {
    WorkStep { id: id.into(), tool: tool.into(), status, started_ms: now, ended_ms: None, log: Vec::new(), detail: None, full: None, why: None, tail: None, error: None, exit: None }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkRun {
    pub started_ms: i64,
    pub steps: Vec<WorkStep>,
    /// The turn failed: the X stays on the step it stopped at until the next turn starts.
    pub failed: bool,
    /// When the turn ended well. The list stays, folded, until the next turn starts.
    pub ended_ms: Option<i64>,
    /// The bot has a running job: after this turn the app starts the next one by itself, so the turn did not end the work.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub continues: bool,
}

fn line(text: &str) -> Option<String> {
    let first = text.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        return None;
    }
    let safe = crate::companion::view::scrub(first, None);
    Some(safe.chars().take(LOG_CHARS).collect())
}

impl WorkRun {
    pub fn new(now: i64) -> Self {
        WorkRun { started_ms: now, steps: Vec::new(), failed: false, ended_ms: None, continues: false }
    }

    /// A retry step ends when the next thing happens.
    fn end_retries(&mut self, now: i64) {
        for s in self.steps.iter_mut().filter(|s| s.status == "retry") {
            s.status = "done";
            s.ended_ms = Some(now);
        }
    }

    #[cfg(test)]
    pub fn apply(&mut self, id: &str, tool: &str, phase: ProgressPhase, ok: bool, log: Option<&str>, now: i64) {
        self.apply_with(id, tool, phase, ok, log, &StepInfo::default(), now);
    }

    /// `apply` with what the step did in plain words (see `StepInfo`). A start for an id that already finished is a new
    /// step (some providers reuse call ids from round to round): the earlier row keeps its own time.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_with(&mut self, id: &str, tool: &str, phase: ProgressPhase, ok: bool, log: Option<&str>, info: &StepInfo, now: i64) {
        self.end_retries(now);
        match phase {
            ProgressPhase::Retry => {
                self.steps.push(new_step(id, "retry", "retry", now));
            }
            ProgressPhase::Start | ProgressPhase::End => {
                let open = self.steps.iter().rposition(|s| s.id == id && s.status == "running");
                let found = match (open, phase) {
                    (Some(i), _) => Some(i),
                    // An end for a step that already ended is the same row; a start for one is a new row.
                    (None, ProgressPhase::End) => self.steps.iter().rposition(|s| s.id == id),
                    _ => None,
                };
                let i = found.unwrap_or_else(|| {
                    self.steps.push(new_step(id, tool, "running", now));
                    self.steps.len() - 1
                });
                let step = &mut self.steps[i];
                if let Some(d) = info.detail.as_deref().and_then(|t| one_line(t, 120)) {
                    step.detail = Some(d);
                }
                if let Some(f) = info.full.as_deref().and_then(|t| block(t, 20, 600)) {
                    step.full = Some(f);
                }
                if let Some(w) = info.why.as_deref().and_then(|t| one_line(t, 120)) {
                    step.why = Some(w);
                }
                if let Some(t) = info.tail.as_deref().and_then(|t| block(t, TAIL_LINES, TAIL_CHARS)) {
                    step.tail = Some(t);
                }
                if let Some(e) = info.error.as_deref().and_then(|t| one_line(t, 160)) {
                    step.error = Some(e);
                }
                if info.exit.is_some() {
                    step.exit = info.exit;
                }
                if let Some(l) = log.and_then(line) {
                    step.log.push(l);
                    let extra = step.log.len().saturating_sub(MAX_LOG);
                    step.log.drain(..extra);
                }
                if phase == ProgressPhase::End {
                    step.status = if ok { "done" } else { "error" };
                    step.ended_ms = Some(now);
                }
            }
        }
        if self.steps.len() > MAX_STEPS {
            let extra = self.steps.len() - MAX_STEPS;
            self.steps.drain(..extra);
        }
    }

    /// The turn is over without a failure (it finished, or Stop ended it): a run with no steps leaves nothing to show.
    pub fn finish(&mut self, now: i64) {
        for s in self.steps.iter_mut().filter(|s| s.status == "running" || s.status == "retry") {
            s.status = "done";
            s.ended_ms.get_or_insert(now);
        }
        self.ended_ms = Some(now);
    }

    /// The turn failed: the step that was running (or the last one) shows the X.
    pub fn fail(&mut self, now: i64) {
        self.failed = true;
        let at = self.steps.iter().rposition(|s| s.status == "running" || s.status == "retry").or_else(|| self.steps.len().checked_sub(1));
        if let Some(i) = at {
            self.steps[i].status = "error";
            self.steps[i].ended_ms.get_or_insert(now);
        }
    }
}

/// The run to show for a conversation: one running now, or the last one that ended (well or not) and has not been replaced.
pub fn shown(work: &std::collections::HashMap<String, WorkRun>, typing: &std::collections::HashMap<String, i64>, conversation: &str) -> Option<WorkRun> {
    work.get(conversation).filter(|w| !w.steps.is_empty() && (w.failed || w.ended_ms.is_some() || typing.contains_key(conversation))).cloned()
}
