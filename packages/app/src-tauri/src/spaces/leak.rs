//! Last check on a bot's words before they leave this PC: blocks what looks private (the home folder, the user's
//! folder, an email, a token, a long secret). A net, not a wall: it misses some things, so anything a bot can read
//! in a space is treated as readable by that space.
//!
//! Every scan works on the original text. Offsets found in a lowercased copy are never used to slice the original
//! (lowercasing can change byte lengths, and a slice inside a multi-byte character panics).

/// What counts as private on this PC. `for_this_pc` fills it in; tests build their own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrivateMarks {
    pub home: Option<String>,
    pub user: Option<String>,
    /// The owner's own emails, machine name and the like, when known.
    pub extra: Vec<String>,
}

impl PrivateMarks {
    /// The home folder and the user name as the rest of the app finds them.
    pub fn for_this_pc() -> Self {
        let home = crate::project_path::home_dir();
        let user = home.as_ref().and_then(|h| h.file_name()).map(|n| n.to_string_lossy().to_string());
        Self { home: home.map(|h| h.to_string_lossy().to_string()), user, extra: Vec::new() }
    }
}

/// Lowercase, `\` as `/`, and a run of slashes as one: `C:\\Users\\Ren` and `c:/users/ren` compare equal.
fn path_form(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        let c = if c == '\\' { '/' } else { c };
        if c == '/' && out.ends_with('/') { continue; }
        out.push(c);
    }
    out
}

fn is_word(c: char) -> bool { c.is_alphanumeric() || c == '_' }

/// `needle` in `hay` where the character after it does not continue a name.
fn contains_whole(hay: &str, needle: &str) -> bool {
    if needle.is_empty() { return false; }
    hay.match_indices(needle).any(|(i, _)| !hay[i + needle.len()..].chars().next().is_some_and(|c| is_word(c) || c == '-'))
}

/// Returns why the text looks private, or None.
pub fn leak_reason(text: &str, marks: &PrivateMarks) -> Option<&'static str> {
    let paths = path_form(text);
    if let Some(h) = &marks.home {
        let h = path_form(h);
        let h = h.trim_end_matches('/');
        if !h.is_empty() && contains_whole(&paths, h) { return Some("home folder"); }
    }
    if let Some(u) = &marks.user {
        let u = path_form(u);
        if !u.is_empty() && !u.contains('/') {
            for p in [format!("/users/{u}"), format!("/home/{u}")] {
                if contains_whole(&paths, &p) { return Some("user folder"); }
            }
        }
    }
    let lower = text.to_lowercase();
    for e in &marks.extra {
        let e = e.to_lowercase();
        if !e.is_empty() && lower.contains(&e) { return Some("owner detail"); }
    }
    if has_email(text) { return Some("email"); }
    if has_token(text) { return Some("token"); }
    if longest_run(text, |c| c.is_ascii_hexdigit()) >= 40 { return Some("long hex"); }
    if longest_run(text, |c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=') >= 40 { return Some("long secret"); }
    None
}

/// Every byte offset where the ASCII `pat` (lowercase) starts in `text`, ignoring ASCII case. A match starts on an
/// ASCII byte, so each offset is a character boundary of `text`.
fn find_ci(text: &str, pat: &str) -> Vec<usize> {
    let (t, p) = (text.as_bytes(), pat.as_bytes());
    if p.is_empty() || t.len() < p.len() { return Vec::new(); }
    (0..=t.len() - p.len()).filter(|&i| t[i..i + p.len()].eq_ignore_ascii_case(p)).collect()
}

/// Characters of a secret that follow a prefix without a break.
fn secret_run(rest: &str) -> usize {
    rest.chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '=')).count()
}

fn has_token(text: &str) -> bool {
    // (prefix, how many secret characters must follow it in one piece)
    const PREFIXED: [(&str, usize); 8] = [
        ("sk-", 16), ("ghp_", 16), ("gho_", 16), ("github_pat_", 16), ("xoxb-", 16), ("xoxp-", 16), ("eyj", 30), ("glpat-", 16),
    ];
    for (prefix, need) in PREFIXED {
        for i in find_ci(text, prefix) {
            // In the middle of a word ("task-based") it is not a key.
            if text[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric()) { continue; }
            if secret_run(&text[i + prefix.len()..]) >= need { return true; }
        }
    }
    // An AWS access key id: AKIA and 16 capitals or digits.
    for (i, _) in text.match_indices("AKIA") {
        let tail: Vec<char> = text[i + 4..].chars().take(16).collect();
        if tail.len() == 16 && tail.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) { return true; }
    }
    !find_ci(text, "-----begin").is_empty() || !find_ci(text, "ssh-ed25519 ").is_empty() || !find_ci(text, "ssh-rsa ").is_empty()
}

/// `name@host.tld` by hand (no regex crate): a local part of `A-Za-z0-9._%+-`, a host of `A-Za-z0-9.-`, a last
/// label of two or more letters.
fn has_email(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let local = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-');
    let host = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-');
    for (i, c) in chars.iter().enumerate() {
        if *c != '@' || i == 0 || !local(chars[i - 1]) { continue; }
        let tail: String = chars[i + 1..].iter().copied().take_while(|c| host(*c)).collect();
        let tail = tail.trim_end_matches('.');
        if let Some((name, tld)) = tail.rsplit_once('.') {
            if !name.is_empty() && tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()) { return true; }
        }
    }
    false
}

fn longest_run(text: &str, ok: impl Fn(char) -> bool) -> usize {
    let (mut best, mut now) = (0usize, 0usize);
    for c in text.chars() {
        if ok(c) { now += 1; best = best.max(now); } else { now = 0; }
    }
    best
}
