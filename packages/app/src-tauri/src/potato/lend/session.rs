//! The lent apps: who has each one right now, its border, its mode and its cleanup.
//!
//! - `Core<O>` is the whole state machine. Every call to Windows goes through the small `Os` trait, so
//!   `session_tests.rs` runs it with a fake and never touches a real window.
//! - `Hub` is the app's one registry (`hub()`), a `Core` over the real OS behind a mutex. It emits the Tauri
//!   event `potato://lend` after every change, and a 100 ms keeper thread (`start`) turns the border's button
//!   presses into take over / hand back and drops lends whose window has gone.
//! - Phases use the same words as the sandbox screen's banner (`sandbox/lease.rs`): free, bot, you, needs_you.
//!   The person's control always wins: a bot call under `you` or `needs_you` is `PersonHasControl`.
//! - Hidden mode is see-through and click-through, so the person cannot use the app; taking over (or a
//!   "needs you" card) therefore shows the app again first.
//!
//! Non-Windows builds use `NoOs`: every call answers `Unsupported`.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;

use super::{Hwnd, LendError, Result, WinInfo};

/// The page reads `potato_lend_list` again whenever this fires. No payload.
pub const LEND_EVENT: &str = "potato://lend";
const REASON_CHARS: usize = 200;
const SNAPSHOT_MAX_NODES: usize = 400;
const SNAPSHOT_TIMEOUT_MS: u64 = 3000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Visible,
    Hidden,
}

impl Mode {
    pub fn parse(text: &str) -> Option<Mode> {
        match text.trim().to_ascii_lowercase().as_str() {
            "visible" => Some(Mode::Visible),
            "hidden" => Some(Mode::Hidden),
            _ => None,
        }
    }
}

/// Who has the app. Same words as `lease.rs` `Banner`: free / bot is using it / you have control / needs you.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Free,
    Bot,
    You,
    NeedsYou,
}

/// What the border shows (mirrors `border::State`, which is Windows only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    Bot,
    You,
    NeedsYou,
    Hidden,
}

/// What the border reports (mirrors `border::BorderEvent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ev {
    /// The tab button was clicked; carries the state shown at click time.
    TabClicked(Shown),
    UserFocused,
    /// The lent window itself became the foreground window (taskbar click, alt-tab).
    Foregrounded,
    /// The lent window was minimized.
    Minimized,
    TargetGone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    TakeOver,
    HandBack,
    Remove,
    /// Minimized: make it see-through, bring it back from the taskbar without activating it, keep it working.
    HideMinimized,
    Nothing,
}

/// Pure: what a border event does, given who has the app.
pub fn action_for(ev: Ev, phase: Phase) -> Action {
    match ev {
        Ev::TargetGone => Action::Remove,
        Ev::TabClicked(Shown::Bot) | Ev::TabClicked(Shown::NeedsYou) => Action::TakeOver,
        Ev::TabClicked(Shown::You) => Action::HandBack,
        Ev::TabClicked(Shown::Hidden) => Action::Nothing,
        Ev::Minimized => Action::HideMinimized,
        // Only a hidden app treats this as "the person took control" (see `action_for_mode`).
        Ev::Foregrounded => Action::Nothing,
        // The person used the app: that is a take over, but only when the bot had it or asked for it.
        Ev::UserFocused => match phase {
            Phase::Bot | Phase::NeedsYou => Action::TakeOver,
            Phase::Free | Phase::You => Action::Nothing,
        },
    }
}

/// Pure: `action_for`, knowing the mode. A hidden app coming to the foreground means the person took control of it.
pub fn action_for_mode(ev: Ev, mode: Mode, phase: Phase) -> Action {
    match (ev, mode) {
        (Ev::Foregrounded, Mode::Hidden) => Action::TakeOver,
        _ => action_for(ev, phase),
    }
}

/// Pure: what the border shows for a mode and phase.
pub fn shown_for(mode: Mode, phase: Phase) -> Shown {
    if mode == Mode::Hidden {
        return Shown::Hidden;
    }
    match phase {
        Phase::Free | Phase::Bot => Shown::Bot,
        Phase::You => Shown::You,
        Phase::NeedsYou => Shown::NeedsYou,
    }
}

/// The window style `Os::see_through` changed, handed back to `Os::restore`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Saved {
    pub ex_style: isize,
    pub was_layered: bool,
    pub alpha: Option<u8>,
    pub was_topmost: bool,
}

pub trait BorderCtl: Send {
    fn set_state(&self, state: Shown);
    fn poll(&self) -> Option<Ev>;
}

/// One input to a lent window. Coordinates are client pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Click(i32, i32),
    DoubleClick(i32, i32),
    /// The second half of a double click only.
    DoubleClickSecond(i32, i32),
    Type(String),
    Key(u16),
    Scroll(i32, i32, i32),
}

impl Input {
    fn send<O: Os>(&self, os: &O, h: Hwnd) -> Result<()> {
        match self {
            Input::Click(x, y) => os.click(h, *x, *y),
            Input::DoubleClick(x, y) => os.double_click(h, *x, *y),
            Input::DoubleClickSecond(x, y) => os.double_click_second(h, *x, *y),
            Input::Type(t) => os.type_text(h, t),
            Input::Key(vk) => os.key(h, *vk),
            Input::Scroll(x, y, d) => os.scroll(h, *x, *y, *d),
        }
    }
}

/// Pure: did the foreground window change between two looks?
pub fn foreground_changed(before: Hwnd, after: Hwnd) -> bool {
    before != after
}

/// Every call to Windows the lend layer makes.
pub trait Os: Send + Sync + 'static {
    type Snap: Send + Sync;
    type Elem: Clone + Send;
    fn info(&self, h: Hwnd) -> Result<WinInfo>;
    fn is_alive(&self, h: Hwnd) -> bool;
    fn own_pid(&self) -> u32;
    /// The window that has the foreground right now (0 for none). Read-only; used only to check that lending
    /// and un-lending never change it.
    fn foreground(&self) -> Hwnd {
        0
    }
    /// Once, when a window is lent (the hub calls it before taking its lock): anything the window needs so its controls can be read (never activates it).
    fn prepare(&self, _h: Hwnd) {}
    fn see_through(&self, h: Hwnd) -> Result<Saved>;
    fn restore(&self, h: Hwnd, saved: Saved) -> Result<()>;
    /// Bring a minimized window back WITHOUT activating it. Does nothing when it is not minimized.
    fn restore_minimized(&self, _h: Hwnd) -> Result<()> {
        Ok(())
    }
    fn start_border(&self, h: Hwnd, label: String) -> Result<Box<dyn BorderCtl>>;
    fn snapshot(&self, h: Hwnd) -> Result<Self::Snap>;
    fn find(&self, snap: &Self::Snap, r#ref: &str) -> Option<Self::Elem>;
    fn capture(&self, h: Hwnd) -> Result<Vec<u8>>;
    fn click(&self, h: Hwnd, x: i32, y: i32) -> Result<()>;
    fn double_click(&self, h: Hwnd, x: i32, y: i32) -> Result<()>;
    /// Only the second half of a double click (WM_LBUTTONDBLCLK + button up): the viewer already sent the first click.
    fn double_click_second(&self, _h: Hwnd, _x: i32, _y: i32) -> Result<()> {
        Err(LendError::Unsupported)
    }
    fn type_text(&self, h: Hwnd, text: &str) -> Result<()>;
    fn key(&self, h: Hwnd, vk: u16) -> Result<()>;
    fn scroll(&self, h: Hwnd, x: i32, y: i32, delta: i32) -> Result<()>;
}

/// One lent app as the page sees it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LentView {
    pub id: String,
    pub hwnd: Hwnd,
    pub pid: u32,
    pub app_name: String,
    pub title: String,
    pub mode: Mode,
    pub phase: Phase,
    /// Why the bot needs the person (phase `needs_you` only).
    pub reason: Option<String>,
    /// False when the app's picture looks frozen (minimized or blank) even after it was made see-through.
    pub picture_live: bool,
}

/// What the mirror saw in one look at the app's picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture {
    Live,
    /// A frame of one colour or near black (maybe just a dark loading screen: never hides the app).
    Blank,
    /// The window is minimized, so it cannot draw.
    Minimized,
}

/// How many paused pictures in a row before the picture is reported as paused.
const PAUSED_IN_A_ROW: u8 = 3;

struct Entry<O: Os> {
    id: String,
    hwnd: Hwnd,
    pid: u32,
    app_name: String,
    title: String,
    mode: Mode,
    phase: Phase,
    reason: Option<String>,
    saved: Option<Saved>,
    border: Box<dyn BorderCtl>,
    snap: Option<Arc<O::Snap>>,
    picture_live: bool,
    /// Paused pictures seen since the picture last drew (or since the hide fix).
    paused_seen: u8,
    /// The hide fix for a paused picture was already tried.
    fix_tried: bool,
}

impl<O: Os> Entry<O> {
    fn view(&self) -> LentView {
        LentView {
            id: self.id.clone(),
            hwnd: self.hwnd,
            pid: self.pid,
            app_name: self.app_name.clone(),
            title: self.title.clone(),
            mode: self.mode,
            phase: self.phase,
            reason: self.reason.clone(),
            picture_live: self.picture_live,
        }
    }
}

pub struct Core<O: Os> {
    os: Arc<O>,
    entries: Vec<Entry<O>>,
    next: u64,
}

impl<O: Os> Core<O> {
    pub fn new(os: Arc<O>) -> Self {
        Core { os, entries: Vec::new(), next: 1 }
    }

    fn at(&self, id: &str) -> Result<usize> {
        self.entries.iter().position(|e| e.id == id).ok_or(LendError::NoSuchApp)
    }

    /// Log (debug) the foreground window before and after `what`, and warn if it changed.
    fn watch_foreground(&self, what: &str, hwnd: Hwnd, before: Hwnd) {
        let after = self.os.foreground();
        tracing::debug!(target: "potato::lend", hwnd, before, after, "foreground around {what}");
        if foreground_changed(before, after) {
            tracing::warn!(target: "potato::lend", hwnd, before, after, "the foreground window changed while a lend {what}");
        }
    }

    /// Stop the border and put the window back as it was. The window may already be gone.
    fn release(&self, mut entry: Entry<O>) {
        let before = self.os.foreground();
        if let Some(saved) = entry.saved.take() {
            let _ = self.os.restore(entry.hwnd, saved);
        }
        let hwnd = entry.hwnd;
        drop(entry.border);
        self.watch_foreground("ended", hwnd, before);
    }

    fn show(&self, i: usize) {
        let e = &self.entries[i];
        e.border.set_state(shown_for(e.mode, e.phase));
    }

    /// Hidden -> Visible (restore the style, draw the border).
    fn make_visible(&mut self, i: usize) {
        if self.entries[i].mode == Mode::Visible {
            return;
        }
        if let Some(saved) = self.entries[i].saved.take() {
            let _ = self.os.restore(self.entries[i].hwnd, saved);
        }
        self.entries[i].mode = Mode::Visible;
    }

    pub fn lend(&mut self, hwnd: Hwnd, mode: Mode) -> Result<LentView> {
        if let Some(e) = self.entries.iter().find(|e| e.hwnd == hwnd) {
            return Ok(e.view());
        }
        let info = self.os.info(hwnd)?;
        if info.elevated {
            return Err(LendError::Os("This app runs as administrator, so it can't be lent.".into()));
        }
        if info.pid == self.os.own_pid() {
            return Err(LendError::Os("This is one of our own windows, so it can't be lent.".into()));
        }
        let fg_before = self.os.foreground();
        let border = self.os.start_border(hwnd, info.app_name.clone())?;
        let saved = if mode == Mode::Hidden {
            // The border is dropped (stopped) if this fails.
            Some(self.os.see_through(hwnd)?)
        } else {
            None
        };
        border.set_state(shown_for(mode, Phase::Free));
        self.watch_foreground("started", hwnd, fg_before);
        let id = format!("l{}", self.next);
        self.next += 1;
        let entry = Entry {
            id,
            hwnd,
            pid: info.pid,
            app_name: info.app_name,
            title: info.title,
            mode,
            phase: Phase::Free,
            reason: None,
            saved,
            border,
            snap: None,
            picture_live: true,
            paused_seen: 0,
            fix_tried: false,
        };
        let view = entry.view();
        self.entries.push(entry);
        Ok(view)
    }

    pub fn bring_back(&mut self, id: &str) -> Result<()> {
        let i = self.at(id)?;
        let entry = self.entries.remove(i);
        self.release(entry);
        Ok(())
    }

    pub fn list(&self) -> Vec<LentView> {
        self.entries.iter().map(Entry::view).collect()
    }

    pub fn view(&self, id: &str) -> Result<LentView> {
        Ok(self.entries[self.at(id)?].view())
    }

    pub fn set_mode(&mut self, id: &str, mode: Mode) -> Result<LentView> {
        let i = self.at(id)?;
        if self.entries[i].mode == mode {
            return Ok(self.entries[i].view());
        }
        match mode {
            Mode::Hidden => {
                if matches!(self.entries[i].phase, Phase::You | Phase::NeedsYou) {
                    return Err(LendError::Os("You have this app right now. Hand it back before hiding it.".into()));
                }
                let saved = self.os.see_through(self.entries[i].hwnd)?;
                self.entries[i].saved = Some(saved);
                self.entries[i].mode = Mode::Hidden;
            }
            Mode::Visible => self.make_visible(i),
        }
        self.show(i);
        Ok(self.entries[i].view())
    }

    /// The person minimized the app: it would stop drawing, so make it see-through (any mode), bring it back from the
    /// taskbar without activating it, and keep it working. If the person has it, they are done with it: it is handed back.
    /// True when the view changed.
    fn hide_minimized(&mut self, i: usize) -> bool {
        let before = self.entries[i].view();
        if self.entries[i].mode == Mode::Visible {
            match self.os.see_through(self.entries[i].hwnd) {
                Ok(saved) => {
                    self.entries[i].saved = Some(saved);
                    self.entries[i].mode = Mode::Hidden;
                }
                Err(_) => return false,
            }
        }
        if matches!(self.entries[i].phase, Phase::You | Phase::NeedsYou) {
            self.entries[i].phase = Phase::Free;
            self.entries[i].reason = None;
        }
        let _ = self.os.restore_minimized(self.entries[i].hwnd);
        self.show(i);
        self.entries[i].view() != before
    }

    /// The mirror looked at the app's picture. A minimized visible app is fixed by hiding it (see-through and uncoverable);
    /// a blank frame never hides anything. `PAUSED_IN_A_ROW` paused pictures in a row (blank or minimized) turn
    /// `picture_live` false; a live one turns it back. Returns true when the view changed.
    pub fn picture_report(&mut self, id: &str, picture: Picture) -> Result<bool> {
        let i = self.at(id)?;
        if picture == Picture::Live {
            let e = &mut self.entries[i];
            let changed = !e.picture_live;
            e.picture_live = true;
            e.paused_seen = 0;
            e.fix_tried = false;
            return Ok(changed);
        }
        let person = matches!(self.entries[i].phase, Phase::You | Phase::NeedsYou);
        if picture == Picture::Minimized && !self.entries[i].fix_tried && self.entries[i].mode == Mode::Visible && !person {
            self.entries[i].fix_tried = true;
            self.entries[i].paused_seen = 0;
            return Ok(self.hide_minimized(i));
        }
        if picture == Picture::Minimized && self.entries[i].mode == Mode::Hidden {
            let _ = self.os.restore_minimized(self.entries[i].hwnd);
        }
        let e = &mut self.entries[i];
        e.paused_seen = e.paused_seen.saturating_add(1);
        if e.paused_seen >= PAUSED_IN_A_ROW && e.picture_live {
            e.picture_live = false;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn take_over(&mut self, id: &str) -> Result<LentView> {
        let i = self.at(id)?;
        self.make_visible(i);
        self.entries[i].phase = Phase::You;
        self.entries[i].reason = None;
        self.show(i);
        Ok(self.entries[i].view())
    }

    /// The person is done: the app is free again (a bot call takes it with `bot_begin`).
    pub fn hand_back(&mut self, id: &str) -> Result<LentView> {
        let i = self.at(id)?;
        if matches!(self.entries[i].phase, Phase::You | Phase::NeedsYou) {
            self.entries[i].phase = Phase::Free;
            self.entries[i].reason = None;
            self.show(i);
        }
        Ok(self.entries[i].view())
    }

    /// The bot asks the person to step in. Refused while the person already has the app.
    pub fn needs_you(&mut self, id: &str, reason: &str) -> Result<LentView> {
        let i = self.at(id)?;
        if self.entries[i].phase == Phase::You {
            return Err(LendError::PersonHasControl);
        }
        self.make_visible(i);
        self.entries[i].phase = Phase::NeedsYou;
        self.entries[i].reason = Some(reason.chars().take(REASON_CHARS).collect());
        self.show(i);
        Ok(self.entries[i].view())
    }

    /// The lease check: the bot is about to use the app. `PersonHasControl` when the person has it or was asked to.
    pub fn bot_begin(&mut self, id: &str) -> Result<()> {
        let i = self.at(id)?;
        if !self.os.is_alive(self.entries[i].hwnd) {
            let entry = self.entries.remove(i);
            self.release(entry);
            return Err(LendError::Gone);
        }
        match self.entries[i].phase {
            Phase::You | Phase::NeedsYou => Err(LendError::PersonHasControl),
            Phase::Free | Phase::Bot => {
                if self.entries[i].phase != Phase::Bot {
                    self.entries[i].phase = Phase::Bot;
                    self.show(i);
                }
                Ok(())
            }
        }
    }

    /// Lease check for an action; gives back what the caller needs to act WITHOUT holding the registry lock.
    pub fn begin_act(&mut self, id: &str) -> Result<(Arc<O>, Hwnd)> {
        self.bot_begin(id)?;
        let i = self.at(id)?;
        Ok((self.os.clone(), self.entries[i].hwnd))
    }

    /// No lease check: reading the app is allowed while the person has it.
    pub fn target(&self, id: &str) -> Result<(Arc<O>, Hwnd)> {
        let i = self.at(id)?;
        Ok((self.os.clone(), self.entries[i].hwnd))
    }

    pub fn store_snapshot(&mut self, id: &str, snap: O::Snap) -> Result<Arc<O::Snap>> {
        let i = self.at(id)?;
        let snap = Arc::new(snap);
        self.entries[i].snap = Some(snap.clone());
        Ok(snap)
    }

    /// Take (and keep) a new snapshot of the app's controls.
    #[cfg_attr(not(test), allow(dead_code))] // the hub takes the snapshot without holding the lock (see `Hub::snapshot`)
    pub fn snapshot(&mut self, id: &str) -> Result<Arc<O::Snap>> {
        let (os, h) = self.target(id)?;
        let snap = os.snapshot(h)?;
        self.store_snapshot(id, snap)
    }

    pub fn cached(&self, id: &str) -> Option<Arc<O::Snap>> {
        let i = self.at(id).ok()?;
        self.entries[i].snap.clone()
    }

    /// An element of the latest snapshot (`None` when there is no snapshot yet or no such ref).
    pub fn element(&self, id: &str, r#ref: &str) -> Result<Option<O::Elem>> {
        let i = self.at(id)?;
        Ok(self.entries[i].snap.as_ref().and_then(|s| self.os.find(s, r#ref)))
    }

    /// Handle the border's events and drop lends whose window has gone. True when anything changed.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        let mut i = 0;
        while i < self.entries.len() {
            let mut remove = !self.os.is_alive(self.entries[i].hwnd);
            let mut acts = Vec::new();
            while let Some(ev) = self.entries[i].border.poll() {
                acts.push(action_for_mode(ev, self.entries[i].mode, self.entries[i].phase));
                match acts.last() {
                    Some(Action::Remove) => break,
                    Some(Action::HideMinimized) => {
                        if self.hide_minimized(i) {
                            changed = true;
                        }
                    }
                    Some(Action::TakeOver) => self.apply_border_action(i, Action::TakeOver, &mut changed),
                    Some(Action::HandBack) => self.apply_border_action(i, Action::HandBack, &mut changed),
                    _ => {}
                }
            }
            if acts.contains(&Action::Remove) {
                remove = true;
            }
            if remove {
                let entry = self.entries.remove(i);
                self.release(entry);
                changed = true;
            } else {
                i += 1;
            }
        }
        changed
    }

    fn apply_border_action(&mut self, i: usize, action: Action, changed: &mut bool) {
        let id = self.entries[i].id.clone();
        let before = self.entries[i].view();
        let _ = match action {
            Action::TakeOver => self.take_over(&id),
            _ => self.hand_back(&id),
        };
        if self.entries[i].view() != before {
            *changed = true;
        }
    }

    /// Restore every window and stop every border (the app is closing).
    pub fn release_all(&mut self) {
        for entry in std::mem::take(&mut self.entries) {
            self.release(entry);
        }
    }
}

impl<O: Os> Drop for Core<O> {
    fn drop(&mut self) {
        self.release_all();
    }
}

// ---- the real OS -----------------------------------------------------------------------------------

#[cfg(windows)]
pub struct RealOs;

#[cfg(windows)]
mod real {
    use super::*;
    use crate::potato::lend::border::{Border, BorderEvent, State};
    use crate::potato::lend::{a11y, capture, hide, input, win};

    fn to_state(s: Shown) -> State {
        match s {
            Shown::Bot => State::Bot,
            Shown::You => State::You,
            Shown::NeedsYou => State::NeedsYou,
            Shown::Hidden => State::Hidden,
        }
    }

    fn from_state(s: State) -> Shown {
        match s {
            State::Bot => Shown::Bot,
            State::You => Shown::You,
            State::NeedsYou => Shown::NeedsYou,
            State::Hidden => Shown::Hidden,
        }
    }

    struct WinBorder(Border);

    impl BorderCtl for WinBorder {
        fn set_state(&self, state: Shown) {
            self.0.set_state(to_state(state));
        }
        fn poll(&self) -> Option<Ev> {
            self.0.events().try_recv().ok().map(|ev| match ev {
                BorderEvent::TabClicked(s) => Ev::TabClicked(from_state(s)),
                BorderEvent::UserFocused => Ev::UserFocused,
                BorderEvent::Foregrounded => Ev::Foregrounded,
                BorderEvent::Minimized => Ev::Minimized,
                BorderEvent::TargetGone => Ev::TargetGone,
            })
        }
    }

    impl Os for RealOs {
        type Snap = a11y::Snapshot;
        type Elem = a11y::Element;
        fn info(&self, h: Hwnd) -> Result<WinInfo> {
            win::info(h)
        }
        fn is_alive(&self, h: Hwnd) -> bool {
            win::is_alive(h)
        }
        fn own_pid(&self) -> u32 {
            std::process::id()
        }
        fn foreground(&self) -> Hwnd {
            win::foreground()
        }
        fn prepare(&self, h: Hwnd) {
            a11y::nudge_chromium(h);
        }
        fn see_through(&self, h: Hwnd) -> Result<Saved> {
            let s = hide::make_see_through(h)?;
            Ok(Saved { ex_style: s.ex_style, was_layered: s.was_layered, alpha: s.alpha, was_topmost: s.was_topmost })
        }
        fn restore(&self, h: Hwnd, saved: Saved) -> Result<()> {
            hide::restore(h, hide::SavedStyle { ex_style: saved.ex_style, was_layered: saved.was_layered, alpha: saved.alpha, was_topmost: saved.was_topmost })
        }
        fn restore_minimized(&self, h: Hwnd) -> Result<()> {
            if hide::is_iconic(h) {
                hide::show_no_activate(h)?;
            }
            Ok(())
        }
        fn start_border(&self, h: Hwnd, label: String) -> Result<Box<dyn BorderCtl>> {
            Ok(Box::new(WinBorder(Border::start(h, label)?)))
        }
        fn snapshot(&self, h: Hwnd) -> Result<a11y::Snapshot> {
            a11y::snapshot_nudged(h, SNAPSHOT_MAX_NODES, SNAPSHOT_TIMEOUT_MS)
        }
        fn find(&self, snap: &a11y::Snapshot, r: &str) -> Option<a11y::Element> {
            a11y::find(snap, r).cloned()
        }
        fn capture(&self, h: Hwnd) -> Result<Vec<u8>> {
            // A JPEG no bigger than 1280 px on its long side: far faster to make and to send than the full-size PNG.
            capture::jpeg_scaled(h, capture::OBSERVE_MAX_SIDE, capture::OBSERVE_JPEG_QUALITY).map(|(jpeg, ..)| jpeg)
        }
        fn click(&self, h: Hwnd, x: i32, y: i32) -> Result<()> {
            input::click(h, x, y)
        }
        fn double_click(&self, h: Hwnd, x: i32, y: i32) -> Result<()> {
            input::double_click(h, x, y)
        }
        fn double_click_second(&self, h: Hwnd, x: i32, y: i32) -> Result<()> {
            input::double_click_second(h, x, y)
        }
        fn type_text(&self, h: Hwnd, text: &str) -> Result<()> {
            input::type_text(h, text)
        }
        fn key(&self, h: Hwnd, vk: u16) -> Result<()> {
            input::key(h, vk)
        }
        fn scroll(&self, h: Hwnd, x: i32, y: i32, delta: i32) -> Result<()> {
            input::scroll(h, x, y, delta)
        }
    }
}

/// Other systems: nothing can be lent.
#[cfg(not(windows))]
pub struct RealOs;

#[cfg(not(windows))]
impl Os for RealOs {
    type Snap = ();
    type Elem = ();
    fn info(&self, _: Hwnd) -> Result<WinInfo> {
        Err(LendError::Unsupported)
    }
    fn is_alive(&self, _: Hwnd) -> bool {
        false
    }
    fn own_pid(&self) -> u32 {
        std::process::id()
    }
    fn see_through(&self, _: Hwnd) -> Result<Saved> {
        Err(LendError::Unsupported)
    }
    fn restore(&self, _: Hwnd, _: Saved) -> Result<()> {
        Err(LendError::Unsupported)
    }
    fn start_border(&self, _: Hwnd, _: String) -> Result<Box<dyn BorderCtl>> {
        Err(LendError::Unsupported)
    }
    fn snapshot(&self, _: Hwnd) -> Result<()> {
        Err(LendError::Unsupported)
    }
    fn find(&self, _: &(), _: &str) -> Option<()> {
        None
    }
    fn capture(&self, _: Hwnd) -> Result<Vec<u8>> {
        Err(LendError::Unsupported)
    }
    fn click(&self, _: Hwnd, _: i32, _: i32) -> Result<()> {
        Err(LendError::Unsupported)
    }
    fn double_click(&self, _: Hwnd, _: i32, _: i32) -> Result<()> {
        Err(LendError::Unsupported)
    }
    fn type_text(&self, _: Hwnd, _: &str) -> Result<()> {
        Err(LendError::Unsupported)
    }
    fn key(&self, _: Hwnd, _: u16) -> Result<()> {
        Err(LendError::Unsupported)
    }
    fn scroll(&self, _: Hwnd, _: i32, _: i32, _: i32) -> Result<()> {
        Err(LendError::Unsupported)
    }
}

/// The snapshot of the real OS (S5 reads `elements` from it).
pub type Snap = <RealOs as Os>::Snap;
/// One element of it.
pub type Elem = <RealOs as Os>::Elem;

// ---- the app's one hub -----------------------------------------------------------------------------

type Listener = Arc<dyn Fn() + Send + Sync>;

pub struct Hub {
    core: Mutex<Core<RealOs>>,
    listener: Mutex<Option<Listener>>,
}

static HUB: OnceLock<Hub> = OnceLock::new();

/// The process-wide registry (made on first use).
pub fn hub() -> &'static Hub {
    HUB.get_or_init(|| Hub { core: Mutex::new(Core::new(Arc::new(RealOs))), listener: Mutex::new(None) })
}

impl Hub {
    fn emit(&self) {
        // Lend an App v2: start or stop the sandbox-screen mirrors to match the list (the lock is free here).
        #[cfg(not(test))]
        {
            let apps: Vec<super::mirror::Lent> = self
                .list()
                .into_iter()
                .map(|v| super::mirror::Lent { title: if v.title.is_empty() { v.app_name.clone() } else { v.title.clone() }, id: v.id, hwnd: v.hwnd })
                .collect();
            super::mirror::reconcile(&apps);
        }
        let listener = self.listener.lock().clone();
        if let Some(listener) = listener {
            listener();
        }
    }

    /// Run a change under the lock, then tell the page.
    fn change<T>(&self, f: impl FnOnce(&mut Core<RealOs>) -> Result<T>) -> Result<T> {
        let out = f(&mut self.core.lock());
        if out.is_ok() {
            self.emit();
        }
        out
    }

    /// Like `change`, for calls that answer whether anything changed: emits only when it did.
    fn change_if(&self, f: impl FnOnce(&mut Core<RealOs>) -> Result<bool>) -> Result<bool> {
        let out = f(&mut self.core.lock());
        if matches!(out, Ok(true)) {
            self.emit();
        }
        out
    }

    pub fn set_listener(&self, listener: Option<Listener>) {
        *self.listener.lock() = listener;
    }

    pub fn lend(&self, hwnd: Hwnd, mode: Mode) -> Result<LentView> {
        // Ask a Chromium window for its controls BEFORE taking the registry lock (it can take up to ~1 s; once per window).
        RealOs.prepare(hwnd);
        self.change(|c| c.lend(hwnd, mode))
    }
    pub fn bring_back(&self, id: &str) -> Result<()> {
        self.change(|c| c.bring_back(id))
    }
    pub fn list(&self) -> Vec<LentView> {
        self.core.lock().list()
    }
    pub fn view(&self, id: &str) -> Result<LentView> {
        self.core.lock().view(id)
    }
    pub fn set_mode(&self, id: &str, mode: Mode) -> Result<LentView> {
        self.change(|c| c.set_mode(id, mode))
    }
    pub fn take_over(&self, id: &str) -> Result<LentView> {
        self.change(|c| c.take_over(id))
    }
    pub fn hand_back(&self, id: &str) -> Result<LentView> {
        self.change(|c| c.hand_back(id))
    }
    pub fn needs_you(&self, id: &str, reason: &str) -> Result<LentView> {
        self.change(|c| c.needs_you(id, reason))
    }
    /// The lease check. `Err(PersonHasControl)` when the person has the app or was asked to step in.
    pub fn bot_begin(&self, id: &str) -> Result<()> {
        self.begin(id).map(|_| ())
    }

    /// Take a new a11y snapshot (kept for `element`). Reading is allowed while the person has the app.
    pub fn snapshot(&self, id: &str) -> Result<Arc<Snap>> {
        let (os, h) = self.core.lock().target(id)?;
        let snap = os.snapshot(h)?; // slow: the lock is not held
        self.core.lock().store_snapshot(id, snap)
    }
    /// The latest snapshot, if one was taken (no new look).
    pub fn snapshot_cached(&self, id: &str) -> Option<Arc<Snap>> {
        self.core.lock().cached(id)
    }
    /// An element of the latest snapshot by ref ("e1"...); its rect is in client coordinates.
    pub fn element(&self, id: &str, r#ref: &str) -> Result<Option<Elem>> {
        self.core.lock().element(id, r#ref)
    }
    /// The mirror's report on the app's picture. Emits only when something changed.
    pub fn picture_report(&self, id: &str, picture: Picture) -> Result<()> {
        self.change_if(|c| c.picture_report(id, picture)).map(|_| ())
    }
    /// JPEG of the app's window, long side at most 1280 px (reading is allowed while the person has the app).
    pub fn capture(&self, id: &str) -> Result<Vec<u8>> {
        let (os, h) = self.core.lock().target(id)?;
        os.capture(h)
    }

    /// Lease check + "bot is using it"; emits only when something visible changed (Free -> Bot, or the window went).
    fn begin(&self, id: &str) -> Result<(Arc<RealOs>, Hwnd)> {
        let (out, changed) = {
            let mut core = self.core.lock();
            let before = core.view(id).ok().map(|v| v.phase);
            let out = core.begin_act(id);
            let after = core.view(id).ok().map(|v| v.phase);
            (out, before != after)
        };
        if changed {
            self.emit();
        }
        out
    }

    fn act<T>(&self, id: &str, f: impl FnOnce(&RealOs, Hwnd) -> Result<T>) -> Result<T> {
        let (os, h) = self.begin(id)?;
        f(&os, h)
    }
    /// The `act_*` helpers check the lease first (`PersonHasControl`), mark the bot as using the app,
    /// then post the input. Coordinates are client pixels.
    pub fn act_click(&self, id: &str, x: i32, y: i32) -> Result<()> {
        self.act(id, |os, h| os.click(h, x, y))
    }
    pub fn act_double_click(&self, id: &str, x: i32, y: i32) -> Result<()> {
        self.act(id, |os, h| os.double_click(h, x, y))
    }
    pub fn act_type(&self, id: &str, text: &str) -> Result<()> {
        self.act(id, |os, h| os.type_text(h, text))
    }
    pub fn act_key(&self, id: &str, vk: u16) -> Result<()> {
        self.act(id, |os, h| os.key(h, vk))
    }
    pub fn act_scroll(&self, id: &str, x: i32, y: i32, delta: i32) -> Result<()> {
        self.act(id, |os, h| os.scroll(h, x, y, delta))
    }

    /// Input that came from the person at the sandbox screen (they took control there): posted to the app's window but NOT
    /// the bot's: no lease check, and the phase does not change.
    pub fn person_input(&self, id: &str, input: &Input) -> Result<()> {
        let (os, h) = self.core.lock().target(id)?;
        input.send(&*os, h)
    }

    /// The same input as the bot's: the lease is checked first (`PersonHasControl`) and the phase becomes Bot.
    pub fn bot_input(&self, id: &str, input: &Input) -> Result<()> {
        self.act(id, |os, h| input.send(os, h))
    }

    pub(super) fn tick(&self) {
        let changed = self.core.lock().tick();
        if changed {
            self.emit();
        }
    }

    /// Put every lent window back (the app is closing).
    pub fn release_all(&self) {
        self.core.lock().release_all();
        #[cfg(not(test))]
        super::mirror::reconcile(&[]);
    }
}

/// Once at startup (`lib.rs`): emits `potato://lend` on every change and starts the 100 ms keeper.
pub fn start(app: Option<tauri::AppHandle>) {
    #[cfg(not(test))]
    if let Some(app) = app {
        use tauri::Emitter;
        hub().set_listener(Some(Arc::new(move || {
            let _ = app.emit(LEND_EVENT, ());
        })));
    }
    #[cfg(test)]
    let _ = app;
    let _ = std::thread::Builder::new().name("potato-lend".into()).spawn(|| loop {
        std::thread::sleep(Duration::from_millis(100));
        hub().tick();
    });
}
