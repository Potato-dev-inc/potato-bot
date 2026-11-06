//! The IO of the brain process (spec sections 1.4 and 1.5): finding Node, starting the brain with an
//! empty environment and no window, reading its output, and stopping its whole tree. The decisions about
//! when to do that are in `keeper.rs` and `service.rs`; this file only does it.
//!
//! - Node is looked up with `ToolEnv { path: shell_env::path_for_spawn() }.resolve("node")`, then
//!   `node --version` (20 or newer) through `hidden_command` with a 3 second limit. The answer is cached
//!   for the launch while it is good, and re-asked at most every 30 seconds while it is not (so the person
//!   who installs Node does not have to restart the app).
//! - The process starts only through `proc_util::hidden_command`, so no window can appear.
//! - The environment is `BrainEnv` applied after `env_clear()`; stdin is null; stdout and stderr are read
//!   line by line into a sink (the log ring, which cleans every line).
//! - `PM_POTATO_BRAIN` replaces the script path, but only in a `dev-fake-tui` build (the pattern of
//!   `PM_E2E_PATH_PREPEND` in `shell_env.rs`).
//!
//! Tests start the same code with a fake `Launcher` or the fake brain script.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::env::BrainEnv;
use crate::worktrees::proc::ToolEnv;

pub const MIN_NODE_MAJOR: u32 = 20;
pub const VERSION_LIMIT: Duration = Duration::from_secs(3);
/// How long a bad answer about Node is trusted.
pub const RETRY_NODE_AFTER: Duration = Duration::from_secs(30);

/// What the PC has to run the brain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeState {
    Ready { path: PathBuf, major: u32 },
    /// No `node` on the path.
    Missing,
    /// There is one, but it is older than 20 (or it did not answer `--version`).
    TooOld { found: String },
}

impl NodeState {
    pub fn is_ready(&self) -> bool {
        matches!(self, NodeState::Ready { .. })
    }
}

/// The major number of `v22.20.0`, or `None` for anything else.
pub fn parse_major(version: &str) -> Option<u32> {
    let v = version.trim().trim_start_matches('v');
    v.split('.').next()?.parse().ok()
}

/// The state for a version answer.
pub fn judge(path: PathBuf, version: &str) -> NodeState {
    match parse_major(version) {
        Some(major) if major >= MIN_NODE_MAJOR => NodeState::Ready { path, major },
        _ => NodeState::TooOld { found: version.trim().chars().take(40).collect() },
    }
}

/// What a brain needs to start.
pub struct SpawnSpec {
    pub node: PathBuf,
    pub script: PathBuf,
    pub cwd: PathBuf,
    pub env: BrainEnv,
}

/// Where the brain's output lines go. Called from the reader threads.
pub type LineSink = Arc<dyn Fn(&str) + Send + Sync>;

/// A started brain process.
pub trait Child: Send {
    fn pid(&self) -> u32;
    /// The exit code once it has ended (`-1` when the system gave none), `None` while it runs.
    fn try_wait(&mut self) -> Option<i32>;
    /// Stops the process and everything it started, and waits for it.
    fn kill_tree(&mut self);
}

pub trait Launcher: Send + Sync {
    fn node(&self) -> NodeState;
    fn spawn(&self, spec: &SpawnSpec, sink: LineSink) -> Result<Box<dyn Child>, String>;
}

struct RealChild {
    child: std::process::Child,
}

impl Child for RealChild {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn try_wait(&mut self) -> Option<i32> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.code().unwrap_or(-1)),
            Ok(None) => None,
            Err(_) => Some(-1),
        }
    }

    fn kill_tree(&mut self) {
        crate::pty::proc_tree::kill_process_tree(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_lines<R: Read + Send + 'static>(stream: R, sink: LineSink) {
    let _ = std::thread::Builder::new().name("potato-brain-log".into()).spawn(move || {
        for line in BufReader::new(stream).lines() {
            match line {
                Ok(line) => sink(&line),
                Err(_) => break,
            }
        }
    });
}

/// The brain script to run: `PM_POTATO_BRAIN` in a dev build, else `brain.mjs` in the resource folder.
pub fn script_path(resources: Option<&Path>) -> Option<PathBuf> {
    #[cfg(feature = "dev-fake-tui")]
    if let Some(path) = std::env::var_os("PM_POTATO_BRAIN").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    // Never the extended-length spelling (a path that starts with two backslashes and a question mark):
    // node cannot start a script named like that.
    let script = super::resource_dirs::without_verbatim_prefix(&resources?.join("brain.mjs"));
    script.is_file().then_some(script)
}

/// True when `PM_POTATO_BRAIN` replaces the brain, which only a `dev-fake-tui` build allows. A stand-in brain
/// has no model to pay for, so the app asks it for no key (and never hands it the person's real one).
pub fn fake_brain_active() -> bool {
    #[cfg(feature = "dev-fake-tui")]
    {
        std::env::var_os("PM_POTATO_BRAIN").is_some_and(|p| !p.is_empty())
    }
    #[cfg(not(feature = "dev-fake-tui"))]
    {
        false
    }
}

static BUNDLED_NODE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();

/// Called once at start-up with the `node.exe` the installer put in the app's resources (`None` when there is none).
pub fn set_bundled_node(path: Option<PathBuf>) {
    let _ = BUNDLED_NODE.set(path);
}

/// The Node that ships inside the app. A debug build also takes the staged copy in the source tree.
pub fn bundled_node() -> Option<PathBuf> {
    let shipped = BUNDLED_NODE.get().cloned().flatten().filter(|p| p.is_file());
    #[cfg(debug_assertions)]
    let shipped = shipped.or_else(|| Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join("node").join("node.exe")).filter(|p| p.is_file()));
    shipped.map(|p| super::resource_dirs::without_verbatim_prefix(&p))
}

/// Expands `%NAME%` references with the values `lookup` gives (an unknown name stays as it is).
pub fn expand_vars(text: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => match lookup(&after[..end]) {
                Some(value) => {
                    out.push_str(&value);
                    rest = &after[end + 1..];
                }
                None => {
                    out.push('%');
                    rest = after;
                }
            },
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The value of a `reg query <key> /v Path` answer (`    Path    REG_EXPAND_SZ    C:;%X%`), expanded.
pub fn parse_reg_path(output: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    output.lines().find_map(|line| {
        let line = line.trim();
        let at = line.find("REG_EXPAND_SZ").map(|i| i + "REG_EXPAND_SZ".len()).or_else(|| line.find("REG_SZ").map(|i| i + "REG_SZ".len()))?;
        let value = line[at..].trim();
        (!value.is_empty()).then(|| expand_vars(value, lookup))
    })
}

/// The PATH Windows has now (the machine's and the user's, read from the registry), which a running app does not see
/// after an installer changes it. Empty off Windows or when the registry cannot be read.
pub fn fresh_windows_path() -> String {
    #[cfg(windows)]
    {
        let lookup = |name: &str| std::env::var(name).ok();
        let mut parts = Vec::new();
        for key in [r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment", r"HKCU\Environment"] {
            let mut command = crate::proc_util::hidden_command("reg.exe");
            command.arg("query").arg(key).arg("/v").arg("Path").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
            if let Ok(out) = command.output() {
                if let Some(path) = parse_reg_path(&String::from_utf8_lossy(&out.stdout), &lookup) {
                    parts.push(path);
                }
            }
        }
        parts.join(";")
    }
    #[cfg(not(windows))]
    {
        String::new()
    }
}

/// Where a Node installer puts `node.exe`, whatever the PATH says.
fn known_node_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(dir) = std::env::var_os(var) {
            out.push(PathBuf::from(dir).join("nodejs").join("node.exe"));
        }
    }
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        out.push(PathBuf::from(dir).join("Programs").join("nodejs").join("node.exe"));
    }
    out
}

/// Every `node` to try, best first: the one inside the app, the one the app's PATH finds, the one a fresh PATH finds,
/// the usual install folders. Same file twice is tried once.
fn node_candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut add = |p: Option<PathBuf>| {
        if let Some(p) = p.filter(|p| p.is_file()) {
            if !out.contains(&p) {
                out.push(p);
            }
        }
    };
    add(bundled_node());
    add(ToolEnv { path: Some(crate::shell_env::path_for_spawn().into()), extra: Vec::new() }.resolve("node"));
    let fresh = fresh_windows_path();
    if !fresh.is_empty() {
        add(ToolEnv { path: Some(fresh.into()), extra: Vec::new() }.resolve("node"));
    }
    for p in known_node_paths() {
        add(Some(p));
    }
    out
}

/// Looks for Node and asks its version, with a time limit. The Node inside the app wins; a Node on the PC is the
/// fallback, found even when it was installed after this app started.
pub fn probe_node() -> NodeState {
    let mut worst: Option<NodeState> = None;
    for path in node_candidates() {
        match ask_node_version(path) {
            ready @ NodeState::Ready { .. } => return ready,
            other => {
                worst.get_or_insert(other);
            }
        }
    }
    worst.unwrap_or(NodeState::Missing)
}

/// For the start-up report (`PM_SBX_DIAG_FILE`): which Node the brain would use, every Node it tried, and what the
/// bundled brain printed when that Node ran it with no settings (it says it is started by the app, then exits).
pub fn diag() -> serde_json::Value {
    let candidates: Vec<String> = node_candidates().iter().map(|p| p.display().to_string()).collect();
    let state = probe_node();
    let (node, major, brain_output) = match &state {
        NodeState::Ready { path, major } => {
            let script = std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(|d| d.join("resources").join("potato-brain").join("brain.mjs")))
                .filter(|s| s.is_file());
            let out = script.and_then(|script| {
                let mut command = crate::proc_util::hidden_command(path);
                command.arg(script).env_clear().stdin(Stdio::null());
                command.output().ok().map(|o| format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim().to_string())
            });
            (Some(path.display().to_string()), Some(*major), out)
        }
        _ => (None, None, None),
    };
    serde_json::json!({ "node": node, "node_major": major, "node_candidates": candidates, "bundled_node": bundled_node().map(|p| p.display().to_string()), "brain_output": brain_output })
}

fn ask_node_version(path: PathBuf) -> NodeState {
    let mut command = crate::proc_util::hidden_command(&path);
    command.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let Ok(mut child) = command.spawn() else { return NodeState::TooOld { found: String::new() } };
    let mut out = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(out) = out.as_mut() {
            let _ = out.take(256).read_to_string(&mut text);
        }
        text
    });
    let deadline = Instant::now() + VERSION_LIMIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                crate::pty::proc_tree::kill_process_tree(child.id());
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    let version = reader.join().unwrap_or_default();
    judge(path, &version)
}

/// The real launcher: Node from the PATH, the brain as a hidden child process.
#[derive(Default)]
pub struct RealLauncher {
    cache: Mutex<Option<(Instant, NodeState)>>,
}

impl RealLauncher {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Launcher for RealLauncher {
    fn node(&self) -> NodeState {
        let mut cache = self.cache.lock();
        if let Some((at, state)) = cache.as_ref() {
            if state.is_ready() || at.elapsed() < RETRY_NODE_AFTER {
                return state.clone();
            }
        }
        let state = probe_node();
        *cache = Some((Instant::now(), state.clone()));
        state
    }

    fn spawn(&self, spec: &SpawnSpec, sink: LineSink) -> Result<Box<dyn Child>, String> {
        let mut command = crate::proc_util::hidden_command(&spec.node);
        command.arg(&spec.script).current_dir(&spec.cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        spec.env.apply(&mut command);
        let mut child = command.spawn().map_err(|e| format!("could not start the brain: {e}"))?;
        if let Some(out) = child.stdout.take() {
            read_lines(out, sink.clone());
        }
        if let Some(err) = child.stderr.take() {
            read_lines(err, sink);
        }
        Ok(Box::new(RealChild { child }))
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    fn lookup(name: &str) -> Option<String> {
        match name {
            "SystemRoot" => Some(r"C:\Windows".to_string()),
            "ProgramFiles" => Some(r"C:\Program Files".to_string()),
            _ => None,
        }
    }

    #[test]
    fn percent_names_are_expanded_and_unknown_ones_stay() {
        assert_eq!(expand_vars(r"%SystemRoot%\system32;%NOPE%\x", &lookup), r"C:\Windows\system32;%NOPE%\x");
        assert_eq!(expand_vars("50% of it", &lookup), "50% of it");
        assert_eq!(expand_vars("%%", &lookup), "%%");
    }

    #[test]
    fn a_reg_query_answer_gives_the_path_a_new_node_install_added() {
        let answer = r"
HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Session Manager\Environment
    Path    REG_EXPAND_SZ    %SystemRoot%\system32;%ProgramFiles%\nodejs\

";
        assert_eq!(parse_reg_path(answer, &lookup).as_deref(), Some(r"C:\Windows\system32;C:\Program Files\nodejs\"));
        assert_eq!(parse_reg_path("ERROR: The system was unable to find the specified registry key or value.", &lookup), None);
        assert_eq!(parse_reg_path(r"    Path    REG_SZ    C:\a;C:\b", &lookup).as_deref(), Some(r"C:\a;C:\b"));
    }

    #[test]
    fn a_node_older_than_20_or_silent_is_not_ready_and_a_missing_one_is_missing() {
        assert!(judge(PathBuf::from("node"), "v22.20.0").is_ready());
        assert!(!judge(PathBuf::from("node"), "v18.19.0").is_ready());
        assert!(!judge(PathBuf::from("node"), "").is_ready());
    }
}
