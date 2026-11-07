//! The away-from-home tunnel for the phone: a Cloudflare "quick tunnel" that points ONLY at the phone server
//! (`http://127.0.0.1:<phone port>`), never at the bridge, MCP or dev port.
//!
//! * cloudflared runs as a DETACHED child (own process group, no window, outside the app's job object when the
//!   job allows breakaway), with its output going to `<data>/phone/cloudflared.log`. It therefore survives an
//!   app restart or update, and the public URL stays the same: on the next start the app REATTACHES.
//! * To reattach, `<data>/phone/tunnel.json` records the pid and the process creation time. A pid is trusted only
//!   when its creation time and image path both match (a recycled pid is never touched) and the public URL
//!   answers our health JSON.
//! * The program is run from a private copy (`<data>/phone/cloudflared-run.exe`), so an installer update can
//!   replace the bundled exe while the tunnel is up.
//! * A supervisor (`run_supervisor`, started on a thread by the caller) restarts a dead process with backoff and
//!   probes the public URL; a failed probe alone never restarts a live process, except after 10 minutes of
//!   continuous failure.
//!
//! No tauri types: it compiles on its own. Everything outside the process (spawn, kill, identity, network probe,
//! clock, local addresses) sits behind traits so the tests use fakes.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const TICK_MS: u64 = 15_000;
const PROBE_EVERY_MS: u64 = 60_000;
/// While the link is failing, ask more often so recovery shows quickly.
const PROBE_RETRY_MS: u64 = 15_000;
/// Until the link has answered once.
const PROBE_FIRST_MS: u64 = 5_000;
const RESTART_AFTER_FAILING_MS: u64 = 10 * 60_000;
const BACKOFF_RESET_AFTER_MS: u64 = 10 * 60_000;
const URL_WAIT_MS: u64 = 90_000;
pub const BACKOFF_MS: [u64; 5] = [5_000, 15_000, 45_000, 120_000, 300_000];

const MSG_MISSING: &str = "The tunnel program is not in this install.";
const MSG_SPAWN: &str = "The tunnel program could not start.";
const MSG_NO_LINK: &str = "The tunnel started but did not give a link. Check your internet connection.";

// ---------------------------------------------------------------- state ---------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Off,
    Starting,
    Running,
    Reconnecting,
    Error,
}

/// What the settings page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelStatus {
    pub on: bool,
    pub phase: Phase,
    pub url: Option<String>,
    /// Plain English, no paths.
    pub error: Option<String>,
}

/// `<data>/phone/tunnel.json`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Persisted {
    pub on: bool,
    pub port: u16,
    pub pid: u32,
    /// Process creation time (Windows FILETIME, 100 ns units) of `pid`.
    pub started_at: u64,
    pub url: Option<String>,
}

pub fn phone_dir(data: &Path) -> PathBuf {
    data.join("phone")
}
pub fn state_path(data: &Path) -> PathBuf {
    phone_dir(data).join("tunnel.json")
}
pub fn log_path(data: &Path) -> PathBuf {
    phone_dir(data).join("cloudflared.log")
}
pub fn run_copy_path(data: &Path) -> PathBuf {
    phone_dir(data).join("cloudflared-run.exe")
}

/// A missing, torn or corrupt file is "off, nothing recorded".
pub fn load_state(data: &Path) -> Persisted {
    std::fs::read_to_string(state_path(data))
        .ok()
        .and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

/// Write to a temp file, then rename over the real one, so a crash never leaves half a file.
pub fn save_state(data: &Path, st: &Persisted) -> std::io::Result<()> {
    let dir = phone_dir(data);
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join("tunnel.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(st).unwrap_or_default())?;
    std::fs::rename(&tmp, state_path(data))
}

/// First `https://<words>.trycloudflare.com` in the text. `api.trycloudflare.com` is cloudflared's own service
/// host (it shows up in error lines), not a tunnel.
pub fn parse_tunnel_url(text: &str) -> Option<String> {
    const SUFFIX: &str = ".trycloudflare.com";
    let mut rest = text;
    while let Some(i) = rest.find("https://") {
        let after = &rest[i + 8..];
        let label_len = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(after.len());
        let label = &after[..label_len];
        if !label.is_empty() && after[label_len..].starts_with(SUFFIX) && !label.eq_ignore_ascii_case("api") {
            return Some(format!("https://{}{}", label.to_ascii_lowercase(), SUFFIX));
        }
        rest = &rest[i + 8..];
    }
    None
}

fn read_log_tail(path: &Path) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(256 * 1024)));
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

fn norm_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\");
    s.strip_prefix("\\\\?\\").unwrap_or(&s).to_ascii_lowercase()
}

// ---------------------------------------------------------------- seams ---------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcId {
    pub pid: u32,
    pub started_at: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcIdentity {
    pub started_at: u64,
    pub image: PathBuf,
}

pub trait Runner: Send + Sync {
    /// Start `exe tunnel --no-autoupdate --url http://127.0.0.1:<port>` detached, output to `log` (truncated).
    fn spawn(&self, exe: &Path, port: u16, log: &Path) -> Result<ProcId, String>;
    /// None when no such live process.
    fn identity(&self, pid: u32) -> Option<ProcIdentity>;
    /// Terminate `pid` only if its creation time is still `started_at`.
    fn kill(&self, pid: u32, started_at: u64);
}
pub trait Probe: Send + Sync {
    /// `GET <base>/api/health` answers 200 with `ok:true`.
    fn health(&self, base_url: &str) -> bool;
}
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
    fn sleep_ms(&self, ms: u64);
}
pub trait NetSnapshot: Send + Sync {
    /// The local IPv4 addresses in use; a change means the network changed.
    fn addrs(&self) -> Vec<String>;
}

#[derive(Clone)]
pub struct Deps {
    pub runner: Arc<dyn Runner>,
    pub probe: Arc<dyn Probe>,
    pub clock: Arc<dyn Clock>,
    pub net: Arc<dyn NetSnapshot>,
}
impl Deps {
    pub fn real() -> Self {
        Deps { runner: Arc::new(RealRunner), probe: Arc::new(RealProbe), clock: Arc::new(RealClock), net: Arc::new(RealNet) }
    }
}

pub struct RealClock;
impl Clock for RealClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
    }
    fn sleep_ms(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

pub struct RealProbe;
impl Probe for RealProbe {
    fn health(&self, base_url: &str) -> bool {
        let url = format!("{}/api/health", base_url.trim_end_matches('/'));
        let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(6)).build();
        match agent.get(&url).call() {
            Ok(r) if r.status() == 200 => r
                .into_json::<serde_json::Value>()
                .map(|v| v.get("ok").and_then(|o| o.as_bool()) == Some(true))
                .unwrap_or(false),
            _ => false,
        }
    }
}

/// The address the OS would use to reach the internet (a UDP "connect" sends nothing).
pub struct RealNet;
impl NetSnapshot for RealNet {
    fn addrs(&self) -> Vec<String> {
        let Ok(s) = std::net::UdpSocket::bind("0.0.0.0:0") else { return vec![] };
        if s.connect("8.8.8.8:80").is_err() {
            return vec![];
        }
        s.local_addr().map(|a| vec![a.ip().to_string()]).unwrap_or_default()
    }
}

pub struct RealRunner;

#[cfg(windows)]
mod win {
    use super::{ProcId, ProcIdentity, Runner};
    use std::path::{Path, PathBuf};
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_TERMINATE,
    };


    fn ft(f: FILETIME) -> u64 {
        ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
    }
    /// (creation, exit) times; exit is 0 while the process runs.
    fn times(h: HANDLE) -> Option<(u64, u64)> {
        let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut c, mut e, mut k, mut u) = (z, z, z, z);
        // SAFETY: h is a valid process handle; the out pointers are valid FILETIMEs.
        (unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) } != 0).then(|| (ft(c), ft(e)))
    }
    fn image(h: HANDLE) -> Option<PathBuf> {
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        // SAFETY: buf has `len` u16s.
        if unsafe { QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) } == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }

    impl Runner for super::RealRunner {
        fn spawn(&self, exe: &Path, port: u16, log: &Path) -> Result<ProcId, String> {
            let mk = |breakaway: bool| -> std::io::Result<std::process::Child> {
                let out = std::fs::File::create(log)?;
                let err = out.try_clone()?;
                crate::proc_util::detached_command(exe, breakaway)
                    .args(["tunnel", "--no-autoupdate", "--url", &format!("http://127.0.0.1:{port}")])
                    .stdin(std::process::Stdio::null())
                    .stdout(out)
                    .stderr(err)
                    .spawn()
            };
            // Leave the app's job object if it is in one that allows it; otherwise start without.
            let child = mk(true).or_else(|_| mk(false)).map_err(|e| e.to_string())?;
            let pid = child.id();
            drop(child); // dropping a Child neither waits for nor kills it
            let started_at = self.identity(pid).map(|i| i.started_at).ok_or_else(|| "it exited at once".to_string())?;
            Ok(ProcId { pid, started_at })
        }
        fn identity(&self, pid: u32) -> Option<ProcIdentity> {
            // SAFETY: plain Win32 calls; the handle is closed before returning.
            unsafe {
                let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if h == 0 {
                    return None;
                }
                let r = match times(h) {
                    Some((started_at, 0)) => image(h).map(|image| ProcIdentity { started_at, image }),
                    _ => None,
                };
                CloseHandle(h);
                r
            }
        }
        fn kill(&self, pid: u32, started_at: u64) {
            // SAFETY: as above.
            unsafe {
                let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE, 0, pid);
                if h == 0 {
                    return;
                }
                if times(h).map(|t| t.0) == Some(started_at) {
                    TerminateProcess(h, 0);
                }
                CloseHandle(h);
            }
        }
    }
}

#[cfg(not(windows))]
impl Runner for RealRunner {
    fn spawn(&self, _: &Path, _: u16, _: &Path) -> Result<ProcId, String> {
        Err("Windows only".into())
    }
    fn identity(&self, _: u32) -> Option<ProcIdentity> {
        None
    }
    fn kill(&self, _: u32, _: u64) {}
}

// ---------------------------------------------------------------- manager -------------------------------------

type Callback = Arc<dyn Fn(Option<String>) + Send + Sync>;

#[derive(Default)]
struct Inner {
    st: Persisted,
    phase: Option<Phase>,
    error: Option<String>,
    url: Option<String>,
    had_url: bool,
    notified: Option<String>,
    pending: Vec<Option<String>>,
    spawned_at: u64,
    last_probe: Option<u64>,
    fail_since: Option<u64>,
    healthy_since: Option<u64>,
    backoff_idx: usize,
    restart_at: Option<u64>,
    last_tick: Option<u64>,
    nets: Option<Vec<String>>,
    last_event: String,
}

pub struct TunnelManager {
    data: PathBuf,
    exe: Option<PathBuf>,
    deps: Deps,
    inner: Mutex<Inner>,
    /// Serialises the operations that change the process (tick, set_on, set_port, start); `inner` is only ever
    /// held briefly so `status()` never waits on a network probe.
    ops: Mutex<()>,
    cb: Mutex<Option<Callback>>,
    stop: AtomicBool,
}

impl TunnelManager {
    pub fn new(data_dir: PathBuf, exe: Option<PathBuf>, deps: Deps) -> Self {
        TunnelManager {
            data: data_dir,
            exe,
            deps,
            inner: Mutex::new(Inner::default()),
            ops: Mutex::new(()),
            cb: Mutex::new(None),
            stop: AtomicBool::new(false),
        }
    }

    /// Called with the new public URL (or None when the tunnel is off) each time it changes.
    pub fn on_url(&self, cb: impl Fn(Option<String>) + Send + Sync + 'static) {
        *self.cb.lock().unwrap() = Some(Arc::new(cb));
    }

    pub fn status(&self) -> TunnelStatus {
        let i = self.inner.lock().unwrap();
        let on = i.st.on || i.phase == Some(Phase::Error);
        let phase = if !on { Phase::Off } else { i.phase.unwrap_or(Phase::Starting) };
        let url = if matches!(phase, Phase::Running | Phase::Reconnecting) { i.url.clone() } else { None };
        TunnelStatus { on: i.st.on, phase, url, error: if on { i.error.clone() } else { None } }
    }

    /// The last thing the supervisor did that is worth a log line (a restart and why).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn last_event(&self) -> String {
        self.inner.lock().unwrap().last_event.clone()
    }

    /// Ask `run_supervisor` to return after its current tick.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn shutdown_supervisor(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        f(&mut self.inner.lock().unwrap())
    }

    fn flush(&self) {
        let events = self.with(|i| std::mem::take(&mut i.pending));
        let cb = self.cb.lock().unwrap().clone();
        if let Some(cb) = cb {
            for e in events {
                cb(e);
            }
        }
    }

    fn save(&self) {
        let st = self.with(|i| i.st.clone());
        if let Err(e) = save_state(&self.data, &st) {
            eprintln!("[phone-tunnel] could not save tunnel.json: {e}");
        }
    }

    fn event(&self, msg: &str) {
        eprintln!("[phone-tunnel] {msg}");
        self.with(|i| i.last_event = msg.to_string());
    }

    /// Queue a callback for a URL change (first URL, new URL, or gone).
    fn publish(i: &mut Inner, url: Option<String>) {
        if i.notified != url {
            i.notified = url.clone();
            i.pending.push(url);
        }
    }

    /// Is the recorded pid still the process we started (creation time and image both match)?
    fn recorded_is_ours(&self) -> bool {
        let (pid, started) = self.with(|i| (i.st.pid, i.st.started_at));
        pid != 0
            && self
                .deps
                .runner
                .identity(pid)
                .map(|id| id.started_at == started && norm_path(&id.image) == norm_path(&run_copy_path(&self.data)))
                .unwrap_or(false)
    }

    /// Kill the recorded process, but only after the identity check.
    fn kill_ours(&self) -> bool {
        if !self.recorded_is_ours() {
            return false;
        }
        let (pid, started) = self.with(|i| (i.st.pid, i.st.started_at));
        self.deps.runner.kill(pid, started);
        true
    }

    fn ensure_run_copy(&self) -> Result<PathBuf, String> {
        let run = run_copy_path(&self.data);
        let _ = std::fs::create_dir_all(phone_dir(&self.data));
        let src = self.exe.as_ref().filter(|p| p.is_file());
        let have = std::fs::metadata(&run).ok().map(|m| m.len());
        match (src, have) {
            (Some(s), h) if h != std::fs::metadata(s).ok().map(|m| m.len()) => {
                // If the old copy is locked (still running) keep using it; the new one comes at the next restart.
                if std::fs::copy(s, &run).is_err() && h.is_none() {
                    return Err(MSG_SPAWN.into());
                }
                Ok(run)
            }
            (_, Some(_)) => Ok(run),
            _ => Err(MSG_MISSING.into()),
        }
    }

    /// Stop any process of ours, start a fresh one at the recorded port, and pick up its URL if it is already
    /// in the log.
    fn start_new(&self, now: u64) -> Result<(), String> {
        let run = self.ensure_run_copy()?;
        self.kill_ours();
        let port = self.with(|i| i.st.port);
        let log = log_path(&self.data);
        let id = self.deps.runner.spawn(&run, port, &log).map_err(|e| {
            eprintln!("[phone-tunnel] spawn failed: {e}");
            MSG_SPAWN.to_string()
        })?;
        self.with(|i| {
            i.st.on = true;
            i.st.pid = id.pid;
            i.st.started_at = id.started_at;
            i.st.url = None;
            i.url = None;
            i.phase = Some(if i.had_url { Phase::Reconnecting } else { Phase::Starting });
            i.error = None;
            i.spawned_at = now;
            i.last_probe = None;
            i.fail_since = None;
            i.healthy_since = None;
            i.restart_at = None;
        });
        self.save();
        self.refresh_url();
        Ok(())
    }

    fn refresh_url(&self) {
        if self.with(|i| i.url.is_some()) {
            return;
        }
        if let Some(u) = parse_tunnel_url(&read_log_tail(&log_path(&self.data))) {
            self.with(|i| {
                i.url = Some(u.clone());
                i.st.url = Some(u.clone());
                i.had_url = true;
                Self::publish(i, Some(u));
            });
            self.save();
        }
    }

    /// Wait before the next restart: 5 s, 15 s, 45 s, 2 min, then 5 min.
    fn schedule_restart(&self, now: u64, error: Option<String>) {
        self.with(|i| {
            if i.healthy_since.is_some_and(|h| now.saturating_sub(h) >= BACKOFF_RESET_AFTER_MS) {
                i.backoff_idx = 0;
            }
            let delay = BACKOFF_MS[i.backoff_idx.min(BACKOFF_MS.len() - 1)];
            i.backoff_idx = (i.backoff_idx + 1).min(BACKOFF_MS.len() - 1);
            i.restart_at = Some(now + delay);
            i.st.pid = 0;
            i.st.url = None;
            i.url = None;
            i.healthy_since = None;
            match error {
                Some(e) => {
                    i.phase = Some(Phase::Error);
                    i.error = Some(e);
                }
                None => {
                    i.phase = Some(if i.had_url { Phase::Reconnecting } else { Phase::Starting });
                }
            }
        });
        self.save();
    }

    /// A start that failed outside the supervisor: show the error; retry later unless the program is missing.
    fn start_failed(&self, now: u64, e: String) {
        if e == MSG_MISSING {
            self.with(|i| {
                i.phase = Some(Phase::Error);
                i.error = Some(e);
            });
        } else {
            self.schedule_restart(now, Some(e));
        }
    }

    // ------------------------------------------------------------ public operations ---------------------------

    /// App start: reattach to a tunnel that is still ours and still answering, otherwise start a new one (when
    /// the switch is on) or clean up (when it is off).
    pub fn on_app_start(&self, port: u16) {
        let _g = self.ops.lock().unwrap();
        let now = self.deps.clock.now_ms();
        let loaded = load_state(&self.data);
        self.with(|i| i.st = loaded.clone());
        if !loaded.on {
            if self.kill_ours() {
                self.event("the tunnel is off: stopped the one left over from before");
            }
            self.with(|i| {
                i.st.pid = 0;
                i.st.url = None;
                i.phase = Some(Phase::Off);
            });
            self.save();
            return;
        }
        if loaded.port == port && self.recorded_is_ours() {
            if let Some(url) = loaded.url.clone() {
                if self.deps.probe.health(&url) {
                    self.with(|i| {
                        i.url = Some(url.clone());
                        i.had_url = true;
                        i.phase = Some(Phase::Running);
                        i.error = None;
                        i.spawned_at = now;
                        i.last_probe = Some(now);
                        i.healthy_since = Some(now);
                        Self::publish(i, Some(url));
                    });
                    self.event("reattached to the running tunnel");
                    self.flush();
                    return;
                }
            }
        }
        self.with(|i| i.st.port = port);
        if let Err(e) = self.start_new(now) {
            self.start_failed(now, e);
        }
        self.flush();
    }

    pub fn set_on(&self, on: bool, port: u16) -> Result<(), String> {
        let _g = self.ops.lock().unwrap();
        let now = self.deps.clock.now_ms();
        if !on {
            self.stop_locked();
            self.flush();
            return Ok(());
        }
        if self.with(|i| i.st.on && i.st.port == port) && (self.recorded_is_ours() || self.with(|i| i.restart_at.is_some())) {
            return Ok(());
        }
        self.with(|i| {
            i.st.port = port;
            i.backoff_idx = 0;
        });
        let r = self.start_new(now);
        if let Err(e) = &r {
            self.with(|i| {
                i.st.on = false;
                i.phase = Some(Phase::Error);
                i.error = Some(e.clone());
            });
            self.save();
        }
        self.flush();
        r
    }

    /// Turn the tunnel off and kill the recorded process (after the identity check).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn stop(&self) {
        let _g = self.ops.lock().unwrap();
        self.stop_locked();
        self.flush();
    }

    fn stop_locked(&self) {
        self.kill_ours();
        self.with(|i| {
            i.st.on = false;
            i.st.pid = 0;
            i.st.url = None;
            i.url = None;
            i.error = None;
            i.phase = Some(Phase::Off);
            i.restart_at = None;
            Self::publish(i, None);
        });
        self.save();
    }

    /// The phone server moved: follow it. Only restarts cloudflared when the tunnel is on (the URL changes).
    pub fn set_port(&self, port: u16) {
        let _g = self.ops.lock().unwrap();
        let now = self.deps.clock.now_ms();
        let (on, same) = self.with(|i| (i.st.on, i.st.port == port));
        if same {
            return;
        }
        self.with(|i| i.st.port = port);
        if !on {
            self.save();
            return;
        }
        self.event("the phone port changed: restarting the tunnel");
        self.with(|i| i.backoff_idx = 0);
        if let Err(e) = self.start_new(now) {
            self.start_failed(now, e);
        }
        self.flush();
    }

    // ------------------------------------------------------------ supervisor ----------------------------------

    /// Blocking loop; the caller runs it on a thread.
    pub fn run_supervisor(self: Arc<Self>) {
        while !self.stop.load(Ordering::SeqCst) {
            self.tick();
            self.deps.clock.sleep_ms(TICK_MS);
        }
    }

    /// One supervisor step (public so tests can drive it).
    pub fn tick(&self) {
        let _g = self.ops.lock().unwrap();
        let now = self.deps.clock.now_ms();
        let (on, jumped) = self.with(|i| {
            let jumped = i.last_tick.is_some_and(|t| now > t + 3 * TICK_MS || now < t);
            i.last_tick = Some(now);
            (i.st.on, jumped)
        });
        if !on {
            return;
        }
        let nets = self.deps.net.addrs();
        let net_changed = self.with(|i| {
            let changed = i.nets.as_ref().is_some_and(|p| *p != nets);
            i.nets = Some(nets);
            changed
        });
        if self.recorded_is_ours() {
            self.handle_alive(now, jumped || net_changed);
        } else {
            self.handle_dead(now);
        }
        self.flush();
    }

    fn handle_dead(&self, now: u64) {
        match self.with(|i| i.restart_at) {
            None => {
                self.event("the tunnel process is gone: restarting soon");
                self.schedule_restart(now, None);
            }
            Some(at) if now >= at => match self.start_new(now) {
                Ok(()) => self.event("restarted the tunnel"),
                Err(e) => {
                    self.event("restart failed");
                    self.schedule_restart(now, Some(e));
                }
            },
            Some(_) => {}
        }
    }

    fn handle_alive(&self, now: u64, force_probe: bool) {
        self.refresh_url();
        let (url, spawned_at) = self.with(|i| (i.url.clone(), i.spawned_at));
        let Some(url) = url else {
            if now.saturating_sub(spawned_at) >= URL_WAIT_MS {
                self.kill_ours();
                self.event("no link after 90 seconds: restarting the tunnel");
                self.schedule_restart(now, Some(MSG_NO_LINK.into()));
            }
            return;
        };
        let (last, failing, healthy) = self.with(|i| (i.last_probe, i.fail_since.is_some(), i.healthy_since.is_some()));
        let every = if failing { PROBE_RETRY_MS } else if healthy { PROBE_EVERY_MS } else { PROBE_FIRST_MS };
        if !(force_probe || last.is_none_or(|l| now.saturating_sub(l) >= every)) {
            return;
        }
        let ok = self.deps.probe.health(&url); // slow: no state lock held
        let restart = self.with(|i| {
            i.last_probe = Some(now);
            if ok {
                i.fail_since = None;
                i.healthy_since.get_or_insert(now);
                i.phase = Some(Phase::Running);
                i.error = None;
                false
            } else {
                let since = *i.fail_since.get_or_insert(now);
                i.phase = Some(if i.healthy_since.is_some() { Phase::Reconnecting } else { Phase::Starting });
                now.saturating_sub(since) >= RESTART_AFTER_FAILING_MS
            }
        });
        if restart {
            self.event("the link has not answered for 10 minutes: restarting the tunnel once");
            if let Err(e) = self.start_new(now) {
                self.schedule_restart(now, Some(e));
            }
        }
    }
}

/// For an uninstaller hook: kill the recorded tunnel (after the identity check) and mark it off. True if killed.
pub fn kill_recorded(data_dir: &Path) -> bool {
    kill_recorded_with(data_dir, &Deps::real())
}

pub fn kill_recorded_with(data_dir: &Path, deps: &Deps) -> bool {
    let mut st = load_state(data_dir);
    let ours = st.pid != 0
        && deps
            .runner
            .identity(st.pid)
            .map(|id| id.started_at == st.started_at && norm_path(&id.image) == norm_path(&run_copy_path(data_dir)))
            .unwrap_or(false);
    if ours {
        deps.runner.kill(st.pid, st.started_at);
    }
    st.on = false;
    st.pid = 0;
    st.url = None;
    let _ = save_state(data_dir, &st);
    ours
}

#[cfg(test)]
#[path = "tunnel_tests.rs"]
mod tunnel_tests;
