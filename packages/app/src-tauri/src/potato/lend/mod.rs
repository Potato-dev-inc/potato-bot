//! Lend an App: the person lends an open Windows app to their bot.

pub type Hwnd = isize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct WinInfo {
    pub hwnd: Hwnd,
    pub pid: u32,
    pub title: String,
    /// File name, e.g. `LINE.exe`.
    pub exe: String,
    /// Name to show people, e.g. `LINE`.
    pub app_name: String,
    pub rect: Rect,
    pub elevated: bool,
    pub minimized: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum LendError {
    #[error("window is gone")]
    Gone,
    #[error("not supported on this OS")]
    #[cfg_attr(windows, allow(dead_code))] // only non-Windows builds answer this
    Unsupported,
    #[error("{0}")]
    Os(String),
    #[error("person has control")]
    PersonHasControl,
    #[error("no such lent app")]
    NoSuchApp,
}

/// Chromium and Electron windows (browsers and Electron apps): their top window class starts with this.
/// They often expose almost no accessibility elements until something asks (see `a11y::nudge_chromium`).
pub fn is_chromium_class(class: &str) -> bool {
    const PREFIX: &str = "Chrome_WidgetWin_";
    class.len() > PREFIX.len() && class.is_char_boundary(PREFIX.len()) && class[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
}

/// Fewer elements than this in a snapshot counts as "no labels" (see `a11y::snapshot_nudged`).
pub const FEW_ELEMENTS: usize = 3;

pub type Result<T> = std::result::Result<T, LendError>;

// S1
#[cfg(windows)]
pub mod capture;
#[cfg(windows)]
pub mod hide;
#[cfg(windows)]
pub mod input;
#[cfg(windows)]
pub mod win;
// S2
#[cfg(windows)]
pub mod a11y;
// S3
#[cfg(windows)]
pub mod border;
// Lend an App v2: mirror into the sandbox screen.
#[cfg(windows)]
pub mod mirror;
#[cfg(windows)]
#[cfg(test)]
mod mirror_tests;
/// Non-Windows builds lend nothing, so there is nothing to mirror.
#[cfg(not(windows))]
pub mod mirror {
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Lent {
        pub id: String,
        pub title: String,
        pub hwnd: super::Hwnd,
    }
    pub fn reconcile(_: &[Lent]) {}
}
// S4
pub mod session;
#[cfg(test)]
mod session_tests;
// Manual live check against a real Notepad window (`#[ignore]`d).
#[cfg(all(test, windows))]
mod live_tests;
