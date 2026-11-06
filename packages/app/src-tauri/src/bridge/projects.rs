//! Project-path resolution shared by the operation, task and shell routes.

use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;

use crate::pty::PaneRegistry;

pub(super) fn resolve_project_root(registry: &Arc<Mutex<PaneRegistry>>) -> Option<PathBuf> {
    crate::event_log::active_project_path().or_else(|| {
        let path = crate::pty::registry::get_project_path(registry);
        let candidate = PathBuf::from(path);
        if crate::project_path::is_valid_project_path(&candidate) {
            Some(candidate)
        } else {
            None
        }
    })
}

