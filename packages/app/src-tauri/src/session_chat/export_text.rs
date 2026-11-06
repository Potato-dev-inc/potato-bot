//! Text for the saved chat page: redacted, home folder and folder paths cut down, HTML-escaped.

use super::home_path::{home_to_tilde, is_sep};
#[cfg(test)]
use crate::redact::{self, RedactContext};

/// Plain text for a file that is not HTML: redacted in strict mode, then with its e-mail addresses masked and its paths cut down to their last names.
/// Not HTML-escaped, so the caller must not put it into markup without escaping. Reuses `cut_paths`.
#[cfg(test)]
pub fn plain(text: &str, ctx: &RedactContext) -> String {
    cut_paths(text, ctx)
}

/// The last name of a path (`C:\Users\you\work\app` gives `app`); empty when there is none.
pub fn last_name(path: &str) -> String {
    segments(path).last().map(|name| name.to_string()).unwrap_or_default()
}

/// Cuts every path in `text` down to its last name. The home folder is first replaced by `~` (see `home_to_tilde`),
/// so `C:\Users\Default User\proj\a.rs` becomes `a.rs` and a bare `~` stays `~`. A path needs at least two names after
/// its root, so `/api` stays as it is. Paths end at whitespace or a quote; sentence punctuation after one is kept.
pub fn strip_paths(text: &str, home: Option<&str>) -> String {
    let text = home_to_tilde(text, home);
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        match path_end(&chars, i) {
            Some(end) => {
                let token: String = chars[i..end].iter().collect();
                out.push_str(&last_name(&token));
                i = end;
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

/// Where the absolute path (or `~` path) starting at `i` ends, when one starts there.
fn path_end(chars: &[char], i: usize) -> Option<usize> {
    let before = if i == 0 { None } else { Some(chars[i - 1]) };
    let drive = chars[i].is_ascii_alphabetic()
        && chars.get(i + 1) == Some(&':')
        && matches!(chars.get(i + 2), Some('\\') | Some('/'))
        && !before.is_some_and(|c| c.is_alphanumeric());
    let boundary = before.map_or(true, |c| c.is_whitespace() || matches!(c, '"' | '\'' | '(' | '=' | '`' | '[' | '<'));
    let rooted = chars[i] == '/' && chars.get(i + 1).is_some_and(|c| *c != '/' && !c.is_whitespace()) && boundary;
    let home = chars[i] == '~' && chars.get(i + 1).is_some_and(|c| is_sep(*c)) && boundary;
    if !drive && !rooted && !home {
        return None;
    }
    let mut end = i;
    while end < chars.len() && !is_path_char_end(chars[end]) {
        end += 1;
    }
    while end > i && matches!(chars[end - 1], '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}') {
        end -= 1;
    }
    let token: String = chars[i..end].iter().collect();
    (segments(&token).len() >= 2).then_some(end)
}

fn is_path_char_end(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '|' | '*' | '?' | '`')
}

fn segments(path: &str) -> Vec<&str> {
    path.split(['\\', '/']).filter(|part| !part.is_empty()).collect()
}

#[cfg(test)]
/// `text` redacted in strict mode, then with its e-mail addresses masked (strict does not mask them, and a page may be
/// sent on), then with its paths cut down (home folder first, then folder paths). Every export text goes through here
/// before it is split into lines, so no line of a page or file keeps an address.
pub fn cut_paths(text: &str, ctx: &RedactContext) -> String {
    let redacted = redact::strict_with(text, ctx);
    strip_paths(&redact::mask_emails(redacted.as_str()), ctx.home.as_deref())
}
