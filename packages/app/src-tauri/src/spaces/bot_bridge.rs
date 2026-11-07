//! Bots in a space, this PC's half. A bot runs only on its owner's PC; every other PC just shows its `BotText`.
//!
//! This file decides which bots of mine get a text line, keeps the caps and the "already handled" list in the
//! space's own store, and cleans a bot's answer before it is posted. It knows nothing of how a line reaches a bot:
//! it talks to a `BotSink`. It holds no path and opens no file.
//!
//! Who counts as the owner comes from the signature only: `MessageRow.member` is the author's member key as the
//! log check proved it, and it must equal both the bot's owner on the roster and this PC's own member key. A
//! display name or "I am the owner" in the text counts for nothing.
use rusqlite::{params, OptionalExtension};

use super::ids::{to_hex, BotId, ChannelId, EntryId, MemberKey, SpaceId};
use super::leak::{leak_reason, PrivateMarks};
use super::limits::{BOT_DAY_CAP, BOT_PERSON_HOUR_CAP, BOT_UNPROMPTED_CAP, BOT_UNPROMPTED_WINDOW_MS, MAX_TEXT_CHARS};
use super::log::{clean_text, KIND_BOT, KIND_TEXT, MAX_TEXT_BYTES};
use super::roster::Roster;
use super::store::{MessageRow, SpaceStore};

/// A line older than this (by the time its author put on it) is history from a sync, not a question: no answer.
pub const MAX_AGE_MS: u64 = 5 * 60 * 1000;
/// Lines of earlier talk a guest turn gets.
pub const RECENT_MAX: usize = 20;
/// Longest earlier line handed over, in characters.
pub const RECENT_LINE_CHARS: usize = 600;
/// The most an owner can raise a bot's day cap to.
pub const DAY_CAP_MAX: u32 = 1000;
/// "Helps" mode answers a question of at least this many characters.
pub const HELPS_MIN_CHARS: usize = 12;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;
const HOUR_MS: u64 = 60 * 60 * 1000;
/// A delivery nobody answered is forgotten after this long.
pub const PENDING_MS: u64 = 30 * 60 * 1000;

// ---- what this PC decides about one of its bots in one space ------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Only when named (`@name`) or when someone replies to its message.
    Called,
    /// Also answers a question nobody named it for.
    Helps,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self { Mode::Called => "called", Mode::Helps => "helps" }
    }
    pub fn parse(s: &str) -> Option<Mode> {
        match s { "called" => Some(Mode::Called), "helps" => Some(Mode::Helps), _ => None }
    }
}

/// One row of `bot_local`. `potato` is this PC's own bot id; it is set only for a bot of mine. For someone
/// else's bot the row exists only to hold `muted` (the viewer hides it on this PC).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BotLocal {
    pub bot: BotId,
    pub potato: Option<String>,
    pub mode: Mode,
    /// Memory on: the bot reads the space diary and its talk is journaled. Off: no diary at all.
    pub memory: bool,
    pub day_cap: u32,
    /// The bot's owner stopped it (on this PC).
    pub silenced: bool,
    /// The viewer hides it on this PC.
    pub muted: bool,
    /// When (and under what name) this PC asked the space owner to add the bot; 0 when it was never asked.
    /// The claim is pending while the roster does not show the bot.
    pub claimed_ms: u64,
    pub claim_name: String,
}

impl BotLocal {
    pub fn new(bot: BotId, potato: Option<String>) -> Self {
        Self { bot, potato, mode: Mode::Called, memory: true, day_cap: BOT_DAY_CAP, silenced: false, muted: false, claimed_ms: 0, claim_name: String::new() }
    }
}

pub fn clamp_day_cap(n: u32) -> u32 { n.clamp(1, DAY_CAP_MAX) }

fn bot_key(b: &BotId) -> Vec<u8> { b.0.to_vec() }

pub fn local_get(store: &SpaceStore, bot: &BotId) -> rusqlite::Result<Option<BotLocal>> {
    store
        .conn
        .query_row(
            "SELECT bot, potato, mode, memory, day_cap, silenced, muted, claimed_ms, claim_name FROM bot_local WHERE bot = ?1",
            params![bot_key(bot)],
            row_local,
        )
        .optional()?
        .transpose()
}

pub fn local_list(store: &SpaceStore) -> rusqlite::Result<Vec<BotLocal>> {
    let mut st = store.conn.prepare("SELECT bot, potato, mode, memory, day_cap, silenced, muted, claimed_ms, claim_name FROM bot_local ORDER BY rowid")?;
    let rows = st.query_map([], row_local)?;
    let mut out = Vec::new();
    for r in rows { if let Ok(l) = r? { out.push(l); } }
    Ok(out)
}

fn row_local(r: &rusqlite::Row<'_>) -> rusqlite::Result<rusqlite::Result<BotLocal>> {
    let id: Vec<u8> = r.get(0)?;
    let mode: String = r.get(2)?;
    let built = (|| {
        let bot = <[u8; 8]>::try_from(id).map(BotId).map_err(|_| rusqlite::Error::InvalidQuery)?;
        Ok(BotLocal {
            bot,
            potato: r.get(1)?,
            mode: Mode::parse(&mode).unwrap_or(Mode::Called),
            memory: r.get::<_, i64>(3)? != 0,
            day_cap: clamp_day_cap(r.get::<_, i64>(4)?.clamp(0, i64::from(u32::MAX)) as u32),
            silenced: r.get::<_, i64>(5)? != 0,
            muted: r.get::<_, i64>(6)? != 0,
            claimed_ms: r.get::<_, i64>(7)?.max(0) as u64,
            claim_name: r.get(8)?,
        })
    })();
    Ok(built)
}

pub fn local_put(store: &SpaceStore, l: &BotLocal) -> rusqlite::Result<()> {
    store
        .conn
        .execute(
            "INSERT OR REPLACE INTO bot_local (bot, potato, mode, memory, day_cap, silenced, muted, claimed_ms, claim_name) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![bot_key(&l.bot), l.potato, l.mode.as_str(), l.memory as i64, clamp_day_cap(l.day_cap) as i64, l.silenced as i64, l.muted as i64, l.claimed_ms as i64, l.claim_name],
        )
        .map(|_| ())
}

pub fn local_delete(store: &SpaceStore, bot: &BotId) -> rusqlite::Result<()> {
    store.conn.execute("DELETE FROM bot_local WHERE bot = ?1", params![bot_key(bot)]).map(|_| ())
}

// ---- caps, handled lines, the held-back counter (all persisted, so a restart changes nothing) ---------------------

fn count(store: &SpaceStore, sql: &str, p: impl rusqlite::Params) -> u32 {
    store.conn.query_row(sql, p, |r| r.get::<_, i64>(0)).map(|n| n.max(0) as u32).unwrap_or(0)
}

/// Answers given in the last 24 hours (a rolling day).
pub fn used_in_day(store: &SpaceStore, bot: &BotId, now: u64) -> u32 {
    count(store, "SELECT COUNT(*) FROM bot_usage WHERE bot = ?1 AND at_ms > ?2", params![bot_key(bot), now.saturating_sub(DAY_MS) as i64])
}

fn used_by_person(store: &SpaceStore, bot: &BotId, person: &MemberKey, now: u64) -> u32 {
    count(
        store,
        "SELECT COUNT(*) FROM bot_usage WHERE bot = ?1 AND person = ?2 AND at_ms > ?3",
        params![bot_key(bot), person.0.to_vec(), now.saturating_sub(HOUR_MS) as i64],
    )
}

fn used_unprompted(store: &SpaceStore, bot: &BotId, now: u64) -> u32 {
    count(
        store,
        "SELECT COUNT(*) FROM bot_usage WHERE bot = ?1 AND kind = 'unprompted' AND at_ms > ?2",
        params![bot_key(bot), now.saturating_sub(BOT_UNPROMPTED_WINDOW_MS) as i64],
    )
}

fn usage_add(store: &SpaceStore, bot: &BotId, person: Option<&MemberKey>, unprompted: bool, now: u64) -> rusqlite::Result<i64> {
    store.conn.execute(
        "INSERT INTO bot_usage (at_ms, bot, person, kind) VALUES (?1, ?2, ?3, ?4)",
        params![now as i64, bot_key(bot), person.map(|p| p.0.to_vec()), if unprompted { "unprompted" } else { "called" }],
    )?;
    Ok(store.conn.last_insert_rowid())
}

/// Gives back a hold on the caps (the bot was not able to take the line after all).
pub fn usage_release(store: &SpaceStore, rowid: i64) {
    let _ = store.conn.execute("DELETE FROM bot_usage WHERE rowid = ?1", params![rowid]);
}

fn handled_has(store: &SpaceStore, entry: &EntryId, bot: &BotId) -> bool {
    count(store, "SELECT COUNT(*) FROM bot_handled WHERE entry = ?1 AND bot = ?2", params![entry.0.to_vec(), bot_key(bot)]) > 0
}

fn handled_mark(store: &SpaceStore, entry: &EntryId, bot: &BotId, now: u64) -> rusqlite::Result<()> {
    store
        .conn
        .execute("INSERT OR IGNORE INTO bot_handled (entry, bot, at_ms) VALUES (?1, ?2, ?3)", params![entry.0.to_vec(), bot_key(bot), now as i64])
        .map(|_| ())
}

fn held_key(bot: &BotId) -> String { format!("bot_held:{}", bot.to_hex()) }

/// Counts one reply that was held back (day number and count, 8 bytes). Nothing about the text is kept.
pub fn held_bump(store: &SpaceStore, bot: &BotId, now: u64) -> rusqlite::Result<()> {
    let day = (now / DAY_MS) as u32;
    let n = held_today(store, bot, now) + 1;
    let mut b = day.to_be_bytes().to_vec();
    b.extend_from_slice(&n.to_be_bytes());
    store.meta_set(&held_key(bot), &b)
}

/// Replies held back today (the day counted by the clock this PC uses for caps: a UTC day number).
pub fn held_today(store: &SpaceStore, bot: &BotId, now: u64) -> u32 {
    let Ok(Some(b)) = store.meta_get(&held_key(bot)) else { return 0 };
    let Ok(b) = <[u8; 8]>::try_from(b) else { return 0 };
    let (day, n) = (u32::from_be_bytes([b[0], b[1], b[2], b[3]]), u32::from_be_bytes([b[4], b[5], b[6], b[7]]));
    if day == (now / DAY_MS) as u32 { n } else { 0 }
}

/// Forgets what can no longer matter: handled lines past the age limit, usage past two days.
pub fn prune(store: &SpaceStore, now: u64) {
    let _ = store.conn.execute("DELETE FROM bot_handled WHERE at_ms < ?1", params![now.saturating_sub(HOUR_MS) as i64]);
    let _ = store.conn.execute("DELETE FROM bot_usage WHERE at_ms < ?1", params![now.saturating_sub(2 * DAY_MS) as i64]);
}

// ---- the port to whatever runs the bots ---------------------------------------------------------------------------

/// A line as a bot is shown it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// The message id (entry hex).
    pub id: String,
    /// `person` or `bot`.
    pub from_kind: &'static str,
    pub from_id: String,
    pub from_name: String,
    pub text: String,
    pub at: u64,
    pub in_reply_to: Option<String>,
}

/// One line handed to one bot of mine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivery {
    /// Unique per delivery. The answer names it.
    pub id: String,
    /// This PC's own bot id.
    pub bot_local: String,
    pub bot: BotId,
    /// The bot's name in this space.
    pub bot_name: String,
    pub space: SpaceId,
    pub space_name: String,
    pub channel: ChannelId,
    pub channel_name: String,
    pub line: Line,
    /// Earlier talk. Empty on an owner turn: the owner's turn sees only the owner's own message.
    pub recent: Vec<Line>,
    /// The line is from the bot's owner (by signature).
    pub owner_turn: bool,
    pub memory: bool,
}

/// Why a bot could not take a line, as the one running it says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Switched off, paused, stopped, or not able to start.
    Unavailable,
    /// Its daily money limit.
    Limit,
    /// A turn of another kind is running in this channel.
    Busy,
    /// A brain it may not use here.
    NotAllowed,
}

/// A Potato bot of this PC that could join a space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub name: String,
}

pub trait BotSink: Send + Sync {
    /// Takes the line or says why not. Nothing is waited for: the answer comes later through the service.
    fn deliver(&self, d: &Delivery) -> Result<(), Refusal>;
    /// The bots of this PC that may be brought into a space.
    fn candidates(&self) -> Vec<Candidate>;
}

// ---- what to deliver ----------------------------------------------------------------------------------------------

/// Why a line did not go to a bot (the tests and the owner's counters read this).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// Not a bot of this PC, or its roster owner is not this PC's member key.
    NotMine,
    Inactive,
    NotText,
    Deleted,
    Silenced,
    Muted,
    Removed,
    /// An archived channel, a channel the space's bot rules do not allow, or an owner who left.
    Closed,
    Blocked,
    Old,
    Handled,
    NotCalled,
    Cap(&'static str),
}

/// What the decision reads.
pub struct Ctx<'a> {
    pub store: &'a SpaceStore,
    pub roster: &'a Roster,
    /// This PC's own member key.
    pub me: &'a MemberKey,
    /// My place in the space is "active".
    pub active: bool,
    pub now: u64,
}

/// A decision to deliver, with the hold taken on the caps (give it back with `usage_release` if the sink refuses).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    pub delivery: Delivery,
    pub usage: i64,
    pub called_by: Option<MemberKey>,
}

fn same_char(a: char, b: char) -> bool { a == b || a.to_lowercase().eq(b.to_lowercase()) }

/// `@name` as a word: nothing alphanumeric or `_` right before the `@` or right after the name; any case. Works on
/// characters, so a name in any script matches and no byte offset is ever used.
pub fn is_called(text: &str, name: &str) -> bool {
    let (t, n): (Vec<char>, Vec<char>) = (text.chars().collect(), name.chars().collect());
    if n.is_empty() || t.len() < n.len() + 1 { return false; }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    (0..=t.len() - n.len() - 1).any(|i| {
        t[i] == '@'
            && (i == 0 || !word(t[i - 1]))
            && n.iter().enumerate().all(|(k, c)| same_char(t[i + 1 + k], *c))
            && !t.get(i + 1 + n.len()).is_some_and(|c| word(*c))
    })
}

fn is_question(text: &str) -> bool {
    let t = text.trim_end();
    (t.ends_with('?') || t.ends_with('？')) && t.chars().count() >= HELPS_MIN_CHARS
}

fn member_name(roster: &Roster, m: &MemberKey) -> String {
    roster.members.get(m).map(|x| x.name.clone()).unwrap_or_else(|| "Someone".to_string())
}

fn line_of(ctx: &Ctx, row: &MessageRow, chars: usize) -> Line {
    let text: String = row.text.chars().take(chars).collect();
    match (row.kind, row.bot) {
        (KIND_BOT, Some(b)) => Line {
            id: row.id.to_hex(),
            from_kind: "bot",
            from_id: b.to_hex(),
            from_name: ctx.roster.bots.get(&b).map(|x| x.name.clone()).unwrap_or_else(|| "a bot".to_string()),
            text,
            at: row.created_ms,
            in_reply_to: row.reply_to.map(|r| r.to_hex()),
        },
        _ => Line {
            id: row.id.to_hex(),
            from_kind: "person",
            from_id: row.member.to_hex(),
            from_name: member_name(ctx.roster, &row.member),
            text,
            at: row.created_ms,
            in_reply_to: row.reply_to.map(|r| r.to_hex()),
        },
    }
}

fn recent_before(ctx: &Ctx, row: &MessageRow) -> Vec<Line> {
    let Ok(page) = ctx.store.messages_page(&row.channel, Some((row.lamport, row.id)), RECENT_MAX * 2) else { return Vec::new() };
    let mut lines: Vec<Line> = page
        .iter()
        .filter(|m| (m.kind == KIND_TEXT || m.kind == KIND_BOT) && m.deleted_by.is_none())
        .filter(|m| m.member == *ctx.me || !ctx.store.is_blocked(&m.member))
        .map(|m| line_of(ctx, m, RECENT_LINE_CHARS))
        .collect();
    let extra = lines.len().saturating_sub(RECENT_MAX);
    lines.drain(..extra);
    lines
}

/// Does `row` reply to a `BotText` of this bot?
fn replies_to_bot(ctx: &Ctx, row: &MessageRow, bot: &BotId) -> bool {
    row.reply_to
        .and_then(|r| ctx.store.message(&r).ok().flatten())
        .is_some_and(|m| m.kind == KIND_BOT && m.bot == Some(*bot) && m.deleted_by.is_none())
}

/// The whole decision for one bot and one stored line. On `Ok` the line is marked handled and a hold is taken on the
/// caps; the caller hands the delivery to the sink and gives the hold back if the sink refuses.
pub fn consider(ctx: &Ctx, row: &MessageRow, local: &BotLocal, delivery_id: String) -> Result<Planned, Skip> {
    let Some(potato) = local.potato.clone() else { return Err(Skip::NotMine) };
    // Rule 2: the roster says this bot belongs to this PC's member key. A row of `bot_local` alone proves nothing.
    if ctx.roster.bot_owners.get(&local.bot) != Some(ctx.me) { return Err(Skip::NotMine); }
    if !ctx.active { return Err(Skip::Inactive); }
    if row.kind != KIND_TEXT { return Err(Skip::NotText); }
    if row.deleted_by.is_some() { return Err(Skip::Deleted); }
    if local.silenced { return Err(Skip::Silenced); }
    if local.muted { return Err(Skip::Muted); }
    let Some(bot) = ctx.roster.bots.get(&local.bot) else { return Err(Skip::Removed) };
    if !ctx.roster.bot_may_speak(&local.bot, &row.channel) { return Err(Skip::Closed); }
    let owner_turn = row.member == *ctx.me;
    if !owner_turn && ctx.store.is_blocked(&row.member) { return Err(Skip::Blocked); }
    if ctx.now.saturating_sub(row.created_ms) > MAX_AGE_MS { return Err(Skip::Old); }
    if handled_has(ctx.store, &row.id, &local.bot) { return Err(Skip::Handled); }

    let called = is_called(&row.text, &bot.name) || replies_to_bot(ctx, row, &local.bot);
    let unprompted = !called;
    if unprompted && !(local.mode == Mode::Helps && is_question(&row.text)) { return Err(Skip::NotCalled); }

    if used_in_day(ctx.store, &local.bot, ctx.now) >= local.day_cap { return Err(Skip::Cap("day")); }
    if !owner_turn {
        if used_by_person(ctx.store, &local.bot, &row.member, ctx.now) >= BOT_PERSON_HOUR_CAP { return Err(Skip::Cap("person")); }
        if unprompted && used_unprompted(ctx.store, &local.bot, ctx.now) >= BOT_UNPROMPTED_CAP { return Err(Skip::Cap("unprompted")); }
    }

    handled_mark(ctx.store, &row.id, &local.bot, ctx.now).map_err(|_| Skip::Cap("store"))?;
    // The owner's own lines count toward the day cap only.
    let usage = usage_add(ctx.store, &local.bot, (!owner_turn).then_some(&row.member), unprompted && !owner_turn, ctx.now)
        .map_err(|_| Skip::Cap("store"))?;
    let channel_name = ctx.roster.channel(&row.channel).map(|c| c.name.clone()).unwrap_or_default();
    Ok(Planned {
        delivery: Delivery {
            id: delivery_id,
            bot_local: potato,
            bot: local.bot,
            bot_name: bot.name.clone(),
            space: ctx.roster.space,
            space_name: ctx.roster.name.clone(),
            channel: row.channel,
            channel_name,
            line: line_of(ctx, row, MAX_TEXT_CHARS),
            recent: if owner_turn { Vec::new() } else { recent_before(ctx, row) },
            owner_turn,
            memory: local.memory,
        },
        usage,
        called_by: called.then_some(row.member),
    })
}

/// Every bot of mine that takes this line. `new_id` makes a delivery id.
pub fn plan(ctx: &Ctx, row: &MessageRow, new_id: &mut dyn FnMut() -> String) -> Vec<Planned> {
    let Ok(locals) = local_list(ctx.store) else { return Vec::new() };
    locals.iter().filter_map(|l| consider(ctx, row, l, new_id()).ok()).collect()
}

// ---- the answer ---------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Clean text, ready to post.
    Post(String),
    /// Nothing to say.
    Empty,
    /// Looked private. Nothing is posted and the text is not kept anywhere; only a counter moves.
    Held(&'static str),
}

/// The text of an answer, in this order: secrets out, control and direction characters out, the leak check, the
/// length caps (4000 characters, and the byte cap an entry has).
pub fn clean_reply(raw: &str, marks: &PrivateMarks) -> Reply {
    let redacted = crate::redact::text(raw).into_inner();
    let cleaned = clean_text(&redacted);
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() { return Reply::Empty; }
    if let Some(why) = leak_reason(&cleaned, marks) { return Reply::Held(why); }
    let mut out = String::new();
    for c in cleaned.chars().take(MAX_TEXT_CHARS) {
        if out.len() + c.len_utf8() > MAX_TEXT_BYTES { break; }
        out.push(c);
    }
    Reply::Post(out.trim_end().to_string())
}

/// `app:space:<space hex>:<channel hex>`: the conversation a delivery and its answer share.
pub fn conversation_of(space: &SpaceId, channel: &ChannelId) -> String {
    format!("app:space:{}:{}", to_hex(&space.0), to_hex(&channel.0))
}
