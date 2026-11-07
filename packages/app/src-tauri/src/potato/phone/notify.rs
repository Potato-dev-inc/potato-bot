//! Tells the phones (Web Push, sent from this PC with its own VAPID key) when a card is waiting or a job has ended.
//!
//! A thread wakes on every change, looks at each bot, and compares with what it saw last time (`diff`, pure). The first
//! look sets the baseline and says nothing, so starting the app never buzzes a phone about old cards. A bot whose
//! "Notify me" is Never says nothing. A phone that has the app open gets no push (the app shows it); the hub decides.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use super::gate::{Changes, Env};
use crate::potato::jobs::Status;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotSnap {
    pub id: String,
    pub name: String,
    /// Open cards: id and what they ask.
    pub open: Vec<(String, String)>,
    pub job: Option<Status>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Alert {
    NeedsYou { bot: String, name: String, action: String },
    JobEnded { bot: String, name: String, state: Status },
}

#[derive(Default)]
pub struct Mem {
    primed: bool,
    cards: HashSet<String>,
    jobs: HashMap<String, Status>,
}

fn active(s: Status) -> bool {
    matches!(s, Status::Running | Status::Waiting)
}

/// What is new since last time. One alert per new card and one per job that went from active to over.
pub fn diff(mem: &mut Mem, snaps: &[BotSnap]) -> Vec<Alert> {
    let mut alerts = Vec::new();
    let mut cards = HashSet::new();
    let mut jobs = HashMap::new();
    for s in snaps {
        for (id, action) in &s.open {
            cards.insert(id.clone());
            if mem.primed && !mem.cards.contains(id) {
                alerts.push(Alert::NeedsYou { bot: s.id.clone(), name: s.name.clone(), action: action.clone() });
            }
        }
        if let Some(job) = s.job {
            jobs.insert(s.id.clone(), job);
            if mem.primed && !active(job) && mem.jobs.get(&s.id).copied().is_some_and(active) {
                alerts.push(Alert::JobEnded { bot: s.id.clone(), name: s.name.clone(), state: job });
            }
        }
    }
    mem.cards = cards;
    mem.jobs = jobs;
    mem.primed = true;
    alerts
}

/// Title, body and tag. Short, plain, and the body is the first line of what the card asks.
pub fn copy(alert: &Alert) -> (String, String, String) {
    match alert {
        Alert::NeedsYou { bot, name, action } => {
            let line: String = action.lines().next().unwrap_or("").chars().take(120).collect();
            (format!("{name} needs you"), line, format!("pm-card-{bot}"))
        }
        Alert::JobEnded { bot, name, state } => {
            let (title, body) = match state {
                Status::Done => (format!("{name} finished"), "The job is done."),
                Status::Failed => (format!("{name} hit a problem"), "The job stopped because of an error."),
                _ => (format!("{name} stopped"), "The job was stopped."),
            };
            (title, body.to_string(), format!("pm-job-{bot}"))
        }
    }
}

pub fn bot_of(alert: &Alert) -> &str {
    match alert {
        Alert::NeedsYou { bot, .. } | Alert::JobEnded { bot, .. } => bot,
    }
}

pub fn snapshot(service: &crate::potato::service::Service) -> Vec<BotSnap> {
    let Ok(state) = service.state() else { return Vec::new() };
    state
        .bots
        .iter()
        .map(|b| BotSnap {
            id: b.id.clone(),
            name: b.name.clone(),
            open: service.open_cards(&b.id).unwrap_or_default().into_iter().map(|c| (c.id, c.action)).collect(),
            job: service.job_view(&b.id).ok().flatten().map(|j| j.state),
        })
        .collect()
}

/// Sends the alerts to every phone that should get them. Blocking (a push is a network call).
pub fn deliver(env: &dyn Env, alerts: &[Alert]) {
    let Some(hub) = env.push() else { return };
    let Some(service) = env.service() else { return };
    for alert in alerts {
        if crate::potato::notify::get(service.root(), bot_of(alert)).is_ok_and(|n| n == crate::potato::notify::Notify::Never) {
            continue;
        }
        let (title, body, tag) = copy(alert);
        let deliveries = hub.alert_deliveries(&title, &body, &tag, bot_of(alert), env.now_ms());
        crate::push::send_deliveries(&hub, deliveries);
    }
}

/// The thread: wakes on every change (or every 10 s), alerts on what is new.
pub fn run(env: Arc<dyn Env>, changes: Arc<Changes>) {
    let mut mem = Mem::default();
    let mut seen = 0;
    loop {
        seen = changes.wait(seen, Duration::from_secs(10));
        let Some(service) = env.service() else { continue };
        let alerts = diff(&mut mem, &snapshot(&service));
        if !alerts.is_empty() {
            deliver(env.as_ref(), &alerts);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(id: &str, open: &[(&str, &str)], job: Option<Status>) -> BotSnap {
        BotSnap { id: id.into(), name: id.to_uppercase(), open: open.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(), job }
    }

    #[test]
    fn the_first_look_says_nothing_even_with_old_cards() {
        let mut mem = Mem::default();
        assert!(diff(&mut mem, &[snap("a", &[("c1", "Deploy")], Some(Status::Running))]).is_empty());
        assert!(diff(&mut mem, &[snap("a", &[("c1", "Deploy")], Some(Status::Running))]).is_empty(), "nothing new");
    }

    #[test]
    fn a_new_card_says_needs_you_once() {
        let mut mem = Mem::default();
        diff(&mut mem, &[snap("a", &[], None)]);
        let alerts = diff(&mut mem, &[snap("a", &[("c1", "Deploy to staging\nmore")], Some(Status::Waiting))]);
        assert_eq!(alerts, vec![Alert::NeedsYou { bot: "a".into(), name: "A".into(), action: "Deploy to staging\nmore".into() }]);
        assert!(diff(&mut mem, &[snap("a", &[("c1", "Deploy")], Some(Status::Waiting))]).is_empty(), "the same card does not buzz again");
        let (title, body, tag) = copy(&alerts[0]);
        assert_eq!((title.as_str(), body.as_str(), tag.as_str()), ("A needs you", "Deploy to staging", "pm-card-a"));
        // An answered card then a new one: the new one counts.
        diff(&mut mem, &[snap("a", &[], None)]);
        assert_eq!(diff(&mut mem, &[snap("a", &[("c2", "Another")], None)]).len(), 1);
    }

    #[test]
    fn a_job_that_ends_says_how_and_a_job_that_keeps_going_or_waits_does_not() {
        let mut mem = Mem::default();
        diff(&mut mem, &[snap("a", &[], Some(Status::Running))]);
        assert!(diff(&mut mem, &[snap("a", &[], Some(Status::Waiting))]).is_empty(), "waiting for a card is not an end");
        let done = diff(&mut mem, &[snap("a", &[], Some(Status::Done))]);
        assert_eq!(copy(&done[0]).0, "A finished");
        assert!(diff(&mut mem, &[snap("a", &[], Some(Status::Done))]).is_empty(), "told once");
        diff(&mut mem, &[snap("a", &[], Some(Status::Running))]);
        assert_eq!(copy(&diff(&mut mem, &[snap("a", &[], Some(Status::Failed))])[0]).0, "A hit a problem");
        diff(&mut mem, &[snap("a", &[], Some(Status::Running))]);
        assert_eq!(copy(&diff(&mut mem, &[snap("a", &[], Some(Status::Stopped))])[0]).0, "A stopped");
    }

    #[test]
    fn a_long_question_is_cut_to_one_short_line() {
        let long = "x".repeat(400);
        let (_, body, _) = copy(&Alert::NeedsYou { bot: "a".into(), name: "A".into(), action: long });
        assert_eq!(body.chars().count(), 120);
    }
}
