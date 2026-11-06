//! The sign-in card in the engine (`login.rs` has the pieces, `login_calls.rs` the moment of the fill).
//!
//! - the brain's `request_login {site, why}`: the owner's private chat only, never a group, a routine or a stranger. It
//!   takes exactly two arguments; anything else is refused, so there is no argument a password could ride in. It makes a
//!   card with three buttons (Sign in, Take over, Not now) and the job waits for it like for any card;
//! - `submit_login`, the app command behind Sign in. It fills the page through the host and answers the card only if that
//!   worked, so the bot is never told "signed in" by a click that did nothing. The password lives in a `Secret` for the
//!   length of this function and is wiped when it ends;
//! - `login_origin`, the real origin of the browser's front tab for the card to show (the bot's words are never shown as
//!   the site), and `forget_site`, the card's Sign out.
//!
//! The brain has no tool, event or message that reaches `submit_login`, `login_origin` or `forget_site`.

use super::*;
use crate::potato::cards::{self, Kind};
use crate::potato::login::{self, Request, Secret};
use crate::potato::sandbox::State as SandboxState;
use serde_json::json;

/// What the card shows about the site: the card's own site, and the origin the browser really has in front.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginOrigin {
    /// The card's site (normalized), as the bot asked for it.
    pub site: String,
    /// The origin of the front tab, read from the browser. `None`: no page could be read, or the computer is off.
    pub origin: Option<String>,
    /// The front tab is exactly the card's site.
    pub matches: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgetOutcome {
    pub cookies_cleared: u64,
}

const MAX_WHY: usize = 300;

fn text_arg(args: &Value, key: &str) -> String {
    args.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

impl Service {
    // ---- the brain's tool ------------------------------------------------------------------------------------------

    /// `request_login {site, why}`.
    pub(super) fn login_tool_item(&self, ctx: &sandbox_calls::Context, call_id: &str, args: &Value) -> Value {
        let refuse = |code: &'static str, message: &str| protocol::sandbox_result_err(call_id, code, message, false);
        let from_routine = self.inner.lock().bots.get(&ctx.bot).is_some_and(|rt| rt.turn_routine.contains(&ctx.conversation));
        let in_dm = ctx.conversation == store::app_conversation(&ctx.bot);
        if !in_dm || !ctx.trust.is_owner() || from_routine {
            return refuse("TOOL_NOT_ALLOWED", "A sign-in card is for your private chat with the owner, not a group, a routine or someone else.");
        }
        // Two arguments and no more. A key the tool does not know is refused whole, not skipped, and its value is not echoed.
        let only_known = args.as_object().is_some_and(|o| o.keys().all(|k| k == "site" || k == "why"));
        if !only_known {
            return refuse("LOGIN_NO_SECRETS", "request_login takes only the site and a short reason. Never put a password, code, cookie or user name in it: the owner types them in the card, and you never see them. Nothing was asked.");
        }
        let site = match login::normalize_site(&text_arg(args, "site")) {
            Ok(site) => site,
            Err(message) => return refuse("LOGIN_BAD_SITE", message),
        };
        let why: String = text_arg(args, "why").chars().filter(|c| !c.is_control() || *c == '\n').take(MAX_WHY).collect();
        let now = self.host.now_ms();
        let new = cards::New {
            kind: Kind::Question,
            action: format!("{} wants to sign in to {}", self.name_of(&ctx.bot), login::display_site(&site)),
            why,
            choices: login::choices(),
            conversation: ctx.conversation.clone(),
            command_hash: None,
            classes: Vec::new(),
            keep_going: None,
            connector: None,
            login: Some(site),
            mcp_add: None,
            speaker: self.name_of(&ctx.bot),
            home: self.home_of(&ctx.bot),
        };
        match cards::raise(&self.root, &ctx.bot, new, now) {
            Ok(raised) => {
                self.host.changed(&ctx.bot);
                protocol::sandbox_result_ok(
                    call_id,
                    json!({
                        "card": true,
                        "cardId": raised.card.id,
                        "note": "A sign-in card is waiting in the chat. Your turn ends now; the owner's answer comes as your next message. You never see the password. Do not claim you are signed in before that."
                    }),
                )
            }
            Err(e) => protocol::sandbox_result_err(call_id, e.code, &e.message, false),
        }
    }

    // ---- the card's buttons ----------------------------------------------------------------------------------------

    /// The open sign-in card of this bot, with its site.
    fn open_login_card(&self, bot: &str, card_id: &str) -> Result<(cards::Card, String)> {
        self.require_bot(bot)?;
        let card = cards::find(&self.root, bot, card_id).ok_or_else(|| err("POTATO_CARD_NONE", "There is no such card."))?;
        let site = card.login.clone().ok_or_else(|| err("POTATO_CARD_BAD_ANSWER", "That is not a sign-in card."))?;
        Ok((card, site))
    }

    /// The running computer, or `None`: asking about a page must never start a stopped one.
    fn running_port(&self) -> Option<sandbox_calls::Port> {
        if !self.sandbox_on.load(Ordering::SeqCst) {
            return None;
        }
        let port = self.port.lock().clone()?;
        port.sandbox.status().is_ok_and(|s| s.state == SandboxState::Running).then_some(port)
    }

    /// Sign in: fills the form in the browser and submits it, then answers the card and lets the job go on. The password
    /// is wrapped before anything else happens and is wiped when this returns. On any failure the card stays open.
    pub fn submit_login(&self, bot: &str, card_id: &str, username: String, password: String) -> Result<AnswerOutcome> {
        let password = Secret::new(password);
        let (card, site) = self.open_login_card(bot, card_id)?;
        match card.state {
            cards::State::Open => {}
            cards::State::Answered => return Err(err("POTATO_CARD_ANSWERED", "That card was already answered.")),
            cards::State::Cancelled => return Err(err("POTATO_CARD_CANCELLED", "That card was cancelled, so it cannot be answered.")),
        }
        let username = username.trim().to_string();
        let bad = |message: &str| err("LOGIN_BAD_INPUT", message);
        if username.is_empty() || password.is_empty() {
            return Err(bad("Type your user name and your password."));
        }
        if username.chars().count() > login::MAX_NAME_CHARS || username.chars().any(char::is_control) {
            return Err(bad("That user name is too long or has a character that cannot be typed."));
        }
        if password.char_count() > login::MAX_PASSWORD_CHARS || password.has_nul() {
            return Err(bad("That password is too long or has a character that cannot be typed."));
        }
        let Some(port) = self.running_port() else {
            return Err(err("LOGIN_NO_PAGE", "The bot's computer is not running, so there is no page to sign in on."));
        };
        let request = Request { origin: site, username, password };
        self.calls.login_fill(&port, bot, &card.conversation, request).map_err(|e| err(e.code, e.message))?;
        // The page tool already waited for the page to show the result: a box that is gone, a new address or a new page
        // is a sign-in, so a box still shown next to those is no reason to hold the card (a later step, or a page that
        // keeps one). A result it could not read came back as an error above and the card stays.
        let now = self.host.now_ms();
        let answered = cards::answer_signed_in(&self.root, bot, card_id, self.home_of(bot).as_deref(), now)?;
        Ok(self.settle_answer(bot, answered, now))
    }

    /// It worked, continue: the person looked at the page themselves. Answers the card as signed in (once) and lets the job or
    /// the turn go on. Nothing is filled and no password is involved.
    pub fn confirm_login(&self, bot: &str, card_id: &str) -> Result<AnswerOutcome> {
        let (card, _site) = self.open_login_card(bot, card_id)?;
        match card.state {
            cards::State::Open => {}
            cards::State::Answered => return Err(err("POTATO_CARD_ANSWERED", "That card was already answered.")),
            cards::State::Cancelled => return Err(err("POTATO_CARD_CANCELLED", "That card was cancelled, so it cannot be answered.")),
        }
        let now = self.host.now_ms();
        let answered = cards::answer_signed_in(&self.root, bot, card_id, self.home_of(bot).as_deref(), now)?;
        Ok(self.settle_answer(bot, answered, now))
    }

    /// The person gave the screen back. Every sign-in card still open for this bot (or, when it is not known whose, for
    /// every bot) is answered as Take over: they did it themselves, so the card must not wait and the bot goes on. Returns
    /// how many cards that closed.
    pub fn login_cards_after_hand_back(&self, bot: Option<&str>) -> usize {
        let bots: Vec<String> = match bot {
            Some(b) => vec![b.to_string()],
            None => store::load_bots(&self.root).unwrap_or_default().into_iter().map(|b| b.id).collect(),
        };
        let mut closed = 0;
        for id in bots {
            if self.require_bot(&id).is_err() {
                continue;
            }
            for card in cards::open_cards(&self.root, &id).into_iter().filter(|c| c.login.is_some()) {
                let now = self.host.now_ms();
                if let Ok(done) = cards::answer(&self.root, &id, &card.id, "1", self.home_of(&id).as_deref(), now) {
                    self.settle_answer(&id, done, now);
                    closed += 1;
                }
            }
        }
        closed
    }

    /// The real origin of the browser's front tab, for the card to show next to the site the bot named.
    pub fn login_origin(&self, bot: &str, card_id: &str) -> Result<LoginOrigin> {
        let (card, site) = self.open_login_card(bot, card_id)?;
        let origin = match (card.state, self.running_port()) {
            (cards::State::Open, Some(port)) => self.calls.front_origin(&port),
            _ => None,
        };
        let matches = origin.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(&site));
        Ok(LoginOrigin { site, origin, matches })
    }

    /// Sign out: the cookies and stored data of the card's site leave the browser.
    pub fn forget_site(&self, bot: &str, card_id: &str) -> Result<ForgetOutcome> {
        let (_card, site) = self.open_login_card(bot, card_id)?;
        let Some(port) = self.running_port() else {
            return Err(err("LOGIN_NO_PAGE", "The bot's computer is not running, so there is no browser to sign out of."));
        };
        let cookies_cleared = self.calls.forget_site(&port, &site).map_err(|e| err(e.code, e.message))?;
        Ok(ForgetOutcome { cookies_cleared })
    }
}
