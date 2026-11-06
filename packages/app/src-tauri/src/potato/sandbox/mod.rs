//! The Potato Bot sandbox: a small Linux computer (a WSL 2 distro the app makes and owns) that a bot can use
//! with a screen, a mouse and a keyboard, away from the person's own files. This folder is the Rust core
//! (piece S1); the screen links, the brain tools, the wizard and the settings page come in later pieces.
//!
//! - `wsl`: the ONE door to `wsl.exe` (name allowlist `^pm-sbx-[a-z0-9-]{1,40}$`, refused flags, UTF-16
//!   output, a fake runner can be injected)
//! - `install`: the steps, the guest files streamed in over stdin, `PROGRESS` parsing, the reset point
//! - `lifecycle`: lazy start, idle stop, readiness checks (waits for the guest's boot verdict), the secrets push
//! - `secrets`: the control token and the two VNC passwords, made by the app, in memory only
//! - `control`: the HTTP client of the guest's control service
//! - `browser`: the page tool's arguments, the checks on what the model wrote (web addresses, action lists) and its answer
//! - `lease`: who holds the screen (a mirror of the brain's `screen-lease.ts` plus expiry and run binding)
//! - `screen`: the sealed screen links (HMAC, one hour, generation, lease) and the hub the Computer panel talks to;
//!   `screen_live`: the app's one hub, its keeper thread and its Tauri events
//!
//! The `Sandbox` trait is what the rest of the app talks to. `WslSandbox` is the real one; `FakeSandbox`
//! serves a static picture and records what it was asked (tests and the `dev-fake-tui` build).
//!
//! Invariants (`guard_tests.rs` checks them): no file but `wsl.rs` builds a `wsl.exe` command or names
//! `--shutdown`; nothing here touches the person's WSL settings file; this folder never writes a file on
//! Windows (the distro and its reset points are written by `wsl.exe` itself); the secrets' values are
//! read in two files only; generic code names no agent and no chat app.

// The callers (screen links, brain tools, the wizard and settings page) land in the next pieces.
#![allow(dead_code)]

pub mod browser;
pub mod control;
pub mod detect;
pub mod files_script;
pub mod image;
pub mod install;
pub mod lease;
pub mod lifecycle;
pub mod screen;
pub mod screen_live;
pub mod manager;
pub mod secret_hub;
pub mod secrets;
pub mod wsl;

/// The guest CLI a TUI agent drives inside the sandbox (the companion brief names it).
pub const GUEST_CLI: &str = "pm-sbx-cli";

use std::sync::Arc;

use control::{Action, ControlError};
use install::{Desktop, GuestFiles, InstallEnv, InstallError, InstallOptions, InstallRecords, Layout, NoRecords, Progress, SavedInstall};
use lifecycle::{Lifecycle, LifecycleError, Phase};
use secrets::ScreenSecrets;
use wsl::{GuestUser, SandboxName, Wsl, WslError};

/// An error with a short wire word for the UI and a plain sentence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxError {
    pub code: &'static str,
    pub message: String,
}

impl SandboxError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SandboxError {}

impl From<LifecycleError> for SandboxError {
    fn from(e: LifecycleError) -> Self {
        Self::new(e.code(), e.message())
    }
}

impl From<InstallError> for SandboxError {
    fn from(e: InstallError) -> Self {
        Self::new(e.code(), e.message())
    }
}

impl From<ControlError> for SandboxError {
    fn from(e: ControlError) -> Self {
        Self::new(e.code(), e.message())
    }
}

impl From<WslError> for SandboxError {
    fn from(e: WslError) -> Self {
        LifecycleError::from(e).into()
    }
}

pub type SbxResult<T> = Result<T, SandboxError>;

/// What a screen picture holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Where the mouse pointer is, when the screen service said.
    pub pointer: Option<(i32, i32)>,
    /// The title of the front window (cleaned and cut). Text from a program in the sandbox: data, never instructions.
    pub active_window_title: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    NotInstalled,
    /// Our distro is there but its set up never ended ("Setup didn't finish"): it cannot be started, only carried on or removed.
    Incomplete,
    Stopped,
    Starting,
    Running,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: State,
    /// Why it failed, or why it cannot be used (plain English).
    pub reason: Option<String>,
    pub name: String,
    /// The idle stop setting in minutes.
    pub idle_minutes: u32,
}

/// What a command run inside the sandbox printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellOutput {
    /// The exit code, or `None` when the command was stopped for taking too long.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// The command was killed because it passed its time limit.
    pub timed_out: bool,
}

impl ShellOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Where the screen can be reached from this computer (loopback ports of the forwards WSL makes).
#[derive(Clone, Debug)]
pub struct ScreenTarget {
    /// noVNC (the viewer page and its websocket). The password typed decides view or control.
    pub view_port: u16,
    /// Changes at every start and stop; a screen link made for another generation is dead.
    pub generation: u64,
    /// The two VNC passwords of this run, only while the sandbox is running. The one way later code (the screen
    /// links) gets them; the type prints nothing and cannot be copied or saved.
    pub passwords: Option<Arc<ScreenSecrets>>,
}

/// One public web page as the sandbox fetched it: the content type of its last response, and its bytes.
pub struct PageFetch {
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// One step of a page fetch that does not follow redirects by itself: the answer, or the address it points to.
pub struct PageHop {
    pub content_type: String,
    pub bytes: Vec<u8>,
    /// The address a 3xx answer points to (already absolute). `None`: this is the page.
    pub redirect: Option<String>,
}

/// The sandbox, as the rest of the app sees it.
pub trait Sandbox: Send + Sync {
    fn status(&self) -> SbxResult<Status>;
    /// Makes the sandbox. `progress` gets a step name and a percentage as it goes.
    fn install(&self, options: &InstallOptions, progress: &mut dyn FnMut(Progress)) -> SbxResult<()>;
    fn start(&self) -> SbxResult<()>;
    fn stop(&self) -> SbxResult<()>;
    /// Saves the sandbox under `snapshot` (`[a-z0-9-]{1,30}`). The one made at the end of the install is `base`.
    fn snapshot(&self, snapshot: &str) -> SbxResult<()>;
    /// Throws the sandbox away and brings back the saved `snapshot`. Stopped afterwards.
    fn reset(&self, snapshot: &str) -> SbxResult<()>;
    /// Removes the sandbox and its reset points.
    fn remove(&self) -> SbxResult<()>;
    fn observe(&self) -> SbxResult<Observation>;
    fn act(&self, actions: &[Action]) -> SbxResult<Observation>;
    /// Runs the page-browser tool inside the sandbox with `args` and returns what it printed.
    fn browser(&self, args: &[String]) -> SbxResult<String>;
    /// Runs the page tool's `login` command with `stdin` as its only input (a user name and a password as one JSON line)
    /// and returns what it printed. The password travels on stdin, never in an argument list. Only the sign-in card calls
    /// this, with the screen held and the bot's tools blocked; the bot has no tool that reaches it.
    fn browser_login(&self, _stdin: Vec<u8>) -> SbxResult<String> {
        Err(SandboxError::new("SANDBOX_LOGIN_UNSUPPORTED", "This sandbox cannot sign in."))
    }
    /// The MCP tools the guest offers (its answer, as the guest sent it). Default: this sandbox has none.
    fn mcp_tools(&self) -> SbxResult<serde_json::Value> {
        Err(SandboxError::new("SANDBOX_MCP_UNSUPPORTED", "This sandbox cannot list tools."))
    }
    /// One MCP tool call inside the guest. Default: unsupported.
    fn mcp_call(&self, _server: &str, _tool: &str, _arguments: &serde_json::Value) -> SbxResult<serde_json::Value> {
        Err(SandboxError::new("SANDBOX_MCP_UNSUPPORTED", "This sandbox cannot call tools."))
    }
    /// Replaces the guest's MCP server list. Default: unsupported.
    fn mcp_config(&self, _servers: &serde_json::Value) -> SbxResult<()> {
        Err(SandboxError::new("SANDBOX_MCP_UNSUPPORTED", "This sandbox cannot set tool servers."))
    }
    /// Runs one shell command as root in the computer, for the app's own installs (a connector's server goes under a
    /// root-owned folder so the bot, who is user pm, cannot change a program that will hold a token). Never for the model.
    fn root_shell(&self, _command: &str, _timeout: std::time::Duration) -> SbxResult<ShellOutput> {
        Err(SandboxError::new("SANDBOX_MCP_UNSUPPORTED", "This sandbox cannot run an install."))
    }
    /// Replaces the tool servers that hold secrets: `[{name, command, args, env}]`. They are owned by a root process in the
    /// computer that only the host can talk to (its stdin and stdout), never by the control service the bot can reach, and
    /// their calls and tool lists come back through `mcp_call` and `mcp_tools` like any other server's. An empty list stops them.
    fn secret_config(&self, _servers: &serde_json::Value) -> SbxResult<()> {
        Err(SandboxError::new("SANDBOX_MCP_UNSUPPORTED", "This sandbox cannot keep tool server secrets."))
    }
    /// Runs a shell command as the sandbox user (a terminal). `cwd` is an absolute path inside the sandbox.
    fn shell(&self, _command: &str, _cwd: Option<&str>, _timeout: std::time::Duration) -> SbxResult<ShellOutput> {
        Err(SandboxError::new("SANDBOX_SHELL_UNSUPPORTED", "This sandbox cannot run commands."))
    }
    /// Reads a file inside the sandbox (at most `max` bytes).
    fn read_file(&self, _path: &str, _max: usize) -> SbxResult<Vec<u8>> {
        Err(SandboxError::new("SANDBOX_FILES_UNSUPPORTED", "This sandbox cannot read files."))
    }
    /// Reads a whole file as raw bytes, for the owner to receive (`send_file`). Not cut at a text limit: a file bigger
    /// than `max` is refused (`SANDBOX_FILE_TOO_BIG`), and the bytes are never decoded as text.
    fn fetch_file(&self, _path: &str, _max: usize) -> SbxResult<Vec<u8>> {
        Err(SandboxError::new("SANDBOX_FILES_UNSUPPORTED", "This sandbox cannot send files."))
    }
    /// Fetches one public web page inside the sandbox, as plain bytes (at most `max`). Default: unsupported.
    fn fetch_page(&self, _url: &str, _max: usize) -> SbxResult<PageFetch> {
        Err(SandboxError::new("SANDBOX_WEB_UNSUPPORTED", "This sandbox cannot read web pages."))
    }
    /// Like `fetch_page`, but asks for ONE address and does not follow a redirect: the answer says where it points, and the caller
    /// decides whether to go there (the owner's site limit checks every step). Default: unsupported.
    fn fetch_page_hop(&self, _url: &str, _max: usize) -> SbxResult<PageHop> {
        Err(SandboxError::new("SANDBOX_WEB_UNSUPPORTED", "This sandbox cannot read web pages."))
    }
    /// Writes (or appends to) a file inside the sandbox, making its folder if needed.
    fn write_file(&self, _path: &str, _bytes: &[u8], _append: bool) -> SbxResult<()> {
        Err(SandboxError::new("SANDBOX_FILES_UNSUPPORTED", "This sandbox cannot write files."))
    }
    /// The person's file browser: runs the one fixed files script in the sandbox with `args` (an operation and its
    /// arguments) and `stdin`. The script is the only place a path is checked, so no caller can skip the check. Not for
    /// the bot's tools: they have `read_file` and the rest.
    fn files_run(&self, _args: &[String], _stdin: Option<Vec<u8>>) -> SbxResult<files_script::FilesOutput> {
        Err(SandboxError::new("SANDBOX_FILES_UNSUPPORTED", "This sandbox cannot browse files."))
    }
    fn screen_target(&self) -> SbxResult<ScreenTarget>;
    /// The person holds the screen (a link or a lease) until `until_ms`: the sandbox counts as in use until then.
    fn person_holds(&self, _until_ms: i64) {}

    // ---- the operating systems (desktops) installed in this sandbox ----

    /// The desktops this sandbox can start in, the active one first (wizard words like `ubuntu`, `mint`).
    fn installed_desktops(&self) -> Vec<String> {
        Vec::new()
    }
    /// The wizard word of the desktop the sandbox starts in.
    fn active_desktop(&self) -> Option<String> {
        None
    }
    /// Installs another desktop into a sandbox that is already set up and makes it the active one.
    fn add_desktop(&self, _word: &str, _progress: &mut dyn FnMut(Progress)) -> SbxResult<()> {
        Err(SandboxError::new("SANDBOX_OS_UNSUPPORTED", "This sandbox cannot have more than one OS."))
    }
    /// Makes an already installed desktop the one the sandbox starts in.
    fn switch_desktop(&self, _word: &str) -> SbxResult<()> {
        Err(SandboxError::new("SANDBOX_OS_UNSUPPORTED", "This sandbox cannot switch OS."))
    }

    // ---- what the settings page needs (each has a default so a sandbox that cannot do it just does not) ----

    /// Asks a running `install` to stop at its next step. Nothing running: nothing happens.
    fn cancel_install(&self) {}
    /// The answers a set up that did not finish started with (to carry it on), when they are known.
    fn resume_options(&self) -> Option<InstallOptions> {
        None
    }
    /// Whether Windows' Linux feature answers. A sandbox that does not need it always says yes.
    fn linux_feature(&self) -> bool {
        true
    }
    /// The idle stop setting. Returns the number that is used (it is kept between 1 and 240).
    fn set_idle_minutes(&self, minutes: u32) -> u32 {
        lifecycle::clamp_idle_minutes(minutes)
    }
    /// Called by the app every half minute: stops the sandbox when it has been idle long enough.
    fn idle_tick(&self) {}
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

const BROWSER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const BROWSER_OUTPUT_CHARS: usize = 64 * 1024;
/// How long a whole-file read for `send_file` may take (a 10 MB file over the WSL pipe).
const FILE_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// How long one `web_fetch` may take in the guest (curl stops itself at 20 seconds).
const PAGE_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Most characters of a command's output the model is shown.
const SHELL_OUTPUT_CHARS: usize = 64 * 1024;
/// The longest a shell command may run (the app's own provisioning may take minutes; the brain's tool caps lower).
pub const SHELL_MAX_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// The environment a terminal starts with: the app's own display plus the DBus address of the desktop session, so
/// `xfconf-query`, `gsettings` and GUI programs reach the running desktop. Harmless when the session is not up yet.
const SHELL_PRELUDE: &str =
    "export PATH=\"$HOME/.local/bin:$PATH\"; [ -r \"$HOME/.pm-sbx/env\" ] && . \"$HOME/.pm-sbx/env\" >/dev/null 2>&1 || true; ";

/// One argument quoted for `/bin/sh`: single quotes, with an embedded quote closed and reopened.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn cut_chars(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        return text;
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str("\n[output cut]");
    out
}

/// The real sandbox: a WSL 2 distro.
pub struct WslSandbox {
    wsl: Wsl,
    name: SandboxName,
    layout: Layout,
    files: GuestFiles,
    lifecycle: Arc<Lifecycle>,
    cancel: std::sync::atomic::AtomicBool,
    records: Arc<dyn InstallRecords>,
    /// A sandbox made before records existed: what the one look inside found (`Some(true)` = it ended).
    legacy: parking_lot::Mutex<Option<bool>>,
    /// A finished record from before the desktop was kept: what the guest says, looked at once (`None` = looked and unknown).
    legacy_desktop: parking_lot::Mutex<Option<Option<Desktop>>>,
    /// The connectors' servers and the pipe to the root process that runs them (see `secret_hub`).
    secret: parking_lot::Mutex<SecretState>,
}

#[derive(Default)]
struct SecretState {
    hub: Option<secret_hub::SecretHub>,
    /// The last list sent, values included, kept in the host so a new hub can be given it again.
    servers: Vec<serde_json::Value>,
}

impl WslSandbox {
    pub fn new(wsl: Wsl, name: SandboxName, layout: Layout, files: GuestFiles, lifecycle: Arc<Lifecycle>) -> Self {
        Self {
            wsl,
            name,
            layout,
            files,
            lifecycle,
            cancel: std::sync::atomic::AtomicBool::new(false),
            records: Arc::new(NoRecords),
            legacy: parking_lot::Mutex::new(None),
            legacy_desktop: parking_lot::Mutex::new(None),
            secret: parking_lot::Mutex::new(SecretState::default()),
        }
    }

    /// Stops the connectors' process (it stops its servers and exits). Safe when there is none.
    fn stop_secret_hub(&self) {
        let hub = self.secret.lock().hub.take();
        drop(hub);
    }

    /// Runs `f` on a live hub, starting one (and giving it the last list) when there is none or it died: the pipe closed,
    /// a crash, the computer stopped and came back. Nothing is started when no connector is set.
    fn with_secret_hub<T>(&self, f: impl FnOnce(&mut secret_hub::SecretHub) -> SbxResult<T>) -> SbxResult<T> {
        let mut state = self.secret.lock();
        if state.servers.is_empty() {
            return Err(SandboxError::new("SANDBOX_MCP_UNSUPPORTED", "No connector is set up."));
        }
        if state.hub.as_mut().map_or(true, |h| !h.alive()) {
            state.hub = None;
            let pipe = self.wsl.spawn_piped_root(&self.name, &["/usr/bin/python3", "-I", secret_hub::HUB_PATH])?;
            let mut hub = secret_hub::SecretHub::new(pipe);
            hub.config(&serde_json::Value::Array(state.servers.clone()))?;
            state.hub = Some(hub);
        }
        let hub = state.hub.as_mut().expect("set above");
        f(hub)
    }

    fn secret_names(&self) -> Vec<String> {
        self.secret.lock().servers.iter().filter_map(|s| s.get("name").and_then(|n| n.as_str()).map(str::to_string)).collect()
    }

    /// Gives the sandbox the store where the host keeps the record of a set up (without one, every sandbox counts as finished).
    pub fn with_records(mut self, records: Arc<dyn InstallRecords>) -> Self {
        self.records = records;
        self
    }

    /// Whether the set up of the distro ended. The record says; a distro without one (made by an older build) is looked at
    /// once: its guest has the marker, or the file the last recipe step writes. A look that fails counts as "finished".
    fn setup_finished(&self) -> bool {
        if !self.records.keeps_records() {
            return true;
        }
        if let Some(saved) = self.records.load(&self.name) {
            return saved.done;
        }
        let mut legacy = self.legacy.lock();
        if let Some(known) = *legacy {
            return known;
        }
        let script = format!("test -f {} || test -f /etc/pm-sbx/desktop.conf", install::GUEST_MARKER);
        let finished = match self.wsl.exec(&self.name, GuestUser::Root, &["/bin/sh", "-c", &script], None, std::time::Duration::from_secs(60)) {
            Ok(out) => match out.code {
                Some(0) => true,
                Some(_) => false,
                None => return true,
            },
            Err(_) => return true,
        };
        *legacy = Some(finished);
        if finished {
            self.records.save(&self.name, &SavedInstall::finished(None));
        }
        finished
    }

    fn ensure_finished(&self) -> SbxResult<()> {
        if self.setup_finished() {
            return Ok(());
        }
        Err(SandboxError::new("SANDBOX_SETUP_UNFINISHED", "Setup didn't finish. Continue setup, or start over."))
    }

    pub fn lifecycle(&self) -> &Arc<Lifecycle> {
        &self.lifecycle
    }

    /// Asks a running `install` to stop at its next step.
    pub fn cancel_install(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn snapshot_path(&self, snapshot: &str) -> SbxResult<std::path::PathBuf> {
        self.layout
            .snapshot_path(&self.name, snapshot)
            .ok_or_else(|| SandboxError::new("SANDBOX_BAD_SNAPSHOT", "A reset point name is lowercase letters, digits and -."))
    }

    fn saved_options(&self) -> Option<InstallOptions> {
        let saved = self.records.load(&self.name)?;
        if let Some(options) = saved.options {
            return Some(options);
        }
        if !saved.done || !self.records.keeps_records() {
            return None;
        }
        // A record from before the desktop was kept: read the guest once, remember it, and carry on.
        let desktop = self.installed_desktop_from_guest()?;
        let options = InstallOptions::new(self.name.clone(), desktop);
        self.records.save(&self.name, &SavedInstall::finished(Some(&options)));
        Some(options)
    }

    fn save_options(&self, options: &InstallOptions) {
        self.records.save(&self.name, &SavedInstall::finished(Some(options)));
    }

    /// The desktop an older sandbox runs, read from its marker or its `desktop.conf`. The first answer is kept; a stopped
    /// distro is left stopped. `None` when it cannot be read (the OS row then stays hidden rather than guessing).
    fn installed_desktop_from_guest(&self) -> Option<Desktop> {
        let mut once = self.legacy_desktop.lock();
        if let Some(known) = *once {
            return known;
        }
        let was_up = matches!(self.lifecycle.phase(), Phase::Ready | Phase::Starting);
        let script = "printf 'word=%s\\n' \"$(sed -n 's/^desktop=//p' /etc/pm-sbx/installed 2>/dev/null | head -1)\"; \
                      printf 'session=%s\\n' \"$(sed -n 's/^PM_SBX_SESSION=//p' /etc/pm-sbx/desktop.conf 2>/dev/null | head -1)\"";
        let desktop = match self.wsl.exec(&self.name, GuestUser::Root, &["/bin/sh", "-c", script], None, std::time::Duration::from_secs(60)) {
            Ok(out) => {
                let text = out.stdout_text();
                let value = |key: &str| {
                    text.lines()
                        .find_map(|l| l.trim().strip_prefix(key).and_then(|rest| rest.strip_prefix('=')))
                        .unwrap_or("")
                        .trim()
                };
                Desktop::parse(value("word")).or_else(|| match value("session") {
                    "cinnamon" => Some(Desktop::Mint),
                    "xfce4" => Some(Desktop::Xfce),
                    "fluxbox" => Some(Desktop::Light),
                    _ => None,
                })
            }
            Err(_) => None,
        };
        *once = Some(desktop);
        if !was_up {
            let _ = self.wsl.terminate(&self.name);
        }
        desktop
    }
}

impl Sandbox for WslSandbox {
    fn status(&self) -> SbxResult<Status> {
        let idle_minutes = self.lifecycle.idle_minutes();
        let make = |state, reason| Status { state, reason, name: self.name.to_string(), idle_minutes };
        if !self.wsl.is_available() {
            return Ok(make(State::NotInstalled, Some("Needs Windows' Linux feature (admin + restart).".to_string())));
        }
        match self.wsl.is_installed(&self.name) {
            Ok(true) => {}
            Ok(false) => return Ok(make(State::NotInstalled, None)),
            // `--status` answered but the distro list did not: the virtual machine cannot start yet (the feature
            // is on, but Windows has not restarted). That is not an internal error.
            Err(_) => return Ok(make(State::NotInstalled, Some("Needs Windows' Linux feature (admin + restart).".to_string()))),
        }
        let phase = self.lifecycle.phase();
        if !matches!(phase, Phase::Ready | Phase::Starting) && !self.setup_finished() {
            return Ok(make(State::Incomplete, Some("Setup didn't finish.".to_string())));
        }
        Ok(match phase {
            Phase::Ready => make(State::Running, None),
            Phase::Starting => make(State::Starting, None),
            Phase::Failed(reason) => make(State::Failed, Some(reason)),
            Phase::Stopped => make(State::Stopped, None),
        })
    }

    fn install(&self, options: &InstallOptions, progress: &mut dyn FnMut(Progress)) -> SbxResult<()> {
        self.cancel.store(false, std::sync::atomic::Ordering::SeqCst);
        let env = install::InstallEnv { wsl: &self.wsl, layout: &self.layout, files: &self.files, lifecycle: &self.lifecycle, records: self.records.as_ref() };
        install::run(&env, options, &self.cancel, &now_ms, progress)?;
        Ok(())
    }

    fn start(&self) -> SbxResult<()> {
        self.ensure_finished()?;
        self.lifecycle.ensure_ready(now_ms())?;
        Ok(())
    }

    fn stop(&self) -> SbxResult<()> {
        self.stop_secret_hub();
        self.lifecycle.stop()?;
        Ok(())
    }

    fn snapshot(&self, snapshot: &str) -> SbxResult<()> {
        let tar = self.snapshot_path(snapshot)?;
        self.stop_secret_hub();
        self.lifecycle.stop()?;
        if let Some(dir) = tar.parent() {
            std::fs::create_dir_all(dir).map_err(|_| SandboxError::new("SANDBOX_SNAPSHOT_FAILED", "The reset point folder could not be made."))?;
        }
        self.wsl.export(&self.name, &tar)?;
        Ok(())
    }

    fn reset(&self, snapshot: &str) -> SbxResult<()> {
        let tar = self.snapshot_path(snapshot)?;
        if !tar.is_file() {
            return Err(SandboxError::new("SANDBOX_NO_SNAPSHOT", "There is no saved reset point to go back to."));
        }
        self.stop_secret_hub();
        self.lifecycle.stop()?;
        if self.wsl.is_installed(&self.name)? {
            self.wsl.unregister(&self.name)?;
        }
        let dir = self.layout.distro_dir(&self.name);
        std::fs::create_dir_all(&dir).map_err(|_| SandboxError::new("SANDBOX_RESET_FAILED", "The sandbox folder could not be made."))?;
        self.wsl.import(&self.name, &dir, &tar)?;
        // The reset point is the sandbox at the end of set up: any OS added or switched to since is gone. The marker inside
        // remembers the desktop the set up ended in, so the OS row follows the guest instead of naming an OS it no longer has.
        if let Some(mut options) = self.saved_options() {
            let script = "sed -n 's/^desktop=//p' /etc/pm-sbx/installed 2>/dev/null | head -1";
            if let Ok(out) = self.wsl.exec(&self.name, GuestUser::Root, &["/bin/sh", "-c", script], None, std::time::Duration::from_secs(60)) {
                if let Some(desktop) = Desktop::parse(out.stdout_text().trim()) {
                    options.desktop = desktop;
                    options.others.clear();
                    self.save_options(&options);
                }
            }
            // Reading the marker started the distro. A reset leaves it stopped, as it found it.
            let _ = self.wsl.terminate(&self.name);
        }
        Ok(())
    }

    fn remove(&self) -> SbxResult<()> {
        self.stop_secret_hub();
        self.lifecycle.stop()?;
        if self.wsl.is_installed(&self.name)? {
            self.wsl.unregister(&self.name)?;
        }
        self.records.clear(&self.name);
        *self.legacy.lock() = None;
        // The reset points are files we wrote through wsl.exe; only those files go, one by one.
        let dir = self.layout.snapshots_dir(&self.name);
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "tar") && path.is_file() {
                    let _ = std::fs::remove_file(path);
                }
            }
            let _ = std::fs::remove_dir(&dir);
        }
        Ok(())
    }

    fn observe(&self) -> SbxResult<Observation> {
        self.ensure_finished()?;
        let _busy = self.lifecycle.begin_use();
        let ready = self.lifecycle.ensure_ready(now_ms())?;
        let shot = ready.client.screenshot()?;
        self.lifecycle.touch(now_ms());
        Ok(shot)
    }

    fn act(&self, actions: &[Action]) -> SbxResult<Observation> {
        self.ensure_finished()?;
        let _busy = self.lifecycle.begin_use();
        let ready = self.lifecycle.ensure_ready(now_ms())?;
        let result = ready.client.act(actions, 300, true)?;
        self.lifecycle.touch(now_ms());
        result.observation.ok_or_else(|| SandboxError::new("SANDBOX_BAD_REPLY", "The sandbox sent an answer the app could not read."))
    }

    fn browser(&self, args: &[String]) -> SbxResult<String> {
        self.ensure_finished()?;
        let _busy = self.lifecycle.begin_use();
        self.lifecycle.ensure_ready(now_ms())?;
        let tool = format!("{}/page-browser/pm-sbx-page-browser", lifecycle::GUEST_SHARE);
        // (`Wsl::exec` as pm puts the sandbox display and the no-WSLg environment in front of the program.)
        let mut argv: Vec<&str> = vec!["/usr/bin/python3", "-I", &tool];
        argv.extend(args.iter().map(String::as_str));
        let out = self.wsl.exec(&self.name, GuestUser::Pm, &argv, None, BROWSER_TIMEOUT)?;
        self.lifecycle.touch(now_ms());
        Ok(out.stdout_text().chars().take(BROWSER_OUTPUT_CHARS).collect())
    }

    fn browser_login(&self, stdin: Vec<u8>) -> SbxResult<String> {
        self.ensure_finished()?;
        let _busy = self.lifecycle.begin_use();
        self.lifecycle.ensure_ready(now_ms())?;
        let tool = format!("{}/page-browser/pm-sbx-page-browser", lifecycle::GUEST_SHARE);
        let argv: Vec<&str> = vec!["/usr/bin/python3", "-I", &tool, "login"];
        let out = self.wsl.exec(&self.name, GuestUser::Pm, &argv, Some(stdin), BROWSER_TIMEOUT)?;
        self.lifecycle.touch(now_ms());
        Ok(out.stdout_text().chars().take(BROWSER_OUTPUT_CHARS).collect())
    }

    fn mcp_tools(&self) -> SbxResult<serde_json::Value> {
        self.ensure_finished()?;
        let _busy = self.lifecycle.begin_use();
        let ready = self.lifecycle.ensure_ready(now_ms())?;
        let mut reply = ready.client.mcp_tools()?;
        let names = self.secret_names();
        if !names.is_empty() {
            // The connectors' servers, from the root process. If it cannot answer they show as failed, not as missing.
            let theirs = match self.with_secret_hub(|hub| hub.tools()) {
                Ok(list) => list.get("servers").cloned().unwrap_or_else(|| serde_json::json!([])),
                Err(error) => serde_json::Value::Array(names.iter().map(|n| serde_json::json!({ "name": n, "status": "failed", "error": error.message, "tools": [] })).collect()),
            };
            if let (Some(mine), Some(more)) = (reply.get_mut("servers").and_then(|v| v.as_array_mut()), theirs.as_array()) {
                mine.extend(more.iter().cloned());
            }
        }
        self.lifecycle.touch(now_ms());
        Ok(reply)
    }

    fn mcp_call(&self, server: &str, tool: &str, arguments: &serde_json::Value) -> SbxResult<serde_json::Value> {
        self.ensure_finished()?;
        let _busy = self.lifecycle.begin_use();
        let ready = self.lifecycle.ensure_ready(now_ms())?;
        // A connector's call goes to the root process over its own pipe, never through the control service.
        let reply = if self.secret_names().iter().any(|n| n == server) {
            self.with_secret_hub(|hub| hub.call(server, tool, arguments))?
        } else {
            ready.client.mcp_call(server, tool, arguments)?
        };
        self.lifecycle.touch(now_ms());
        Ok(reply)
    }

    fn mcp_config(&self, servers: &serde_json::Value) -> SbxResult<()> {
        self.ensure_finished()?;
        let _busy = self.lifecycle.begin_use();
        let ready = self.lifecycle.ensure_ready(now_ms())?;
        ready.client.mcp_config(servers)?;
        self.lifecycle.touch(now_ms());
        Ok(())
    }

    fn secret_config(&self, servers: &serde_json::Value) -> SbxResult<()> {
        self.ensure_finished()?;
        let list = servers.as_array().cloned().unwrap_or_default();
        if list.is_empty() {
            self.secret.lock().servers.clear();
            self.stop_secret_hub();
            return Ok(());
        }
        let _busy = self.lifecycle.begin_use();
        self.lifecycle.ensure_ready(now_ms())?;
        self.secret.lock().servers = list;
        // A hub that is alive gets the new list; a dead or missing one is started with it.
        let alive = self.secret.lock().hub.as_mut().is_some_and(|h| h.alive());
        if alive {
            let all = serde_json::Value::Array(self.secret.lock().servers.clone());
            self.with_secret_hub(|hub| hub.config(&all))?;
        } else {
            self.with_secret_hub(|_| Ok(()))?;
        }
        self.lifecycle.touch(now_ms());
        Ok(())
    }

    fn root_shell(&self, command: &str, timeout: std::time::Duration) -> SbxResult<ShellOutput> {
        self.ensure_finished()?;
        let _busy = self.lifecycle.begin_use();
        self.lifecycle.ensure_ready(now_ms())?;
        let out = match self.wsl.exec(&self.name, GuestUser::Root, &["/bin/sh", "-c", command], None, timeout.min(SHELL_MAX_TIMEOUT)) {
            Ok(out) => out,
            Err(WslError::Timeout) => {
                return Ok(ShellOutput { code: None, stdout: String::new(), stderr: String::new(), timed_out: true });
            }
            Err(e) => return Err(e.into()),
        };
        self.lifecycle.touch(now_ms());
        Ok(ShellOutput { code: out.code, stdout: cut_chars(out.stdout_text(), SHELL_OUTPUT_CHARS), stderr: cut_chars(out.stderr_text(), SHELL_OUTPUT_CHARS), timed_out: false })
    }

    fn shell(&self, command: &str, cwd: Option<&str>, timeout: std::time::Duration) -> SbxResult<ShellOutput> {
        if command.as_bytes().len() > 16 * 1024 {
            return Err(SandboxError::new("SANDBOX_BAD_COMMAND", "That command is too long. Split it into smaller ones."));
        }
        let timeout = timeout.min(SHELL_MAX_TIMEOUT).max(std::time::Duration::from_secs(1));
        self.ensure_finished()?;
        let _busy = self.lifecycle.begin_use();
        self.lifecycle.ensure_ready(now_ms())?;
        let mut line = String::from(SHELL_PRELUDE);
        if let Some(dir) = cwd {
            line.push_str("cd -- ");
            line.push_str(&shell_quote(dir));
            line.push_str(" && ");
        }
        line.push_str(command);
        let out = match self.wsl.exec(&self.name, GuestUser::Pm, &["/bin/sh", "-lc", &line], None, timeout) {
            Ok(out) => out,
            Err(WslError::Timeout) => {
                self.lifecycle.touch(now_ms());
                return Ok(ShellOutput {
                    code: None,
                    stdout: String::new(),
                    stderr: "The command took too long and was stopped.".to_string(),
                    timed_out: true,
                });
            }
            Err(e) => return Err(e.into()),
        };
        self.lifecycle.touch(now_ms());
        Ok(ShellOutput {
            code: out.code,
            stdout: cut_chars(out.stdout_text(), SHELL_OUTPUT_CHARS),
            stderr: cut_chars(out.stderr_text(), SHELL_OUTPUT_CHARS),
            timed_out: false,
        })
    }

    fn read_file(&self, path: &str, max: usize) -> SbxResult<Vec<u8>> {
        self.ensure_finished()?;
        let _busy = self.lifecycle.begin_use();
        self.lifecycle.ensure_ready(now_ms())?;
        let script = "test -f -- \"$1\" || { echo 'not a file' >&2; exit 2; }; head -c \"$2\" -- \"$1\"";
        let max_text = max.to_string();
        let out = self.wsl.exec(
            &self.name,
            GuestUser::Pm,
            &["/bin/sh", "-c", script, "sh", path, &max_text],
            None,
            std::time::Duration::from_secs(30),
        )?;
        self.lifecycle.touch(now_ms());
        if !out.success() {
            return Err(SandboxError::new("SANDBOX_NO_FILE", format!("{path} could not be read.")));
        }
        Ok(out.stdout)
    }

    fn fetch_file(&self, path: &str, max: usize) -> SbxResult<Vec<u8>> {
        self.ensure_finished()?;
        let _busy = self.lifecycle.begin_use();
        self.lifecycle.ensure_ready(now_ms())?;
        // Exit codes: 2 missing or not a file, 3 a folder, 4 bigger than `max`. The path is an argv word, never
        // spliced into the script; `head -c` caps the read in case the file grows between the two looks.
        let script = "test -d \"$1\" && exit 3; test -f \"$1\" || exit 2; s=$(stat -c %s -- \"$1\") || exit 2; [ \"$s\" -le \"$2\" ] || exit 4; head -c \"$2\" -- \"$1\"";
        let max_text = max.to_string();
        let out = self.wsl.exec(
            &self.name,
            GuestUser::Pm,
            &["/bin/sh", "-c", script, "sh", path, &max_text],
            None,
            FILE_FETCH_TIMEOUT,
        )?;
        self.lifecycle.touch(now_ms());
        match out.code {
            Some(0) => Ok(out.stdout),
            Some(3) => Err(SandboxError::new("SANDBOX_NOT_A_FILE", "That is a folder, not a file. Zip it first (run_terminal: zip -r name.zip folder), then send the zip.")),
            Some(4) => Err(SandboxError::new("SANDBOX_FILE_TOO_BIG", "That file is bigger than 10 MB, so it cannot be sent. Zip it or split it into parts, then send those.")),
            _ => Err(SandboxError::new("SANDBOX_NO_FILE", "There is no file at that path. Check the name with list_files.")),
        }
    }

    fn fetch_page(&self, url: &str, max: usize) -> SbxResult<PageFetch> {
        self.ensure_finished()?;
        let _busy = self.lifecycle.begin_use();
        self.lifecycle.ensure_ready(now_ms())?;
        // curl with argv: the address is one word, never part of a shell line. http and https only, for the first
        // address and for every redirect; at most 5 redirects; 20 seconds; the body is cut off past `max` bytes.
        // curl writes the final content type to stderr (write-out), so stdout is the body alone.
        let max_text = max.to_string();
        let out = match self.wsl.exec(
            &self.name,
            GuestUser::Pm,
            &[
                "/usr/bin/curl",
                "--silent",
                "--show-error",
                "--fail",
                "--location",
                "--max-redirs",
                "5",
                "--proto",
                "=http,https",
                "--proto-redir",
                "=http,https",
                "--max-time",
                "20",
                "--max-filesize",
                max_text.as_str(),
                "--write-out",
                "%{stderr}%{content_type}",
                "--url",
                url,
            ],
            None,
            PAGE_FETCH_TIMEOUT,
        ) {
            Ok(out) => out,
            Err(WslError::Timeout) => return Err(SandboxError::new("SANDBOX_PAGE_TIMEOUT", "The page took too long to answer.")),
            Err(e) => return Err(e.into()),
        };
        self.lifecycle.touch(now_ms());
        match out.code {
            Some(0) => {
                let content_type = out.stderr_text().trim().to_string();
                let mut bytes = out.stdout;
                bytes.truncate(max);
                Ok(PageFetch { content_type, bytes })
            }
            Some(22) => Err(SandboxError::new("SANDBOX_PAGE_HTTP", "The page did not load: the site answered with an error.")),
            Some(28) => Err(SandboxError::new("SANDBOX_PAGE_TIMEOUT", "The page took too long to answer.")),
            Some(47) => Err(SandboxError::new("SANDBOX_PAGE_REDIRECTS", "The page sent the bot round too many redirects.")),
            Some(63) => Err(SandboxError::new("SANDBOX_PAGE_TOO_BIG", "That page is bigger than 1 MB, so it was not read.")),
            _ => Err(SandboxError::new("SANDBOX_PAGE_FAILED", "The page could not be fetched.")),
        }
    }

    fn fetch_page_hop(&self, url: &str, max: usize) -> SbxResult<PageHop> {
        self.ensure_finished()?;
        let _busy = self.lifecycle.begin_use();
        self.lifecycle.ensure_ready(now_ms())?;
        // Like `fetch_page`, with no `--location`: curl stops at the first answer and prints where a redirect points. The
        // write-out (on stderr) is three lines: the content type, the status, the redirect address.
        let max_text = max.to_string();
        let out = match self.wsl.exec(
            &self.name,
            GuestUser::Pm,
            &[
                "/usr/bin/curl",
                "--silent",
                "--show-error",
                "--proto",
                "=http,https",
                "--max-time",
                "20",
                "--max-filesize",
                max_text.as_str(),
                "--write-out",
                "%{stderr}%{content_type}\n%{http_code}\n%{redirect_url}",
                "--url",
                url,
            ],
            None,
            PAGE_FETCH_TIMEOUT,
        ) {
            Ok(out) => out,
            Err(WslError::Timeout) => return Err(SandboxError::new("SANDBOX_PAGE_TIMEOUT", "The page took too long to answer.")),
            Err(e) => return Err(e.into()),
        };
        self.lifecycle.touch(now_ms());
        match out.code {
            Some(0) => {
                let meta = out.stderr_text();
                let mut lines = meta.lines();
                let content_type = lines.next().unwrap_or("").trim().to_string();
                let status: u16 = lines.next().unwrap_or("").trim().parse().unwrap_or(0);
                let redirect = lines.next().map(str::trim).filter(|r| !r.is_empty()).map(str::to_string);
                if (300..400).contains(&status) {
                    return match redirect {
                        Some(next) => Ok(PageHop { content_type, bytes: Vec::new(), redirect: Some(next) }),
                        None => Err(SandboxError::new("SANDBOX_PAGE_FAILED", "The page could not be fetched.")),
                    };
                }
                if status >= 400 {
                    return Err(SandboxError::new("SANDBOX_PAGE_HTTP", "The page did not load: the site answered with an error."));
                }
                let mut bytes = out.stdout;
                bytes.truncate(max);
                Ok(PageHop { content_type, bytes, redirect: None })
            }
            Some(28) => Err(SandboxError::new("SANDBOX_PAGE_TIMEOUT", "The page took too long to answer.")),
            Some(63) => Err(SandboxError::new("SANDBOX_PAGE_TOO_BIG", "That page is bigger than 1 MB, so it was not read.")),
            _ => Err(SandboxError::new("SANDBOX_PAGE_FAILED", "The page could not be fetched.")),
        }
    }

    fn write_file(&self, path: &str, bytes: &[u8], append: bool) -> SbxResult<()> {
        self.ensure_finished()?;
        let _busy = self.lifecycle.begin_use();
        self.lifecycle.ensure_ready(now_ms())?;
        let redirect = if append { ">>" } else { ">" };
        let script = format!("mkdir -p -- \"$(dirname -- \"$1\")\" && cat {redirect} \"$1\"");
        let out = self.wsl.exec(
            &self.name,
            GuestUser::Pm,
            &["/bin/sh", "-c", &script, "sh", path],
            Some(bytes.to_vec()),
            std::time::Duration::from_secs(30),
        )?;
        self.lifecycle.touch(now_ms());
        out.ok().map(|_| ()).map_err(|_| SandboxError::new("SANDBOX_FILE_WRITE_FAILED", format!("{path} could not be written.")))
    }

    fn files_run(&self, args: &[String], stdin: Option<Vec<u8>>) -> SbxResult<files_script::FilesOutput> {
        self.ensure_finished()?;
        let _busy = self.lifecycle.begin_use();
        self.lifecycle.ensure_ready(now_ms())?;
        let script = files_script::script();
        let mut argv: Vec<&str> = vec!["/bin/sh", "-c", script.as_str(), "sh"];
        argv.extend(args.iter().map(String::as_str));
        let out = match self.wsl.exec(&self.name, GuestUser::Pm, &argv, stdin, FILE_FETCH_TIMEOUT) {
            Ok(out) => out,
            Err(WslError::Timeout) => {
                self.lifecycle.touch(now_ms());
                return Ok(files_script::FilesOutput { code: None, stdout: Vec::new() });
            }
            Err(e) => return Err(e.into()),
        };
        self.lifecycle.touch(now_ms());
        Ok(files_script::FilesOutput { code: out.code, stdout: out.stdout })
    }

    fn person_holds(&self, until_ms: i64) {
        self.lifecycle.hold_for_person(until_ms);
    }

    fn installed_desktops(&self) -> Vec<String> {
        self.saved_options().map(|o| o.desktops().iter().map(|d| d.word().to_string()).collect()).unwrap_or_default()
    }

    fn active_desktop(&self) -> Option<String> {
        self.saved_options().map(|o| o.desktop.word().to_string())
    }

    fn add_desktop(&self, word: &str, progress: &mut dyn FnMut(Progress)) -> SbxResult<()> {
        let desktop = Desktop::parse(word).ok_or_else(|| SandboxError::new("SANDBOX_BAD_OPTIONS", "Pick an OS this version offers."))?;
        let mut saved = self
            .saved_options()
            .ok_or_else(|| SandboxError::new("SANDBOX_OS_UNKNOWN", "This sandbox was made by an older version. Start over to change its OS."))?;
        if saved.desktops().iter().any(|d| d.recipe_id() == desktop.recipe_id()) {
            return Err(SandboxError::new("SANDBOX_OS_INSTALLED", "That OS is already installed in the sandbox."));
        }
        let was_ready = matches!(self.lifecycle.phase(), Phase::Ready | Phase::Starting);
        // The plan's resume lane installs into the distro that is already there and leaves the add-ons alone.
        let mut run = InstallOptions::new(self.name.clone(), desktop);
        run.resume = true;
        run.snapshot = false;
        self.cancel.store(false, std::sync::atomic::Ordering::SeqCst);
        let env = InstallEnv { wsl: &self.wsl, layout: &self.layout, files: &self.files, lifecycle: &self.lifecycle, records: self.records.as_ref() };
        install::run(&env, &run, &self.cancel, &now_ms, progress)?;
        // `install::run` saved the run's own answers; keep the ones from setup and remember every desktop now installed.
        let mut others = vec![saved.desktop];
        others.extend(saved.others.iter().copied());
        others.retain(|d| *d != desktop);
        saved.others = others;
        saved.desktop = desktop;
        self.save_options(&saved);
        if was_ready {
            let _ = self.lifecycle.ensure_ready(now_ms());
        }
        Ok(())
    }

    fn switch_desktop(&self, word: &str) -> SbxResult<()> {
        let desktop = Desktop::parse(word).ok_or_else(|| SandboxError::new("SANDBOX_BAD_OPTIONS", "Pick an OS this version offers."))?;
        let mut saved = self.saved_options().ok_or_else(|| SandboxError::new("SANDBOX_NOT_INSTALLED", "The sandbox is not set up yet."))?;
        if !saved.desktops().iter().any(|d| d.word() == word) {
            return Err(SandboxError::new("SANDBOX_OS_NOT_INSTALLED", "That OS is not installed in the sandbox yet."));
        }
        let was_ready = matches!(self.lifecycle.phase(), Phase::Ready | Phase::Starting);
        if was_ready {
            self.lifecycle.stop()?;
        }
        let script = format!(
            "umask 022 && mkdir -p /etc/pm-sbx && printf '%s\\n' 'PM_SBX_SESSION={}' > /etc/pm-sbx/desktop.conf",
            desktop.session()
        );
        self.wsl
            .exec(&self.name, GuestUser::Root, &["/bin/sh", "-c", &script], None, std::time::Duration::from_secs(60))
            .and_then(|out| out.ok())?;
        let mut others = vec![saved.desktop];
        others.extend(saved.others.iter().copied());
        others.retain(|d| d.recipe_id() != desktop.recipe_id());
        saved.others = others;
        saved.desktop = desktop;
        self.save_options(&saved);
        if was_ready {
            self.lifecycle.ensure_ready(now_ms())?;
        } else {
            // Writing the file started the distro. A switch on a stopped sandbox leaves it stopped.
            let _ = self.wsl.terminate(&self.name);
        }
        Ok(())
    }

    fn screen_target(&self) -> SbxResult<ScreenTarget> {
        Ok(ScreenTarget {
            view_port: self.lifecycle.view_port(),
            generation: self.lifecycle.generation(),
            passwords: self.lifecycle.screen_secrets(),
        })
    }

    fn cancel_install(&self) {
        WslSandbox::cancel_install(self);
    }

    fn resume_options(&self) -> Option<InstallOptions> {
        if !self.records.keeps_records() {
            return None;
        }
        let saved = self.records.load(&self.name)?;
        if saved.done {
            return None;
        }
        saved.options
    }

    fn linux_feature(&self) -> bool {
        self.wsl.is_available()
    }

    fn set_idle_minutes(&self, minutes: u32) -> u32 {
        self.lifecycle.set_idle_minutes(minutes);
        self.lifecycle.idle_minutes()
    }

    fn idle_tick(&self) {
        // A failed stop is tried again at the next tick.
        let _ = self.lifecycle.idle_check(now_ms());
    }
}

// ---- the fake ------------------------------------------------------------------------------------

/// Everything a `FakeSandbox` was asked, in order.
#[cfg(any(test, feature = "dev-fake-tui"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recorded {
    Install,
    Start,
    Stop,
    Snapshot(String),
    Reset(String),
    Remove,
    Observe,
    Act(Vec<Action>),
    Browser(Vec<String>),
    /// A `login` call: how many bytes went on its stdin (the bytes themselves are only in `FakeSandbox::login_stdin`).
    BrowserLogin(usize),
    Shell(String),
    RootShell(String),
    SecretConfig(Vec<String>),
    ReadFile(String),
    FetchFile(String),
    WriteFile(String, usize),
    /// A file browser call: the operation and its arguments.
    FilesRun(Vec<String>),
    /// One step of a page fetch (the address asked for).
    FetchPageHop(String),
}

#[cfg(any(test, feature = "dev-fake-tui"))]
#[derive(Default)]
struct FakeState {
    installed: bool,
    running: bool,
    calls: Vec<Recorded>,
    generation: u64,
    fail_next: Option<SandboxError>,
    browser_reply: String,
    /// Answers to the next `browser` calls, first in first out; then `browser_reply`.
    browser_queue: Vec<String>,
    /// What each `browser_login` got on its stdin, and a hook that runs while one is in progress.
    login_seen: Vec<Vec<u8>>,
    on_login: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    /// `None`: the default. Set by `set_idle_minutes`.
    idle_minutes: Option<u32>,
    no_linux_feature: bool,
    cancelled: bool,
    desktops: Vec<String>,
    active_desktop: Option<String>,
    /// Files the fake holds, by path (for `fetch_file`).
    files: std::collections::HashMap<String, Vec<u8>>,
    /// The fake is a small file system: `read_file` reads `files` (a missing path is `SANDBOX_NO_FILE`) and `write_file` writes them.
    file_system: bool,
    /// The last server list `mcp_config` got, and the reply `mcp_call` gives (for the connector tests).
    mcp_config: Option<serde_json::Value>,
    mcp_reply: Option<serde_json::Value>,
    secret_config: Option<serde_json::Value>,
    /// What `files_run` does: a test sets a function that plays the guest script (it may run the real script on a folder of
    /// this computer, from the test's own file). Without one the fake answers `SANDBOX_FILES_UNSUPPORTED`.
    files_hook: Option<FilesHook>,
    /// The answers `fetch_page_hop` gives, first in first out (an empty queue answers a plain empty page).
    page_hops: Vec<PageHop>,
    /// The answers `shell` gives, first in first out (an empty queue answers "fake shell", exit 0).
    shell_queue: Vec<ShellOutput>,
}

#[cfg(any(test, feature = "dev-fake-tui"))]
pub type FilesHook = std::sync::Arc<dyn Fn(&[String], Option<Vec<u8>>) -> files_script::FilesOutput + Send + Sync>;

/// A sandbox with no WSL: it serves one static picture and records every call. Starts out installed and
/// stopped; `not_installed()` makes one that needs `install` first.
#[cfg(any(test, feature = "dev-fake-tui"))]
pub struct FakeSandbox {
    state: parking_lot::Mutex<FakeState>,
    name: SandboxName,
}

#[cfg(any(test, feature = "dev-fake-tui"))]
impl FakeSandbox {
    pub fn new() -> Self {
        Self { state: parking_lot::Mutex::new(FakeState { installed: true, ..Default::default() }), name: SandboxName::main() }
    }

    pub fn not_installed() -> Self {
        Self { state: parking_lot::Mutex::new(FakeState::default()), name: SandboxName::main() }
    }

    /// The next call fails with this error (once).
    pub fn fail_next(&self, error: SandboxError) {
        self.state.lock().fail_next = Some(error);
    }

    /// Makes the fake keep what is written and refuse to read what is not there, like a real computer.
    pub fn use_file_system(&self) {
        self.state.lock().file_system = true;
    }

    /// A file on the fake computer, if there is one.
    pub fn file(&self, path: &str) -> Option<Vec<u8>> {
        self.state.lock().files.get(path).cloned()
    }

    /// Puts a file on the fake computer (for `fetch_file`).
    pub fn put_file(&self, path: &str, bytes: &[u8]) {
        self.state.lock().files.insert(path.to_string(), bytes.to_vec());
    }

    /// The last list of tool servers sent to the fake computer.
    pub fn queue_shell_results(&self, results: Vec<ShellOutput>) {
        self.state.lock().shell_queue = results;
    }

    pub fn queue_page_hops(&self, hops: Vec<PageHop>) {
        self.state.lock().page_hops = hops;
    }

    pub fn set_files_hook(&self, hook: FilesHook) {
        self.state.lock().files_hook = Some(hook);
    }

    pub fn last_mcp_config(&self) -> Option<serde_json::Value> {
        self.state.lock().mcp_config.clone()
    }

    /// The connectors' servers, with values, as the last `secret_config` left them (what the root process would hold).
    pub fn last_secret_config(&self) -> Option<serde_json::Value> {
        self.state.lock().secret_config.clone()
    }

    /// What the next `mcp_call`s answer (a guest's `{content, isError}`).
    pub fn set_mcp_reply(&self, reply: serde_json::Value) {
        self.state.lock().mcp_reply = Some(reply);
    }

    pub fn set_browser_reply(&self, reply: &str) {
        self.state.lock().browser_reply = reply.to_string();
    }

    /// The next `browser` calls answer with these, in order (then the usual reply).
    pub fn queue_browser_replies(&self, replies: &[&str]) {
        self.state.lock().browser_queue = replies.iter().map(|r| r.to_string()).collect();
    }

    /// What each `browser_login` got on its stdin (the only place a fake password may be).
    pub fn login_stdin(&self) -> Vec<Vec<u8>> {
        self.state.lock().login_seen.clone()
    }

    /// Runs `hook` while a `browser_login` is in progress (the fill and submit moment).
    pub fn on_login(&self, hook: impl Fn() + Send + Sync + 'static) {
        self.state.lock().on_login = Some(std::sync::Arc::new(hook));
    }

    /// A PC without Windows' Linux feature: `install` fails with the "needs Windows' Linux feature" error.
    pub fn without_linux_feature(self) -> Self {
        self.state.lock().no_linux_feature = true;
        self
    }

    /// True after `cancel_install` was called.
    pub fn was_cancelled(&self) -> bool {
        self.state.lock().cancelled
    }

    pub fn calls(&self) -> Vec<Recorded> {
        self.state.lock().calls.clone()
    }

    /// Every action of every `act` call, flattened.
    pub fn actions(&self) -> Vec<Action> {
        self.calls()
            .into_iter()
            .filter_map(|c| if let Recorded::Act(a) = c { Some(a) } else { None })
            .flatten()
            .collect()
    }

    pub fn is_running(&self) -> bool {
        self.state.lock().running
    }

    /// The picture the fake serves: a small grey PNG.
    pub fn picture() -> Observation {
        let (width, height) = (160u32, 100u32);
        let data: Vec<u8> = (0..width * height).map(|i| 40 + ((i % width) * 100 / width) as u8).collect();
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, width, height);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            if let Ok(mut writer) = encoder.write_header() {
                let _ = writer.write_image_data(&data);
            }
        }
        Observation { png, width, height, pointer: Some((10, 10)), active_window_title: Some("Fake desktop".to_string()) }
    }

    fn enter(&self, call: Recorded) -> SbxResult<parking_lot::MutexGuard<'_, FakeState>> {
        let mut state = self.state.lock();
        state.calls.push(call);
        match state.fail_next.take() {
            Some(error) => Err(error),
            None => Ok(state),
        }
    }

    /// A screen call: starts the fake if it was stopped, like the real one.
    fn use_screen(&self, call: Recorded) -> SbxResult<()> {
        let mut state = self.enter(call)?;
        if !state.installed {
            return Err(SandboxError::new("SANDBOX_NOT_INSTALLED", "The sandbox is not set up yet."));
        }
        if !state.running {
            state.running = true;
            state.generation += 1;
        }
        Ok(())
    }
}

#[cfg(any(test, feature = "dev-fake-tui"))]
impl Default for FakeSandbox {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "dev-fake-tui"))]
impl Sandbox for FakeSandbox {
    fn status(&self) -> SbxResult<Status> {
        let state = self.state.lock();
        let which = match (state.installed, state.running) {
            (false, _) => State::NotInstalled,
            (true, false) => State::Stopped,
            (true, true) => State::Running,
        };
        let idle_minutes = state.idle_minutes.unwrap_or(lifecycle::DEFAULT_IDLE_MINUTES);
        Ok(Status { state: which, reason: None, name: self.name.to_string(), idle_minutes })
    }

    fn install(&self, options: &InstallOptions, progress: &mut dyn FnMut(Progress)) -> SbxResult<()> {
        let mut state = self.enter(Recorded::Install)?;
        options.validate()?;
        if state.no_linux_feature {
            return Err(InstallError::NeedsWindowsFeature.into());
        }
        if state.installed {
            return Err(InstallError::AlreadyInstalled.into());
        }
        let steps = install::plan(options);
        for (i, step) in steps.iter().enumerate() {
            progress(Progress {
                step: step.id(),
                title: step.title(),
                bytes: None,
                index: i + 1,
                of: steps.len(),
                percent: install::percent_at(&steps, i, 1.0),
                detail: None,
            });
        }
        state.installed = true;
        state.desktops = options.desktops().iter().map(|d| d.word().to_string()).collect();
        state.active_desktop = Some(options.desktop.word().to_string());
        Ok(())
    }

    fn installed_desktops(&self) -> Vec<String> {
        self.state.lock().desktops.clone()
    }

    fn active_desktop(&self) -> Option<String> {
        self.state.lock().active_desktop.clone()
    }

    fn add_desktop(&self, word: &str, progress: &mut dyn FnMut(Progress)) -> SbxResult<()> {
        let desktop = Desktop::parse(word).ok_or_else(|| SandboxError::new("SANDBOX_BAD_OPTIONS", "Pick an OS this version offers."))?;
        let mut state = self.state.lock();
        if !state.installed {
            return Err(SandboxError::new("SANDBOX_NOT_INSTALLED", "The sandbox is not set up yet."));
        }
        if state.desktops.iter().any(|d| d == word) {
            return Err(SandboxError::new("SANDBOX_OS_INSTALLED", "That OS is already installed in the sandbox."));
        }
        let steps = install::plan(&InstallOptions { name: self.name.clone(), desktop, others: Vec::new(), addons: Vec::new(), options: Vec::new(), snapshot: false, resume: true });
        for (i, step) in steps.iter().enumerate() {
            progress(Progress {
                step: step.id(),
                title: step.title(),
                bytes: None,
                index: i + 1,
                of: steps.len(),
                percent: install::percent_at(&steps, i, 1.0),
                detail: None,
            });
        }
        state.desktops.push(word.to_string());
        state.active_desktop = Some(word.to_string());
        Ok(())
    }

    fn switch_desktop(&self, word: &str) -> SbxResult<()> {
        let mut state = self.state.lock();
        if !state.desktops.iter().any(|d| d == word) {
            return Err(SandboxError::new("SANDBOX_OS_NOT_INSTALLED", "That OS is not installed in the sandbox yet."));
        }
        state.active_desktop = Some(word.to_string());
        Ok(())
    }

    fn start(&self) -> SbxResult<()> {
        self.use_screen(Recorded::Start)
    }

    fn stop(&self) -> SbxResult<()> {
        let mut state = self.enter(Recorded::Stop)?;
        if state.running {
            state.running = false;
            state.generation += 1;
        }
        Ok(())
    }

    fn snapshot(&self, snapshot: &str) -> SbxResult<()> {
        let _state = self.enter(Recorded::Snapshot(snapshot.to_string()))?;
        if !install::valid_snapshot_id(snapshot) {
            return Err(SandboxError::new("SANDBOX_BAD_SNAPSHOT", "A reset point name is lowercase letters, digits and -."));
        }
        Ok(())
    }

    fn reset(&self, snapshot: &str) -> SbxResult<()> {
        let mut state = self.enter(Recorded::Reset(snapshot.to_string()))?;
        if !install::valid_snapshot_id(snapshot) {
            return Err(SandboxError::new("SANDBOX_BAD_SNAPSHOT", "A reset point name is lowercase letters, digits and -."));
        }
        state.running = false;
        state.generation += 1;
        Ok(())
    }

    fn remove(&self) -> SbxResult<()> {
        let mut state = self.enter(Recorded::Remove)?;
        state.installed = false;
        state.running = false;
        state.generation += 1;
        state.desktops.clear();
        state.active_desktop = None;
        Ok(())
    }

    fn observe(&self) -> SbxResult<Observation> {
        self.use_screen(Recorded::Observe)?;
        Ok(Self::picture())
    }

    fn act(&self, actions: &[Action]) -> SbxResult<Observation> {
        control::plan(actions)?;
        self.use_screen(Recorded::Act(actions.to_vec()))?;
        Ok(Self::picture())
    }

    fn browser(&self, args: &[String]) -> SbxResult<String> {
        self.use_screen(Recorded::Browser(args.to_vec()))?;
        let mut state = self.state.lock();
        Ok(if state.browser_queue.is_empty() { state.browser_reply.clone() } else { state.browser_queue.remove(0) })
    }

    fn browser_login(&self, stdin: Vec<u8>) -> SbxResult<String> {
        self.use_screen(Recorded::BrowserLogin(stdin.len()))?;
        let hook = {
            let mut state = self.state.lock();
            state.login_seen.push(stdin);
            state.on_login.clone()
        };
        if let Some(hook) = hook {
            hook();
        }
        let mut state = self.state.lock();
        Ok(if state.browser_queue.is_empty() { state.browser_reply.clone() } else { state.browser_queue.remove(0) })
    }

    fn root_shell(&self, command: &str, _timeout: std::time::Duration) -> SbxResult<ShellOutput> {
        self.use_screen(Recorded::RootShell(command.to_string()))?;
        Ok(ShellOutput { code: Some(0), stdout: String::new(), stderr: String::new(), timed_out: false })
    }

    fn secret_config(&self, servers: &serde_json::Value) -> SbxResult<()> {
        let names = servers.as_array().map(|l| l.iter().filter_map(|s| s["name"].as_str().map(str::to_string)).collect()).unwrap_or_default();
        let mut state = self.enter(Recorded::SecretConfig(names))?;
        state.secret_config = Some(servers.clone());
        Ok(())
    }

    fn mcp_config(&self, servers: &serde_json::Value) -> SbxResult<()> {
        self.state.lock().mcp_config = Some(servers.clone());
        Ok(())
    }

    /// The computer's own tool servers: none on the fake (a test that wants some looks at `mcp_config`).
    fn mcp_tools(&self) -> SbxResult<serde_json::Value> {
        Ok(serde_json::json!({ "servers": [] }))
    }

    fn mcp_call(&self, _server: &str, _tool: &str, _arguments: &serde_json::Value) -> SbxResult<serde_json::Value> {
        self.state.lock().mcp_reply.clone().ok_or_else(|| SandboxError::new("SANDBOX_MCP_UNSUPPORTED", "This sandbox cannot call tools."))
    }

    fn shell(&self, command: &str, _cwd: Option<&str>, _timeout: std::time::Duration) -> SbxResult<ShellOutput> {
        self.use_screen(Recorded::Shell(command.to_string()))?;
        // In file-system mode the one command the app itself sends to remove a file takes the file out.
        if let Some(path) = command.strip_prefix("rm -f -- '").and_then(|rest| rest.strip_suffix('\'')) {
            let mut state = self.state.lock();
            if state.file_system {
                state.files.remove(path);
            }
        }
        let mut state = self.state.lock();
        if !state.shell_queue.is_empty() {
            return Ok(state.shell_queue.remove(0));
        }
        Ok(ShellOutput { code: Some(0), stdout: "fake shell".to_string(), stderr: String::new(), timed_out: false })
    }

    fn read_file(&self, path: &str, max: usize) -> SbxResult<Vec<u8>> {
        self.use_screen(Recorded::ReadFile(path.to_string()))?;
        let state = self.state.lock();
        if state.file_system {
            return match state.files.get(path) {
                Some(bytes) => Ok(bytes.iter().take(max).copied().collect()),
                None => Err(SandboxError::new("SANDBOX_NO_FILE", format!("{path} could not be read."))),
            };
        }
        Ok(b"fake file".to_vec())
    }

    fn fetch_file(&self, path: &str, max: usize) -> SbxResult<Vec<u8>> {
        self.use_screen(Recorded::FetchFile(path.to_string()))?;
        let state = self.state.lock();
        match state.files.get(path) {
            Some(bytes) if bytes.len() <= max => Ok(bytes.clone()),
            Some(_) => Err(SandboxError::new("SANDBOX_FILE_TOO_BIG", "That file is bigger than 10 MB, so it cannot be sent. Zip it or split it into parts, then send those.")),
            None => Err(SandboxError::new("SANDBOX_NO_FILE", "There is no file at that path. Check the name with list_files.")),
        }
    }

    fn fetch_page_hop(&self, url: &str, _max: usize) -> SbxResult<PageHop> {
        self.use_screen(Recorded::FetchPageHop(url.to_string()))?;
        let mut state = self.state.lock();
        if state.page_hops.is_empty() {
            return Ok(PageHop { content_type: "text/plain".to_string(), bytes: Vec::new(), redirect: None });
        }
        Ok(state.page_hops.remove(0))
    }

    fn files_run(&self, args: &[String], stdin: Option<Vec<u8>>) -> SbxResult<files_script::FilesOutput> {
        self.use_screen(Recorded::FilesRun(args.to_vec()))?;
        let hook = self.state.lock().files_hook.clone();
        match hook {
            Some(hook) => Ok(hook(args, stdin)),
            None => Err(SandboxError::new("SANDBOX_FILES_UNSUPPORTED", "This sandbox cannot browse files.")),
        }
    }

    fn write_file(&self, path: &str, bytes: &[u8], append: bool) -> SbxResult<()> {
        self.use_screen(Recorded::WriteFile(path.to_string(), bytes.len()))?;
        let mut state = self.state.lock();
        if state.file_system {
            let file = state.files.entry(path.to_string()).or_default();
            if !append {
                file.clear();
            }
            file.extend_from_slice(bytes);
        }
        Ok(())
    }

    fn screen_target(&self) -> SbxResult<ScreenTarget> {
        Ok(ScreenTarget { view_port: lifecycle::DEFAULT_VIEW_PORT, generation: self.state.lock().generation, passwords: None })
    }

    fn cancel_install(&self) {
        self.state.lock().cancelled = true;
    }

    fn linux_feature(&self) -> bool {
        !self.state.lock().no_linux_feature
    }

    fn set_idle_minutes(&self, minutes: u32) -> u32 {
        let used = lifecycle::clamp_idle_minutes(minutes);
        self.state.lock().idle_minutes = Some(used);
        used
    }
}

#[cfg(test)]
mod browser_tests;
#[cfg(test)]
mod control_tests;
#[cfg(test)]
mod detect_tests;
#[cfg(test)]
mod egress_chain_tests;
#[cfg(test)]
pub(crate) mod fakes_tests;
#[cfg(test)]
mod guard_tests;
#[cfg(test)]
mod image_tests;
#[cfg(test)]
mod install_tests;
#[cfg(test)]
mod live;
#[cfg(test)]
mod lease_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod manager_tests;
#[cfg(test)]
mod sandbox_tests;
#[cfg(test)]
mod screen_tests;
#[cfg(test)]
mod secret_hub_tests;
#[cfg(test)]
mod wsl_tests;

#[cfg(test)]
mod resume_tests;
