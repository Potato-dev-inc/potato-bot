//! The push state in one place: keys, subscriptions, which phones are looking right now, the
//! short-lived "Allow / Deny" grants, and the trigger gate. Everything takes `now_ms` so it can be
//! tested without a clock, and nothing here does network I/O (see `sender.rs`).

use super::companion_replies::{self, Message};
use super::store::{Subscription, SubscriptionStore};
use super::triggers::{coalesced_copy_view, copy_for_view, is_apple_endpoint, shown_in_full, Gate, Kind, PromptInfo, Verdict, View, Words};
use super::vapid::VapidKeys;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::HashMap;

/// How long an Allow / Deny grant can be redeemed.
pub const GRANT_TTL_MS: u64 = 10 * 60_000;
const MAX_GRANTS: usize = 64;
/// Where the service worker posts an Allow / Deny tap (relative to the PWA origin).
pub const ACT_PATH: &str = "/bridge/push/act";

/// How the hub learns who is calling. The real one asks the pairing store.
pub trait DeviceDirectory: Send + Sync {
    fn device_exists(&self, device_id: &str) -> bool;
    /// Whether the device may answer approvals (its `answer` ability). A notification only carries
    /// Allow / Deny for a device that may use them: the tap would be refused anyway.
    fn device_can_answer(&self, device_id: &str) -> bool;
    /// Whether the device may watch the companion replies (its `watch` ability).
    fn device_can_watch(&self, device_id: &str) -> bool;
}

/// What an Allow / Deny token is good for: one answer to one prompt on one pane, from one device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub device_id: String,
    pub pane_id: String,
    pub prompt_id: String,
    /// The attention card the notification announced, with the occurrence and screen fingerprint it
    /// showed. A tap is answered through the same path as the phone card (`attention::answer`), which
    /// re-reads the live screen and types nothing when it is no longer this prompt. `None` only when
    /// the attention list had no card yet; the old prompt-id check then guards the tap.
    pub item: Option<ItemRef>,
    expires_ms: u64,
}

/// The attention item a notification is about (the deep link, and what a lock-screen tap answers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRef {
    /// The opaque item id; the phone opens `/?item=<id>`.
    pub id: String,
    pub occurrence: u32,
    pub fingerprint: String,
}

/// What the dispatcher wants to tell the phones.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub kind: Kind,
    /// Rate-limit and tag key: the pane id, or the operation id for a pane-less run.
    pub key: String,
    pub pane_id: Option<String>,
    pub agent_label: String,
    /// What the sidebar calls the pane (the person's title, else the numbered agent label), the
    /// first words of the notification title: "auth refactor · Needs you".
    pub heading: Option<String>,
    pub prompt: Option<PromptInfo>,
    /// The attention card this is about, when the list has it.
    pub item: Option<ItemRef>,
    /// The one fact a kind adds to its first line (see `triggers::copy_for_view`).
    pub detail: Option<String>,
}

impl Candidate {
    /// The prompt's risk in the attention scale, for the Focus tiers (`attention::focus::policy`).
    pub fn focus_risk(&self) -> Option<crate::attention::model::Level> {
        self.prompt.as_ref().and_then(|p| p.risk.as_ref()).map(|r| crate::attention::model::Level::from(r.level))
    }
}

/// A notification the rate limit held back instead of dropping: the newest candidate for its key and
/// how many were held. It is released when the window ends (`PushHub::take_due`).
#[derive(Debug, Clone)]
pub struct Held {
    pub candidate: Candidate,
    pub count: u32,
    /// When the window of the notification that went out first ends.
    pub due_ms: u64,
}

/// One message for one phone.
#[derive(Debug, Clone)]
pub struct Delivery {
    #[allow(dead_code)]
    pub device_id: String,
    pub subscription: Subscription,
    pub payload: super::sender::PushPayload,
    pub urgent: bool,
    /// The pane the message is about (for dropping it when the prompt is gone).
    pub pane_id: Option<String>,
    pub kind: Kind,
}

pub struct PushHub {
    vapid: VapidKeys,
    store: Mutex<SubscriptionStore>,
    visible_until: Mutex<HashMap<String, u64>>,
    grants: Mutex<HashMap<String, Grant>>,
    gate: Mutex<Gate>,
    /// Notifications the 30 s rate limit held back, by key (pane id, or `q-<id>` for a question).
    held: Mutex<HashMap<String, Held>>,
    /// The last notification held back per device because its PWA was on screen.
    suppressed: Mutex<HashMap<String, (Delivery, u64)>>,
    /// Held-back notifications released by "hidden" presence reports, waiting to be sent.
    #[allow(dead_code)]
    outbox: Mutex<Vec<Delivery>>,
    /// One message per companion per minute (`companion_replies`).
    replies: Mutex<companion_replies::Gate>,
    directory: Box<dyn DeviceDirectory>,
}

impl PushHub {
    pub fn new(vapid: VapidKeys, store: SubscriptionStore, directory: Box<dyn DeviceDirectory>) -> Self {
        Self {
            vapid,
            store: Mutex::new(store),
            visible_until: Mutex::new(HashMap::new()),
            grants: Mutex::new(HashMap::new()),
            gate: Mutex::new(Gate::default()),
            held: Mutex::new(HashMap::new()),
            suppressed: Mutex::new(HashMap::new()),
            outbox: Mutex::new(Vec::new()),
            replies: Mutex::new(companion_replies::Gate::default()),
            directory,
        }
    }

    pub fn vapid(&self) -> &VapidKeys {
        &self.vapid
    }

    pub fn gate(&self) -> &Mutex<Gate> {
        &self.gate
    }

    pub fn store(&self) -> &Mutex<SubscriptionStore> {
        &self.store
    }

    pub fn companion_replies(&self) -> &Mutex<companion_replies::Gate> {
        &self.replies
    }

    pub fn has_subscribers(&self) -> bool {
        !self.store.lock().is_empty()
    }

    // --- presence -----------------------------------------------------------------------------

    /// Feed a pane status update through the gate. Leaving `waiting_input` invalidates everything
    /// that could still answer or announce that prompt: open Allow/Deny grants and held-back pushes.
    pub fn note_pane_status(&self, pane_id: &str, status: &str) -> Option<Kind> {
        let kind = self.gate.lock().entered(pane_id, status);
        if status != "waiting_input" {
            self.drop_pane_prompts(pane_id);
        }
        // A held prompt or error that is no longer true is not announced later.
        self.held.lock().retain(|key, held| {
            key != pane_id
                || match held.candidate.kind {
                    kind if kind.is_pane_prompt() => status == "waiting_input",
                    Kind::Error => status == "error",
                    _ => true,
                }
        });
        kind
    }

    fn drop_pane_prompts(&self, pane_id: &str) {
        self.grants.lock().retain(|_, g| g.pane_id != pane_id);
        self.suppressed
            .lock()
            .retain(|_, (d, _)| !(d.kind.is_pane_prompt() && d.pane_id.as_deref() == Some(pane_id)));
    }

    pub fn is_visible(&self, device_id: &str, now_ms: u64) -> bool {
        self.visible_until.lock().get(device_id).is_some_and(|until| *until > now_ms)
    }

    // --- Allow / Deny grants ------------------------------------------------------------------

    fn issue_grant(&self, device_id: &str, pane_id: &str, prompt_id: &str, item: Option<&ItemRef>, now_ms: u64) -> String {
        let mut grants = self.grants.lock();
        grants.retain(|_, g| g.expires_ms > now_ms);
        if grants.len() >= MAX_GRANTS {
            if let Some(oldest) = grants.iter().min_by_key(|(_, g)| g.expires_ms).map(|(k, _)| k.clone()) {
                grants.remove(&oldest);
            }
        }
        let secret: [u8; 32] = rand::random();
        let token = URL_SAFE_NO_PAD.encode(secret);
        grants.insert(
            token.clone(),
            Grant {
                device_id: device_id.to_string(),
                pane_id: pane_id.to_string(),
                prompt_id: prompt_id.to_string(),
                item: item.cloned(),
                expires_ms: now_ms + GRANT_TTL_MS,
            },
        );
        token
    }

    // --- subscriptions ------------------------------------------------------------------------

    // --- deciding and building ----------------------------------------------------------------

    // --- held back by the rate limit ----------------------------------------------------------

    /// A second thing on a pane inside the 30 s window: keep the newest, count it, and remember
    /// when the window ends. Finished runs are not urgent and are not held.
    fn hold(&self, candidate: &Candidate, now_ms: u64) {
        if !candidate.kind.is_urgent() {
            return;
        }
        let due_ms = self.gate.lock().last_sent(&candidate.key).map(|at| at + super::triggers::RATE_LIMIT_MS).unwrap_or(now_ms);
        let mut held = self.held.lock();
        let count = held.get(&candidate.key).map_or(0, |h| h.count) + 1;
        held.insert(candidate.key.clone(), Held { candidate: candidate.clone(), count, due_ms });
    }

    /// Everything whose window has ended, removed from the hold. The caller decides, from the
    /// attention list, what is still open (`plan_held`).
    pub fn take_due(&self, now_ms: u64) -> Vec<Held> {
        let mut held = self.held.lock();
        let due: Vec<String> = held.iter().filter(|(_, h)| h.due_ms <= now_ms).map(|(k, _)| k.clone()).collect();
        due.into_iter().filter_map(|key| held.remove(&key)).collect()
    }

    /// When the next held notification is due, if any.
    pub fn next_held_due(&self) -> Option<u64> {
        self.held.lock().values().map(|h| h.due_ms).min()
    }

    /// The window ended. `open` is how many blocking things are open on that pane right now:
    /// none, and nothing is sent; one, and it gets its normal notification; several, and ONE
    /// summary ("auth refactor · 2 things need you") that never carries a button.
    pub fn plan_held(&self, held: &Held, open: u32, now_ms: u64) -> (Verdict, Vec<Delivery>) {
        match open {
            0 => (Verdict::Resolved, Vec::new()),
            1 => self.plan(&held.candidate, now_ms),
            n => {
                let candidate = &held.candidate;
                self.gate.lock().mark_sent(now_ms, &candidate.key);
                let risk = candidate.prompt.as_ref().and_then(|p| p.risk.as_ref());
                let words = |view| coalesced_copy_view(&candidate.agent_label, candidate.heading.as_deref(), n, risk, view);
                (Verdict::Send, self.deliveries(candidate, &words, false, now_ms))
            }
        }
    }

    /// Apply the gate (duplicate prompt, 30 s per pane), then build one message per subscribed
    /// device that is not looking at the PWA right now. A rate-limited candidate is held, not dropped.
    pub fn plan(&self, candidate: &Candidate, now_ms: u64) -> (Verdict, Vec<Delivery>) {
        let prompt_id = candidate.prompt.as_ref().map(|p| p.prompt_id.as_str());
        let verdict = self.gate.lock().admit(now_ms, &candidate.key, candidate.kind, prompt_id);
        if verdict == Verdict::RateLimited {
            self.hold(candidate, now_ms);
        }
        if verdict != Verdict::Send {
            return (verdict, Vec::new());
        }
        // This one is going out now: whatever was held for the key is covered by it.
        self.held.lock().remove(&candidate.key);
        // What the fullest version of the notification says (Android, desktop browsers). It decides
        // whether Allow / Deny can be offered at all: every phone's own version is cut down from it.
        let full = copy_for_view(candidate.kind, &candidate.agent_label, candidate.heading.as_deref(), candidate.prompt.as_ref(), candidate.detail.as_deref(), View::Full);
        // Nothing secret goes to a push service. If redaction changed the text, the body no longer
        // shows the whole command exactly, so the notification is open-only (no Allow button).
        let redaction_changed = crate::redact::text(&full.title).as_str() != full.title
            || crate::redact::text(&full.body).as_str() != full.body
            || candidate.prompt.as_ref().is_some_and(|p| crate::redact::text(&p.text).as_str() != p.text);
        // Allow is only offered when the body shows the WHOLE command and cwd: a long command is cut
        // at 120 characters, and the cut tail could be what actually runs. Then it is open-only.
        let actionable = candidate.kind == Kind::WaitingInput
            && candidate.prompt.as_ref().is_some_and(|p| {
                // Belt and braces: `actionable` already means low risk; check again here.
                let calm = p.risk.as_ref().is_some_and(|r| r.level == crate::prompt_risk::RiskLevel::Low);
                p.actionable && calm && shown_in_full(&p.text)
            })
            && candidate.pane_id.is_some()
            && !redaction_changed;
        let words = |view| {
            copy_for_view(candidate.kind, &candidate.agent_label, candidate.heading.as_deref(), candidate.prompt.as_ref(), candidate.detail.as_deref(), view)
        };
        (verdict, self.deliveries(candidate, &words, actionable, now_ms))
    }

    /// One message per subscribed phone that is not looking at the PWA right now, each in the words
    /// that phone gets (`words(view)`): an iPhone's has no command and no buttons (Apple shows no
    /// action buttons), and a phone with "Hide details on lock screen" on shows no detail at all.
    /// `actionable` adds Allow / Deny (a one-time grant) for the phones that may answer and read the
    /// full version.
    fn deliveries(&self, candidate: &Candidate, words: &dyn Fn(View) -> Words, actionable: bool, now_ms: u64) -> Vec<Delivery> {
        let prompt_id = candidate.prompt.as_ref().map(|p| p.prompt_id.as_str());
        // Focus: a card that waits for the person arrives without the buzz (the phone shows it silently).
        let quiet = crate::attention::focus::live::quiet_for(candidate.kind, candidate.focus_risk());
        let mut deliveries = Vec::new();
        for (device_id, subscription, hide_details) in self.store.lock().all_with_preferences() {
            let view = if hide_details {
                View::Hidden
            } else if is_apple_endpoint(&subscription.endpoint) {
                View::Ios
            } else {
                View::Full
            };
            let words = words(view);
            let (title, body) = (crate::redact::text(&words.title), crate::redact::text(&words.body));
            let mut extra = serde_json::Map::new();
            extra.insert("v".into(), json!(2));
            extra.insert("kind".into(), json!(candidate.kind.as_str()));
            extra.insert("tag".into(), json!(format!("pm-{}", candidate.key)));
            if quiet {
                extra.insert("quiet".into(), json!(true));
            }
            if let Some(pane_id) = &candidate.pane_id {
                extra.insert("pane_id".into(), Value::String(pane_id.clone()));
            }
            if let Some(item) = &candidate.item {
                // The tap opens this card: `/?item=<id>`.
                extra.insert("item_id".into(), Value::String(item.id.clone()));
            }
            if candidate.kind == Kind::Question {
                // Version 1 phones open their question list.
                extra.insert("target".into(), Value::String("questions".into()));
            }
            if let (Some(prompt), true) = (&candidate.prompt, candidate.kind.is_pane_prompt()) {
                extra.insert("prompt_id".into(), Value::String(prompt.prompt_id.clone()));
            }
            if actionable && view == View::Full && self.directory.device_can_answer(&device_id) {
                let pane_id = candidate.pane_id.as_deref().unwrap_or_default();
                let prompt_id = prompt_id.unwrap_or_default();
                extra.insert(
                    "act".into(),
                    json!({
                        "url": ACT_PATH,
                        "token": self.issue_grant(&device_id, pane_id, prompt_id, candidate.item.as_ref(), now_ms),
                    }),
                );
            }
            let payload = super::sender::PushPayload::new(&title, &body, extra);
            let delivery = Delivery {
                device_id: device_id.clone(),
                subscription,
                payload,
                urgent: candidate.kind.is_urgent(),
                pane_id: candidate.pane_id.clone(),
                kind: candidate.kind,
            };
            if self.is_visible(&device_id, now_ms) {
                // The phone is looking: hold it back, release it if the phone goes hidden soon.
                self.suppressed.lock().insert(device_id, (delivery, now_ms));
            } else {
                deliveries.push(delivery);
            }
        }
        deliveries
    }

    /// One companion reply, one message per phone that may watch (`watch` ability) and is not looking
    /// at the PWA right now (the phone shows the reply itself). Carries no pane, no buttons, and is not
    /// urgent: the phone may show it when it next wakes.
    pub fn companion_deliveries(&self, message: &Message, now_ms: u64) -> Vec<Delivery> {
        let mut deliveries = Vec::new();
        for (device_id, subscription, hide_details) in self.store.lock().all_with_preferences() {
            // Read at send time, never at hold time: a phone that was unpaired, lost `watch`, or turned
            // "Hide details" on during the minute gets what it is entitled to now.
            if !self.directory.device_exists(&device_id) || !self.directory.device_can_watch(&device_id) || self.is_visible(&device_id, now_ms) {
                continue;
            }
            let title = crate::redact::text(&message.title());
            let body = crate::redact::text(&message.body(hide_details));
            let mut extra = serde_json::Map::new();
            extra.insert("v".into(), json!(2));
            extra.insert("kind".into(), json!(Kind::CompanionReply.as_str()));
            extra.insert("tag".into(), json!(format!("pm-companion-{}", message.companion_id)));
            extra.insert("companion_id".into(), json!(message.companion_id));
            let payload = super::sender::PushPayload::new(&title, &body, extra);
            deliveries.push(Delivery {
                device_id,
                subscription,
                payload,
                urgent: false,
                pane_id: None,
                kind: Kind::CompanionReply,
            });
        }
        deliveries
    }

    /// The end-of-Focus message, or its one reminder (`push::focus`): one message per phone that may watch
    /// (`watch` ability) and is not looking at the PWA. Same rules as `night_deliveries`; counts only, not
    /// urgent, and never a pane (a Focus message names none).
    pub fn focus_deliveries(&self, message: &super::focus::Message, now_ms: u64) -> Vec<Delivery> {
        let mut deliveries = Vec::new();
        for (device_id, subscription, hide_details) in self.store.lock().all_with_preferences() {
            // Read at send time: a phone that lost `watch` or was unpaired gets nothing.
            if !self.directory.device_exists(&device_id) || !self.directory.device_can_watch(&device_id) || self.is_visible(&device_id, now_ms) {
                continue;
            }
            let title = crate::redact::text(&message.title);
            let body = crate::redact::text(&message.body(hide_details));
            let mut extra = serde_json::Map::new();
            extra.insert("v".into(), json!(2));
            extra.insert("kind".into(), json!(Kind::Focus.as_str()));
            extra.insert("tag".into(), json!("pm-focus"));
            let payload = super::sender::PushPayload::new(&title, &body, extra);
            deliveries.push(Delivery {
                device_id,
                subscription,
                payload,
                urgent: false,
                pane_id: None,
                kind: Kind::Focus,
            });
        }
        deliveries
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::collections::HashMap;

    /// token -> device id; a device exists while it is in `devices`.
    pub struct FakeDirectory {
        pub tokens: Mutex<HashMap<String, String>>,
        /// Devices paired without the `answer` ability.
        pub cannot_answer: Mutex<Vec<String>>,
        /// Devices paired without the `watch` ability.
        pub cannot_watch: Mutex<Vec<String>>,
    }

    impl FakeDirectory {
        pub fn new(pairs: &[(&str, &str)]) -> Self {
            Self {
                tokens: Mutex::new(pairs.iter().map(|(t, d)| (t.to_string(), d.to_string())).collect()),
                cannot_answer: Mutex::new(Vec::new()),
                cannot_watch: Mutex::new(Vec::new()),
            }
        }
    }

    impl DeviceDirectory for std::sync::Arc<FakeDirectory> {
        fn device_exists(&self, device_id: &str) -> bool {
            self.tokens.lock().values().any(|d| d == device_id)
        }
        fn device_can_answer(&self, device_id: &str) -> bool {
            !self.cannot_answer.lock().iter().any(|d| d == device_id)
        }
        fn device_can_watch(&self, device_id: &str) -> bool {
            !self.cannot_watch.lock().iter().any(|d| d == device_id)
        }
    }

    pub fn temp_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("pm-push-hub-{name}-{}", std::process::id()))
    }

    pub fn hub_with(name: &str, directory: std::sync::Arc<FakeDirectory>) -> PushHub {
        let dir = temp_dir(name);
        let _ = std::fs::remove_dir_all(&dir);
        let vapid = VapidKeys::load_or_create(&dir.join("vapid.json")).unwrap();
        let store = SubscriptionStore::load(dir.join("push.json"), vapid.public_key_b64url());
        PushHub::new(vapid, store, Box::new(directory))
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::push::store::test_subscription;
    use std::sync::Arc;

    /// A phone that reported itself visible this recently counts as looking at the screen (restored for the tests).
    const VISIBLE_TTL_MS: u64 = 45_000;

    fn low_risk() -> super::super::triggers::PromptRisk {
        super::super::triggers::PromptRisk { level: crate::prompt_risk::RiskLevel::Low, headline: "Run a command".into() }
    }

    fn command_prompt(id: &str) -> PromptInfo {
        PromptInfo { prompt_id: id.into(), text: "Run command: ls".into(), actionable: true, risk: Some(low_risk()) }
    }

    fn waiting(key: &str, prompt: Option<PromptInfo>) -> Candidate {
        Candidate { kind: Kind::WaitingInput, key: key.into(), pane_id: Some(key.into()), agent_label: "Kilo".into(), heading: None, prompt, item: None, detail: None }
    }

    fn act_of(d: &Delivery) -> Option<Value> {
        serde_json::from_slice::<Value>(&d.payload).unwrap().get("act").cloned()
    }

    fn question(id: &str, text: &str) -> Candidate {
        Candidate {
            kind: Kind::Question,
            key: format!("q-{id}"),
            pane_id: None,
            agent_label: "Kilo".into(),
            heading: None,
            prompt: Some(PromptInfo { prompt_id: String::new(), text: text.into(), actionable: false, risk: None }),
            item: None,
            detail: None,
        }
    }

    fn payload_of(deliveries: &[Delivery], device: &str) -> Value {
        let d = deliveries.iter().find(|d| d.device_id == device).expect("that phone is told");
        serde_json::from_slice(&d.payload).unwrap()
    }

}
