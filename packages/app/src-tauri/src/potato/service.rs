//! The Potato Bot engine: one object that owns every bot's brain process, its inbox and what the brain
//! reports (spec sections 1.5, 2, 3 and 6). The bridge routes (`bridge/routes/potato_brain.rs`) and the
//! Tauri commands (`commands/potato.rs`) are thin callers of it.
//!
//! What it does, in the order a message travels:
//!
//! 1. `send`: `rules::decide` (here it gives `owner_pc` and full tools), the message is saved (redacted),
//!    the brain is asked for (lazy start), and a `send` item goes into the bot's inbox.
//! 2. `inbox_poll`: the brain's long poll. Every spawn has a new token and a new inbox.
//! 3. `ingest`: what the brain posts. A reply is saved, redacted and shown; `usage` is priced with
//!    `cost.rs` and judged against the daily limit; `helper_request` is stored as a card (starting a helper
//!    is a later piece); an `error` becomes a plain sentence in the thread. Nothing the brain says reaches
//!    the person unredacted, and a brain can only write into its own bot's thread.
//! 4. `tick`: the supervisor, every half second. It sees a process leave, runs the keeper's actions
//!    (start, ask to leave, kill) and resumes a bot that stopped for the day's limit when the day changes.
//!
//! The things the service needs from the app (clock, secrets, resource folder, events) come through the
//! `Host` trait and the process through the `Launcher` trait, so the tests run it on fakes. Nothing here
//! names an agent or a chat app.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use serde::Serialize;
use serde_json::Value;

use super::auth::{self, Brains};
use super::cost::{self, Budget};
use super::env::{self, Brain, HostEnv};
use super::fallback;
use super::hold;
use super::keeper::{Action, Keeper, Phase};
use super::log::{self, LogRing};
use super::protocol::{self, ErrorCode, Event, HelperStatus, PauseReason, Tools, Trust};

mod card_flow;
mod agent_flow;
mod chart;
mod file_flow;
mod job_flow;
mod login_flow;
mod room_flow;
mod self_flow;
mod peer_flow;
mod wait_flow;
pub mod portable;
#[cfg(test)]
#[path = "portable_tests.rs"]
mod portable_tests;
pub use agent_flow::AgentsView;
pub use card_flow::AnswerOutcome;
pub use login_flow::{ForgetOutcome, LoginOrigin};
use job_flow::{JobAfter, JobTurn};
use super::reply_cost;
use super::reactions;
use super::rooms;
use super::rules::{self, Caps, Channel, Decision, Facts, Sender};
use super::history;
use super::jobs;
use super::mcp;
use super::open_work;
use super::routines;
use super::waits;
use super::sandbox_calls::{self, Calls, Gates, Port};
use super::sidecar::{Child, Launcher, LineSink, NodeState, SpawnSpec};
use super::wire_names;
use super::bot_settings::{self, BotOptions, BotSettings, ToolSwitches};
use super::store::{self, BrainKind, FromKind, Message, MsgKind, StoreError};
use super::usage::{self, UsageReport};
use crate::secrets::{self, SecretMask};

pub const SECRET_DEEPSEEK: &str = "deepseek_api_key";
/// The key of a Custom brain (any OpenAI-compatible endpoint). In the brain's environment only.
pub const SECRET_PROVIDER: &str = "potato_provider_key";
/// The Go plan key the person pasted in Settings. In the brain's environment only; the sign-in the plan's own tool keeps is the fallback.
pub const SECRET_GO: &str = "potato_go_key";

/// The credential-store name of one bot's own provider key. The UI may set it and the bot's brain reads it;
/// the webview can never read it back (`secrets::is_sidecar_secret` allows this shape).
pub fn provider_secret_name(bot: &str) -> String {
    format!("{SECRET_PROVIDER}_{bot}")
}
/// A `shutdown` is given this long before the process tree is killed.
pub const SHUTDOWN_GRACE_MS: i64 = 2_000;
/// A bot that reported `typing` and then went quiet stops looking busy after this long.
pub const TYPING_TTL_MS: i64 = 120_000;
/// Items kept in an inbox that nobody has fetched.
const INBOX_CAP: usize = 200;

/// The code the app shows for a turn that died after doing work (the brain's own code stays in the log).
pub const JOB_STOPPED: &str = "JOB_STOPPED";

/// A turn that failed after it ran tool calls, for a reason that is not the person's to fix in Settings.
pub fn job_stopped(code: ErrorCode, steps: u64) -> bool {
    steps > 0 && matches!(code, ErrorCode::ProviderDown | ErrorCode::Network | ErrorCode::Internal | ErrorCode::SurfaceAuth)
}

/// The thread note for such a turn: the work is not lost, and the person can go on.
pub fn stopped_job_note(name: &str, steps: u64) -> String {
    format!("{name}'s job stopped after {steps} steps because of an error. Say continue and {name} will pick it up.")
}
/// Open helper cards per bot.
pub const MAX_OPEN_CARDS: usize = 3;
/// Two minutes of sends, kept to count the last minute.
const SEND_WINDOW_MS: i64 = 60_000;

/// What the app lends the engine.
pub trait Host: Send + Sync {
    fn now_ms(&self) -> i64;
    /// The local date `YYYY-MM-DD` of an instant.
    fn local_day(&self, ms: i64) -> String;
    /// `http://127.0.0.1:PORT`, once the bridge is listening.
    fn bridge_url(&self) -> Option<String>;
    fn profile(&self, bot: &str) -> Option<BotProfile>;
    fn host_env(&self) -> HostEnv;
    fn secret(&self, name: &str) -> Option<String>;
    /// Removes a secret this app stored for an agent (its owner removed the agent). Nothing by default.
    fn delete_secret(&self, _name: &str) {}
    /// The active Go plan key from the agent's own store; used only for the Go brain.
    fn go_key(&self) -> Option<String> {
        None
    }
    /// The `potato-brain` resource folder (`brain.mjs`, `persona/`).
    fn resources(&self) -> Option<PathBuf>;
    /// Something about this bot changed: tell the windows (`potato://changed`).
    fn changed(&self, bot: &str);
    /// A group changed: tell the windows (`potato://room-changed`). Nothing by default.
    fn room_changed(&self, _room_id: &str) {}
    /// A reply went into a bot's thread. `on`: the bot's "When it replies" setting (the phones get a push).
    fn reply_pushed(&self, _bot: &str, _name: &str, _text: &str, _on: bool) {}
    /// Gives the bot a new name (the companion's name, so it stays one name everywhere). Used by a bot's own name change
    /// and its Undo; the Advanced tab's Rename does the same in its command. Refuses by default.
    fn rename(&self, _bot: &str, _name: &str) -> std::result::Result<(), String> {
        Err("The name cannot be changed here.".to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotProfile {
    pub name: String,
    /// The companion's home folder, taken out of any text that is kept.
    pub folder: Option<String>,
}

/// Why a bot cannot start right now. Nothing here is a crash: it waits for the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Blocked {
    NeedsKey,
    NeedsNode,
    /// The brain program is not installed (a broken install).
    MissingProgram,
    /// The bridge is not listening yet.
    NoBridge,
}

/// What the page shows for a bot's brain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrainState {
    /// Potato Bot is switched off.
    Off,
    /// Not running: it starts on the next message.
    Idle,
    Starting,
    Ready,
    /// A crash: it starts again in a moment.
    Restarting,
    /// It kept crashing: only "Start again" starts it.
    Stopped,
    NeedsKey,
    NeedsNode,
    MissingProgram,
    DailyLimit,
    /// "Through my agents": no brain process of its own.
    Agents,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MoodNow {
    pub mood: String,
    pub intensity: u64,
    pub energy: u64,
    pub hunger: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BotView {
    pub id: String,
    /// The person archived this chat: the list hides it under Archived, and Restore brings it back. Nothing is deleted.
    pub archived: bool,
    pub name: String,
    pub talk_persona: String,
    pub brain_state: BrainState,
    /// The bot's own talk brain, when it has one (`None`: the Settings brain).
    pub talk_brain: Option<BrainKind>,
    /// The last error code the brain reported and that nothing has cleared since.
    pub last_error: Option<String>,
    pub typing: bool,
    /// The steps of the turn running now, or of the turn that just failed. None: no steps to show.
    pub work: Option<super::work::WorkRun>,
    pub mood: Option<MoodNow>,
    pub provider: Option<String>,
    pub unread: u32,
    pub last_line: Option<String>,
    pub last_at: Option<i64>,
    pub used_today_nano: u64,
    pub limit_cents: u32,
    /// No limit is on: the page shows no limit line and no strip.
    pub limit_off: bool,
    pub open_cards: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeView {
    pub ready: bool,
    pub found: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PotatoState {
    pub enabled: bool,
    pub settings: store::Settings,
    pub key: SecretMask,
    pub node: NodeView,
    pub bots: Vec<BotView>,
}

/// Who a message for a brain is from. A routine is trusted like the person at this computer, but it is
/// marked as a routine in the thread and to the brain.
#[derive(Debug, Clone, Copy)]
pub(super) enum Origin<'a> {
    Person,
    Routine { title: &'a str, context: Option<&'a str> },
    /// The next turn of a running job: started by the host, not shown as a line in the thread. `context` is the job's
    /// record (goal, to-do list, steps, spending).
    Job { context: &'a str },
    /// The bot's turn after the owner answered a card, when no job was waiting for it: the decision is the text. Not a line
    /// in the thread (the card shows the answer).
    Answer,
    /// A message of the person that was already in the thread when the app was closed, sent to the brain again with its own
    /// id (`jobs::QueuedSend`). Not saved a second time.
    Queued { id: &'a str },
    /// Another of the owner's bots wrote to this bot (`message_bot`, `peer_flow.rs`). Shown as a peer card. It runs as a turn nobody trusted
    /// (no owner tools, no computer, no memory), whatever the message says.
    Peer { from_id: &'a str, from_name: &'a str },
}

impl<'a> Origin<'a> {
    /// The saved line: the person's own words, or the routine's name and prompt.
    fn message(self, at: i64, text: &str) -> Message {
        match self {
            Origin::Person => Message::new(at, FromKind::You, None, MsgKind::Text, text),
            Origin::Routine { title, .. } => Message::new(at, FromKind::Note, Some("routine"), MsgKind::Text, &format!("Routine: {title}\n\n{text}")),
            Origin::Job { .. } => Message::new(at, FromKind::Note, Some("job"), MsgKind::Note, text),
            Origin::Answer => Message::new(at, FromKind::Note, Some("answer"), MsgKind::Note, text),
            Origin::Queued { id } => {
                let mut message = Message::new(at, FromKind::You, None, MsgKind::Text, text);
                message.id = id.to_string();
                message
            }
            Origin::Peer { from_id, from_name } => {
                let mut message = Message::new(at, FromKind::Note, Some("peer"), MsgKind::Note, text);
                message.peer = Some(store::PeerCard { dir: "in".into(), bot_id: from_id.to_string(), bot_name: from_name.to_string() });
                message
            }
        }
    }

    fn from_id(self) -> &'a str {
        match self {
            Origin::Person | Origin::Job { .. } | Origin::Answer | Origin::Queued { .. } => "owner",
            Origin::Routine { .. } => "routine",
            Origin::Peer { from_id, .. } => from_id,
        }
    }

    fn from_name(self) -> &'a str {
        match self {
            Origin::Person | Origin::Job { .. } | Origin::Answer | Origin::Queued { .. } => "you",
            Origin::Routine { .. } => "routine",
            Origin::Peer { from_name, .. } => from_name,
        }
    }

    fn context(self) -> Option<&'a str> {
        match self {
            Origin::Person | Origin::Answer | Origin::Queued { .. } | Origin::Peer { .. } => None,
            Origin::Routine { context, .. } => context,
            Origin::Job { context } => Some(context),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendOutcome {
    /// The id of the saved message; the brain's reply names it in `inReplyTo`.
    pub id: String,
    pub state: BrainState,
    /// The message went into the brain's inbox. False when the bot cannot answer yet (no key, no Node,
    /// the daily limit): the message is saved with a note that says why.
    pub delivered: bool,
}

// ---- inbox --------------------------------------------------------------------------------------

#[derive(Default)]
struct Inbox {
    /// Index of `items[0]`.
    base: u64,
    items: VecDeque<Value>,
    /// The highest index the brain has been handed (the end of its last answer).
    handed: u64,
}

impl Inbox {
    fn push(&mut self, item: Value) {
        self.items.push_back(item);
        while self.items.len() > INBOX_CAP {
            self.items.pop_front();
            self.base += 1;
        }
    }

    fn end(&self) -> u64 {
        self.base + self.items.len() as u64
    }

    /// Items from position `at` on, and the position to ask with next.
    fn since(&mut self, at: u64) -> (Vec<Value>, u64) {
        let from = at.max(self.base);
        let skip = (from - self.base) as usize;
        let items: Vec<Value> = self.items.iter().skip(skip).cloned().collect();
        let next = self.end();
        self.handed = self.handed.max(next);
        (items, next)
    }
}

struct Session {
    child: Box<dyn Child>,
    inbox: Inbox,
    secrets: Vec<String>,
    shutdown_at: Option<i64>,
    /// `send` id -> conversation, for the sends the brain has not answered.
    in_flight: HashMap<String, String>,
    /// `send` id -> its place in the inbox.
    send_index: HashMap<String, u64>,
}

struct BotRt {
    keeper: Keeper,
    session: Option<Session>,
    log: Arc<Mutex<LogRing>>,
    blocked: Option<Blocked>,
    provider: Option<String>,
    typing: HashMap<String, i64>,
    /// The live step list of each running turn, by conversation (see work.rs).
    work: HashMap<String, super::work::WorkRun>,
    last_error: Option<ErrorCode>,
    /// Steps the last failed turn had run (0: it did no work). With a code in `job_stopped`, the thread says so.
    last_steps: u64,
    mood: Option<MoodNow>,
    sends_ms: VecDeque<i64>,
    /// Sends for the next session: they came while none was running, or while one was leaving.
    queued: Vec<Value>,
    turn_trust: HashMap<String, Trust>,
    /// Conversations whose current turn is a routine's own run (not a person's message).
    turn_routine: HashSet<String>,
    /// The day on which the bot was paused for its limit.
    paused_for_limit: Option<String>,
    last_surface: String,
    /// The newest reply's last segment: the turn's usage (it comes after the reply) is recorded on it.
    last_reply_id: Option<String>,
    /// Where the run is in its brain chain (`fallback.rs`): 0 is the talk brain. Reset when a new run starts.
    chain_step: usize,
    /// Accepted sends whose turn has not reported usage or failed yet, oldest first. Each holds an estimate of its cost
    /// against the day's limit, so a queue of sends cannot pass the limit before any of them is charged (H6b-3).
    reserved: VecDeque<Reservation>,
    /// The real cost of the newest turns; the next estimate is their average (the floor when there is none).
    recent_turn_nano: VecDeque<u64>,
    /// The local day the holds in `reserved` belong to; a new day clears them.
    reserved_day: String,
    /// The turn the bot's running job waits for (`job_flow.rs`). At most one is open at a time.
    job_turn: Option<JobTurn>,
    /// A turn of the bot ended while a job ran; the next job turn is decided when its usage arrives.
    job_after: Option<JobAfter>,
    /// The person's messages sent while a turn was running that the bot has not picked up yet: the thread shows "Queued".
    queued_ids: HashSet<String>,
    /// When the app last answered a status question itself (`steer.rs`).
    last_status_note_ms: i64,
    /// Conversations whose current turn has read text from outside the chat (a page, a file, a command, a tool, an old
    /// chat). Such a turn cannot change the bot's own settings (`self_flow.rs`). Cleared when a new turn starts.
    turn_tainted: HashSet<String>,
    /// A change was made in a turn that needs the brain to start again: it stops when the turn's usage comes in.
    restart_after_turn: bool,
    /// Messages another bot wrote to this one that have not been answered: this bot's saved message id to the bot that wrote it.
    /// The answer is copied to that bot's chat as a peer card (`peer_flow.rs`).
    peer_asks: HashMap<String, String>,
}

/// One accepted send's hold on the day's limit.
struct Reservation {
    id: String,
    nano: u64,
}

/// How many finished turns the estimate is averaged over.
const RECENT_TURNS: usize = 10;

impl BotRt {
    fn reserved_nano(&self) -> u64 {
        self.reserved.iter().fold(0u64, |sum, r| sum.saturating_add(r.nano))
    }

    /// What one new send is held for: the average of the recent turns, or the floor before any turn has finished.
    fn reserve_estimate(&self) -> u64 {
        if self.recent_turn_nano.is_empty() {
            return cost::RESERVE_FLOOR_NANO;
        }
        let total = self.recent_turn_nano.iter().fold(0u64, |sum, c| sum.saturating_add(*c));
        total / self.recent_turn_nano.len() as u64
    }

    fn new() -> BotRt {
        BotRt {
            keeper: Keeper::new(),
            session: None,
            log: Arc::new(Mutex::new(LogRing::new())),
            blocked: None,
            provider: None,
            typing: HashMap::new(),
            work: HashMap::new(),
            last_error: None,
            last_steps: 0,
            mood: None,
            sends_ms: VecDeque::new(),
            queued: Vec::new(),
            turn_trust: HashMap::new(),
            turn_routine: HashSet::new(),
            paused_for_limit: None,
            last_surface: "app".into(),
            last_reply_id: None,
            chain_step: 0,
            reserved: VecDeque::new(),
            recent_turn_nano: VecDeque::new(),
            reserved_day: String::new(),
            job_turn: None,
            job_after: None,
            queued_ids: HashSet::new(),
            last_status_note_ms: i64::MIN / 2,
            turn_tainted: HashSet::new(),
            restart_after_turn: false,
            peer_asks: HashMap::new(),
        }
    }
}

#[derive(Default)]
struct Inner {
    bots: HashMap<String, BotRt>,
}

/// A piece of work for another thread (a sandbox call can take seconds; the brain's post must not wait for it).
pub(super) type Job = Box<dyn FnOnce() + Send>;
pub(super) type Spawner = Arc<dyn Fn(Job) + Send + Sync>;

fn thread_spawner() -> Spawner {
    Arc::new(|job: Job| {
        let _ = std::thread::Builder::new().name("potato-sandbox-call".into()).spawn(job);
    })
}

pub struct Service {
    root: PathBuf,
    host: Arc<dyn Host>,
    launcher: Arc<dyn Launcher>,
    brains: Arc<Brains>,
    inner: Mutex<Inner>,
    cv: Condvar,
    me: Weak<Service>,
    /// The bot's computer: the dispatcher, the sandbox wired in (if any) and the Labs switch.
    calls: Calls,
    port: Mutex<Option<Port>>,
    sandbox_on: AtomicBool,
    spawner: Mutex<Spawner>,
    /// The helpers' world (`helpers.rs`): what starts them, and the ones now running.
    pub(super) helpers: super::helpers::HelperState,
    /// When the job files were last looked at for a job nobody is running (`job_flow.rs`).
    job_scan_ms: AtomicI64,
    /// The agent turns now running, by bot: the flag that kills the agent's process when it turns true (`agent_flow.rs`).
    agent_runs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// One agent turn's time limit in seconds (a test lowers it).
    agent_turn_seconds: std::sync::atomic::AtomicU64,
    /// How long past that limit the host waits for the sandbox to answer (the sandbox kills the agent itself at the limit).
    agent_grace_ms: std::sync::atomic::AtomicU64,
}

type Result<T> = std::result::Result<T, StoreError>;

fn err(code: &'static str, message: impl Into<String>) -> StoreError {
    StoreError::new(code, message)
}

impl Service {
    pub fn new(root: PathBuf, host: Arc<dyn Host>, launcher: Arc<dyn Launcher>, brains: Arc<Brains>) -> Arc<Service> {
        Arc::new_cyclic(|me| Service {
            calls: Calls::new(root.clone(), host.clone()),
            root,
            host,
            launcher,
            brains,
            inner: Mutex::new(Inner::default()),
            cv: Condvar::new(),
            me: me.clone(),
            port: Mutex::new(None),
            sandbox_on: AtomicBool::new(false),
            spawner: Mutex::new(thread_spawner()),
            helpers: super::helpers::HelperState::default(),
            job_scan_ms: AtomicI64::new(i64::MIN),
            agent_runs: Mutex::new(HashMap::new()),
            agent_turn_seconds: std::sync::atomic::AtomicU64::new(super::custom_agents::TURN_SECONDS),
            agent_grace_ms: std::sync::atomic::AtomicU64::new(20_000),
        })
    }

    // ---- the bot's computer --------------------------------------------------------------------

    /// Wires in the sandbox (and the screen lease it shares with the Computer panel). `None` unwires it.
    pub fn set_sandbox_port(&self, port: Option<Port>) {
        *self.port.lock() = port;
    }

    /// The Sandbox Labs switch (`potato_sandbox_enabled`).
    pub fn set_sandbox_enabled(&self, on: bool) {
        self.sandbox_on.store(on, Ordering::SeqCst);
    }

    /// Whether a brain is told it has a computer: Potato Bot on, the Labs switch on, and a sandbox wired in.
    pub fn sandbox_offered(&self) -> bool {
        self.brains.enabled() && self.sandbox_on.load(Ordering::SeqCst) && self.port.lock().is_some()
    }

    #[cfg(test)]
    pub(super) fn set_spawner(&self, spawner: Spawner) {
        *self.spawner.lock() = spawner;
    }

    #[cfg(test)]
    pub(super) fn calls(&self) -> &Calls {
        &self.calls
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn brains(&self) -> &Arc<Brains> {
        &self.brains
    }

    pub fn enabled(&self) -> bool {
        self.brains.enabled()
    }

    pub(super) fn name_of(&self, bot: &str) -> String {
        self.host.profile(bot).map(|p| p.name).unwrap_or_else(|| bot.to_string())
    }

    pub(super) fn home_of(&self, bot: &str) -> Option<String> {
        self.host.profile(bot).and_then(|p| p.folder)
    }

    pub(super) fn require_bot(&self, bot: &str) -> Result<store::Bot> {
        store::load_bots(&self.root)?
            .into_iter()
            .find(|b| b.id == bot)
            .ok_or_else(|| err("POTATO_BOT_NOT_FOUND", "No bot has that name."))
    }

    // ---- saved messages ------------------------------------------------------------------------

    pub(super) fn save(&self, bot: &str, conversation: &str, message: Message) -> Result<Message> {
        store::append_message(&self.root, conversation, message, self.home_of(bot).as_deref())
    }

    /// A note row in the bot's app thread (best effort: a damaged file is logged, not raised).
    pub(super) fn note(&self, bot: &str, text: &str) {
        let message = Message::new(self.host.now_ms(), FromKind::Note, None, MsgKind::Note, text);
        if let Err(e) = self.save(bot, &store::app_conversation(bot), message) {
            tracing::debug!(%e, "could not save a Potato Bot note");
        }
    }

    pub fn messages(&self, bot: &str, limit: usize) -> Result<Vec<Message>> {
        self.require_bot(bot)?;
        let mut messages = store::read_messages(&self.root, &store::app_conversation(bot), limit.clamp(1, 5000), self.home_of(bot).as_deref())?;
        let costs = reply_cost::load(&self.root);
        let queued = self.inner.lock().bots.get(bot).map(|rt| rt.queued_ids.clone()).unwrap_or_default();
        let reacted = reactions::load(&self.root);
        let undone = super::self_settings::undone_ids(&self.root);
        for m in &mut messages {
            if let Some(change) = m.change.as_mut() {
                change.undone = undone.contains(&change.id);
            }
            m.cost_nano = costs.get(&m.id).copied();
            m.queued = queued.contains(&m.id);
            m.reactions = reacted.get(&m.id).cloned().unwrap_or_default();
        }
        Ok(messages)
    }

    /// Search chats: words in every bot chat and group thread (see `search.rs`). Read only.
    pub fn search_chats(&self, query: &str) -> Result<super::search::Results> {
        super::search::run(&self.root, query, &|bot| self.name_of(bot))
    }

    /// The owner reacts to a message of the bot's thread (or takes the reaction back). App command only: the brain has
    /// no way here. Returns the message's reactions now. A message that is not in this thread is refused.
    pub fn toggle_reaction(&self, bot: &str, message_id: &str, emoji: &str) -> Result<Vec<String>> {
        self.require_bot(bot)?;
        if !reactions::allowed(emoji) {
            return Err(err("POTATO_REACTION_INVALID", "That reaction is not one of the choices."));
        }
        let thread = store::read_messages(&self.root, &store::app_conversation(bot), 500, None)?;
        if !thread.iter().any(|m| m.id == message_id) {
            return Err(err("POTATO_MESSAGE_NOT_FOUND", "That message is not in this chat any more."));
        }
        reactions::toggle(&self.root, message_id, emoji)?;
        self.host.changed(bot);
        Ok(reactions::load(&self.root).remove(message_id).unwrap_or_default())
    }

    // ---- the person's side ---------------------------------------------------------------------

    pub fn add_bot(&self, bot: &str) -> Result<store::Bot> {
        let settings = store::load_settings(&self.root)?;
        if self.host.profile(bot).is_none() {
            return Err(err("POTATO_BOT_NOT_FOUND", "Make that companion first, then turn it into a Potato Bot."));
        }
        let added = store::add_bot(&self.root, bot, &settings.default_persona, self.host.now_ms())?;
        self.host.changed(bot);
        Ok(added)
    }

    /// Changes the voice. The name, threads, memory and mood stay. The brain restarts, because its
    /// persona prefix is fixed when it starts.
    pub fn set_persona(&self, bot: &str, pack: &str) -> Result<store::Bot> {
        let before = self.require_bot(bot)?;
        let after = store::set_talk_persona(&self.root, bot, pack)?;
        if before.talk_persona != after.talk_persona {
            let line = match pack {
                "potato" => "Now talking as Potato.",
                "pro" => "Now talking as Professional.",
                _ => "Now talking as Fish.",
            };
            self.note(bot, line);
            self.stop_bot(bot);
        }
        self.host.changed(bot);
        Ok(after)
    }

    pub fn settings(&self) -> Result<store::Settings> {
        store::load_settings(&self.root)
    }

    /// The settings a bot's brain runs with: the saved ones, with the bot's own talk brain and provider put in
    /// when it has them. Everything else (fallbacks, voice) stays the app's.
    pub(super) fn settings_for(&self, bot: &str, settings: &store::Settings) -> store::Settings {
        let mut own = settings.clone();
        let bot_own = bot_settings::load(&self.root, bot).unwrap_or_default();
        own.brain = bot_settings::effective_brain(settings.brain, &bot_own);
        if let Some(custom) = bot_own.custom {
            own.custom = custom;
        }
        if let Some(model) = bot_own.go_model.filter(|m| !m.is_empty()) {
            own.go_model = model;
        }
        own
    }

    /// Saves the bot's own provider endpoint/model (`None`: the default in Settings). A running brain is stopped,
    /// so the next message starts on the new provider.
    pub fn set_provider(&self, bot: &str, custom: Option<store::CustomProvider>) -> Result<BotSettings> {
        self.require_bot(bot)?;
        let before = bot_settings::load(&self.root, bot)?.custom;
        let saved = bot_settings::update(&self.root, bot, |s| {
            s.custom = bot_settings::clean_provider(custom);
            Ok(())
        })?;
        if before != saved.custom {
            self.note(bot, "Now using its own provider.");
            self.stop_bot(bot);
        }
        self.host.changed(bot);
        Ok(saved)
    }

    /// Picks the model the bot uses on the Go plan (`None`: the default in Settings). A running brain is stopped, so
    /// the next message starts on the new model.
    pub fn set_go_model(&self, bot: &str, model: Option<String>) -> Result<BotSettings> {
        self.require_bot(bot)?;
        let before = bot_settings::load(&self.root, bot)?.go_model;
        let saved = bot_settings::update(&self.root, bot, |s| {
            s.go_model = store::clean_model_id(model.as_deref());
            Ok(())
        })?;
        if before != saved.go_model {
            self.stop_bot(bot);
        }
        self.host.changed(bot);
        Ok(saved)
    }

    /// Picks the bot's own talk brain (`None`: the default in Settings). A running brain is stopped, so the
    /// next message starts it on the new one.
    pub fn set_talk_brain(&self, bot: &str, brain: Option<BrainKind>) -> Result<BotSettings> {
        self.require_bot(bot)?;
        let before = bot_settings::load(&self.root, bot)?.brain;
        let saved = bot_settings::update(&self.root, bot, |s| {
            s.brain = brain;
            Ok(())
        })?;
        if before != brain {
            let line = match brain {
                Some(kind) => format!("Now using {}.", bot_settings::brain_name(kind)),
                None => "Now using the default brain.".to_string(),
            };
            self.note(bot, &line);
            self.stop_bot(bot);
        }
        self.host.changed(bot);
        Ok(saved)
    }

    /// Saves a new version of the instructions (nothing when the text is the one in use). A running brain is
    /// stopped, because its persona is read when it starts.
    pub fn set_instructions(&self, bot: &str, text: &str) -> Result<BotSettings> {
        self.require_bot(bot)?;
        let now = self.host.now_ms();
        let mut changed = false;
        let saved = bot_settings::update(&self.root, bot, |s| {
            changed = bot_settings::save_instructions(s, text, now)?;
            Ok(())
        })?;
        self.after_instructions(bot, changed);
        Ok(saved)
    }

    /// Makes the version saved at `at` the one in use (it is added again as the newest version).
    pub fn restore_instructions(&self, bot: &str, at: i64) -> Result<BotSettings> {
        self.require_bot(bot)?;
        let now = self.host.now_ms();
        let mut changed = false;
        let saved = bot_settings::update(&self.root, bot, |s| {
            changed = bot_settings::restore_instructions(s, at, now)?;
            Ok(())
        })?;
        self.after_instructions(bot, changed);
        Ok(saved)
    }

    fn after_instructions(&self, bot: &str, changed: bool) {
        if changed {
            self.stop_bot(bot);
        }
        self.host.changed(bot);
    }

    /// The tool switches. They are checked before each tool call, so nothing restarts.
    pub fn set_tools(&self, bot: &str, tools: ToolSwitches) -> Result<BotSettings> {
        self.require_bot(bot)?;
        let saved = bot_settings::update(&self.root, bot, |s| {
            s.tools = tools;
            Ok(())
        })?;
        self.host.changed(bot);
        Ok(saved)
    }

    /// Sets how much the bot asks (the Levels setting). Only the app command calls this.
    pub fn set_level(&self, bot: &str, level: super::risk::Level) -> Result<BotSettings> {
        self.require_bot(bot)?;
        let saved = bot_settings::update(&self.root, bot, |s| {
            s.level = level;
            Ok(())
        })?;
        self.host.changed(bot);
        Ok(saved)
    }

    /// Sets the owner's limit on the sites the bot's web tools open. Only the app command calls this; no tool the bot can call reaches it.
    pub fn set_site_limit(&self, bot: &str, enabled: bool, sites: &[String]) -> Result<BotSettings> {
        self.require_bot(bot)?;
        let sites = bot_settings::clean_sites(sites)?;
        let saved = bot_settings::update(&self.root, bot, |s| {
            s.site_limit = bot_settings::SiteLimit { enabled, sites };
            Ok(())
        })?;
        self.host.changed(bot);
        Ok(saved)
    }

    /// Whether the bot exists (the check the commands make before they act on its companion).
    pub fn require_bot_exists(&self, bot: &str) -> Result<()> {
        self.require_bot(bot).map(|_| ())
    }

    /// What the Advanced tab shows for a bot.
    pub fn bot_options(&self, bot: &str) -> Result<BotOptions> {
        self.require_bot(bot)?;
        let settings = bot_settings::load(&self.root, bot)?;
        Ok(bot_settings::options_of(&settings, self.sandbox_on.load(Ordering::SeqCst)))
    }

    /// The bot's chat as Markdown (the text only; the save goes through the page's save path).
    pub fn export_markdown(&self, bot: &str) -> Result<String> {
        self.require_bot(bot)?;
        let messages = store::read_messages(&self.root, &store::app_conversation(bot), bot_settings::EXPORT_MESSAGES, self.home_of(bot).as_deref())?;
        Ok(bot_settings::chat_markdown(&self.name_of(bot), &messages))
    }

    /// Deletes a bot: its brain is stopped, it leaves the list, and its chat, settings, read mark and brain
    /// folder are moved into `potato/.removed/` (nothing is deleted). The name must be typed exactly. The
    /// companion stays: it is still a companion, and can be made a bot again.
    pub fn delete_bot(&self, bot: &str, typed_name: &str) -> Result<()> {
        self.require_bot(bot)?;
        if typed_name.trim() != self.name_of(bot) {
            return Err(err("POTATO_NAME_MISMATCH", "Type the bot's name exactly to delete it."));
        }
        let now = self.host.now_ms();
        let name = self.name_of(bot);
        self.stop_all_of(bot, Duration::from_millis(500));
        // Moved into Recently deleted (potato/trash.rs): kept for 7 days, brought back by `potato_trash_restore`.
        super::trash::trash_bot(&self.root, bot, &name, now)?;
        self.inner.lock().bots.remove(bot);
        self.host.changed(bot);
        Ok(())
    }

    pub fn set_settings(&self, new: store::Settings) -> Result<store::Settings> {
        let old = store::load_settings(&self.root)?;
        store::save_settings(&self.root, &new)?;
        let saved = store::load_settings(&self.root)?;
        if old.brain != saved.brain {
            // A different brain starts fresh.
            self.stop_all(Duration::from_millis(300));
        }
        self.recheck_limits();
        Ok(saved)
    }

    /// Archive or restore a bot's chat. Archiving stops its brain (the next message starts it again); no message is removed.
    pub fn set_archived(&self, bot: &str, archived: bool) -> Result<()> {
        self.require_bot(bot)?;
        if archived {
            self.stop_bot_by_person(bot);
        }
        store::set_archived(&self.root, bot, archived)?;
        self.host.changed(bot);
        Ok(())
    }

    pub fn mark_read(&self, bot: &str) -> Result<()> {
        self.require_bot(bot)?;
        store::mark_read(&self.root, bot, self.host.now_ms())?;
        self.host.changed(bot);
        Ok(())
    }

    pub fn forget_talk_memory(&self, bot: &str) -> Result<()> {
        self.require_bot(bot)?;
        self.stop_all_of(bot, Duration::from_millis(500));
        let stamp = format!("{}", self.host.now_ms());
        store::forget_talk_memory(&self.root, bot, &stamp)?;
        self.note(bot, "Talk memory was cleared. The old copy is kept in the removed folder.");
        self.host.changed(bot);
        Ok(())
    }

    fn rate_ok(rt: &mut BotRt, now: i64) -> u32 {
        while rt.sends_ms.front().is_some_and(|at| now - *at >= SEND_WINDOW_MS) {
            rt.sends_ms.pop_front();
        }
        rt.sends_ms.len() as u32
    }

    /// What stops a bot from starting at all, found before anything is spawned.
    fn preflight(&self, bot: &str, settings: &store::Settings) -> Option<Blocked> {
        match settings.brain {
            BrainKind::Agents => None,
            BrainKind::Custom if !self.has_provider_key(bot) || !settings.custom.ready() => Some(Blocked::NeedsKey),
            BrainKind::Deepseek if !self.has_key() => Some(Blocked::NeedsKey),
            _ => {
                if !self.launcher.node().is_ready() {
                    return Some(Blocked::NeedsNode);
                }
                if super::sidecar::script_path(self.host.resources().as_deref()).is_none() {
                    return Some(Blocked::MissingProgram);
                }
                if self.host.bridge_url().is_none() {
                    return Some(Blocked::NoBridge);
                }
                None
            }
        }
    }

    fn has_key(&self) -> bool {
        self.host.secret(SECRET_DEEPSEEK).is_some_and(|k| !k.trim().is_empty())
    }

    /// The bot's own provider key, else the app-wide one (a bot made before per-bot providers keeps working).
    fn has_provider_key(&self, bot: &str) -> bool {
        self.host
            .secret(&provider_secret_name(bot))
            .or_else(|| self.host.secret(SECRET_PROVIDER))
            .is_some_and(|k| !k.trim().is_empty())
    }

    fn blocked_note(&self, bot: &str, blocked: Blocked) -> String {
        let name = self.name_of(bot);
        match blocked {
            Blocked::NeedsKey => format!("Add this bot's model key in Settings to talk with {name}."),
            Blocked::NeedsNode => "Potato Bot needs Node.js 20 or newer.".to_string(),
            Blocked::MissingProgram => "Potato Bot's brain program is missing. Reinstall Puppet Master.".to_string(),
            Blocked::NoBridge => "Puppet Master is still starting. Try again in a moment.".to_string(),
        }
    }

    /// One message from the person at this computer.
    pub fn send(&self, bot: &str, text: &str) -> Result<SendOutcome> {
        self.send_as(bot, text, Origin::Person)
    }

    /// A message for a bot's brain, from the person (`Origin::Person`) or from a due routine (`Origin::Routine`).
    /// Both are trusted as the owner at this computer; the routine's late note travels as the item's `context`.
    pub(super) fn send_as(&self, bot: &str, text: &str, origin: Origin<'_>) -> Result<SendOutcome> {
        self.require_bot(bot)?;
        let now = self.host.now_ms();
        let settings = self.settings_for(bot, &store::load_settings(&self.root)?);
        let day = self.host.local_day(now);
        let used = cost::day_of(&self.root, &day, bot).map_err(|e| err("POTATO_STORE_INVALID", e))?.cost_nano();
        let over = cost::budget_with(used, settings.daily_limit()) == Budget::Over;
        let held = hold::is_held(&self.root);
        let text = text.trim();
        let conversation = store::app_conversation(bot);
        let name = self.name_of(bot);
        // Asked of the PC before the lock is taken: finding Node can take a moment.
        let blocked = if self.brains.enabled() { self.preflight(bot, &settings) } else { None };

        let mut g = self.inner.lock();
        let rt = g.bots.entry(bot.to_string()).or_insert_with(BotRt::new);
        // The day's cost plus what the queued sends are held for, plus this send's estimate: past the limit, refuse.
        let estimate = rt.reserve_estimate();
        let committed = used.saturating_add(rt.reserved_nano());
        let reserve_over = cost::reserve_over(committed.saturating_add(estimate), settings.daily_limit());
        let caps = Caps {
            disabled: !self.brains.enabled(),
            paused: held,
            over_limit: over || reserve_over,
            // A job turn is the app's own, one at a time: it is not part of the person's 20 sends a minute.
            sends_last_minute: if matches!(origin, Origin::Job { .. } | Origin::Queued { .. }) { 0 } else { Self::rate_ok(rt, now) },
        };
        let facts = Facts {
            surface: "app",
            sender: Sender::App,
            channel: Channel::NotAChannel,
            mentioned: true,
            replied_to_bot: false,
            from_bot: false,
            age_ms: 0,
            text_is_empty: text.is_empty(),
        };
        let decision = rules::decide(&facts, &rules::Config::default(), &caps);
        let (trust, tools) = match decision {
            Decision::Send { trust, tools, .. } => {
                // Another bot's message is a turn nobody trusted: no owner tools, no computer, no memory, whatever it says.
                if matches!(origin, Origin::Peer { .. }) { (Trust::Others, Tools::NoMemory) } else { (trust, tools) }
            }
            Decision::Drop(rules::Why::Disabled) => return Err(err("POTATO_OFF", "Potato Bot is switched off.")),
            Decision::Drop(rules::Why::Empty) => return Err(err("POTATO_EMPTY", "Write a message first.")),
            Decision::Drop(rules::Why::Paused) => return Err(err("POTATO_PAUSED", "Bots are paused. Press Start bots again to resume.")),
            Decision::Drop(rules::Why::RateLimited) => return Err(err("POTATO_RATE", "That is too fast. Wait a moment, then try again.")),
            // A job turn past the day's limit is not saved or noted here: the job says so when it stops.
            Decision::Drop(rules::Why::OverLimit) if matches!(origin, Origin::Job { .. } | Origin::Queued { .. }) => {
                return Ok(SendOutcome { id: String::new(), state: BrainState::DailyLimit, delivered: false });
            }
            Decision::Drop(rules::Why::OverLimit) => {
                drop(g);
                // A job's turn or the owner's card answer is the model's own context, not a line of the chat: it is not shown.
                let mine = origin.message(now, text);
                let saved = if matches!(origin, Origin::Job { .. } | Origin::Answer) { mine } else { self.save(bot, &conversation, mine)? };
                match origin {
                    Origin::Routine { title, .. } => self.note(bot, &format!("Skipped: daily limit reached: {title}")),
                    Origin::Person | Origin::Job { .. } | Origin::Answer | Origin::Queued { .. } | Origin::Peer { .. } => self.note(bot, &format!("{name} has used today's limit ({}). Raise it in Settings or wait until tomorrow.", cost::dollars(settings.daily_limit_cents))),
                }
                self.host.changed(bot);
                return Ok(SendOutcome { id: saved.id, state: BrainState::DailyLimit, delivered: false });
            }
            Decision::Drop(_) | Decision::Pending => return Err(err("POTATO_REFUSED", "That message was not sent.")),
        };

        let mine = origin.message(now, text);
        // A job's next turn is not a line in the thread: the job's own notes tell what it does.
        let saved = if matches!(origin, Origin::Job { .. } | Origin::Answer | Origin::Queued { .. }) { mine } else { self.save(bot, &conversation, mine)? };
        // A routine run is written as "due, not yet taken" before it goes to the queue; the brain's reply clears it.
        if let Origin::Routine { title, .. } = origin {
            super::routines::add_pending(&self.root, super::routines::PendingRun { id: saved.id.clone(), bot: bot.to_string(), title: title.to_string(), at_ms: now });
        }
        let sent_text = crate::redact::text(&store::clean_text(text, self.home_of(bot).as_deref())).into_inner();

        if settings.brain == BrainKind::Agents {
            drop(g);
            if self.agent_brain_id(bot).is_some() {
                return self.agent_turn(bot, &saved.id, &conversation, origin, &sent_text);
            }
            self.note(bot, "Talking through your agents is not ready yet.");
            self.host.changed(bot);
            return Ok(SendOutcome { id: saved.id, state: BrainState::Agents, delivered: false });
        }
        if let Some(blocked) = blocked {
            rt.blocked = Some(blocked);
            let line = self.blocked_note(bot, blocked);
            drop(g);
            self.note(bot, &line);
            self.host.changed(bot);
            let state = if blocked == Blocked::NeedsKey { BrainState::NeedsKey } else if blocked == Blocked::NeedsNode { BrainState::NeedsNode } else { BrainState::MissingProgram };
            return Ok(SendOutcome { id: saved.id, state, delivered: false });
        }
        if rt.keeper.is_stopped() {
            let line = log::stopped_note(&name, &rt.log.lock().lines());
            drop(g);
            self.note(bot, &line);
            self.host.changed(bot);
            return Ok(SendOutcome { id: saved.id, state: BrainState::Stopped, delivered: false });
        }
        rt.blocked = None;
        if rt.session.is_none() {
            // A new run starts on the talk brain; a fallback only lasts for the run it was needed in.
            rt.chain_step = 0;
        }
        if matches!(origin, Origin::Routine { .. }) {
            // Brain-started turns have their own daily cap; the person's own messages do not count against it (spec 6.2).
            let taken = cost::take_brain_turn(&self.root, &day, bot, cost::DEFAULT_BRAIN_TURNS);
            match taken {
                Ok(true) => {}
                Ok(false) => {
                    drop(g);
                    self.note(bot, &format!("{name} has started {} things on its own today. It will wait until tomorrow.", cost::DEFAULT_BRAIN_TURNS));
                    self.host.changed(bot);
                    return Err(err("POTATO_TURNS", "The brain-started turns for today are used up."));
                }
                Err(e) => return Err(err("POTATO_STORE_INVALID", e)),
            }
        }
        rt.reserved.push_back(Reservation { id: saved.id.clone(), nano: estimate });
        if !matches!(origin, Origin::Job { .. } | Origin::Queued { .. }) {
            rt.sends_ms.push_back(now);
        }
        // The person writes while a turn for this chat is running: the bot reads the message between its tool rounds, and
        // until it has, the thread shows "Queued". A job keeps the message too, so a restart does not lose it.
        // (Files are touched after the engine lock is let go, below.)
        let mut keep_waiting = false;
        let mut status_run = None;
        if matches!(origin, Origin::Person | Origin::Queued { .. }) {
            let turn_open = rt.typing.contains_key(&conversation) || rt.session.as_ref().is_some_and(|s| s.in_flight.values().any(|c| *c == conversation));
            if turn_open {
                rt.queued_ids.insert(saved.id.clone());
                keep_waiting = true;
                // A status question while one tool call has run a long time: the app answers from its own record.
                if matches!(origin, Origin::Person) && super::steer::is_status_question(text) && now - rt.last_status_note_ms >= super::steer::NOTE_EVERY_MS {
                    status_run = rt.work.get(&conversation).cloned();
                }
            }
        }
        // A new turn starts clean; a message the person adds to a turn already running does not clean it.
        let running_now = rt.typing.contains_key(&conversation) || rt.session.as_ref().is_some_and(|s| s.in_flight.values().any(|c| *c == conversation));
        if !running_now {
            rt.turn_tainted.remove(&conversation);
        }
        if let Origin::Peer { from_id, .. } = origin {
            rt.peer_asks.insert(saved.id.clone(), from_id.to_string());
        }
        rt.turn_trust.insert(conversation.clone(), trust);
        if matches!(origin, Origin::Routine { .. }) {
            rt.turn_routine.insert(conversation.clone());
        } else {
            rt.turn_routine.remove(&conversation);
        }
        let item = protocol::send_item(&protocol::SendFacts {
            id: &saved.id,
            conversation_id: &conversation,
            surface: "app",
            from_id: origin.from_id(),
            from_name: origin.from_name(),
            trust,
            text: &sent_text,
            context: origin.context(),
            tools,
        });
        // The computer is offered only to a turn whose trust allows it (never `others`).
        let item = protocol::with_capabilities(item, self.sandbox_offered() && rules::tool_access(trust).sandbox);
        self.deliver(rt, &saved.id, &conversation, item);
        if matches!(origin, Origin::Job { .. }) {
            rt.job_turn = Some(JobTurn::Turn(saved.id.clone()));
        }
        rt.keeper.want_run(now);
        let actions = rt.keeper.step(now);
        self.run_actions(&mut g, bot, actions);
        let state = self.state_of(&g, bot, &settings, over);
        drop(g);
        if keep_waiting {
            jobs::queue_send(&self.root, bot, &saved.id, &sent_text, now);
        }
        if let Some(run) = status_run {
            let running = jobs::load(&self.root, bot).filter(jobs::Job::is_running);
            if let Some(line) = super::steer::slow_step_note(&name, &run, running.as_ref(), now) {
                if let Some(rt) = self.inner.lock().bots.get_mut(bot) {
                    rt.last_status_note_ms = now;
                }
                self.note(bot, &line);
            }
        }
        self.host.changed(bot);
        Ok(SendOutcome { id: saved.id, state, delivered: true })
    }

    /// Puts a `send` into the live session's inbox, or keeps it for the next one.
    fn deliver(&self, rt: &mut BotRt, id: &str, conversation: &str, item: Value) {
        match rt.session.as_mut() {
            Some(session) if session.shutdown_at.is_none() => {
                session.send_index.insert(id.to_string(), session.inbox.end());
                session.in_flight.insert(id.to_string(), conversation.to_string());
                session.inbox.push(item);
                self.cv.notify_all();
            }
            _ => rt.queued.push(item),
        }
    }

    /// "Start again": forgets the crashes and starts.
    pub fn start_bot(&self, bot: &str) -> Result<()> {
        self.require_bot(bot)?;
        if hold::is_held(&self.root) {
            return Err(err("POTATO_PAUSED", "Bots are paused. Press Start bots again to resume."));
        }
        let now = self.host.now_ms();
        let settings = self.settings_for(bot, &store::load_settings(&self.root)?);
        let blocked = self.preflight(bot, &settings);
        let mut g = self.inner.lock();
        let rt = g.bots.entry(bot.to_string()).or_insert_with(BotRt::new);
        rt.blocked = blocked;
        if blocked.is_none() && settings.brain != BrainKind::Agents && self.brains.enabled() {
            rt.keeper.retry(now);
            let actions = rt.keeper.step(now);
            self.run_actions(&mut g, bot, actions);
        }
        drop(g);
        self.host.changed(bot);
        Ok(())
    }

    /// The page's Stop button: asks the brain to leave. The next message starts it again.
    pub fn stop_bot(&self, bot: &str) {
        self.cancel_agent(bot);
        let now = self.host.now_ms();
        let mut g = self.inner.lock();
        if let Some(rt) = g.bots.get_mut(bot) {
            rt.keeper.person_stop();
            let actions = rt.keeper.step(now);
            self.run_actions(&mut g, bot, actions);
        }
        drop(g);
        self.host.changed(bot);
    }

    /// The page's Stop button as the person pressed it: the bot's job ends for good, then the brain leaves. Other
    /// restarts of the brain (a new persona, a new key) use `stop_bot` and leave the job running.
    pub fn stop_bot_by_person(&self, bot: &str) {
        self.stop_job(bot, jobs::reason::STOPPED_BY_YOU);
        self.stop_bot(bot);
    }

    /// How many brains are running right now (the "stop everything" sheet counts them).
    pub fn running_bots(&self) -> usize {
        self.inner.lock().bots.values().filter(|rt| rt.session.is_some()).count()
    }

    /// Stops every brain now: the kill switch, the panic button and the app's exit. Each brain is asked to
    /// leave and given `grace`; what is still there is killed with its whole tree.
    pub fn stop_all(&self, grace: Duration) {
        let bots: Vec<String> = self.inner.lock().bots.keys().cloned().collect();
        for bot in &bots {
            self.ask_to_leave(bot);
        }
        self.finish_stops(&bots, grace);
    }

    fn stop_all_of(&self, bot: &str, grace: Duration) {
        self.ask_to_leave(bot);
        self.finish_stops(&[bot.to_string()], grace);
    }

    fn ask_to_leave(&self, bot: &str) {
        self.cancel_agent(bot);
        let now = self.host.now_ms();
        let mut g = self.inner.lock();
        if let Some(rt) = g.bots.get_mut(bot) {
            rt.keeper.person_stop();
            let actions = rt.keeper.step(now);
            self.run_actions(&mut g, bot, actions);
        }
    }

    fn finish_stops(&self, bots: &[String], grace: Duration) {
        let deadline = std::time::Instant::now() + grace;
        loop {
            let mut any = false;
            {
                let mut g = self.inner.lock();
                for bot in bots {
                    let gone = g.bots.get_mut(bot).and_then(|rt| rt.session.as_mut()).map(|s| s.child.try_wait().is_some());
                    match gone {
                        Some(true) => self.end_session(&mut g, bot, true),
                        Some(false) => any = true,
                        None => {}
                    }
                }
            }
            if !any || std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let mut g = self.inner.lock();
        for bot in bots {
            if let Some(session) = g.bots.get_mut(bot).and_then(|rt| rt.session.as_mut()) {
                session.child.kill_tree();
            }
            self.end_session(&mut g, bot, true);
        }
        drop(g);
        for bot in bots {
            self.host.changed(bot);
        }
    }

    /// Potato Bot on or off (the Labs switch). Off kills every brain, makes the brain routes 404 and
    /// withdraws the helper cards.
    pub fn set_enabled(&self, on: bool) {
        let was = self.brains.enabled();
        if was && !on {
            // The routes stay open for a moment so `shutdown` can reach a brain; then they close and
            // whatever is left is killed.
            self.stop_all(Duration::from_millis(300));
            self.brains.set_enabled(false);
            self.stop_all(Duration::ZERO);
            self.withdraw_cards();
        } else {
            self.brains.set_enabled(on);
        }
    }

    /// A card nobody answered is withdrawn when Potato Bot goes off.
    fn withdraw_cards(&self) {
        let Ok(bots) = store::load_bots(&self.root) else { return };
        for bot in bots {
            for request_id in self.open_card_ids(&bot.id) {
                let mut row = Message::new(self.host.now_ms(), FromKind::Note, None, MsgKind::HelperResult, "Withdrawn: Potato Bot was switched off.");
                row.request_id = Some(request_id);
                let _ = self.save(&bot.id, &store::app_conversation(&bot.id), row);
            }
            self.host.changed(&bot.id);
        }
    }

    // ---- views ---------------------------------------------------------------------------------

    pub(super) fn open_card_ids(&self, bot: &str) -> Vec<String> {
        let Ok(messages) = store::read_messages(&self.root, &store::app_conversation(bot), 500, None) else { return Vec::new() };
        let closed: Vec<&str> = messages.iter().filter(|m| m.kind == MsgKind::HelperResult).filter_map(|m| m.request_id.as_deref()).collect();
        messages
            .iter()
            .filter(|m| m.kind == MsgKind::HelperCard)
            .filter_map(|m| m.request_id.clone())
            .filter(|id| !closed.contains(&id.as_str()))
            .collect()
    }

    fn state_of(&self, g: &Inner, bot: &str, settings: &store::Settings, over: bool) -> BrainState {
        if !self.brains.enabled() {
            return BrainState::Off;
        }
        if settings.brain == BrainKind::Agents {
            // An approved agent of the owner's own runs in the sandbox; no companion chat stands in for the thread.
            if self.agent_brain_id(bot).is_some() {
                return if self.agent_running(bot) { BrainState::Ready } else { BrainState::Idle };
            }
            return BrainState::Agents;
        }
        let Some(rt) = g.bots.get(bot) else {
            return match self.preflight_cheap(bot, settings) {
                Some(Blocked::NeedsKey) => BrainState::NeedsKey,
                Some(Blocked::NeedsNode) => BrainState::NeedsNode,
                Some(Blocked::MissingProgram) => BrainState::MissingProgram,
                _ if over => BrainState::DailyLimit,
                _ => BrainState::Idle,
            };
        };
        match rt.keeper.phase() {
            Phase::Stopped => return BrainState::Stopped,
            Phase::Starting { .. } => return BrainState::Starting,
            Phase::Waiting { .. } => return BrainState::Restarting,
            Phase::Ready => {
                return if over && rt.paused_for_limit.is_some() { BrainState::DailyLimit } else { BrainState::Ready };
            }
            Phase::Off => {}
        }
        match rt.blocked.or_else(|| self.preflight_cheap(bot, settings)) {
            Some(Blocked::NeedsKey) => BrainState::NeedsKey,
            Some(Blocked::NeedsNode) => BrainState::NeedsNode,
            Some(Blocked::MissingProgram) => BrainState::MissingProgram,
            _ if over => BrainState::DailyLimit,
            _ => BrainState::Idle,
        }
    }

    /// Like `preflight`, but never probes Node (a probe can take seconds): the key only, and what the
    /// last probe found.
    fn preflight_cheap(&self, bot: &str, settings: &store::Settings) -> Option<Blocked> {
        match settings.brain {
            BrainKind::Deepseek if !self.has_key() => Some(Blocked::NeedsKey),
            BrainKind::Custom if !self.has_provider_key(bot) || !settings.custom.ready() => Some(Blocked::NeedsKey),
            _ => None,
        }
    }

    pub fn state(&self) -> Result<PotatoState> {
        let settings = store::load_settings(&self.root)?;
        let now = self.host.now_ms();
        let day = self.host.local_day(now);
        let reads = store::load_reads(&self.root)?;
        let key = secrets::mask(self.host.secret(SECRET_DEEPSEEK).as_deref());
        let node = match self.launcher.node() {
            NodeState::Ready { .. } => NodeView { ready: true, found: None },
            NodeState::Missing => NodeView { ready: false, found: None },
            NodeState::TooOld { found } => NodeView { ready: false, found: Some(found) },
        };
        let archived = store::load_archived(&self.root)?;
        let mut bots = Vec::new();
        for bot in store::load_bots(&self.root)? {
            let used = cost::day_of(&self.root, &day, &bot.id).map(|d| d.cost_nano()).unwrap_or(0);
            let over = cost::budget_with(used, settings.daily_limit()) == Budget::Over;
            let thread = store::read_messages(&self.root, &store::app_conversation(&bot.id), 200, self.home_of(&bot.id).as_deref()).unwrap_or_default();
            let read_at = reads.get(&bot.id).copied().unwrap_or(0);
            let unread = thread.iter().filter(|m| m.from.kind == FromKind::Bot && m.at > read_at).count() as u32;
            let last = thread.iter().rev().find(|m| m.kind != MsgKind::Note);
            let job_running = jobs::load(&self.root, &bot.id).is_some_and(|j| j.is_running());
            let g = self.inner.lock();
            let rt = g.bots.get(&bot.id);
            let brain_state = self.state_of(&g, &bot.id, &self.settings_for(&bot.id, &settings), over);
            let view = BotView {
                archived: archived.contains(&bot.id),
                name: self.name_of(&bot.id),
                talk_persona: bot.talk_persona.clone(),
                brain_state,
                talk_brain: bot_settings::load(&self.root, &bot.id).unwrap_or_default().brain,
                last_error: rt.and_then(|r| r.last_error.map(|c| if job_stopped(c, r.last_steps) { JOB_STOPPED.to_string() } else { c.as_str().to_string() })),
                typing: rt.is_some_and(|r| r.typing.values().any(|at| now - *at < TYPING_TTL_MS)),
                work: rt.and_then(|r| super::work::shown(&r.work, &r.typing, &store::app_conversation(&bot.id))).map(|mut w| {
                    // A turn that ended inside a running job is not the end of the work: the next turn starts by itself.
                    w.continues = job_running;
                    w
                }),
                mood: rt.and_then(|r| r.mood.clone()),
                provider: rt.and_then(|r| r.provider.clone()),
                unread,
                last_line: last.map(|m| m.text.chars().take(80).collect()),
                last_at: last.map(|m| m.at),
                used_today_nano: used,
                limit_cents: settings.daily_limit_cents,
                limit_off: settings.daily_limit_off,
                open_cards: (self.open_card_ids(&bot.id).len() + crate::potato::cards::open_cards(&self.root, &bot.id).len()) as u32,
                id: bot.id,
            };
            drop(g);
            bots.push(view);
        }
        Ok(PotatoState { enabled: self.brains.enabled(), settings, key, node, bots })
    }

    /// The process id of the bot's running brain, if it has one.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn brain_pid(&self, bot: &str) -> Option<u32> {
        self.inner.lock().bots.get(bot).and_then(|rt| rt.session.as_ref()).map(|s| s.child.pid())
    }

    /// The cleaned lines of the brain's log (200 at most).
    pub fn log_lines(&self, bot: &str) -> Vec<String> {
        self.inner.lock().bots.get(bot).map(|rt| rt.log.lock().lines()).unwrap_or_default()
    }

    // ---- hooks for the helpers (helpers.rs) ---------------------------------------------------

    /// Puts one item into the live brain's inbox. False when the bot has no running brain (or one that is
    /// leaving): the item is not kept, because a brain that starts later knows nothing of the request.
    pub(super) fn push_to_brain(&self, bot: &str, item: Value) -> bool {
        let mut g = self.inner.lock();
        match g.bots.get_mut(bot).and_then(|rt| rt.session.as_mut()) {
            Some(session) if session.shutdown_at.is_none() => {
                session.inbox.push(item);
                self.cv.notify_all();
                true
            }
            _ => false,
        }
    }

    /// Whether the brain may be given a turn of its own now: it is not paused for the day's limit and the
    /// day's cost is under the limit.
    pub(super) fn brain_may_take_a_turn(&self, bot: &str) -> bool {
        let Ok(settings) = store::load_settings(&self.root) else { return false };
        let day = self.host.local_day(self.host.now_ms());
        let used = cost::day_of(&self.root, &day, bot).map(|d| d.cost_nano()).unwrap_or(0);
        if cost::budget_with(used, settings.daily_limit()) == Budget::Over {
            return false;
        }
        !self.inner.lock().bots.get(bot).is_some_and(|rt| rt.paused_for_limit.is_some())
    }

    /// A helper is still working: the idle timer of that bot's brain does not run out.
    pub(super) fn keep_awake(&self, bot: &str) {
        let now = self.host.now_ms();
        if let Some(rt) = self.inner.lock().bots.get_mut(bot) {
            rt.keeper.touch(now);
        }
    }

    pub(super) fn host(&self) -> &Arc<dyn Host> {
        &self.host
    }

    pub(super) fn changed(&self, bot: &str) {
        self.host.changed(bot);
    }

    // ---- keeper actions ------------------------------------------------------------------------

    fn run_actions(&self, g: &mut Inner, bot: &str, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Spawn => self.spawn(g, bot),
                Action::Shutdown => {
                    let now = self.host.now_ms();
                    if let Some(session) = g.bots.get_mut(bot).and_then(|rt| rt.session.as_mut()) {
                        if session.shutdown_at.is_none() {
                            session.shutdown_at = Some(now);
                            session.inbox.push(protocol::shutdown_item());
                            self.cv.notify_all();
                        }
                    }
                }
                Action::Kill => {
                    if let Some(rt) = g.bots.get(bot) {
                        rt.log.lock().push("the brain did not say hello in time", &[]);
                    }
                    if let Some(session) = g.bots.get_mut(bot).and_then(|rt| rt.session.as_mut()) {
                        session.child.kill_tree();
                    }
                    self.end_session(g, bot, false);
                }
                Action::RaiseStopped => {
                    let name = self.name_of(bot);
                    let lines = g.bots.get(bot).map(|rt| rt.log.lock().lines()).unwrap_or_default();
                    let line = log::stopped_note(&name, &lines);
                    self.note(bot, &line);
                    self.note_waiting_rooms(g, bot, &line);
                }
            }
        }
    }

    /// A bot that cannot start leaves its reason in every group that has a line waiting for it, so a person who wrote
    /// in a group is never left with silence.
    fn note_waiting_rooms(&self, g: &Inner, bot: &str, text: &str) {
        let Some(rt) = g.bots.get(bot) else { return };
        let mut seen: Vec<&str> = Vec::new();
        for item in &rt.queued {
            let Some(conversation) = item.get("conversationId").and_then(Value::as_str) else { continue };
            if store::app_room_of(conversation).is_none() || seen.contains(&conversation) {
                continue;
            }
            seen.push(conversation);
            self.conversation_note(bot, conversation, self.host.now_ms(), text);
        }
    }

    fn spawn(&self, g: &mut Inner, bot: &str) {
        let now = self.host.now_ms();
        let rt = g.bots.entry(bot.to_string()).or_insert_with(BotRt::new);
        match self.try_spawn(rt, bot) {
            Ok(()) => rt.keeper.spawned(now),
            Err(Some(blocked)) => {
                // Not a crash: it waits for the person.
                rt.blocked = Some(blocked);
                rt.keeper.person_stop();
                let line = self.blocked_note(bot, blocked);
                self.note(bot, &line);
                self.note_waiting_rooms(g, bot, &line);
            }
            Err(None) => {
                let actions = rt.keeper.spawn_failed(now);
                self.run_actions(g, bot, actions);
            }
        }
    }

    /// Starts the process. `Err(Some(_))`: something the person must fix; `Err(None)`: it failed (a crash).
    fn try_spawn(&self, rt: &mut BotRt, bot: &str) -> std::result::Result<(), Option<Blocked>> {
        let settings = self.settings_for(bot, &store::load_settings(&self.root).map_err(fail)?);
        let bot_own = bot_settings::load(&self.root, bot).map_err(fail)?;
        if settings.brain == BrainKind::Agents {
            return Err(None);
        }
        let node = match self.launcher.node() {
            NodeState::Ready { path, .. } => path,
            _ => return Err(Some(Blocked::NeedsNode)),
        };
        let resources = self.host.resources();
        let script = super::sidecar::script_path(resources.as_deref()).ok_or(Some(Blocked::MissingProgram))?;
        let url = self.host.bridge_url().ok_or(Some(Blocked::NoBridge))?;
        let key = self.host.secret(SECRET_DEEPSEEK).filter(|k| !k.trim().is_empty());
        let provider_key = self
            .host
            .secret(&provider_secret_name(bot))
            .or_else(|| self.host.secret(SECRET_PROVIDER))
            .filter(|k| !k.trim().is_empty());
        let chain = fallback::chain(settings.brain, &settings.fallbacks, key.is_some());
        rt.chain_step = rt.chain_step.min(chain.len() - 1);
        let kind = chain[rt.chain_step];
        let go_key = (kind == BrainKind::Go)
            .then(|| self.host.secret(SECRET_GO).filter(|k| !k.trim().is_empty()).or_else(|| self.host.go_key()))
            .flatten()
            .filter(|k| !k.trim().is_empty());
        if kind == BrainKind::Deepseek && key.is_none() {
            return Err(Some(Blocked::NeedsKey));
        }
        if kind == BrainKind::Custom && (provider_key.is_none() || !settings.custom.ready()) {
            return Err(Some(Blocked::NeedsKey));
        }
        let pack = store::load_bots(&self.root).map_err(fail)?.into_iter().find(|b| b.id == bot).map(|b| b.talk_persona).ok_or(None::<Blocked>)?;
        let cwd = store::ensure_brain_dirs(&self.root, bot).map_err(fail)?;
        if let Some(persona) = resources.as_ref().map(|r| r.join("persona")).filter(|p| p.is_dir()) {
            store::seed_persona(&self.root, bot, &persona).map_err(fail)?;
        }
        let token = auth::mint_token();
        let host_env = self.host.host_env();
        let vault = store::vault_dir(&self.root, bot).map_err(fail)?;
        let persona_dir = store::persona_dir(&self.root, bot).map_err(fail)?;
        // The instructions in use travel the same way as the persona: a file in the persona folder.
        bot_settings::write_instructions_file(&persona_dir, &bot_own).map_err(fail)?;
        let state_dir = store::state_dir(&self.root, bot).map_err(fail)?;
        let share_dir = store::share_dir(&self.root, bot).map_err(fail)?;
        let brain_env = env::build(&env::Inputs {
            brain: match kind {
                BrainKind::Go => Brain::Go,
                BrainKind::Custom => Brain::Custom,
                _ => Brain::DeepSeek,
            },
            pack: &pack,
            model: match kind {
                BrainKind::Custom => Some(settings.custom.model.as_str()),
                BrainKind::Go => Some(settings.go_model.as_str()),
                _ => None,
            },
            vault: &vault,
            persona_dir: &persona_dir,
            state_dir: &state_dir,
            share_dir: &share_dir,
            url: &url,
            token: &token,
            bot,
            deepseek_key: key.as_deref(),
            go_key: go_key.as_deref(),
            custom_base_url: Some(settings.custom.base_url.as_str()),
            custom_key: provider_key.as_deref(),
            host: &host_env,
        });
        let secret_values = brain_env.secrets();
        self.brains.register(bot, &token);
        let ring = rt.log.clone();
        let hide = secret_values.clone();
        let sink: LineSink = Arc::new(move |line: &str| {
            let cleaned = ring.lock().push(line, &hide);
            if super::log::is_failure_line(&cleaned) {
                // Why a turn failed (a code, a short reason, numbers) is worth seeing without debug logging.
                tracing::info!(target: "potato_brain", "{cleaned}");
            } else {
                tracing::debug!(target: "potato_brain", "{cleaned}");
            }
        });
        let spec = SpawnSpec { node, script, cwd, env: brain_env };
        match self.launcher.spawn(&spec, sink) {
            Ok(child) => {
                self.brains.set_pid(bot, child.pid());
                let mut inbox = Inbox::default();
                // Paused for the daily limit: the new brain hears about it before any message it is given.
                if rt.paused_for_limit.is_some() {
                    inbox.push(protocol::pause_item(PauseReason::DailyLimit));
                }
                let mut session = Session {
                    child,
                    inbox: Inbox::default(),
                    secrets: secret_values,
                    shutdown_at: None,
                    in_flight: HashMap::new(),
                    send_index: HashMap::new(),
                };
                // A send that comes back from a dead session holds its place on the limit again, once.
                let estimate = rt.reserve_estimate();
                for item in rt.queued.drain(..) {
                    if let (Some(id), Some(conversation)) = (item.get("id").and_then(Value::as_str), item.get("conversationId").and_then(Value::as_str)) {
                        session.send_index.insert(id.to_string(), inbox.end());
                        session.in_flight.insert(id.to_string(), conversation.to_string());
                        if !rt.reserved.iter().any(|r| r.id == id) {
                            rt.reserved.push_back(Reservation { id: id.to_string(), nano: estimate });
                        }
                    }
                    inbox.push(item);
                }
                session.inbox = inbox;
                rt.session = Some(session);
                rt.provider = None;
                self.cv.notify_all();
                Ok(())
            }
            Err(e) => {
                self.brains.unregister(bot);
                tracing::warn!(%bot, error = %e, "could not start the brain");
                rt.log.lock().push(&e, &[]);
                Err(None)
            }
        }
    }

    /// The session is over (the process left or was killed): forget its token, keep what was not
    /// handed to it for the next one, and say so for what it never answered.
    fn end_session(&self, g: &mut Inner, bot: &str, asked: bool) {
        let now = self.host.now_ms();
        let Some(rt) = g.bots.get_mut(bot) else { return };
        let Some(session) = rt.session.take() else { return };
        // Whatever the dead session had in flight is not charged by it: its holds go back. A requeued send holds
        // its place again when it is handed to the next session.
        rt.reserved.retain(|r| !session.in_flight.contains_key(&r.id));
        self.brains.unregister(bot);
        let _ = &session.secrets;
        rt.typing.clear();
        rt.work.clear();
        self.calls.end_bot(self.port.lock().as_ref(), bot);
        let mut lost: Vec<(String, String)> = Vec::new();
        for (id, conversation) in &session.in_flight {
            let index = session.send_index.get(id).copied().unwrap_or(0);
            if index >= session.inbox.handed {
                // Never handed to the brain: it goes to the next one.
                if let Some(item) = session.inbox.items.get((index.saturating_sub(session.inbox.base)) as usize) {
                    rt.queued.push(item.clone());
                    if !rt.reserved.iter().any(|r| &r.id == id) {
                        rt.reserved.push_back(Reservation { id: id.clone(), nano: rt.reserve_estimate() });
                    }
                }
            } else {
                lost.push((id.clone(), conversation.clone()));
            }
        }
        // The job turn the dead brain was running is gone (unless it waits in the queue for the next brain): the job
        // starts its next turn again from its file (`job_flow.rs`).
        let queued_job_turn = matches!(&rt.job_turn, Some(JobTurn::Turn(id)) if rt.queued.iter().any(|i| i.get("id").and_then(Value::as_str) == Some(id.as_str())));
        if !queued_job_turn {
            rt.job_turn = None;
        }
        rt.job_after = None;
        if !lost.is_empty() {
            let name = self.name_of(bot);
            for (_, conversation) in &lost {
                self.conversation_note(bot, conversation, now, &format!("{name}'s brain stopped before it answered."));
            }
        }
        let actions = if rt.keeper.is_alive() { rt.keeper.exited(now, asked) } else { Vec::new() };
        // A stop that was asked for may still want a start (a new message came while it was leaving).
        let more = if asked { rt.keeper.step(now) } else { Vec::new() };
        let all: Vec<Action> = actions.into_iter().chain(more).collect();
        self.run_actions(g, bot, all);
        self.host.changed(bot);
    }

    /// The supervisor. Called every half second by the live thread; the tests call it by hand.
    pub fn tick(&self) {
        self.watch_helpers();
        let now = self.host.now_ms();
        let day = self.host.local_day(now);
        let bots: Vec<String> = self.inner.lock().bots.keys().cloned().collect();
        for bot in bots {
            let mut g = self.inner.lock();
            // A new day: yesterday's holds are not for today's limit.
            if let Some(rt) = g.bots.get_mut(&bot) {
                if rt.reserved_day.is_empty() {
                    rt.reserved_day = day.clone();
                } else if rt.reserved_day != day {
                    rt.reserved.clear();
                    rt.reserved_day = day.clone();
                }
            }
            let exited = g.bots.get_mut(&bot).and_then(|rt| rt.session.as_mut()).and_then(|s| s.child.try_wait().map(|code| (code, s.shutdown_at.is_some())));
            if let Some((code, asked)) = exited {
                if !asked {
                    if let Some(rt) = g.bots.get(&bot) {
                        rt.log.lock().push(&format!("the brain exited with code {code}"), &[]);
                    }
                }
                self.end_session(&mut g, &bot, asked);
            }
            // A brain that was asked to leave is killed after the grace.
            let overdue = g.bots.get(&bot).and_then(|rt| rt.session.as_ref()).is_some_and(|s| s.shutdown_at.is_some_and(|at| now - at >= SHUTDOWN_GRACE_MS));
            if overdue {
                if let Some(session) = g.bots.get_mut(&bot).and_then(|rt| rt.session.as_mut()) {
                    session.child.kill_tree();
                }
                self.end_session(&mut g, &bot, true);
            }
            let actions = g.bots.get_mut(&bot).map(|rt| rt.keeper.step(now)).unwrap_or_default();
            self.run_actions(&mut g, &bot, actions);
            // A new day lifts the daily pause.
            if let Some(rt) = g.bots.get_mut(&bot) {
                if rt.paused_for_limit.as_deref().is_some_and(|d| d != day) {
                    rt.paused_for_limit = None;
                    if let Some(session) = rt.session.as_mut() {
                        session.inbox.push(protocol::resume_item());
                        self.cv.notify_all();
                    }
                }
            }
        }
        self.job_tick(now);
    }

    /// A raised limit lifts the pause at once.
    fn recheck_limits(&self) {
        let Ok(settings) = store::load_settings(&self.root) else { return };
        let day = self.host.local_day(self.host.now_ms());
        let mut g = self.inner.lock();
        let bots: Vec<String> = g.bots.keys().cloned().collect();
        for bot in bots {
            let used = cost::day_of(&self.root, &day, &bot).map(|d| d.cost_nano()).unwrap_or(0);
            if let Some(rt) = g.bots.get_mut(&bot) {
                if rt.paused_for_limit.is_some() && cost::budget_with(used, settings.daily_limit()) != Budget::Over {
                    rt.paused_for_limit = None;
                    if let Some(session) = rt.session.as_mut() {
                        session.inbox.push(protocol::resume_item());
                        self.cv.notify_all();
                    }
                }
            }
        }
    }

    // ---- the brain's side ----------------------------------------------------------------------

    /// `GET /potato/brain/inbox`: items from position `at`, waiting up to `wait_ms` when there are none.
    pub fn inbox_poll(&self, bot: &str, at: u64, wait_ms: u64) -> Value {
        let wait = Duration::from_millis(wait_ms.min(protocol::MAX_WAIT_MS));
        let deadline = std::time::Instant::now() + wait;
        let mut g = self.inner.lock();
        loop {
            let Some(session) = g.bots.get_mut(bot).and_then(|rt| rt.session.as_mut()) else {
                return serde_json::json!({ "v": 1, (wire_names::POLL_POSITION): at, "items": [] });
            };
            let (items, next) = session.inbox.since(at);
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if !items.is_empty() || left.is_zero() {
                return serde_json::json!({ "v": 1, (wire_names::POLL_POSITION): next, "items": items });
            }
            self.cv.wait_for(&mut g, left);
        }
    }

    /// `POST /potato/brain/events`.
    pub fn ingest(&self, bot: &str, events: Vec<Event>) {
        let now = self.host.now_ms();
        let mut changed = false;
        for event in events {
            changed |= self.on_event(bot, event, now);
        }
        if changed {
            self.host.changed(bot);
        }
    }

    /// A bot may post into its own DM, and into a group only when it is a member of that group. Any other
    /// conversation, or a group the bot is not in, is refused.
    fn own_conversation(&self, bot: &str, conversation: &str) -> bool {
        if let Some(room) = store::app_room_of(conversation) {
            return rooms::store::load_rooms(&self.root).map(|rooms| rooms.iter().any(|r| r.id == room && r.bot_ids.iter().any(|b| b == bot))).unwrap_or(false);
        }
        store::app_bot_of(conversation) == Some(bot)
    }

    fn on_event(&self, bot: &str, event: Event, now: i64) -> bool {
        // Only a bot that is known and running may post.
        if self.require_bot(bot).is_err() {
            return false;
        }
        match event {
            Event::Hello { protocol: version, provider, .. } => {
                let mut g = self.inner.lock();
                let Some(rt) = g.bots.get_mut(bot) else { return false };
                if version == protocol::V {
                    rt.keeper.hello(now);
                    rt.provider = Some(provider);
                }
                true
            }
            Event::Typing { conversation_id, on } => {
                if !self.own_conversation(bot, &conversation_id) {
                    return false;
                }
                let mut g = self.inner.lock();
                let Some(rt) = g.bots.get_mut(bot) else { return false };
                rt.keeper.touch(now);
                if on {
                    rt.work.insert(conversation_id.clone(), super::work::WorkRun::new(now));
                    rt.typing.insert(conversation_id, now);
                } else {
                    // The list stays, folded, after the turn (a failed one keeps its X) until the next turn starts.
                    if let Some(w) = rt.work.get_mut(&conversation_id).filter(|w| !w.failed) {
                        w.finish(now);
                    }
                    rt.typing.remove(&conversation_id);
                }
                true
            }
            Event::Progress { conversation_id, id, tool, phase, ok, log, info } => {
                if !self.own_conversation(bot, &conversation_id) {
                    return false;
                }
                let mut g = self.inner.lock();
                let Some(rt) = g.bots.get_mut(bot) else { return false };
                rt.keeper.touch(now);
                rt.typing.insert(conversation_id.clone(), now);
                rt.work.entry(conversation_id).or_insert_with(|| super::work::WorkRun::new(now)).apply_with(&id, &tool, phase, ok, log.as_deref(), &info, now);
                true
            }
            Event::Reply { conversation_id, in_reply_to, segments, kaomoji, note, .. } => {
                if !self.own_conversation(bot, &conversation_id) {
                    return false;
                }
                // A group: the reply is a room line, passed on to the other bots (not a DM message).
                if let Some(room) = store::app_room_of(&conversation_id) {
                    return self.on_room_reply(bot, room, &in_reply_to, segments, kaomoji, note, now);
                }
                // A note (message_user) is still part of a running turn: the screen, the in-flight turn and the
                // pending routine stay as they are, and the final reply still ends them. The owner still gets it.
                let name = self.name_of(bot);
                let mut was_routine_run = false;
                let mood = if note {
                    let mut g = self.inner.lock();
                    let Some(rt) = g.bots.get_mut(bot) else { return false };
                    rt.keeper.touch(now);
                    rt.mood.as_ref().map(|m| m.mood.clone())
                } else {
                    // The turn is over: the bot lets go of the screen (the person's hold is not touched).
                    self.calls.end_run(self.port.lock().as_ref(), bot, &conversation_id);
                    let mut g = self.inner.lock();
                    let Some(rt) = g.bots.get_mut(bot) else { return false };
                    rt.keeper.touch(now);
                    rt.last_error = None;
                    if let Some(session) = rt.session.as_mut() {
                        session.in_flight.remove(&in_reply_to);
                    }
                    rt.queued_ids.remove(&in_reply_to);
                    was_routine_run = super::routines::clear_pending(&self.root, &in_reply_to);
                    rt.mood.as_ref().map(|m| m.mood.clone())
                };
                if !note {
                    jobs::unqueue(&self.root, bot, &in_reply_to);
                }
                let said = crate::companion::view::scrub(&segments.join(" "), None);
                let last = segments.len().saturating_sub(1);
                let mut last_id = None;
                for (i, mut segment) in segments.into_iter().enumerate() {
                    if i == last {
                        if let Some(face) = kaomoji.as_deref().filter(|k| !k.trim().is_empty()) {
                            segment.push(' ');
                            segment.push_str(face.trim());
                        }
                    }
                    let mut message = Message::new(now, FromKind::Bot, Some(&name), MsgKind::Text, &segment);
                    if i == last {
                        message.mood = mood.clone();
                    }
                    match self.save(bot, &conversation_id, message) {
                        Ok(saved) if i == last => last_id = Some(saved.id),
                        Ok(_) => {}
                        Err(e) => tracing::debug!(%e, "could not save a reply"),
                    }
                }
                if was_routine_run {
                    // The routine's history line links to the reply in the thread.
                    super::routines::record_reply(&self.root, &in_reply_to, last_id.as_deref());
                }
                if !note {
                    if let Some(rt) = self.inner.lock().bots.get_mut(bot) {
                        rt.last_reply_id = last_id;
                    }
                }
                if !note {
                    self.job_turn_ended(bot, Some(&in_reply_to), false, now);
                }
                if !note {
                    self.relay_peer_reply(bot, &in_reply_to, &said, now);
                }
                self.host.reply_pushed(bot, &name, &said, super::notify::pushes_replies(&self.root, bot));
                true
            }
            Event::Pass { conversation_id, in_reply_to, reason } => {
                // Only a group has a pass: the bot read a line and chose not to answer.
                if !self.own_conversation(bot, &conversation_id) || store::app_room_of(&conversation_id).is_none() {
                    return false;
                }
                if reason == "failed" {
                    if let Some(w) = self.inner.lock().bots.get_mut(bot).and_then(|rt| rt.work.get_mut(&conversation_id)) {
                        w.fail(now);
                    }
                }
                self.on_room_pass(bot, &conversation_id, &in_reply_to, now)
            }
            Event::Mood { mood, intensity, energy, hunger } => {
                let mut g = self.inner.lock();
                let Some(rt) = g.bots.get_mut(bot) else { return false };
                rt.keeper.touch(now);
                rt.mood = Some(MoodNow { mood, intensity, energy, hunger });
                true
            }
            Event::Usage { model, prompt_tokens, completion_tokens, cache_hit_tokens, cache_miss_tokens } => {
                self.on_usage(bot, now, &model, cost::Tokens { prompt: prompt_tokens, completion: completion_tokens, cache_hit: cache_hit_tokens, cache_miss: cache_miss_tokens });
                // The turn that just ended is paid for: now the job can decide whether to go on.
                self.job_after_turn(bot, now);
                true
            }
            Event::Picked { conversation_id, ids } => {
                if !self.own_conversation(bot, &conversation_id) {
                    return false;
                }
                {
                    let mut g = self.inner.lock();
                    let Some(rt) = g.bots.get_mut(bot) else { return false };
                    rt.keeper.touch(now);
                    for id in &ids {
                        rt.queued_ids.remove(id);
                        // The message went into a turn that is already running (or starts one): that turn pays for it,
                        // so the hold for this send goes back.
                        rt.reserved.retain(|r| &r.id != id);
                        if let Some(session) = rt.session.as_mut() {
                            session.in_flight.remove(id);
                        }
                    }
                }
                for id in &ids {
                    jobs::unqueue(&self.root, bot, id);
                }
                self.host.changed(bot);
                true
            }
            Event::HelperRequest { request_id, conversation_id, task, project } => self.on_helper_request(bot, now, request_id, conversation_id, task, project),
            Event::SandboxCall { call_id, conversation_id, tool, args, image } => self.on_sandbox_call(bot, now, call_id, conversation_id, tool, args, image),
            Event::Error { code, conversation_id, in_reply_to, steps } => {
                // A turn that already did work is not started again on another brain: the files are on the computer.
                if !job_stopped(code, steps) && self.fall_back(bot, now, code, in_reply_to.as_deref()) {
                    let name = self.name_of(bot);
                    let why = if code == ErrorCode::ProviderLimit { "model is busy or out of balance" } else { "model is not answering" };
                    let line = format!("{name}'s {why}. Trying your next fallback.");
                    let conversation = conversation_id.filter(|c| self.own_conversation(bot, c)).unwrap_or_else(|| store::app_conversation(bot));
                    self.conversation_note(bot, &conversation, now, &line);
                    return true;
                }
                let name = self.name_of(bot);
                let was_routine_run = in_reply_to.as_deref().is_some_and(|id| super::routines::clear_pending(&self.root, id));
                // Paused for the daily limit: the message is refused with the same note the pause gave.
                let paused = self.inner.lock().bots.get(bot).is_some_and(|rt| rt.paused_for_limit.is_some());
                let line = match code {
                    ErrorCode::ProviderLimit if paused => {
                        let limit = store::load_settings(&self.root).map(|s| cost::dollars(s.daily_limit_cents)).unwrap_or_default();
                        format!("{name} has used today's limit ({limit}). Raise it in Settings or wait until tomorrow.")
                    }
                    ErrorCode::ProviderAuth => format!("{name} could not use the model key. Check it in Settings."),
                    // A job goes on by itself after a failed turn: it must not tell the person to say continue.
                    _ if job_stopped(code, steps) && jobs::load(&self.root, bot).is_some_and(|j| j.is_running()) => {
                        format!("{name} hit an error in the middle of the job. The job goes on by itself.")
                    }
                    _ if job_stopped(code, steps) => stopped_job_note(&name, steps),
                    ErrorCode::ProviderLimit => "The model is out of balance or too busy. Try again later.".to_string(),
                    ErrorCode::ProviderDown => "The model is not answering right now. Try again in a minute.".to_string(),
                    ErrorCode::Network => format!("{name} could not reach the internet."),
                    ErrorCode::PersonaTooBig => format!("{name}'s persona text is too long."),
                    ErrorCode::SurfaceAuth | ErrorCode::Internal => format!("Something went wrong inside {name}'s brain."),
                };
                {
                    let mut g = self.inner.lock();
                    let Some(rt) = g.bots.get_mut(bot) else { return false };
                    rt.keeper.touch(now);
                    rt.last_error = Some(code);
                    rt.last_steps = steps;
                    // The step the turn stopped at gets the X.
                    let own = conversation_id.clone().filter(|c| c.starts_with("app:")).unwrap_or_else(|| store::app_conversation(bot));
                    if let Some(w) = rt.work.get_mut(&own) {
                        w.fail(now);
                    }
                    if let (Some(session), Some(id)) = (rt.session.as_mut(), in_reply_to.as_deref()) {
                        session.in_flight.remove(id);
                    }
                    // A failed send is never charged: its hold on the limit goes back.
                    if let Some(id) = in_reply_to.as_deref() {
                        rt.reserved.retain(|r| r.id != id);
                        rt.queued_ids.remove(id);
                    }
                }
                if let Some(id) = in_reply_to.as_deref() {
                    jobs::unqueue(&self.root, bot, id);
                }
                if let (true, Some(id)) = (was_routine_run, in_reply_to.as_deref()) {
                    super::routines::record_failure(&self.root, id, &line);
                }
                let conversation = conversation_id.filter(|c| self.own_conversation(bot, c)).unwrap_or_else(|| store::app_conversation(bot));
                self.conversation_note(bot, &conversation, now, &line);
                // A failed turn of a job is a turn without progress, not the end of the job.
                if store::app_room_of(&conversation).is_none() {
                    self.job_turn_ended(bot, in_reply_to.as_deref(), true, now);
                    self.job_after_turn(bot, now);
                }
                true
            }
            Event::Unknown(_) => false,
        }
    }

    fn on_usage(&self, bot: &str, now: i64, model: &str, tokens: cost::Tokens) {
        let Ok(settings) = store::load_settings(&self.root) else { return };
        let day = self.host.local_day(now);
        let surface = self.inner.lock().bots.get(bot).map(|rt| rt.last_surface.clone()).unwrap_or_else(|| "app".into());
        let charged = match cost::charge_with(&self.root, &day, bot, &surface, model, tokens, settings.daily_limit()) {
            Ok(charged) => charged,
            Err(e) => {
                tracing::warn!(%bot, error = %e, "could not save usage");
                return;
            }
        };
        // A running job is charged what the bot spends: its own turns and the person's messages in between.
        jobs::add_spend(&self.root, bot, charged.cost_nano);
        if charged.cost_nano > 0 && jobs::load(&self.root, bot).is_some_and(|j| matches!(j.status, jobs::Status::Running | jobs::Status::Waiting)) {
            self.host.changed(bot);
        }
        // The turn's real cost replaces its send's hold (the oldest one); the average for the next estimate follows it.
        let mut restart = false;
        let reply = self.inner.lock().bots.get_mut(bot).and_then(|rt| {
            rt.reserved.pop_front();
            // A change made in this turn that the brain reads at start (instructions, persona, model): it starts again now.
            restart = rt.restart_after_turn && rt.reserved.is_empty();
            if restart {
                rt.restart_after_turn = false;
            }
            rt.recent_turn_nano.push_back(charged.cost_nano);
            while rt.recent_turn_nano.len() > RECENT_TURNS {
                rt.recent_turn_nano.pop_front();
            }
            rt.last_reply_id.take()
        });
        if restart {
            self.stop_bot(bot);
        }
        // The turn's reply (saved just before this usage) carries the turn's cost: an estimate, shown as "about".
        if let Some(id) = reply {
            if let Err(e) = reply_cost::record(&self.root, &id, charged.cost_nano) {
                tracing::debug!(%e, "could not save a reply's cost");
            }
        }
        let name = self.name_of(bot);
        let limit = cost::dollars(settings.daily_limit_cents);
        if charged.limit_now {
            self.note(bot, &format!("{name} has used today's limit ({limit}). Raise it in Settings or wait until tomorrow."));
            let mut g = self.inner.lock();
            if let Some(rt) = g.bots.get_mut(bot) {
                rt.paused_for_limit = Some(day);
                if let Some(session) = rt.session.as_mut() {
                    session.inbox.push(protocol::pause_item(PauseReason::DailyLimit));
                    self.cv.notify_all();
                }
            }
        } else if charged.warn_now {
            self.note(bot, &format!("{name} has used 80% of today's limit ({limit})."));
        }
    }

    /// A provider error on a message that has a fallback brain left: the brain that failed is stopped, the message it
    /// failed on goes back to the queue, and the next brain starts for it (`fallback.rs` decides the brain). Returns
    /// true when it switched; false leaves the error to be shown as before.
    fn fall_back(&self, bot: &str, now: i64, code: ErrorCode, in_reply_to: Option<&str>) -> bool {
        let Some(id) = in_reply_to else { return false };
        let Ok(settings) = store::load_settings(&self.root) else { return false };
        // The chain starts from the brain this bot runs on: its own choice when it has one, else the Settings brain.
        let settings = self.settings_for(bot, &settings);
        let chain = fallback::chain(settings.brain, &settings.fallbacks, self.has_key());
        let mut g = self.inner.lock();
        let Some(rt) = g.bots.get_mut(bot) else { return false };
        // Paused for the daily limit: a fallback brain would answer past the cap, so the message is refused instead.
        if rt.paused_for_limit.is_some() && code == ErrorCode::ProviderLimit {
            return false;
        }
        let step = rt.chain_step.min(chain.len() - 1);
        let Some(next) = fallback::next_step(chain.len(), step, code) else { return false };
        let Some(session) = rt.session.as_mut() else { return false };
        if session.shutdown_at.is_some() || !session.in_flight.contains_key(id) {
            return false;
        }
        // The message is needed again: it must still be in the inbox this brain was given.
        let Some(&index) = session.send_index.get(id) else { return false };
        let Some(offset) = index.checked_sub(session.inbox.base).map(|o| o as usize) else { return false };
        let Some(item) = session.inbox.items.get(offset).cloned() else { return false };
        session.in_flight.remove(id);
        session.send_index.remove(id);
        session.child.kill_tree();
        // The hold goes and comes back once: the message waits for the next brain, so it keeps its place on the limit.
        rt.reserved.retain(|r| r.id != id);
        rt.queued.push(item);
        let estimate = rt.reserve_estimate();
        rt.reserved.push_back(Reservation { id: id.to_string(), nano: estimate });
        rt.chain_step = next;
        rt.last_error = None;
        rt.keeper.touch(now);
        // Asked to leave (not a crash): the keeper still wants a run, so the next brain starts at once.
        self.end_session(&mut g, bot, true);
        true
    }

    /// Settings > Potato Bot > Usage: the last 30 days per bot and in total, read only.
    pub fn usage_report(&self) -> Result<UsageReport> {
        let settings = store::load_settings(&self.root)?;
        let today = self.host.local_day(self.host.now_ms());
        let bots: Vec<(String, String)> = store::load_bots(&self.root)?.into_iter().map(|b| (b.id.clone(), self.name_of(&b.id))).collect();
        usage::build(&self.root, &today, &bots, settings.daily_limit_cents, settings.daily_limit_off, settings.sandbox_actions_per_day).map_err(|e| err("POTATO_STORE_INVALID", e))
    }

    /// Runs a check for a wait or a watching routine (`checks.rs`). The same switches as the bot's own tools: the app and the
    /// Sandbox switch are on, a sandbox is set up, and the bot's own Computer switch (a command) or Web switch (an address) is on.
    /// A refusal ends the wait or the watch; it is never turned into a card.
    pub(super) fn run_check(&self, bot: &str, check: &super::checks::Check) -> std::result::Result<sandbox_calls::CheckRun, sandbox_calls::CallError> {
        use sandbox_calls::CallError;
        if !self.brains.enabled() {
            return Err(CallError::new("POTATO_OFF", "Potato Bot is switched off."));
        }
        if !self.sandbox_on.load(Ordering::SeqCst) {
            return Err(CallError::new("SANDBOX_OFF", "The bot's computer is switched off."));
        }
        let tools = bot_settings::load(&self.root, bot)
            .map_err(|_| CallError::new("TOOL_OFF", "The bot's settings could not be read, so the check was not run."))?
            .tools;
        let tool = if check.kind == super::checks::CheckKind::Url { "web_fetch" } else { "run_terminal" };
        if !bot_settings::sandbox_tool_on(&tools, tool) {
            return Err(CallError::new("TOOL_OFF", "The tool this check needs is switched off for this bot."));
        }
        let port = self.port.lock().clone().ok_or_else(|| CallError::new("SANDBOX_NOT_SET_UP", "The bot's computer is not set up."))?;
        self.calls.run_check(&port, bot, check)
    }

    /// A `sandbox_call`: checked and run on another thread, answered with a `sandbox_result` in the inbox. The
    /// trust of the turn is the one Puppet Master recorded when it sent the message, never anything the brain says.
    #[allow(clippy::too_many_arguments)]
    fn on_sandbox_call(&self, bot: &str, now: i64, call_id: String, conversation_id: String, tool: String, args: Value, image: bool) -> bool {
        if !self.own_conversation(bot, &conversation_id) {
            return false;
        }
        let trust = {
            let mut g = self.inner.lock();
            let Some(rt) = g.bots.get_mut(bot) else { return false };
            rt.keeper.touch(now);
            rt.turn_trust.get(&conversation_id).copied().unwrap_or(Trust::Others)
        };
        let Some(me) = self.me.upgrade() else { return false };
        let ctx = sandbox_calls::Context { bot: bot.to_string(), conversation: conversation_id, trust, want_image: image };
        let spawn = self.spawner.lock().clone();
        spawn(Box::new(move || me.run_sandbox_call(ctx, call_id, tool, args)));
        false
    }

    fn run_sandbox_call(&self, ctx: sandbox_calls::Context, call_id: String, tool: String, args: Value) {
        // A tool that brings in text from outside the chat marks the turn: it can no longer change the bot's own settings.
        if sandbox_calls::Tool::parse(&tool).is_some_and(sandbox_calls::Tool::reads_outside) || history::is_tool(&tool) {
            if let Some(rt) = self.inner.lock().bots.get_mut(&ctx.bot) {
                rt.turn_tainted.insert(ctx.conversation.clone());
            }
        }
        // The to-do tools (`scratchpad_*`) are not the bot's computer: they need the owner's turn, no switch and no lab.
        let item = if open_work::is_tool(&tool) {
            if !ctx.trust.is_owner() {
                protocol::sandbox_result_err(&call_id, "TOOL_NOT_ALLOWED", "Only the owner can use the to-do list in this chat.", false)
            } else {
                match open_work::call(&self.root, &ctx.bot, &tool, &args) {
                    Ok(result) => protocol::sandbox_result_ok(&call_id, result),
                    Err(e) => protocol::sandbox_result_err(&call_id, e.code, &e.message, false),
                }
            }
        } else if jobs::is_tool(&tool) {
            self.job_tool_item(&ctx, &call_id, &tool, &args)
        } else if tool == super::login::TOOL {
            self.login_tool_item(&ctx, &call_id, &args)
        } else if matches!(tool.as_str(), "request_approval" | "ask_user" | mcp::CONNECTOR_TOOL) {
            self.card_tool_item(&ctx, &call_id, &tool, &args)
        } else if routines::is_tool(&tool) {
            // The schedule tools: owner only, and never from a routine's own run (a routine must not add routines).
            let from_routine = self.inner.lock().bots.get(&ctx.bot).is_some_and(|rt| rt.turn_routine.contains(&ctx.conversation));
            if !ctx.trust.is_owner() {
                protocol::sandbox_result_err(&call_id, "TOOL_NOT_ALLOWED", "Only the owner can change this bot's routines in this chat.", false)
            } else if from_routine {
                protocol::sandbox_result_err(&call_id, "TOOL_NOT_ALLOWED", "A routine cannot change routines. Only the owner can.", false)
            } else {
                match routines::call(&self.root, &ctx.bot, &tool, &args, self.host.now_ms()) {
                    Ok(result) => protocol::sandbox_result_ok(&call_id, result),
                    Err(e) => protocol::sandbox_result_err(&call_id, e.code, &e.message, false),
                }
            }
        } else if waits::is_tool(&tool) {
            self.wait_tool_item(&ctx, &call_id, &tool, &args)
        } else if chart::is_tool(&tool) {
            // render_plot: a chart in the owner's private chat. No card at any level (it reads and sends nothing).
            if !ctx.trust.is_owner() {
                protocol::sandbox_result_err(&call_id, "TOOL_NOT_ALLOWED", "Only the owner can have this bot draw a chart.", false)
            } else {
                let profile = self.host.profile(&ctx.bot);
                let who = profile.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| ctx.bot.clone());
                let home = profile.and_then(|p| p.folder);
                match chart::call(&self.root, &ctx.bot, &ctx.conversation, &who, home.as_deref(), &args, self.host.now_ms()) {
                    Ok(result) => protocol::sandbox_result_ok(&call_id, result),
                    Err(e) => protocol::sandbox_result_err(&call_id, e.code, &e.message, false),
                }
            }
        } else if tool == "hand_off" {
            self.hand_off_item(&ctx, &call_id, &args)
        } else if tool == "message_bot" {
            self.message_bot_item(&ctx, &call_id, &args)
        } else if super::self_settings::is_tool(&tool) {
            self.self_setting_item(&ctx, &call_id, &args)
        } else if history::is_tool(&tool) {
            // The owner's chat with this bot, read only: owner turns (a routine's run may read it too, a stranger's may not).
            if !ctx.trust.is_owner() {
                protocol::sandbox_result_err(&call_id, "TOOL_NOT_ALLOWED", "Only the owner can search this bot's chats.", false)
            } else {
                match history::call(&self.root, &ctx.bot, &tool, &args) {
                    Ok(result) => protocol::sandbox_result_ok(&call_id, result),
                    Err(e) => protocol::sandbox_result_err(&call_id, e.code, &e.message, false),
                }
            }
        } else if tool == mcp::ADD_TOOL && args.get("url").is_some_and(|u| !u.is_null()) {
            // A tool server by link is only ever suggested: a card the owner confirms. It never takes a secret.
            self.mcp_link_card(&ctx, &call_id, &args)
        } else if tool == mcp::ADD_TOOL {
            // A tool server: the owner's own chat only, never a routine's run, with the computer on and its switch on.
            let from_routine = self.inner.lock().bots.get(&ctx.bot).is_some_and(|rt| rt.turn_routine.contains(&ctx.conversation));
            let tools = bot_settings::load(&self.root, &ctx.bot).unwrap_or_default().tools;
            let switched_on = self.sandbox_on.load(Ordering::SeqCst) && self.brains.enabled();
            let port = self.port.lock().clone();
            if !ctx.trust.is_owner() || from_routine {
                protocol::sandbox_result_err(&call_id, "TOOL_NOT_ALLOWED", "Only the owner can add a tool server in this chat.", false)
            } else if !switched_on {
                protocol::sandbox_result_err(&call_id, "SANDBOX_OFF", "The bot's computer is switched off. Turn on Sandbox in Settings > General > Labs.", false)
            } else if !bot_settings::sandbox_tool_on(&tools, "mcp_call_tool") {
                protocol::sandbox_result_err(&call_id, "TOOL_OFF", "This tool is off for this bot. Turn it on in its panel, under Advanced.", false)
            } else {
                match port {
                    None => protocol::sandbox_result_err(&call_id, "SANDBOX_NOT_SET_UP", "The bot's computer is not set up yet. Set it up in Settings > Potato Bot.", false),
                    Some(port) => match mcp::call_add(&self.root, &ctx.bot, &args, port.sandbox.as_ref()) {
                        Ok(result) => protocol::sandbox_result_ok(&call_id, result),
                        Err(e) => protocol::sandbox_result_err(&call_id, e.code, &e.message, false),
                    },
                }
            }
        } else if !bot_settings::sandbox_tool_on(&bot_settings::load(&self.root, &ctx.bot).unwrap_or_default().tools, &tool) {
            protocol::sandbox_result_err(&call_id, "TOOL_OFF", "This tool is off for this bot. Turn it on in its panel, under Advanced.", false)
        } else {
            let gates = Gates { bot_on: self.brains.enabled(), sandbox_on: self.sandbox_on.load(Ordering::SeqCst) };
            let port = self.port.lock().clone();
            match self.calls.execute(port.as_ref(), gates, &ctx, &tool, &args) {
                Ok(result) => protocol::sandbox_result_ok(&call_id, result),
                Err(e) => protocol::sandbox_result_err(&call_id, e.code, &e.message, e.needs_you),
            }
        };
        // A brain that left meanwhile does not get it: a new brain knows nothing of the call.
        self.push_to_brain(&ctx.bot, item);
        self.keep_awake(&ctx.bot);
    }

    fn on_helper_request(&self, bot: &str, now: i64, request_id: String, conversation_id: String, task: String, project: Option<String>) -> bool {
        if !self.own_conversation(bot, &conversation_id) {
            return false;
        }
        let trust = self.inner.lock().bots.get(bot).and_then(|rt| rt.turn_trust.get(&conversation_id).copied()).unwrap_or(Trust::Others);
        let refuse = |status: HelperStatus, summary: &str| {
            let mut g = self.inner.lock();
            if let Some(session) = g.bots.get_mut(bot).and_then(|rt| rt.session.as_mut()) {
                session.inbox.push(protocol::helper_update_item(&request_id, status, summary));
                self.cv.notify_all();
            }
        };
        if !rules::tool_access(trust).helpers {
            refuse(HelperStatus::Refused, "");
            return false;
        }
        if !bot_settings::load(&self.root, bot).unwrap_or_default().tools.helpers {
            refuse(HelperStatus::Declined, "Helpers are off for this bot. Turn them on in its panel, under Advanced.");
            return false;
        }
        if self.open_card_ids(bot).len() >= MAX_OPEN_CARDS {
            refuse(HelperStatus::Declined, "There are already too many open helper cards.");
            return false;
        }
        let mut card = Message::new(now, FromKind::Bot, Some(&self.name_of(bot)), MsgKind::HelperCard, &task);
        card.request_id = Some(request_id);
        card.project = self.card_folder(project.as_deref().and_then(project_name));
        self.save(bot, &conversation_id, card).is_ok()
    }
}

/// A start that failed for a reason that is not the person's to fix: a crash.
fn fail<E>(_: E) -> Option<Blocked> {
    None
}

/// What a bot asked for as a folder, reduced to a name: the last part of whatever it wrote, with no
/// slash and no dots-only part. A path the model wrote is never used, only matched by name later.
fn project_name(raw: &str) -> Option<String> {
    let name = raw.split(['/', '\\']).map(str::trim).rfind(|part| !part.is_empty())?;
    if name.chars().all(|c| c == '.') {
        return None;
    }
    Some(store::clean_text(name, None).chars().take(protocol::MAX_PROJECT).collect())
}
