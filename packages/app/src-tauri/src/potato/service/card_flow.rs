//! Decision cards in the engine (Professional mode, P4). The pieces that need the engine, not just the files:
//!
//! - the brain's two card tools, `request_approval` and `ask_user` (as a card with choices). Both come as a `sandbox_call`,
//!   like the job tools; both are the owner's own turn in the private chat, never a group, a routine or a stranger;
//! - the answer, `answer_card`, which only the app command `potato_answer_card` calls: it records the answer (`cards.rs`)
//!   and then either starts the job's next turn with the decision in its record, or, with no job waiting, starts a normal
//!   turn that carries the decision;
//! - Keep going cards for a job that reached a limit or got stuck;
//! - closing open cards when the person presses Stop or Pause all.
//!
//! The brain has no tool, event or message that answers a card. A line the brain or a chat writes that says "approved" is
//! only text.

use super::*;
use crate::potato::cards::{self, JobEffect, Kind};
use crate::potato::{bot_settings, mcp};
use serde_json::json;

/// What the app command gives back after an answer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerOutcome {
    pub card_id: String,
    /// `allow_once`, `allow_job`, `deny` or `choice:N`.
    pub answer: String,
    /// The bot's job was started again with the decision.
    pub job_resumed: bool,
    /// A normal turn carrying the decision was sent (no job was waiting).
    pub turn_started: bool,
}

fn text_arg(args: &Value, key: &str) -> String {
    args.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn choices_arg(args: &Value) -> Vec<String> {
    args.get("choices")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

impl Service {
    // ---- the brain's tools -----------------------------------------------------------------------------------------

    /// The answer to `request_approval` or `ask_user`.
    pub(super) fn card_tool_item(&self, ctx: &sandbox_calls::Context, call_id: &str, tool: &str, args: &Value) -> Value {
        let refuse = |message: &str| protocol::sandbox_result_err(call_id, "TOOL_NOT_ALLOWED", message, false);
        let from_routine = self.inner.lock().bots.get(&ctx.bot).is_some_and(|rt| rt.turn_routine.contains(&ctx.conversation));
        let in_dm = ctx.conversation == store::app_conversation(&ctx.bot);
        let owner = ctx.trust.is_owner() && !from_routine;
        let mut connector: Option<String> = None;
        let (kind, action, why, choices) = if tool == mcp::CONNECTOR_TOOL {
            if !in_dm || !owner {
                return refuse("A connector card is for your private chat with the owner, not a group.");
            }
            let (label, listed) = mcp::connector_label(&text_arg(args, "service"));
            if label.is_empty() {
                return refuse("Say which service you need, for example GitHub.");
            }
            let have = bot_settings::load(&self.root, &ctx.bot).map(|s| s.mcp_servers.iter().any(|x| x.connector && x.enabled && x.name == label.to_ascii_lowercase())).unwrap_or(false);
            if have {
                return protocol::sandbox_result_ok(call_id, json!({ "card": false, "note": "That connector is already connected. Use mcp_list_tools to see its tools." }));
            }
            let mut why: String = text_arg(args, "why").chars().filter(|c| !c.is_control() || *c == '\n').take(300).collect();
            if why.trim().is_empty() {
                why = if listed { "It is a connector in Settings.".to_string() } else { "We have no one-click connector for it; use Add your own in Settings.".to_string() };
            }
            connector = Some(label.clone());
            (Kind::Question, format!("{} needs {label}", self.name_of(&ctx.bot)), why, vec!["Open Connectors".to_string(), "Not now".to_string()])
        } else if tool == "ask_user" {
            // Not in the app's private chat (a phone chat, say), or nothing to click: the brain asks in plain text instead.
            let choices = choices_arg(args);
            if !in_dm || !owner || choices.is_empty() {
                return protocol::sandbox_result_ok(call_id, json!({ "card": false }));
            }
            (Kind::Question, text_arg(args, "question"), String::new(), choices)
        } else {
            if !in_dm {
                return refuse("An approval card is for your private chat with the owner, not a group.");
            }
            if !owner {
                return refuse("Only the owner's own turn can ask for an approval card.");
            }
            let choices = choices_arg(args);
            (if choices.is_empty() { Kind::Approval } else { Kind::Question }, text_arg(args, "action"), text_arg(args, "why"), choices)
        };
        let now = self.host.now_ms();
        let new = cards::New {
            kind,
            action,
            why,
            choices,
            conversation: ctx.conversation.clone(),
            command_hash: None,
            classes: Vec::new(),
            keep_going: None,
            connector,
            login: None,
            mcp_add: None,
            speaker: self.name_of(&ctx.bot),
            home: self.home_of(&ctx.bot),
        };
        match cards::raise(&self.root, &ctx.bot, new, now) {
            Ok(raised) => {
                self.host.changed(&ctx.bot);
                protocol::sandbox_result_ok(
                    call_id,
                    json!({
                        "card": true,
                        "cardId": raised.card.id,
                        "note": "A card is waiting in the chat. Your turn ends now; the owner's answer comes as your next message. Do not claim anything was approved before that."
                    }),
                )
            }
            Err(e) => protocol::sandbox_result_err(call_id, e.code, &e.message, false),
        }
    }

    // ---- answering -------------------------------------------------------------------------------------------------

    /// The person's answer to a card (`potato_answer_card`). One answer only; the card must be this bot's and open.
    pub fn answer_card(&self, bot: &str, card_id: &str, answer: &str) -> Result<AnswerOutcome> {
        self.require_bot(bot)?;
        let now = self.host.now_ms();
        let mut done = cards::answer(&self.root, bot, card_id, answer, self.home_of(bot).as_deref(), now)?;
        // An add-a-tool-server card: only this button, pressed by the owner, adds the server, with no token and not Local.
        if let (Some(add), cards::Answer::Choice(0)) = (done.card.mcp_add.clone(), done.answer) {
            done.decision = match mcp::add_remote(&self.root, bot, &add.name, &add.url, false, None) {
                Ok(()) => format!(
                    "Your owner chose \"Add it\": the tool server {} is added (no sign-in or token yet). If it needs one, tell your owner to use Sign in or paste a token in Settings > Connectors; never ask for it in chat. Then use mcp_list_tools.",
                    add.name
                ),
                Err(e) => format!("Your owner chose \"Add it\", but the tool server {} could not be added: {} Tell your owner.", add.name, e.message),
            };
        }
        Ok(self.settle_answer(bot, done, now))
    }

    /// `add_mcp_server` with a link: a card the owner confirms. Nothing is added here. The link was checked (public https, no key in
    /// it) and the name is free; the owner's private chat only.
    pub(super) fn mcp_link_card(&self, ctx: &sandbox_calls::Context, call_id: &str, args: &Value) -> Value {
        let refuse = |code: &'static str, message: &str| protocol::sandbox_result_err(call_id, code, message, false);
        let from_routine = self.inner.lock().bots.get(&ctx.bot).is_some_and(|rt| rt.turn_routine.contains(&ctx.conversation));
        if ctx.conversation != store::app_conversation(&ctx.bot) || !ctx.trust.is_owner() || from_routine {
            return refuse("TOOL_NOT_ALLOWED", "A tool server card is for your private chat with the owner, not a group or a routine.");
        }
        let (name, url) = match mcp::suggest_link(&self.root, &ctx.bot, args) {
            Ok(pair) => pair,
            Err(e) => return refuse(e.code, &e.message),
        };
        let mut why: String = text_arg(args, "why").chars().filter(|c| !c.is_control() || *c == '\n').take(200).collect();
        if why.trim().is_empty() {
            why = "It has no sign-in or token yet.".to_string();
        }
        let bot_name = self.name_of(&ctx.bot);
        let new = cards::New {
            kind: Kind::Question,
            action: format!("{bot_name} wants to add the tool server {name}"),
            why: format!("{url} - {why}"),
            choices: vec!["Add it".to_string(), "Not now".to_string()],
            conversation: ctx.conversation.clone(),
            command_hash: None,
            classes: Vec::new(),
            keep_going: None,
            connector: None,
            login: None,
            mcp_add: Some(cards::McpAdd { name, url }),
            speaker: bot_name,
            home: self.home_of(&ctx.bot),
        };
        match cards::raise(&self.root, &ctx.bot, new, self.host.now_ms()) {
            Ok(raised) => {
                self.host.changed(&ctx.bot);
                protocol::sandbox_result_ok(
                    call_id,
                    json!({
                        "card": true,
                        "cardId": raised.card.id,
                        "note": "A card is waiting in the chat. Your turn ends now; the owner's answer comes as your next message. Nothing is added until they press Add it."
                    }),
                )
            }
            Err(e) => protocol::sandbox_result_err(call_id, e.code, &e.message, false),
        }
    }

    /// The Always allow button (`potato_answer_card_always`). Saves a rule for this bot and the card's one command class,
    /// and lets this card's command run now. Only the app command calls this; the text path `answer_card` refuses the
    /// same words. Refused for a card that is not offered it (rm, force-push, two classes, a question).
    pub fn answer_card_always(&self, bot: &str, card_id: &str) -> Result<AnswerOutcome> {
        self.require_bot(bot)?;
        let now = self.host.now_ms();
        let done = cards::answer_always(&self.root, bot, card_id, self.home_of(bot).as_deref(), now)?;
        Ok(self.settle_answer(bot, done, now))
    }

    /// Remove (Settings > Advanced): forgets one saved Always allow rule. The next command of that class asks again.
    /// A rule that is not there is not an error. Only the app command calls this.
    pub fn remove_always_rule(&self, bot: &str, class: &str) -> Result<BotSettings> {
        self.require_bot(bot)?;
        let saved = bot_settings::update(&self.root, bot, |s| {
            s.always_allow.retain(|r| r.class != class);
            Ok(())
        })?;
        self.host.changed(bot);
        Ok(saved)
    }

    /// What an answer does after it is recorded: the job goes on (or stops), or a normal turn carries the decision.
    pub(super) fn settle_answer(&self, bot: &str, done: cards::Answered, now: i64) -> AnswerOutcome {
        self.host.changed(bot);
        let mut out = AnswerOutcome { card_id: done.card.id.clone(), answer: done.answer.wire(), job_resumed: false, turn_started: false };
        match done.job {
            JobEffect::Resumed => {
                // The job file now says running with the decision in it. If a turn is still open, or the brain is not
                // ready, the next turn end or the 5-second tick starts it: the decision is not lost.
                out.job_resumed = true;
                self.job_resume(bot, now);
            }
            JobEffect::Stopped(job) => {
                if let Some(rt) = self.inner.lock().bots.get_mut(bot) {
                    rt.job_after = None;
                    if rt.job_turn == Some(JobTurn::Claimed) {
                        rt.job_turn = None;
                    }
                }
                let why = job.stop_reason.clone().unwrap_or_else(|| jobs::reason::STOPPED_BY_YOU.to_string());
                self.job_stopped_note(bot, &why, job.steps_total);
            }
            JobEffect::None => {
                // Stop for a Keep going card with no job is not a turn; every other answer tells the bot.
                if done.card.keep_going.is_none() {
                    match self.send_as(bot, &done.decision, Origin::Answer) {
                        Ok(outcome) => out.turn_started = outcome.delivered,
                        Err(e) => {
                            let name = self.name_of(bot);
                            self.note(bot, &format!("Your answer was saved, but {name} could not pick it up right now: {}", e.message));
                        }
                    }
                }
            }
        }
        out
    }

    // ---- Keep going ------------------------------------------------------------------------------------------------

    /// A job reached a limit or got stuck: save it as it is, raise the Keep going card (the job waits for it). False when
    /// the card could not be made; the caller then stops the job as before.
    pub(super) fn raise_keep_going(&self, bot: &str, job: &jobs::Job, why: &str, now: i64) -> bool {
        let mut job = job.clone();
        job.updated_ms = now;
        if jobs::save(&self.root, bot, &job).is_err() {
            return false;
        }
        let name = self.name_of(bot);
        let (action, why_text) = jobs::keep_going_text_for_card(&name, &job, why);
        let new = cards::New {
            kind: Kind::Question,
            action,
            why: why_text,
            choices: vec![cards::KEEP_GOING.to_string(), cards::STOP.to_string()],
            conversation: store::app_conversation(bot),
            command_hash: None,
            classes: Vec::new(),
            keep_going: Some(why.to_string()),
            connector: None,
            login: None,
            mcp_add: None,
            speaker: name,
            home: self.home_of(bot),
        };
        match cards::raise(&self.root, bot, new, now) {
            Ok(_) => {
                self.host.changed(bot);
                true
            }
            Err(e) => {
                tracing::warn!(%bot, error = %e.message, "could not raise the Keep going card");
                false
            }
        }
    }

    // ---- Stop and Pause all -----------------------------------------------------------------------------------------

    /// Closes the bot's open cards as cancelled and forgets what was allowed.
    pub(super) fn cancel_cards(&self, bot: &str) {
        let now = self.host.now_ms();
        if cards::cancel_all(&self.root, bot, self.home_of(bot).as_deref(), now) > 0 {
            self.host.changed(bot);
        }
    }

    /// The bot's job, for the job bar. `None`: it has none.
    pub fn job_view(&self, bot: &str) -> Result<Option<jobs::View>> {
        self.require_bot(bot)?;
        Ok(jobs::view(&self.root, bot))
    }

    /// The cards still waiting for the bot's owner.
    pub fn open_cards(&self, bot: &str) -> Result<Vec<cards::Card>> {
        self.require_bot(bot)?;
        Ok(cards::open_cards(&self.root, bot))
    }
}
