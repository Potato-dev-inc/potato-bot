//! The message log: one signed, sealed entry per message, reaction, delete, file offer, bot answer.
//!
//! Private-space format (founder 2026-11-07): a bot answer is `BotText` (kind 7). It is signed by the device of the
//! bot's owner, and the bot id sits in the header (clear, signed, and part of the sealed data's associated data) so
//! every receiver can check it against the roster without opening the body. Running a bot is somebody else's job.
use sha2::{Digest, Sha256};

use super::identity::SpaceIdentity;
use super::ids::{BotId, ChannelId, DeviceKey, EntryId, MemberKey, SpaceId};
use super::limits::{KEEP_MS, MAX_ENTRY_BYTES, MAX_TEXT_CHARS, NAME_MAX_CHARS};
use super::roster::Roster;
use super::wire::{Reader, Writer};
use crate::secure_core::crypto::{open_with_key, seal_with_key};
use crate::secure_core::sign;

pub const ENTRY_DOMAIN: &[u8] = b"pm/spaces/entry/v1";
pub const MAGIC: &[u8; 4] = b"PMSE";
/// Magic through bot id: 122 bytes of the first layout plus the 8-byte bot id.
pub const HEADER_LEN: usize = 130;
pub const FUTURE_SKEW_MS: u64 = 10 * 60 * 1000;
pub const REACTIONS: [&str; 8] = ["thumbs_up", "heart", "laugh", "wow", "sad", "party", "eyes", "check"];
/// Longest text in bytes. 4000 characters of 4 bytes would be 16 000 bytes, which with the mentions no longer fits
/// the biggest padding bucket (16 380 bytes), so the byte cap is lower than 4 x the character cap.
pub const MAX_TEXT_BYTES: usize = 15_000;
pub const MAX_MENTIONS: usize = 20;
/// A bot answers a person. Answering another bot is off; the byte is in the format so it can be allowed later.
pub const BOT_HOPS_MAX: u8 = 0;

pub const KIND_TEXT: u8 = 1;
pub const KIND_REACT: u8 = 3;
pub const KIND_DELETE: u8 = 4;
pub const KIND_LEAVE: u8 = 5;
pub const KIND_FILE: u8 = 6;
pub const KIND_BOT: u8 = 7;
/// A member asks the space owner's PC to put the member's bot on the roster. Hidden from the feed.
pub const KIND_BOT_CLAIM: u8 = 8;
/// A member takes their own bot off the roster. Hidden from the feed.
pub const KIND_BOT_DROP: u8 = 9;
/// A member asks the space owner's PC to publish their new name. Hidden from the feed. Older builds see a reserved kind.
pub const KIND_NAME_CLAIM: u8 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogError(pub &'static str);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryHeader {
    pub space: SpaceId,
    pub channel: ChannelId,
    pub device: DeviceKey,
    pub seq: u64,
    pub lamport: u64,
    pub created_ms: u64,
    pub epoch: u32,
    pub kind: u8,
    pub refers: Option<EntryId>,
    /// Which bot spoke. Set for `BotText` and for nothing else.
    pub bot: Option<BotId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Text { text: String, mentions: Vec<MemberKey> },
    React { code: String, on: bool },
    Delete,
    Leave,
    /// Raw body, decoded by files.rs (D8.1).
    File(Vec<u8>),
    /// A bot's answer. `called_by` is the member who called it (None if unprompted). The entry's `refers` is the
    /// message it answers. `hops` is 0 (a bot answering a bot is off).
    BotText { text: String, called_by: Option<MemberKey>, hops: u8 },
    /// `sig` is the author's member signature over `roster::bot_claim_bytes` (domain `BOT_DOMAIN`). The owner's PC
    /// turns it into an `AddBot` op if the roster rules accept it.
    BotClaim { bot: BotId, name: String, sig: [u8; 64] },
    /// Only the author's own bot is taken off.
    BotDrop { bot: BotId },
    /// `sig` is the author's member signature over `roster::name_claim_bytes` (domain `NAME_DOMAIN`).
    NameClaim { name: String, sig: [u8; 64] },
    Reserved(u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub header: EntryHeader,
    pub nonce: [u8; 24],
    pub sealed: Vec<u8>,
    pub sig: [u8; 64],
}

impl EntryHeader {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.fixed(MAGIC).u8(1).fixed(&self.space.0).fixed(&self.channel.0).fixed(&self.device.0)
            .u64(self.seq).u64(self.lamport).u64(self.created_ms).u32(self.epoch).u8(self.kind)
            .fixed(&self.refers.map(|e| e.0).unwrap_or([0u8; 32]))
            .fixed(&self.bot.map(|b| b.0).unwrap_or([0u8; 8]));
        w.finish()
    }
}

impl Entry {
    pub fn seal(me: &SpaceIdentity, header: EntryHeader, body: &Body, epoch_key: &[u8; 32]) -> Result<Entry, LogError> {
        if header.device != me.device_key() { return Err(LogError("not my device")); }
        let plain = encode_body(body)?;
        let head = header.to_bytes();
        let (nonce, sealed) = seal_with_key(epoch_key, &head, &plain).map_err(|_| LogError("seal"))?;
        let mut e = Entry { header, nonce, sealed, sig: [0u8; 64] };
        e.sig = me.sign_as_device(ENTRY_DOMAIN, &e.signed_part());
        if e.to_bytes().len() > MAX_ENTRY_BYTES { return Err(LogError("too big")); }
        Ok(e)
    }

    pub(super) fn signed_part(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.fixed(&self.header.to_bytes()).fixed(&self.nonce).blob(&self.sealed);
        w.finish()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = self.signed_part();
        b.extend_from_slice(&self.sig);
        b
    }

    /// Size cap first, then magic, version, kind below 64, no trailing bytes.
    pub fn from_bytes(b: &[u8]) -> Result<Entry, LogError> {
        if b.len() > MAX_ENTRY_BYTES { return Err(LogError("too big")); }
        let bad = |_| LogError("bad entry");
        let mut r = Reader::new(b);
        if &r.fixed::<4>().map_err(bad)? != MAGIC { return Err(LogError("bad entry")); }
        if r.u8().map_err(bad)? != 1 { return Err(LogError("bad version")); }
        let space = SpaceId(r.fixed().map_err(bad)?);
        let channel = ChannelId(r.fixed().map_err(bad)?);
        let device = DeviceKey(r.fixed().map_err(bad)?);
        let seq = r.u64().map_err(bad)?;
        let lamport = r.u64().map_err(bad)?;
        let created_ms = r.u64().map_err(bad)?;
        let epoch = r.u32().map_err(bad)?;
        let kind = r.u8().map_err(bad)?;
        if kind >= 64 { return Err(LogError("bad kind")); }
        let refers: [u8; 32] = r.fixed().map_err(bad)?;
        let bot: [u8; 8] = r.fixed().map_err(bad)?;
        let nonce = r.fixed().map_err(bad)?;
        let sealed = r.blob(MAX_ENTRY_BYTES).map_err(bad)?.to_vec();
        let sig = r.fixed().map_err(bad)?;
        r.done().map_err(bad)?;
        let header = EntryHeader {
            space, channel, device, seq, lamport, created_ms, epoch, kind,
            refers: if refers == [0u8; 32] { None } else { Some(EntryId(refers)) },
            bot: if bot == [0u8; 8] { None } else { Some(BotId(bot)) },
        };
        Ok(Entry { header, nonce, sealed, sig })
    }

    /// The message id: SHA-256 of every byte including the signature. Stores and diaries key lines by it.
    pub fn id(&self) -> EntryId { EntryId(Sha256::digest(self.to_bytes()).into()) }

    pub fn signature_ok(&self) -> bool {
        sign::verify(&self.header.device.0, ENTRY_DOMAIN, &self.signed_part(), &self.sig)
    }

    pub fn open(&self, epoch_key: &[u8; 32]) -> Result<Body, LogError> {
        let plain = open_with_key(epoch_key, &self.nonce, &self.header.to_bytes(), &self.sealed)
            .map_err(|_| LogError("cannot open"))?;
        decode_body(self.header.kind, &plain)
    }
}

fn check_text(text: &str) -> Result<(), LogError> {
    if text.chars().count() > MAX_TEXT_CHARS || text.len() > MAX_TEXT_BYTES { return Err(LogError("text too long")); }
    Ok(())
}

pub fn kind_of(body: &Body) -> u8 {
    match body {
        Body::Text { .. } => KIND_TEXT,
        Body::React { .. } => KIND_REACT,
        Body::Delete => KIND_DELETE,
        Body::Leave => KIND_LEAVE,
        Body::File(_) => KIND_FILE,
        Body::BotText { .. } => KIND_BOT,
        Body::BotClaim { .. } => KIND_BOT_CLAIM,
        Body::BotDrop { .. } => KIND_BOT_DROP,
        Body::NameClaim { .. } => KIND_NAME_CLAIM,
        Body::Reserved(k) => *k,
    }
}

pub fn encode_body(body: &Body) -> Result<Vec<u8>, LogError> {
    let mut w = Writer::new();
    w.u8(1);
    match body {
        Body::Text { text, mentions } => {
            check_text(text)?;
            if mentions.len() > MAX_MENTIONS { return Err(LogError("too many mentions")); }
            w.str(text).u8(mentions.len() as u8);
            for m in mentions { w.fixed(&m.0); }
        }
        Body::React { code, on } => {
            if !REACTIONS.contains(&code.as_str()) { return Err(LogError("unknown reaction")); }
            w.str(code).u8(*on as u8);
        }
        Body::Delete | Body::Leave => {}
        Body::File(raw) => { w.fixed(raw); }
        Body::BotText { text, called_by, hops } => {
            check_text(text)?;
            if *hops > BOT_HOPS_MAX { return Err(LogError("a bot may not answer a bot")); }
            w.str(text).fixed(&called_by.map(|m| m.0).unwrap_or([0u8; 32])).u8(*hops);
        }
        Body::BotClaim { bot, name, sig } => {
            if name.chars().count() > NAME_MAX_CHARS || name.is_empty() { return Err(LogError("bad bot name")); }
            w.fixed(&bot.0).str(name).fixed(sig);
        }
        Body::BotDrop { bot } => { w.fixed(&bot.0); }
        Body::NameClaim { name, sig } => {
            if name.chars().count() > NAME_MAX_CHARS || name.is_empty() { return Err(LogError("bad name")); }
            w.str(name).fixed(sig);
        }
        Body::Reserved(_) => return Err(LogError("reserved kind")),
    }
    Ok(w.finish())
}

pub fn decode_body(kind: u8, plain: &[u8]) -> Result<Body, LogError> {
    let bad = |_| LogError("bad body");
    if kind >= 64 { return Err(LogError("bad kind")); }
    if kind == 2 || (11..64).contains(&kind) { return Ok(Body::Reserved(kind)); }
    let mut r = Reader::new(plain);
    if r.u8().map_err(bad)? != 1 { return Err(LogError("bad body version")); }
    let body = match kind {
        KIND_TEXT => {
            let text = r.str(MAX_TEXT_BYTES).map_err(bad)?;
            check_text(&text)?;
            if text != clean_text(&text) { return Err(LogError("unclean text")); }
            let n = r.u8().map_err(bad)? as usize;
            if n > MAX_MENTIONS { return Err(LogError("too many mentions")); }
            let mut mentions = Vec::with_capacity(n);
            for _ in 0..n { mentions.push(MemberKey(r.fixed().map_err(bad)?)); }
            Body::Text { text, mentions }
        }
        KIND_REACT => {
            let code = r.str(32).map_err(bad)?;
            if !REACTIONS.contains(&code.as_str()) { return Err(LogError("unknown reaction")); }
            let on = match r.u8().map_err(bad)? { 0 => false, 1 => true, _ => return Err(LogError("bad body")) };
            Body::React { code, on }
        }
        KIND_DELETE => Body::Delete,
        KIND_LEAVE => Body::Leave,
        KIND_FILE => Body::File(r.skip_rest().to_vec()),
        KIND_BOT => {
            let text = r.str(MAX_TEXT_BYTES).map_err(bad)?;
            check_text(&text)?;
            if text != clean_text(&text) { return Err(LogError("unclean text")); }
            let who: [u8; 32] = r.fixed().map_err(bad)?;
            let hops = r.u8().map_err(bad)?;
            if hops > BOT_HOPS_MAX { return Err(LogError("a bot may not answer a bot")); }
            Body::BotText { text, called_by: if who == [0u8; 32] { None } else { Some(MemberKey(who)) }, hops }
        }
        KIND_BOT_CLAIM => {
            let bot = BotId(r.fixed().map_err(bad)?);
            let name = r.str(NAME_MAX_CHARS * 4).map_err(bad)?;
            if name.chars().count() > NAME_MAX_CHARS || name.is_empty() { return Err(LogError("bad bot name")); }
            Body::BotClaim { bot, name, sig: r.fixed().map_err(bad)? }
        }
        KIND_BOT_DROP => Body::BotDrop { bot: BotId(r.fixed().map_err(bad)?) },
        KIND_NAME_CLAIM => {
            let name = r.str(NAME_MAX_CHARS * 4).map_err(bad)?;
            if name.chars().count() > NAME_MAX_CHARS || name.is_empty() { return Err(LogError("bad name")); }
            Body::NameClaim { name, sig: r.fixed().map_err(bad)? }
        }
        _ => return Err(LogError("bad kind")),
    };
    r.done().map_err(bad)?;
    Ok(body)
}

/// Keep new lines and tabs, drop other control chars and bidi overrides, trim the end, cap at 4000 chars.
pub fn clean_text(raw: &str) -> String {
    let kept: String = raw
        .chars()
        .filter(|c| {
            (!c.is_control() || matches!(*c, '\n' | '\t'))
                && !matches!(*c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200B}'..='\u{200F}')
        })
        .take(MAX_TEXT_CHARS)
        .collect();
    kept.trim_end().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Valid; `member` is the author's member key.
    Ok { member: MemberKey },
    /// Valid but too old to keep (older than KEEP_MS): do not store, raise the floor.
    Old,
    Reject(&'static str),
}

/// Every check a receiver runs before storing, in this order. Does not open the body.
pub fn check(roster: &Roster, e: &Entry, now_ms: u64) -> Verdict {
    let h = &e.header;
    if h.space != roster.space { return Verdict::Reject("wrong space"); }
    let Some((member, until)) = roster.author(&h.device) else { return Verdict::Reject("unknown author") };
    if h.epoch == 0 || h.epoch > roster.epoch { return Verdict::Reject("future epoch"); }
    if let Some(limit) = until {
        if h.epoch >= limit { return Verdict::Reject("removed author"); }
        // Partial defence only: `created_ms` is set by the author, so a removed member can still claim a date
        // from before the removal. What this stops is the lazy forgery (a plain current date in the old epoch).
        if roster.past_ms.get(&h.device).is_some_and(|t| h.created_ms > t.saturating_add(FUTURE_SKEW_MS)) {
            return Verdict::Reject("after removal");
        }
    }
    if !e.signature_ok() { return Verdict::Reject("bad signature"); }
    if h.seq == 0 { return Verdict::Reject("bad seq"); }
    if h.created_ms > now_ms + FUTURE_SKEW_MS { return Verdict::Reject("from the future"); }
    if h.created_ms + KEEP_MS < now_ms { return Verdict::Old; }
    if roster.channel(&h.channel).is_none() { return Verdict::Reject("unknown channel"); }
    if matches!(h.kind, KIND_REACT | KIND_DELETE) && h.refers.is_none() { return Verdict::Reject("missing target"); }
    if h.kind == KIND_BOT {
        // Only what never changes is checked here: the bot existed and is this member's. Removal of the bot, the
        // bot rules and a closed channel are as of today and differ between PCs and over time, so they are
        // display/send filters (`Roster::bot_may_speak`, checked when sending), not reasons to refuse history.
        let Some(bot) = h.bot else { return Verdict::Reject("missing bot") };
        let Some(owner) = roster.bot_owners.get(&bot) else { return Verdict::Reject("bot not on the roster") };
        if *owner != member { return Verdict::Reject("bot is not yours"); }
    } else if h.bot.is_some() {
        return Verdict::Reject("bot id on a non-bot entry");
    }
    Verdict::Ok { member }
}
