//! Search chats: one box that looks for words in every bot's chat and every group's thread, newest match first, for the
//! Search chats page and its Ctrl+K entry. Local only: nothing leaves the machine, nothing is written.
//!
//! The "index" is a remembered, lower-cased copy of each thread, kept per thread and refreshed only when its file
//! changed (size and change time). The first search reads the files; later ones touch only a thread that moved. The
//! caller runs a search off the UI thread (`commands/potato_search.rs`), and a thread is bounded by the store
//! (two files of 2 MB for a chat, 5000 lines for a group), so a long history is a scan of memory, not of the disk.
//!
//! A search is words, all of which must be in the message (any order, any case). It returns at most `MAX_HITS` hits with a
//! short snippet and the character ranges to highlight, and `from_end`: how many messages come after it, which tells the
//! page how far back the thread must be read to show the message.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;

use super::rooms::store as rstore;
use super::store::{self, FromKind, MsgKind, StoreError, StoreResult};

/// Longest search text, in characters.
pub const MAX_QUERY_CHARS: usize = 100;
/// Most words in one search (the rest are ignored).
pub const MAX_WORDS: usize = 8;
/// Most hits one search returns.
pub const MAX_HITS: usize = 50;
/// Characters of a message shown before the first match, and after it.
pub const SNIPPET_BEFORE: usize = 40;
pub const SNIPPET_AFTER: usize = 90;
/// Threads kept in memory; more than this and the copy starts again.
const CACHE_MAX: usize = 128;
const BAD_QUERY: &str = "POTATO_SEARCH_BAD_QUERY";

struct Line {
    id: String,
    at: i64,
    speaker: String,
    /// The message as written; empty for a row that is not a chat line (a card, a note).
    text: String,
    lower: String,
}

type Stamp = (u64, i64, u64, i64);

struct Thread {
    stamp: Stamp,
    lines: Arc<Vec<Line>>,
}

static CACHE: Lazy<Mutex<HashMap<String, Thread>>> = Lazy::new(Default::default);

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    /// `bot` or `group`.
    pub kind: &'static str,
    pub owner_id: String,
    pub owner_name: String,
    pub message_id: String,
    pub at: i64,
    pub speaker: String,
    pub snippet: String,
    /// Character ranges `[start, end)` of the snippet to highlight.
    pub marks: Vec<[usize; 2]>,
    /// How many messages come after this one in its thread.
    pub from_end: usize,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Results {
    pub hits: Vec<Hit>,
    /// Matches in all, which may be more than `hits` holds.
    pub matches: usize,
    /// Messages looked through.
    pub searched: usize,
}

pub fn file_stamp(path: &Path) -> (u64, i64) {
    std::fs::metadata(path)
        .map(|m| {
            let at = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
            (m.len(), at)
        })
        .unwrap_or((0, 0))
}

/// The same letters with each letter lower-cased to one letter, so a character keeps its place.
fn fold(text: &str) -> String {
    text.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}

/// The words of a search: lower-cased, no empty ones, at most `MAX_WORDS`.
pub fn words(query: &str) -> StoreResult<Vec<String>> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > MAX_QUERY_CHARS {
        return Err(StoreError::new(BAD_QUERY, format!("Type the words to look for, {MAX_QUERY_CHARS} characters at most.")));
    }
    Ok(fold(query).split_whitespace().take(MAX_WORDS).map(str::to_string).collect())
}

fn dm_lines(root: &Path, bot: &str, bot_name: &str) -> StoreResult<Arc<Vec<Line>>> {
    let conversation = store::app_conversation(bot);
    let path = store::conversation_path(root, &conversation)?;
    let key = format!("{}|{conversation}", root.display());
    let (a, b) = (file_stamp(&store::old_path(&path)), file_stamp(&path));
    let stamp = (a.0, a.1, b.0, b.1);
    if let Some(t) = CACHE.lock().get(&key) {
        if t.stamp == stamp {
            return Ok(t.lines.clone());
        }
    }
    let all = store::read_messages(root, &conversation, usize::MAX, None)?;
    let lines: Vec<Line> = all
        .into_iter()
        .map(|m| {
            let searchable = m.kind == MsgKind::Text && !m.text.trim().is_empty();
            let speaker = match m.from.kind {
                FromKind::You => "You".to_string(),
                FromKind::Bot => m.from.name.clone().unwrap_or_else(|| bot_name.to_string()),
                FromKind::Person => m.from.name.clone().unwrap_or_else(|| "Someone".to_string()),
                FromKind::Note => String::new(),
            };
            let text = if searchable { m.text } else { String::new() };
            Line { id: m.id, at: m.at, speaker, lower: fold(&text), text }
        })
        .collect();
    Ok(remember(key, stamp, lines))
}

fn room_lines(root: &Path, room_id: &str) -> Arc<Vec<Line>> {
    let key = format!("{}|room:{room_id}", root.display());
    let (len, at) = rstore::stamp(root, room_id);
    let stamp = (len, at, 0, 0);
    if let Some(t) = CACHE.lock().get(&key) {
        if t.stamp == stamp {
            return t.lines.clone();
        }
    }
    let lines: Vec<Line> = rstore::read_all(root, room_id)
        .into_iter()
        .map(|l| Line { lower: fold(&l.text), speaker: l.from.name, text: l.text, id: l.id, at: l.at })
        .collect();
    remember(key, stamp, lines)
}

fn remember(key: String, stamp: Stamp, lines: Vec<Line>) -> Arc<Vec<Line>> {
    let lines = Arc::new(lines);
    let mut cache = CACHE.lock();
    if cache.len() >= CACHE_MAX && !cache.contains_key(&key) {
        cache.clear();
    }
    cache.insert(key, Thread { stamp, lines: lines.clone() });
    lines
}

/// Every place of `chars` where `needle` starts.
fn find_all(chars: &[char], needle: &[char]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > chars.len() {
        return Vec::new();
    }
    (0..=chars.len() - needle.len()).filter(|&i| chars[i..i + needle.len()] == *needle).collect()
}

fn snippet_of(line: &Line, words: &[String]) -> (String, Vec<[usize; 2]>) {
    let chars: Vec<char> = line.text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let lower: Vec<char> = line.lower.chars().collect();
    let needles: Vec<Vec<char>> = words.iter().map(|w| w.chars().collect()).collect();
    let first = needles.iter().filter_map(|n| find_all(&lower, n).first().map(|&i| (i, n.len()))).min().unwrap_or((0, 0));
    let from = first.0.saturating_sub(SNIPPET_BEFORE);
    let to = (first.0 + first.1 + SNIPPET_AFTER).min(chars.len());
    let lead = usize::from(from > 0);
    let mut out = String::new();
    if from > 0 {
        out.push('…');
    }
    out.extend(chars[from..to].iter());
    if to < chars.len() {
        out.push('…');
    }
    let mut marks: Vec<[usize; 2]> = Vec::new();
    for n in &needles {
        for i in find_all(&lower, n) {
            if i >= from && i + n.len() <= to {
                marks.push([i - from + lead, i - from + lead + n.len()]);
            }
        }
    }
    marks.sort();
    // Overlapping marks (two words that share letters) become one.
    let mut merged: Vec<[usize; 2]> = Vec::new();
    for m in marks {
        match merged.last_mut() {
            Some(last) if m[0] <= last[1] => last[1] = last[1].max(m[1]),
            _ => merged.push(m),
        }
    }
    (out, merged)
}

type Source = (&'static str, String, String, Arc<Vec<Line>>);

/// Looks through every bot's chat and every group's thread. `bot_name` gives a bot's name for the hit.
pub fn run(root: &Path, query: &str, bot_name: &dyn Fn(&str) -> String) -> StoreResult<Results> {
    let words = words(query)?;
    let mut out = Results::default();
    let mut sources: Vec<Source> = Vec::new();
    for bot in store::load_bots(root)? {
        let name = bot_name(&bot.id);
        match dm_lines(root, &bot.id, &name) {
            Ok(lines) => sources.push(("bot", bot.id.clone(), name, lines)),
            // One damaged chat does not hide the others.
            Err(e) => tracing::debug!(%e, "search skipped a chat"),
        }
    }
    for room in rstore::load_rooms(root).unwrap_or_default() {
        let lines = room_lines(root, &room.id);
        sources.push(("group", room.id, room.name, lines));
    }
    let mut found: Vec<Hit> = Vec::new();
    for (kind, owner_id, owner_name, lines) in &sources {
        out.searched += lines.len();
        let mut kept = 0usize;
        for (i, line) in lines.iter().enumerate().rev() {
            if line.lower.is_empty() || !words.iter().all(|w| line.lower.contains(w.as_str())) {
                continue;
            }
            out.matches += 1;
            if kept >= MAX_HITS {
                continue;
            }
            kept += 1;
            let (snippet, marks) = snippet_of(line, &words);
            found.push(Hit {
                kind,
                owner_id: owner_id.clone(),
                owner_name: owner_name.clone(),
                message_id: line.id.clone(),
                at: line.at,
                speaker: line.speaker.clone(),
                snippet,
                marks,
                from_end: lines.len() - 1 - i,
            });
        }
    }
    found.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.message_id.cmp(&b.message_id)));
    found.truncate(MAX_HITS);
    out.hits = found;
    Ok(out)
}

/// How many threads are remembered (a test checks the second search reuses them).
#[cfg(test)]
pub fn cached_threads() -> usize {
    CACHE.lock().len()
}
