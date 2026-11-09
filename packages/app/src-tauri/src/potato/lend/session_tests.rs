//! The lend state machine with a fake OS: no real window is touched.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use parking_lot::Mutex;

use super::session::*;
use super::{Hwnd, LendError, Rect, Result, WinInfo};

const OWN_PID: u32 = 1;

#[derive(Default)]
struct World {
    windows: HashMap<Hwnd, WinInfo>,
    dead: HashSet<Hwnd>,
    events: HashMap<Hwnd, VecDeque<Ev>>,
    /// Last state each border was told to show.
    shown: HashMap<Hwnd, Shown>,
    borders: HashSet<Hwnd>,
    see_through: HashSet<Hwnd>,
    log: Vec<String>,
    border_fails: bool,
}

#[derive(Default)]
struct Fake(Mutex<World>);

struct FakeBorder(Hwnd, Arc<Fake>);

impl BorderCtl for FakeBorder {
    fn set_state(&self, state: Shown) {
        self.1 .0.lock().shown.insert(self.0, state);
    }
    fn poll(&self) -> Option<Ev> {
        self.1 .0.lock().events.get_mut(&self.0).and_then(|q| q.pop_front())
    }
}

impl Drop for FakeBorder {
    fn drop(&mut self) {
        self.1 .0.lock().borders.remove(&self.0);
    }
}

struct FakeOs(Arc<Fake>);

impl FakeOs {
    fn w(&self) -> parking_lot::MutexGuard<'_, World> {
        self.0 .0.lock()
    }
}

impl Os for FakeOs {
    type Snap = Vec<String>;
    type Elem = String;
    fn info(&self, h: Hwnd) -> Result<WinInfo> {
        let w = self.w();
        if w.dead.contains(&h) {
            return Err(LendError::Gone);
        }
        w.windows.get(&h).cloned().ok_or(LendError::Gone)
    }
    fn is_alive(&self, h: Hwnd) -> bool {
        let w = self.w();
        w.windows.contains_key(&h) && !w.dead.contains(&h)
    }
    fn own_pid(&self) -> u32 {
        OWN_PID
    }
    fn see_through(&self, h: Hwnd) -> Result<Saved> {
        self.w().see_through.insert(h);
        Ok(Saved { ex_style: 0x100, was_layered: false, alpha: None })
    }
    fn restore(&self, h: Hwnd, saved: Saved) -> Result<()> {
        assert_eq!(saved.ex_style, 0x100);
        self.w().see_through.remove(&h);
        self.w().log.push(format!("restore {h}"));
        Ok(())
    }
    fn start_border(&self, h: Hwnd, _label: String) -> Result<Box<dyn BorderCtl>> {
        if self.w().border_fails {
            return Err(LendError::Os("no border".into()));
        }
        self.w().borders.insert(h);
        Ok(Box::new(FakeBorder(h, self.0.clone())))
    }
    fn snapshot(&self, h: Hwnd) -> Result<Vec<String>> {
        self.w().log.push(format!("snapshot {h}"));
        Ok(vec!["e1".into(), "e2".into()])
    }
    fn find(&self, snap: &Vec<String>, r: &str) -> Option<String> {
        snap.iter().find(|e| e.as_str() == r).cloned()
    }
    fn capture(&self, _h: Hwnd) -> Result<Vec<u8>> {
        Ok(vec![1, 2, 3])
    }
    fn click(&self, h: Hwnd, x: i32, y: i32) -> Result<()> {
        self.w().log.push(format!("click {h} {x},{y}"));
        Ok(())
    }
    fn double_click(&self, h: Hwnd, x: i32, y: i32) -> Result<()> {
        self.w().log.push(format!("dbl {h} {x},{y}"));
        Ok(())
    }
    fn type_text(&self, h: Hwnd, text: &str) -> Result<()> {
        self.w().log.push(format!("type {h} {text}"));
        Ok(())
    }
    fn key(&self, h: Hwnd, vk: u16) -> Result<()> {
        self.w().log.push(format!("key {h} {vk}"));
        Ok(())
    }
    fn scroll(&self, h: Hwnd, x: i32, y: i32, d: i32) -> Result<()> {
        self.w().log.push(format!("scroll {h} {x},{y},{d}"));
        Ok(())
    }
}

fn win(hwnd: Hwnd, pid: u32, elevated: bool) -> WinInfo {
    WinInfo {
        hwnd,
        pid,
        title: format!("Window {hwnd}"),
        exe: "LINE.exe".into(),
        app_name: "LINE".into(),
        rect: Rect { x: 0, y: 0, w: 800, h: 600 },
        elevated,
        minimized: false,
    }
}

fn setup() -> (Arc<Fake>, Core<FakeOs>) {
    let fake = Arc::new(Fake::default());
    {
        let mut w = fake.0.lock();
        w.windows.insert(10, win(10, 500, false));
        w.windows.insert(11, win(11, 501, false));
        w.windows.insert(66, win(66, 600, true));
        w.windows.insert(77, win(77, OWN_PID, false));
    }
    let core = Core::new(Arc::new(FakeOs(fake.clone())));
    (fake, core)
}

fn push(fake: &Fake, h: Hwnd, ev: Ev) {
    fake.0.lock().events.entry(h).or_default().push_back(ev);
}

fn shown(fake: &Fake, h: Hwnd) -> Option<Shown> {
    fake.0.lock().shown.get(&h).copied()
}

#[test]
fn lend_starts_free_with_a_border_and_a_short_id() {
    let (fake, mut core) = setup();
    let v = core.lend(10, Mode::Visible).unwrap();
    assert_eq!((v.id.as_str(), v.phase, v.mode, v.app_name.as_str()), ("l1", Phase::Free, Mode::Visible, "LINE"));
    assert_eq!(shown(&fake, 10), Some(Shown::Bot));
    assert!(fake.0.lock().borders.contains(&10));
    assert_eq!(core.lend(11, Mode::Visible).unwrap().id, "l2");
    assert_eq!(core.list().len(), 2);
}

#[test]
fn lending_the_same_window_twice_returns_the_existing_one() {
    let (_fake, mut core) = setup();
    let a = core.lend(10, Mode::Visible).unwrap();
    let b = core.lend(10, Mode::Hidden).unwrap();
    assert_eq!(a, b);
    assert_eq!(core.list().len(), 1);
}

#[test]
fn elevated_and_own_windows_are_refused_with_a_clear_error() {
    let (fake, mut core) = setup();
    let e = core.lend(66, Mode::Visible).unwrap_err().to_string();
    assert!(e.contains("administrator"), "{e}");
    let e = core.lend(77, Mode::Visible).unwrap_err().to_string();
    assert!(e.contains("our own"), "{e}");
    assert!(matches!(core.lend(999, Mode::Visible), Err(LendError::Gone)));
    assert!(core.list().is_empty());
    assert!(fake.0.lock().borders.is_empty());
}

#[test]
fn hidden_lend_is_see_through_with_a_hidden_border_and_bring_back_restores() {
    let (fake, mut core) = setup();
    let v = core.lend(10, Mode::Hidden).unwrap();
    assert!(fake.0.lock().see_through.contains(&10));
    assert_eq!(shown(&fake, 10), Some(Shown::Hidden));
    core.bring_back(&v.id).unwrap();
    assert!(!fake.0.lock().see_through.contains(&10));
    assert!(fake.0.lock().borders.is_empty());
    assert!(core.list().is_empty());
    assert!(matches!(core.bring_back(&v.id), Err(LendError::NoSuchApp)));
}

#[test]
fn a_failed_border_leaves_nothing_lent() {
    let (fake, mut core) = setup();
    fake.0.lock().border_fails = true;
    assert!(core.lend(10, Mode::Hidden).is_err());
    assert!(core.list().is_empty());
    assert!(fake.0.lock().see_through.is_empty());
}

#[test]
fn set_mode_toggles_the_style_and_the_border() {
    let (fake, mut core) = setup();
    let id = core.lend(10, Mode::Visible).unwrap().id;
    core.set_mode(&id, Mode::Hidden).unwrap();
    assert!(fake.0.lock().see_through.contains(&10));
    assert_eq!(shown(&fake, 10), Some(Shown::Hidden));
    core.set_mode(&id, Mode::Visible).unwrap();
    assert!(!fake.0.lock().see_through.contains(&10));
    assert_eq!(shown(&fake, 10), Some(Shown::Bot));
}

#[test]
fn phases_free_bot_you_free_and_the_lease_check() {
    let (fake, mut core) = setup();
    let id = core.lend(10, Mode::Visible).unwrap().id;
    core.bot_begin(&id).unwrap();
    assert_eq!(core.view(&id).unwrap().phase, Phase::Bot);
    core.take_over(&id).unwrap();
    assert_eq!(core.view(&id).unwrap().phase, Phase::You);
    assert_eq!(shown(&fake, 10), Some(Shown::You));
    assert!(matches!(core.bot_begin(&id), Err(LendError::PersonHasControl)));
    assert!(matches!(core.begin_act(&id), Err(LendError::PersonHasControl)));
    core.hand_back(&id).unwrap();
    assert_eq!(core.view(&id).unwrap().phase, Phase::Free);
    core.bot_begin(&id).unwrap();
    assert_eq!(core.view(&id).unwrap().phase, Phase::Bot);
}

#[test]
fn needs_you_blocks_the_bot_until_hand_back_and_is_refused_while_you_have_control() {
    let (fake, mut core) = setup();
    let id = core.lend(10, Mode::Visible).unwrap().id;
    let long = "x".repeat(500);
    let v = core.needs_you(&id, &long).unwrap();
    assert_eq!(v.phase, Phase::NeedsYou);
    assert_eq!(v.reason.unwrap().chars().count(), 200);
    assert_eq!(shown(&fake, 10), Some(Shown::NeedsYou));
    assert!(matches!(core.bot_begin(&id), Err(LendError::PersonHasControl)));
    core.take_over(&id).unwrap();
    assert!(core.view(&id).unwrap().reason.is_none());
    assert!(matches!(core.needs_you(&id, "code"), Err(LendError::PersonHasControl)));
    core.hand_back(&id).unwrap();
    core.needs_you(&id, "code").unwrap();
    core.hand_back(&id).unwrap();
    assert_eq!(core.view(&id).unwrap().phase, Phase::Free);
}

#[test]
fn hidden_mode_shows_the_app_when_the_person_takes_over_or_is_asked() {
    let (fake, mut core) = setup();
    let id = core.lend(10, Mode::Hidden).unwrap().id;
    core.take_over(&id).unwrap();
    assert_eq!(core.view(&id).unwrap().mode, Mode::Visible);
    assert!(!fake.0.lock().see_through.contains(&10));
    // hiding is refused while the person has it
    assert!(core.set_mode(&id, Mode::Hidden).is_err());
    core.hand_back(&id).unwrap();
    core.set_mode(&id, Mode::Hidden).unwrap();
    core.needs_you(&id, "look").unwrap();
    assert_eq!(core.view(&id).unwrap().mode, Mode::Visible);
}

#[test]
fn event_to_action_mapping() {
    use Action::*;
    assert_eq!(action_for(Ev::TabClicked(Shown::Bot), Phase::Bot), TakeOver);
    assert_eq!(action_for(Ev::TabClicked(Shown::NeedsYou), Phase::NeedsYou), TakeOver);
    assert_eq!(action_for(Ev::TabClicked(Shown::You), Phase::You), HandBack);
    assert_eq!(action_for(Ev::TargetGone, Phase::Free), Remove);
    assert_eq!(action_for(Ev::UserFocused, Phase::Bot), TakeOver);
    assert_eq!(action_for(Ev::UserFocused, Phase::NeedsYou), TakeOver);
    assert_eq!(action_for(Ev::UserFocused, Phase::Free), Nothing);
    assert_eq!(action_for(Ev::UserFocused, Phase::You), Nothing);
    assert_eq!(shown_for(Mode::Hidden, Phase::You), Shown::Hidden);
    assert_eq!(shown_for(Mode::Visible, Phase::Free), Shown::Bot);
}

#[test]
fn tick_applies_border_events() {
    let (fake, mut core) = setup();
    let id = core.lend(10, Mode::Visible).unwrap().id;
    assert!(!core.tick());
    core.bot_begin(&id).unwrap();
    push(&fake, 10, Ev::TabClicked(Shown::Bot));
    assert!(core.tick());
    assert_eq!(core.view(&id).unwrap().phase, Phase::You);
    push(&fake, 10, Ev::TabClicked(Shown::You));
    assert!(core.tick());
    assert_eq!(core.view(&id).unwrap().phase, Phase::Free);
    core.bot_begin(&id).unwrap();
    push(&fake, 10, Ev::UserFocused);
    assert!(core.tick());
    assert_eq!(core.view(&id).unwrap().phase, Phase::You);
}

#[test]
fn target_gone_event_and_a_dead_window_drop_the_lend() {
    let (fake, mut core) = setup();
    core.lend(10, Mode::Hidden).unwrap();
    core.lend(11, Mode::Visible).unwrap();
    push(&fake, 10, Ev::TargetGone);
    assert!(core.tick());
    assert_eq!(core.list().len(), 1);
    assert!(!fake.0.lock().borders.contains(&10));
    fake.0.lock().dead.insert(11);
    assert!(core.tick());
    assert!(core.list().is_empty());
    assert!(fake.0.lock().borders.is_empty());
}

#[test]
fn bot_begin_on_a_dead_window_drops_it_and_says_gone() {
    let (fake, mut core) = setup();
    let id = core.lend(10, Mode::Visible).unwrap().id;
    fake.0.lock().dead.insert(10);
    assert!(matches!(core.bot_begin(&id), Err(LendError::Gone)));
    assert!(core.list().is_empty());
}

#[test]
fn snapshot_is_kept_for_element_lookups() {
    let (_fake, mut core) = setup();
    let id = core.lend(10, Mode::Visible).unwrap().id;
    assert_eq!(core.element(&id, "e1").unwrap(), None);
    core.snapshot(&id).unwrap();
    assert_eq!(core.element(&id, "e2").unwrap().as_deref(), Some("e2"));
    assert_eq!(core.element(&id, "e9").unwrap(), None);
    assert!(matches!(core.element("nope", "e1"), Err(LendError::NoSuchApp)));
}

#[test]
fn reading_is_allowed_while_the_person_has_control_but_acting_is_not() {
    let (fake, mut core) = setup();
    let id = core.lend(10, Mode::Visible).unwrap().id;
    core.take_over(&id).unwrap();
    assert!(core.snapshot(&id).is_ok());
    assert!(core.target(&id).is_ok());
    assert!(matches!(core.begin_act(&id), Err(LendError::PersonHasControl)));
    assert!(!fake.0.lock().log.iter().any(|l| l.starts_with("click")));
    core.hand_back(&id).unwrap();
    let (os, h) = core.begin_act(&id).unwrap();
    os.click(h, 3, 4).unwrap();
    assert!(fake.0.lock().log.contains(&"click 10 3,4".to_string()));
}

#[test]
fn dropping_the_core_restores_every_hidden_window() {
    let (fake, mut core) = setup();
    core.lend(10, Mode::Hidden).unwrap();
    core.lend(11, Mode::Hidden).unwrap();
    assert_eq!(fake.0.lock().see_through.len(), 2);
    drop(core);
    assert!(fake.0.lock().see_through.is_empty());
    assert!(fake.0.lock().borders.is_empty());
}

#[test]
fn mode_parse_and_view_json() {
    assert_eq!(Mode::parse("Hidden"), Some(Mode::Hidden));
    assert_eq!(Mode::parse("nope"), None);
    let (_f, mut core) = setup();
    let id = core.lend(10, Mode::Visible).unwrap().id;
    core.needs_you(&id, "r").unwrap();
    let j = serde_json::to_value(core.view(&id).unwrap()).unwrap();
    assert_eq!(j["phase"], "needs_you");
    assert_eq!(j["appName"], "LINE");
    assert_eq!(j["mode"], "visible");
}

#[test]
fn nearest_downscale_never_enlarges_and_keeps_aspect() {
    let px = vec![7u8; 640 * 480 * 4];
    #[cfg(windows)]
    {
        let (w, h, out) = super::capture::downscale_nearest(&px, 640, 480, 320);
        assert_eq!((w, h, out.len()), (320, 240, 320 * 240 * 4));
        let (w, h, out) = super::capture::downscale_nearest(&px, 640, 480, 1000);
        assert_eq!((w, h, out.len()), (640, 480, px.len()));
    }
    #[cfg(not(windows))]
    let _ = px;
}
