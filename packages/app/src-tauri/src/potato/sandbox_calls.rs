//! The bot's computer, as the brain's tools use it: `computer_observe`, `computer_act`, `browser_navigate`,
//! `browser_snapshot`, `browser_act` and `ask_person_for_screen`. The brain sends a `sandbox_call` event; this file decides whether it
//! may run, runs it, and builds the answer (`sandbox_result`). The engine (`service.rs`) calls it on its own thread.
//!
//! The order of the checks, and why. Everything before the sandbox is touched is cheap and cannot start anything:
//!
//! 1. Potato Bot is on and the Sandbox Labs switch is on, else `POTATO_OFF` / `SANDBOX_OFF`;
//! 2. the rules (`rules::tool_access`): the trust of the turn, set by Puppet Master when it sent the message,
//!    may use the sandbox. Anyone may; only the person may also use local helpers (that is `helpers`' check);
//! 3. the tool is one of the five, and the arguments are valid (`validate`): sizes, counts, kinds, coordinates,
//!    web addresses (`sandbox/browser.rs`). A refusal here costs nothing;
//! 4. a sandbox is installed and wired in (`Port`), else `SANDBOX_NOT_SET_UP`;
//! 4b. the hard risk gate (`risk.rs`, `cards.rs`), for `run_terminal` only: a command that touches the risky list does not run
//!    unless the owner allowed exactly it (once), or its class for the running job. Otherwise it makes a card and the call is
//!    refused (`APPROVAL_PENDING`); outside the private chat it is refused with "Ask me in our private chat". It sits after the
//!    checks above and before the lease and the cap, and it only reads the command text, which the host holds, never a flag the
//!    model could set. It is a net, not a wall: a script file or a string built at run time can hide a command, so the
//!    sandbox stays the real boundary;
//! 5. the screen lease. The bot holds it or takes it. The person holding it is `LEASE_HELD_BY_PERSON` (with
//!    `needsYou: true`: say so and wait). A lease that was taken, ran out or was handed back since the last call
//!    is `LEASE_LOST` once ("look again before you act") and the next call goes on with a new one;
//! 6. the day's action cap (`cost::spend_sandbox_actions`, a setting, default 500). A call that would pass the
//!    cap is refused whole and not counted;
//! 7. the call itself, one at a time (the screen has one pointer). It starts the sandbox if it is stopped.
//!
//! `ask_person_for_screen {reason}` is the sixth tool. It is not a screen action: it costs nothing against the cap and runs
//! no program. It raises the "Needs you" card (Take over / Skip, 30 minutes) on the Computer panel, for a bot that is stuck
//! (a login, a code, a check it cannot pass). If the person already has the screen it says so and raises nothing.
//!
//! What goes back is plain JSON the brain can read: window title, pointer, a frame id, whether the screen changed
//! since the last look, and, for a model that can see, the picture. For a model that cannot (`image: false` in the
//! call), `computer_observe` returns words: the page text of the front Chrome tab when there is one, else a short
//! note. Every string in an answer is redacted, and a page's words are data: the brain fences them.
//!
//! Nothing here starts a process or builds a `wsl.exe` command; the sandbox does (`sandbox/wsl.rs`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use parking_lot::Mutex;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::protocol::{Trust, MAX_SANDBOX_ARGS_BYTES};
use super::bot_settings;
use super::cards;
use super::checks;
use super::jobs;
use super::risk;
use super::rules;
use super::sandbox::browser::{self, BrowserError};
use super::sandbox::control::{self, Action, Button, Direction};
use super::sandbox::lease::{Holder, LeaseError, NeedsYou, ScreenLease};
use super::mcp;
use super::artifacts;
use super::sandbox::{shell_quote, FileRange, Observation, PageFetch, Sandbox, SandboxError};
use super::service::Host;
use super::rooms::store::{self as rstore, RoomFrom, RoomLine, Speaker};
use super::store::{self, FileCard, FromKind, Message, MsgKind};

#[cfg_attr(not(test), allow(dead_code))]
pub const TOOL_NAMES: [&str; 19] = [
    "computer_observe",
    "computer_act",
    "browser_navigate",
    "browser_snapshot",
    "browser_act",
    "web_fetch",
    "ask_person_for_screen",
    "run_terminal",
    "read_file",
    "write_file",
    "edit_file",
    "list_files",
    "set_wallpaper",
    "mcp_list_tools",
    "mcp_call_tool",
    "send_file",
    "app_observe",
    "app_act",
    "task",
];
/// The longest description a `send_file` card carries (the card keeps it short).
pub const MAX_SEND_DESCRIPTION_CHARS: usize = 300;
/// The longest reason a bot gives when it asks the person for the screen (the card keeps 200 of it).
pub const MAX_ASK_REASON_CHARS: usize = 300;
/// Most actions in one `computer_act` or `browser_act` call.
pub const MAX_ACTIONS: usize = 24;
/// A picture bigger than this (PNG bytes) is not sent; the answer says so and gives the words instead.
pub const MAX_IMAGE_BYTES: usize = 1_500_000;
/// The wait before a read-only call is tried again after the screen did not answer.
const READ_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(500);
/// The longest a window title is kept in an answer.
const MAX_TITLE_CHARS: usize = 200;
const MAX_COORD: f64 = 16_384.0;
const MAX_WAIT_MS: f64 = 5000.0;
/// Most bytes a `read_file` may read or a `write_file` may write in one call.
pub const MAX_FILE_BYTES: usize = 512 * 1024;
/// Most characters of a terminal command.
pub const MAX_COMMAND_CHARS: usize = 16 * 1024;
/// The terminal's default and longest time limits (milliseconds).
pub const DEFAULT_TERMINAL_MS: u64 = 30_000;
pub const MAX_TERMINAL_MS: u64 = 120_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    ComputerObserve,
    ComputerAct,
    BrowserNavigate,
    BrowserSnapshot,
    BrowserAct,
    WebFetch,
    AskPersonForScreen,
    SendFile,
    RunTerminal,
    ReadFile,
    WriteFile,
    EditFile,
    ListFiles,
    SetWallpaper,
    McpListTools,
    McpCallTool,
    /// A lent Windows app (`lend_tools.rs`): runs on the owner's PC, not in the sandbox.
    AppObserve,
    AppAct,
    /// Background tasks owned by the computer's control service (start, status, tail, stop).
    Task,
}

impl Tool {
    /// The lent-app tools do not use the sandbox: no VM, no screen lease, no Sandbox switch.
    pub fn is_lent_app(self) -> bool {
        matches!(self, Tool::AppObserve | Tool::AppAct)
    }

    /// Does the answer to this tool carry text from outside the chat (a page, a file, a command's output, a tool server, the
    /// screen)? A turn that called one cannot change the bot's own settings (`service/self_flow.rs`). Writing a file, sending
    /// one, listing a folder and asking the person to look do not bring text in.
    pub fn reads_outside(self) -> bool {
        !matches!(self, Tool::WriteFile | Tool::EditFile | Tool::SendFile | Tool::ListFiles | Tool::AskPersonForScreen | Tool::SetWallpaper)
    }

    pub fn parse(name: &str) -> Option<Tool> {
        Some(match name {
            "computer_observe" => Tool::ComputerObserve,
            "computer_act" => Tool::ComputerAct,
            "browser_navigate" => Tool::BrowserNavigate,
            "browser_snapshot" => Tool::BrowserSnapshot,
            "browser_act" => Tool::BrowserAct,
            "web_fetch" => Tool::WebFetch,
            "ask_person_for_screen" => Tool::AskPersonForScreen,
            "send_file" => Tool::SendFile,
            "run_terminal" => Tool::RunTerminal,
            "read_file" => Tool::ReadFile,
            "write_file" => Tool::WriteFile,
            "edit_file" => Tool::EditFile,
            "list_files" => Tool::ListFiles,
            "set_wallpaper" => Tool::SetWallpaper,
            "mcp_list_tools" => Tool::McpListTools,
            "mcp_call_tool" => Tool::McpCallTool,
            "app_observe" => Tool::AppObserve,
            "app_act" => Tool::AppAct,
            "task" => Tool::Task,
            _ => return None,
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn name(self) -> &'static str {
        match self {
            Tool::ComputerObserve => "computer_observe",
            Tool::ComputerAct => "computer_act",
            Tool::BrowserNavigate => "browser_navigate",
            Tool::BrowserSnapshot => "browser_snapshot",
            Tool::BrowserAct => "browser_act",
            Tool::WebFetch => "web_fetch",
            Tool::AskPersonForScreen => "ask_person_for_screen",
            Tool::SendFile => "send_file",
            Tool::RunTerminal => "run_terminal",
            Tool::ReadFile => "read_file",
            Tool::WriteFile => "write_file",
            Tool::EditFile => "edit_file",
            Tool::ListFiles => "list_files",
            Tool::SetWallpaper => "set_wallpaper",
            Tool::McpListTools => "mcp_list_tools",
            Tool::McpCallTool => "mcp_call_tool",
            Tool::AppObserve => "app_observe",
            Tool::AppAct => "app_act",
            Tool::Task => "task",
        }
    }
}

/// An error result: a short code, one plain sentence, and whether the person has the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallError {
    pub code: &'static str,
    pub message: String,
    pub needs_you: bool,
}

impl CallError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), needs_you: false }
    }
}

impl From<BrowserError> for CallError {
    fn from(e: BrowserError) -> Self {
        CallError::new(e.code(), e.message())
    }
}

impl From<SandboxError> for CallError {
    fn from(e: SandboxError) -> Self {
        CallError::new(e.code, e.message)
    }
}

impl From<control::ControlError> for CallError {
    fn from(e: control::ControlError) -> Self {
        CallError::new("BAD_ACTIONS", e.message())
    }
}

fn bad(message: impl Into<String>) -> CallError {
    CallError::new("BAD_ARGS", message)
}

/// What a valid call asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Observe,
    /// `show`: the answer carries the screen after the actions.
    Act { actions: Vec<Action>, show: bool },
    Navigate { url: String },
    /// One public page read as text (curl inside the sandbox). Owner turns only; read only, so a retry is safe.
    WebFetch { url: String },
    Snapshot,
    /// Already rebuilt by `browser::clean_actions`.
    BrowserAct { actions: Vec<Value> },
    /// Not a screen action: the "Needs you" card. `reason` is trimmed and not empty.
    AskPerson { reason: String },
    Terminal { command: String, cwd: Option<String>, timeout_ms: u64 },
    /// `offset`: start at this byte. `tail_bytes`: read the END of the file instead (this many bytes). Never both.
    ReadFile { path: String, offset: Option<u64>, tail_bytes: Option<usize> },
    /// A background task: start, look at, read the end of, or stop a program that keeps running between turns.
    Task(TaskCall),
    /// A file from the sandbox into the owner's chat as a card. `name` is the label (no folder part).
    SendFile { path: String, name: String, description: Option<String> },
    WriteFile { path: String, content: String, append: bool },
    EditFile { path: String, old: String, new: String, all: bool },
    ListFiles { path: String },
    Wallpaper { target: WallpaperTarget },
    /// The MCP tools the guest offers. Read only: no action is counted.
    McpList,
    /// One MCP tool call. Counts as one action; never repeated (it may already have run).
    McpCall { server: String, tool: String, arguments: Value },
    /// A lent app: its controls (and a picture). Read only.
    AppObserve { app: Option<String>, image: bool },
    /// One input to a lent app.
    AppAct { app: String, action: super::lend_tools::AppAction },
}

/// One call to the bot's background tasks. A program is an argv list and a folder, never a shell string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskCall {
    Start { argv: Vec<String>, cwd: String, label: Option<String> },
    /// One task, or (no id) all of them.
    Status { id: Option<String> },
    Tail { id: String, bytes: Option<u32> },
    Stop { id: String },
}

/// Most words in a task's argv and most characters in one word (the control service checks the same numbers).
pub const MAX_TASK_ARGS: usize = 32;
pub const MAX_TASK_ARG_CHARS: usize = 16 * 1024;
const MAX_TASK_LABEL_CHARS: usize = 80;
const MAX_TASK_TAIL_BYTES: f64 = 65_536.0;

impl TaskCall {
    /// The control service's route name and JSON body for this call.
    pub fn wire(&self) -> (&'static str, Value) {
        match self {
            TaskCall::Start { argv, cwd, label } => {
                let mut body = json!({ "argv": argv, "cwd": cwd });
                if let Some(label) = label {
                    body["label"] = json!(label);
                }
                ("start", body)
            }
            TaskCall::Status { id: Some(id) } => ("status", json!({ "id": id })),
            TaskCall::Status { id: None } => ("status", json!({})),
            TaskCall::Tail { id, bytes } => {
                let mut body = json!({ "id": id });
                if let Some(bytes) = bytes {
                    body["bytes"] = json!(bytes);
                }
                ("tail", body)
            }
            TaskCall::Stop { id } => ("stop", json!({ "id": id })),
        }
    }
}

/// What a task `status` or `tail` is called in the answer cache: bot, call, task and size. None for the calls that change things.
fn task_key(bot: &str, request: &Request) -> Option<String> {
    match request {
        Request::Task(TaskCall::Status { id }) => Some(format!("{bot}\u{1}status\u{1}{}", id.as_deref().unwrap_or("*"))),
        Request::Task(TaskCall::Tail { id, bytes }) => Some(format!("{bot}\u{1}tail\u{1}{id}\u{1}{}", bytes.unwrap_or(0))),
        _ => None,
    }
}

/// An argv as one command line, for the risk check and the approval card: a word is quoted only when it needs it.
fn argv_line(argv: &[String]) -> String {
    argv.iter()
        .map(|word| {
            let plain = !word.is_empty() && word.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./=:,+@%".contains(&b));
            if plain { word.clone() } else { shell_quote(word) }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn task_id(obj: &Map<String, Value>) -> Result<String, CallError> {
    let id = obj.get("id").and_then(Value::as_str).map(str::trim).unwrap_or("");
    let ok = id.len() == 10 && id.starts_with("t-") && id[2..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if ok { Ok(id.to_string()) } else { Err(bad("Give the task id the start call answered with (like t-1a2b3c4d).")) }
}

fn validate_task(obj: &Map<String, Value>) -> Result<Request, CallError> {
    let action = obj.get("action").and_then(Value::as_str).map(str::trim).unwrap_or("");
    Ok(Request::Task(match action {
        "start" => {
            if obj.contains_key("command") {
                return Err(bad("A task takes argv (a list of words), not a command string. To use a pipe or a redirect, write a script file and start [\"bash\", \"script.sh\"]."));
            }
            let words = obj.get("argv").and_then(Value::as_array).ok_or_else(|| bad("Give argv: the program and its arguments as a list of strings."))?;
            if words.is_empty() || words.len() > MAX_TASK_ARGS {
                return Err(bad(format!("argv must hold 1 to {MAX_TASK_ARGS} words.")));
            }
            let mut argv = Vec::with_capacity(words.len());
            for word in words {
                let Some(word) = word.as_str() else { return Err(bad("Every word of argv must be a string.")) };
                if word.chars().count() > MAX_TASK_ARG_CHARS || word.contains('\0') {
                    return Err(bad("A word of argv is too long or holds a NUL."));
                }
                argv.push(word.to_string());
            }
            if argv[0].trim().is_empty() {
                return Err(bad("argv[0] must be the program to run."));
            }
            let cwd = optional_guest_path(obj, "cwd")?.unwrap_or_else(|| "/home/pm".to_string());
            let label = obj
                .get("label")
                .and_then(Value::as_str)
                .map(|l| l.chars().filter(|c| !c.is_control()).take(MAX_TASK_LABEL_CHARS).collect::<String>())
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty());
            TaskCall::Start { argv, cwd, label }
        }
        "status" => {
            let id = match obj.get("id") {
                None | Some(Value::Null) => None,
                Some(_) => Some(task_id(obj)?),
            };
            TaskCall::Status { id }
        }
        "tail" => {
            let bytes = match number(obj, "bytes")? {
                None => None,
                Some(v) if (1.0..=MAX_TASK_TAIL_BYTES).contains(&v) => Some(v.floor() as u32),
                Some(_) => return Err(bad("bytes must be between 1 and 65536.")),
            };
            TaskCall::Tail { id: task_id(obj)?, bytes }
        }
        "stop" => TaskCall::Stop { id: task_id(obj)? },
        _ => return Err(bad("action must be start, status, tail or stop.")),
    }))
}

/// A wallpaper is a picture inside the sandbox or one plain colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WallpaperTarget {
    Picture(String),
    Colour(String),
}

impl Request {
    /// How many actions the call counts against the day's cap.
    pub fn cost(&self) -> u32 {
        match self {
            Request::Act { actions, .. } => actions.len() as u32,
            Request::BrowserAct { actions } => actions.len() as u32,
            Request::AskPerson { .. } | Request::McpList => 0,
            // Looking at a task costs nothing; `Calls::task_cached` is what stops a bot from asking over and over. Start and stop are actions.
            Request::Task(TaskCall::Status { .. } | TaskCall::Tail { .. }) => 0,
            _ => 1,
        }
    }
}

/// What one check found (see `Calls::run_check`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckRun {
    /// The condition is true: the command exited 0, or the page loaded (and held the words asked for).
    pub passed: bool,
    pub exit: Option<i32>,
    /// What it printed, redacted and cut: the end of a command's output, the start of a page.
    pub output: String,
    /// A short fingerprint of the whole result (`checks::fingerprint`), for "did it change".
    pub fingerprint: String,
}

impl CheckRun {
    /// The computer or the page did not answer: not passed, and the same fingerprint every time so it never reads as a change.
    fn unanswered(why: String) -> CheckRun {
        CheckRun { passed: false, exit: None, output: why, fingerprint: "unanswered".to_string() }
    }
}

/// Switches that can change at any time.
#[derive(Clone, Copy, Debug)]
pub struct Gates {
    /// Potato Bot itself (the Labs kill switch).
    pub bot_on: bool,
    /// The Sandbox Labs switch.
    pub sandbox_on: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// A web address inside the sandbox (a test page) may be opened. Only a test or a dev build can set it.
    pub allow_local_pages: bool,
}

impl Options {
    /// The dev switch: only a test or a `dev-fake-tui` build reads it; a release build is always strict.
    pub fn from_environment() -> Options {
        let dev = cfg!(any(test, feature = "dev-fake-tui"));
        Options { allow_local_pages: dev && std::env::var("PM_SBX_ALLOW_LOCAL_PAGES").is_ok_and(|v| v == "1") }
    }
}

// ---- reading the arguments -----------------------------------------------------------------------

fn number(obj: &Map<String, Value>, key: &str) -> Result<Option<f64>, CallError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_f64() {
            Some(n) if n.is_finite() => Ok(Some(n)),
            _ => Err(bad(format!("{key} must be a number."))),
        },
    }
}

fn coordinate(obj: &Map<String, Value>, key: &str, n: usize) -> Result<Option<i32>, CallError> {
    match number(obj, key)? {
        None => Ok(None),
        Some(v) if (0.0..=MAX_COORD).contains(&v) => Ok(Some(v.round() as i32)),
        Some(_) => Err(bad(format!("Action {n}: {key} is outside the screen."))),
    }
}

fn need_point(obj: &Map<String, Value>, n: usize, kind: &str) -> Result<(i32, i32), CallError> {
    match (coordinate(obj, "x", n)?, coordinate(obj, "y", n)?) {
        (Some(x), Some(y)) => Ok((x, y)),
        _ => Err(bad(format!("Action {n}: {kind} needs x and y."))),
    }
}

fn modifier(name: &str) -> Option<&'static str> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => "ctrl",
        "alt" | "option" => "alt",
        "shift" => "shift",
        "super" | "meta" | "win" | "windows" | "cmd" | "command" => "super",
        _ => return None,
    })
}

fn one_action(n: usize, value: &Value, pointer: Option<(i32, i32)>, opts: &Options) -> Result<Action, CallError> {
    let obj = value.as_object().ok_or_else(|| bad(format!("Action {n} must be an object.")))?;
    let kind = obj.get("kind").and_then(Value::as_str).unwrap_or("");
    Ok(match kind {
        "click" => {
            let (x, y) = need_point(obj, n, kind)?;
            let double = obj.get("double").and_then(Value::as_bool).unwrap_or(false);
            let button = match obj.get("button").and_then(Value::as_str).unwrap_or("left") {
                "left" => Button::Left,
                "right" => Button::Right,
                _ => return Err(bad(format!("Action {n}: button is left or right."))),
            };
            match (double, button) {
                (true, Button::Left) => Action::DoubleClick { x, y },
                (true, Button::Right) => return Err(bad(format!("Action {n}: a double click uses the left button."))),
                (false, button) => Action::Click { x, y, button },
            }
        }
        "move" => {
            let (x, y) = need_point(obj, n, kind)?;
            Action::Move { x, y }
        }
        "drag" => {
            let from = need_point(obj, n, kind)?;
            let (tx, ty) = (coordinate(obj, "to_x", n)?, coordinate(obj, "to_y", n)?);
            match (tx, ty) {
                (Some(tx), Some(ty)) => Action::Drag { from, to: (tx, ty) },
                _ => return Err(bad(format!("Action {n}: drag needs to_x and to_y."))),
            }
        }
        "down" | "up" => return Err(bad(format!("Action {n}: down and up are not supported. Use drag, with x, y, to_x and to_y."))),
        "type" => {
            let text = obj.get("text").and_then(Value::as_str).unwrap_or("");
            Action::Type { text: text.to_string() }
        }
        "key" => {
            let key = obj.get("key").and_then(Value::as_str).unwrap_or("").trim();
            if key.is_empty() {
                return Err(bad(format!("Action {n}: key needs a key name.")));
            }
            let mut parts: Vec<&str> = Vec::new();
            if let Some(mods) = obj.get("modifiers") {
                let list = mods.as_array().ok_or_else(|| bad(format!("Action {n}: modifiers is a list.")))?;
                for m in list {
                    let name = m.as_str().and_then(modifier).ok_or_else(|| bad(format!("Action {n}: a modifier is ctrl, alt, shift or super.")))?;
                    parts.push(name);
                }
            }
            parts.push(key);
            Action::Key { combo: parts.join("+") }
        }
        "scroll" => {
            let at = match (coordinate(obj, "x", n)?, coordinate(obj, "y", n)?) {
                (Some(x), Some(y)) => (x, y),
                (None, None) => pointer.ok_or_else(|| bad(format!("Action {n}: scroll needs x and y until you have looked at the screen once.")))?,
                _ => return Err(bad(format!("Action {n}: give both x and y, or neither."))),
            };
            let direction = match obj.get("direction").and_then(Value::as_str).unwrap_or("down") {
                "up" => Direction::Up,
                "down" => Direction::Down,
                "left" => Direction::Left,
                "right" => Direction::Right,
                _ => return Err(bad(format!("Action {n}: direction is up, down, left or right."))),
            };
            let amount = number(obj, "amount")?.unwrap_or(3.0).round().clamp(1.0, 20.0) as u32;
            Action::Scroll { x: at.0, y: at.1, direction, amount }
        }
        "wait" => {
            let ms = number(obj, "ms")?.unwrap_or(500.0).round().clamp(0.0, MAX_WAIT_MS) as u32;
            Action::Wait { ms }
        }
        "focus" => {
            let app = obj.get("application").and_then(Value::as_str).unwrap_or("").trim().to_ascii_lowercase();
            if app.is_empty() {
                return Err(bad(format!("Action {n}: focus needs an application (chrome, terminal, files, editor or paint).")));
            }
            let arg = match obj.get("uri").and_then(Value::as_str).map(str::trim).filter(|u| !u.is_empty()) {
                Some(uri) => Some(browser::check_launch_arg(uri, opts.allow_local_pages)?),
                None => None,
            };
            Action::Launch { app, arg }
        }
        _ => return Err(bad(format!("Action {n}: the kind must be click, move, drag, type, key, scroll, wait or focus."))),
    })
}

/// A path inside the sandbox. A relative path is relative to the sandbox user's home (`/home/pm`). `..` is refused so a
/// call cannot walk out of a folder by spelling it. The sandbox itself is the wall: this runs as the unprivileged user.
fn guest_path(obj: &Map<String, Value>, key: &str) -> Result<String, CallError> {
    let raw = obj.get(key).and_then(Value::as_str).map(str::trim).unwrap_or("");
    if raw.is_empty() {
        return Err(bad(format!("Give a {key}.")));
    }
    if raw.len() > 4096 || raw.contains('\0') {
        return Err(bad(format!("That {key} is not a usable path.")));
    }
    let path = if raw == "~" {
        "/home/pm".to_string()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        format!("/home/pm/{rest}")
    } else if raw.starts_with('/') {
        raw.to_string()
    } else {
        format!("/home/pm/{raw}")
    };
    if path.split('/').any(|part| part == "..") {
        return Err(bad(format!("That {key} has .. in it; give a plain path inside the sandbox.")));
    }
    Ok(path)
}

fn optional_guest_path(obj: &Map<String, Value>, key: &str) -> Result<Option<String>, CallError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(_) => Ok(Some(guest_path(obj, key)?)),
    }
}

fn image_path(path: &str) -> Result<String, CallError> {
    let lower = path.to_ascii_lowercase();
    let ok = [".png", ".jpg", ".jpeg", ".webp", ".bmp", ".gif"].iter().any(|ext| lower.ends_with(ext));
    if !ok {
        return Err(bad("The wallpaper picture must be a png, jpg, webp, bmp or gif file."));
    }
    Ok(path.to_string())
}

fn wallpaper_target(obj: &Map<String, Value>) -> Result<WallpaperTarget, CallError> {
    let path = optional_guest_path(obj, "path")?;
    let colour = obj.get("color").and_then(Value::as_str).map(str::trim).filter(|c| !c.is_empty());
    match (path, colour) {
        (Some(path), None) => Ok(WallpaperTarget::Picture(image_path(&path)?)),
        (None, Some(colour)) => {
            let hex = colour.strip_prefix('#').unwrap_or(colour);
            if (hex.len() != 6 && hex.len() != 3) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(bad("The colour is one hex value like #1b1b1f."));
            }
            let full = if hex.len() == 3 { hex.chars().flat_map(|c| [c, c]).collect::<String>() } else { hex.to_string() };
            Ok(WallpaperTarget::Colour(format!("#{}", full.to_ascii_lowercase())))
        }
        (Some(_), Some(_)) => Err(bad("Give a picture path or a colour, not both.")),
        (None, None) => Err(bad("Give the picture path, or a colour like #1b1b1f.")),
    }
}

/// Checks the arguments of one call. `pointer` is where the pointer was at the last look (for a scroll with no
/// position). A refusal costs nothing: nothing has been touched.
impl Request {
    /// Calls that only look or read: repeating one cannot change anything on the computer.
    pub fn is_read_only(&self) -> bool {
        matches!(self, Request::Observe | Request::Snapshot | Request::AppObserve { .. } | Request::ReadFile { .. } | Request::ListFiles { .. } | Request::McpList | Request::WebFetch { .. } | Request::Task(TaskCall::Status { .. } | TaskCall::Tail { .. }))
        // `SendFile` is not read-only: it posts a chat message, so it is never repeated.
    }
}

pub fn validate(tool: Tool, args: &Value, pointer: Option<(i32, i32)>, opts: &Options) -> Result<Request, CallError> {
    if args.to_string().len() > MAX_SANDBOX_ARGS_BYTES {
        return Err(bad("That is too much to send in one call. Split it into smaller calls."));
    }
    let obj = args.as_object().ok_or_else(|| bad("The arguments must be an object."))?;
    Ok(match tool {
        Tool::ComputerObserve => Request::Observe,
        Tool::BrowserSnapshot => Request::Snapshot,
        Tool::AppObserve => super::lend_tools::parse_observe(obj)?,
        Tool::AppAct => super::lend_tools::parse_act(obj)?,
        Tool::AskPersonForScreen => {
            let reason = obj.get("reason").and_then(Value::as_str).map(str::trim).unwrap_or("");
            if reason.is_empty() {
                return Err(bad("ask_person_for_screen needs a reason: one plain sentence saying what you need the person to do."));
            }
            if reason.chars().count() > MAX_ASK_REASON_CHARS {
                return Err(bad("Keep the reason to one or two short sentences."));
            }
            Request::AskPerson { reason: reason.to_string() }
        }
        Tool::WebFetch => {
            let url = obj.get("url").and_then(Value::as_str).ok_or_else(|| bad("web_fetch needs a url."))?;
            Request::WebFetch { url: browser::check_url(url, opts.allow_local_pages)? }
        }
        Tool::BrowserNavigate => {
            let url = obj.get("url").and_then(Value::as_str).ok_or_else(|| bad("browser_navigate needs a url."))?;
            Request::Navigate { url: browser::check_url(url, opts.allow_local_pages)? }
        }
        Tool::BrowserAct => {
            let actions = browser::clean_actions(obj.get("actions").unwrap_or(&Value::Null))?;
            // The tool's own limit, and the size of what will be sent, are checked now so a refusal costs nothing.
            browser::act_args(&actions)?;
            Request::BrowserAct { actions }
        }
        Tool::ComputerAct => {
            let list = obj.get("actions").and_then(Value::as_array).ok_or_else(|| bad("computer_act needs a list of actions."))?;
            if list.is_empty() || list.len() > MAX_ACTIONS {
                return Err(bad("Give 1 to 24 actions."));
            }
            let mut actions = Vec::with_capacity(list.len());
            let mut here = pointer;
            for (i, item) in list.iter().enumerate() {
                let action = one_action(i + 1, item, here, opts)?;
                match &action {
                    Action::Move { x, y } | Action::Click { x, y, .. } | Action::DoubleClick { x, y } | Action::Scroll { x, y, .. } => here = Some((*x, *y)),
                    Action::Drag { to, .. } => here = Some(*to),
                    _ => {}
                }
                actions.push(action);
            }
            // The exact shapes the sandbox's control service takes (text, key names, step count).
            control::plan(&actions)?;
            let show = obj.get("observe").and_then(Value::as_bool).unwrap_or(true);
            Request::Act { actions, show }
        }
        Tool::RunTerminal => {
            let command = obj.get("command").and_then(Value::as_str).map(str::trim).unwrap_or("");
            if command.is_empty() {
                return Err(bad("run_terminal needs a command."));
            }
            if command.chars().count() > MAX_COMMAND_CHARS {
                return Err(bad("That command is too long. Split it into smaller ones."));
            }
            let cwd = optional_guest_path(obj, "cwd")?;
            let timeout_ms = number(obj, "timeout_ms")?
                .unwrap_or(DEFAULT_TERMINAL_MS as f64)
                .round()
                .clamp(1000.0, MAX_TERMINAL_MS as f64) as u64;
            Request::Terminal { command: command.to_string(), cwd, timeout_ms }
        }
        Tool::ReadFile => {
            let path = guest_path(obj, "path")?;
            let offset = match number(obj, "offset")? {
                None => None,
                Some(v) if v >= 0.0 && v <= 9.0e15 => Some(v.floor() as u64),
                Some(_) => return Err(bad("offset must be zero or more.")),
            };
            let tail_bytes = match number(obj, "tail_bytes")? {
                None => None,
                Some(v) if v >= 1.0 => Some((v.floor().min(MAX_FILE_BYTES as f64)) as usize),
                Some(_) => return Err(bad("tail_bytes must be at least 1.")),
            };
            if offset.is_some() && tail_bytes.is_some() {
                return Err(bad("Give offset or tail_bytes, not both."));
            }
            Request::ReadFile { path, offset, tail_bytes }
        }
        Tool::SendFile => {
            let path = guest_path(obj, "path")?;
            // The label is the file's own name unless the bot gives one; any folder part is dropped.
            let given = obj.get("name").and_then(Value::as_str).map(str::trim).unwrap_or("");
            let base = if given.is_empty() { path.as_str() } else { given };
            let name = artifacts::clean_name(base);
            let description = obj
                .get("description")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .map(|d| d.chars().take(MAX_SEND_DESCRIPTION_CHARS).collect::<String>());
            Request::SendFile { path, name, description }
        }
        Tool::WriteFile => {
            let path = guest_path(obj, "path")?;
            let content = obj.get("content").and_then(Value::as_str).ok_or_else(|| bad("write_file needs content."))?;
            if content.len() > MAX_FILE_BYTES {
                return Err(bad("That file is too big to write in one call (512 KB at most)."));
            }
            let append = obj.get("append").and_then(Value::as_bool).unwrap_or(false);
            Request::WriteFile { path, content: content.to_string(), append }
        }
        Tool::EditFile => {
            let path = guest_path(obj, "path")?;
            let old = obj.get("old").and_then(Value::as_str).unwrap_or("");
            if old.is_empty() {
                return Err(bad("edit_file needs the exact text to replace (old)."));
            }
            let new = obj.get("new").and_then(Value::as_str).ok_or_else(|| bad("edit_file needs the replacement text (new)."))?;
            let all = obj.get("all").and_then(Value::as_bool).unwrap_or(false);
            Request::EditFile { path, old: old.to_string(), new: new.to_string(), all }
        }
        Tool::ListFiles => {
            let path = optional_guest_path(obj, "path")?.unwrap_or_else(|| "/home/pm".to_string());
            Request::ListFiles { path }
        }
        Tool::SetWallpaper => Request::Wallpaper { target: wallpaper_target(obj)? },
        Tool::McpListTools => Request::McpList,
        Tool::Task => return validate_task(obj),
        Tool::McpCallTool => {
            let server = obj.get("server").and_then(Value::as_str).map(str::trim).unwrap_or("");
            if !mcp::valid_server_name(server) {
                return Err(bad("mcp_call_tool needs a server name from mcp_list_tools (letters, digits, - and _, up to 32)."));
            }
            let tool = obj.get("tool").and_then(Value::as_str).map(str::trim).unwrap_or("");
            if tool.is_empty() || tool.chars().count() > mcp::MAX_TOOL_CHARS {
                return Err(bad("mcp_call_tool needs the tool's name as mcp_list_tools shows it."));
            }
            let arguments = match obj.get("arguments") {
                None | Some(Value::Null) => json!({}),
                Some(v @ Value::Object(_)) => v.clone(),
                Some(_) => return Err(bad("The tool's arguments must be a JSON object.")),
            };
            Request::McpCall { server: server.to_string(), tool: tool.to_string(), arguments }
        }
    })
}

// ---- the sandbox and the screen ------------------------------------------------------------------

/// The sandbox and the screen lease it shares with the Computer panel and the screen links.
#[derive(Clone)]
pub struct Port {
    pub sandbox: Arc<dyn Sandbox>,
    pub lease: Arc<Mutex<ScreenLease>>,
}

/// Who is asking. `trust` comes from the turn Puppet Master sent, never from the brain.
#[derive(Clone, Debug)]
pub struct Context {
    pub bot: String,
    pub conversation: String,
    pub trust: Trust,
    /// The model can look at a picture, so the answer may carry one.
    pub want_image: bool,
}

#[derive(Default)]
struct Run {
    lease_id: Option<String>,
    /// The bot's hold was lost (taken over, ran out) and it has not been told yet.
    stale: bool,
    frame: Option<String>,
    pointer: Option<(i32, i32)>,
}

pub struct Calls {
    pub(super) root: PathBuf,
    pub(super) host: Arc<dyn Host>,
    opts: Mutex<Options>,
    runs: Mutex<HashMap<String, Run>>,
    /// One call at a time: the screen has one pointer and one keyboard.
    pub(super) serial: Mutex<()>,
    /// True while the owner's sign-in card is filling a page (`login_calls.rs`): every call from the bot is refused.
    pub(super) login_busy: std::sync::atomic::AtomicBool,
    /// The last fresh answer to each task `status` / `tail` (when, and what), so a bot that polls gets the same answer back
    /// instead of a new look every time. Keyed by bot, call and task (`task_key`).
    task_seen: Mutex<HashMap<String, (i64, Value)>>,
}

/// A task `status` or `tail` gives a fresh answer at most this often per task; in between the last answer comes back.
pub const TASK_POLL_MS: i64 = 30_000;

/// A tool name that reads: it starts with get, list, read, search, fetch or find, and the word ends there (get_issue,
/// listFiles, search-docs). A tool named otherwise may change things. A name is only a name: this is a net, not a wall.
fn mcp_read_shaped(tool: &str) -> bool {
    let lower = tool.to_ascii_lowercase();
    ["get", "list", "read", "search", "fetch", "find"].iter().any(|word| {
        lower.strip_prefix(word).is_some_and(|rest| rest.chars().next().map_or(true, |c| !c.is_ascii_lowercase()) || tool.chars().nth(word.len()).is_some_and(|c| c.is_ascii_uppercase()))
    })
}

pub(super) fn run_id(bot: &str, conversation: &str) -> String {
    format!("{bot}@{conversation}")
}

impl Calls {
    /// A page inside the sandbox itself may be opened (a test or dev build only).
    pub fn allows_local_pages(&self) -> bool {
        self.opts.lock().allow_local_pages
    }

    pub fn new(root: PathBuf, host: Arc<dyn Host>) -> Calls {
        Calls { root, host, opts: Mutex::new(Options::from_environment()), runs: Mutex::new(HashMap::new()), serial: Mutex::new(()), login_busy: Default::default(), task_seen: Mutex::new(HashMap::new()) }
    }

    /// Runs one call and logs how long it took: the tool kind, the milliseconds and the outcome code. Never the arguments.
    /// `port` is `None` when no sandbox is wired in.
    pub fn execute(&self, port: Option<&Port>, gates: Gates, ctx: &Context, tool: &str, args: &Value) -> Result<Value, CallError> {
        let started = std::time::Instant::now();
        let result = self.run_call(port, gates, ctx, tool, args);
        let kind = Tool::parse(tool).map_or("unknown", Tool::name);
        let outcome = match &result {
            Ok(_) => "ok",
            Err(error) => error.code,
        };
        tracing::info!(kind, ms = started.elapsed().as_millis() as u64, outcome, "sandbox call");
        result
    }

    fn run_call(&self, port: Option<&Port>, gates: Gates, ctx: &Context, tool: &str, args: &Value) -> Result<Value, CallError> {
        if !gates.bot_on {
            return Err(CallError::new("POTATO_OFF", "Potato Bot is switched off."));
        }
        // A lent app is on the owner's PC: no VM, no Sandbox switch.
        if let Some(app_tool) = Tool::parse(tool).filter(|t| t.is_lent_app()) {
            return self.run_app_call(gates, ctx, app_tool, args);
        }
        if !gates.sandbox_on {
            return Err(CallError::new("SANDBOX_OFF", "The bot's computer is switched off. Turn on Sandbox in Settings > General > Labs."));
        }
        if !rules::tool_access(ctx.trust).sandbox {
            return Err(CallError::new("TOOL_NOT_ALLOWED", "This chat may not use the bot's computer."));
        }
        let Some(tool) = Tool::parse(tool) else {
            return Err(CallError::new("UNKNOWN_TOOL", "There is no such computer tool."));
        };
        // The owner is signing in on the screen right now: nothing of the bot's reaches the computer until it is done
        // (no look at the screen, no terminal, no page tool), so it cannot read the box or swap the page under the fill.
        if self.login_busy.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(CallError::new("LOGIN_IN_PROGRESS", "Your owner is signing in on the computer for a moment. Wait a few seconds, then try again."));
        }
        // Reading the web is for the owner only: a stranger's chat may use the screen, but not this.
        if tool == Tool::WebFetch && ctx.trust != Trust::OwnerPc && ctx.trust != Trust::OwnerRemote {
            return Err(CallError::new("TOOL_NOT_ALLOWED", "Only the owner may have the bot read a web page."));
        }
        let id = run_id(&ctx.bot, &ctx.conversation);
        let pointer = self.runs.lock().get(&id).and_then(|r| r.pointer);
        let opts = *self.opts.lock();
        let request = validate(tool, args, pointer, &opts)?;
        let Some(port) = port else {
            return Err(CallError::new("SANDBOX_NOT_SET_UP", "The bot's computer is not set up yet. Set it up in Settings > Potato Bot."));
        };
        // The owner's limit on sites (when they set one): the web tools open only the listed sites. Before the risk gate, the screen
        // and the day's cap, so a refused address costs nothing.
        self.site_gate(ctx, &request)?;
        let mcp_server = if let Request::McpCall { server, .. } = &request { Some(server.clone()) } else { None };
        if let (Some(server), Request::McpCall { tool, .. }) = (&mcp_server, &request) {
            self.connector_precheck(ctx, server)?;
            self.tool_list_gate(ctx, server, tool)?;
        }
        // The hard risk gate: a risky shell command becomes a card for the owner instead of running. After the switches,
        // the trust, the arguments and the sandbox check; before the screen is taken and the day's cap is spent.
        let once = self.risk_gate(ctx, &request)?;

        let now = self.host.now_ms();
        if let Request::AskPerson { reason } = &request {
            let mut answer = self.ask_person(port, ctx, &id, reason, now)?;
            redact(&mut answer);
            return Ok(answer);
        }
        // A server by link is called from this PC: it does not use the screen, so it neither takes the screen lease nor waits
        // for the one-call-at-a-time lock.
        let remote = match &request {
            Request::McpCall { server, .. } => bot_settings::load(&self.root, &ctx.bot).ok().and_then(|s| mcp::remote_of(&s, server)),
            _ => None,
        };
        // A status or tail asked again inside the poll window gets the last answer: no new look, no screen, no action.
        let task_key = task_key(&ctx.bot, &request);
        if let Some(key) = &task_key {
            if let Some(answer) = self.task_cached(key, now) {
                return Ok(answer);
            }
        }
        let task_changes = matches!(request, Request::Task(TaskCall::Start { .. } | TaskCall::Stop { .. }));
        if remote.is_none() {
            self.hold_screen(port, ctx, &id, now)?;
        }
        self.spend(ctx, now, request.cost())?;
        // The stored "allow once" is used up only now that the command is really going to run.
        if let Some(hash) = &once {
            if !cards::consume_once(&self.root, &ctx.bot, hash, self.host.now_ms()) {
                return Err(CallError::new("APPROVAL_PENDING", "The owner's approval for this command was already used, and it has not run. Ask again if you still need it."));
            }
        }
        if let (Some(server), Request::McpCall { tool, arguments, .. }) = (&remote, &request) {
            return self.remote_call(ctx, server, tool, arguments);
        }

        let _one_at_a_time = self.serial.lock();
        let answer = self.perform_retrying(port, ctx, &id, request)?;
        let mut answer = answer;
        // A connector's token never comes back in an answer, even from a tool that prints its environment.
        if let Some(server) = &mcp_server {
            if let Ok(settings) = bot_settings::load(&self.root, &ctx.bot) {
                mcp::scrub_answer(&ctx.bot, &settings, server, &mut answer);
            }
        }
        redact(&mut answer);
        if task_changes {
            // The tasks changed: the old answers are no longer true.
            let prefix = format!("{}\u{1}", ctx.bot);
            self.task_seen.lock().retain(|key, _| !key.starts_with(&prefix));
        }
        if let Some(key) = task_key {
            let mut seen = self.task_seen.lock();
            seen.retain(|_, (at, _)| now - *at < 10 * TASK_POLL_MS);
            seen.insert(key, (now, answer.clone()));
        }
        Ok(answer)
    }

    /// The last fresh answer to this task look, when it is still inside the poll window: marked `cached: true`, with how many
    /// seconds are left until a new look is allowed.
    fn task_cached(&self, key: &str, now: i64) -> Option<Value> {
        let seen = self.task_seen.lock();
        let (at, answer) = seen.get(key)?;
        let age = now - *at;
        if !(0..TASK_POLL_MS).contains(&age) {
            return None;
        }
        let mut answer = answer.clone();
        if let Some(map) = answer.as_object_mut() {
            map.insert("cached".to_string(), json!(true));
            map.insert("freshAnswerInSeconds".to_string(), json!(((TASK_POLL_MS - age) + 999) / 1000));
            map.insert("note".to_string(), json!("This is the answer from a moment ago. Do not ask again yet: do something else, or use wait_until."));
        }
        Some(answer)
    }

    /// The owner's site limit for this bot, read fresh from its settings file. A file that cannot be read is the strictest answer: the
    /// limit is taken as on with no site, so a damaged file never lets a web tool through.
    fn site_limit(&self, bot: &str) -> bot_settings::SiteLimit {
        match bot_settings::load(&self.root, bot) {
            Ok(settings) => settings.site_limit,
            Err(_) => bot_settings::SiteLimit { enabled: true, sites: Vec::new() },
        }
    }

    /// Refuses a web address the bot's site limit does not list. Covers `web_fetch`, `browser_navigate` and a web address opened
    /// with `computer_act` (`focus`). It does not see links clicked inside a page, the terminal, or the apps in the computer.
    fn site_gate(&self, ctx: &Context, request: &Request) -> Result<(), CallError> {
        let mut addresses: Vec<&str> = Vec::new();
        match request {
            Request::WebFetch { url } | Request::Navigate { url } => addresses.push(url),
            Request::Act { actions, .. } => {
                for action in actions {
                    if let Action::Launch { arg: Some(arg), .. } = action {
                        if browser::host_of(arg).is_some() {
                            addresses.push(arg);
                        }
                    }
                }
            }
            _ => return Ok(()),
        }
        if addresses.is_empty() {
            return Ok(());
        }
        let limit = self.site_limit(&ctx.bot);
        if !limit.enabled {
            return Ok(());
        }
        for address in addresses {
            let host = browser::host_of(address).unwrap_or_default();
            if !bot_settings::site_allowed(&limit, &host) {
                return Err(CallError::new("SITE_NOT_ALLOWED", bot_settings::site_refusal(&host)));
            }
        }
        Ok(())
    }

    /// `web_fetch`. With a site limit, the page is fetched one address at a time and every redirect is checked against the list
    /// before it is followed (a listed site cannot send the bot to an unlisted one). Without one, the plain fetch.
    fn web_fetch_for(&self, sandbox: &dyn Sandbox, ctx: &Context, url: &str) -> Result<Value, CallError> {
        let (final_url, page) = self.fetch_for_bot(sandbox, &ctx.bot, url)?;
        Ok(page_json(&final_url, &page))
    }

    /// One page for a bot: the plain fetch, or, with the bot's site limit on, one address at a time with every redirect checked.
    /// Returns the address the page was really read from.
    fn fetch_for_bot(&self, sandbox: &dyn Sandbox, bot: &str, url: &str) -> Result<(String, PageFetch), CallError> {
        let limit = self.site_limit(bot);
        if !limit.enabled {
            return Ok((url.to_string(), sandbox.fetch_page(url, MAX_PAGE_BYTES)?));
        }
        let allow_local = self.opts.lock().allow_local_pages;
        let mut current = url.to_string();
        for _ in 0..=MAX_FETCH_REDIRECTS {
            let host = browser::host_of(&current).unwrap_or_default();
            if !bot_settings::site_allowed(&limit, &host) {
                return Err(CallError::new("SITE_NOT_ALLOWED", bot_settings::site_refusal(&host)));
            }
            let hop = sandbox.fetch_page_hop(&current, MAX_PAGE_BYTES)?;
            match hop.redirect {
                Some(next) => current = browser::check_url(&next, allow_local)?,
                None => return Ok((current, PageFetch { content_type: hop.content_type, bytes: hop.bytes })),
            }
        }
        Err(CallError::new("SANDBOX_PAGE_REDIRECTS", "The page sent the bot round too many redirects."))
    }

    /// Runs one check (`checks.rs`) for a timer: a wait or a watching routine. Not a tool call: it takes no screen, spends nothing
    /// from the day's cap and makes no card, because a check only looks. The check is validated again here, and a web address goes
    /// through the bot's site limit like `web_fetch`. `Err` is a refusal that ends the wait or the watch (the check is not allowed
    /// any more); `Ok` with `passed: false` is a condition that is not true yet, or a page or computer that did not answer.
    pub fn run_check(&self, port: &Port, bot: &str, check: &checks::Check) -> Result<CheckRun, CallError> {
        if self.login_busy.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(CheckRun::unanswered("The owner is signing in on the computer right now.".to_string()));
        }
        let allow_local = self.opts.lock().allow_local_pages;
        let again = checks::parse(&check.text, check.contains.as_deref(), allow_local).map_err(|e| CallError::new("CHECK_REFUSED", e.message))?;
        if again.kind != check.kind {
            return Err(CallError::new("CHECK_REFUSED", "That check is not the kind it says it is."));
        }
        match again.kind {
            checks::CheckKind::Command => {
                let out = match port.sandbox.shell(&again.text, None, std::time::Duration::from_secs(checks::CHECK_TIMEOUT_SECS)) {
                    Ok(out) => out,
                    Err(e) => return Ok(CheckRun::unanswered(e.message)),
                };
                if out.timed_out {
                    return Ok(CheckRun::unanswered("The command took too long and was stopped.".to_string()));
                }
                let mut whole = out.stdout.clone();
                if !out.success() && !out.stderr.trim().is_empty() {
                    whole.push('\n');
                    whole.push_str(out.stderr.trim_end());
                }
                let fingerprint = checks::fingerprint(out.code, &out.stdout);
                let output = crate::redact::text(&checks::tail(whole.trim_end(), checks::MAX_OUTPUT_CHARS)).into_inner();
                Ok(CheckRun { passed: out.success(), exit: out.code, output, fingerprint })
            }
            checks::CheckKind::Url => {
                let url = browser::check_url(&again.text, allow_local)?;
                let host = browser::host_of(&url).unwrap_or_default();
                let limit = self.site_limit(bot);
                if !bot_settings::site_allowed(&limit, &host) {
                    return Err(CallError::new("SITE_NOT_ALLOWED", bot_settings::site_refusal(&host)));
                }
                match self.fetch_for_bot(port.sandbox.as_ref(), bot, &url) {
                    Ok((_, page)) => {
                        let body = String::from_utf8_lossy(&page.bytes).to_string();
                        let holds = again.contains.as_deref().map_or(true, |words| body.to_lowercase().contains(&words.to_lowercase()));
                        let fingerprint = checks::fingerprint(None, &body);
                        let head: String = body.trim().chars().take(checks::MAX_OUTPUT_CHARS).collect();
                        Ok(CheckRun { passed: holds, exit: None, output: crate::redact::text(&head).into_inner(), fingerprint })
                    }
                    Err(e) if matches!(e.code, "SITE_NOT_ALLOWED" | "URL_NOT_ALLOWED") => Err(e),
                    Err(e) => Ok(CheckRun::unanswered(e.message)),
                }
            }
        }
    }

    /// A connector (a service the owner connected with a token) is used only by the bot that has it switched on, only in
    /// the owner's own turn, and never in a room: a room holds several bots and lines written by other people.
    fn connector_precheck(&self, ctx: &Context, server: &str) -> Result<(), CallError> {
        let settings = bot_settings::load(&self.root, &ctx.bot).unwrap_or_default();
        let listed = mcp::catalog().iter().any(|e| e.connector && e.name == server);
        match mcp::connector_of(&settings, server) {
            Some(own) if own.enabled => {
                if !ctx.trust.is_owner() || store::app_room_of(&ctx.conversation).is_some() || store::app_space_of(&ctx.conversation).is_some() {
                    return Err(CallError::new("TOOL_NOT_ALLOWED", "A connector is for the owner's own chat with this bot, not for a group or someone else."));
                }
                Ok(())
            }
            // The bot's own plain server that happens to share a connector's name is an ordinary tool server.
            None if settings.mcp_servers.iter().any(|s| s.name == server) => Ok(()),
            _ if listed || settings.mcp_servers.iter().any(|s| s.name == server && s.connector) => Err(CallError::new(
                "CONNECTOR_NOT_CONNECTED",
                "This bot has no such connector switched on. Use request_connector to ask the owner to connect it in Settings > Potato Bot > Connectors. Never ask for a token in chat.",
            )),
            _ => Ok(()),
        }
    }

    /// The owner's list of tools for this server (Connectors > Tools): a tool it does not allow is refused here, in the host, before
    /// any card is made and before anything is sent, whatever tool name or bridge the call came through. Checked at call time
    /// against the settings file as it is now.
    fn tool_list_gate(&self, ctx: &Context, server: &str, tool: &str) -> Result<(), CallError> {
        let settings = bot_settings::load(&self.root, &ctx.bot).unwrap_or_default();
        match settings.mcp_servers.iter().find(|s| s.name == server) {
            Some(own) if !own.tool_allowed(tool) => Err(CallError::new("TOOL_NOT_ALLOWED", "Your owner switched this tool off for you. Do not look for another way to do the same thing.")),
            _ => Ok(()),
        }
    }

    /// What a call needs the owner's answer for, at the bot's level. `None`: it may run. A gated call is `(key, cwd,
    /// what the card shows, the card's subject, [(class, label)])`: the key is hashed for "allow once", the classes are
    /// what "allow for this job" remembers.
    #[allow(clippy::type_complexity)]
    fn needs_answer(&self, ctx: &Context, request: &Request) -> Option<(String, Option<String>, String, String, Vec<(String, String)>)> {
        let level = risk::level_for(&self.root, &ctx.bot);
        if level == risk::Level::Full {
            return None;
        }
        let ask_first = level == risk::Level::AskFirst;
        let change = || (risk::CHANGES.class.to_string(), risk::CHANGES.label.to_string());
        match request {
            Request::Terminal { command, cwd, .. } => {
                let mut needs: Vec<(String, String)> = risk::classify(command, cwd.as_deref()).iter().map(|r| (r.class.to_string(), r.label.to_string())).collect();
                if ask_first && !risk::read_only(command) {
                    needs.push(change());
                }
                (!needs.is_empty()).then(|| (command.clone(), cwd.clone(), command.clone(), "This command".to_string(), needs))
            }
            Request::McpCall { server, tool, arguments } if mcp_read_shaped(tool) => {
                // A read through a connector reads the owner's real account (mail, issues, pages), so it asks too, unless the
                // owner turned on Always allow reads for that connector.
                let settings = bot_settings::load(&self.root, &ctx.bot).unwrap_or_default();
                let own = mcp::connector_of(&settings, server).filter(|s| !s.always_allow_reads)?;
                let key = format!("mcp_call_tool {server} {tool} {arguments}");
                let needs = vec![("connected read".to_string(), format!("reads from your {} account", own.name))];
                Some((key, None, format!("Read with the tool {tool} on {server}"), "This read".to_string(), needs))
            }
            Request::McpCall { server, tool, arguments } if !mcp_read_shaped(tool) => {
                let key = format!("mcp_call_tool {server} {tool} {arguments}");
                let needs = vec![("connected tool".to_string(), "may change things or use a real account through a connected tool".to_string())];
                Some((key, None, format!("Use the tool {tool} on {server}"), "This tool call".to_string(), needs))
            }
            Request::Task(TaskCall::Start { argv, cwd, .. }) => {
                // A task is a command that outlives the turn: it gets the same check as a terminal command.
                let command = argv_line(argv);
                let mut needs: Vec<(String, String)> = risk::classify(&command, Some(cwd)).iter().map(|r| (r.class.to_string(), r.label.to_string())).collect();
                if ask_first && !risk::read_only(&command) {
                    needs.push(change());
                }
                (!needs.is_empty()).then(|| (command.clone(), Some(cwd.clone()), command, "This task".to_string(), needs))
            }
            Request::SendFile { path, name, .. } if ask_first => {
                let needs = vec![("send file".to_string(), "puts a file from its computer into the chat".to_string())];
                Some((format!("send_file {path} {name}"), None, format!("Send the file {name} from {path}"), "This".to_string(), needs))
            }
            Request::WriteFile { path, content, append } if ask_first => {
                let action = format!("{} the file {path}", if *append { "Add to" } else { "Write" });
                Some((format!("write_file {path} {append}\n{content}"), None, action, "This".to_string(), vec![change()]))
            }
            Request::EditFile { path, old, new, all } if ask_first => {
                Some((format!("edit_file {path} {all}\n{old}\n{new}"), None, format!("Edit the file {path}"), "This".to_string(), vec![change()]))
            }
            _ => None,
        }
    }

    /// The hard gate (P5, P6, the MCP gate). A call that the bot's level says needs the owner does not run on the model's
    /// say-so: it makes a card, and the call is refused with a plain line that tells the model to wait. The decision is
    /// made here, in the host, from what the call carries and the bot's level, after the switches, the trust and the
    /// arguments have passed. Nothing in the call, and nothing the model has written anywhere, can say "approved" or change
    /// the level: the only things that let a gated call through are a stored `allow_once` for exactly this call (hash), a
    /// stored `allow_job` for its classes in the running job, or the bot's level being Full. The first two are written only
    /// by the owner's answer (`cards.rs`), the level only by the app command.
    fn risk_gate(&self, ctx: &Context, request: &Request) -> Result<Option<String>, CallError> {
        let Some((key, cwd, action, subject, needs)) = self.needs_answer(ctx, request) else {
            return Ok(None);
        };
        // Only the owner, in the private chat, can answer a card. A group or a stranger's chat cannot.
        // Rule 10: a card never reaches a Space channel, and its answer could not be routed back to this turn.
        if store::app_space_of(&ctx.conversation).is_some() {
            return Err(CallError::new("APPROVAL_NOT_HERE", "Do this in your private chat with me."));
        }
        if ctx.conversation != store::app_conversation(&ctx.bot) || !ctx.trust.is_owner() {
            return Err(CallError::new("APPROVAL_NOT_HERE", "Ask me in our private chat"));
        }
        let now = self.host.now_ms();
        let hash = cards::command_hash(&key, cwd.as_deref());
        if cards::has_once(&self.root, &ctx.bot, &hash, now) {
            return Ok(Some(hash));
        }
        let classes: Vec<&str> = needs.iter().map(|(class, _)| class.as_str()).collect();
        if let Some(job) = jobs::load(&self.root, &ctx.bot).filter(|j| j.status == jobs::Status::Running) {
            if cards::job_allows(&self.root, &ctx.bot, &job.id, &classes) {
                return Ok(None);
            }
        }
        if cards::is_denied(&self.root, &ctx.bot, &hash, now) {
            return Err(CallError::new(
                "APPROVAL_DENIED",
                "The owner said no to this command. Do not run it again and do not look for another way to do the same thing. Do something else, or tell the owner what you could not do.",
            ));
        }
        // "Always allow": the owner said yes ahead of time to this bot's commands of exactly this class. Only a class that
        // may be always allowed counts (`risk::always_allowable`), and it was written only by the app command.
        if let [class] = classes.as_slice() {
            if bot_settings::always_allows(&self.root, &ctx.bot, &[*class]) {
                let home = self.host.profile(&ctx.bot).and_then(|p| p.folder);
                cards::note_auto_allowed(&self.root, home.as_deref(), &ctx.conversation, &action, class, now);
                self.host.changed(&ctx.bot);
                return Ok(None);
            }
        }
        let profile = self.host.profile(&ctx.bot);
        let name = profile.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| ctx.bot.clone());
        let home = profile.and_then(|p| p.folder);
        let what: Vec<&str> = needs.iter().map(|(_, label)| label.as_str()).collect();
        let new = cards::New {
            kind: cards::Kind::Approval,
            action,
            why: format!("{subject} {}.", what.join(" and ")),
            choices: Vec::new(),
            conversation: ctx.conversation.clone(),
            command_hash: Some(hash),
            classes: classes.iter().map(|c| c.to_string()).collect(),
            keep_going: None,
            connector: None,
            login: None,
            mcp_add: None,
            speaker: name,
            home,
        };
        match cards::raise(&self.root, &ctx.bot, new, now) {
            Ok(_) => {
                self.host.changed(&ctx.bot);
                Err(CallError::new(
                    "APPROVAL_PENDING",
                    "This command needs the owner's answer first, and it has not run. A card is waiting in the chat and your turn ends now. Do not run it again, do not try another way to do the same thing, and do not claim it was approved: the answer will come as your next message.",
                ))
            }
            Err(e) => Err(CallError::new("APPROVAL_UNAVAILABLE", format!("This command needs the owner's answer, and the card could not be made: {} It has not run.", e.message))),
        }
    }

    fn hold_screen(&self, port: &Port, ctx: &Context, id: &str, now: i64) -> Result<(), CallError> {
        let mut lease = port.lease.lock();
        let mut runs = self.runs.lock();
        let run = runs.entry(id.to_string()).or_default();
        let had = run.lease_id.take();
        if let Some(held) = &had {
            if lease.renew(held, now).is_ok() {
                run.lease_id = had;
                return Ok(());
            }
            run.stale = true;
        }
        match lease.take_for_bot(&ctx.bot, id, now) {
            Ok(new_id) => {
                run.lease_id = Some(new_id);
                if run.stale {
                    // The screen was used by someone else, or the hold ran out, since the last call.
                    run.stale = false;
                    run.frame = None;
                    return Err(CallError::new("LEASE_LOST", "The screen was used by someone else since your last call. Look at the screen again before you act."));
                }
                Ok(())
            }
            Err(LeaseError::PersonHasControl) => Err(CallError {
                code: "LEASE_HELD_BY_PERSON",
                message: "The person is using the screen right now. Tell them you are waiting, and try again after they hand it back.".to_string(),
                needs_you: true,
            }),
            Err(LeaseError::BusyWithBot(other)) => {
                // One shared computer: the second bot waits. Say who has it, by name, and that the wait is short.
                let who = self.host.profile(&other).map(|p| p.name).unwrap_or(other);
                Err(CallError::new("LEASE_BUSY_WITH_BOT", format!("The computer is busy with {who} right now. Wait a little, then try again. If it stays busy, tell the person you are waiting.")))
            }
            Err(other) => Err(CallError::new(other.code(), "The screen could not be taken.")),
        }
    }

    pub(super) fn spend(&self, ctx: &Context, now: i64, cost: u32) -> Result<(), CallError> {
        let cap = store::load_settings(&self.root).map(|s| s.sandbox_actions_per_day).unwrap_or(super::cost::DEFAULT_SANDBOX_ACTIONS);
        let day = self.host.local_day(now);
        match super::cost::spend_sandbox_actions(&self.root, &day, &ctx.bot, cost, cap) {
            Ok(spent) if spent.allowed => Ok(()),
            Ok(spent) => {
                // A running job remembers it: the app pauses the job and asks, instead of the bot giving up on its own.
                super::jobs::mark_actions_capped(&self.root, &ctx.bot, &day, spent.cap, now);
                Err(CallError::new(
                    "SANDBOX_DAILY_CAP",
                    format!("The bot has used {} of its {} computer actions for today. It can use the computer again tomorrow, or the person can raise the limit.", spent.used, spent.cap),
                ))
            }
            Err(_) => Err(CallError::new("SANDBOX_COUNT_FAILED", "The day's action count could not be saved, so nothing was run. Try the same call again in a moment.")),
        }
    }

    fn perform(&self, port: &Port, ctx: &Context, id: &str, request: Request) -> Result<Value, CallError> {
        let sandbox: &dyn Sandbox = &*port.sandbox;
        match request {
            Request::Observe => {
                let observation = sandbox.observe()?;
                let mut out = self.describe(id, &observation, ctx.want_image);
                if !ctx.want_image {
                    let page = front_page_text(sandbox, &observation);
                    let words = if page.is_some() { "The page text is in page." } else { "Use browser_navigate and browser_snapshot for web pages, and computer_act with focus to open an app." };
                    out.insert("note".into(), json!(format!("This model cannot see pictures, so you get words. {words}")));
                    if let Some(page) = page {
                        out.insert("page".into(), page);
                    }
                }
                Ok(Value::Object(out))
            }
            Request::Act { actions, show } => {
                let observation = sandbox.act(&actions)?;
                let mut out = self.describe(id, &observation, ctx.want_image && show);
                out.insert("done".into(), json!(actions.len()));
                if !show {
                    for key in ["image", "frameId", "changed", "width", "height", "pointer", "activeWindow"] {
                        out.remove(key);
                    }
                }
                Ok(Value::Object(out))
            }
            Request::Navigate { url } => Ok(navigate(sandbox, &url)?),
            Request::Snapshot => Ok(browser::run(sandbox, &browser::snapshot_args())?),
            Request::WebFetch { url } => self.web_fetch_for(sandbox, ctx, &url),
            Request::BrowserAct { actions } => Ok(browser::run(sandbox, &browser::act_args(&actions)?)?),
            Request::Terminal { command, cwd, timeout_ms } => {
                let out = sandbox.shell(&command, cwd.as_deref(), std::time::Duration::from_millis(timeout_ms))?;
                Ok(json!({
                    "ok": out.success(),
                    "exitCode": out.code,
                    "stdout": out.stdout,
                    "stderr": out.stderr,
                    "timedOut": out.timed_out,
                }))
            }
            Request::Task(call) => {
                let (op, body) = call.wire();
                Ok(sandbox.task_call(op, &body)?)
            }
            Request::SendFile { path, name, description } => self.send_file(port, ctx, &path, &name, description.as_deref()),
            Request::ReadFile { path, offset, tail_bytes } if offset.is_some() || tail_bytes.is_some() => {
                let range = match (offset, tail_bytes) {
                    (Some(o), _) => FileRange::From(o),
                    _ => FileRange::Tail,
                };
                let slice = sandbox.read_file_range(&path, range, tail_bytes.unwrap_or(MAX_FILE_BYTES))?;
                let start = slice.start(range);
                let end = start + slice.bytes.len() as u64;
                let text = String::from_utf8_lossy(&slice.bytes).to_string();
                Ok(json!({ "ok": true, "path": path, "bytes": slice.bytes.len(), "totalBytes": slice.total, "startByte": start, "endByte": end, "truncated": end < slice.total, "text": text }))
            }
            Request::ReadFile { path, .. } => {
                let bytes = sandbox.read_file(&path, MAX_FILE_BYTES)?;
                let truncated = bytes.len() >= MAX_FILE_BYTES;
                let text = String::from_utf8_lossy(&bytes).to_string();
                Ok(json!({ "ok": true, "path": path, "bytes": bytes.len(), "truncated": truncated, "text": text }))
            }
            Request::WriteFile { path, content, append } => {
                self.keep_old_file(sandbox, ctx, &path, "write");
                sandbox.write_file(&path, content.as_bytes(), append)?;
                Ok(json!({ "ok": true, "path": path, "bytes": content.len(), "append": append }))
            }
            Request::EditFile { path, old, new, all } => edit_file(sandbox, &path, &old, &new, all, &|| self.keep_old_file(sandbox, ctx, &path, "edit")),
            Request::ListFiles { path } => list_files(sandbox, &path),
            Request::Wallpaper { target } => {
                let script = wallpaper_script(&target);
                let out = sandbox.shell(&script, None, std::time::Duration::from_secs(30))?;
                if !out.success() {
                    let why = if out.stderr.trim().is_empty() { out.stdout } else { out.stderr };
                    return Err(CallError::new("WALLPAPER_FAILED", format!("The wallpaper was not changed: {}", why.trim().chars().take(300).collect::<String>())));
                }
                Ok(json!({
                    "ok": true,
                    "note": match &target {
                        WallpaperTarget::Picture(_) => "The desktop wallpaper is now that picture.",
                        WallpaperTarget::Colour(_) => "The desktop wallpaper is now that colour.",
                    }
                }))
            }
            // Answered before the screen is touched (`execute`); never reaches the sandbox.
            Request::AskPerson { .. } => Err(CallError::new("BAD_ARGS", "That is not a screen action.")),
            Request::AppObserve { .. } | Request::AppAct { .. } => Err(CallError::new("BAD_ARGS", "That is a lent-app tool, not a screen action.")),
            Request::McpList => {
                let mut reply = sandbox.mcp_tools()?;
                if let Value::Object(map) = &mut reply {
                    map.insert("ok".into(), json!(true));
                }
                self.add_remote_tools(ctx, &mut reply);
                if let Ok(settings) = bot_settings::load(&self.root, &ctx.bot) {
                    mcp::filter_listed_tools(&settings, &mut reply);
                }
                Ok(reply)
            }
            Request::McpCall { server, tool, arguments } => {
                let reply = sandbox.mcp_call(&server, &tool, &arguments)?;
                Ok(mcp::answer(&reply))
            }
        }
    }

    /// One call to a server by link, run on this PC. The token is read here and never leaves this function except in the
    /// request to the server's own address. An answer, an error and a note are scrubbed of its forms before the brain reads them.
    fn remote_call(&self, ctx: &Context, server: &bot_settings::McpServer, tool: &str, arguments: &Value) -> Result<Value, CallError> {
        let wire = mcp::wire_for(server);
        match mcp::call_remote(&ctx.bot, server, wire.as_ref(), tool, arguments) {
            Ok(reply) => {
                let mut answer = mcp::answer(&reply);
                if let Ok(settings) = bot_settings::load(&self.root, &ctx.bot) {
                    mcp::scrub_answer(&ctx.bot, &settings, &server.name, &mut answer);
                }
                redact(&mut answer);
                Ok(answer)
            }
            Err(e) => Err(CallError::new(e.code, mcp::remote_message(&ctx.bot, server, &e))),
        }
    }

    /// Adds the bot's servers by link to a tools list in the shape the computer's list has. A server that does not answer is
    /// shown as failed with a plain line. Only for the owner's own chat outside a room, like every connector.
    fn add_remote_tools(&self, ctx: &Context, reply: &mut Value) {
        let Ok(settings) = bot_settings::load(&self.root, &ctx.bot) else { return };
        if !ctx.trust.is_owner() || store::app_room_of(&ctx.conversation).is_some() || store::app_space_of(&ctx.conversation).is_some() {
            return;
        }
        let Some(list) = reply.get_mut("servers").and_then(Value::as_array_mut) else { return };
        for server in settings.mcp_servers.iter().filter(|s| s.enabled && !s.url.is_empty()) {
            let wire = mcp::wire_for(server);
            let entry = match mcp::list_remote(&ctx.bot, server, wire.as_ref()) {
                Ok(tools) => json!({
                    "name": server.name,
                    "status": "ready",
                    "tools": tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "inputSchema": t.input_schema })).collect::<Vec<_>>(),
                }),
                Err(e) => json!({ "name": server.name, "status": "failed", "error": mcp::remote_message(&ctx.bot, server, &e), "tools": [] }),
            };
            list.push(entry);
        }
    }

    /// While the bot has a job, keeps the old content of a file in its home before `write_file` or `edit_file` changes it, so
    /// the job can list it with Restore (`file_undo.rs`). Never stops the change: a problem is only logged.
    fn keep_old_file(&self, sandbox: &dyn Sandbox, ctx: &Context, path: &str, op: &str) {
        let Some(job) = super::jobs::load(&self.root, &ctx.bot).filter(|j| matches!(j.status, super::jobs::Status::Running | super::jobs::Status::Waiting)) else { return };
        super::file_undo::snapshot(&self.root, &ctx.bot, &job.id, sandbox, path, op, self.host.now_ms());
    }

    /// The read-only calls get one more try, after a short wait, when the screen did not answer. Actions, terminal
    /// commands and writes are never repeated: they could run twice.
    fn perform_retrying(&self, port: &Port, ctx: &Context, id: &str, request: Request) -> Result<Value, CallError> {
        if !request.is_read_only() {
            return self.perform(port, ctx, id, request);
        }
        match self.perform(port, ctx, id, request.clone()) {
            Err(error) if error.code == "SANDBOX_UNREACHABLE" => {
                std::thread::sleep(READ_RETRY_DELAY);
                self.perform(port, ctx, id, request)
            }
            other => other,
        }
    }

    /// `ask_person_for_screen`: the bot asks the person to step in. It must hold the screen for the card to exist, so it takes
    /// (or renews) its hold first; asking needs no fresh look, so a "the screen changed hands" notice is not an error here.
    /// `send_file`: a file from the sandbox goes into this chat as a card. The bytes are read whole (never decoded as
    /// text), kept with the artifacts (which refuse more than 10 MB), and a bot message points at them. The message
    /// text names the file too, so a surface that cannot show a card still shows the name.
    fn send_file(&self, port: &Port, ctx: &Context, path: &str, name: &str, description: Option<&str>) -> Result<Value, CallError> {
        let bytes = port.sandbox.fetch_file(path, artifacts::MAX_SAVE_BYTES)?;
        let now = self.host.now_ms();
        let saved = artifacts::put(&self.root, &ctx.bot, name, &bytes, now).map_err(|e| CallError::new(e.code, e.message))?;
        let profile = self.host.profile(&ctx.bot);
        let who = profile.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| ctx.bot.clone());
        let home = profile.and_then(|p| p.folder);
        let mut text = format!("Sent a file: {}", saved.name);
        if let Some(description) = description {
            text.push('\n');
            text.push_str(description);
        }
        let card = FileCard { id: saved.id.clone(), name: saved.name.clone(), kind: saved.kind, size: saved.size };
        // In a group the card goes into the group's own file, with the sending bot's id, so it opens with the right bot.
        if let Some(room_id) = store::app_room_of(&ctx.conversation) {
            let line = RoomLine {
                id: rstore::new_id('l'),
                from: RoomFrom { kind: Speaker::Bot, id: ctx.bot.clone(), name: who },
                text,
                at: now,
                hop: 1,
                in_reply_to: None,
                file: Some(card),
                handoff: None,
            };
            rstore::append_line(&self.root, room_id, line, home.as_deref()).map_err(|_| CallError::new("ROOM_SAVE_FAILED", "The file could not be put in the group. Try again."))?;
            self.host.room_changed(room_id);
            return Ok(json!({
                "ok": true,
                "name": saved.name,
                "size": saved.size,
                "note": "The file is now in the group as a card. Say so briefly in your reply; do not paste the file's contents.",
            }));
        }
        let mut message = Message::new(now, FromKind::Bot, Some(&who), MsgKind::Text, &text);
        message.file = Some(card);
        store::append_message(&self.root, &ctx.conversation, message, home.as_deref()).map_err(|e| CallError::new(e.code, e.message))?;
        Ok(json!({
            "ok": true,
            "name": saved.name,
            "size": saved.size,
            "note": "The file is now in the chat as a card. Say so briefly in your reply; do not paste the file's contents.",
        }))
    }

    fn ask_person(&self, port: &Port, ctx: &Context, id: &str, reason: &str, now: i64) -> Result<Value, CallError> {
        match self.hold_screen(port, ctx, id, now) {
            Ok(()) => {}
            Err(e) if e.code == "LEASE_LOST" => {}
            Err(e) if e.code == "LEASE_HELD_BY_PERSON" => {
                return Ok(json!({
                    "asked": false,
                    "note": "The person already has the screen, so there is nobody to ask. Wait for them to hand it back, and do not use the computer meanwhile.",
                }));
            }
            Err(e) => return Err(e),
        }
        let card = self.needs_you(port, &ctx.bot, &ctx.conversation, reason)?;
        Ok(json!({
            "asked": true,
            "cardId": card.card_id,
            "expiresInMinutes": (card.expires_ms - card.asked_ms) / 60_000,
            "note": "The person sees a card on the Computer panel with Take over and Skip. Tell them in one short line what you need, then stop using the computer until they answer.",
        }))
    }

    /// The part of an answer that is about the screen. Remembers the frame and the pointer for the run.
    pub(super) fn describe(&self, id: &str, obs: &Observation, with_image: bool) -> Map<String, Value> {
        let frame = format!("f-{}", hex8(&Sha256::digest(&obs.png)));
        let changed = {
            let mut runs = self.runs.lock();
            let run = runs.entry(id.to_string()).or_default();
            let changed = run.frame.as_ref().map(|last| *last != frame);
            run.frame = Some(frame.clone());
            if obs.pointer.is_some() {
                run.pointer = obs.pointer;
            }
            changed
        };
        let mut out = Map::new();
        out.insert("frameId".into(), json!(frame));
        out.insert("width".into(), json!(obs.width));
        out.insert("height".into(), json!(obs.height));
        if let Some((x, y)) = obs.pointer {
            out.insert("pointer".into(), json!({ "x": x, "y": y }));
        }
        if let Some(title) = &obs.active_window_title {
            let title: String = title.chars().filter(|c| !c.is_control()).take(MAX_TITLE_CHARS).collect();
            out.insert("activeWindow".into(), json!(title));
        }
        out.insert("changed".into(), changed.map_or(Value::Null, Value::Bool));
        if with_image {
            if obs.png.len() <= MAX_IMAGE_BYTES {
                let mime = picture_mime(&obs.png);
                out.insert("image".into(), json!({ "mime": mime, "data": base64::engine::general_purpose::STANDARD.encode(&obs.png) }));
            } else {
                out.insert("note".into(), json!("The picture was too big to send, even as a small JPEG, so you get the words only. Call computer_observe again, or browser_snapshot for a web page."));
            }
        }
        out
    }

    /// The run is over (the bot answered): its hold on the screen goes. The person's hold is not touched.
    pub fn end_run(&self, port: Option<&Port>, bot: &str, conversation: &str) {
        let id = run_id(bot, conversation);
        self.runs.lock().remove(&id);
        if let Some(port) = port {
            port.lease.lock().end_run(&id, self.host.now_ms());
        }
    }

    /// The bot's brain is gone: every run of that bot ends.
    pub fn end_bot(&self, port: Option<&Port>, bot: &str) {
        let prefix = format!("{bot}@");
        let ids: Vec<String> = self.runs.lock().keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
        for id in ids {
            self.runs.lock().remove(&id);
            if let Some(port) = port {
                port.lease.lock().end_run(&id, self.host.now_ms());
            }
        }
    }

    /// The bot, holding the screen, asks the person to step in (the "Needs you" card, 30 minutes). The Computer
    /// panel shows the card (Take over / Skip); the brain's `ask_person_for_screen` tool raises it.
    pub fn needs_you(&self, port: &Port, bot: &str, conversation: &str, reason: &str) -> Result<NeedsYou, CallError> {
        let id = run_id(bot, conversation);
        let held = self.runs.lock().get(&id).and_then(|r| r.lease_id.clone());
        let Some(held) = held else {
            return Err(CallError::new(LeaseError::NotTheHolder.code(), "The bot does not hold the screen."));
        };
        let now = self.host.now_ms();
        port.lease.lock().ask_for_help(&held, reason, now).map_err(|e| CallError::new(e.code(), "The bot cannot ask now."))
    }

    /// True when the lease table says this bot is using the screen right now (for the Computer panel's header).
    #[allow(dead_code)]
    pub fn bot_is_using(&self, port: &Port, bot: &str) -> bool {
        let now = self.host.now_ms();
        port.lease.lock().current(now).is_some_and(|l| matches!(&l.holder, Holder::Bot(b) if b == bot))
    }
}

/// How long a freshly started Chrome is given before the page tool looks for it again.
const BROWSER_START_WAIT_MS: u32 = 3000;

/// `edit_file`: read, replace (once or everywhere), write back. A text that is not in the file is a plain error.
fn edit_file(sandbox: &dyn Sandbox, path: &str, old: &str, new: &str, all: bool, before_write: &dyn Fn()) -> Result<Value, CallError> {
    let bytes = sandbox.read_file(path, MAX_FILE_BYTES)?;
    if bytes.len() >= MAX_FILE_BYTES {
        return Err(CallError::new("FILE_TOO_BIG", "That file is too big to edit in one call. Use read_file and write_file in pieces."));
    }
    let text = String::from_utf8(bytes).map_err(|_| CallError::new("FILE_NOT_TEXT", "That file is not plain text."))?;
    let found = text.matches(old).count();
    if found == 0 {
        return Err(CallError::new("EDIT_TEXT_NOT_FOUND", "That exact text is not in the file. Read it again and copy the text exactly."));
    }
    let updated = if all { text.replace(old, new) } else { text.replacen(old, new, 1) };
    let replacements = if all { found } else { 1 };
    before_write();
    sandbox.write_file(path, updated.as_bytes(), false)?;
    Ok(json!({ "ok": true, "path": path, "replacements": replacements, "bytes": updated.len() }))
}

/// `list_files`: the entries of one folder, two levels at most.
fn list_files(sandbox: &dyn Sandbox, path: &str) -> Result<Value, CallError> {
    let command = format!("find -- {} -maxdepth 2 -printf '%y %10s %P\\n' 2>/dev/null | sort", shell_quote(path));
    let out = sandbox.shell(&command, None, std::time::Duration::from_secs(20))?;
    if !out.success() {
        return Err(CallError::new("LIST_FAILED", format!("That folder could not be listed: {}", out.stderr.trim())));
    }
    let entries: Vec<&str> = out.stdout.lines().filter(|line| !line.trim().is_empty()).take(500).collect();
    Ok(json!({ "ok": true, "path": path, "entries": entries, "truncated": out.stdout.lines().count() > 500 }))
}

/// The shell script that changes the desktop wallpaper. It finds the desktop that is running (Cinnamon, MATE, GNOME or
/// XFCE) through `gsettings`/`xfconf`, and always falls back to `xsetroot` for a solid colour.
fn wallpaper_script(target: &WallpaperTarget) -> String {
    let prelude =
        "export DISPLAY=\"${DISPLAY:-:99}\"; [ -r \"$HOME/.pm-sbx/env\" ] && . \"$HOME/.pm-sbx/env\" >/dev/null 2>&1 || true; ";
    match target {
        WallpaperTarget::Picture(path) => {
            let uri = shell_quote(&format!("file://{path}"));
            let file = shell_quote(path);
            format!(
                "{prelude}URI={uri}; FILE={file}; \
if command -v gsettings >/dev/null 2>&1 && gsettings get org.cinnamon.desktop.background picture-uri >/dev/null 2>&1; then \
  gsettings set org.cinnamon.desktop.background picture-uri \"$URI\"; \
  gsettings set org.cinnamon.desktop.background picture-uri-dark \"$URI\" >/dev/null 2>&1 || true; \
  gsettings set org.cinnamon.desktop.background picture-options 'zoom' >/dev/null 2>&1 || true; \
elif command -v gsettings >/dev/null 2>&1 && gsettings get org.mate.background picture-filename >/dev/null 2>&1; then \
  gsettings set org.mate.background picture-filename \"$FILE\"; \
elif command -v gsettings >/dev/null 2>&1 && gsettings get org.gnome.desktop.background picture-uri >/dev/null 2>&1; then \
  gsettings set org.gnome.desktop.background picture-uri \"$URI\"; \
elif command -v xfconf-query >/dev/null 2>&1 && xfconf-query -c xfce4-desktop -l >/dev/null 2>&1; then \
  for p in $(xfconf-query -c xfce4-desktop -l | grep -E '/backdrop/.*/last-image$'); do xfconf-query -c xfce4-desktop -p \"$p\" -s \"$URI\"; done; \
  xfdesktop --reload >/dev/null 2>&1 || true; \
elif command -v feh >/dev/null 2>&1; then feh --bg-scale -- \"$FILE\"; \
elif command -v xwallpaper >/dev/null 2>&1; then xwallpaper --zoom -- \"$FILE\"; \
else echo 'No wallpaper tool was found in this desktop.' >&2; exit 3; fi"
            )
        }
        WallpaperTarget::Colour(colour) => {
            let hex = colour.trim_start_matches('#');
            let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
            let (r, g, b) = (channel(0), channel(2), channel(4));
            let colour = shell_quote(colour);
            format!(
                "{prelude}COLOUR={colour}; \
if command -v gsettings >/dev/null 2>&1 && gsettings get org.cinnamon.desktop.background primary-color >/dev/null 2>&1; then \
  gsettings set org.cinnamon.desktop.background picture-options 'none'; \
  gsettings set org.cinnamon.desktop.background primary-color \"$COLOUR\"; \
elif command -v gsettings >/dev/null 2>&1 && gsettings get org.mate.background primary-color >/dev/null 2>&1; then \
  gsettings set org.mate.background picture-filename ''; \
  gsettings set org.mate.background color-shading-type 'solid'; \
  gsettings set org.mate.background primary-color \"$COLOUR\"; \
elif command -v gsettings >/dev/null 2>&1 && gsettings get org.gnome.desktop.background primary-color >/dev/null 2>&1; then \
  gsettings set org.gnome.desktop.background picture-options 'none'; \
  gsettings set org.gnome.desktop.background primary-color \"$COLOUR\"; \
elif command -v xfconf-query >/dev/null 2>&1 && xfconf-query -c xfce4-desktop -l >/dev/null 2>&1; then \
  for p in $(xfconf-query -c xfce4-desktop -l | grep -E '/rgba1$'); do xfconf-query -c xfce4-desktop -p \"$p\" -t double -t double -t double -t double -s {r} -s {g} -s {b} -s 1 >/dev/null || true; done; \
  for p in $(xfconf-query -c xfce4-desktop -l | grep -E '/color-style$'); do xfconf-query -c xfce4-desktop -p \"$p\" -s 0 >/dev/null || true; done; \
fi; \
command -v xsetroot >/dev/null 2>&1 && xsetroot -solid \"$COLOUR\" >/dev/null 2>&1 || true"
            )
        }
    }
}

/// `browser_navigate`. The page tool talks to Chrome through its debug port, so a Chrome that is not up yet (the first
/// navigation of a fresh sandbox) is not a failure the bot has to work around: the app starts it (the launcher that
/// opens Chrome with that port), waits a moment, and asks again, once.
/// The most bytes of one page the bot reads (the brain keeps 20,000 characters of the text it makes from them).
pub const MAX_PAGE_BYTES: usize = 1024 * 1024;

/// The most redirects a limited fetch follows (the plain fetch follows five too).
const MAX_FETCH_REDIRECTS: usize = 5;

/// `web_fetch`: the page's bytes and its content type, as the sandbox fetched them. The brain makes the text.
fn page_json(url: &str, page: &PageFetch) -> Value {
    let truncated = page.bytes.len() >= MAX_PAGE_BYTES;
    let body = String::from_utf8_lossy(&page.bytes).to_string();
    json!({ "ok": true, "url": url, "contentType": page.content_type, "bytes": page.bytes.len(), "truncated": truncated, "body": body })
}

fn navigate(sandbox: &dyn Sandbox, url: &str) -> Result<Value, CallError> {
    let args = browser::navigate_args(url)?;
    let first = browser::run(sandbox, &args)?;
    if !browser_is_down(&first) {
        return Ok(first);
    }
    sandbox.act(&[Action::Launch { app: "chrome".to_string(), arg: None }, Action::Wait { ms: BROWSER_START_WAIT_MS }])?;
    Ok(browser::run(sandbox, &args)?)
}

/// The page tool's answer when there is no browser to talk to.
fn browser_is_down(reply: &Value) -> bool {
    reply.get("ok").and_then(Value::as_bool) == Some(false) && reply.get("error").and_then(Value::as_str).is_some_and(|e| e.starts_with("CDP not ready"))
}

/// The type of a captured picture, from its first bytes: the guest sends JPEG, and PNG when it cannot make one.
fn picture_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else {
        "image/png"
    }
}

/// Eight hex characters of a digest.
fn hex8(digest: &[u8]) -> String {
    digest.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// The words of the front Chrome tab, when the front window looks like one and the page tool answers.
pub(super) fn front_page_text(sandbox: &dyn Sandbox, obs: &Observation) -> Option<Value> {
    let title = obs.active_window_title.as_deref()?.to_ascii_lowercase();
    if !(title.contains("chrome") || title.contains("chromium")) {
        return None;
    }
    let reply = browser::run(sandbox, &browser::snapshot_args()).ok()?;
    (reply.get("ok").and_then(Value::as_bool) == Some(true)).then_some(reply)
}

/// Every string in an answer goes through the redactor (a page can print a key). The picture's bytes are left alone.
pub(super) fn redact(value: &mut Value) {
    match value {
        Value::String(s) => *s = crate::redact::text(s).into_inner(),
        Value::Array(items) => items.iter_mut().for_each(redact),
        Value::Object(map) => {
            for (key, inner) in map.iter_mut() {
                if key == "image" {
                    continue;
                }
                redact(inner);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
impl Calls {
    pub fn set_options(&self, opts: Options) {
        *self.opts.lock() = opts;
    }
}
