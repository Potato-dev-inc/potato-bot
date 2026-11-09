//! Store an incoming entry: check, insert, decrypt into messages, apply reactions and deletes.
use std::collections::BTreeMap;

use rusqlite::{params, OptionalExtension};

use super::files::FileOffer;
use super::ids::{to_hex, DeviceKey, EntryId, MemberKey};
use super::log::{self, Body, Entry, Verdict, KIND_DELETE};
use super::ratelimit::{author_over_limit, Over};
use super::roster::Roster;
use super::store::SpaceStore;

/// Epoch keys this PC can use, by epoch.
#[derive(Default, Clone)]
pub struct EpochKeys(pub BTreeMap<u32, [u8; 32]>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ingest {
    /// Stored. `shown` = a messages row was written or changed (UI should refresh).
    Stored { id: EntryId, shown: bool },
    Duplicate,
    Old,
    Rejected(&'static str),
}

/// Most a stored entry's lamport may be ahead of the local clock.
const LAMPORT_AHEAD_MAX: u64 = 1_000_000;

pub fn ingest_entry(store: &SpaceStore, roster: &Roster, keys: &EpochKeys, bytes: &[u8], now_ms: u64) -> Ingest {
    let Ok(e) = Entry::from_bytes(bytes) else { return Ingest::Rejected("unreadable") };
    let id = e.id();
    if store.has_entry(&id).unwrap_or(false) { return Ingest::Duplicate; }
    let member = match log::check(roster, &e, now_ms) {
        Verdict::Ok { member } => member,
        Verdict::Old => {
            let _ = store.raise_floor(&e.header.device, e.header.seq);
            return Ingest::Old;
        }
        Verdict::Reject(r) => return Ingest::Rejected(r),
    };
    // A lamport far ahead of this PC's clock would drag every later send up with it (and could reach i64::MAX).
    if e.header.lamport > store.lamport_now().unwrap_or(0).saturating_add(LAMPORT_AHEAD_MAX) {
        return Ingest::Rejected("lamport too far ahead");
    }
    // Too many entries from this author in their own timestamps (same answer on every PC): not stored, so not forwarded.
    if !matches!(author_over_limit(store, &e.header.device, e.header.created_ms), Ok(Over::No)) {
        return Ingest::Rejected("too fast");
    }
    // The same (device, seq) with other bytes: the author signed two entries with one seq. A placeholder does not count.
    if store.has_seq(&e.header.device, e.header.seq).unwrap_or(false)
        && !store.is_placeholder(&e.header.device, e.header.seq).unwrap_or(false)
    {
        return Ingest::Rejected("seq reused");
    }
    let key = keys.0.get(&e.header.epoch);
    let body = key.and_then(|k| e.open(k).ok());
    if key.is_some() && body.is_none() { return Ingest::Rejected("cannot open"); }
    // A file offer that breaks the size, chunk or name rules is refused like any other bad entry.
    if let Some(Body::File(raw)) = &body {
        if let Err(why) = FileOffer::from_body(raw) { return Ingest::Rejected(why); }
    }
    // One transaction: the entry (replacing a gone placeholder), the lamport clock, then the body.
    let stored = store.in_tx(|s| {
        s.insert_entry(&e, &id, bytes, now_ms)?;
        s.bump_lamport(e.header.lamport)?;
        match &body {
            Some(b) => apply_body(s, roster, &e, &id, &member, b, now_ms),
            None => Ok(false),
        }
    });
    match stored {
        Ok(shown) => Ingest::Stored { id, shown },
        Err(_) => Ingest::Rejected("store"),
    }
}

/// Text/Bot/File -> messages row (and apply a waiting delete for it). React -> reactions (last lamport wins).
/// Delete -> if the target exists and (its author is the deleter or the deleter is the owner): hide the message and
/// drop the target's sealed bytes. If the target is not here yet, nothing else: when it arrives, a stored valid Delete
/// for it is applied first. Leave -> nothing in messages (the owner's service turns it into RemoveMember + Rotate).
/// Reserved -> nothing.
fn apply_body(s: &SpaceStore, roster: &Roster, e: &Entry, id: &EntryId, member: &MemberKey, body: &Body, now_ms: u64)
    -> rusqlite::Result<bool>
{
    let h = &e.header;
    match body {
        Body::Text { text, .. } => put_message(s, roster, e, id, member, text, None, now_ms),
        Body::BotText { text, .. } => put_message(s, roster, e, id, member, text, None, now_ms),
        Body::File(raw) => {
            // The offer was checked on the way in. The row is how `Get file` finds the offer; no bytes yet (have = 0).
            if let Ok(offer) = FileOffer::from_body(raw) {
                s.conn.execute(
                    "INSERT OR IGNORE INTO files (sha, name, size, have, offer, cached_ms) VALUES (?1, ?2, ?3, 0, ?4, NULL)",
                    params![offer.sha.to_vec(), offer.name, offer.size as i64, id.0.to_vec()],
                )?;
            }
            put_message(s, roster, e, id, member, "", Some(to_hex(raw)), now_ms)
        }
        Body::React { code, on } => {
            let Some(target) = h.refers else { return Ok(false) };
            s.conn.execute(
                "INSERT INTO reactions (target, member, emoji, is_on, lamport, entry) VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(target, member, emoji) DO UPDATE SET is_on = excluded.is_on, lamport = excluded.lamport, \
                 entry = excluded.entry WHERE excluded.lamport > reactions.lamport \
                 OR (excluded.lamport = reactions.lamport AND excluded.entry > reactions.entry)",
                params![target.0.to_vec(), member.0.to_vec(), code, *on as i64, h.lamport as i64, id.0.to_vec()],
            )?;
            Ok(true)
        }
        Body::Delete => {
            let Some(target) = h.refers else { return Ok(false) };
            apply_delete(s, roster, &target, member, now_ms)
        }
        // A bot claim or drop is for the owner's PC to act on (`service/bots.rs`); it is never a message in the feed.
        Body::Leave | Body::BotClaim { .. } | Body::BotDrop { .. } | Body::NameClaim { .. } | Body::Reserved(_) => Ok(false),
    }
}

fn put_message(
    s: &SpaceStore, roster: &Roster, e: &Entry, id: &EntryId, member: &MemberKey, text: &str, file_json: Option<String>,
    now_ms: u64,
) -> rusqlite::Result<bool> {
    let h = &e.header;
    s.conn.execute(
        "INSERT OR REPLACE INTO messages (id, channel, member, device, bot, lamport, created_ms, kind, text, reply_to, \
         deleted_by, deleted_ms, file_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, NULL, ?11)",
        params![
            id.0.to_vec(), h.channel.0.to_vec(), member.0.to_vec(), h.device.0.to_vec(), h.bot.map(|b| b.0.to_vec()),
            h.lamport as i64, h.created_ms as i64, h.kind as i64, text, h.refers.map(|r| r.0.to_vec()), file_json
        ],
    )?;
    // A Delete for this message may have arrived first: apply the first valid one.
    let waiting: Vec<Vec<u8>> = {
        let mut st = s.conn.prepare("SELECT bytes FROM entries WHERE ref_id = ?1 AND kind = ?2 AND gone = 0 ORDER BY lamport, id")?;
        let rows = st.query_map(params![id.0.to_vec(), KIND_DELETE as i64], |r| r.get::<_, Vec<u8>>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for raw in waiting {
        let Ok(del) = Entry::from_bytes(&raw) else { continue };
        let Some((deleter, _)) = roster.author(&del.header.device) else { continue };
        if apply_delete(s, roster, id, &deleter, now_ms)? { break; }
    }
    Ok(true)
}

/// Hides the target if `deleter` may delete it. Returns true when something changed.
fn apply_delete(s: &SpaceStore, roster: &Roster, target: &EntryId, deleter: &MemberKey, now_ms: u64) -> rusqlite::Result<bool> {
    let dev: Option<(Vec<u8>, i64)> = s
        .conn
        .query_row("SELECT device, gone FROM entries WHERE id = ?1", params![target.0.to_vec()], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let Some((dev, gone)) = dev else { return Ok(false) };
    if gone == 1 { return Ok(false); }
    let Ok(dev) = <[u8; 32]>::try_from(dev) else { return Ok(false) };
    let Some((author, _)) = roster.author(&DeviceKey(dev)) else { return Ok(false) };
    if author != *deleter && !roster.is_owner(deleter) { return Ok(false); }
    s.conn.execute(
        "UPDATE messages SET deleted_by = ?2, deleted_ms = ?3, text = '', file_json = NULL WHERE id = ?1",
        params![target.0.to_vec(), deleter.0.to_vec(), now_ms as i64],
    )?;
    s.conn.execute("UPDATE entries SET bytes = x'', size = 0, gone = 1 WHERE id = ?1", params![target.0.to_vec()])?;
    Ok(true)
}
