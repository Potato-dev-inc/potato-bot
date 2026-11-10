//! The running Potato Bot: the one `Service` of the app, the `Host` that connects it to Puppet Master, and
//! the supervisor thread. Everything else in `potato/` is testable without this file.
//!
//! - `start(app)` once at startup (`lib.rs`): installs the service and starts the `potato-keeper` thread.
//!   No brain starts here: a bot's brain starts lazily on its first message.
//! - `apply_settings(public)` is called whenever the public settings change: the Labs switch
//!   `potato_bot_enabled` is the kill switch (off kills every brain, the brain routes answer 404, helper
//!   cards are withdrawn).
//! - `set_bridge_port(port)` is told by the bridge when it is listening; the brain gets that address.
//! - `stop_all(grace)` is for the panic button and for the app's exit.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::Value;
use tauri::AppHandle;

use super::auth;
use super::env::HostEnv;
use super::helpers::{CompanionAgent, Folder, Hands, RunState, Started};
use super::protocol::HelperStatus;
use super::sandbox_calls::Port;
use super::service::{BotProfile, Host, Service};
use super::sidecar::RealLauncher;
use crate::agent_runs::AgentRunRequest;
use crate::companion::broad::Places;

pub const LABS_KEY: &str = "potato_bot_enabled";
/// The Sandbox Labs switch. It does nothing unless Potato Bot is on too.
pub const SANDBOX_LABS_KEY: &str = "potato_sandbox_enabled";
/// Lend an App (Labs): the person lends an open Windows app to their bot. Off by default.
pub const LEND_LABS_KEY: &str = "potato_lend_app_enabled";
/// Bot groups. Does nothing unless Potato Bot is on too. Off by default.
#[cfg_attr(not(test), allow(dead_code))] // read by the group screen (G9)
pub const GROUPS_LABS_KEY: &str = "potato_groups_enabled";
pub const CHANGED_EVENT: &str = "potato://changed";
/// A group changed (a line, a rename, a new or removed group). Payload `{ roomId }`.
pub const ROOM_CHANGED_EVENT: &str = "potato://room-changed";

static SERVICE: OnceLock<Arc<Service>> = OnceLock::new();
static BRIDGE_PORT: AtomicU16 = AtomicU16::new(0);
/// The Sandbox Labs switch and the sandbox, kept here until the service exists (settings are applied once at
/// startup, possibly before it).
static SANDBOX_LABS: AtomicBool = AtomicBool::new(false);
static LEND_LABS: AtomicBool = AtomicBool::new(false);
static SANDBOX_PORT: Mutex<Option<Port>> = Mutex::new(None);

pub fn service() -> Option<Arc<Service>> {
    SERVICE.get().cloned()
}

/// The address of the bridge on this computer (`http://127.0.0.1:<port>`), once it is listening.
pub fn bridge_base() -> Option<String> {
    match BRIDGE_PORT.load(Ordering::SeqCst) {
        0 => None,
        port => Some(format!("http://127.0.0.1:{port}")),
    }
}

pub fn set_bridge_port(port: u16) {
    BRIDGE_PORT.store(port, Ordering::SeqCst);
}

/// The Labs switch in a settings value: on only when it is exactly `true`.
pub fn labs_on(public: &Value) -> bool {
    public.get(LABS_KEY).and_then(Value::as_bool).unwrap_or(false)
}

/// The Sandbox Labs switch in a settings value: on only when it is exactly `true`.
/// Whether Lend an App is on right now (the last applied settings).
pub fn lend_labs_on() -> bool {
    LEND_LABS.load(Ordering::SeqCst)
}

/// The Lend an App switch in a settings value: on only when it is exactly `true`.
pub fn lend_labs_in(public: &Value) -> bool {
    public.get(LEND_LABS_KEY).and_then(Value::as_bool).unwrap_or(false)
}

pub fn sandbox_labs_on(public: &Value) -> bool {
    public.get(SANDBOX_LABS_KEY).and_then(Value::as_bool).unwrap_or(false)
}

/// The bot groups switch in a settings value: on only when it is exactly `true`, and Potato Bot is on too.
pub fn groups_labs_on(public: &Value) -> bool {
    labs_on(public) && public.get(GROUPS_LABS_KEY).and_then(Value::as_bool).unwrap_or(false)
}

/// Wires the sandbox (with the screen lease it shares with the Computer panel) to the bots' tools. Until it is
/// called no brain is told it has a computer. Only `sandbox_host::sync` calls it: it makes the port from the one
/// sandbox and the screen hub's one lease, and calls it with `None` when the Labs switch is off or nothing is set up.
pub fn install_sandbox(port: Option<Port>) {
    *SANDBOX_PORT.lock() = port.clone();
    if let Some(service) = service() {
        service.set_sandbox_port(port);
    }
}

/// The sandbox the bots' tools use right now, if one is wired in (tests look at what the host wired).
#[cfg(test)]
pub fn sandbox_port() -> Option<Port> {
    SANDBOX_PORT.lock().clone()
}

/// Called with the public settings whenever they change (and once at startup, from the saved file).
pub fn apply_settings(public: &Value) {
    // The Computer (the sandbox screen) follows both switches.
    super::sandbox::screen_live::apply_settings(public);
    let sandbox = sandbox_labs_on(public);
    SANDBOX_LABS.store(sandbox, Ordering::SeqCst);
    LEND_LABS.store(lend_labs_in(public), Ordering::SeqCst);
    if let Some(service) = service() {
        service.set_sandbox_enabled(sandbox);
    }
    super::sandbox_host::apply_settings(public);
    let on = labs_on(public);
    match service() {
        Some(service) => service.set_enabled(on),
        None => auth::brains().set_enabled(on),
    }
}

/// Stops every brain (the panic button, the app's exit).
pub fn stop_all(grace: Duration) {
    if let Some(service) = service() {
        service.stop_all(grace);
    }
    // The bot's small Linux computer leaves with the app too.
    super::sandbox_host::stop_on_exit();
}

/// "Stop everything" (panic button): pauses every bot, holds every routine and puts Fish's computer to sleep.
/// The hold is saved before anything is stopped, so a routine due in the meantime is recorded, not run.
pub fn pause_all(grace: Duration) -> super::hold::Hold {
    let asleep = super::sandbox_host::sleep();
    let now = chrono::Utc::now().timestamp_millis();
    super::hold::pause(&root(), service().as_deref(), asleep, grace, now)
}

pub fn root() -> PathBuf {
    crate::app_paths::app_data_dir().join("potato")
}

/// True when `pane_id` is the companion pane of a Potato Bot (it talks as that bot). The companion's
/// agent gets the flags its TUI needs to reach the bot's own computer (see `TuiProfile::wsl_extra_args`).
pub fn is_bot_companion_pane(pane_id: &str) -> bool {
    let Some(bot) = pane_id.strip_prefix("companion-") else { return false };
    super::store::load_bots(&root()).is_ok_and(|bots| bots.iter().any(|b| b.id == bot))
}

/// True when `folder` is the home folder of a Potato Bot's companion. A chat session (ACP) started in
/// that folder belongs to the bot, so its agent may need the bot's own computer (see `acp_full_access`).
pub fn is_bot_home(folder: &std::path::Path) -> bool {
    let Ok(bots) = super::store::load_bots(&root()) else { return false };
    let Ok(companions) = crate::companion::store::load(&crate::app_paths::app_data_dir()) else { return false };
    let real = |path: &std::path::Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let wanted = real(folder);
    companions.iter().any(|c| bots.iter().any(|b| b.id == c.id) && real(std::path::Path::new(&c.folder)) == wanted)
}

/// The bot's own computer is a WSL distro, and Codex's Windows sandbox cannot start WSL
/// (`Wsl/Service/CreateInstance/E_ACCESSDENIED`). While the Sandbox switch is on, the bot's own agent
/// (TUI or chat) therefore runs without Codex's sandbox. Other panes are untouched.
pub fn wsl_needs_full_access() -> bool {
    super::sandbox_host::enabled()
}

struct LiveHost {
    app: Option<AppHandle>,
}

impl Host for LiveHost {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
    }

    fn local_day(&self, ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms)
            .map(|utc| utc.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "1970-01-01".to_string())
    }

    fn bridge_url(&self) -> Option<String> {
        match BRIDGE_PORT.load(Ordering::SeqCst) {
            0 => None,
            port => Some(format!("http://127.0.0.1:{port}")),
        }
    }

    fn profile(&self, bot: &str) -> Option<BotProfile> {
        let companions = crate::companion::store::load(&crate::app_paths::app_data_dir()).ok()?;
        companions.into_iter().find(|c| c.id == bot).map(|c| BotProfile { name: c.name, folder: Some(c.folder) })
    }

    fn host_env(&self) -> HostEnv {
        HostEnv::from_process(crate::shell_env::path_for_spawn())
    }

    fn rename(&self, bot: &str, name: &str) -> Result<(), String> {
        let companions = crate::companion::live::service().ok_or_else(|| "Companions are not ready yet. Try again in a moment.".to_string())?;
        companions
            .update(bot, crate::companion::manage::Patch { name: Some(name.to_string()), ..Default::default() })
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn secret(&self, name: &str) -> Option<String> {
        // A stand-in brain (e2e builds only) needs no key: the page must not ask for one, and the person's real
        // key in the credential store is neither read nor sent to it.
        if super::sidecar::fake_brain_active() {
            return (name == super::service::SECRET_DEEPSEEK).then(|| "e2e-stand-in-key".to_string());
        }
        crate::secrets::read_secret(name)
    }

    fn delete_secret(&self, name: &str) {
        // Only an agent's own secret names are ever removed this way.
        if super::custom_agents::is_agent_secret_name(name) {
            let _ = crate::secrets::SecretBackend::delete(crate::secrets::backend(), name);
        }
    }

    fn go_key(&self) -> Option<String> {
        if super::sidecar::fake_brain_active() {
            return None;
        }
        crate::opencode::go_key::read_active()
    }

    fn resources(&self) -> Option<PathBuf> {
        // A debug build runs from the checkout, where `npm run dev` has just made sure the generated folder matches
        // the sources (scripts/ensure-tauri-resources.mjs). The copy cargo made under target/debug/resources is only
        // refreshed when cargo reruns the build script, so it can be an older brain: the checkout's folder wins.
        #[cfg(all(not(test), debug_assertions))]
        {
            let staged = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join("potato-brain");
            if staged.join("brain.mjs").is_file() {
                return Some(staged);
            }
        }
        #[cfg(not(test))]
        if let Some(dir) = self.app.as_ref().and_then(|app| super::resource_dirs::installed(app, "potato-brain", "brain.mjs")) {
            return Some(dir);
        }
        // A dev build has no installed resource folder: the generated one in the source tree. A release build never
        // looks there.
        #[cfg(debug_assertions)]
        {
            let staged = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join("potato-brain");
            return staged.is_dir().then_some(staged);
        }
        #[cfg(not(debug_assertions))]
        None
    }

    fn reply_pushed(&self, bot: &str, name: &str, text: &str, on: bool) {
        crate::notifications::bot_reply(bot, name, text, on);
    }

    fn changed(&self, bot: &str) {
        // Phones waiting on the long poll wake up.
        super::phone::live::changes().bump();
        // Not in unit tests: emitting through the app handle links the windowing runtime into the lib test exe.
        #[cfg(not(test))]
        if let Some(app) = &self.app {
            use tauri::Emitter;
            let _ = app.emit(CHANGED_EVENT, serde_json::json!({ "botId": bot }));
        }
        #[cfg(test)]
        let _ = bot;
    }

    fn room_changed(&self, room_id: &str) {
        super::phone::live::changes().bump();
        #[cfg(not(test))]
        if let Some(app) = &self.app {
            use tauri::Emitter;
            let _ = app.emit(ROOM_CHANGED_EVENT, serde_json::json!({ "roomId": room_id }));
        }
        #[cfg(test)]
        let _ = room_id;
    }
}

/// The real hands of the helpers: the companion's agent, the person's recent projects, `run_agent`, the
/// operation store and the model maker's address.
struct LiveHands {
    app: AppHandle,
    registry: Arc<Mutex<crate::pty::PaneRegistry>>,
}

/// Recent projects the engine looks at, most recent first.
const RECENT_LIMIT: usize = 40;

impl Hands for LiveHands {
    fn agent_of(&self, bot: &str) -> Option<CompanionAgent> {
        let companions = crate::companion::store::load(&crate::app_paths::app_data_dir()).ok()?;
        let c = companions.into_iter().find(|c| c.id == bot)?;
        Some(CompanionAgent {
            label: crate::tui::descriptors()
                .into_iter()
                .find(|d| d.id == c.agent_type)
                .map(|d| d.label)
                .unwrap_or_else(|| c.agent_type.clone()),
            agent_type: c.agent_type,
            pane_id: crate::companion::service::pane_id_of(&c.id),
            home: Some(c.folder),
        })
    }

    fn recent_folders(&self) -> Vec<Folder> {
        let mut others = crate::operations::indexed_project_paths();
        others.push(crate::standby::default_project(&self.registry));
        others.truncate(RECENT_LIMIT);
        crate::worktrees::folder::recent_light(&crate::app_paths::app_data_dir(), &others, RECENT_LIMIT)
            .into_iter()
            .filter(|p| p.state != "missing")
            .map(|p| Folder { name: p.name, path: p.path })
            .collect()
    }

    fn places(&self) -> Places {
        Places::from_env()
    }

    fn start(&self, request: AgentRunRequest) -> Result<Started, String> {
        let registry = self.registry.clone();
        let app = self.app.clone();
        let view = crate::agent_runs::run_agent(request, registry, app, None).map_err(|e| e.message)?;
        if let Some(pane_id) = view.pane_id.as_deref() {
            // A helper is the bot's own work, not one of the person's agents: the main Inbox, Sidebar and
            // palette leave it out. The bot's thread and Open work are where it shows up.
            self.registry.lock().set_lane(pane_id, super::POTATO_PANE_LANE);
        }
        Ok(Started { operation_id: view.operation_id, pane_id: view.pane_id })
    }

    fn poll(&self, project: &str, operation_id: &str) -> RunState {
        use crate::operations::OperationStatus;
        let Ok(op) = crate::operations::get_operation(project, operation_id) else { return RunState::Gone };
        let pane_id = op.pane_id.clone();
        match op.status {
            OperationStatus::Completed => RunState::Done { status: HelperStatus::Finished, summary: op.result.unwrap_or_default(), pane_id },
            OperationStatus::Failed => RunState::Done { status: HelperStatus::Failed, summary: op.error.map(|e| e.message).unwrap_or_default(), pane_id },
            OperationStatus::Cancelled => RunState::Done { status: HelperStatus::Stopped, summary: String::new(), pane_id },
            _ => RunState::Running,
        }
    }

    fn check_key(&self, key: &str) -> Result<(), String> {
        // A stand-in brain (e2e builds only) has no model behind it, so there is nothing to ask.
        if super::sidecar::fake_brain_active() {
            return Ok(());
        }
        ask_model(key)
    }

    fn check_go_key(&self, key: &str) -> Result<(), String> {
        if super::sidecar::fake_brain_active() {
            return Ok(());
        }
        crate::opencode::go_key::check_key(key)
    }
}

/// One tiny turn: "hi", one word back at most. The key is sent to the model maker's address and nowhere else;
/// no redirect is followed, and the answer's body is never shown or logged.
fn ask_model(key: &str) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(20))
        .redirects(0)
        .build();
    let body = serde_json::json!({ "model": "deepseek-chat", "messages": [{ "role": "user", "content": "hi" }], "max_tokens": 1 });
    match agent.post("https://api.deepseek.com/chat/completions").set("Authorization", &format!("Bearer {key}")).send_json(body) {
        Ok(resp) if resp.status() == 200 => Ok(()),
        Ok(_) => Err("DeepSeek gave an answer that was not expected. Try again.".to_string()),
        Err(ureq::Error::Status(401, _)) => Err("DeepSeek did not accept that key.".to_string()),
        Err(ureq::Error::Status(402, _)) => Err("The key works, but the DeepSeek account has no balance.".to_string()),
        Err(ureq::Error::Status(429, _)) => Err("DeepSeek is busy right now. Try again in a minute.".to_string()),
        Err(ureq::Error::Status(_, _)) => Err("DeepSeek could not answer right now. Try again in a minute.".to_string()),
        Err(ureq::Error::Transport(_)) => Err("Could not reach DeepSeek. Check your internet.".to_string()),
    }
}

/// Installs the service (once) and starts its supervisor thread.
pub fn start(app: Option<AppHandle>, registry: Option<Arc<Mutex<crate::pty::PaneRegistry>>>) -> Option<Arc<Service>> {
    if let Some(app) = app.clone() {
        super::sandbox_host::set_app(app);
    }
    let hands = app.clone().zip(registry).map(|(app, registry)| Arc::new(LiveHands { app, registry }));
    let service = Service::new(root(), Arc::new(LiveHost { app }), Arc::new(RealLauncher::new()), auth::brains().clone());
    // A run that was handed over before the last quit and never answered is written as missed (never run again).
    super::routines::recover_pending(&service, chrono::Utc::now().timestamp_millis());
    if SERVICE.set(service.clone()).is_err() {
        return None;
    }
    service.set_sandbox_enabled(SANDBOX_LABS.load(Ordering::SeqCst));
    service.set_sandbox_port(SANDBOX_PORT.lock().clone());
    if let Some(hands) = hands {
        service.set_hands(hands);
    }
    let ticker = service.clone();
    let _ = std::thread::Builder::new().name("potato-keeper".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_millis(500));
        ticker.tick();
    });
    // The routines tick: every 30 s, the due routines go to their bots (a missed tick is caught up once).
    let routines = service.clone();
    let _ = std::thread::Builder::new().name("potato-routines".into()).spawn(move || loop {
        super::routines::run_due(&routines, chrono::Utc::now().timestamp_millis());
        super::trash::maybe_purge(routines.root(), chrono::Utc::now().timestamp_millis());
        // Attached files older than 7 days leave the PC (at most once a day).
        super::attachments::maybe_sweep(routines.root(), chrono::Utc::now().timestamp_millis());
        std::thread::sleep(Duration::from_secs(30));
    });
    // The waits tick: every 30 s, the due checks run (no model) and a wait that ended wakes its bot once.
    let waiter = service.clone();
    let _ = std::thread::Builder::new().name("potato-waits".into()).spawn(move || loop {
        super::waits::run_due(&waiter, chrono::Utc::now().timestamp_millis());
        std::thread::sleep(Duration::from_secs(30));
    });
    // Spaces may already be running: connect the two (a no-op while Spaces is off).
    super::space_link::connect_live();
    Some(service)
}
