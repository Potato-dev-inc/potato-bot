//! Files a person attaches to a message in a bot's thread (pictures and text files).
//!
//! The file is saved in the bot's own folder (`<app data>/potato/brain/<bot>/attachments/`) and the message says where
//! it is: on the bot's computer when that is running (the file is copied to `/home/pm/attachments/` there), else on the
//! PC only. A text file that fits is also quoted in the message, so the bot can read it at once. A picture also goes
//! to the brain as a picture (`attachment_images`), for a model that can look at it. File contents go nowhere but that
//! folder, that computer and that message (which only the bot's own brain receives). Nothing here opens a network
//! connection or a path the caller names: the folder is fixed, the file name is cleaned, and what kind of file it is
//! comes from its first bytes, never from the name the page claims. Files older than 7 days are swept away.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use base64::Engine as _;
use serde::Deserialize;

pub use super::attachment_images::ImagePart;
use super::sandbox::{Sandbox, State};
use super::store::{self, StoreError, MAX_MESSAGE_CHARS};

/// Files in one message.
pub const MAX_FILES: usize = 4;
/// One file, in bytes (the same 2 MiB the store uses for its own files).
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
/// All files of one message together.
pub const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;
/// The folder inside the bot's brain folder.
pub const FOLDER: &str = "attachments";
/// Where the files are copied to inside the bot's computer (its home is `/home/pm`).
pub const GUEST_DIR: &str = "/home/pm/attachments";
/// How long a saved file is kept on the PC.
pub const KEEP_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
static LAST_SWEEP_MS: AtomicI64 = AtomicI64::new(i64::MIN);
/// Below this many characters of room left, a text file is not quoted (the saved file is still named).
const MIN_QUOTE: usize = 60;

/// One file as the page sends it. `data` is base64. `kind` is only a hint; the bytes decide.
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Incoming {
    pub name: String,
    #[serde(default)]
    pub kind: String,
    pub data: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Text,
}

/// Where a saved file is, as far as the bot's computer goes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Guest {
    /// The computer is off (or the bot has none): the file is on the PC only.
    #[default]
    Off,
    /// Copied to this absolute path inside the computer.
    At(String),
    /// The computer is on but the copy failed.
    Failed,
}

/// A file after it is checked and saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    /// The cleaned name the person sees.
    pub name: String,
    /// Where it is, relative to the bot's folder: `attachments/<stamp>-<n>-<name>`.
    pub rel: String,
    pub kind: Kind,
    pub bytes: usize,
    /// The text of a text file.
    pub text: Option<String>,
    pub guest: Guest,
}

fn bad(message: impl Into<String>) -> StoreError {
    StoreError::new("POTATO_ATTACHMENT", message)
}

/// The last part of a path, with anything but letters, digits, `.`, `-`, `_` and spaces turned into `_`; never
/// empty, never starting with a dot, at most 60 characters (the end is kept, so the extension stays).
pub fn safe_name(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').to_string();
    if trimmed.is_empty() {
        return "file".to_string();
    }
    let count = trimmed.chars().count();
    if count <= 60 {
        trimmed
    } else {
        trimmed.chars().skip(count - 60).collect()
    }
}

/// `png`, `jpg`, `gif` or `webp` from the first bytes of a file, or `None`.
pub fn image_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// `1.5 MB`, `12 KB`, `300 B`.
pub fn size_word(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} KB", (bytes + 512) / 1024)
    } else {
        format!("{bytes} B")
    }
}

/// One file's kind and cleaned name from its bytes. A picture must start like a picture; anything else must be
/// plain text (valid UTF-8, no zero bytes). The saved name keeps the person's extension for text and takes the real
/// one for a picture.
fn inspect(file: &Incoming, bytes: &[u8]) -> Result<(Kind, String, Option<String>), StoreError> {
    let name = safe_name(&file.name);
    if bytes.is_empty() {
        return Err(bad(format!("{name} is empty.")));
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err(bad(format!("{name} is bigger than {}.", size_word(MAX_FILE_BYTES))));
    }
    if let Some(ext) = image_ext(bytes) {
        let stem = name.rsplit_once('.').map(|(s, _)| s).filter(|s| !s.is_empty()).unwrap_or(&name);
        return Ok((Kind::Image, format!("{stem}.{ext}"), None));
    }
    if file.kind == "image" {
        return Err(bad(format!("{name} is not a picture Potato Bot can keep (png, jpg, gif or webp).")));
    }
    if bytes.contains(&0) {
        return Err(bad(format!("{name} is not a text file.")));
    }
    match String::from_utf8(bytes.to_vec()) {
        Ok(text) => Ok((Kind::Text, name, Some(text))),
        Err(_) => Err(bad(format!("{name} is not a text file."))),
    }
}

fn dir_of(root: &Path, bot: &str) -> Result<PathBuf, StoreError> {
    Ok(store::brain_dir(root, bot)?.join(FOLDER))
}

/// Checks every file, then saves them all (none is saved if one is refused). `stamp` makes the file names unique.
pub fn save_all(root: &Path, bot: &str, files: &[Incoming], stamp: i64) -> Result<Vec<Saved>, StoreError> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    if files.len() > MAX_FILES {
        return Err(bad(format!("Attach at most {MAX_FILES} files to one message.")));
    }
    let dir = dir_of(root, bot)?;
    let mut checked: Vec<(Vec<u8>, Kind, String, Option<String>)> = Vec::new();
    let mut total = 0usize;
    for file in files {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(file.data.trim())
            .map_err(|_| bad(format!("{} could not be read.", safe_name(&file.name))))?;
        total += bytes.len();
        if total > MAX_TOTAL_BYTES {
            return Err(bad(format!("The files together are bigger than {}.", size_word(MAX_TOTAL_BYTES))));
        }
        let (kind, name, text) = inspect(file, &bytes)?;
        checked.push((bytes, kind, name, text));
    }
    crate::secure_file::create_private_dir(&dir).map_err(|e| bad(format!("Could not make the attachments folder: {e}.")))?;
    let mut saved: Vec<Saved> = Vec::new();
    for (n, (bytes, kind, name, text)) in checked.into_iter().enumerate() {
        let file_name = format!("{stamp}-{}-{name}", n + 1);
        if let Err(e) = std::fs::write(dir.join(&file_name), &bytes) {
            forget(root, bot, &saved);
            return Err(bad(format!("Could not save {name}: {e}.")));
        }
        saved.push(Saved { name, rel: format!("{FOLDER}/{file_name}"), kind, bytes: bytes.len(), text, guest: Guest::Off });
    }
    Ok(saved)
}

/// Removes files this module saved (a send that failed). Only names under the bot's attachments folder.
pub fn forget(root: &Path, bot: &str, saved: &[Saved]) {
    let Ok(base) = store::brain_dir(root, bot) else { return };
    for file in saved {
        if file.rel.starts_with(&format!("{FOLDER}/")) && !file.rel.contains("..") {
            let _ = std::fs::remove_file(base.join(&file.rel));
        }
    }
}

/// The longest run of backticks in `text`, so a quote can use a longer fence.
fn longest_ticks(text: &str) -> usize {
    let (mut best, mut run) = (0, 0);
    for c in text.chars() {
        if c == '`' {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

/// Where the bot can find a file, in the words of the message line.
pub fn where_words(file: &Saved) -> String {
    match &file.guest {
        Guest::At(path) => format!("saved on your computer at {path}"),
        Guest::Failed => format!("saved at {} on the PC (it could not be copied to your computer)", file.rel),
        Guest::Off => format!("saved at {} on the PC (your computer is off)", file.rel),
    }
}

/// The message the brain gets: the person's words, one line per file, then the text of each text file that still
/// fits (cut with a note when it does not). Never longer than the 4000 characters a message may have.
pub fn compose(text: &str, saved: &[Saved]) -> Result<String, StoreError> {
    let head = text.trim();
    let lines: Vec<String> = saved
        .iter()
        .map(|f| {
            let what = if f.kind == Kind::Image { "image" } else { "file" };
            format!("[Attached {what}: {} ({}), {}]", f.name, size_word(f.bytes), where_words(f))
        })
        .collect();
    let mut out = String::new();
    if !head.is_empty() {
        out.push_str(head);
        out.push_str("\n\n");
    }
    out.push_str(&lines.join("\n"));
    let used = out.chars().count();
    if used > MAX_MESSAGE_CHARS {
        return Err(bad("That message is too long with the attachments. Shorten it a little."));
    }
    let mut room = MAX_MESSAGE_CHARS - used;
    for file in saved {
        let Some(body) = file.text.as_deref() else { continue };
        let body = body.trim_end();
        if body.is_empty() {
            continue;
        }
        let fence = "`".repeat(longest_ticks(body).max(2) + 1);
        // "\n\n" + fence + "\n" + body + "\n" + fence, and a marker when it is cut.
        let label = format!("{}:", file.name);
        let frame = 2 + label.chars().count() + 1 + fence.len() + 1 + 1 + fence.len();
        let cut_note = "\n(Cut here. The whole file is saved.)";
        if room < frame + MIN_QUOTE {
            continue;
        }
        let full = body.chars().count();
        let (shown, note) = if frame + full <= room {
            (body.to_string(), "")
        } else {
            let keep = room.saturating_sub(frame + cut_note.chars().count());
            (body.chars().take(keep).collect::<String>(), cut_note)
        };
        let block = format!("\n\n{label}\n{fence}\n{shown}\n{fence}{note}");
        room = room.saturating_sub(block.chars().count());
        out.push_str(&block);
    }
    Ok(out)
}

// ---- the bot's computer ---------------------------------------------------------------------------------------------

fn file_name_of(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// Whether the computer is running right now. Asking never starts it.
pub fn is_running(computer: &dyn Sandbox) -> bool {
    matches!(computer.status(), Ok(status) if status.state == State::Running)
}

/// The bot's computer, only when it can be used now: the sandbox is on for Potato Bot, this bot has the computer tool,
/// and the computer is running. A stopped computer is left stopped.
pub fn running_computer(service: &super::service::Service, bot: &str) -> Option<Arc<dyn Sandbox>> {
    if !service.sandbox_offered() {
        return None;
    }
    if !super::bot_settings::load(service.root(), bot).map(|b| b.tools.computer).unwrap_or(false) {
        return None;
    }
    let sandbox = super::sandbox_host::manager().ok()?.sandbox().clone();
    is_running(sandbox.as_ref()).then_some(sandbox)
}

/// Copies every saved file into the computer at `/home/pm/attachments/<same file name>`, as the sandbox user. A copy that
/// fails marks that file (and the ones after it, which would fail the same way) and never stops the message.
pub fn place_on_computer(computer: &dyn Sandbox, root: &Path, bot: &str, saved: &mut [Saved]) {
    let Ok(base) = store::brain_dir(root, bot) else { return };
    let mut failed = false;
    for file in saved.iter_mut() {
        let guest = format!("{GUEST_DIR}/{}", file_name_of(&file.rel));
        let copied = !failed && std::fs::read(base.join(&file.rel)).is_ok_and(|bytes| computer.write_file(&guest, &bytes, false).is_ok());
        failed = !copied;
        file.guest = if copied { Guest::At(guest) } else { Guest::Failed };
    }
}

// ---- one send --------------------------------------------------------------------------------------------------------

/// What a send with files is made of.
#[derive(Debug)]
pub struct Prepared {
    pub saved: Vec<Saved>,
    pub message: String,
    pub images: Vec<ImagePart>,
}

/// Up to 4 of the saved pictures, ready for the brain.
pub fn images_of(root: &Path, bot: &str, saved: &[Saved]) -> Vec<ImagePart> {
    let Ok(base) = store::brain_dir(root, bot) else { return Vec::new() };
    saved
        .iter()
        .filter(|f| f.kind == Kind::Image)
        .filter_map(|f| std::fs::read(base.join(&f.rel)).ok())
        .filter_map(|bytes| super::attachment_images::prepare(&bytes))
        .take(super::attachment_images::MAX_IMAGES)
        .collect()
}

/// Saves the files, copies them into `computer` when there is one running, and makes the message and the pictures. When
/// the message cannot be made (too long) nothing stays saved.
pub fn prepare(root: &Path, bot: &str, text: &str, files: &[Incoming], stamp: i64, computer: Option<&dyn Sandbox>) -> Result<Prepared, StoreError> {
    let mut saved = save_all(root, bot, files, stamp)?;
    if let Some(computer) = computer {
        place_on_computer(computer, root, bot, &mut saved);
    }
    match compose(text, &saved) {
        Ok(message) => {
            let images = images_of(root, bot, &saved);
            Ok(Prepared { saved, message, images })
        }
        Err(e) => {
            forget(root, bot, &saved);
            Err(e)
        }
    }
}

// ---- cleanup -----------------------------------------------------------------------------------------------------------

/// Pure: a file last changed at `modified_ms` is expired when it is `keep_ms` or more old. A time in the future is not.
pub fn is_expired(modified_ms: i64, now_ms: i64, keep_ms: i64) -> bool {
    now_ms.saturating_sub(modified_ms) >= keep_ms
}

/// Deletes the plain files directly inside `dir` that are expired. Folders and links are left alone, nothing deeper is
/// looked at. Returns how many files went.
pub fn sweep_dir(dir: &Path, now_ms: i64, keep_ms: i64) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    for entry in entries.flatten() {
        // `DirEntry::file_type` does not follow a link, so a link is not a file here.
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64);
        if modified.is_some_and(|m| is_expired(m, now_ms, keep_ms)) && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Sweeps the attachments folder of every bot (`<root>/brain/<bot>/attachments`).
pub fn sweep_all(root: &Path, now_ms: i64) -> usize {
    let Ok(bots) = std::fs::read_dir(root.join("brain")) else { return 0 };
    bots.flatten()
        .filter(|b| b.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|b| b.file_name().to_str().and_then(|name| dir_of(root, name).ok()))
        .map(|dir| sweep_dir(&dir, now_ms, KEEP_MS))
        .sum()
}

/// `sweep_all` at most once a day (the routines tick calls it every 30 s, so it also runs at app start).
pub fn maybe_sweep(root: &Path, now_ms: i64) {
    let last = LAST_SWEEP_MS.load(Ordering::SeqCst);
    if last != i64::MIN && now_ms - last < DAY_MS {
        return;
    }
    LAST_SWEEP_MS.store(now_ms, Ordering::SeqCst);
    sweep_all(root, now_ms);
}
