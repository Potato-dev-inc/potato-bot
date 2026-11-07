use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize};

#[derive(Default)]
struct FR {
    next_pid: u32,
    spawn_n: u32,
    procs: HashMap<u32, (u64, PathBuf)>,
    spawns: Vec<(u16, u64)>, // (port, wall time)
    kills: Vec<u32>,
    auto_die: bool,
    fail_spawn: bool,
}
struct FakeRunner {
    st: Mutex<FR>,
    clock: Arc<FakeClock>,
}
impl FakeRunner {
    fn add_proc(&self, started: u64, image: PathBuf) -> u32 {
        let mut s = self.st.lock().unwrap();
        s.next_pid += 1;
        let pid = 1000 + s.next_pid;
        s.procs.insert(pid, (started, image));
        pid
    }
    fn spawns(&self) -> Vec<(u16, u64)> {
        self.st.lock().unwrap().spawns.clone()
    }
    fn kills(&self) -> Vec<u32> {
        self.st.lock().unwrap().kills.clone()
    }
    fn alive(&self, pid: u32) -> bool {
        self.st.lock().unwrap().procs.contains_key(&pid)
    }
    fn die_all(&self) {
        self.st.lock().unwrap().procs.clear();
    }
}
impl Runner for FakeRunner {
    fn spawn(&self, exe: &Path, port: u16, log: &Path) -> Result<ProcId, String> {
        let mut s = self.st.lock().unwrap();
        if s.fail_spawn {
            return Err("nope C:\\secret\\path".into());
        }
        s.next_pid += 1;
        s.spawn_n += 1;
        let pid = 1000 + s.next_pid;
        let started = 5_000_000 + s.spawn_n as u64;
        let now = self.clock.now_ms();
        s.spawns.push((port, now));
        if !s.auto_die {
            s.procs.insert(pid, (started, exe.to_path_buf()));
        }
        std::fs::write(
            log,
            format!(
                "INF Requesting new quick Tunnel on https://api.trycloudflare.com ...\nINF |  https://calm-fox-{}-{}.trycloudflare.com  |\n",
                port, s.spawn_n
            ),
        )
        .unwrap();
        Ok(ProcId { pid, started_at: started })
    }
    fn identity(&self, pid: u32) -> Option<ProcIdentity> {
        self.st.lock().unwrap().procs.get(&pid).map(|(s, p)| ProcIdentity { started_at: *s, image: p.clone() })
    }
    fn kill(&self, pid: u32, started_at: u64) {
        let mut s = self.st.lock().unwrap();
        if s.procs.get(&pid).map(|p| p.0) == Some(started_at) {
            s.procs.remove(&pid);
        }
        s.kills.push(pid);
    }
}

struct FakeProbe {
    ok: AtomicBool,
    calls: AtomicUsize,
    urls: Mutex<Vec<String>>,
}
impl Probe for FakeProbe {
    fn health(&self, base: &str) -> bool {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.urls.lock().unwrap().push(base.to_string());
        self.ok.load(Ordering::SeqCst)
    }
}
struct FakeClock(AtomicU64);
impl FakeClock {
    fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}
impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
    fn sleep_ms(&self, ms: u64) {
        self.advance(ms);
    }
}
struct FakeNet(Mutex<Vec<String>>);
impl NetSnapshot for FakeNet {
    fn addrs(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

struct Rig {
    dir: PathBuf,
    runner: Arc<FakeRunner>,
    probe: Arc<FakeProbe>,
    clock: Arc<FakeClock>,
    net: Arc<FakeNet>,
    mgr: Arc<TunnelManager>,
    urls: Arc<Mutex<Vec<Option<String>>>>,
}
impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
static N: AtomicUsize = AtomicUsize::new(0);

fn rig_with(exe: bool) -> Rig {
    let dir = std::env::temp_dir().join(format!("pm-tunnel-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let bundled = dir.join("bundled-cloudflared.exe");
    std::fs::write(&bundled, b"MZ fake").unwrap();
    let clock = Arc::new(FakeClock(AtomicU64::new(1_000_000)));
    let runner = Arc::new(FakeRunner { st: Mutex::new(FR::default()), clock: clock.clone() });
    let probe = Arc::new(FakeProbe { ok: AtomicBool::new(true), calls: AtomicUsize::new(0), urls: Mutex::new(vec![]) });
    let net = Arc::new(FakeNet(Mutex::new(vec!["192.168.1.5".into()])));
    let deps = Deps { runner: runner.clone(), probe: probe.clone(), clock: clock.clone(), net: net.clone() };
    let data = dir.join("data");
    let mgr = Arc::new(TunnelManager::new(data, if exe { Some(bundled) } else { None }, deps));
    let urls = Arc::new(Mutex::new(vec![]));
    let u = urls.clone();
    mgr.on_url(move |x| u.lock().unwrap().push(x));
    Rig { dir, runner, probe, clock, net, mgr, urls }
}
fn rig() -> Rig {
    rig_with(true)
}
impl Rig {
    fn data(&self) -> PathBuf {
        self.dir.join("data")
    }
    fn run_exe(&self) -> PathBuf {
        run_copy_path(&self.data())
    }
    fn probes(&self) -> usize {
        self.probe.calls.load(Ordering::SeqCst)
    }
    /// Tick every `step` ms for `total` ms.
    fn run_for(&self, total: u64, step: u64) {
        let mut t = 0;
        while t < total {
            self.clock.advance(step);
            self.mgr.tick();
            t += step;
        }
    }
}

#[test]
fn parses_the_quick_tunnel_url_from_log_text() {
    let log = "2026 INF Requesting new quick Tunnel on https://api.trycloudflare.com...\n2026 INF +--------+\n2026 INF |  https://Bright-Lake-Pine-Owl.trycloudflare.com  |\n";
    assert_eq!(parse_tunnel_url(log).as_deref(), Some("https://bright-lake-pine-owl.trycloudflare.com"));
    assert_eq!(parse_tunnel_url("https://api.trycloudflare.com only"), None);
    assert_eq!(parse_tunnel_url("https://example.com and https://x.trycloudflare.org"), None);
    assert_eq!(parse_tunnel_url(""), None);
}

#[test]
fn state_round_trips_and_survives_a_torn_file() {
    let r = rig();
    let st = Persisted { on: true, port: 1420, pid: 77, started_at: 123456789012, url: Some("https://a-b.trycloudflare.com".into()) };
    save_state(&r.data(), &st).unwrap();
    assert_eq!(load_state(&r.data()), st);
    assert!(!phone_dir(&r.data()).join("tunnel.json.tmp").exists());
    // torn, empty, garbage, wrong shape, missing
    for bad in ["{\"on\":tr", "", "not json", "[1,2]"] {
        std::fs::write(state_path(&r.data()), bad).unwrap();
        assert_eq!(load_state(&r.data()), Persisted::default());
    }
    std::fs::remove_file(state_path(&r.data())).unwrap();
    assert_eq!(load_state(&r.data()), Persisted::default());
    // unknown extra fields and missing ones are tolerated
    std::fs::write(state_path(&r.data()), "{\"on\":true,\"extra\":1}").unwrap();
    assert!(load_state(&r.data()).on);
}

fn seed_recorded(r: &Rig, started_recorded: u64, started_real: u64, image: PathBuf, port: u16) -> u32 {
    let pid = r.runner.add_proc(started_real, image);
    save_state(&r.data(), &Persisted { on: true, port, pid, started_at: started_recorded, url: Some("https://old-one.trycloudflare.com".into()) }).unwrap();
    pid
}

#[test]
fn reattaches_when_pid_start_time_image_and_url_match() {
    let r = rig();
    let pid = seed_recorded(&r, 42, 42, r.run_exe(), 1420);
    r.mgr.on_app_start(1420);
    assert!(r.runner.spawns().is_empty());
    assert!(r.runner.kills().is_empty());
    assert!(r.runner.alive(pid));
    let s = r.mgr.status();
    assert_eq!(s.phase, Phase::Running);
    assert_eq!(s.url.as_deref(), Some("https://old-one.trycloudflare.com"));
    assert_eq!(r.urls.lock().unwrap().clone(), vec![Some("https://old-one.trycloudflare.com".to_string())]);
    assert_eq!(r.probe.urls.lock().unwrap()[0], "https://old-one.trycloudflare.com");
}

#[test]
fn starts_new_when_recorded_pid_has_another_start_time() {
    let r = rig();
    let other = seed_recorded(&r, 42, 99, r.run_exe(), 1420);
    r.mgr.on_app_start(1420);
    assert_eq!(r.runner.spawns().len(), 1);
    assert!(r.runner.alive(other), "a recycled pid must not be killed");
    assert!(!r.runner.kills().contains(&other));
    assert_eq!(r.urls.lock().unwrap().clone(), vec![Some("https://calm-fox-1420-1.trycloudflare.com".to_string())]);
}

#[test]
fn starts_new_when_recorded_pid_is_another_program() {
    let r = rig();
    let other = seed_recorded(&r, 42, 42, PathBuf::from("C:\\Windows\\notepad.exe"), 1420);
    r.mgr.on_app_start(1420);
    assert_eq!(r.runner.spawns().len(), 1);
    assert!(r.runner.alive(other));
}

#[test]
fn starts_new_when_the_url_no_longer_answers_and_stops_the_old_one() {
    let r = rig();
    let old = seed_recorded(&r, 42, 42, r.run_exe(), 1420);
    r.probe.ok.store(false, Ordering::SeqCst);
    r.mgr.on_app_start(1420);
    assert_eq!(r.runner.spawns().len(), 1);
    assert!(!r.runner.alive(old), "the dead-link process is ours, so it is replaced, not leaked");
    assert_ne!(r.mgr.status().url.as_deref(), Some("https://old-one.trycloudflare.com"));
}

#[test]
fn starts_new_when_the_port_changed_while_the_app_was_closed() {
    let r = rig();
    let old = seed_recorded(&r, 42, 42, r.run_exe(), 1420);
    r.mgr.on_app_start(1555);
    assert_eq!(r.runner.spawns()[0].0, 1555);
    assert!(!r.runner.alive(old));
}

#[test]
fn off_with_a_recorded_process_kills_it_after_the_identity_check() {
    let r = rig();
    let pid = seed_recorded(&r, 42, 42, r.run_exe(), 1420);
    save_state(&r.data(), &Persisted { on: false, port: 1420, pid, started_at: 42, url: None }).unwrap();
    r.mgr.on_app_start(1420);
    assert_eq!(r.runner.kills(), vec![pid]);
    assert!(!r.runner.alive(pid));
    assert_eq!(r.mgr.status().phase, Phase::Off);
    assert_eq!(load_state(&r.data()).pid, 0);
}

#[test]
fn off_does_not_kill_a_pid_with_a_different_identity() {
    let r = rig();
    let a = r.runner.add_proc(99, r.run_exe()); // same image, other start time
    let b = r.runner.add_proc(42, PathBuf::from("C:\\x\\other.exe")); // same start time, other image
    save_state(&r.data(), &Persisted { on: false, port: 1, pid: a, started_at: 42, url: None }).unwrap();
    r.mgr.on_app_start(1);
    save_state(&r.data(), &Persisted { on: false, port: 1, pid: b, started_at: 42, url: None }).unwrap();
    r.mgr.on_app_start(1);
    assert!(r.runner.kills().is_empty());
    assert!(r.runner.alive(a) && r.runner.alive(b));
}

#[test]
fn kill_recorded_for_the_uninstaller_checks_identity_too() {
    let r = rig();
    let pid = seed_recorded(&r, 42, 42, r.run_exe(), 1420);
    let deps = Deps { runner: r.runner.clone(), probe: r.probe.clone(), clock: r.clock.clone(), net: r.net.clone() };
    assert!(kill_recorded_with(&r.data(), &deps));
    assert!(!r.runner.alive(pid));
    assert!(!load_state(&r.data()).on);
    let other = seed_recorded(&r, 42, 7, r.run_exe(), 1420);
    assert!(!kill_recorded_with(&r.data(), &deps));
    assert!(r.runner.alive(other));
}

#[test]
fn set_on_starts_runs_after_a_good_probe_and_off_stops() {
    let r = rig();
    r.mgr.set_on(true, 1420).unwrap();
    assert_eq!(r.mgr.status().phase, Phase::Starting);
    r.run_for(15_000, 5_000);
    let s = r.mgr.status();
    assert_eq!((s.on, s.phase), (true, Phase::Running));
    assert_eq!(r.runner.spawns().len(), 1);
    assert_eq!(r.runner.spawns()[0].0, 1420);
    // state on disk
    let st = load_state(&r.data());
    assert!(st.on && st.pid != 0 && st.port == 1420 && st.url.is_some());
    // same request again is a no-op
    r.mgr.set_on(true, 1420).unwrap();
    assert_eq!(r.runner.spawns().len(), 1);
    r.mgr.set_on(false, 1420).unwrap();
    assert_eq!(r.mgr.status(), TunnelStatus { on: false, phase: Phase::Off, url: None, error: None });
    assert_eq!(r.runner.kills().len(), 1);
    assert_eq!(r.urls.lock().unwrap().last().unwrap(), &None);
    r.run_for(60_000, 15_000);
    assert_eq!(r.runner.spawns().len(), 1, "off stays off");
}

#[test]
fn missing_program_is_a_plain_error() {
    let r = rig_with(false);
    let e = r.mgr.set_on(true, 1420).unwrap_err();
    assert_eq!(e, "The tunnel program is not in this install.");
    let s = r.mgr.status();
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(s.error.as_deref(), Some("The tunnel program is not in this install."));
    assert!(!s.on);
}

#[test]
fn spawn_failure_hides_paths() {
    let r = rig();
    r.runner.st.lock().unwrap().fail_spawn = true;
    let e = r.mgr.set_on(true, 1420).unwrap_err();
    assert_eq!(e, "The tunnel program could not start.");
    assert!(!e.contains('\\'));
}

#[test]
fn backoff_is_5_15_45_120_300_and_stays_at_300() {
    let r = rig();
    r.runner.st.lock().unwrap().auto_die = true;
    r.mgr.set_on(true, 1420).unwrap();
    // the process is dead from the start; tick every second for a long while
    r.run_for(1_200_000, 1_000);
    let t: Vec<u64> = r.runner.spawns().iter().map(|s| s.1).collect();
    let gaps: Vec<u64> = t.windows(2).map(|w| (w[1] - w[0]) / 1_000).collect();
    // each gap = delay + 1 s (the next tick notices the death) with the first 1 s being the detection tick
    assert_eq!(&gaps[..6], &[6, 16, 46, 121, 301, 301], "gaps were {gaps:?}");
}

#[test]
fn backoff_resets_after_ten_minutes_of_health() {
    let r = rig();
    r.mgr.set_on(true, 1420).unwrap();
    // a few quick deaths to push the backoff up
    for _ in 0..3 {
        r.runner.die_all();
        r.run_for(60_000, 1_000);
    }
    let before = r.runner.spawns().len();
    // now live and healthy for 11 minutes
    r.run_for(11 * 60_000, 15_000);
    assert_eq!(r.mgr.status().phase, Phase::Running);
    assert_eq!(r.runner.spawns().len(), before);
    r.runner.die_all();
    let died_at = r.clock.now_ms();
    r.run_for(20_000, 1_000);
    let last = r.runner.spawns().last().unwrap().1;
    assert!(last - died_at <= 8_000, "should wait the first step (5 s), waited {} ms", last - died_at);
}

#[test]
fn a_failed_probe_alone_never_restarts_a_live_process_before_ten_minutes() {
    let r = rig();
    r.mgr.set_on(true, 1420).unwrap();
    r.run_for(30_000, 15_000);
    assert_eq!(r.mgr.status().phase, Phase::Running);
    r.probe.ok.store(false, Ordering::SeqCst);
    r.run_for(9 * 60_000, 15_000);
    assert_eq!(r.runner.spawns().len(), 1);
    assert_eq!(r.mgr.status().phase, Phase::Reconnecting);
    assert!(r.mgr.status().url.is_some());
    // 10 minutes of continuous failure: restart once, new URL
    r.run_for(3 * 60_000, 15_000);
    assert_eq!(r.runner.spawns().len(), 2);
    assert!(r.mgr.last_event().contains("10 minutes"));
    assert_eq!(r.urls.lock().unwrap().last().unwrap().as_deref(), Some("https://calm-fox-1420-2.trycloudflare.com"));
    // and not again right away
    r.run_for(5 * 60_000, 15_000);
    assert_eq!(r.runner.spawns().len(), 2);
}

#[test]
fn a_recovered_probe_clears_the_failure_clock() {
    let r = rig();
    r.mgr.set_on(true, 1420).unwrap();
    r.run_for(30_000, 15_000);
    r.probe.ok.store(false, Ordering::SeqCst);
    r.run_for(8 * 60_000, 15_000);
    r.probe.ok.store(true, Ordering::SeqCst);
    r.run_for(30_000, 15_000);
    r.probe.ok.store(false, Ordering::SeqCst);
    r.run_for(8 * 60_000, 15_000);
    assert_eq!(r.runner.spawns().len(), 1);
}

#[test]
fn probes_the_public_url_about_once_a_minute() {
    let r = rig();
    r.mgr.set_on(true, 1420).unwrap();
    r.run_for(30_000, 15_000); // first probe happens, link verified
    let base = r.probes();
    r.run_for(10 * 60_000, 15_000);
    let n = r.probes() - base;
    assert!((9..=10).contains(&n), "{n} probes in 10 minutes");
}

#[test]
fn a_clock_jump_after_sleep_probes_at_once() {
    let r = rig();
    r.mgr.set_on(true, 1420).unwrap();
    r.run_for(30_000, 15_000);
    r.run_for(15_000, 15_000);
    let base = r.probes();
    r.run_for(15_000, 15_000); // an ordinary tick, not due
    assert_eq!(r.probes(), base);
    r.clock.advance(50_000); // 50 s: more than 3 ticks, less than the 60 s probe interval
    r.mgr.tick();
    assert_eq!(r.probes(), base + 1);
}

#[test]
fn a_local_network_change_probes_at_once() {
    let r = rig();
    r.mgr.set_on(true, 1420).unwrap();
    r.run_for(30_000, 15_000);
    let base = r.probes();
    r.run_for(15_000, 15_000);
    assert_eq!(r.probes(), base);
    *r.net.0.lock().unwrap() = vec!["10.0.0.9".into()];
    r.run_for(15_000, 15_000);
    assert_eq!(r.probes(), base + 1);
    r.run_for(15_000, 15_000);
    assert_eq!(r.probes(), base + 1, "the same network again is not a change");
}

#[test]
fn set_port_restarts_only_when_on_and_fires_the_callback_with_the_new_url() {
    let r = rig();
    r.mgr.set_port(1500); // off: just remembered
    assert!(r.runner.spawns().is_empty());
    r.mgr.set_on(true, 1500).unwrap();
    r.run_for(15_000, 5_000);
    assert_eq!(r.urls.lock().unwrap().clone(), vec![Some("https://calm-fox-1500-1.trycloudflare.com".to_string())]);
    let first = r.runner.st.lock().unwrap().procs.keys().copied().next().unwrap();
    r.mgr.set_port(1500); // same port: nothing
    assert_eq!(r.runner.spawns().len(), 1);
    r.mgr.set_port(1600);
    assert_eq!(r.runner.spawns().len(), 2);
    assert_eq!(r.runner.spawns()[1].0, 1600);
    assert!(!r.runner.alive(first));
    assert_eq!(r.urls.lock().unwrap().last().unwrap().as_deref(), Some("https://calm-fox-1600-2.trycloudflare.com"));
    assert_eq!(load_state(&r.data()).port, 1600);
}

#[test]
fn a_process_that_never_prints_a_link_is_restarted_with_a_plain_error() {
    let r = rig();
    r.mgr.set_on(true, 1420).unwrap();
    // wipe the link out of the log and keep it empty
    r.runner.st.lock().unwrap().auto_die = false;
    {
        let mut i = r.mgr.inner.lock().unwrap();
        i.url = None;
        i.st.url = None;
    }
    std::fs::write(log_path(&r.data()), "INF starting\n").unwrap();
    r.run_for(90_000, 5_000);
    let s = r.mgr.status();
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(s.error.as_deref(), Some("The tunnel started but did not give a link. Check your internet connection."));
}

#[test]
fn the_tunnel_only_ever_targets_the_given_phone_port() {
    let r = rig();
    r.mgr.set_on(true, 1420).unwrap();
    r.runner.die_all();
    r.run_for(60_000, 1_000);
    r.mgr.set_port(1888);
    assert!(r.runner.spawns().iter().all(|s| s.0 == 1420 || s.0 == 1888));
    assert_eq!(r.runner.spawns().last().unwrap().0, 1888);
}
