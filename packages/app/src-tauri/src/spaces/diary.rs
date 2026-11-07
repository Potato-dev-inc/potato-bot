//! The space diary: what was said in a space, kept for a bot on its owner's PC, one JSON line per message.
//!
//! `<bot vault>/spaces/<space hex>/journal/YYYY-MM-DD.jsonl`, one object per line:
//! `{"id":"<entry hex>","at":0,"who":"Sam","via":"Space","channel":"general","text":".."}`.
//! The bot reads it only through its own memory tools, which wrap it as outside text. It is never put in notes, the
//! people file, a skill or the system prompt. Nothing here asks a model to write a summary, so a delete only has to
//! take the line out of the file.
//!
//! Which vault a bot has is the caller's to say (`DiaryHome`); this file only knows paths below it.
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::TimeZone;
use serde_json::json;

use super::ids::SpaceId;

/// A day file stops growing here. Later lines of that day are not kept.
pub const DAY_FILE_MAX_BYTES: u64 = 2 * 1024 * 1024;
/// Day files older than this go when a bot leaves a space and when the app starts.
pub const KEEP_MS: u64 = 90 * 24 * 60 * 60 * 1000;
/// The tag on a line from this app's own channel.
pub const VIA_SPACE: &str = "Space";

/// Where a bot's vault is. The Potato side answers; this file never names how the vault is made.
pub trait DiaryHome: Send + Sync {
    fn vault_of(&self, bot: &str) -> Option<PathBuf>;
}

/// One message as the diary keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalLine {
    /// The message id (entry hex): a delete finds the line by it.
    pub id: String,
    pub at: u64,
    pub who: String,
    pub via: String,
    pub channel: String,
    pub text: String,
}

/// `<vault>/spaces/<space hex>/journal`.
pub fn journal_dir(vault: &Path, space: &SpaceId) -> PathBuf {
    vault.join("spaces").join(space.to_hex()).join("journal")
}

/// The local calendar day of an instant, `YYYY-MM-DD` (the day the bot's own clock calls it).
pub fn day_of(at_ms: u64) -> String {
    match chrono::Local.timestamp_millis_opt(at_ms as i64).single() {
        Some(t) => t.format("%Y-%m-%d").to_string(),
        None => "1970-01-01".to_string(),
    }
}

fn only_hex(s: &str) -> bool { !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit()) }

/// Adds one line to its day's file. `Ok(false)`: the file is full for that day, nothing was written.
pub fn append(vault: &Path, space: &SpaceId, line: &JournalLine) -> std::io::Result<bool> {
    if !only_hex(&line.id) { return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad message id")); }
    let dir = journal_dir(vault, space);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.jsonl", day_of(line.at)));
    let mut text = json!({ "id": line.id, "at": line.at, "who": line.who, "via": line.via, "channel": line.channel, "text": line.text })
        .to_string();
    text.push('\n');
    let have = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if have + text.len() as u64 > DAY_FILE_MAX_BYTES { return Ok(false); }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    file.write_all(text.as_bytes())?;
    Ok(true)
}

/// Every day file of a space, oldest first.
fn day_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl") && p.file_stem().and_then(|s| s.to_str()).is_some_and(is_day))
        .collect();
    out.sort();
    out
}

fn is_day(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
}

/// Takes the lines of one message out of the diary (all day files, so a change of time zone cannot hide one). A file
/// left empty is removed. Returns how many lines went.
pub fn remove(vault: &Path, space: &SpaceId, entry_hex: &str) -> std::io::Result<usize> {
    if !only_hex(entry_hex) { return Ok(0); }
    let needle = format!("\"id\":\"{entry_hex}\"");
    let mut gone = 0;
    for path in day_files(&journal_dir(vault, space)) {
        let body = std::fs::read_to_string(&path)?;
        if !body.contains(&needle) { continue; }
        let keep: Vec<&str> = body
            .lines()
            .filter(|l| {
                let hit = serde_json::from_str::<serde_json::Value>(l).ok().and_then(|v| v.get("id").and_then(|i| i.as_str()).map(|i| i == entry_hex)).unwrap_or(false);
                if hit { gone += 1; }
                !hit
            })
            .collect();
        if keep.is_empty() {
            std::fs::remove_file(&path)?;
        } else {
            let tmp = path.with_extension("jsonl.tmp");
            std::fs::write(&tmp, format!("{}\n", keep.join("\n")))?;
            std::fs::rename(&tmp, &path)?;
        }
    }
    Ok(gone)
}

/// Removes day files older than 90 days. Returns how many files went.
pub fn prune(vault: &Path, space: &SpaceId, now_ms: u64) -> usize {
    let cutoff = day_of(now_ms.saturating_sub(KEEP_MS));
    let mut gone = 0;
    for path in day_files(&journal_dir(vault, space)) {
        let day = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if day < cutoff.as_str() && std::fs::remove_file(&path).is_ok() { gone += 1; }
    }
    gone
}

/// Removes a whole space's diary (the space was forgotten on this PC).
pub fn wipe(vault: &Path, space: &SpaceId) {
    let _ = std::fs::remove_dir_all(vault.join("spaces").join(space.to_hex()));
}

/// Every line of a space's diary, oldest day first (tests and the app's own counters read with this).
pub fn read_all(vault: &Path, space: &SpaceId) -> Vec<JournalLine> {
    let mut out = Vec::new();
    for path in day_files(&journal_dir(vault, space)) {
        let Ok(body) = std::fs::read_to_string(&path) else { continue };
        for l in body.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            out.push(JournalLine { id: s("id"), at: v.get("at").and_then(|x| x.as_u64()).unwrap_or(0), who: s("who"), via: s("via"), channel: s("channel"), text: s("text") });
        }
    }
    out
}
