//! Old copies of a damaged index. A file that cannot be opened is moved aside as `chat-index.bad-<ms>`, together with
//! its `-wal` and `-shm` files (a log left behind would be replayed into the new, empty file). The copies hold old
//! chat text, so "Clear search index" removes them, and the status counts them.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// `path` with `suffix` added to its file name (`chat-index.sqlite` + `-wal`).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The log and shared-memory files SQLite keeps next to `path`.
fn companions(path: &Path) -> [PathBuf; 2] {
    [with_suffix(path, "-wal"), with_suffix(path, "-shm")]
}

/// Moves the index file at `path` aside as `<name>.bad-<stamp>`, with its log and shared-memory files under the same
/// name plus their endings. The main file moves first; if it cannot move, nothing else does.
pub fn move_aside(path: &Path, stamp: i64) -> std::io::Result<PathBuf> {
    let aside = path.with_extension(format!("bad-{stamp}"));
    std::fs::rename(path, &aside)?;
    for (from, to) in companions(path).into_iter().zip(companions(&aside)) {
        if from.exists() && std::fs::rename(&from, &to).is_err() {
            // A log that cannot move must not stay: it would be attached to the new file. Drop it instead.
            let _ = std::fs::remove_file(&from);
        }
    }
    Ok(aside)
}

