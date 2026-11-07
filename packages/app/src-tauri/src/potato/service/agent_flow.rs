//! The owner's own agent as a bot's brain ("Through my agents", `custom_agents.rs`). A message to such a bot does not go to a brain
//! process: the agent is run headless inside the sandbox, one process per turn, and its words become the bot's reply.
//!
//! What this file decides, in this order, before anything runs:
//! 1. only the owner's own words reach this brain: a message, a routine the owner made, a job the owner started, a queued
//!    message. A bot's message (peer), a card answer, a group or any other route is refused with a plain reason, because the
//!    agent runs its own tools and the host cannot limit them. The sandbox is the boundary;
//! 2. the sandbox must be switched on and wired in (the agent never runs on this PC);
//! 3. the agent must be in the bot's approved list with the fingerprint the owner approved.
//!
//! The risk gate, cards and levels do not apply to the agent's own tools. Its answer is untrusted text: it goes into the thread
//! and nowhere else (not into notes, memory or the instructions), and a failure shows a plain error: no other brain takes over.
//! The turn is killed by Stop (`cancel_agent`) and by its time limit. The cost is not counted: it is the agent's own account.

use super::*;
use crate::potato::custom_agents::{self as agents, CustomAgent};

/// What the Advanced tab shows of a bot's agents.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    pub id: String,
    pub label: String,
    pub description: String,
    pub command: String,
    pub fingerprint: String,
    pub approved_at: i64,
    /// The values this agent asks the owner for. The value itself never leaves the credential store; the page sets one under `key`.
    pub secrets: Vec<AgentSecretView>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSecretView {
    /// The environment name the agent reads.
    pub env: String,
    /// The credential-store name the Settings field sets.
    pub key: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentsView {
    pub agents: Vec<AgentView>,
    pub brain_agent: Option<String>,
    pub own_account_note: &'static str,
    pub decides_note: &'static str,
    /// The sandbox is switched on and wired in, so an agent turn can run.
    pub sandbox_ready: bool,
}

fn view_of(bot: &str, agent: &CustomAgent) -> AgentView {
    let (description, command, secrets) = match agents::parse(&agent.toml) {
        Ok(spec) => (
            spec.description.clone(),
            agents::command_text(&spec),
            spec.secret_names.iter().map(|env| AgentSecretView { env: env.clone(), key: agents::agent_secret_name(bot, &agent.id, env) }).collect(),
        ),
        Err(_) => (String::new(), String::new(), Vec::new()),
    };
    AgentView { id: agent.id.clone(), label: agent.label.clone(), description, command, fingerprint: agent.fingerprint.clone(), approved_at: agent.approved_at, secrets }
}

/// The wording of a refusal that is the owner's to read.
fn agent_err(e: agents::DefError) -> StoreError {
    err("POTATO_AGENT_INVALID", e.to_string())
}

impl Service {
    /// The agent this bot talks through, when one is picked. A pick that is no longer approved is still returned: the turn then
    /// refuses with the reason, instead of the bot quietly going back to another brain.
    pub(super) fn agent_brain_id(&self, bot: &str) -> Option<String> {
        let own = bot_settings::load(&self.root, bot).ok()?;
        own.brain_agent.clone()
    }

    pub(in crate::potato) fn agent_running(&self, bot: &str) -> bool {
        self.agent_runs.lock().contains_key(bot)
    }

    /// Stop: kills the agent's process tree in the sandbox (the flag is read by the runner, `Sandbox::agent_run`).
    pub(super) fn cancel_agent(&self, bot: &str) {
        if let Some(flag) = self.agent_runs.lock().get(bot) {
            flag.store(true, Ordering::SeqCst);
        }
    }

    // ---- Settings: add, approve, remove, pick -------------------------------------------------------------------

    /// Checks a pasted definition and returns the summary the owner reads. Writes nothing.
    pub fn agent_check(&self, text: &str) -> std::result::Result<agents::Summary, agents::DefError> {
        agents::summary(text)
    }

    pub fn agents_view(&self, bot: &str) -> Result<AgentsView> {
        self.require_bot(bot)?;
        let own = bot_settings::load(&self.root, bot)?;
        Ok(AgentsView {
            agents: own.custom_agents.iter().map(|a| view_of(bot, a)).collect(),
            brain_agent: own.brain_agent.clone(),
            own_account_note: agents::OWN_ACCOUNT_NOTE,
            decides_note: agents::DECIDES_NOTE,
            sandbox_ready: self.sandbox_offered(),
        })
    }

    /// The owner approved what the summary showed: the definition is checked again here and stored with its fingerprint.
    /// `fingerprint` is the one the owner saw; if the text no longer matches it, nothing is saved.
    pub fn agent_add(&self, bot: &str, text: &str, fingerprint: &str) -> Result<AgentsView> {
        self.require_bot(bot)?;
        if agents::fingerprint(text) != fingerprint {
            return Err(err("POTATO_AGENT_CHANGED", "The definition changed after you read the summary. Check it again."));
        }
        let now = self.host.now_ms();
        bot_settings::update(&self.root, bot, |s| agents::approve(&mut s.custom_agents, text, now).map(|_| ()).map_err(agent_err))?;
        self.host.changed(bot);
        self.agents_view(bot)
    }

    pub fn agent_remove(&self, bot: &str, id: &str) -> Result<AgentsView> {
        self.require_bot(bot)?;
        let own = bot_settings::load(&self.root, bot)?;
        let was_brain = own.brain_agent.as_deref() == Some(id);
        let secret_names: Vec<String> = own
            .custom_agents
            .iter()
            .find(|a| a.id == id)
            .and_then(|a| agents::parse(&a.toml).ok())
            .map(|spec| spec.secret_names.iter().map(|env| agents::agent_secret_name(bot, id, env)).collect())
            .unwrap_or_default();
        // The agent's saved space goes first: when the sandbox cannot delete it, the agent stays, so the owner can try again.
        if was_brain {
            self.cancel_agent(bot);
        }
        if self.sandbox_offered() {
            if let Some(port) = self.port.lock().clone() {
                let run = self.agent_runs.lock().contains_key(bot);
                if run {
                    return Err(err("POTATO_AGENT_BUSY", "This agent is working. Press Stop, wait a moment, then remove it."));
                }
                port.sandbox
                    .agent_home_remove(&agents::home_image(bot, id))
                    .map_err(|e| err("POTATO_AGENT_SPACE", format!("The agent's saved space could not be deleted, so it was not removed: {}", e.message)))?;
            }
        }
        for name in &secret_names {
            self.host.delete_secret(name);
        }
        bot_settings::update(&self.root, bot, |s| {
            s.custom_agents.retain(|a| a.id != id);
            if s.brain_agent.as_deref() == Some(id) {
                s.brain_agent = None;
            }
            Ok(())
        })?;
        self.host.changed(bot);
        self.agents_view(bot)
    }

    /// Picks the approved agent this bot talks through (`None`: none). The agent must be in the bot's approved list.
    pub fn agent_pick(&self, bot: &str, id: Option<&str>) -> Result<AgentsView> {
        self.require_bot(bot)?;
        bot_settings::update(&self.root, bot, |s| {
            if let Some(id) = id {
                agents::approved_spec(&s.custom_agents, id).map_err(agent_err)?;
            }
            s.brain_agent = id.map(str::to_string);
            Ok(())
        })?;
        self.cancel_agent(bot);
        self.host.changed(bot);
        self.agents_view(bot)
    }

    // ---- a turn ------------------------------------------------------------------------------------------------

    fn agent_refuse(&self, bot: &str, saved_id: &str, line: &str) -> Result<SendOutcome> {
        self.note(bot, line);
        self.host.changed(bot);
        Ok(SendOutcome { id: saved_id.to_string(), state: BrainState::Idle, delivered: false })
    }

    /// One turn through the bot's agent. Called from `send_as` once the message is saved. Returns at once; the answer is saved
    /// from another thread when the agent ends.
    pub(super) fn agent_turn(&self, bot: &str, saved_id: &str, conversation: &str, origin: Origin<'_>, text: &str) -> Result<SendOutcome> {
        let name = self.name_of(bot);
        // 1. Only the owner's own words. The agent runs its own tools, which the host cannot limit.
        if matches!(origin, Origin::Peer { .. } | Origin::Answer) || conversation != store::app_conversation(bot) {
            return self.agent_refuse(
                bot,
                saved_id,
                &format!("{name} talks through your own agent, which cannot be limited, so it answers only you in your private chat. This message was not passed on."),
            );
        }
        // 2. Only in the sandbox.
        if !self.sandbox_offered() {
            return self.agent_refuse(
                bot,
                saved_id,
                &format!("{name}'s agent runs only in the sandbox, and the sandbox is not switched on or not set up. Set it up in Settings > Potato Bot, then send again."),
            );
        }
        let Some(port) = self.port.lock().clone() else {
            return self.agent_refuse(bot, saved_id, &format!("{name}'s agent runs only in the sandbox, and the sandbox is not set up yet."));
        };
        // 3. An approved agent, as approved.
        let own = bot_settings::load(&self.root, bot)?;
        let spec = match own.brain_agent.as_deref().map(|id| agents::approved_spec(&own.custom_agents, id)) {
            Some(Ok(spec)) => spec,
            Some(Err(e)) => return self.agent_refuse(bot, saved_id, &format!("{name}'s agent cannot run: {e}")),
            None => return self.agent_refuse(bot, saved_id, &format!("{name} has no agent picked. Pick one on the Advanced tab.")),
        };
        if matches!(origin, Origin::Routine { .. }) {
            let day = self.host.local_day(self.host.now_ms());
            if !cost::take_brain_turn(&self.root, &day, bot, cost::DEFAULT_BRAIN_TURNS).unwrap_or(false) {
                return self.agent_refuse(bot, saved_id, &format!("{name} has started {} things on its own today. It will wait until tomorrow.", cost::DEFAULT_BRAIN_TURNS));
            }
        }
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut runs = self.agent_runs.lock();
            if runs.contains_key(bot) {
                drop(runs);
                return self.agent_refuse(bot, saved_id, &format!("{} is still working on your last message. Press Stop, or wait for the answer.", spec.label));
            }
            // Every agent turn in this sandbox runs as the same user, so only one runs at a time.
            if !runs.is_empty() {
                drop(runs);
                return self.agent_refuse(bot, saved_id, &format!("Another agent is working in the sandbox right now. Send again when it has finished, then {} can answer.", spec.label));
            }
            runs.insert(bot.to_string(), cancel.clone());
        }
        let prompt = self.agent_prompt(bot, &name, &own, saved_id, text);
        let Some(me) = self.me.upgrade() else {
            self.agent_runs.lock().remove(bot);
            return self.agent_refuse(bot, saved_id, "Puppet Master is shutting down.");
        };
        let bot_id = bot.to_string();
        let sandbox = port.sandbox.clone();
        // The agent's own secrets, as the owner typed them in Settings. They go to its process through the script's input and nowhere else.
        let secrets: Vec<(String, String)> = spec
            .secret_names
            .iter()
            .filter_map(|env| self.host.secret(&agents::agent_secret_name(bot, &spec.id, env)).map(|v| v.trim().to_string()).filter(|v| agents::secret_value_ok(v)).map(|v| (env.clone(), v)))
            .collect();
        let script = agents::launch_script(&spec, &prompt, self.agent_seconds(), bot, &secrets);
        let timeout = Duration::from_secs(self.agent_seconds()) + Duration::from_millis(self.agent_grace_ms.load(Ordering::SeqCst));
        let label = spec.label.clone();
        let spawn = self.spawner.lock().clone();
        spawn(Box::new(move || {
            let result = sandbox.agent_run(&script, timeout, &cancel).map(|mut out| {
                // What the agent printed is shown, so a secret it printed is taken out first.
                out.stdout = agents::hide_secrets(&out.stdout, &secrets);
                out.stderr = agents::hide_secrets(&out.stderr, &secrets);
                out
            });
            me.agent_finished(&bot_id, &label, result);
        }));
        self.host.changed(bot);
        Ok(SendOutcome { id: saved_id.to_string(), state: BrainState::Ready, delivered: true })
    }

    /// A test lowers the turn's time limit (seconds) and the wait past it (milliseconds).
    #[cfg(test)]
    pub(in crate::potato) fn set_agent_limits(&self, seconds: u64, grace_ms: u64) {
        self.agent_turn_seconds.store(seconds, Ordering::SeqCst);
        self.agent_grace_ms.store(grace_ms, Ordering::SeqCst);
    }

    /// The seconds one turn may run (a test lowers it).
    fn agent_seconds(&self) -> u64 {
        self.agent_turn_seconds.load(Ordering::SeqCst)
    }

    /// The prompt: the bot's instructions, a short excerpt of its notes, the recent chat, the new message. Every part has its
    /// keys taken out (`redact`). Nothing from connectors, settings or the credential store goes in.
    fn agent_prompt(&self, bot: &str, name: &str, own: &BotSettings, newest_id: &str, text: &str) -> String {
        let clean = |s: &str| crate::redact::text(s).into_inner();
        let persona = clean(bot_settings::current_instructions(own));
        let mut memory = String::new();
        if let Ok(notes) = super::super::memory_notes::list(&self.root, bot) {
            for note in notes.iter().take(3) {
                if let Ok(body) = super::super::memory_notes::read(&self.root, bot, &note.path) {
                    let piece: String = body.chars().take(400).collect();
                    memory.push_str(&format!("- {}: {}\n", note.path, piece.trim()));
                }
            }
        }
        let memory = clean(&memory);
        let history: Vec<(String, String)> = store::read_messages(&self.root, &store::app_conversation(bot), 14, self.home_of(bot).as_deref())
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.kind == MsgKind::Text && m.id != newest_id && matches!(m.from.kind, FromKind::You | FromKind::Bot))
            .map(|m| (if m.from.kind == FromKind::You { "Owner".to_string() } else { name.to_string() }, clean(&m.text.chars().take(600).collect::<String>())))
            .collect();
        agents::build_prompt(name, &persona, &memory, &history, text)
    }

    /// The agent's process ended: save its words as the bot's reply, or a plain error. No other brain is tried.
    pub(super) fn agent_finished(&self, bot: &str, label: &str, result: std::result::Result<super::super::sandbox::ShellOutput, super::super::sandbox::SandboxError>) {
        let stopped = self.agent_runs.lock().remove(bot).is_some_and(|flag| flag.load(Ordering::SeqCst));
        let name = self.name_of(bot);
        let line = match result {
            Ok(out) => agents::reading(&out, label),
            Err(e) if e.code == "AGENT_STOPPED" => {
                self.note(bot, &format!("Stopped. {label} was killed in the sandbox."));
                self.host.changed(bot);
                return;
            }
            Err(e) => Err(format!("{label} could not run: {}", e.message)),
        };
        if stopped {
            self.note(bot, &format!("Stopped. {label} was killed in the sandbox."));
            self.host.changed(bot);
            return;
        }
        match line {
            Ok(reply) => {
                // The agent's words are untrusted text: redacted, shown in the thread, and nothing else.
                let reply = crate::redact::text(&reply).into_inner();
                let message = Message::new(self.host.now_ms(), FromKind::Bot, Some(&name), MsgKind::Text, &reply);
                if let Err(e) = self.save(bot, &store::app_conversation(bot), message) {
                    tracing::debug!(%e, "could not save an agent reply");
                }
            }
            Err(why) => self.note(bot, &why),
        }
        self.host.changed(bot);
    }
}
