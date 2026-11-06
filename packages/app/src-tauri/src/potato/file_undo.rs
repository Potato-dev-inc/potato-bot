//! Undo for a bot's file changes in a job. Before the bot's `write_file` or `edit_file` changes a file in its own home
//! (`/home/pm/...`) while a job runs, the file's old content is kept here, once per file per job: the first change is the one
//! that matters, because it is what the file looked like before the job touched it. The job lists the files with a Restore
//! button for each; Restore puts the old content back (or removes a file the job made).
//!
//! What it does not cover, and the page says so: anything a terminal command (`run_terminal`) changed, a file outside the
//! bot's home, and a file bigger than `MAX_SNAPSHOT_BYTES` (it is listed as "not saved", with no Restore). A job keeps at most
//! `MAX_FILES` files and `MAX_TOTAL_BYTES` of old content; past that a file is listed as not saved too.
//!
//! Layout: `<app data>/potato/file-undo/<bot>/<job>/index.json` and one `<n>.bin` per saved file. A job's folder goes with
//! the job's record being replaced by a new job (the oldest folders past `KEEP_JOBS` are removed). Nothing here is read by the
//! model; the commands are the app's.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::sandbox::{shell_quote, Sandbox};
use super::store::{self, StoreError, StoreResult};

/// Where the bot's home is inside its computer. Only files under it are kept.
pub const HOME: &str = "/home/pm/";
/// The biggest file whose old content is kept (the same as the biggest file a write may make).
pub const MAX_SNAPSHOT_BYTES: usize = 512 * 1024;
pub const MAX_FILES: usize = 100;
pub const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;
/// Job folders kept per bot; the older ones are removed when a new one starts.
pub const KEEP_JOBS: usize = 5;

const NOT_FOUND: &str = "POTATO_FILE_UNDO_NOT_FOUND";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// The old content is kept: Restore works.
    Saved,
    /// The file did not exist: Restore removes it.
    Created,
    /// The file was bigger than `MAX_SNAPSHOT_BYTES`: not kept.
    TooBig,
    /// The job had already kept `MAX_FILES` files or `MAX_TOTAL_BYTES`: not kept.
    OverLimit,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// `f-` and a number, unique in the job.
    pub id: String,
    /// The path inside the computer.
    pub path: String,
    pub state: State,
    /// Bytes of old content kept (`Saved`), or the size it had (`TooBig`).
    pub bytes: usize,
    /// When the bot first changed it in this job.
    pub at: i64,
    /// `write` or `edit`.
    pub op: String,
    pub restored: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct Index {
    #[serde(default)]
    entries: Vec<Entry>,
    /// More files were changed than the list holds (`MAX_FILES`); the rest were not kept.
    #[serde(default)]
    overflow: bool,
}

/// What the job shows: its id and its files, oldest first.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub job_id: String,
    pub files: Vec<Entry>,
    /// More files were changed than the list holds; the page says so.
    pub overflow: bool,
}

fn dir(root: &Path, bot: &str, job: &str) -> StoreResult<PathBuf> {
    if !crate::companion::model::valid_id(bot) {
        return Err(StoreError::new("POTATO_BOT_NOT_FOUND", "No bot has that name."));
    }
    if !valid_job(job) {
        return Err(StoreError::new(NOT_FOUND, "That job is not known."));
    }
    Ok(root.join("file-undo").join(bot).join(job))
}

/// A job id is `job-` and eight hex digits (`jobs.rs`); it names a folder, so nothing else gets through.
fn valid_job(job: &str) -> bool {
    job.starts_with("job-") && job.len() <= 24 && job[4..].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn index_path(folder: &Path) -> PathBuf {
    folder.join("index.json")
}

fn read_index(folder: &Path) -> StoreResult<Index> {
    Ok(store::read_json(&index_path(folder), "the file undo list")?.unwrap_or_default())
}

fn write_index(folder: &Path, index: &Index) -> StoreResult<()> {
    store::write_json(&index_path(folder), index, "the file undo list")
}

fn blob(folder: &Path, id: &str) -> PathBuf {
    folder.join(format!("{id}.bin"))
}

/// Is this a path whose old content is kept? Inside the bot's home, no `..`.
pub fn covered(path: &str) -> bool {
    path.starts_with(HOME) && path.len() > HOME.len() && !path.split('/').any(|part| part == "..")
}

/// Keeps the old content of `path` before the bot changes it in `job`. Does nothing for a path that is not covered, or one
/// already kept in this job. Never fails the caller: a problem here is logged, and the bot's change goes on.
pub fn snapshot(root: &Path, bot: &str, job: &str, sandbox: &dyn Sandbox, path: &str, op: &str, now: i64) {
    if !covered(path) {
        return;
    }
    if let Err(e) = try_snapshot(root, bot, job, sandbox, path, op, now) {
        tracing::debug!(%e, "could not keep the old content of a file");
    }
}

fn try_snapshot(root: &Path, bot: &str, job: &str, sandbox: &dyn Sandbox, path: &str, op: &str, now: i64) -> StoreResult<()> {
    let _files = store::files_lock();
    let folder = dir(root, bot, job)?;
    let mut index = read_index(&folder)?;
    if index.entries.iter().any(|e| e.path == path) {
        return Ok(());
    }
    // One look at the file. A missing file is "created"; any other trouble (the computer is down) keeps nothing: the
    // change itself would fail too.
    let old = match sandbox.read_file(path, MAX_SNAPSHOT_BYTES + 1) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.code == "SANDBOX_NO_FILE" => None,
        Err(e) => return Err(StoreError::new("POTATO_FILE_UNDO_READ", e.message)),
    };
    if index.entries.len() >= MAX_FILES {
        index.overflow = true;
        return write_index(&folder, &index);
    }
    let kept: usize = index.entries.iter().filter(|e| e.state == State::Saved).map(|e| e.bytes).sum();
    let id = format!("f-{}", index.entries.len() + 1);
    let mut entry = Entry { id: id.clone(), path: path.to_string(), state: State::Created, bytes: 0, at: now, op: op.to_string(), restored: false };
    match old {
        None => {}
        Some(bytes) if bytes.len() > MAX_SNAPSHOT_BYTES => {
            entry.state = State::TooBig;
            entry.bytes = bytes.len();
        }
        Some(bytes) if kept + bytes.len() > MAX_TOTAL_BYTES => {
            entry.state = State::OverLimit;
            entry.bytes = bytes.len();
        }
        Some(bytes) => {
            fs::create_dir_all(&folder).map_err(|e| StoreError::new("POTATO_STORE_INVALID", format!("Could not make the undo folder: {e}.")))?;
            crate::secure_file::write_private_bytes(&blob(&folder, &id), &bytes).map_err(|e| StoreError::new("POTATO_STORE_INVALID", e))?;
            entry.state = State::Saved;
            entry.bytes = bytes.len();
        }
    }
    index.entries.push(entry);
    fs::create_dir_all(&folder).map_err(|e| StoreError::new("POTATO_STORE_INVALID", format!("Could not make the undo folder: {e}.")))?;
    write_index(&folder, &index)?;
    prune(root, bot, job);
    Ok(())
}

/// Removes the job folders of a bot beyond the newest `KEEP_JOBS` (by the folder's change time), never the one in use.
fn prune(root: &Path, bot: &str, keep_job: &str) {
    let parent = root.join("file-undo").join(bot);
    let Ok(read) = fs::read_dir(&parent) else { return };
    let mut jobs: Vec<(std::time::SystemTime, PathBuf)> = read
        .flatten()
        .filter(|e| e.file_name().to_string_lossy() != keep_job)
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    jobs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in jobs.into_iter().skip(KEEP_JOBS - 1) {
        let _ = fs::remove_dir_all(path);
    }
}

/// The files kept for a job, oldest first. A job with none reads as an empty list. (The page reads `view`.)
#[cfg(test)]
pub fn list(root: &Path, bot: &str, job: &str) -> StoreResult<Vec<Entry>> {
    let folder = dir(root, bot, job)?;
    Ok(read_index(&folder)?.entries)
}

/// What the job shows: the files and whether the list overflowed.
pub fn view(root: &Path, bot: &str, job: &str) -> StoreResult<View> {
    let index = read_index(&dir(root, bot, job)?)?;
    Ok(View { job_id: job.to_string(), files: index.entries, overflow: index.overflow })
}

/// Puts one file back as it was before the job changed it: its old content, or gone when the job made it. Marks it restored.
pub fn restore(root: &Path, bot: &str, job: &str, id: &str, sandbox: &dyn Sandbox) -> StoreResult<Entry> {
    let _files = store::files_lock();
    let folder = dir(root, bot, job)?;
    let mut index = read_index(&folder)?;
    let at = index.entries.iter().position(|e| e.id == id).ok_or_else(|| StoreError::new(NOT_FOUND, "That file is not in the list any more."))?;
    let entry = index.entries[at].clone();
    if !covered(&entry.path) {
        return Err(StoreError::new(NOT_FOUND, "That file is not in the list any more."));
    }
    let fail = |e: super::sandbox::SandboxError| StoreError::new("POTATO_FILE_UNDO_FAILED", format!("The file was not restored: {}", e.message));
    match entry.state {
        State::Saved => {
            let bytes = fs::read(blob(&folder, &entry.id)).map_err(|_| StoreError::new(NOT_FOUND, "The saved copy of that file is gone."))?;
            sandbox.write_file(&entry.path, &bytes, false).map_err(fail)?;
        }
        State::Created => {
            let out = sandbox.shell(&format!("rm -f -- {}", shell_quote(&entry.path)), None, std::time::Duration::from_secs(20)).map_err(fail)?;
            if !out.success() {
                return Err(StoreError::new("POTATO_FILE_UNDO_FAILED", "The file was not removed."));
            }
        }
        State::TooBig | State::OverLimit => {
            return Err(StoreError::new("POTATO_FILE_UNDO_NOT_SAVED", "The old content of that file was not kept, so it cannot be restored."));
        }
    }
    index.entries[at].restored = true;
    write_index(&folder, &index)?;
    Ok(index.entries[at].clone())
}
