//! Pane titles. The ambient/tray feature is gone; a pane keeps an optional person-given title.

use parking_lot::Mutex;
use std::collections::HashMap;

static TITLES: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

pub mod store {
    use std::collections::HashMap;

    pub fn get(pane_id: &str) -> Option<String> {
        super::TITLES.lock().as_ref().and_then(|m| m.get(pane_id).cloned())
    }

    pub fn set_user_title(pane_id: &str, title: &str) -> Option<String> {
        let mut guard = super::TITLES.lock();
        let map = guard.get_or_insert_with(HashMap::new);
        let text = title.trim();
        if text.is_empty() {
            map.remove(pane_id);
            None
        } else {
            map.insert(pane_id.to_string(), text.to_string());
            Some(text.to_string())
        }
    }

    pub fn observe_input(_pane_id: &str, _agent: &str, _screen_line: &str, _text: &str, _append_newline: bool) -> bool {
        false
    }
}

pub mod title {
    use once_cell::sync::Lazy;
    use regex::Regex;

    /// Whole-word secret keywords, case-insensitive: "password", "API key", "verification code"...
    static SECRET_KEYWORD: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?i)\b(?:passwords?|passphrase|passcode|pin|tokens?|secret|api[\s_-]?key|apikey|otp|2fa|verification code|enter (?:the )?code|private key)\b",
        )
        .expect("secret keyword pattern is valid")
    });

    /// A prompt terminator at the end of the trimmed line: `:`, `?` or `>`, optionally followed by an
    /// empty default such as `[]` or `(input hidden)`.
    static PROMPT_END: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"[:?>]\s*(?:\[\]|\(input hidden\))?$").expect("prompt terminator pattern is valid"));

    /// How far from the end of the line a secret keyword may end and still count, in characters.
    const KEYWORD_WINDOW_CHARS: usize = 40;

    /// The last non-blank line of a screen dump (empty when the screen is blank).
    pub fn last_line(screen: &str) -> &str {
        screen.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("")
    }

    /// True when the screen line is a prompt asking for a secret (`Password:`, `Enter passphrase ...:`,
    /// a PIN), so what is typed after it must not be recorded. The line must look like a prompt (end in
    /// `:` `?` `>`), and a secret keyword must end within the last [`KEYWORD_WINDOW_CHARS`] characters.
    /// Status lines such as "12.4k tokens" do not qualify.
    pub fn is_secret_prompt(screen_line: &str) -> bool {
        let line = screen_line.trim();
        if !PROMPT_END.is_match(line) {
            return false;
        }
        SECRET_KEYWORD
            .find_iter(line)
            .any(|found| line[found.end()..].chars().count() <= KEYWORD_WINDOW_CHARS)
    }
}

pub fn notify_changed() {}

pub fn announce_title_change(_app: &tauri::AppHandle, _pane_id: &str) {}

pub fn agent_label(agent_type: &str) -> String {
    crate::tui::descriptors()
        .into_iter()
        .find(|d| d.id == agent_type)
        .map(|d| d.label)
        .unwrap_or_else(|| agent_type.to_string())
}

pub fn window_title(agent: &str, pane_id: &str, stored: Option<&str>) -> String {
    stored
        .filter(|t| !t.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{} ({})", agent_label(agent), &pane_id[..pane_id.len().min(8)]))
}
