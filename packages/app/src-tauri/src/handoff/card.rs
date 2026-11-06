//! The cards Puppet Master raises when an agent hits its usage limit, and the answers to them.
//! Nothing here presses a key in a pane or resumes an agent by itself.

use super::detect::LimitHit;
use super::reset::{when_text, Clock};
use crate::questions::{Choice, LimitInfo, NewQuestion, Question, QuestionStore};

pub const KIND_USAGE_LIMIT: &str = "usage_limit";
pub const KIND_RESET_REMINDER: &str = "reset_reminder";
pub const CHOICE_REMIND: &str = "remind";
pub const CHOICE_STOP: &str = "stop";
pub const CHOICE_OK: &str = "ok";
/// Without a printed reset, "remind me" means this long from now.
pub const REMIND_FALLBACK_MS: u64 = 60 * 60 * 1000;
/// How long the card stays in the Inbox.
pub const CARD_TIMEOUT_S: u64 = 12 * 3600;

fn choice(id: &str, label: impl Into<String>) -> Choice {
    Choice { id: id.into(), label: label.into() }
}

/// The card for `hit` on a pane of `agent_label`. `has_run`: an agent run is bound to the pane.
pub fn limit_card(
    hit: &LimitHit,
    agent_label: &str,
    pane_id: &str,
    has_run: bool,
    clock: &dyn Clock,
) -> NewQuestion {
    let reset_ms = hit.reset_at_ms.and_then(|ms| i64::try_from(ms).ok());
    let at = reset_ms.map(|ms| when_text(ms, clock));
    let question = match &at {
        Some(at) => format!("{agent_label} ran out of usage. It can continue at {at}."),
        None => format!("{agent_label} ran out of usage. It didn't say when it can continue."),
    };
    let context = hit.self_resume_armed.then(|| format!("{agent_label} is set to continue on its own."));
    let remind = match &at {
        Some(at) => format!("Remind me at {at}"),
        None => "Remind me in 1 hour".to_string(),
    };
    let stop = if has_run { "Stop here" } else { "Dismiss" };
    NewQuestion {
        asker: None,
        agent: Some(agent_label.to_string()),
        pane_id: Some(pane_id.to_string()),
        question,
        choices: vec![choice(CHOICE_REMIND, remind), choice(CHOICE_STOP, stop)],
        allow_free_text: Some(false),
        context,
        timeout_s: Some(CARD_TIMEOUT_S),
        urgency: Some("high".into()),
        kind: None,
        limit: Some(LimitInfo {
            kind: KIND_USAGE_LIMIT.into(),
            agent_label: agent_label.to_string(),
            window: hit.window.clone(),
            reset_at_ms: hit.reset_at_ms,
            reset_text: hit.reset_text.clone(),
            self_resume_armed: hit.self_resume_armed,
            has_run,
        }),
        consent: None,
    }
}

/// The card a reminder raises once the reset time has come.
pub fn reminder_card(agent_label: &str, pane_id: &str, window: Option<String>) -> NewQuestion {
    NewQuestion {
        asker: None,
        agent: Some(agent_label.to_string()),
        pane_id: Some(pane_id.to_string()),
        question: format!("{agent_label}'s usage limit should have reset by now."),
        choices: vec![choice(CHOICE_OK, "Got it")],
        allow_free_text: Some(false),
        context: Some("You asked to be reminded. Open its terminal to carry on.".into()),
        timeout_s: Some(CARD_TIMEOUT_S),
        urgency: Some("normal".into()),
        kind: None,
        limit: Some(LimitInfo {
            kind: KIND_RESET_REMINDER.into(),
            agent_label: agent_label.to_string(),
            window,
            reset_at_ms: None,
            reset_text: None,
            self_resume_armed: false,
            has_run: false,
        }),
        consent: None,
    }
}

/// Raise `card` in `store` (and tell the windows in the live app).
pub fn raise(store: &QuestionStore, card: NewQuestion, now: u64) -> Option<Question> {
    let created = store.create(card, now).ok()?;
    crate::push::notify_question(&created);
    #[cfg(not(test))]
    if let Some(app) = crate::bridge::live_app() {
        crate::questions::routes::publish_changed(&app);
    }
    Some(created)
}

/// What a person's answer to a limit card asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Remind { pane_id: String, agent_label: String, window: Option<String>, due_ms: u64 },
    StopRun { pane_id: String },
    Nothing,
}

/// Read a resolved question: only an answer counts, a dismissal is "never mind".
pub fn decision(question: &Question, now: u64) -> Decision {
    let Some(info) = &question.limit else { return Decision::Nothing };
    let Some(answer) = question.answer.as_ref().filter(|_| question.state == crate::questions::QuestionState::Answered)
    else {
        return Decision::Nothing;
    };
    let Some(pane_id) = question.pane_id.clone() else { return Decision::Nothing };
    let choice = answer.choice_id.as_deref().unwrap_or_default();
    match (info.kind.as_str(), choice) {
        (KIND_USAGE_LIMIT, CHOICE_REMIND) => Decision::Remind {
            pane_id,
            agent_label: info.agent_label.clone(),
            window: info.window.clone(),
            due_ms: info.reset_at_ms.filter(|at| *at > now).unwrap_or(now + REMIND_FALLBACK_MS),
        },
        (KIND_USAGE_LIMIT, CHOICE_STOP) if info.has_run => Decision::StopRun { pane_id },
        _ => Decision::Nothing,
    }
}
