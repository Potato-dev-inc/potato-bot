//! "Wait until": the bot's tool `wait_until {check, every, timeout}` and the timer behind it. Written for Puppet Master from the
//! idea of OpenMausBot's waits (the host re-checks on a timer and wakes the bot with the result); no code copied.
//!
//! - The bot calls the tool, the turn ENDS, and from then on no model runs. The waits are kept in `<app data>/potato/waits.json`
//!   (they survive a restart). Every 30 seconds `run_due` looks for a wait whose time has come and runs its check
//!   (`checks.rs`: a read-only command or a web address) through `Service::run_check`: the same switches as the bot's own tools,
//!   the risk gate's read-only reading of the command, and the owner's site list for an address.
//! - The check passes (the command exits 0, the page loads), or the time runs out, or the check is no longer allowed: the wait is
//!   removed and ONE turn starts, as a routine run (`Origin::Routine`), with the result in a fenced record. That turn counts toward
//!   the bot's daily brain-started turns like a routine does, and it cannot change settings, add routines or start another wait.
//! - Limits: 5 waits per bot, 40 in all; a check every 60 seconds at the soonest and 24 hours at the latest; up to 7 days to wait
//!   (default 1 hour, every 5 minutes). Only in the owner's private chat, never in a group.
//! - A wait keeps the bot's computer awake (every check counts as use) until it ends. The Routines tab lists the waits with Cancel.
//! - The clock is passed in so the tests need no real time.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::checks::{self, Check, CheckKind};
use super::service::{Origin, Service};
use super::store::{self, StoreError};

pub const FILE_VERSION: u32 = 1;
pub const MAX_WAITS_PER_BOT: usize = 5;
pub const MAX_WAITS_TOTAL: usize = 40;
pub const MIN_EVERY_SECS: u64 = 60;
pub const MAX_EVERY_SECS: u64 = 24 * 3_600;
pub const DEFAULT_EVERY_SECS: u64 = 300;
pub const MIN_TIMEOUT_SECS: u64 = 60;
pub const MAX_TIMEOUT_SECS: u64 = 7 * 86_400;
pub const DEFAULT_TIMEOUT_SECS: u64 = 3_600;
/// A check that cannot be read this many times in a row ends the wait.
pub const MAX_ERRORS_IN_A_ROW: u32 = 10;
const INVALID: &str = "POTATO_WAIT_INVALID";

pub const TOOL_NAMES: [&str; 2] = ["wait_until", "wait_cancel"];

pub fn is_tool(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Wait {
    pub id: String,
    pub bot: String,
    pub check: Check,
    /// What the bot is waiting for, in its own words (one line).
    pub note: String,
    pub every_secs: u64,
    pub timeout_secs: u64,
    pub created_ms: i64,
    pub deadline_ms: i64,
    pub next_check_ms: i64,
    /// Checks made so far.
    pub checks: u32,
    /// Checks in a row that could not be read.
    #[serde(default)]
    pub errors: u32,
    /// The last thing a check printed, short, for the Routines tab.
    #[serde(default)]
    pub last: String,
}

#[derive(Serialize, Deserialize)]
struct WaitsFile {
    v: u32,
    waits: Vec<Wait>,
}

/// What the Routines tab shows for a wait.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WaitView {
    pub id: String,
    pub bot: String,
    /// The note, or the check itself when there is no note.
    pub title: String,
    pub check: String,
    pub kind: CheckKind,
    pub every_words: String,
    pub checks: u32,
    pub next_check_ms: i64,
    pub deadline_ms: i64,
    pub last: String,
}

pub fn view(wait: &Wait) -> WaitView {
    WaitView {
        id: wait.id.clone(),
        bot: wait.bot.clone(),
        title: if wait.note.is_empty() { wait.check.text.clone() } else { wait.note.clone() },
        check: wait.check.text.clone(),
        kind: wait.check.kind,
        every_words: checks::duration_words(wait.every_secs),
        checks: wait.checks,
        next_check_ms: wait.next_check_ms,
        deadline_ms: wait.deadline_ms,
        last: wait.last.clone(),
    }
}

fn invalid(message: &str) -> StoreError {
    StoreError::new(INVALID, message)
}

pub fn waits_path(root: &Path) -> std::path::PathBuf {
    root.join("waits.json")
}

/// A missing file is empty; a damaged one is an error and is never overwritten.
fn load_file(root: &Path) -> Result<WaitsFile, StoreError> {
    match store::read_json::<WaitsFile>(&waits_path(root), "waits.json")? {
        None => Ok(WaitsFile { v: FILE_VERSION, waits: Vec::new() }),
        Some(file) if file.v > FILE_VERSION => Err(invalid("waits.json was written by a newer Puppet Master.")),
        Some(file) => Ok(file),
    }
}

fn save_file(root: &Path, waits: &[Wait]) -> Result<(), StoreError> {
    store::write_json(&waits_path(root), &WaitsFile { v: FILE_VERSION, waits: waits.to_vec() }, "waits.json")
}

pub fn list(root: &Path, bot: Option<&str>) -> Result<Vec<WaitView>, StoreError> {
    let _files = store::files_lock();
    Ok(load_file(root)?.waits.iter().filter(|w| bot.is_none_or(|b| w.bot == b)).map(view).collect())
}

fn text_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(|s| s.trim().to_string())
}

/// A duration the bot wrote: a number of seconds, or "90s", "5m", "2h", "1d". Absent: the default.
fn duration_arg(args: &Value, key: &str, default: u64, min: u64, max: u64, what: &str) -> Result<u64, StoreError> {
    let seconds = match args.get(key) {
        None | Some(Value::Null) => return Ok(default),
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) if s.trim().is_empty() => return Ok(default),
        Some(Value::String(s)) => checks::parse_duration(s),
        Some(_) => None,
    }
    .ok_or_else(|| StoreError::new(INVALID, format!("{what} must be a time like 90s, 5m, 2h or 1d.")))?;
    if seconds < min || seconds > max {
        return Err(StoreError::new(
            INVALID,
            format!("{what} must be between {} and {}.", checks::duration_words(min), checks::duration_words(max)),
        ));
    }
    Ok(seconds)
}

/// The checks a new wait is made of, read from the tool's arguments.
pub fn make(bot: &str, args: &Value, allow_local_pages: bool, now_ms: i64) -> Result<Wait, StoreError> {
    let raw = text_arg(args, "check").unwrap_or_default();
    let contains = text_arg(args, "contains");
    let check = checks::parse(&raw, contains.as_deref(), allow_local_pages)?;
    let every = duration_arg(args, "every", DEFAULT_EVERY_SECS, MIN_EVERY_SECS, MAX_EVERY_SECS, "every")?;
    let timeout = duration_arg(args, "timeout", DEFAULT_TIMEOUT_SECS, MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS, "timeout")?;
    if every > timeout {
        return Err(invalid("every must not be longer than timeout."));
    }
    let note = checks::clean_note(&text_arg(args, "note").unwrap_or_default());
    Ok(Wait {
        id: format!("w-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]),
        bot: bot.to_string(),
        check,
        note,
        every_secs: every,
        timeout_secs: timeout,
        created_ms: now_ms,
        deadline_ms: now_ms + (timeout as i64) * 1000,
        // The first look is at the next tick: a condition that is already true wakes the bot at once.
        next_check_ms: now_ms,
        checks: 0,
        errors: 0,
        last: String::new(),
    })
}

/// Adds a wait for a bot. At most 5 per bot and 40 in all.
pub fn create(root: &Path, wait: Wait) -> Result<Wait, StoreError> {
    let _files = store::files_lock();
    let mut file = load_file(root)?;
    let mine: Vec<&Wait> = file.waits.iter().filter(|w| w.bot == wait.bot).collect();
    if mine.len() >= MAX_WAITS_PER_BOT {
        let open: Vec<String> = mine.iter().map(|w| format!("{} ({})", w.id, if w.note.is_empty() { &w.check.text } else { &w.note })).collect();
        return Err(StoreError::new(
            INVALID,
            format!("You already have {MAX_WAITS_PER_BOT} waits open: {}. Let one finish or cancel one with wait_cancel.", open.join("; ")),
        ));
    }
    if file.waits.len() >= MAX_WAITS_TOTAL {
        return Err(invalid("There are too many waits open across the bots. Try again later."));
    }
    file.waits.push(wait.clone());
    save_file(root, &file.waits)?;
    Ok(wait)
}

/// Removes a wait of this bot (a bot never sees another bot's waits).
pub fn cancel(root: &Path, bot: Option<&str>, id: &str) -> Result<Wait, StoreError> {
    let _files = store::files_lock();
    let mut file = load_file(root)?;
    let at = file
        .waits
        .iter()
        .position(|w| w.id == id && bot.is_none_or(|b| w.bot == b))
        .ok_or_else(|| StoreError::new("POTATO_WAIT_NOT_FOUND", "There is no open wait with that id."))?;
    let gone = file.waits.remove(at);
    save_file(root, &file.waits)?;
    Ok(gone)
}

/// The two tools. The caller has checked that the turn is the owner's own private chat, not a routine's run, with the computer and
/// the web switches as the check needs them.
pub fn call(root: &Path, bot: &str, tool: &str, args: &Value, allow_local_pages: bool, now_ms: i64) -> Result<Value, StoreError> {
    match tool {
        "wait_until" => {
            let wait = create(root, make(bot, args, allow_local_pages, now_ms)?)?;
            let what = if wait.note.is_empty() { wait.check.text.clone() } else { wait.note.clone() };
            let message = format!(
                "Waiting for {what}. I will look every {} for up to {} and come back when it is done.",
                checks::duration_words(wait.every_secs),
                checks::duration_words(wait.timeout_secs)
            );
            Ok(json!({ "id": wait.id, "kind": wait.check.kind, "everySecs": wait.every_secs, "timeoutSecs": wait.timeout_secs, "message": message }))
        }
        "wait_cancel" => {
            let id = text_arg(args, "id").unwrap_or_default();
            let gone = cancel(root, Some(bot), &id)?;
            Ok(json!({ "cancelled": gone.id }))
        }
        _ => Err(invalid("Unknown wait tool.")),
    }
}

// ---- the timer -----------------------------------------------------------------------------------

/// How a wait ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ending {
    Passed,
    TimedOut,
    /// The check is not allowed any more, or could not be read ten times: the reason in plain words.
    Stopped(String),
}

/// What one check did to a wait: it goes on, or it ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Wait,
    End(Ending),
}

/// Moves a wait on after one check. `found` is the check's answer: `Ok(run)`, or `Err(reason)` for a refusal.
/// Pure: the clock is `now_ms`.
pub fn step(wait: &mut Wait, found: Result<&super::sandbox_calls::CheckRun, &super::sandbox_calls::CallError>, now_ms: i64) -> Step {
    wait.checks += 1;
    match found {
        Err(refused) => {
            wait.last = refused.message.chars().take(200).collect();
            Step::End(Ending::Stopped(refused.message.clone()))
        }
        Ok(run) => {
            wait.last = run.output.chars().take(200).collect();
            if run.passed {
                return Step::End(Ending::Passed);
            }
            // A page or computer that did not answer is an error in a row; a condition that is simply false is not.
            if run.fingerprint == "unanswered" {
                wait.errors += 1;
                if wait.errors >= MAX_ERRORS_IN_A_ROW {
                    return Step::End(Ending::Stopped(format!("The check could not be read {MAX_ERRORS_IN_A_ROW} times in a row. The last answer was: {}", wait.last)));
                }
            } else {
                wait.errors = 0;
            }
            if now_ms >= wait.deadline_ms {
                return Step::End(Ending::TimedOut);
            }
            wait.next_check_ms = (now_ms + (wait.every_secs as i64) * 1000).min(wait.deadline_ms);
            Step::Wait
        }
    }
}

/// The text and the record a woken bot gets.
pub fn wake_message(wait: &Wait, ending: &Ending, last_output: &str, now_ms: i64) -> (String, String) {
    let (words, heading) = match ending {
        Ending::Passed => ("A wait you set has finished: the check passed. Read the record below, then tell the owner what you found or go on with the task.", "Outcome: the check passed"),
        Ending::TimedOut => ("A wait you set has run out of time without the check passing. Read the record below, then tell the owner what you see and what you suggest.", "Outcome: it ran out of time"),
        Ending::Stopped(_) => ("A wait you set was stopped before it finished. Read the record below, then tell the owner and say what you suggest.", "Outcome: the wait was stopped"),
    };
    let mut rows: Vec<(&str, String)> = Vec::new();
    if !wait.note.is_empty() {
        rows.push(("Waiting for", wait.note.clone()));
    }
    rows.push(("Check", wait.check.text.clone()));
    if let Some(contains) = &wait.check.contains {
        rows.push(("Page had to hold", contains.clone()));
    }
    rows.push(("Checks made", wait.checks.to_string()));
    rows.push(("Waited", checks::duration_words(((now_ms - wait.created_ms).max(0) / 1000) as u64)));
    if let Ending::Stopped(reason) = ending {
        rows.push(("Why it stopped", reason.clone()));
    }
    (words.to_string(), checks::record_text(heading, &rows, Some(last_output)))
}

/// Runs the checks that are due and wakes the bots whose wait ended. Every 30 seconds, from its own thread. Does nothing while
/// Potato Bot is switched off or the bots are paused (the clock goes on: a wait that ran out meanwhile ends when they are back).
pub fn run_due(service: &Service, now_ms: i64) -> usize {
    if !service.enabled() || super::hold::is_held(service.root()) {
        return 0;
    }
    let due: Vec<Wait> = {
        let _files = store::files_lock();
        match load_file(service.root()) {
            Ok(file) => file.waits.into_iter().filter(|w| w.next_check_ms <= now_ms).collect(),
            Err(e) => {
                tracing::warn!(%e, "could not read the waits; they wait");
                return 0;
            }
        }
    };
    for wait in &due {
        let found = service.run_check(&wait.bot, &wait.check);
        settle(service, wait, found, now_ms);
    }
    due.len()
}

/// Writes down what one check found. An ending removes the wait first and then wakes the bot (so a wake is never sent twice).
fn settle(service: &Service, started: &Wait, found: Result<super::sandbox_calls::CheckRun, super::sandbox_calls::CallError>, now_ms: i64) {
    let ending = {
        let _files = store::files_lock();
        let Ok(mut file) = load_file(service.root()) else { return };
        let Some(at) = file.waits.iter().position(|w| w.id == started.id) else { return };
        let mut wait = file.waits[at].clone();
        let outcome = step(&mut wait, found.as_ref(), now_ms);
        match outcome {
            Step::Wait => {
                file.waits[at] = wait;
                if let Err(e) = save_file(service.root(), &file.waits) {
                    tracing::warn!(%e, "could not save a wait");
                }
                return;
            }
            Step::End(ending) => {
                file.waits.remove(at);
                if let Err(e) = save_file(service.root(), &file.waits) {
                    tracing::warn!(%e, "could not save the waits");
                    return;
                }
                (wait, ending)
            }
        }
    };
    let (wait, ending) = ending;
    let last = found.as_ref().map(|r| r.output.clone()).unwrap_or_else(|e| e.message.clone());
    wake(service, wait, &ending, &last, now_ms);
}

/// Starts the one turn that tells the bot how its wait ended.
fn wake(service: &Service, wait: Wait, ending: &Ending, last_output: &str, now_ms: i64) {
    let (text, context) = wake_message(&wait, ending, last_output, now_ms);
    let sent = service.send_as(&wait.bot, &text, Origin::Routine { title: "Wait finished", context: Some(&context) });
    if let Err(e) = sent {
        tracing::warn!(%e, "a wait could not wake its bot");
        service.note(&wait.bot, "A wait finished, but the bot could not be woken for it. Ask it what happened.");
    }
}
