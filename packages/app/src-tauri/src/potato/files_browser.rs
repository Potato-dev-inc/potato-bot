//! The person's file browser for the bot's computer (the Files section of the Computer tab): browse the bot's home folder,
//! look at a file, download a file to Windows, upload a file from Windows. Written for Puppet Master.
//!
//! - Every call is a host call: the person's click comes in through a Tauri command (`commands/potato_files.rs`), and
//!   this file talks to the computer only through `Sandbox::files_run`, the one door that runs the fixed guest script
//!   (`packages/potato-sandbox/guest/files/pm-sbx-files.sh`). Not through `read_file`, `write_file` or `shell`, and the bot
//!   has no tool that reaches any of this.
//! - The paths are checked in that script, in the computer, where symbolic links are real: a path must end up inside the
//!   home folder, and not in a top-level hidden folder (`.pm-sbx` has the screen passwords, `.browser-profiles` has the bot's
//!   logins). `/opt/pm-connectors`, `/run` and the hub logs are outside home, so they are refused by the same rule. This
//!   file adds only plain checks of its own (size, name, empty, control characters) and turns the script's exit codes into
//!   plain sentences.
//! - What the person sees stays on this computer. Nothing here is sent to a model, a thread or a log.
//! - Sizes: a listing shows 2000 entries, a text preview 64 KB, a picture 2 MB, and a download or an upload 10 MB (the
//!   answer says to zip or split bigger files).

use std::path::Path;

use base64::Engine;
use serde::Serialize;

use super::artifacts;
use super::sandbox::Sandbox;
use super::store::{StoreError, StoreResult};

pub const MAX_LIST: usize = 2000;
pub const MAX_TEXT_PREVIEW: usize = 64 * 1024;
pub const MAX_IMAGE_PREVIEW: usize = 2 * 1024 * 1024;
pub const MAX_TRANSFER: usize = 10 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const MAX_NAME_BYTES: usize = 200;
/// The script's own temporary name while an upload is written; never shown.
const UPLOAD_TEMP_PREFIX: &str = ".pm-upload.";

const INVALID: &str = "POTATO_FILES_INVALID";

fn err(code: &'static str, message: impl Into<String>) -> StoreError {
    StoreError::new(code, message)
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Dir,
    File,
    Other,
}

/// One row of a folder.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    /// The entry is a symbolic link. It is opened by the same checks as any path, so a link out of home is refused then.
    pub link: bool,
    pub size: u64,
    pub modified_ms: i64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    /// `~` or `~/docs/reports`: where the list is, from home.
    pub path: String,
    pub entries: Vec<Entry>,
    /// There were more than 2000 entries; the rest are not shown.
    pub truncated: bool,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PreviewKind {
    Text,
    Image,
    Other,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FilePreview {
    pub name: String,
    pub size: u64,
    pub kind: PreviewKind,
    pub text: Option<String>,
    /// The text is only the start of a bigger file.
    pub truncated: bool,
    /// A picture as a `data:` address.
    pub image: Option<String>,
}

// ---- the plain checks and the words --------------------------------------------------------------

/// The path as it goes to the script. Empty means home. Control characters and absurd lengths are refused here; the script
/// does the real check (`..`, outside home, hidden, links).
fn clean_path(raw: &str) -> StoreResult<String> {
    let path = raw.trim();
    if path.is_empty() {
        return Ok("~".to_string());
    }
    if path.len() > MAX_PATH_BYTES || path.chars().any(char::is_control) {
        return Err(err(INVALID, "That is not a usable path."));
    }
    Ok(path.to_string())
}

/// The last part of a path, for a title.
fn last_part(path: &str) -> String {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string()
}

/// A name for a file that is going into the computer: the last part of whatever was given (either kind of slash), no control
/// characters, no dot-only name, at most 200 bytes. The script checks it again.
pub fn clean_upload_name(raw: &str) -> StoreResult<String> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let name = base.trim_end_matches(['.', ' ']);
    if name.is_empty() || name.chars().all(|c| c == '.') || name.chars().any(char::is_control) || name.len() > MAX_NAME_BYTES {
        return Err(err("POTATO_FILES_BAD_NAME", "That file name cannot be used in the computer. Rename the file on this PC and try again."));
    }
    Ok(name.to_string())
}

fn exit_error(code: Option<i32>) -> StoreError {
    match code {
        None => err("POTATO_FILES_TIMEOUT", "The computer took too long to answer. Try again."),
        Some(2) => err("POTATO_FILES_NOT_FOUND", "There is nothing at that path."),
        Some(3) => err("POTATO_FILES_NOT_FOLDER", "That is not a folder."),
        Some(4) => err("POTATO_FILES_TOO_BIG", "That file is bigger than 10 MB. Ask the bot to zip it or split it into parts."),
        Some(5) => err("POTATO_FILES_OUTSIDE", "That is outside the bot's home folder (or a hidden folder of it), so it cannot be opened here."),
        Some(6) => err("POTATO_FILES_EXISTS", "A file with that name is already in that folder. Rename the file on this PC and try again."),
        Some(7) => err("POTATO_FILES_NOT_FILE", "That is not a regular file."),
        Some(8) => err("POTATO_FILES_WRITE_FAILED", "The file could not be written in the computer."),
        Some(10) => err(INVALID, "Give a path."),
        Some(11) => err(INVALID, "A path with .. in it cannot be used. Open the folders one by one."),
        Some(12) => err("POTATO_FILES_BAD_NAME", "That file name cannot be used in the computer."),
        Some(20) => err("POTATO_FILES_NO_HOME", "The bot's home folder is missing."),
        Some(_) => err(INVALID, "The computer could not do that."),
    }
}

/// One run of the guest script. A non-zero exit becomes a plain sentence.
fn run(sandbox: &dyn Sandbox, args: &[&str], stdin: Option<Vec<u8>>) -> StoreResult<Vec<u8>> {
    let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
    let out = sandbox.files_run(&args, stdin).map_err(|e| err(e.code, e.message))?;
    match out.code {
        Some(0) => Ok(out.stdout),
        code => Err(exit_error(code)),
    }
}

// ---- list ----------------------------------------------------------------------------------------

fn show_path(rel: &str) -> String {
    if rel.is_empty() {
        "~".to_string()
    } else {
        format!("~/{rel}")
    }
}

fn kind_of_type(letter: &str) -> EntryKind {
    match letter {
        "d" => EntryKind::Dir,
        "f" => EntryKind::File,
        _ => EntryKind::Other,
    }
}

/// The records the script printed for `list`: the first is `P<TAB>path`, then one per entry
/// (`type<TAB>target type<TAB>size<TAB>mtime<TAB>name`), each ended by a NUL.
fn parse_listing(bytes: &[u8]) -> StoreResult<Listing> {
    let mut records = bytes.split(|b| *b == 0).filter(|r| !r.is_empty());
    let first = records.next().map(|r| String::from_utf8_lossy(r).into_owned()).unwrap_or_default();
    let Some(rel) = first.strip_prefix("P\t") else {
        return Err(err(INVALID, "The computer's answer could not be read."));
    };
    let mut entries = Vec::new();
    let mut truncated = false;
    for record in records {
        if entries.len() >= MAX_LIST {
            truncated = true;
            break;
        }
        let text = String::from_utf8_lossy(record);
        let mut parts = text.splitn(5, '\t');
        let (Some(kind), Some(target), Some(size), Some(mtime), Some(name)) = (parts.next(), parts.next(), parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if name.is_empty() || name.contains('/') || name.starts_with(UPLOAD_TEMP_PREFIX) {
            continue;
        }
        let link = kind == "l";
        let entry_kind = kind_of_type(if link { target } else { kind });
        let seconds: f64 = mtime.parse().unwrap_or(0.0);
        entries.push(Entry { name: name.to_string(), kind: entry_kind, link, size: size.parse().unwrap_or(0), modified_ms: (seconds * 1000.0) as i64 });
    }
    entries.sort_by(|a, b| (a.kind != EntryKind::Dir, a.name.to_lowercase()).cmp(&(b.kind != EntryKind::Dir, b.name.to_lowercase())));
    Ok(Listing { path: show_path(rel), entries, truncated })
}

/// The folder at `path` (empty: home).
pub fn list(sandbox: &dyn Sandbox, path: &str) -> StoreResult<Listing> {
    let path = clean_path(path)?;
    parse_listing(&run(sandbox, &["list", &path], None)?)
}

// ---- preview -------------------------------------------------------------------------------------

fn image_mime(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => return None,
    })
}

fn stat(sandbox: &dyn Sandbox, path: &str) -> StoreResult<(char, u64)> {
    let out = run(sandbox, &["stat", path], None)?;
    let text = String::from_utf8_lossy(&out).trim().to_string();
    let (kind, size) = text.split_once('\t').ok_or_else(|| err(INVALID, "The computer's answer could not be read."))?;
    Ok((kind.chars().next().unwrap_or('o'), size.trim().parse().unwrap_or(0)))
}

/// A look at one file: the start of a text file, a small picture, or just its name and size. Nothing is run or drawn as
/// a page: HTML and SVG are shown as text or not at all.
pub fn preview(sandbox: &dyn Sandbox, path: &str) -> StoreResult<FilePreview> {
    let path = clean_path(path)?;
    let name = last_part(&path);
    let (kind, size) = stat(sandbox, &path)?;
    match kind {
        'f' => {}
        'd' => return Err(err("POTATO_FILES_NOT_FILE", "That is a folder. Open it instead.")),
        _ => return Err(err("POTATO_FILES_NOT_FILE", "That is not a regular file.")),
    }
    let shown_name = name.clone();
    let other = |size: u64| FilePreview { name: shown_name.clone(), size, kind: PreviewKind::Other, text: None, truncated: false, image: None };
    let ext = artifacts::extension(&name);
    if let Some(mime) = image_mime(&ext) {
        if size as usize > MAX_IMAGE_PREVIEW {
            return Ok(other(size));
        }
        let bytes = run(sandbox, &["get", &path, &MAX_IMAGE_PREVIEW.to_string()], None)?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        return Ok(FilePreview { name, size, kind: PreviewKind::Image, text: None, truncated: false, image: Some(format!("data:{mime};base64,{encoded}")) });
    }
    if matches!(artifacts::kind_of(&ext), artifacts::Kind::Pdf) {
        return Ok(other(size));
    }
    let bytes = run(sandbox, &["head", &path, &MAX_TEXT_PREVIEW.to_string()], None)?;
    // A NUL in the first bytes means it is not text (a program, an archive, a database): no preview.
    if bytes.contains(&0) {
        return Ok(other(size));
    }
    let truncated = size as usize > MAX_TEXT_PREVIEW;
    Ok(FilePreview { name, size, kind: PreviewKind::Text, text: Some(String::from_utf8_lossy(&bytes).into_owned()), truncated, image: None })
}

// ---- download and upload -------------------------------------------------------------------------

/// The bytes of a file of the computer and its name, for the save dialog's path (`save_download`).
pub fn download(sandbox: &dyn Sandbox, path: &str) -> StoreResult<(String, Vec<u8>)> {
    let path = clean_path(path)?;
    let bytes = run(sandbox, &["get", &path, &MAX_TRANSFER.to_string()], None)?;
    Ok((last_part(&path), bytes))
}

/// Writes a downloaded file where the person picked in the save dialog (an absolute path to a new or chosen file).
pub fn save_download(dest: &Path, bytes: &[u8]) -> StoreResult<()> {
    if !dest.is_absolute() || dest.is_dir() {
        return Err(err("POTATO_FILES_BAD_DEST", "Pick a file name to save to."));
    }
    if !dest.parent().is_some_and(Path::is_dir) {
        return Err(err("POTATO_FILES_BAD_DEST", "That folder is not there."));
    }
    std::fs::write(dest, bytes).map_err(|_| err("POTATO_FILES_BAD_DEST", "The file could not be saved there."))
}

/// A file from Windows that the person picked, ready to go into the computer: its cleaned name and bytes. Folders and
/// files over 10 MB are refused here, before anything is read.
pub fn read_upload(src: &Path) -> StoreResult<(String, Vec<u8>)> {
    if !src.is_absolute() {
        return Err(err("POTATO_FILES_BAD_SOURCE", "Pick a file on this PC."));
    }
    let meta = std::fs::metadata(src).map_err(|_| err("POTATO_FILES_BAD_SOURCE", "That file could not be read."))?;
    if !meta.is_file() {
        return Err(err("POTATO_FILES_BAD_SOURCE", "Pick a single file. To send a folder, zip it first."));
    }
    if meta.len() as usize > MAX_TRANSFER {
        return Err(exit_error(Some(4)));
    }
    let name = clean_upload_name(&src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())?;
    let bytes = std::fs::read(src).map_err(|_| err("POTATO_FILES_BAD_SOURCE", "That file could not be read."))?;
    Ok((name, bytes))
}

/// Puts `bytes` in the folder `dir` of the computer under `name`. A name that is already there is never replaced. Returns
/// where the file is, from home (`~/docs/plan.md`).
pub fn upload(sandbox: &dyn Sandbox, dir: &str, name: &str, bytes: Vec<u8>) -> StoreResult<String> {
    let dir = clean_path(dir)?;
    let name = clean_upload_name(name)?;
    if bytes.len() > MAX_TRANSFER {
        return Err(exit_error(Some(4)));
    }
    let out = run(sandbox, &["put", &dir, &name, &MAX_TRANSFER.to_string()], Some(bytes))?;
    Ok(show_path(String::from_utf8_lossy(&out).trim()))
}
