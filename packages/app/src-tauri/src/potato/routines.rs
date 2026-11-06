//! Routines of Potato Bot (phase P9a): a prompt a bot runs on a schedule, in `<app data>/potato/routines.json`.
//!
//! - A schedule is a preset (`hourly`, `daily` at HH:MM, `weekdays` at HH:MM, `everyMinutes` 15 to 1440), a
//!   standard 5-field `cron` (read by `routines/cron.rs`, with the same 15 minute floor) or a
//!   one-time `once` at a local date and time. Times are the computer's local clock (no zone is stored).
//! - `next_after` is the pure next-run math. `take_due` is the pure scheduler step: it picks what is due at `now`,
//!   moves each routine on, and returns what to send. A routine due several times while the app was closed runs
//!   ONCE (the missed ticks are coalesced); a run that starts 15 minutes or more after it was due gets a note in
//!   its `context` saying it ran late.
//! - `run_due` is the 30 second tick (started with the live service): it saves the moved routines first, then
//!   hands each one to the bot's brain as a message from `routine`, trusted like the person at this computer.
//!   Helpers still need Start.
//! - Run history: the last 10 runs of each routine (when, ok / failed / skipped with a plain sentence, and the bot's
//!   reply message in the thread) in `routine_runs.json`. A run is recorded when it is handed to the bot and is
//!   updated when the brain answers (`record_reply`, `record_failure`, called by the service).
//! - Every whole-file write takes the store lock (`store::files_lock`).
//!
//! The clock is passed in (`now_ms`) so the tests need no real time. Nothing here names an agent or a chat app.

use std::path::Path;

use chrono::{Datelike, Local, NaiveDateTime, NaiveTime, TimeZone, Timelike, Weekday};
use serde_json::{json, Value};
use serde::{Deserialize, Serialize};

use super::checks::{self, Check, CheckKind};
use super::sandbox_calls::{CallError, CheckRun};
use super::service::{Origin, SendOutcome, Service};
use super::store::{self, StoreError};

pub mod cron;
#[cfg(test)]
mod cron_tests;

pub const FILE_VERSION: u32 = 1;
pub const MIN_EVERY_MINUTES: u32 = 15;
pub const MAX_EVERY_MINUTES: u32 = 1440;
/// A run that starts this many minutes after it was due says so to the brain.
pub const LATE_AFTER_MINUTES: i64 = 15;
pub const MAX_ROUTINES_PER_BOT: usize = 20;
pub const MAX_TITLE_CHARS: usize = 80;
pub const MAX_PROMPT_CHARS: usize = 2000;
const INVALID: &str = "POTATO_ROUTINE_INVALID";
const NEXT_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";
const ONCE_FORMAT: &str = "%Y-%m-%dT%H:%M";

/// When a routine runs. Serialized as `{ "kind": "daily", "time": "09:00" }` and so on.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Schedule {
    /// Every hour, on the hour.
    Hourly,
    /// Every day at `time` ("HH:MM", 24 hours).
    Daily { time: String },
    /// Monday to Friday at `time`.
    Weekdays { time: String },
    /// Every `minutes`, on a grid counted from local midnight.
    EveryMinutes { minutes: u32 },
    /// Once, at a local date and time ("YYYY-MM-DDTHH:MM").
    Once { at: String },
    /// A standard 5-field cron ("minute hour day-of-month month day-of-week"), on the local clock. Runs are at least
    /// 15 minutes apart (checked when it is saved).
    Cron { expr: String },
}

/// "Only when something changes": what the form sends for a watching routine. The check is a read-only command or a web address
/// (`checks.rs`); at each time on the schedule it is run WITHOUT a model, and the routine's prompt is sent to the bot only when
/// the result is not the one seen last time.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WatchInput {
    pub check: String,
    /// Web address only: the page must also hold these words for the check to count as true.
    #[serde(default)]
    pub contains: Option<String>,
}

/// A watching routine's check and what it saw last.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Watch {
    pub check: Check,
    /// The fingerprint of the last result that was read. `None`: no look yet (the first look only saves what it saw).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// The start of what the last look printed, redacted and short, so the next change can say what it was before.
    #[serde(default)]
    pub last_output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_ms: Option<i64>,
    /// Looks in a row that could not be read.
    #[serde(default)]
    pub failures: u32,
}

/// What the form sends: a name, what to do, and when.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoutineInput {
    pub title: String,
    pub prompt: String,
    pub schedule: Schedule,
    /// Run only when this check's result changes. Absent: the routine runs at every time on its schedule.
    #[serde(default)]
    pub watch: Option<WatchInput>,
}

/// One routine as stored.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub id: String,
    pub bot: String,
    pub title: String,
    pub prompt: String,
    pub schedule: Schedule,
    #[serde(default)]
    pub paused: bool,
    /// A one-time routine that has run. It never runs again unless it is edited to a future time.
    #[serde(default)]
    pub done: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_ms: Option<i64>,
    /// The next run as local wall-clock time ("YYYY-MM-DDTHH:MM:SS"). None while paused or done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run: Option<String>,
    pub created_ms: i64,
    /// A watching routine (see `Watch`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<Watch>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RoutinesFile {
    pub v: u32,
    pub routines: Vec<Routine>,
}

/// What the page shows for a routine.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoutineView {
    pub id: String,
    pub bot: String,
    pub title: String,
    pub prompt: String,
    pub schedule: Schedule,
    /// "Weekdays 6am", "Every hour", "Once on Nov 2 at 9am" and so on.
    pub summary: String,
    /// `active`, `paused` or `done`.
    pub status: &'static str,
    pub paused: bool,
    pub done: bool,
    pub last_run_ms: Option<i64>,
    pub next_run_ms: Option<i64>,
    /// A watching routine: what it watches and when it last looked.
    pub watch: Option<WatchView>,
}

/// What the page shows of a watching routine.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WatchView {
    pub check: String,
    pub kind: CheckKind,
    pub contains: Option<String>,
    pub last_checked_ms: Option<i64>,
}

/// One run the tick sends to a brain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Due {
    pub id: String,
    pub bot: String,
    pub title: String,
    pub prompt: String,
    /// The late note, when the run started 15 minutes or more after it was due.
    pub context: Option<String>,
    /// A watching routine's check: the prompt goes to the bot only when its result changed.
    pub check: Option<Check>,
}

fn invalid(message: &str) -> StoreError {
    StoreError::new(INVALID, message)
}

// ---- time math (pure: local wall-clock values only) -----------------------------------------------

/// "HH:MM" to a time of day. Anything else is None.
pub fn parse_clock(text: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(text.trim(), "%H:%M").ok()
}

fn parse_once(text: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(text.trim(), ONCE_FORMAT).ok()
}

fn parse_next(text: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(text, NEXT_FORMAT).ok()
}

fn format_next(at: NaiveDateTime) -> String {
    at.format(NEXT_FORMAT).to_string()
}

/// The first run strictly after `after`, or None when the schedule has no run left.
pub fn next_after(schedule: &Schedule, after: NaiveDateTime) -> Option<NaiveDateTime> {
    match schedule {
        Schedule::Hourly => {
            let top = after.date().and_hms_opt(after.hour(), 0, 0)?;
            Some(top + chrono::TimeDelta::hours(1))
        }
        Schedule::Daily { time } => {
            let at = parse_clock(time)?;
            let today = after.date().and_time(at);
            Some(if today > after { today } else { today + chrono::TimeDelta::days(1) })
        }
        Schedule::Weekdays { time } => {
            let at = parse_clock(time)?;
            (0..8).map(|offset| after.date() + chrono::TimeDelta::days(offset)).map(|day| day.and_time(at)).find(|candidate| {
                *candidate > after && !matches!(candidate.weekday(), Weekday::Sat | Weekday::Sun)
            })
        }
        Schedule::EveryMinutes { minutes } => {
            if !(MIN_EVERY_MINUTES..=MAX_EVERY_MINUTES).contains(minutes) {
                return None;
            }
            let step = i64::from(*minutes) * 60;
            let seconds = i64::from(after.num_seconds_from_midnight());
            let next = (seconds / step + 1) * step;
            let midnight = after.date().and_hms_opt(0, 0, 0)?;
            Some(midnight + chrono::TimeDelta::seconds(next))
        }
        Schedule::Once { at } => parse_once(at).filter(|at| *at > after),
        Schedule::Cron { expr } => cron::Cron::parse(expr).ok()?.next_after(after),
    }
}

/// "6am", "6:30am", "12pm".
fn clock_words(time: NaiveTime) -> String {
    let hour = time.hour();
    let suffix = if hour < 12 { "am" } else { "pm" };
    let hour12 = if hour % 12 == 0 { 12 } else { hour % 12 };
    if time.minute() == 0 {
        format!("{hour12}{suffix}")
    } else {
        format!("{hour12}:{:02}{suffix}", time.minute())
    }
}

/// The schedule in plain words, for the list.
pub fn summary(schedule: &Schedule) -> String {
    match schedule {
        Schedule::Hourly => "Every hour".to_string(),
        Schedule::Daily { time } => format!("Every day {}", parse_clock(time).map(clock_words).unwrap_or_else(|| time.clone())),
        Schedule::Weekdays { time } => format!("Weekdays {}", parse_clock(time).map(clock_words).unwrap_or_else(|| time.clone())),
        Schedule::EveryMinutes { minutes } => format!("Every {minutes} minutes"),
        Schedule::Once { at } => match parse_once(at) {
            Some(at) => format!("Once on {} at {}", at.format("%b %-d"), clock_words(at.time())),
            None => "Once".to_string(),
        },
        Schedule::Cron { expr } => format!("Cron {}", expr.split_whitespace().collect::<Vec<_>>().join(" ")),
    }
}

fn late_note(due: NaiveDateTime, now: NaiveDateTime) -> String {
    format!(
        "This run is late. It was due at {} and started at {}. Adjust anything that depends on the time of day, and mention the delay if it matters.",
        due.format("%b %-d, %-I:%M %p"),
        now.format("%b %-d, %-I:%M %p"),
    )
}

// ---- validation -----------------------------------------------------------------------------------

/// Checks a form before it is saved. `now` is the local wall clock; a one-time run must be in the future.
pub fn validate(input: &RoutineInput, now: NaiveDateTime) -> Result<(), StoreError> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err(invalid("Give the routine a name."));
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(invalid("The name is too long. Keep it under 80 letters."));
    }
    let prompt = input.prompt.trim();
    if prompt.is_empty() {
        return Err(invalid("Write what the bot should do."));
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(invalid("That is too long. Keep it under 2000 letters."));
    }
    if let Some(watch) = &input.watch {
        checks::parse(&watch.check, watch.contains.as_deref(), false)?;
    }
    match &input.schedule {
        Schedule::Hourly => Ok(()),
        Schedule::Daily { time } | Schedule::Weekdays { time } => {
            parse_clock(time).map(|_| ()).ok_or_else(|| invalid("Pick a time like 09:00."))
        }
        Schedule::EveryMinutes { minutes } => {
            if (MIN_EVERY_MINUTES..=MAX_EVERY_MINUTES).contains(minutes) {
                Ok(())
            } else {
                Err(invalid("Pick every 15 minutes or more."))
            }
        }
        Schedule::Once { at } => match parse_once(at) {
            Some(at) if at > now => Ok(()),
            Some(_) => Err(invalid("Pick a time in the future.")),
            None => Err(invalid("Pick a date and time.")),
        },
        Schedule::Cron { expr } => cron::check(expr).map(|_| ()).map_err(|message| invalid(&message)),
    }
}

/// The next `count` runs of a cron (as instants), for the preview under the cron box in the form. The same checks as
/// saving: a cron that would be refused answers with the reason in plain words.
pub fn cron_preview(expr: &str, now_ms: i64, count: usize) -> Result<Vec<i64>, StoreError> {
    let runs = cron::next_runs(expr, local_naive(now_ms), count.min(10)).map_err(|message| invalid(&message))?;
    Ok(runs.into_iter().filter_map(instant_ms).collect())
}

// ---- the scheduler step (pure) --------------------------------------------------------------------

/// Picks the routines due at `now` (local wall clock; `now_ms` is the same instant), moves each one on, and
/// returns what to send. One run per routine however many times it was missed. A paused or finished routine
/// never runs. A routine with no next run gets one now (a file from an older build).
pub fn take_due(routines: &mut [Routine], now: NaiveDateTime, now_ms: i64) -> Vec<Due> {
    let mut out = Vec::new();
    for routine in routines.iter_mut() {
        if routine.paused || routine.done {
            continue;
        }
        let Some(due) = routine.next_run.as_deref().and_then(parse_next) else {
            routine.next_run = next_after(&routine.schedule, now).map(format_next);
            continue;
        };
        if due > now {
            continue;
        }
        let context = ((now - due).num_minutes() >= LATE_AFTER_MINUTES).then(|| late_note(due, now));
        if matches!(routine.schedule, Schedule::Once { .. }) {
            routine.done = true;
            routine.next_run = None;
        } else {
            routine.next_run = next_after(&routine.schedule, now).map(format_next);
        }
        routine.last_run_ms = Some(now_ms);
        out.push(Due {
            id: routine.id.clone(),
            bot: routine.bot.clone(),
            title: routine.title.clone(),
            prompt: routine.prompt.trim().to_string(),
            context,
            check: routine.watch.as_ref().map(|w| w.check.clone()),
        });
    }
    out
}

/// Local wall-clock time of an instant.
pub fn local_naive(ms: i64) -> NaiveDateTime {
    match Local.timestamp_millis_opt(ms).single() {
        Some(at) => at.naive_local(),
        None => chrono::DateTime::<chrono::Utc>::UNIX_EPOCH.naive_utc(),
    }
}

/// An instant of a local wall-clock time (the earlier one in a repeated hour; the clock is read as UTC in a gap).
fn instant_ms(at: NaiveDateTime) -> Option<i64> {
    match Local.from_local_datetime(&at) {
        chrono::LocalResult::Single(t) => Some(t.timestamp_millis()),
        chrono::LocalResult::Ambiguous(first, _) => Some(first.timestamp_millis()),
        chrono::LocalResult::None => Some(at.and_utc().timestamp_millis()),
    }
}

pub fn view(routine: &Routine) -> RoutineView {
    let status = if routine.done {
        "done"
    } else if routine.paused {
        "paused"
    } else {
        "active"
    };
    RoutineView {
        id: routine.id.clone(),
        bot: routine.bot.clone(),
        title: routine.title.clone(),
        prompt: routine.prompt.clone(),
        schedule: routine.schedule.clone(),
        summary: summary(&routine.schedule),
        status,
        paused: routine.paused,
        done: routine.done,
        last_run_ms: routine.last_run_ms,
        next_run_ms: routine.next_run.as_deref().and_then(parse_next).and_then(instant_ms),
        watch: routine.watch.as_ref().map(|w| WatchView { check: w.check.text.clone(), kind: w.check.kind, contains: w.check.contains.clone(), last_checked_ms: w.last_checked_ms }),
    }
}

/// The watch a form describes. The same check as before keeps what it saw (no new "first look"); a different one starts fresh.
fn build_watch(input: &RoutineInput, before: Option<&Watch>) -> Result<Option<Watch>, StoreError> {
    let Some(raw) = &input.watch else { return Ok(None) };
    let check = checks::parse(&raw.check, raw.contains.as_deref(), false)?;
    Ok(Some(match before {
        Some(old) if old.check == check => old.clone(),
        _ => Watch { check, fingerprint: None, last_output: String::new(), last_checked_ms: None, failures: 0 },
    }))
}

// ---- the file -------------------------------------------------------------------------------------

pub fn routines_path(root: &Path) -> std::path::PathBuf {
    root.join("routines.json")
}

/// Reads the file under the store lock. A missing file is empty; a damaged one is an error and is never overwritten.
fn load_file(root: &Path) -> Result<RoutinesFile, StoreError> {
    let file: Option<RoutinesFile> = store::read_json(&routines_path(root), "routines.json")?;
    match file {
        None => Ok(RoutinesFile { v: FILE_VERSION, routines: Vec::new() }),
        Some(file) if file.v > FILE_VERSION => Err(invalid("routines.json was written by a newer Puppet Master.")),
        Some(file) => Ok(file),
    }
}

fn save_file(root: &Path, routines: &[Routine]) -> Result<(), StoreError> {
    store::write_json(&routines_path(root), &RoutinesFile { v: FILE_VERSION, routines: routines.to_vec() }, "routines.json")
}

pub fn list(root: &Path, bot: Option<&str>) -> Result<Vec<RoutineView>, StoreError> {
    let _files = store::files_lock();
    Ok(load_file(root)?.routines.iter().filter(|r| bot.is_none_or(|b| r.bot == b)).map(view).collect())
}

fn find_in(routines: &[Routine], id: &str) -> Result<usize, StoreError> {
    routines.iter().position(|r| r.id == id).ok_or_else(|| StoreError::new("POTATO_ROUTINE_NOT_FOUND", "No routine has that name."))
}

fn bot_exists(root: &Path, bot: &str) -> Result<(), StoreError> {
    if store::load_bots(root)?.iter().any(|b| b.id == bot) {
        Ok(())
    } else {
        Err(StoreError::new("POTATO_BOT_NOT_FOUND", "No bot has that name."))
    }
}

/// Makes a routine for a bot. At most 20 per bot.
pub fn create(root: &Path, bot: &str, input: &RoutineInput, now_ms: i64) -> Result<RoutineView, StoreError> {
    let now = local_naive(now_ms);
    validate(input, now)?;
    bot_exists(root, bot)?;
    let _files = store::files_lock();
    let mut file = load_file(root)?;
    if file.routines.iter().filter(|r| r.bot == bot).count() >= MAX_ROUTINES_PER_BOT {
        return Err(invalid("This bot has 20 routines already. Delete one first."));
    }
    let routine = Routine {
        id: format!("r-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]),
        bot: bot.to_string(),
        title: input.title.trim().to_string(),
        prompt: input.prompt.trim().to_string(),
        schedule: input.schedule.clone(),
        paused: false,
        done: false,
        last_run_ms: None,
        next_run: next_after(&input.schedule, now).map(format_next),
        created_ms: now_ms,
        watch: build_watch(input, None)?,
    };
    file.routines.push(routine.clone());
    save_file(root, &file.routines)?;
    Ok(view(&routine))
}

/// Changes a routine's name, prompt and schedule. A one-time routine that ran can be set to a new future time.
pub fn update(root: &Path, id: &str, input: &RoutineInput, now_ms: i64) -> Result<RoutineView, StoreError> {
    let now = local_naive(now_ms);
    validate(input, now)?;
    let _files = store::files_lock();
    let mut file = load_file(root)?;
    let index = find_in(&file.routines, id)?;
    let routine = &mut file.routines[index];
    routine.title = input.title.trim().to_string();
    routine.prompt = input.prompt.trim().to_string();
    routine.schedule = input.schedule.clone();
    routine.watch = build_watch(input, routine.watch.as_ref())?;
    routine.done = false;
    routine.next_run = if routine.paused { None } else { next_after(&routine.schedule, now).map(format_next) };
    let saved = view(routine);
    save_file(root, &file.routines)?;
    Ok(saved)
}

/// Pause (`paused = true`) or resume. Resuming looks forward from now: the time paused is never run late.
pub fn set_paused(root: &Path, id: &str, paused: bool, now_ms: i64) -> Result<RoutineView, StoreError> {
    let now = local_naive(now_ms);
    let _files = store::files_lock();
    let mut file = load_file(root)?;
    let index = find_in(&file.routines, id)?;
    let routine = &mut file.routines[index];
    routine.paused = paused;
    routine.next_run = if paused || routine.done { None } else { next_after(&routine.schedule, now).map(format_next) };
    let saved = view(routine);
    save_file(root, &file.routines)?;
    Ok(saved)
}

/// The 30 second tick. Does nothing while Potato Bot is switched off. Moves the due routines on and saves that
/// first, then sends each one to its brain. Returns how many were due.
pub fn run_due(service: &Service, now_ms: i64) -> usize {
    if !service.enabled() {
        return 0;
    }
    let root = service.root();
    let held = super::hold::is_held(root);
    let dues = {
        let _files = store::files_lock();
        let mut file = match load_file(root) {
            Ok(file) => file,
            Err(e) => {
                tracing::warn!(%e, "could not read the routines; they wait");
                return 0;
            }
        };
        let dues = take_due(&mut file.routines, local_naive(now_ms), now_ms);
        if !dues.is_empty() {
            if let Err(e) = save_file(root, &file.routines) {
                tracing::warn!(%e, "could not save the routines; they will run again later");
                return 0;
            }
        }
        dues
    };
    for due in &dues {
        if held {
            service.note(&due.bot, &super::hold::skipped_note(&due.title));
            record_run(root, run_record(&due.id, &due.bot, now_ms, Err(&StoreError::new("POTATO_PAUSED", "paused"))));
            continue;
        }
        // A watching routine looks first, with no model, and sends its prompt only when what it saw is not what it saw before.
        if let Some(check) = &due.check {
            watch_run(service, due, check, now_ms);
            continue;
        }
        let origin = Origin::Routine { title: &due.title, context: due.context.as_deref() };
        let sent = service.send_as(&due.bot, &due.prompt, origin);
        record_run(root, run_record(&due.id, &due.bot, now_ms, sent.as_ref()));
        if let Err(e) = sent {
            tracing::warn!(%e, routine = %due.id, "a routine could not reach its bot");
            // A refused run is never silent: its history line says why (the code is stable, the text is for people).
            if let Some(line) = refused_note(&e.code, &due.title) {
                service.note(&due.bot, &line);
            }
        }
    }
    dues.len()
}

// ---- watching routines: run a check on the schedule, start a turn only when its result changes ---------

/// What one look of a watching routine means.
#[derive(Debug, PartialEq, Eq)]
pub enum WatchOutcome {
    /// The first look: it only saves what it saw.
    First,
    /// The same result as last time: nothing happens.
    Same,
    /// A different result: a turn starts. `before` is the start of what the last look printed.
    Changed { before: String },
    /// The check could not be read or was refused: nothing is saved as seen, and no turn starts.
    Failed(String),
}

/// Pure: compares one look with what the routine saw last. A nonzero exit is a result like any other; only a check that
/// did not answer at all, or was refused, is a failure.
pub fn watch_outcome(watch: &Watch, found: Result<&CheckRun, &CallError>) -> WatchOutcome {
    match found {
        Err(refused) => WatchOutcome::Failed(refused.message.clone()),
        Ok(run) if run.fingerprint == "unanswered" => WatchOutcome::Failed(run.output.clone()),
        Ok(run) => match &watch.fingerprint {
            None => WatchOutcome::First,
            Some(old) if *old == run.fingerprint => WatchOutcome::Same,
            Some(_) => WatchOutcome::Changed { before: watch.last_output.clone() },
        },
    }
}

/// The most of a look's output kept in the file for the next "before".
const MAX_WATCH_OUTPUT_CHARS: usize = 1000;

/// The record a bot reads when its watch saw a change (or when "Run now" looks at once).
fn watch_record(heading: &str, check: &Check, before: Option<&str>, now: &str) -> String {
    let mut rows: Vec<(&str, String)> = vec![("Watching", check.text.clone())];
    if let Some(words) = &check.contains {
        rows.push(("Page had to hold", words.clone()));
    }
    if let Some(before) = before.filter(|b| !b.trim().is_empty()) {
        rows.push(("Before", checks::tail(before.trim_end(), MAX_WATCH_OUTPUT_CHARS)));
    }
    rows.push(("Now", checks::tail(now.trim_end(), MAX_WATCH_OUTPUT_CHARS)));
    checks::record_text(heading, &rows, None)
}

/// One scheduled look of a watching routine.
fn watch_run(service: &Service, due: &Due, check: &Check, now_ms: i64) {
    let root = service.root();
    let found = service.run_check(&due.bot, check);
    let (outcome, streak) = {
        let _files = store::files_lock();
        let Ok(mut file) = load_file(root) else { return };
        let Some(routine) = file.routines.iter_mut().find(|r| r.id == due.id) else { return };
        let Some(watch) = routine.watch.as_mut() else { return };
        // Edited since the tick took it: the next time will use the new check.
        if watch.check != *check {
            return;
        }
        let outcome = watch_outcome(watch, found.as_ref());
        watch.last_checked_ms = Some(now_ms);
        match (&outcome, &found) {
            (WatchOutcome::Failed(_), _) => watch.failures += 1,
            (_, Ok(run)) => {
                watch.fingerprint = Some(run.fingerprint.clone());
                watch.last_output = run.output.chars().take(MAX_WATCH_OUTPUT_CHARS).collect();
                watch.failures = 0;
            }
            _ => {}
        }
        let streak = watch.failures;
        if let Err(e) = save_file(root, &file.routines) {
            tracing::warn!(%e, "could not save what a watching routine saw");
            return;
        }
        (outcome, streak)
    };
    let record = |result: RunResult, text: String| RunRecord { routine: due.id.clone(), bot: due.bot.clone(), at_ms: now_ms, result, text, message_id: None, reply_id: None };
    match outcome {
        WatchOutcome::First => record_run(root, record(RunResult::Ok, "First look saved. It starts a turn when this changes.".to_string())),
        WatchOutcome::Same => record_run(root, record(RunResult::Skipped, "Nothing changed. No turn started.".to_string())),
        WatchOutcome::Failed(reason) => {
            let short: String = reason.chars().take(160).collect();
            record_run(root, record(RunResult::Failed, format!("Could not look: {short}")));
            // One line in the chat for a streak, not one per look.
            if streak == 1 {
                service.note(&due.bot, &format!("Watching \"{}\" could not look: {short}", due.title));
            }
        }
        WatchOutcome::Changed { before } => {
            let now = found.as_ref().map(|r| r.output.clone()).unwrap_or_default();
            let mut context = watch_record("Something changed", check, Some(&before), &now);
            if let Some(late) = &due.context {
                context = format!("{late}\n\n{context}");
            }
            let origin = Origin::Routine { title: &due.title, context: Some(&context) };
            let sent = service.send_as(&due.bot, &due.prompt, origin);
            record_run(root, run_record(&due.id, &due.bot, now_ms, sent.as_ref()));
            if let Err(e) = sent {
                tracing::warn!(%e, routine = %due.id, "a watching routine could not reach its bot");
                if let Some(line) = refused_note(&e.code, &due.title) {
                    service.note(&due.bot, &line);
                }
            }
        }
    }
}

/// The history line for a routine run the app refused to send. `None` when the refusal already left its own line.
pub fn refused_note(code: &str, title: &str) -> Option<String> {
    Some(match code {
        "POTATO_TURNS" | "POTATO_BOT_NOT_FOUND" => return None,
        "POTATO_PAUSED" => super::hold::skipped_note(title),
        "POTATO_RATE" => format!("Skipped: too many messages in a minute: {title}"),
        "POTATO_OFF" => format!("Skipped: Potato Bot is switched off: {title}"),
        _ => format!("Skipped: the message could not be sent: {title}"),
    })
}

// ---- runs that are due but not yet taken by a brain (survive a restart) ---------------------------

/// A routine run that was written to the thread and handed to a bot, and that no brain has answered yet.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingRun {
    /// The saved message id: the brain's reply or error names it in `inReplyTo`.
    pub id: String,
    pub bot: String,
    pub title: String,
    pub at_ms: i64,
}

pub fn pending_path(root: &Path) -> std::path::PathBuf {
    root.join("routine_pending.json")
}

/// The pending runs on disk. A damaged file is read as none (logged), so the app still starts.
fn load_pending(root: &Path) -> Vec<PendingRun> {
    match store::read_json::<Vec<PendingRun>>(&pending_path(root), "routine pending runs") {
        Ok(found) => found.unwrap_or_default(),
        Err(e) => {
            tracing::warn!(%e, "could not read the pending routine runs");
            Vec::new()
        }
    }
}

fn save_pending(root: &Path, runs: &[PendingRun]) {
    if let Err(e) = store::write_json(&pending_path(root), &runs.to_vec(), "routine pending runs") {
        tracing::warn!(%e, "could not save the pending routine runs");
    }
}

/// Written BEFORE the run goes to the queue, so a quit before the brain takes it leaves it on disk.
pub fn add_pending(root: &Path, run: PendingRun) {
    let _files = store::files_lock();
    let mut runs = load_pending(root);
    runs.push(run);
    save_pending(root, &runs);
}

/// The brain answered (a reply or an error) for this message: the run was taken.
/// True when that message was a pending routine run (so the caller knows to update its history line).
pub fn clear_pending(root: &Path, id: &str) -> bool {
    let _files = store::files_lock();
    let runs = load_pending(root);
    if runs.iter().any(|r| r.id == id) {
        save_pending(root, &runs.into_iter().filter(|r| r.id != id).collect::<Vec<_>>());
        true
    } else {
        false
    }
}

#[cfg(test)]
pub fn pending_count(root: &Path) -> usize {
    load_pending(root).len()
}

#[cfg(test)]
pub fn pending_ids(root: &Path) -> Vec<String> {
    load_pending(root).into_iter().map(|r| r.id).collect()
}

/// On a fresh start: every run still pending was never answered. It is written as missed (never run again, so it
/// cannot run twice) and the list is cleared.
pub fn recover_pending(service: &Service, _now_ms: i64) {
    let missed = {
        let _files = store::files_lock();
        let runs = load_pending(service.root());
        save_pending(service.root(), &[]);
        runs
    };
    for run in missed {
        service.note(&run.bot, &format!("Missed while the app was closed: {}", run.title));
        record_failure(service.root(), &run.id, MISSED_TEXT);
    }
}

/// "Run now": sends the routine's prompt at once. The schedule does not move.
pub fn run_now(service: &Service, id: &str, now_ms: i64) -> Result<SendOutcome, StoreError> {
    let routine = {
        let _files = store::files_lock();
        let mut file = load_file(service.root())?;
        let index = find_in(&file.routines, id)?;
        file.routines[index].last_run_ms = Some(now_ms);
        let routine = file.routines[index].clone();
        save_file(service.root(), &file.routines)?;
        routine
    };
    // A watching routine looks first and gives the bot what it sees now (even if nothing changed); what it saw is saved, so the
    // next scheduled look compares with what the bot has just been shown.
    let mut context: Option<String> = None;
    if let Some(watch) = &routine.watch {
        let run = service.run_check(&routine.bot, &watch.check).map_err(|e| StoreError::new(e.code, e.message))?;
        let before = watch.fingerprint.is_some().then_some(watch.last_output.as_str());
        context = Some(watch_record("A look, because it was asked for", &watch.check, before, &run.output));
        if run.fingerprint != "unanswered" {
            let _files = store::files_lock();
            let mut file = load_file(service.root())?;
            if let Some(saved) = file.routines.iter_mut().find(|r| r.id == routine.id).and_then(|r| r.watch.as_mut()) {
                saved.fingerprint = Some(run.fingerprint.clone());
                saved.last_output = run.output.chars().take(MAX_WATCH_OUTPUT_CHARS).collect();
                saved.last_checked_ms = Some(now_ms);
                saved.failures = 0;
                save_file(service.root(), &file.routines)?;
            }
        }
    }
    let sent = service.send_as(&routine.bot, routine.prompt.trim(), Origin::Routine { title: &routine.title, context: context.as_deref() });
    record_run(service.root(), run_record(&routine.id, &routine.bot, now_ms, sent.as_ref()));
    sent
}

// ---- run history (the last runs of each routine, for the Routines tab) --------------------------------

/// How many runs of one routine are kept.
pub const MAX_RUNS_PER_ROUTINE: usize = 10;
/// A ceiling for the whole file (runs of deleted routines would otherwise stay for ever).
const MAX_RUNS_TOTAL: usize = 400;
const WAITING_TEXT: &str = "Sent. Waiting for the reply.";
const REPLIED_TEXT: &str = "The bot replied.";
const MISSED_TEXT: &str = "Missed: the app was closed before the bot took this run.";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RunResult {
    Ok,
    Failed,
    Skipped,
}

/// One run as stored. `message_id` is the saved "Routine: ..." message the brain's answer names; `reply_id` is the
/// bot's reply message in the thread. A record with an empty `routine` is a brain answer that arrived before its run
/// was written down; `record_run` joins them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub routine: String,
    pub bot: String,
    pub at_ms: i64,
    pub result: RunResult,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct RunsFile {
    v: u32,
    runs: Vec<RunRecord>,
}

/// What the page shows for a run.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunView {
    pub routine_id: String,
    pub at_ms: i64,
    pub result: RunResult,
    pub text: String,
    /// The bot's reply message in the thread, once there is one.
    pub reply_id: Option<String>,
}

pub fn runs_path(root: &Path) -> std::path::PathBuf {
    root.join("routine_runs.json")
}

fn load_runs(root: &Path) -> Result<RunsFile, StoreError> {
    let file: Option<RunsFile> = store::read_json(&runs_path(root), "routine_runs.json")?;
    match file {
        None => Ok(RunsFile { v: FILE_VERSION, runs: Vec::new() }),
        Some(file) if file.v > FILE_VERSION => Err(invalid("routine_runs.json was written by a newer Puppet Master.")),
        Some(file) => Ok(file),
    }
}

/// Changes the history under the store lock. A file that cannot be read is left alone (a run still goes ahead).
fn edit_runs(root: &Path, change: impl FnOnce(&mut Vec<RunRecord>)) {
    let _files = store::files_lock();
    let mut file = match load_runs(root) {
        Ok(file) => file,
        Err(e) => {
            tracing::warn!(%e, "could not read the routine run history; it is left as it is");
            return;
        }
    };
    change(&mut file.runs);
    if let Err(e) = store::write_json(&runs_path(root), &file, "routine_runs.json") {
        tracing::warn!(%e, "could not save the routine run history");
    }
}

/// Adds a run, joins it with an answer that came first, and keeps the last 10 of its routine.
pub fn record_run(root: &Path, mut run: RunRecord) {
    edit_runs(root, |runs| {
        if let Some(id) = run.message_id.as_deref() {
            if let Some(at) = runs.iter().position(|r| r.routine.is_empty() && r.message_id.as_deref() == Some(id)) {
                let early = runs.remove(at);
                run.result = early.result;
                run.text = early.text;
                run.reply_id = early.reply_id;
            }
        }
        let routine = run.routine.clone();
        runs.push(run);
        while runs.iter().filter(|r| r.routine == routine).count() > MAX_RUNS_PER_ROUTINE {
            if let Some(oldest) = runs.iter().position(|r| r.routine == routine) {
                runs.remove(oldest);
            }
        }
        while runs.len() > MAX_RUNS_TOTAL {
            runs.remove(0);
        }
    });
}

/// The brain answered the run `message_id`; `reply_id` is its reply message.
pub fn record_reply(root: &Path, message_id: &str, reply_id: Option<&str>) {
    settle(root, message_id, RunResult::Ok, REPLIED_TEXT, reply_id);
}

/// The brain could not do the run `message_id`. `text` is a plain sentence.
pub fn record_failure(root: &Path, message_id: &str, text: &str) {
    settle(root, message_id, RunResult::Failed, text, None);
}

fn settle(root: &Path, message_id: &str, result: RunResult, text: &str, reply_id: Option<&str>) {
    edit_runs(root, |runs| {
        let reply = reply_id.map(str::to_string);
        match runs.iter_mut().find(|r| r.message_id.as_deref() == Some(message_id)) {
            Some(run) => {
                run.result = result;
                run.text = text.to_string();
                run.reply_id = reply;
            }
            // The answer came before the run was written down: keep it until `record_run` joins them.
            None => runs.push(RunRecord {
                routine: String::new(),
                bot: String::new(),
                at_ms: 0,
                result,
                text: text.to_string(),
                message_id: Some(message_id.to_string()),
                reply_id: reply,
            }),
        }
    });
}

/// The history line for a run the app handed to the bot, or could not.
fn run_record(routine: &str, bot: &str, at_ms: i64, sent: Result<&SendOutcome, &StoreError>) -> RunRecord {
    let (result, text, message_id) = match sent {
        Ok(out) if out.delivered => (RunResult::Ok, WAITING_TEXT.to_string(), Some(out.id.clone())),
        Ok(out) => {
            let (result, text) = unsent_run(out.state);
            (result, text, None)
        }
        Err(e) => {
            let (result, text) = refused_run(e.code);
            (result, text, None)
        }
    };
    RunRecord { routine: routine.to_string(), bot: bot.to_string(), at_ms, result, text, message_id, reply_id: None }
}

/// Why a run that was saved still did not reach the bot's brain.
pub fn unsent_run(state: super::service::BrainState) -> (RunResult, String) {
    use super::service::BrainState as B;
    match state {
        B::DailyLimit => (RunResult::Skipped, "Skipped: the daily limit was reached.".into()),
        B::NeedsKey => (RunResult::Failed, "The bot has no model key yet. Add one in Settings.".into()),
        B::NeedsNode => (RunResult::Failed, "Node.js is missing, so the bot cannot start.".into()),
        B::MissingProgram => (RunResult::Failed, "The bot's program is missing, so it cannot start.".into()),
        B::Stopped => (RunResult::Failed, "The bot stopped after crashing. Start it again.".into()),
        B::Agents => (RunResult::Failed, "Talking through your agents is not ready yet.".into()),
        B::Off => (RunResult::Failed, "Potato Bot is switched off.".into()),
        B::Idle | B::Starting | B::Ready | B::Restarting => (RunResult::Failed, "The bot did not take this run.".into()),
    }
}

/// Why the app refused to send a run (the code is stable, the words are for people).
pub fn refused_run(code: &str) -> (RunResult, String) {
    match code {
        "POTATO_PAUSED" => (RunResult::Skipped, "Skipped: bots were paused.".into()),
        "POTATO_RATE" => (RunResult::Skipped, "Skipped: too many messages in a minute.".into()),
        "POTATO_OFF" => (RunResult::Skipped, "Skipped: Potato Bot was switched off.".into()),
        "POTATO_TURNS" => (RunResult::Skipped, "Skipped: the bot used up its turns for today.".into()),
        "POTATO_BOT_NOT_FOUND" => (RunResult::Failed, "The bot no longer exists.".into()),
        _ => (RunResult::Failed, "The message could not be sent.".into()),
    }
}

/// The last runs of every routine of one bot, newest first.
pub fn runs_of(root: &Path, bot: &str) -> Result<Vec<RunView>, StoreError> {
    let _files = store::files_lock();
    let mut runs: Vec<RunRecord> = load_runs(root)?.runs.into_iter().filter(|r| r.bot == bot && !r.routine.is_empty()).collect();
    runs.reverse();
    runs.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
    Ok(runs.into_iter().map(|r| RunView { routine_id: r.routine, at_ms: r.at_ms, result: r.result, text: r.text, reply_id: r.reply_id }).collect())
}


// ---- the bot's own schedule tools (the model calls these on an owner's turn) ----------------------

/// The four schedule tools. Bot-level changes go through the same file and checks as the routines screen.
pub const TOOL_NAMES: [&str; 4] = ["schedule_list", "schedule_create", "schedule_update", "schedule_cancel"];

pub fn is_tool(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

fn tool_view(view: &RoutineView) -> Value {
    json!({
        "id": view.id,
        "title": view.title,
        "prompt": view.prompt,
        "when": view.summary,
        "status": view.status,
        "nextRunMs": view.next_run_ms,
        "watching": view.watch.as_ref().map(|w| w.check.clone()),
    })
}

/// A routine of this bot, or the same not-found answer for anyone else's (a bot never sees another bot's routines).
fn owned_routine(root: &Path, bot: &str, id: &str) -> Result<Routine, StoreError> {
    let _files = store::files_lock();
    let file = load_file(root)?;
    let index = find_in(&file.routines, id)?;
    let routine = file.routines[index].clone();
    if routine.bot != bot {
        return Err(StoreError::new("POTATO_ROUTINE_NOT_FOUND", "No routine has that name."));
    }
    Ok(routine)
}

fn tool_text(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(|s| s.trim().to_string())
}

/// The form a tool call describes. Missing fields come from `current` when there is one (an update), else they are
/// an error. The schedule is `{ "kind": "weekdays", "time": "09:00" }` and so on: the same shape the file stores.
fn input_from(args: &Value, current: Option<&Routine>) -> Result<RoutineInput, StoreError> {
    let title = match tool_text(args, "title") {
        Some(title) => title,
        None => current.map(|r| r.title.clone()).ok_or_else(|| invalid("Give the routine a name."))?,
    };
    let prompt = match tool_text(args, "prompt") {
        Some(prompt) => prompt,
        None => current.map(|r| r.prompt.clone()).ok_or_else(|| invalid("Write what the bot should do."))?,
    };
    let schedule = match args.get("schedule") {
        Some(value) => serde_json::from_value::<Schedule>(value.clone()).map_err(|_| {
            invalid("Pick a schedule: hourly, daily (time), weekdays (time), everyMinutes (minutes), once (at) or cron (expr).")
        })?,
        None => current.map(|r| r.schedule.clone()).ok_or_else(|| invalid("Pick when it runs."))?,
    };
    // `watch: { check, contains? }` makes the routine run only when the check's result changes; `watch: null` takes it off.
    let watch = match args.get("watch") {
        None => current.and_then(|r| r.watch.as_ref()).map(|w| WatchInput { check: w.check.text.clone(), contains: w.check.contains.clone() }),
        Some(Value::Null) => None,
        Some(value) => Some(serde_json::from_value::<WatchInput>(value.clone()).map_err(|_| invalid("watch is { check, contains } : a read-only command or a web address, and for an address optionally words the page must hold."))?),
    };
    Ok(RoutineInput { title, prompt, schedule, watch })
}

/// Runs one schedule tool for `bot`. The caller has checked the owner's trust and that the turn is not a routine's own
/// run. Times are the computer's local clock, as the routines screen uses.
pub fn call(root: &Path, bot: &str, tool: &str, args: &Value, now_ms: i64) -> Result<Value, StoreError> {
    match tool {
        "schedule_list" => {
            let views = list(root, Some(bot))?;
            Ok(json!({ "routines": views.iter().map(tool_view).collect::<Vec<_>>(), "maxRoutines": MAX_ROUTINES_PER_BOT }))
        }
        "schedule_create" => {
            let input = input_from(args, None)?;
            let view = create(root, bot, &input, now_ms)?;
            Ok(json!({ "routine": tool_view(&view) }))
        }
        "schedule_update" => {
            let id = tool_text(args, "id").unwrap_or_default();
            let current = owned_routine(root, bot, &id)?;
            let input = input_from(args, Some(&current))?;
            let view = update(root, &id, &input, now_ms)?;
            Ok(json!({ "routine": tool_view(&view) }))
        }
        "schedule_cancel" => {
            let id = tool_text(args, "id").unwrap_or_default();
            owned_routine(root, bot, &id)?;
            // Into Recently deleted, like the screen's Delete: it can be restored for 7 days.
            let item = super::trash::trash_routine(root, &id, now_ms)?;
            Ok(json!({ "cancelled": id, "trashId": item.id }))
        }
        _ => Err(StoreError::new(INVALID, "Unknown schedule tool.")),
    }
}
