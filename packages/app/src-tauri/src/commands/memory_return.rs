//! `autoMemoryReclaim=gradual` in the person's `.wslconfig`: WSL then hands unused memory back to Windows instead of keeping
//! what the sandbox once used (the "VmmemWSL" line in Task Manager). It only takes effect when WSL next starts, so the app never
//! restarts WSL for it (that would stop the person's other distros too); it says so.
//!
//! Safe by construction: the person's other settings are kept byte for byte (line endings too), a key the person already set
//! (to any value) is never changed, a copy of the file is kept once, and the file is written only through a temp file and a
//! rename. Pure text work is in `with_auto_reclaim`; the file work is in `enable`.
//!
//! This file is deliberately NOT in `potato/sandbox/`: the sandbox code never touches the person's home or their WSL settings
//! (a guard test enforces it). This is the one named exception, run only by the person's own button press, and it edits one line.

use std::path::{Path, PathBuf};

pub const KEY: &str = "autoMemoryReclaim";
pub const VALUE: &str = "gradual";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// The text with the setting added.
    Added(String),
    /// The person already has the key; the value is theirs and is left alone.
    AlreadySet(String),
}

fn key_value(line: &str) -> Option<(&str, &str)> {
    let t = line.trim();
    if t.starts_with('#') || t.starts_with(';') {
        return None;
    }
    let (k, v) = t.split_once('=')?;
    Some((k.trim(), v.trim()))
}

/// Adds `autoMemoryReclaim=gradual` under `[experimental]` (making the section if there is none).
pub fn with_auto_reclaim(text: &str) -> Change {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut in_experimental = false;
    let mut header_at: Option<usize> = None;
    for (i, raw) in lines.iter().enumerate() {
        let t = raw.trim();
        if t.starts_with('[') && t.ends_with(']') {
            in_experimental = t.eq_ignore_ascii_case("[experimental]");
            if in_experimental {
                header_at = Some(i);
            }
            continue;
        }
        if in_experimental {
            if let Some((k, v)) = key_value(raw) {
                if k.eq_ignore_ascii_case(KEY) {
                    return Change::AlreadySet(v.to_string());
                }
            }
        }
    }
    let added = format!("{KEY}={VALUE}{eol}");
    match header_at {
        Some(at) => {
            let mut out = String::new();
            for (i, raw) in lines.iter().enumerate() {
                out.push_str(raw);
                if i == at {
                    if !raw.ends_with('\n') {
                        out.push_str(eol);
                    }
                    out.push_str(&added);
                }
            }
            Change::Added(out)
        }
        None => {
            let mut out = text.to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push_str(eol);
            }
            if !out.is_empty() {
                out.push_str(eol);
            }
            out.push_str(&format!("[experimental]{eol}{added}"));
            Change::Added(out)
        }
    }
}

/// Where the person's file is.
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(|home| PathBuf::from(home).join(".wslconfig"))
}

/// What the file says now: the value (`gradual`, or the person's own), or `None` when the key is not there.
pub fn current(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    match with_auto_reclaim(&text) {
        Change::AlreadySet(v) => Some(v),
        Change::Added(_) => None,
    }
}

/// Turns it on. Returns what was found: `Added` (written now), or `AlreadySet(value)` (left alone).
pub fn enable(path: &Path) -> std::io::Result<Change> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let text = existing.clone().unwrap_or_default();
    match with_auto_reclaim(&text) {
        Change::AlreadySet(v) => Ok(Change::AlreadySet(v)),
        Change::Added(new_text) => {
            if existing.is_some() {
                let backup = path.with_file_name(".wslconfig.potato-backup");
                if !backup.exists() {
                    std::fs::copy(path, &backup)?;
                }
            }
            let tmp = path.with_file_name(".wslconfig.potato-tmp");
            std::fs::write(&tmp, &new_text)?;
            std::fs::rename(&tmp, path)?;
            Ok(Change::Added(new_text))
        }
    }
}


use serde::Serialize;

/// Whether WSL gives unused memory back to Windows (`autoMemoryReclaim` in the person's .wslconfig).
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryReturn {
    /// `on` (gradual), `other` (the person chose a different value; left alone) or `off`.
    pub state: &'static str,
    pub value: Option<String>,
}

fn memory_return_state(value: Option<String>) -> MemoryReturn {
    match value.as_deref() {
        Some("gradual") => MemoryReturn { state: "on", value },
        Some(_) => MemoryReturn { state: "other", value },
        None => MemoryReturn { state: "off", value: None },
    }
}

#[tauri::command]
pub fn potato_sandbox_memory_return_get() -> MemoryReturn {
    memory_return_state(default_path().and_then(|p| current(&p)))
}

/// Adds `autoMemoryReclaim=gradual` to the person's .wslconfig (keeping everything else, with a copy kept once). It takes effect
/// the next time WSL starts; WSL is not restarted here.
#[tauri::command]
pub fn potato_sandbox_memory_return_enable() -> Result<MemoryReturn, String> {
    let path = default_path().ok_or_else(|| "SANDBOX_NO_HOME: Could not find your user folder.".to_string())?;
    enable(&path).map_err(|_| "SANDBOX_WSLCONFIG: Could not write your .wslconfig file. Nothing was changed.".to_string())?;
    Ok(memory_return_state(current(&path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn added(text: &str) -> String {
        match with_auto_reclaim(text) {
            Change::Added(t) => t,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_empty_or_missing_file_gets_the_section() {
        assert_eq!(added(""), "[experimental]\nautoMemoryReclaim=gradual\n");
    }

    #[test]
    fn the_persons_other_settings_are_kept_exactly_and_the_section_is_appended() {
        let before = "[wsl2]\r\nmemory=8GB\r\nprocessors=4\r\n# my note\r\n[interop]\r\nenabled=true";
        let out = added(before);
        assert!(out.starts_with(before), "{out:?}");
        assert!(out.ends_with("[experimental]\r\nautoMemoryReclaim=gradual\r\n"), "{out:?}");
    }

    #[test]
    fn an_existing_experimental_section_gets_the_key_right_under_its_header_and_nothing_else_moves() {
        let before = "[wsl2]\nmemory=8GB\n[experimental]\nsparseVhd=true\nhostAddressLoopback=true\n";
        assert_eq!(added(before), "[wsl2]\nmemory=8GB\n[experimental]\nautoMemoryReclaim=gradual\nsparseVhd=true\nhostAddressLoopback=true\n");
    }

    #[test]
    fn a_value_the_person_already_chose_is_never_changed_whatever_it_is() {
        assert_eq!(with_auto_reclaim("[experimental]\nautoMemoryReclaim=dropcache\n"), Change::AlreadySet("dropcache".into()));
        assert_eq!(with_auto_reclaim("[experimental]\n  AutoMemoryReclaim = disabled\n"), Change::AlreadySet("disabled".into()));
        // in another section, or commented out, it does not count
        assert!(matches!(with_auto_reclaim("[wsl2]\nautoMemoryReclaim=x\n"), Change::Added(_)));
        assert!(matches!(with_auto_reclaim("[experimental]\n# autoMemoryReclaim=gradual\n"), Change::Added(_)));
    }

    #[test]
    fn the_file_is_backed_up_once_written_through_a_rename_and_a_second_run_changes_nothing() {
        let dir = std::env::temp_dir().join(format!("pm-wslconfig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".wslconfig");
        std::fs::write(&path, "[wsl2]\nmemory=6GB\n").unwrap();
        assert_eq!(current(&path), None);
        assert!(matches!(enable(&path).unwrap(), Change::Added(_)));
        assert_eq!(std::fs::read_to_string(dir.join(".wslconfig.potato-backup")).unwrap(), "[wsl2]\nmemory=6GB\n");
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.starts_with("[wsl2]\nmemory=6GB\n") && written.contains("autoMemoryReclaim=gradual"));
        assert_eq!(current(&path).as_deref(), Some("gradual"));
        assert_eq!(enable(&path).unwrap(), Change::AlreadySet("gradual".into()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), written, "a second run writes nothing");
        assert!(!dir.join(".wslconfig.potato-tmp").exists());
        // no file at all: it is made, with no backup to keep
        let none = dir.join("sub");
        std::fs::create_dir_all(&none).unwrap();
        let fresh = none.join(".wslconfig");
        assert!(matches!(enable(&fresh).unwrap(), Change::Added(_)));
        assert!(!none.join(".wslconfig.potato-backup").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
