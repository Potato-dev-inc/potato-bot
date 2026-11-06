//! The sign-in card's pieces that need no engine: which site a card may be for, the one place a password is held in
//! the app, what the host sends the page tool, and how its answer is read.
//!
//! The password's whole trip: the card's hidden field, one app command, a `Secret` here, the page tool's stdin
//! (`browser_login`), the page's password box. It is never in an argument list (`ps` shows those), a log line, a
//! thread row, a card, a job record, a step, an answer to the bot or any file. `Secret` has no `Clone`, no `Serialize`
//! and a `Debug` that prints nothing, and it overwrites its bytes when it is dropped.
//!
//! What the page tool answers is read forgivingly and said in fixed sentences: a word the page wrote never reaches the
//! owner's card or the bot through this file.

use serde_json::Value;

use super::sandbox::browser;

/// The brain's tool: the bot is at a sign-in page and asks its owner to sign in.
pub const TOOL: &str = "request_login";

/// The three buttons of the card, in order. Only `SIGN_IN` is answered by `submit_login`, never by a plain answer.
pub const SIGN_IN: &str = "Sign in";
pub const TAKE_OVER: &str = "Take over";
pub const NOT_NOW: &str = "Not now";

pub fn choices() -> Vec<String> {
    vec![SIGN_IN.to_string(), TAKE_OVER.to_string(), NOT_NOW.to_string()]
}

pub const MAX_NAME_CHARS: usize = 200;
pub const MAX_PASSWORD_CHARS: usize = 500;

// ---- the site ------------------------------------------------------------------------------------------------

/// The origin a card is for: `https://host` (and `:port` when it is not 443), from what the bot wrote. A word like
/// `github.com`, an address with a path, or `https://GitHub.com/login?next=/` all become `https://github.com`. Plain
/// http, a name or password in the address, a private or local host and anything that is not an address are refused.
pub fn normalize_site(raw: &str) -> Result<String, &'static str> {
    let text = raw.trim();
    if text.is_empty() || text.chars().count() > 300 {
        return Err("Say which site, for example github.com.");
    }
    let lower = text.to_ascii_lowercase();
    let not_https = "Sign-in is only for sites that start with https. This one does not.";
    let rest = match lower.strip_prefix("https://") {
        Some(rest) => rest,
        None => {
            // `http://...`, `ftp://...`, and a scheme with no slashes (`javascript:...`, `mailto:...`) are not https.
            let scheme_end = lower.find(':');
            let has_scheme = scheme_end.is_some_and(|i| i > 0 && lower[..i].bytes().all(|b| b.is_ascii_alphabetic()) && !lower[i + 1..].starts_with(|c: char| c.is_ascii_digit()));
            if has_scheme {
                return Err(not_https);
            }
            lower.as_str()
        }
    };
    let candidate = format!("https://{rest}");
    // The same checks the bot's own `browser_navigate` goes through, so a site it cannot open is not a site it can ask for.
    let checked = browser::check_url(&candidate, false).map_err(|e| match e {
        browser::BrowserError::Url(why) => why,
        browser::BrowserError::Actions(_) => "That is not a site address.",
    })?;
    let after = &checked["https://".len()..];
    let end = after.find(['/', '?', '#']).unwrap_or(after.len());
    let authority = &after[..end];
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    // A sign-in is for a site with a name. A bare address (any spelling) is not one a card will vouch for.
    if host.starts_with('[') || host.split('.').all(|label| !label.is_empty() && label.bytes().all(|b| b.is_ascii_digit() || b == b'x')) {
        return Err("Sign-in is for sites with a name, not a bare address.");
    }
    Ok(match port {
        None | Some("443") => format!("https://{host}"),
        Some(p) => format!("https://{host}:{p}"),
    })
}

/// The site as a person reads it on the card: the host, without `https://`.
pub fn display_site(origin: &str) -> &str {
    origin.strip_prefix("https://").unwrap_or(origin)
}

// ---- the secret ----------------------------------------------------------------------------------------------

/// Overwrites bytes in a way the compiler may not drop.
pub(crate) fn wipe(buffer: &mut [u8]) {
    for byte in buffer.iter_mut() {
        // SAFETY: `byte` is a valid, aligned, exclusive reference to one initialized byte.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// A password. Held as bytes, wiped on drop, never printed, never serialized, never cloned.
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(text: String) -> Secret {
        Secret(text.into_bytes())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn char_count(&self) -> usize {
        String::from_utf8_lossy(&self.0).chars().count()
    }

    pub fn has_nul(&self) -> bool {
        self.0.contains(&0)
    }

    fn as_text(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }

    #[cfg(test)]
    pub fn bytes_for_test(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

/// What the page tool is given. Debug prints the site only.
pub struct Request {
    pub origin: String,
    pub username: String,
    pub password: Secret,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Request({})", self.origin)
    }
}

impl Request {
    /// The one JSON line the page tool reads on stdin. The caller wipes it after the call.
    pub fn stdin_bytes(&self) -> Vec<u8> {
        let one = |text: &str| serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
        let mut line = format!("{{\"origin\":{},\"username\":{},\"password\":", one(&self.origin), one(&self.username)).into_bytes();
        let mut password = serde_json::to_string(self.password.as_text().as_ref()).unwrap_or_else(|_| "\"\"".to_string()).into_bytes();
        line.extend_from_slice(&password);
        wipe(&mut password);
        line.extend_from_slice(b"}\n");
        line
    }
}

// ---- reading the page tool -----------------------------------------------------------------------------------

/// A refusal or failure, in a fixed sentence. `code` is a short wire word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginError {
    pub code: &'static str,
    pub message: String,
}

impl LoginError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

/// The code of a sign-in whose result the page tool could not read. The card offers Try again and It worked, continue.
pub const UNCLEAR: &str = "LOGIN_UNCLEAR";

const USE_TAKE_OVER: &str = " Use Take over to sign in yourself.";

/// The front tab's origin from the `origin` command's answer. `None`: no tab could be read.
pub fn parse_origin_reply(raw: &str) -> Option<String> {
    let value = last_object(raw)?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let origin = value.get("origin").and_then(Value::as_str)?.trim();
    (!origin.is_empty() && origin.len() <= 300 && !origin.chars().any(|c| c.is_control() || c.is_whitespace())).then(|| origin.to_string())
}

/// The check the host makes before the password leaves it: the front tab's real origin is the card's site, over https.
pub fn check_front(expected: &str, actual: Option<&str>) -> Result<(), LoginError> {
    let Some(actual) = actual else {
        return Err(LoginError::new("LOGIN_NO_PAGE", format!("The bot's browser has no page open to sign in on.{USE_TAKE_OVER}")));
    };
    if actual.eq_ignore_ascii_case("null") || actual.starts_with("http:") {
        return Err(LoginError::new("LOGIN_NOT_HTTPS", format!("The page in front is not a secure (https) page, so nothing was filled.{USE_TAKE_OVER}")));
    }
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(LoginError::new(
            "LOGIN_WRONG_SITE",
            format!("The page in front is {}, not {}, so nothing was filled.", display_site(actual), display_site(expected)),
        ));
    }
    Ok(())
}

/// What a finished sign-in tells the host. Nothing the page wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    pub origin_after: String,
    pub login_box_still_shown: bool,
    pub cleared_from_page: bool,
}

/// Reads the `login` command's answer. Failures become fixed sentences; the tool's own words are not passed on.
pub fn parse_login_reply(raw: &str) -> Result<Done, LoginError> {
    let Some(value) = last_object(raw) else {
        return Err(LoginError::new("LOGIN_FAILED", format!("The browser did not answer, and it is not known whether the form was filled.{USE_TAKE_OVER}")));
    };
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(Done {
            origin_after: value.get("origin").and_then(Value::as_str).unwrap_or("").chars().take(200).collect(),
            login_box_still_shown: value.get("loginBoxStillShown").and_then(Value::as_bool).unwrap_or(false),
            cleared_from_page: value.get("clearedFromPage").and_then(Value::as_bool).unwrap_or(false),
        });
    }
    let code = value.get("code").and_then(Value::as_str).unwrap_or("");
    Err(match code {
        "LOGIN_WRONG_SITE" => LoginError::new("LOGIN_WRONG_SITE", "The page in front is not that site, so nothing was filled."),
        "LOGIN_NOT_HTTPS" => LoginError::new("LOGIN_NOT_HTTPS", "Sign-in is only for secure (https) pages, so nothing was filled."),
        "LOGIN_NO_FORM" | "LOGIN_REFUSED" => LoginError::new(
            "LOGIN_NO_FORM",
            format!("There is no single sign-in form on this page to fill (it may be a sign-up page, or the password comes on the next page).{USE_TAKE_OVER}"),
        ),
        "LOGIN_PASSWORD_LEFT" => LoginError::new(
            "LOGIN_PASSWORD_LEFT",
            "The form was submitted, but the password could not be cleared from the page. Do not leave the page open; use Take over to check it.",
        ),
        "LOGIN_NOT_ACCEPTED" => LoginError::new(
            "LOGIN_NOT_ACCEPTED",
            "The page showed an error after Sign in, so the password may be wrong. It was cleared from the page. Try again, or use Take over.",
        ),
        "LOGIN_UNCLEAR" => LoginError::new(UNCLEAR, "Couldn't tell if that worked. Try again, or if the page shows you are signed in, choose It worked, continue."),
        "LOGIN_BAD_ARGS" => LoginError::new("LOGIN_BAD_ARGS", "A user name and a password are both needed."),
        _ => LoginError::new("LOGIN_FAILED", format!("The browser could not finish the sign-in.{USE_TAKE_OVER}")),
    })
}

/// The `forget` command's answer: how many cookies went.
pub fn parse_forget_reply(raw: &str) -> Result<u64, LoginError> {
    let value = last_object(raw).ok_or_else(|| LoginError::new("LOGIN_FAILED", "The browser did not answer."))?;
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(value.get("cookiesCleared").and_then(Value::as_u64).unwrap_or(0))
    } else {
        Err(LoginError::new("LOGIN_FAILED", "The browser could not sign out of that site. Open the sandbox browser and try again."))
    }
}

fn last_object(raw: &str) -> Option<Value> {
    raw.trim().lines().rev().map(str::trim).filter(|l| l.starts_with('{')).find_map(|l| serde_json::from_str::<Value>(l).ok()).filter(Value::is_object)
}
