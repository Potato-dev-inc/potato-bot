//! Reading the log back: the tip to continue from, the recent lines for the Settings list, and the
//! chain check.

use super::{Entry, Head, Tip, GENESIS, HEAD_FILE};
use std::path::{Path, PathBuf};

/// The month files in order, oldest first.
fn log_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

fn read_head(dir: &Path) -> Option<Head> {
    serde_json::from_slice(&std::fs::read(dir.join(HEAD_FILE)).ok()?).ok()
}

/// The newest line's position, or the head file, or the start of a new chain.
pub(super) fn load_tip(dir: &Path) -> Tip {
    let last_line = log_files(dir)
        .last()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| text.lines().rev().find(|l| !l.trim().is_empty()).map(str::to_string));
    if let Some(entry) = last_line.and_then(|l| serde_json::from_str::<Entry>(&l).ok()) {
        return Tip { seq: entry.seq, hash: entry.hash, ts_ms: entry.ts_ms };
    }
    match read_head(dir) {
        Some(head) => Tip { seq: head.seq, hash: head.hash, ts_ms: 0 },
        None => Tip { seq: 0, hash: GENESIS.to_string(), ts_ms: 0 },
    }
}

