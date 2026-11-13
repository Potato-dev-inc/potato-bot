//! Decision cards (Professional mode, P4): the bot asks, the person answers with a button. A card is an approval ("may I
//! run this?", three buttons) or a question (up to four choices). One file per bot, `cards.json` in the bot's state
//! folder, so a restart finds the open cards and their answers.
//!
//! The rules that make a card worth anything:
//!
//! - An answer comes only from `answer`, which the app command `potato_answer_card` calls. The brain has no way in: there
//!   is no brain tool, no event and no chat text that reaches it. A card id that does not exist, or belongs to another
//!   bot's file, is refused.
//! - "Always allow" (`answer_always`) is the one answer that is more than this command: it saves a rule for this bot and
//!   this exact command class (`bot_settings::always_allow`). It is offered only when the card names exactly one class
//!   and `risk::always_allowable` says so (never rm, force-push or anything destroying). The text path `answer` refuses
//!   it in every spelling; only the app command behind the button reaches `answer_always`.
//! - One answer only. A second answer, and an answer to a cancelled card, is a plain error and changes nothing.
//! - Stop and Pause all close every open card as cancelled (`cancel_all`) and forget what was allowed.
//! - `allow_once` lets exactly that command (its hash) run once. `allow_job` lets that class of command run for the rest
//!   of that job. `deny` is remembered for a while, so the same command is refused instead of asked again.
//! - A card raised during a running job puts the job to `waiting` and stops its clock. The answer flips the job back,
//!   with the decision written into the job record, in the same locked step that records the answer.
//!
//! Nothing here runs a model or starts a turn: `service/card_flow.rs` does that.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::jobs::{self, reason, Job, Status};
use super::store::{self, files_lock, DecisionCard, FromKind, Message, MsgKind, StoreError, StoreResult};
use crate::secure_file;

pub const FILE: &str = "cards.json";
pub const ASIDE_FILE: &str = "cards.broken.json";
/// Open cards per bot.
pub const MAX_OPEN: usize = 5;
/// Cards kept in the file; the oldest closed ones go first.
const KEPT: usize = 60;
/// A command the owner said no to is refused, not asked again, for this long.
pub const DENY_MEMORY_MS: i64 = 6 * 60 * 60 * 1000;
/// An "allow once" that was never used is forgotten after this long.
pub const ONCE_MEMORY_MS: i64 = 60 * 60 * 1000;
pub const MAX_ACTION_CHARS: usize = 300;
pub const MAX_WHY_CHARS: usize = 400;
pub const MAX_CHOICES: usize = 4;
pub const MAX_CHOICE_CHARS: usize = 60;
const MAX_ID_CHARS: usize = 40;

/// The two buttons of a Keep going card, in order.
pub const KEEP_GOING: &str = "Keep going";
pub const STOP: &str = "Stop";

const NONE: &str = "POTATO_CARD_NONE";
const ANSWERED: &str = "POTATO_CARD_ANSWERED";
const CANCELLED: &str = "POTATO_CARD_CANCELLED";
const BAD_ANSWER: &str = "POTATO_CARD_BAD_ANSWER";
const TOO_MANY: &str = "POTATO_CARD_TOO_MANY";
const INVALID: &str = "POTATO_CARD_INVALID";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Approval,
    Question,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum State {
    #[default]
    Open,
    Answered,
    Cancelled,
}

/// A card that asks the owner to add a tool server by link. Only the owner's button adds it (`service/card_flow.rs`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct McpAdd {
    pub name: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct Card {
    /// `card-` and twelve hex digits.
    pub id: String,
    pub kind: Kind,
    /// What the bot wants to do (an approval) or asks (a question).
    pub action: String,
    pub why: String,
    pub choices: Vec<String>,
    pub created_ms: i64,
    pub state: State,
    /// `allow_once`, `allow_job`, `deny` or `choice:N`, once answered.
    pub answer: Option<String>,
    pub answered_ms: Option<i64>,
    /// The conversation the card sits in: the owner's private chat with the bot.
    pub conversation: String,
    /// The job that was running when the card was raised.
    pub job: Option<String>,
    /// The hash of the command an approval is about (`command_hash`).
    pub command_hash: Option<String>,
    /// The risky-list classes the command touches, for "allow for this job".
    pub classes: Vec<String>,
    /// A Keep going card: why the job stopped (`jobs::reason`).
    pub keep_going: Option<String>,
    /// A connector card: the service the bot wants connected. The first choice opens Settings > Connectors.
    pub connector: Option<String>,
    /// A sign-in card: the https origin the bot asked to sign in to (`login.rs`). Its three buttons are Sign in, Take over
    /// and Not now; Sign in is answered only by `answer_signed_in`, after the host has filled the page.
    pub login: Option<String>,
    /// An add-a-tool-server card: the name and https link the bot suggested. The first button adds it, with no token.
    pub mcp_add: Option<McpAdd>,
    /// The approval may be answered "Always allow": it names exactly one command class and that class may be always
    /// allowed (`risk::always_allowable`). Never true for rm, force-push or a question.
    pub can_always_allow: bool,
}

impl Default for Card {
    fn default() -> Card {
        Card {
            id: String::new(),
            kind: Kind::Question,
            action: String::new(),
            why: String::new(),
            choices: Vec::new(),
            created_ms: 0,
            state: State::Open,
            answer: None,
            answered_ms: None,
            conversation: String::new(),
            job: None,
            command_hash: None,
            classes: Vec::new(),
            keep_going: None,
            connector: None,
            login: None,
            mcp_add: None,
            can_always_allow: false,
        }
    }
}

/// Whether "Always allow" may be offered, and accepted, for this card. Read from the card itself, so a card file edited by
/// hand cannot widen it past the class list.
fn always_ok(card: &Card) -> bool {
    card.kind == Kind::Approval
        && card.keep_going.is_none()
        && card.connector.is_none()
        && card.login.is_none()
        && card.command_hash.is_some()
        && card.classes.len() == 1
        && super::risk::always_allowable(&card.classes[0])
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct Once {
    hash: String,
    card: String,
    at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct JobGrant {
    job: String,
    class: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct Denied {
    hash: String,
    at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct File {
    pub cards: Vec<Card>,
    once: Vec<Once>,
    job_grants: Vec<JobGrant>,
    denied: Vec<Denied>,
}

/// An answer, as it was understood.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    AllowOnce,
    AllowJob,
    /// Allow once, and always for this bot and this class. Never parsed from text: only `answer_always` makes it.
    AlwaysAllow,
    Deny,
    Choice(usize),
}

impl Answer {
    /// The words the UI sends (`allow_once`, `allow_job`, `deny`, or a choice number from 0), checked against the card.
    pub fn parse(text: &str, card: &Card) -> StoreResult<Answer> {
        let bad = || StoreError::new(BAD_ANSWER, "That is not one of this card's answers.");
        let text = text.trim();
        match card.kind {
            Kind::Approval => match text {
                "allow_once" => Ok(Answer::AllowOnce),
                "allow_job" => Ok(Answer::AllowJob),
                "deny" => Ok(Answer::Deny),
                _ => Err(bad()),
            },
            Kind::Question => {
                let n: usize = text.parse().map_err(|_| bad())?;
                if n < card.choices.len() {
                    Ok(Answer::Choice(n))
                } else {
                    Err(bad())
                }
            }
        }
    }

    /// The word kept in the file, read back (a card answered before a crash).
    pub fn from_wire(text: &str) -> Option<Answer> {
        match text {
            "allow_once" => Some(Answer::AllowOnce),
            "allow_job" => Some(Answer::AllowJob),
            "always_allow" => Some(Answer::AlwaysAllow),
            "deny" => Some(Answer::Deny),
            other => other.strip_prefix("choice:").and_then(|n| n.parse().ok()).map(Answer::Choice),
        }
    }

    /// The word kept in the file and sent to the UI.
    pub fn wire(self) -> String {
        match self {
            Answer::AllowOnce => "allow_once".into(),
            Answer::AllowJob => "allow_job".into(),
            Answer::AlwaysAllow => "always_allow".into(),
            Answer::Deny => "deny".into(),
            Answer::Choice(n) => format!("choice:{n}"),
        }
    }
}

/// What a new card is made of.
#[derive(Clone, Debug)]
pub struct New {
    pub kind: Kind,
    pub action: String,
    pub why: String,
    pub choices: Vec<String>,
    pub conversation: String,
    pub command_hash: Option<String>,
    pub classes: Vec<String>,
    pub keep_going: Option<String>,
    pub connector: Option<String>,
    pub login: Option<String>,
    /// Set for an add-a-tool-server card (`add_mcp_server` with a link).
    pub mcp_add: Option<McpAdd>,
    /// The bot's name, for the thread row.
    pub speaker: String,
    /// The bot's home folder, taken out of any text that is kept.
    pub home: Option<String>,
}

pub struct Raised {
    pub card: Card,
}

/// What the answer did to the bot's job.
#[derive(Debug, PartialEq)]
pub enum JobEffect {
    /// The card had no job waiting for it.
    None,
    /// The job is running again, with the decision in its record. The caller starts its next turn.
    Resumed,
    /// A Keep going card was answered Stop: the job is stopped (the caller says so in the thread).
    Stopped(Job),
}

#[derive(Debug)]
pub struct Answered {
    pub card: Card,
    pub answer: Answer,
    /// One plain sentence for the bot: what the owner decided.
    pub decision: String,
    pub job: JobEffect,
}

fn path(root: &std::path::Path, bot: &str) -> StoreResult<PathBuf> {
    Ok(store::state_dir(root, bot)?.join(FILE))
}

fn aside_path(root: &std::path::Path, bot: &str) -> StoreResult<PathBuf> {
    Ok(store::state_dir(root, bot)?.join(ASIDE_FILE))
}

/// The bot's cards and grants. A file that cannot be read is moved aside and reads as empty: no open card, nothing
/// allowed. That is the safe direction.
pub fn load(root: &std::path::Path, bot: &str) -> File {
    let _files = files_lock();
    let Ok(file) = path(root, bot) else { return File::default() };
    let text = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return File::default(),
        Err(e) => {
            tracing::warn!(%bot, %e, "could not read the cards file");
            put_aside(root, bot);
            return File::default();
        }
    };
    match serde_json::from_str::<File>(&text) {
        Ok(parsed) => parsed,
        Err(_) => {
            tracing::warn!(%bot, "the cards file is damaged; it was kept aside");
            put_aside(root, bot);
            File::default()
        }
    }
}

fn put_aside(root: &std::path::Path, bot: &str) {
    if let (Ok(from), Ok(to)) = (path(root, bot), aside_path(root, bot)) {
        let _ = fs::remove_file(&to);
        if fs::rename(&from, &to).is_err() {
            let _ = fs::remove_file(&from);
        }
    }
}

fn save(root: &std::path::Path, bot: &str, file: &mut File) -> StoreResult<()> {
    let _files = files_lock();
    // The oldest closed cards go first when the file grows.
    while file.cards.len() > KEPT {
        match file.cards.iter().position(|c| c.state != State::Open) {
            Some(at) => {
                file.cards.remove(at);
            }
            None => break,
        }
    }
    let target = path(root, bot)?;
    let text = serde_json::to_string_pretty(file).map_err(|e| StoreError::new(INVALID, format!("Could not save the card: {e}.")))?;
    secure_file::write_private(&target, &text).map_err(|e| StoreError::new(INVALID, e))
}

fn one_line(raw: &str, max: usize) -> String {
    let line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    store::clean_text(&line, None).chars().take(max).collect()
}

fn new_id() -> String {
    format!("card-{}", &uuid::Uuid::new_v4().simple().to_string()[..12])
}

/// The number a command is known by: its text and the folder it runs in. Exactly that command, nothing near it.
pub fn command_hash(command: &str, cwd: Option<&str>) -> String {
    let mut sha = Sha256::new();
    sha.update(b"command\0");
    sha.update(cwd.unwrap_or("").as_bytes());
    sha.update(b"\0");
    sha.update(command.trim().as_bytes());
    sha.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn post(root: &std::path::Path, home: Option<&str>, conversation: &str, message: Message) {
    if let Err(e) = store::append_message(root, conversation, message, home) {
        tracing::debug!(error = %e.message, "could not save a card row");
    }
}

// ---- raising a card ---------------------------------------------------------------------------------------------------

/// Makes a card, writes its row in the thread, and puts a running job to `waiting` (the job's clock stops). An open card
/// for the same command is returned instead of a second one.
pub fn raise(root: &std::path::Path, bot: &str, new: New, now: i64) -> StoreResult<Raised> {
    let _files = files_lock();
    store::require_bot(root, bot)?;
    let mut file = load(root, bot);
    if let Some(hash) = &new.command_hash {
        if let Some(open) = file.cards.iter().find(|c| c.state == State::Open && c.command_hash.as_ref() == Some(hash)) {
            return Ok(Raised { card: open.clone() });
        }
    }
    if file.cards.iter().filter(|c| c.state == State::Open).count() >= MAX_OPEN {
        return Err(StoreError::new(TOO_MANY, "There are already too many open cards. Answer or stop them first."));
    }
    let action = one_line(&new.action, MAX_ACTION_CHARS);
    if action.is_empty() {
        return Err(StoreError::new(INVALID, "Say what you want to do first."));
    }
    let mut choices: Vec<String> = new.choices.iter().map(|c| one_line(c, MAX_CHOICE_CHARS)).filter(|c| !c.is_empty()).take(MAX_CHOICES).collect();
    if new.kind == Kind::Approval {
        choices.clear();
    } else if choices.is_empty() {
        return Err(StoreError::new(INVALID, "A question card needs at least one choice."));
    }
    let job = jobs::load(root, bot).filter(|j| j.status == Status::Running);
    let mut card = Card {
        id: new_id(),
        kind: new.kind,
        action,
        why: one_line(&new.why, MAX_WHY_CHARS),
        choices,
        created_ms: now,
        conversation: new.conversation.clone(),
        job: job.as_ref().map(|j| j.id.clone()),
        command_hash: new.command_hash.clone(),
        classes: new.classes.clone(),
        keep_going: new.keep_going.clone(),
        connector: new.connector.clone(),
        login: new.login.clone(),
        mcp_add: new.mcp_add.clone(),
        ..Card::default()
    };
    card.can_always_allow = always_ok(&card);
    file.cards.push(card.clone());
    save(root, bot, &mut file)?;
    // The job waits for the card; its clock stops until the answer.
    if let Some(mut job) = job {
        job.close_clock(now);
        job.status = Status::Waiting;
        job.waiting = Some(card.id.clone());
        job.updated_ms = now;
        if let Err(e) = jobs::save(root, bot, &job) {
            tracing::warn!(%bot, error = %e.message, "could not put the job to waiting");
        }
    }
    let mut row = Message::new(now, FromKind::Bot, Some(&new.speaker), MsgKind::DecisionCard, &card.action);
    row.request_id = Some(card.id.clone());
    row.card = Some(DecisionCard { kind: card.kind, why: card.why.clone(), choices: card.choices.clone(), keep_going: card.keep_going.is_some(), connector: card.connector.clone(), login: card.login.clone(), can_always_allow: card.can_always_allow, answer: None });
    post(root, new.home.as_deref(), &card.conversation, row);
    Ok(Raised { card })
}

// ---- answering --------------------------------------------------------------------------------------------------------

fn decision_text(card: &Card, answer: Answer) -> String {
    match (answer, card.kind) {
        (Answer::AllowOnce, _) => format!("Your owner chose Allow once for: {}. Run exactly that command now; it is allowed one time only.", card.action),
        (Answer::AllowJob, _) => format!(
            "Your owner chose Allow for this job for: {}. Commands of the kind \"{}\" are allowed for the rest of this job.",
            card.action,
            card.classes.join(", ")
        ),
        (Answer::AlwaysAllow, _) => format!(
            "Your owner chose Always allow for: {}. Run exactly that command now. From now on, commands of the kind \"{}\" run without asking you again; each one is noted in the chat.",
            card.action,
            card.classes.join(", ")
        ),
        (Answer::Deny, _) => format!(
            "Your owner chose Deny for: {}. Do not run it and do not look for another way to do the same thing. Do something else, or tell your owner what you could not do.",
            card.action
        ),
        (Answer::Choice(n), _) if card.login.is_some() => login_decision(card, n),
        (Answer::Choice(n), _) => {
            let choice = card.choices.get(n).map_or("", String::as_str);
            if matches!(card.keep_going.as_deref(), Some(reason::DAILY_LIMIT | reason::ACTION_LIMIT)) {
                format!("Your owner chose \"{choice}\".{}", if n == 0 { " Go on with the goal." } else { "" })
            } else if card.keep_going.is_some() {
                if n == 0 {
                    format!("Your owner chose \"{choice}\": the job has more time and money for another round. Go on with the goal.")
                } else {
                    format!("Your owner chose \"{choice}\".")
                }
            } else {
                format!("Your owner answered your question \"{}\": {}.", card.action, choice)
            }
        }
    }
}

/// What the bot is told about a sign-in card. The site is the card's own normalized origin, never a page's words, and
/// nothing here is, or could hold, the password.
fn login_decision(card: &Card, n: usize) -> String {
    let site = card.login.as_deref().map_or("", super::login::display_site);
    match n {
        0 => format!(
            "Your owner signed in to {site} through the card and the sign-in form was submitted. You never see the password. Look at the page with browser_snapshot to see whether it worked. While you are signed in you can act as your owner on this site, and what needs asking still follows When to ask."
        ),
        1 => format!(
            "Your owner chose Take over: they are signing in to {site} themselves on the screen (a code or a check). Your computer calls will say the person is using the screen until they hand it back. Do not ask for the password. When you can use the computer again, look at the page."
        ),
        _ => format!(
            "Your owner chose Not now for signing in to {site}. Do not ask again and do not look for another way in. Do something else, or tell your owner what you could not do."
        ),
    }
}

fn result_line(card: &Card, answer: Answer) -> String {
    if let (Some(site), Answer::Choice(n)) = (&card.login, answer) {
        let site = super::login::display_site(site);
        return match n {
            0 => format!("You signed in to {site}."),
            1 => format!("You took over to sign in to {site} yourself."),
            _ => "You chose: Not now.".to_string(),
        };
    }
    match answer {
        Answer::AllowOnce => "You allowed it once.".to_string(),
        Answer::AllowJob => "You allowed this kind of command for this job.".to_string(),
        Answer::AlwaysAllow => format!("You chose Always allow for this kind of command: one that {}.", card.classes.first().map_or_else(String::new, |c| super::risk::class_description(c))),
        Answer::Deny => "You said no.".to_string(),
        Answer::Choice(n) => format!("You chose: {}.", card.choices.get(n).map_or("", String::as_str)),
    }
}

/// Records the person's answer. The only way a card is answered. `home`: the bot's home folder, for the thread row.
/// A sign-in card's Sign in is not an answer this takes: only `answer_signed_in` records it.
pub fn answer(root: &std::path::Path, bot: &str, card_id: &str, text: &str, home: Option<&str>, now: i64) -> StoreResult<Answered> {
    answer_with(root, bot, card_id, text, home, now, false, false)
}

/// Always allow, answered by the app command behind the button. The one way a rule is made: this card's command runs now,
/// and this bot's later commands of the card's one class run without a card. Refused unless the card carries exactly one
/// class that may be always allowed (never rm or force-push).
pub fn answer_always(root: &std::path::Path, bot: &str, card_id: &str, home: Option<&str>, now: i64) -> StoreResult<Answered> {
    answer_with(root, bot, card_id, "always_allow", home, now, false, true)
}

/// Sign in, answered after the host filled the page and submitted it. The one way a sign-in card gets that answer.
pub fn answer_signed_in(root: &std::path::Path, bot: &str, card_id: &str, home: Option<&str>, now: i64) -> StoreResult<Answered> {
    answer_with(root, bot, card_id, "0", home, now, true, false)
}

#[allow(clippy::too_many_arguments)]
fn answer_with(root: &std::path::Path, bot: &str, card_id: &str, text: &str, home: Option<&str>, now: i64, signed_in: bool, always: bool) -> StoreResult<Answered> {
    let _files = files_lock();
    store::require_bot(root, bot)?;
    if card_id.is_empty() || card_id.chars().count() > MAX_ID_CHARS {
        return Err(StoreError::new(NONE, "There is no such card."));
    }
    let mut file = load(root, bot);
    let at = file.cards.iter().position(|c| c.id == card_id).ok_or_else(|| StoreError::new(NONE, "There is no such card."))?;
    let card = file.cards[at].clone();
    match card.state {
        State::Open => {}
        State::Answered => return Err(StoreError::new(ANSWERED, "That card was already answered.")),
        State::Cancelled => return Err(StoreError::new(CANCELLED, "That card was cancelled, so it cannot be answered.")),
    }
    let answer = if always {
        if !always_ok(&card) {
            return Err(StoreError::new(BAD_ANSWER, "Always allow is not offered for this kind of command. Nothing was answered."));
        }
        Answer::AlwaysAllow
    } else {
        Answer::parse(text, &card)?
    };
    if card.login.is_some() && answer == Answer::Choice(0) && !signed_in {
        return Err(StoreError::new(BAD_ANSWER, "Sign in works only from the Sign in button, which fills the page. Nothing was answered."));
    }
    if signed_in && (card.login.is_none() || answer != Answer::Choice(0)) {
        return Err(StoreError::new(BAD_ANSWER, "That card is not a sign-in card."));
    }
    let decision = decision_text(&card, answer);
    {
        let stored = &mut file.cards[at];
        stored.state = State::Answered;
        stored.answer = Some(answer.wire());
        stored.answered_ms = Some(now);
    }
    // What the answer allows.
    match answer {
        Answer::AllowOnce => {
            if let Some(hash) = &card.command_hash {
                file.once.push(Once { hash: hash.clone(), card: card.id.clone(), at: now });
            }
        }
        Answer::AllowJob => match (&card.job, &card.command_hash) {
            (Some(job), _) => {
                for class in &card.classes {
                    if !file.job_grants.iter().any(|g| &g.job == job && &g.class == class) {
                        file.job_grants.push(JobGrant { job: job.clone(), class: class.clone() });
                    }
                }
                // A job grant covers the class, the card's own command included; it needs no separate "once".
            }
            // No job to scope it to: it is an "allow once".
            (None, Some(hash)) => file.once.push(Once { hash: hash.clone(), card: card.id.clone(), at: now }),
            (None, None) => {}
        },
        Answer::AlwaysAllow => {
            if let Some(hash) = &card.command_hash {
                file.once.push(Once { hash: hash.clone(), card: card.id.clone(), at: now });
            }
        }
        Answer::Deny => {
            if let Some(hash) = &card.command_hash {
                file.denied.retain(|d| &d.hash != hash);
                file.denied.push(Denied { hash: hash.clone(), at: now });
            }
        }
        Answer::Choice(_) => {}
    }
    let rule_class = if answer == Answer::AlwaysAllow { card.classes.first().cloned() } else { None };
    if let Some(class) = &rule_class {
        super::bot_settings::update(root, bot, |settings| {
            if !settings.always_allow.iter().any(|r| &r.class == class) {
                settings.always_allow.push(super::bot_settings::AlwaysRule { class: class.clone(), at: now });
            }
            Ok(())
        })?;
    }
    if let Err(e) = save(root, bot, &mut file) {
        // The answer was not saved, so the rule must not stay.
        if let Some(class) = &rule_class {
            let _ = super::bot_settings::update(root, bot, |settings| {
                settings.always_allow.retain(|r| &r.class != class);
                Ok(())
            });
        }
        return Err(e);
    }

    // The job that waited for this card.
    let job = settle_job(root, bot, &card, answer, &decision, now)?;

    let mut row = Message::new(now, FromKind::Note, None, MsgKind::DecisionResult, &result_line(&card, answer));
    row.request_id = Some(card.id.clone());
    row.card = Some(DecisionCard { kind: card.kind, why: String::new(), choices: Vec::new(), keep_going: card.keep_going.is_some(), connector: card.connector.clone(), login: card.login.clone(), can_always_allow: false, answer: Some(answer.wire()) });
    post(root, home, &card.conversation, row);
    let mut closed = card;
    closed.state = State::Answered;
    closed.answer = Some(answer.wire());
    Ok(Answered { card: closed, answer, decision, job })
}

/// The job that waited for this card goes on (or stops, for a Keep going card answered Stop). Does nothing when the job
/// is not waiting for exactly this card. Used by the answer, and by the recovery of an answer that a crash cut short.
fn settle_job(root: &std::path::Path, bot: &str, card: &Card, answer: Answer, decision: &str, now: i64) -> StoreResult<JobEffect> {
    Ok(match jobs::load(root, bot) {
        Some(mut job) if job.status == Status::Waiting && job.waiting.as_deref() == Some(card.id.as_str()) => {
            job.waiting = None;
            job.updated_ms = now;
            let stop = card.keep_going.as_deref().filter(|_| answer == Answer::Choice(1));
            if let Some(why) = stop {
                job.status = Status::Stopped;
                job.stop_reason = Some(why.to_string());
                let _ = jobs::save(root, bot, &job);
                JobEffect::Stopped(job)
            } else {
                if answer == Answer::Choice(0) {
                    match card.keep_going.as_deref() {
                        Some(reason::TIME_LIMIT) => job.extra_ms = job.extra_ms.saturating_add(jobs::MAX_JOB_MS),
                        Some(reason::MONEY_LIMIT) => job.extra_nano = job.extra_nano.saturating_add(jobs::MAX_JOB_NANO),
                        Some(reason::NO_PROGRESS) => job.no_progress = 0,
                        _ => {}
                    }
                }
                job.status = Status::Running;
                job.decision = Some(decision.to_string());
                jobs::save(root, bot, &job)?;
                JobEffect::Resumed
            }
        }
        _ => JobEffect::None,
    })
}

/// The crash gap: the card's answer was saved, but the app went down before the job was told. A job still waiting for a
/// card that is answered (not cancelled) goes on now, exactly as the answer would have made it. Once it runs the job no
/// longer waits for that card, so this finds nothing the second time. `None`: nothing to recover.
pub fn recover_waiting(root: &std::path::Path, bot: &str, now: i64) -> Option<JobEffect> {
    let _files = files_lock();
    let job = jobs::load(root, bot).filter(|j| j.status == Status::Waiting)?;
    let waiting = job.waiting.clone()?;
    let card = load(root, bot).cards.into_iter().find(|c| c.id == waiting && c.state == State::Answered)?;
    let answer = Answer::from_wire(card.answer.as_deref()?)?;
    let decision = decision_text(&card, answer);
    match settle_job(root, bot, &card, answer, &decision, now) {
        Ok(JobEffect::None) => None,
        Ok(effect) => Some(effect),
        Err(e) => {
            tracing::warn!(%bot, error = %e.message, "could not recover the job that waited for an answered card");
            None
        }
    }
}

/// Closes every open card as cancelled and forgets everything that was allowed (Stop, Pause all). Returns how many.
pub fn cancel_all(root: &std::path::Path, bot: &str, home: Option<&str>, now: i64) -> usize {
    let _files = files_lock();
    let mut file = load(root, bot);
    let mut closed = Vec::new();
    for card in file.cards.iter_mut().filter(|c| c.state == State::Open) {
        card.state = State::Cancelled;
        card.answer = Some("cancelled".to_string());
        card.answered_ms = Some(now);
        closed.push(card.clone());
    }
    let had_grants = !file.once.is_empty() || !file.job_grants.is_empty();
    if closed.is_empty() && !had_grants {
        return 0;
    }
    file.once.clear();
    file.job_grants.clear();
    if let Err(e) = save(root, bot, &mut file) {
        tracing::warn!(%bot, error = %e.message, "could not cancel the cards");
        return 0;
    }
    for card in &closed {
        let mut row = Message::new(now, FromKind::Note, None, MsgKind::DecisionResult, "Cancelled.");
        row.request_id = Some(card.id.clone());
        row.card = Some(DecisionCard { kind: card.kind, why: String::new(), choices: Vec::new(), keep_going: card.keep_going.is_some(), connector: card.connector.clone(), login: card.login.clone(), can_always_allow: false, answer: Some("cancelled".to_string()) });
        post(root, home, &card.conversation, row);
    }
    closed.len()
}

/// The plain line in the thread when a command ran because of an Always allow rule: what ran and why it did not ask.
pub fn note_auto_allowed(root: &std::path::Path, home: Option<&str>, conversation: &str, command: &str, class: &str, now: i64) {
    let text = format!("Auto-allowed: {} (your Always allow rule for a command that {}).", one_line(command, MAX_ACTION_CHARS), super::risk::class_description(class));
    post(root, home, conversation, Message::new(now, FromKind::Note, None, MsgKind::Note, &text));
}

// ---- what the gate asks ------------------------------------------------------------------------------------------------

/// Uses up the "allow once" for exactly this command, if there is one that has not run out.
pub fn consume_once(root: &std::path::Path, bot: &str, hash: &str, now: i64) -> bool {
    let _files = files_lock();
    let mut file = load(root, bot);
    file.once.retain(|o| now - o.at <= ONCE_MEMORY_MS);
    let Some(at) = file.once.iter().position(|o| o.hash == hash) else { return false };
    file.once.remove(at);
    save(root, bot, &mut file).is_ok()
}

/// Is every class of the command allowed for this job?
pub fn job_allows(root: &std::path::Path, bot: &str, job: &str, classes: &[&str]) -> bool {
    if classes.is_empty() {
        return false;
    }
    let file = load(root, bot);
    classes.iter().all(|class| file.job_grants.iter().any(|g| g.job == job && g.class == *class))
}

/// Did the owner say no to exactly this command, not long ago?
pub fn is_denied(root: &std::path::Path, bot: &str, hash: &str, now: i64) -> bool {
    load(root, bot).denied.iter().any(|d| d.hash == hash && now - d.at <= DENY_MEMORY_MS)
}

/// The cards still waiting for an answer, oldest first.
pub fn open_cards(root: &std::path::Path, bot: &str) -> Vec<Card> {
    load(root, bot).cards.into_iter().filter(|c| c.state == State::Open).collect()
}

/// One card by id.
#[cfg_attr(not(test), allow(dead_code))]
pub fn find(root: &std::path::Path, bot: &str, card_id: &str) -> Option<Card> {
    load(root, bot).cards.into_iter().find(|c| c.id == card_id)
}

/// Is there an "allow once" for exactly this command that has not run out? Does not use it up.
pub fn has_once(root: &std::path::Path, bot: &str, hash: &str, now: i64) -> bool {
    load(root, bot).once.iter().any(|o| o.hash == hash && now - o.at <= ONCE_MEMORY_MS)
}
