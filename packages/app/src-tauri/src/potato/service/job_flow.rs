//! Jobs in the engine (Professional mode, P3). A job is a saved chain of turns (`jobs.rs` keeps the record). The host,
//! not the model, starts each turn:
//!
//! - When a turn of the job ends (its reply and then its usage, or an error), the host reads the job file, adds the time
//!   and money, decides whether the turn made progress, and either stops the job (a limit, no progress) or sends the next
//!   turn with the goal, the to-do list, the step log and the spending as its context.
//! - After a restart, a job marked running starts again once. A tick looks at the job files every few seconds, so a
//!   brain that crashed, or an app that was closed, does not leave a job hanging.
//! - One turn is open at a time (`BotRt::job_turn`). A turn is claimed under the engine's lock before anything is sent,
//!   so two resumes at the same moment start one turn. A person's message in between queues behind the open turn.
//! - Stop and Pause all end the job for good. Quitting the app does not: the job stays running and resumes.
//! - The job tools are the owner's own turns in the bot's private chat. A group, a routine's run and a stranger are
//!   refused here, whatever the brain offers.
//!
//! Lock order: the engine's lock (`inner`) is never taken while the store lock (`files_lock`) is held. Job turns do not
//! use the brain-started count of the day (`cost::take_brain_turn`); they stop at the job's limits and the daily pause.

use super::*;
use crate::potato::jobs::{reason, Job, Status};

/// How long after a reply the usage is waited for before the next turn is decided anyway.
const USAGE_WAIT_MS: i64 = 15_000;
/// How often the job files are looked at for a job nobody is running.
const SCAN_EVERY_MS: i64 = 5_000;
const GOAL_NOTE_CHARS: usize = 120;

/// The turn the bot's job waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum JobTurn {
    /// The turn in which the job was started: it ends with the next turn that ends in the private chat.
    Starting,
    /// A next turn is being decided and sent. Nothing else may start one meanwhile.
    Claimed,
    /// A job turn in the brain's inbox or running, by the id of its `send`.
    Turn(String),
}

/// A turn ended while a job ran; its usage is still to come.
#[derive(Debug, Clone)]
pub(super) struct JobAfter {
    in_reply_to: Option<String>,
    failed: bool,
    at: i64,
}

/// Does the end of the turn `id` close the open job turn?
fn closes(open: &Option<JobTurn>, id: Option<&str>) -> bool {
    match open {
        Some(JobTurn::Starting) => true,
        Some(JobTurn::Turn(turn)) => id == Some(turn.as_str()),
        Some(JobTurn::Claimed) | None => false,
    }
}

enum Plan {
    Nothing,
    Stop(Job, &'static str),
    /// A limit or a stuck job: a Keep going card was raised and the job waits for the answer.
    Ask,
    Send(Job),
}

impl Service {
    // ---- the tools ------------------------------------------------------------------------------------------------

    /// The answer to `job_start`, `job_log` or `job_done`.
    pub(super) fn job_tool_item(&self, ctx: &sandbox_calls::Context, call_id: &str, tool: &str, args: &Value) -> Value {
        let refuse = |message: &str| protocol::sandbox_result_err(call_id, "TOOL_NOT_ALLOWED", message, false);
        let from_routine = self.inner.lock().bots.get(&ctx.bot).is_some_and(|rt| rt.turn_routine.contains(&ctx.conversation));
        if ctx.conversation != store::app_conversation(&ctx.bot) {
            return refuse("A job runs in your private chat with the bot, not in a group.");
        }
        if !ctx.trust.is_owner() {
            return refuse("Only the owner can start a job in this chat.");
        }
        if from_routine {
            return refuse("A routine cannot start a job. Only the owner can.");
        }
        let now = self.host.now_ms();
        match jobs::call(&self.root, &ctx.bot, &ctx.conversation, tool, args, now) {
            Ok(result) => {
                let name = self.name_of(&ctx.bot);
                match tool {
                    "job_start" => {
                        // The turn that is running now is the job's first.
                        if let Some(rt) = self.inner.lock().bots.get_mut(&ctx.bot) {
                            if rt.job_turn.is_none() {
                                rt.job_turn = Some(JobTurn::Starting);
                            }
                        }
                        let goal = jobs::load(&self.root, &ctx.bot).map(|j| j.goal).unwrap_or_default();
                        let goal: String = goal.chars().take(GOAL_NOTE_CHARS).collect();
                        self.note(&ctx.bot, &format!("{name} started a job: {goal}"));
                    }
                    "job_done" => self.note(&ctx.bot, &format!("{name} finished the job.")),
                    _ => {}
                }
                self.host.changed(&ctx.bot);
                protocol::sandbox_result_ok(call_id, result)
            }
            Err(e) => protocol::sandbox_result_err(call_id, e.code, &e.message, false),
        }
    }

    // ---- the end of a turn ----------------------------------------------------------------------------------------

    /// A turn of the private chat ended (a reply, or an error with `failed`). If the bot has a running job, the next step
    /// waits for the turn's usage; without one, a job turn that was open is closed.
    pub(super) fn job_turn_ended(&self, bot: &str, in_reply_to: Option<&str>, failed: bool, now: i64) {
        // An older turn whose usage never came is decided first.
        self.job_after_turn(bot, now);
        let running = jobs::load(&self.root, bot).is_some_and(|j| j.is_running());
        let mut g = self.inner.lock();
        let Some(rt) = g.bots.get_mut(bot) else { return };
        if running {
            rt.job_after = Some(JobAfter { in_reply_to: in_reply_to.map(str::to_string), failed, at: now });
        } else if closes(&rt.job_turn, in_reply_to) {
            rt.job_turn = None;
        }
    }

    /// The usage of the turn that ended has arrived (or has not come in time): decide the job's next turn.
    pub(super) fn job_after_turn(&self, bot: &str, now: i64) {
        let (after, matched, claimed) = {
            let mut g = self.inner.lock();
            let Some(rt) = g.bots.get_mut(bot) else { return };
            let Some(after) = rt.job_after.take() else { return };
            let matched = closes(&rt.job_turn, after.in_reply_to.as_deref());
            if matched {
                rt.job_turn = None;
            }
            // Another turn is open (the person's, or the job's own): nothing to do for this one.
            let claimed = rt.job_turn.is_none();
            if claimed {
                rt.job_turn = Some(JobTurn::Claimed);
            }
            (after, matched, claimed)
        };
        if claimed {
            self.job_next(bot, now, Some((matched, after.failed)), false);
        }
    }

    /// Decides and sends the next turn of a claimed job. `ended`: the turn that just ended, and whether it failed.
    /// Returns true when a turn was started.
    fn job_next(&self, bot: &str, now: i64, ended: Option<(bool, bool)>, restarted: bool) -> bool {
        let plan = self.job_plan(bot, now, ended);
        match plan {
            Plan::Nothing => {
                self.release_claim(bot);
                false
            }
            Plan::Stop(job, why) => {
                self.release_claim(bot);
                self.job_stopped_note(bot, why, job.steps_total);
                false
            }
            Plan::Ask => {
                self.release_claim(bot);
                self.host.changed(bot);
                false
            }
            Plan::Send(job) => {
                let todo = open_work::list(&self.root, bot).unwrap_or_default();
                // A job that waited for an answer starts again with the decision in its record, not as a restart.
                let answered = job.decision.is_some();
                let restarted = restarted && !answered;
                let context = jobs::context(&job, &todo, now, restarted);
                let text = if answered { jobs::answered_text() } else { jobs::keep_going_text() };
                match self.send_as(bot, text, Origin::Job { context: &context }) {
                    Ok(outcome) if outcome.delivered => {
                        if answered {
                            jobs::clear_decision(&self.root, bot);
                        }
                        if restarted {
                            let name = self.name_of(bot);
                            self.note(bot, &format!("{name} is picking the job up again after a restart."));
                            // What the person wrote before the restart and the bot never picked up goes to the brain again, in order.
                            self.redeliver_queued(bot);
                        }
                        self.host.changed(bot);
                        true
                    }
                    Ok(outcome) => {
                        // The daily money pause asks like the other stops: Keep going or Stop. The card says it can only
                        // go on tomorrow (or once the limit is raised).
                        if outcome.state == BrainState::DailyLimit {
                            if let Some(job) = jobs::load(&self.root, bot).filter(Job::is_running) {
                                if self.raise_keep_going(bot, &job, reason::DAILY_LIMIT, now) {
                                    self.release_claim(bot);
                                    self.host.changed(bot);
                                    return false;
                                }
                            }
                            self.end_job(bot, reason::DAILY_LIMIT, now);
                        } else {
                            self.end_job(bot, reason::CANNOT_START, now);
                        }
                        false
                    }
                    // Potato Bot is off: the job waits, still running, for it to come back.
                    Err(e) if e.code == "POTATO_OFF" => {
                        self.release_claim(bot);
                        false
                    }
                    Err(e) if e.code == "POTATO_PAUSED" => {
                        self.end_job(bot, reason::PAUSED, now);
                        false
                    }
                    Err(_) => {
                        self.end_job(bot, reason::CANNOT_START, now);
                        false
                    }
                }
            }
        }
    }

    /// After a restart: the person's messages that were waiting for the bot (kept in the job record) are sent again, oldest first,
    /// with the ids they have in the thread. The running job turn reads them between its tool rounds.
    fn redeliver_queued(&self, bot: &str) {
        let Some(job) = jobs::load(&self.root, bot).filter(Job::is_running) else { return };
        for waiting in job.queued {
            if let Err(e) = self.send_as(bot, &waiting.text, Origin::Queued { id: &waiting.id }) {
                tracing::debug!(%bot, code = e.code, "could not send a waiting message again");
            }
        }
    }

    /// The job file side of the decision. Holds the store lock, never the engine's.
    fn job_plan(&self, bot: &str, now: i64, ended: Option<(bool, bool)>) -> Plan {
        let _files = store::files_lock();
        let Some(mut job) = jobs::load(&self.root, bot).filter(Job::is_running) else { return Plan::Nothing };
        match ended {
            Some((true, failed)) => {
                job.close_clock(now);
                job.turns += 1;
                let progressed = !failed && (job.steps_total != job.snapshot_steps || jobs::todo_fingerprint(&self.root, bot) != job.snapshot_todo);
                job.no_progress = if progressed { 0 } else { job.no_progress + 1 };
            }
            // No turn was open (a restart, a crash): the time the app was closed is not the job's.
            _ => job.turn_started_ms = None,
        }
        let money_cap = store::load_settings(&self.root).map(|s| s.daily_limit().is_some()).unwrap_or(true);
        let stop = if job.no_progress >= jobs::MAX_NO_PROGRESS { Some(reason::NO_PROGRESS) } else { job.limit_hit_with(now, money_cap) };
        job.updated_ms = now;
        if let Some(why) = stop {
            job.close_clock(now);
            // A limit or a stuck job asks the owner first: Keep going or Stop.
            if matches!(why, reason::TIME_LIMIT | reason::MONEY_LIMIT | reason::NO_PROGRESS) && self.raise_keep_going(bot, &job, why, now) {
                return Plan::Ask;
            }
            job.status = Status::Stopped;
            job.stop_reason = Some(why.to_string());
            return match jobs::save(&self.root, bot, &job) {
                Ok(()) => Plan::Stop(job, why),
                Err(_) => Plan::Nothing,
            };
        }
        job.snapshot_steps = job.steps_total;
        job.snapshot_todo = jobs::todo_fingerprint(&self.root, bot);
        job.turn_started_ms = Some(now);
        match jobs::save(&self.root, bot, &job) {
            Ok(()) => Plan::Send(job),
            Err(e) => {
                tracing::warn!(%bot, error = %e.message, "could not save the job");
                Plan::Nothing
            }
        }
    }

    /// Gives back a claim that was not used. A turn that was sent has replaced it.
    fn release_claim(&self, bot: &str) {
        if let Some(rt) = self.inner.lock().bots.get_mut(bot) {
            if rt.job_turn == Some(JobTurn::Claimed) {
                rt.job_turn = None;
            }
        }
    }

    // ---- stopping --------------------------------------------------------------------------------------------------

    pub(super) fn job_stopped_note(&self, bot: &str, why: &str, steps: u64) {
        self.note(bot, &jobs::stop_note(&self.name_of(bot), why, steps));
        self.host.changed(bot);
    }

    /// Stops the bot's running job after the fact (the decision was already made): saves, says so, closes the turn.
    fn end_job(&self, bot: &str, why: &'static str, now: i64) {
        self.release_claim(bot);
        if let Some(job) = jobs::stop(&self.root, bot, why, now) {
            self.job_stopped_note(bot, why, job.steps_total);
        }
    }

    /// Stops the bot's job for good, if it has one that runs or waits. No more turns follow.
    pub(super) fn stop_job(&self, bot: &str, why: &'static str) {
        let now = self.host.now_ms();
        let stopped = jobs::stop(&self.root, bot, why, now);
        // Open cards close as cancelled: an answer to them is refused from now on, and what they allowed is forgotten.
        self.cancel_cards(bot);
        if let Some(rt) = self.inner.lock().bots.get_mut(bot) {
            rt.job_turn = None;
            rt.job_after = None;
        }
        if let Some(job) = stopped {
            self.job_stopped_note(bot, why, job.steps_total);
        }
    }

    /// Stops the job of every bot ("Stop everything").
    pub fn stop_jobs(&self, why: &'static str) {
        let Ok(bots) = store::load_bots(&self.root) else { return };
        for bot in bots {
            self.stop_job(&bot.id, why);
        }
    }

    // ---- resuming --------------------------------------------------------------------------------------------------

    /// Starts the next turn of the bot's job when none is open: after a restart, a crash, or a failed start. At most one
    /// of any number of callers at the same moment wins. Returns true when a turn was started.
    pub(super) fn job_resume(&self, bot: &str, now: i64) -> bool {
        if !self.brains.enabled() || !jobs::load(&self.root, bot).is_some_and(|j| j.is_running()) {
            return false;
        }
        {
            let mut g = self.inner.lock();
            let rt = g.bots.entry(bot.to_string()).or_insert_with(BotRt::new);
            if rt.job_turn.is_some() || rt.job_after.is_some() {
                return false;
            }
            rt.job_turn = Some(JobTurn::Claimed);
        }
        self.job_next(bot, now, None, true)
    }

    /// Every bot's running job without an open turn gets one. Returns how many were started.
    pub fn resume_jobs(&self) -> usize {
        let now = self.host.now_ms();
        let Ok(bots) = store::load_bots(&self.root) else { return 0 };
        bots.iter()
            .filter(|b| {
                // A card answered right before a crash: its job goes on now (the answer would have done this).
                if let Some(effect) = crate::potato::cards::recover_waiting(&self.root, &b.id, now) {
                    self.host.changed(&b.id);
                    if let crate::potato::cards::JobEffect::Stopped(job) = effect {
                        let why = job.stop_reason.clone().unwrap_or_else(|| reason::STOPPED_BY_YOU.to_string());
                        self.job_stopped_note(&b.id, &why, job.steps_total);
                    }
                }
                self.job_resume(&b.id, now)
            })
            .count()
    }

    /// Called by the tick: a turn whose usage never came is decided, and the job files are looked at every few seconds.
    pub(super) fn job_tick(&self, now: i64) {
        let stale: Vec<String> = {
            let g = self.inner.lock();
            g.bots.iter().filter(|(_, rt)| rt.job_after.as_ref().is_some_and(|a| now - a.at >= USAGE_WAIT_MS)).map(|(bot, _)| bot.clone()).collect()
        };
        for bot in stale {
            self.job_after_turn(&bot, now);
        }
        let last = self.job_scan_ms.load(Ordering::SeqCst);
        if last == i64::MIN || now - last >= SCAN_EVERY_MS {
            self.job_scan_ms.store(now, Ordering::SeqCst);
            self.resume_jobs();
        }
    }
}
