//! The files of a group of bots:
//! - `rooms/rooms.json`: `{ v: 1, rooms: [{ id, name, botIds, createdMs, lastMs, unread }] }`.
//! - `rooms/<room id>.jsonl`: one line per message, oldest first, at most 5000 lines kept.
//!
//! Ids are `r` or `l` plus 12 lowercase hex digits, made here. Only such an id ever becomes a file name, so a
//! path cannot leave the folder. A damaged index is an error and is never overwritten.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::potato::store::{self, STORE_VERSION};
use crate::secure_file;

pub const MIN_BOTS: usize = 2;
pub const MAX_BOTS: usize = 5;
pub const MAX_ROOMS: usize = 50;
pub const MAX_NAME: usize = 60;
pub const MAX_LINES_KEPT: usize = 5_000;
pub const MAX_TEXT: usize = 8_000;
/// The most lines one read returns.
pub const MAX_READ: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Room {
    pub id: String,
    pub name: String,
    pub bot_ids: Vec<String>,
    pub created_ms: i64,
    pub last_ms: i64,
    pub unread: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Speaker {
    Person,
    Bot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomFrom {
    pub kind: Speaker,
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomLine {
    pub id: String,
    pub from: RoomFrom,
    pub text: String,
    pub at: i64,
    /// 0 for a person's line; a bot's reply has the hop of the line it answers plus one.
    pub hop: u8,
    pub in_reply_to: Option<String>,
    /// A file the bot sent into the group (`send_file`). The card is opened with the sending bot's id (`from.id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<store::FileCard>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomError {
    NameEmpty,
    NameTooLong,
    TooFewBots,
    TooManyBots,
    DuplicateBot,
    UnknownBot(String),
    TooManyRooms,
    NotFound,
    BadId,
    Io(String),
}

impl RoomError {
    /// A short wire word, a colon, then plain English for the screen.
    pub fn to_wire(&self) -> String {
        match self {
            RoomError::NameEmpty => "POTATO_ROOM_NAME: Give the group a name.".into(),
            RoomError::NameTooLong => "POTATO_ROOM_NAME: Use 60 characters or fewer.".into(),
            RoomError::TooFewBots => "POTATO_ROOM_BOTS: Pick at least 2 bots.".into(),
            RoomError::TooManyBots => "POTATO_ROOM_BOTS: A group can have up to 5 bots.".into(),
            RoomError::DuplicateBot => "POTATO_ROOM_BOTS: Each bot can be in a group once.".into(),
            RoomError::UnknownBot(_) => "POTATO_ROOM_BOTS: One of these bots no longer exists.".into(),
            RoomError::TooManyRooms => "POTATO_ROOM_LIMIT: You can have up to 50 groups.".into(),
            RoomError::NotFound => "POTATO_ROOM_GONE: This group no longer exists.".into(),
            RoomError::BadId => "POTATO_ROOM_ID: That group id is not valid.".into(),
            RoomError::Io(_) => "POTATO_ROOM_IO: Could not save the group. Try again.".into(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct RoomsFile {
    v: u32,
    rooms: Vec<Room>,
}

/// A new id: `prefix` and twelve lowercase hex digits.
pub fn new_id(prefix: char) -> String {
    format!("{prefix}{}", &uuid::Uuid::new_v4().simple().to_string()[..12])
}

/// `prefix` and exactly twelve lowercase hex digits.
pub fn valid_id(prefix: char, id: &str) -> bool {
    id.len() == 13 && id.starts_with(prefix) && id[1..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn dir(root: &Path) -> PathBuf {
    root.join("rooms")
}

pub fn index_path(root: &Path) -> PathBuf {
    dir(root).join("rooms.json")
}

/// `<root>/rooms/<room id>.jsonl`; `None` for an id that is not a room id.
pub fn lines_path(root: &Path, room_id: &str) -> Option<PathBuf> {
    valid_id('r', room_id).then(|| dir(root).join(format!("{room_id}.jsonl")))
}

/// The name and the bots of a new group, checked: the name trimmed and cleaned, 1 to 60 characters; 2 to 5
/// bots, each once, each in `known_bots`, in the order given.
pub fn validate(name: &str, bot_ids: &[String], known_bots: &[String]) -> Result<(String, Vec<String>), RoomError> {
    let name = store::clean_text(name.trim(), None).trim().to_string();
    if name.is_empty() {
        return Err(RoomError::NameEmpty);
    }
    if name.chars().count() > MAX_NAME {
        return Err(RoomError::NameTooLong);
    }
    if bot_ids.len() < MIN_BOTS {
        return Err(RoomError::TooFewBots);
    }
    if bot_ids.len() > MAX_BOTS {
        return Err(RoomError::TooManyBots);
    }
    for (i, bot) in bot_ids.iter().enumerate() {
        if !known_bots.contains(bot) {
            return Err(RoomError::UnknownBot(bot.clone()));
        }
        if bot_ids[..i].contains(bot) {
            return Err(RoomError::DuplicateBot);
        }
    }
    Ok((name, bot_ids.to_vec()))
}

/// Every room in the index. A missing index is empty; a damaged one is an error and is never overwritten.
pub fn load_rooms(root: &Path) -> Result<Vec<Room>, RoomError> {
    let _files = store::files_lock();
    let text = match fs::read_to_string(index_path(root)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(RoomError::Io(e.to_string())),
    };
    let file: RoomsFile = serde_json::from_str(&text).map_err(|e| RoomError::Io(format!("damaged index: {e}")))?;
    if file.v != STORE_VERSION {
        return Err(RoomError::Io(format!("index version {}", file.v)));
    }
    Ok(file.rooms)
}

pub fn save_rooms(root: &Path, rooms: &[Room]) -> Result<(), RoomError> {
    let _files = store::files_lock();
    if rooms.len() > MAX_ROOMS {
        return Err(RoomError::TooManyRooms);
    }
    let file = RoomsFile { v: STORE_VERSION, rooms: rooms.to_vec() };
    let text = serde_json::to_string_pretty(&file).map_err(|e| RoomError::Io(e.to_string()))?;
    secure_file::create_private_dir(&dir(root)).map_err(|e| RoomError::Io(e.to_string()))?;
    secure_file::write_private(&index_path(root), &text).map_err(RoomError::Io)
}

/// The text kept for a line: secrets and the home folder taken out, cut to 8000 characters.
pub fn clean_line(text: &str, home: Option<&str>) -> String {
    crate::companion::view::scrub(text, home).chars().take(MAX_TEXT).collect()
}

/// Appends one line (its text is cleaned here), then keeps only the newest 5000 lines.
pub fn append_line(root: &Path, room_id: &str, mut line: RoomLine, home: Option<&str>) -> Result<(), RoomError> {
    let path = lines_path(root, room_id).ok_or(RoomError::BadId)?;
    line.text = clean_line(&line.text, home);
    let json = serde_json::to_string(&line).map_err(|e| RoomError::Io(e.to_string()))?;
    {
        let _files = store::files_lock();
        secure_file::create_private_dir(&dir(root)).map_err(|e| RoomError::Io(e.to_string()))?;
        let mut file = OpenOptions::new().create(true).append(true).open(&path).map_err(|e| RoomError::Io(e.to_string()))?;
        writeln!(file, "{json}").map_err(|e| RoomError::Io(e.to_string()))?;
    }
    trim_lines(root, room_id)
}

/// The newest `limit` lines (at most 500), oldest first. A missing file is empty; a broken line is skipped.
pub fn read_lines(root: &Path, room_id: &str, limit: usize) -> Vec<RoomLine> {
    let Some(path) = lines_path(root, room_id) else { return Vec::new() };
    let _files = store::files_lock();
    let Ok(text) = fs::read_to_string(&path) else { return Vec::new() };
    let lines: Vec<RoomLine> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let keep = limit.min(MAX_READ);
    lines[lines.len().saturating_sub(keep)..].to_vec()
}

/// When the file has more than 5000 lines, rewrites it with the newest 5000. Lines are kept as they are.
pub fn trim_lines(root: &Path, room_id: &str) -> Result<(), RoomError> {
    let path = lines_path(root, room_id).ok_or(RoomError::BadId)?;
    let _files = store::files_lock();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(RoomError::Io(e.to_string())),
    };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() <= MAX_LINES_KEPT {
        return Ok(());
    }
    let mut kept = lines[lines.len() - MAX_LINES_KEPT..].join("\n");
    kept.push('\n');
    secure_file::write_private(&path, &kept).map_err(RoomError::Io)
}

/// Removes the room from the index and deletes its lines. A room that is not in the index is `NotFound`.
pub fn delete_room(root: &Path, room_id: &str) -> Result<(), RoomError> {
    let path = lines_path(root, room_id).ok_or(RoomError::BadId)?;
    let mut rooms = load_rooms(root)?;
    let before = rooms.len();
    rooms.retain(|r| r.id != room_id);
    if rooms.len() == before {
        return Err(RoomError::NotFound);
    }
    save_rooms(root, &rooms)?;
    let _files = store::files_lock();
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(RoomError::Io(e.to_string())),
    }
}

/// Every kept line of a group (at most 5000), oldest first. For the chat search, which reads whole threads; the chat
/// itself reads `read_lines`. A missing file is empty; a broken line is skipped.
pub fn read_all(root: &Path, room_id: &str) -> Vec<RoomLine> {
    let Some(path) = lines_path(root, room_id) else { return Vec::new() };
    let _files = store::files_lock();
    let Ok(text) = fs::read_to_string(&path) else { return Vec::new() };
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// The size and change time of a group's lines file, to tell whether a remembered copy is still current.
pub fn stamp(root: &Path, room_id: &str) -> (u64, i64) {
    lines_path(root, room_id).map(|p| super::super::search::file_stamp(&p)).unwrap_or((0, 0))
}
