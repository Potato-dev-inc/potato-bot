//! Phone-sized panes: a paired phone that types into a pane's tab leases the PTY size, so the agent
//! redraws for the phone's screen instead of the phone shrinking a picture of the PC's grid.
//!
//! Viewing alone never takes the size: only input from the phone claims it (the phone then renews
//! the lease while it keeps typing). The size is sticky: when the phone leaves the tab, hides, or
//! stops renewing (asleep), the pane keeps the phone size. The desktop pane window takes the size
//! back with `reclaim` (a click or focus in its window), which restores the desktop size.
//!
//! Several phones may hold one pane (each with its own `viewer` id): the pane takes the smallest
//! of their sizes (like tmux with several clients), so it fits every one of them.
//! While an entry exists, resizes from the desktop pane window are remembered, not applied.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;

use super::PaneRegistry;

type Size = (u16, u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Viewer {
    size: Size,
    expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PaneLease {
    /// Phones currently holding the size. Empty when the size is sticky (phone gone, size kept).
    viewers: HashMap<String, Viewer>,
    /// The size the desktop wants (pane size before the first lease, or its latest resize since).
    desktop: Size,
}

impl PaneLease {
}

#[derive(Debug, Default)]
pub struct Leases {
    by_pane: HashMap<String, PaneLease>,
}

impl Leases {

    /// The desktop pane window takes its size back: removes the entry and returns the desktop size
    /// to apply. None when no entry existed.
    pub fn reclaim(&mut self, pane_id: &str) -> Option<Size> {
        self.by_pane.remove(pane_id).map(|lease| lease.desktop)
    }

    /// The desktop window resized the pane. True when a phone holds it: the size is remembered
    /// for later and must not be applied now.
    pub fn desktop_resize(&mut self, pane_id: &str, cols: u16, rows: u16) -> bool {
        match self.by_pane.get_mut(pane_id) {
            Some(lease) => {
                lease.desktop = (cols, rows);
                true
            }
            None => false,
        }
    }

    /// A phone holds the pane's size (live or sticky): the desktop must not take it silently.
    pub fn holds(&self, pane_id: &str) -> bool {
        self.by_pane.contains_key(pane_id)
    }

}

static LEASES: Lazy<Mutex<Leases>> = Lazy::new(|| Mutex::new(Leases::default()));

/// Resize the PTY and tell the phone and the desktop window. Errors (pane gone) are ignored.
fn apply(registry: &Mutex<PaneRegistry>, pane_id: &str, (cols, rows): Size) {
    if let Ok(true) = super::registry_resize(registry, pane_id, cols, rows) {
        crate::bridge::notify_pane_resized(pane_id, cols, rows);
        crate::bridge::notify_pty_resized_by_lease(pane_id, cols, rows);
    }
}

/// The desktop pane window takes the pane back from the phone: restores the desktop size.
/// Returns the size applied, or None when no phone held the pane.
pub fn reclaim(registry: &Mutex<PaneRegistry>, pane_id: &str) -> Option<Size> {
    let desktop = LEASES.lock().reclaim(pane_id)?;
    tracing::info!(pane = %pane_id, to = ?desktop, "phone size lease reclaimed by the desktop");
    apply(registry, pane_id, desktop);
    // Also when the size did not change: the desktop windows must drop their "sized for phone" notice.
    crate::bridge::notify_panes_changed();
    Some(desktop)
}

/// A phone holds this pane's size (live or sticky).
pub fn is_held(pane_id: &str) -> bool {
    LEASES.lock().holds(pane_id)
}

/// The desktop pane window asks for a size: true when a phone holds the pane (remembered only).
pub fn desktop_resize(pane_id: &str, cols: u16, rows: u16) -> bool {
    LEASES.lock().desktop_resize(pane_id, cols, rows)
}
