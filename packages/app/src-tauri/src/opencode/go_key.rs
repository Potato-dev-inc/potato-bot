//! The OpenCode Go key, read from the place OpenCode 2.x actually keeps it.
//!
//! OpenCode 1.x wrote `auth.json`; 2.x keeps credentials in `opencode.db`, table `credential`, one
//! active row per integration (`opencode-go`, `opencode`). The brain can read `auth.json` itself,
//! but it cannot open the database on every Node version the app supports, so the host reads the
//! active key here and hands it to the brain as `OPENCODE_API_KEY` (only for the Go brain).
//!
//! Never logs or returns anything but the secret itself. Missing, locked or malformed files are
//! just "no key": the brain then falls back to `auth.json` or reports the same provider error.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

/// The integrations that can hold the Go key, most specific first.
const INTEGRATIONS: [&str; 2] = ["opencode-go", "opencode"];

/// The active Go key, if OpenCode 2.x stores one.
pub fn read_active() -> Option<String> {
    candidates().into_iter().find_map(|path| read_at(&path))
}

/// The places OpenCode 2.x may keep its database on this host.
fn candidates() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    let mut list: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        list.push(PathBuf::from(dir).join("opencode").join("opencode.db"));
    }
    if let Some(home) = &home {
        list.push(home.join(".local").join("share").join("opencode").join("opencode.db"));
        list.push(home.join(".config").join("opencode").join("opencode.db"));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        list.push(PathBuf::from(appdata).join("opencode").join("opencode.db"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        list.push(PathBuf::from(local).join("opencode").join("opencode.db"));
    }
    list
}

/// Read the active credential from one database file. Never throws.
pub(crate) fn read_at(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    // Read-only first. A WAL-mode database needs its `-shm` sidecar, which a read-only
    // connection cannot create when OpenCode is not running; then a plain connection is used
    // (we only ever SELECT, and SQLite may checkpoint the WAL on close).
    read_conn(path, true).or_else(|| read_conn(path, false))
}

fn read_conn(path: &Path, read_only: bool) -> Option<String> {
    let conn = if read_only {
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?
    } else {
        Connection::open(path).ok()?
    };
    let _ = conn.busy_timeout(Duration::from_millis(500));
    for integration in INTEGRATIONS {
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM credential WHERE integration_id = ?1 AND active = 1 ORDER BY rowid DESC LIMIT 1",
                [integration],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten();
        if let Some(key) = value.as_deref().and_then(key_from_value) {
            return Some(key);
        }
    }
    None
}

/// The secret inside a credential row's JSON value: an API key, or an OAuth access token.
pub(crate) fn key_from_value(value: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(value).ok()?;
    json.get("key")
        .and_then(serde_json::Value::as_str)
        .filter(|key| !key.is_empty())
        .or_else(|| json.get("access").and_then(serde_json::Value::as_str).filter(|access| !access.is_empty()))
        .map(str::to_string)
}
