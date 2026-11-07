//! One SQLite file per space. Never holds keys or invite secrets (those live in the OS keystore).
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use sha2::{Digest, Sha256};

use super::ids::{BotId, ChannelId, DeviceKey, EntryId, InviteId, MemberKey, NodeKey};
use super::log::Entry;
use super::invite::{InviteRecord, InviteState};
use super::roster::ControlOp;

pub const SCHEMA: &str = "
PRAGMA auto_vacuum = INCREMENTAL;
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS control_ops (
  seq   INTEGER PRIMARY KEY,
  hash  BLOB NOT NULL UNIQUE,
  bytes BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS authors (
  device BLOB PRIMARY KEY,
  floor  INTEGER NOT NULL DEFAULT 0,
  high   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS entries (
  id          BLOB PRIMARY KEY,
  device      BLOB NOT NULL,
  seq         INTEGER NOT NULL,
  channel     BLOB NOT NULL,
  lamport     INTEGER NOT NULL,
  created_ms  INTEGER NOT NULL,
  received_ms INTEGER NOT NULL,
  kind        INTEGER NOT NULL,
  ref_id      BLOB,
  epoch       INTEGER NOT NULL,
  bytes       BLOB NOT NULL,
  size        INTEGER NOT NULL,
  gone        INTEGER NOT NULL DEFAULT 0,
  UNIQUE (device, seq)
);
CREATE INDEX IF NOT EXISTS entries_by_created ON entries (created_ms);
CREATE INDEX IF NOT EXISTS entries_by_ref ON entries (ref_id);

CREATE TABLE IF NOT EXISTS messages (
  id          BLOB PRIMARY KEY REFERENCES entries(id) ON DELETE CASCADE,
  channel     BLOB NOT NULL,
  member      BLOB NOT NULL,
  device      BLOB NOT NULL,
  bot         BLOB,
  lamport     INTEGER NOT NULL,
  created_ms  INTEGER NOT NULL,
  kind        INTEGER NOT NULL,
  text        TEXT NOT NULL,
  reply_to    BLOB,
  deleted_by  BLOB,
  deleted_ms  INTEGER,
  file_json   TEXT
);
CREATE INDEX IF NOT EXISTS messages_by_channel ON messages (channel, lamport, created_ms, id);

CREATE TABLE IF NOT EXISTS reactions (
  target  BLOB NOT NULL,
  member  BLOB NOT NULL,
  emoji   TEXT NOT NULL,
  is_on   INTEGER NOT NULL,
  lamport INTEGER NOT NULL,
  entry   BLOB NOT NULL,
  PRIMARY KEY (target, member, emoji)
);

CREATE TABLE IF NOT EXISTS reads (
  channel      BLOB PRIMARY KEY,
  last_lamport INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS invites (
  id         BLOB PRIMARY KEY,
  created_ms INTEGER NOT NULL,
  expires_ms INTEGER NOT NULL,
  strikes    INTEGER NOT NULL DEFAULT 0,
  state      TEXT NOT NULL CHECK (state IN ('open','pending','used','dead','cancelled')),
  joiner     BLOB
);

CREATE TABLE IF NOT EXISTS join_requests (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  invite       BLOB NOT NULL,
  joiner       BLOB NOT NULL,
  cert         BLOB NOT NULL,
  name         TEXT NOT NULL,
  received_ms  INTEGER NOT NULL,
  answer_by_ms INTEGER NOT NULL,
  state        TEXT NOT NULL CHECK (state IN ('waiting','let_in','refused','expired'))
);

CREATE TABLE IF NOT EXISTS blocks (
  member   BLOB PRIMARY KEY,
  since_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS peers (
  node         BLOB PRIMARY KEY,
  relay        TEXT NOT NULL DEFAULT '',
  lan          TEXT NOT NULL DEFAULT '',
  last_seen_ms INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS files (
  sha       BLOB PRIMARY KEY,
  name      TEXT NOT NULL,
  size      INTEGER NOT NULL,
  have      INTEGER NOT NULL DEFAULT 0,
  offer     BLOB NOT NULL,
  cached_ms INTEGER
);

CREATE TABLE IF NOT EXISTS bot_usage (
  at_ms  INTEGER NOT NULL,
  bot    BLOB NOT NULL,
  person BLOB,
  kind   TEXT NOT NULL CHECK (kind IN ('called','unprompted'))
);
CREATE INDEX IF NOT EXISTS bot_usage_by_time ON bot_usage (at_ms);

PRAGMA user_version = 1;
";

/// Version 2 (bots in a space): what only this PC knows about a bot, and the lines a bot was already given.
/// Run on a fresh store after `SCHEMA`, and once on an older store when it is opened.
pub const MIGRATE_2: &str = "
CREATE TABLE IF NOT EXISTS bot_local (
  bot      BLOB PRIMARY KEY,
  potato   TEXT,
  mode     TEXT NOT NULL DEFAULT 'called' CHECK (mode IN ('called','helps')),
  memory   INTEGER NOT NULL DEFAULT 1,
  day_cap  INTEGER NOT NULL DEFAULT 30,
  silenced INTEGER NOT NULL DEFAULT 0,
  muted    INTEGER NOT NULL DEFAULT 0,
  claimed_ms  INTEGER NOT NULL DEFAULT 0,
  claim_name  TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS bot_handled (
  entry BLOB NOT NULL,
  bot   BLOB NOT NULL,
  at_ms INTEGER NOT NULL,
  PRIMARY KEY (entry, bot)
);
CREATE INDEX IF NOT EXISTS bot_handled_by_time ON bot_handled (at_ms);
CREATE INDEX IF NOT EXISTS bot_usage_by_bot ON bot_usage (bot, at_ms);

PRAGMA user_version = 2;
";

pub struct SpaceStore { pub(crate) conn: Connection }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinRequestRow {
    pub id: i64,
    pub invite: InviteId,
    pub joiner: NodeKey,
    pub cert: Vec<u8>,
    pub name: String,
    pub received_ms: u64,
    pub answer_by_ms: u64,
    pub state: String,
}

fn bad(msg: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, msg.into())
}

fn arr<const N: usize>(v: Vec<u8>) -> rusqlite::Result<[u8; N]> {
    v.try_into().map_err(|_| bad("wrong length"))
}

impl SpaceStore {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_memory() -> rusqlite::Result<Self> { Self::init(Connection::open_in_memory()?) }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v == 0 {
            conn.execute_batch(SCHEMA)?;
            conn.execute_batch(MIGRATE_2)?;
        } else {
            conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
            if v < 2 {
                conn.execute_batch(MIGRATE_2)?;
            }
        }
        Ok(Self { conn })
    }

    pub fn schema_version(&self) -> rusqlite::Result<i64> {
        self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))
    }

    pub fn meta_get(&self, key: &str) -> rusqlite::Result<Option<Vec<u8>>> {
        self.conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0)).optional()
    }
    pub fn meta_set(&self, key: &str, value: &[u8]) -> rusqlite::Result<()> {
        self.conn
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map(|_| ())
    }
    pub fn meta_delete(&self, key: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM meta WHERE key = ?1", params![key]).map(|_| ())
    }
    /// Missing key = 0.
    pub fn meta_u64(&self, key: &str) -> rusqlite::Result<u64> {
        Ok(match self.meta_get(key)? {
            Some(v) => u64::from_be_bytes(arr::<8>(v)?),
            None => 0,
        })
    }
    pub fn meta_set_u64(&self, key: &str, v: u64) -> rusqlite::Result<()> { self.meta_set(key, &v.to_be_bytes()) }
    /// Local Lamport clock (0 when nothing was stored or sent yet).
    pub fn lamport_now(&self) -> rusqlite::Result<u64> { self.meta_u64("lamport") }

    /// Appends only if `op.seq` is the last seq + 1. Otherwise writes nothing and returns Ok(false).
    pub fn control_append(&self, op: &ControlOp) -> rusqlite::Result<bool> {
        let last: i64 = self.conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM control_ops", [], |r| r.get(0))?;
        if op.seq as i64 != last + 1 {
            return Ok(false);
        }
        self.conn.execute(
            "INSERT INTO control_ops (seq, hash, bytes) VALUES (?1, ?2, ?3)",
            params![op.seq as i64, op.hash().to_vec(), op.to_bytes()],
        )?;
        Ok(true)
    }
    pub fn control_ops(&self) -> rusqlite::Result<Vec<ControlOp>> { self.control_ops_after(0) }
    /// Ops with seq greater than `seq`, in order. A row that does not parse is an Err.
    pub fn control_ops_after(&self, seq: u64) -> rusqlite::Result<Vec<ControlOp>> {
        let mut st = self.conn.prepare("SELECT bytes FROM control_ops WHERE seq > ?1 ORDER BY seq")?;
        let rows = st.query_map(params![seq as i64], |r| r.get::<_, Vec<u8>>(0))?;
        let mut out = Vec::new();
        for b in rows {
            out.push(ControlOp::from_bytes(&b?).map_err(|e| bad(&e.0))?);
        }
        Ok(out)
    }
    pub fn control_hash_at(&self, seq: u64) -> rusqlite::Result<Option<[u8; 32]>> {
        let v: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT hash FROM control_ops WHERE seq = ?1", params![seq as i64], |r| r.get(0))
            .optional()?;
        v.map(arr::<32>).transpose()
    }

    pub fn invite_put(&self, r: &InviteRecord) -> rusqlite::Result<()> {
        let (state, joiner) = state_to_sql(r.state);
        self.conn
            .execute(
                "INSERT INTO invites (id, created_ms, expires_ms, strikes, state, joiner) VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(id) DO UPDATE SET created_ms = excluded.created_ms, expires_ms = excluded.expires_ms, \
                 strikes = excluded.strikes, state = excluded.state, joiner = excluded.joiner",
                params![
                    r.id.0.to_vec(), r.created_ms as i64, r.expires_ms as i64, r.strikes as i64, state,
                    joiner.map(|j| j.to_vec())
                ],
            )
            .map(|_| ())
    }
    pub fn invite_get(&self, id: &InviteId) -> rusqlite::Result<Option<InviteRecord>> {
        self.conn
            .query_row(
                "SELECT id, created_ms, expires_ms, strikes, state, joiner FROM invites WHERE id = ?1",
                params![id.0.to_vec()],
                invite_row,
            )
            .optional()?
            .transpose()
    }
    pub fn invites(&self) -> rusqlite::Result<Vec<InviteRecord>> {
        let mut st = self
            .conn
            .prepare("SELECT id, created_ms, expires_ms, strikes, state, joiner FROM invites ORDER BY created_ms, id")?;
        let rows = st.query_map([], invite_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    pub fn join_request_add(
        &self, invite: &InviteId, joiner: &NodeKey, cert: &[u8], name: &str, received_ms: u64, answer_by_ms: u64,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO join_requests (invite, joiner, cert, name, received_ms, answer_by_ms, state) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'waiting')",
            params![invite.0.to_vec(), joiner.0.to_vec(), cert, name, received_ms as i64, answer_by_ms as i64],
        )?;
        Ok(self.conn.last_insert_rowid())
    }
    pub fn join_request_set(&self, id: i64, state: &str) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE join_requests SET state = ?2 WHERE id = ?1", params![id, state]).map(|_| ())
    }
    pub fn join_request(&self, id: i64) -> rusqlite::Result<Option<JoinRequestRow>> {
        self.conn
            .query_row(
                "SELECT id, invite, joiner, cert, name, received_ms, answer_by_ms, state FROM join_requests WHERE id = ?1",
                params![id],
                join_row,
            )
            .optional()?
            .transpose()
    }
    pub fn join_requests_waiting(&self) -> rusqlite::Result<Vec<JoinRequestRow>> {
        let mut st = self.conn.prepare(
            "SELECT id, invite, joiner, cert, name, received_ms, answer_by_ms, state FROM join_requests \
             WHERE state = 'waiting' ORDER BY id",
        )?;
        let rows = st.query_map([], join_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }
}

fn join_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<rusqlite::Result<JoinRequestRow>> {
    let (invite, joiner): (Vec<u8>, Vec<u8>) = (r.get(1)?, r.get(2)?);
    let row = (|| {
        Ok(JoinRequestRow {
            id: r.get(0)?,
            invite: InviteId(arr::<8>(invite)?),
            joiner: NodeKey(arr::<32>(joiner)?),
            cert: r.get(3)?,
            name: r.get(4)?,
            received_ms: r.get::<_, i64>(5)? as u64,
            answer_by_ms: r.get::<_, i64>(6)? as u64,
            state: r.get(7)?,
        })
    })();
    Ok(row)
}

fn invite_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<rusqlite::Result<InviteRecord>> {
    let id: Vec<u8> = r.get(0)?;
    let text: String = r.get(4)?;
    let joiner: Option<Vec<u8>> = r.get(5)?;
    let row = (|| {
        Ok(InviteRecord {
            id: InviteId(arr::<8>(id)?),
            created_ms: r.get::<_, i64>(1)? as u64,
            expires_ms: r.get::<_, i64>(2)? as u64,
            strikes: r.get::<_, i64>(3)? as u32,
            state: state_from_sql(&text, joiner).ok_or_else(|| bad("invite state"))?,
        })
    })();
    Ok(row)
}

/// InviteState <-> (state text, joiner column).
pub fn state_to_sql(s: InviteState) -> (&'static str, Option<[u8; 32]>) {
    match s {
        InviteState::Open => ("open", None),
        InviteState::Pending(n) => ("pending", Some(n.0)),
        InviteState::Used => ("used", None),
        InviteState::Dead => ("dead", None),
        InviteState::Cancelled => ("cancelled", None),
    }
}

pub fn state_from_sql(text: &str, joiner: Option<Vec<u8>>) -> Option<InviteState> {
    Some(match text {
        "open" => InviteState::Open,
        "pending" => InviteState::Pending(NodeKey(joiner?.try_into().ok()?)),
        "used" => InviteState::Used,
        "dead" => InviteState::Dead,
        "cancelled" => InviteState::Cancelled,
        _ => return None,
    })
}

// ----- D3.3: entries, author heads, messages, reactions, reads -----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageRow {
    pub id: EntryId,
    pub channel: ChannelId,
    pub member: MemberKey,
    pub device: DeviceKey,
    /// Set when a bot spoke: the author `member` is then the bot's owner.
    pub bot: Option<BotId>,
    pub lamport: u64,
    pub created_ms: u64,
    pub kind: u8,
    pub text: String,
    pub reply_to: Option<EntryId>,
    pub deleted_by: Option<MemberKey>,
    pub deleted_ms: Option<u64>,
    pub file_json: Option<String>,
}

const MESSAGE_COLS: &str =
    "id, channel, member, device, bot, lamport, created_ms, kind, text, reply_to, deleted_by, deleted_ms, file_json";

fn opt_arr<const N: usize>(v: Option<Vec<u8>>) -> rusqlite::Result<Option<[u8; N]>> {
    v.map(arr::<N>).transpose()
}

fn message_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<rusqlite::Result<MessageRow>> {
    let (id, channel, member, device): (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
    let (bot, reply, del): (Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>) = (r.get(4)?, r.get(9)?, r.get(10)?);
    let row = (|| {
        Ok(MessageRow {
            id: EntryId(arr(id)?),
            channel: ChannelId(arr(channel)?),
            member: MemberKey(arr(member)?),
            device: DeviceKey(arr(device)?),
            bot: opt_arr::<8>(bot)?.map(BotId),
            lamport: r.get::<_, i64>(5)? as u64,
            created_ms: r.get::<_, i64>(6)? as u64,
            kind: r.get::<_, i64>(7)? as u8,
            text: r.get(8)?,
            reply_to: opt_arr::<32>(reply)?.map(EntryId),
            deleted_by: opt_arr::<32>(del)?.map(MemberKey),
            deleted_ms: r.get::<_, Option<i64>>(11)?.map(|v| v as u64),
            file_json: r.get(12)?,
        })
    })();
    Ok(row)
}

/// The id of a "gone" placeholder: SHA-256 of "gone", the author device and the seq.
pub fn placeholder_id(device: &DeviceKey, seq: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"gone");
    h.update(device.0);
    h.update(seq.to_be_bytes());
    h.finalize().into()
}

impl SpaceStore {
    /// Runs `f` in one transaction: all of it is kept, or none of it.
    pub fn in_tx<T>(&self, f: impl FnOnce(&SpaceStore) -> rusqlite::Result<T>) -> rusqlite::Result<T> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        match f(self) {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    pub fn has_entry(&self, id: &EntryId) -> rusqlite::Result<bool> {
        self.conn
            .query_row("SELECT 1 FROM entries WHERE id = ?1", params![id.0.to_vec()], |_| Ok(()))
            .optional()
            .map(|v| v.is_some())
    }
    pub fn has_seq(&self, device: &DeviceKey, seq: u64) -> rusqlite::Result<bool> {
        self.conn
            .query_row("SELECT 1 FROM entries WHERE device = ?1 AND seq = ?2", params![device.0.to_vec(), seq as i64], |_| Ok(()))
            .optional()
            .map(|v| v.is_some())
    }
    /// The (device, seq) row is a gone record (a placeholder, or an entry whose bytes were dropped).
    pub fn is_gone(&self, device: &DeviceKey, seq: u64) -> rusqlite::Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT gone FROM entries WHERE device = ?1 AND seq = ?2",
                params![device.0.to_vec(), seq as i64],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            == Some(1))
    }
    /// The (device, seq) row is a placeholder made by `insert_gone_placeholder` (a real entry may replace it).
    pub fn is_placeholder(&self, device: &DeviceKey, seq: u64) -> rusqlite::Result<bool> {
        let id: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT id FROM entries WHERE device = ?1 AND seq = ?2", params![device.0.to_vec(), seq as i64], |r| r.get(0))
            .optional()?;
        Ok(id.map(|i| i == placeholder_id(device, seq).to_vec()).unwrap_or(false))
    }

    /// One row per author device: (device, low, high). low = floor + 1, high = highest seq seen.
    pub fn heads(&self) -> rusqlite::Result<Vec<(DeviceKey, u64, u64)>> {
        let mut st = self.conn.prepare("SELECT device, floor, high FROM authors ORDER BY device")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))?;
        let mut out = Vec::new();
        for r in rows {
            let (d, floor, high) = r?;
            out.push((DeviceKey(arr(d)?), floor as u64 + 1, high as u64));
        }
        Ok(out)
    }
    pub fn author_floor(&self, device: &DeviceKey) -> rusqlite::Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT floor FROM authors WHERE device = ?1", params![device.0.to_vec()], |r| r.get::<_, i64>(0))
            .optional()?
            .unwrap_or(0) as u64)
    }
    /// floor = max(floor, seq)
    pub fn raise_floor(&self, device: &DeviceKey, seq: u64) -> rusqlite::Result<()> {
        self.conn
            .execute(
                "INSERT INTO authors (device, floor, high) VALUES (?1, ?2, ?2) \
                 ON CONFLICT(device) DO UPDATE SET floor = MAX(floor, excluded.floor), high = MAX(high, excluded.high)",
                params![device.0.to_vec(), seq as i64],
            )
            .map(|_| ())
    }
    fn bump_high(&self, device: &DeviceKey, seq: u64) -> rusqlite::Result<()> {
        self.conn
            .execute(
                "INSERT INTO authors (device, floor, high) VALUES (?1, 0, ?2) \
                 ON CONFLICT(device) DO UPDATE SET high = MAX(high, excluded.high)",
                params![device.0.to_vec(), seq as i64],
            )
            .map(|_| ())
    }
    /// Seqs in [from, to] that are neither stored nor below the floor. At most `max`, ascending.
    pub fn missing(&self, device: &DeviceKey, from: u64, to: u64, max: usize) -> rusqlite::Result<Vec<u64>> {
        let lo = from.max(self.author_floor(device)? + 1);
        let mut out = Vec::new();
        if lo > to || max == 0 { return Ok(out); }
        let mut st = self
            .conn
            .prepare("SELECT seq FROM entries WHERE device = ?1 AND seq >= ?2 AND seq <= ?3 ORDER BY seq")?;
        let rows = st.query_map(params![device.0.to_vec(), lo as i64, to as i64], |r| r.get::<_, i64>(0))?;
        let mut next = lo;
        for s in rows {
            let s = s? as u64;
            while next < s && out.len() < max {
                out.push(next);
                next += 1;
            }
            if out.len() >= max { return Ok(out); }
            next = s + 1;
        }
        while next <= to && out.len() < max {
            out.push(next);
            next += 1;
        }
        Ok(out)
    }

    /// For a Want: (bytes of stored, not gone entries) and (seqs that are gone). A gone entry is never served.
    pub fn entries_for(&self, device: &DeviceKey, seqs: &[u64]) -> rusqlite::Result<(Vec<Vec<u8>>, Vec<u64>)> {
        let (mut have, mut gone) = (Vec::new(), Vec::new());
        for seq in seqs {
            let row: Option<(i64, Vec<u8>)> = self
                .conn
                .query_row(
                    "SELECT gone, bytes FROM entries WHERE device = ?1 AND seq = ?2",
                    params![device.0.to_vec(), *seq as i64],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            match row {
                Some((1, _)) => gone.push(*seq),
                Some((_, bytes)) => have.push(bytes),
                None => {}
            }
        }
        Ok((have, gone))
    }

    /// Stores an entry and raises the author's `high`. A placeholder for the same (device, seq) is replaced.
    pub fn insert_entry(&self, e: &Entry, id: &EntryId, bytes: &[u8], now_ms: u64) -> rusqlite::Result<()> {
        let h = &e.header;
        if self.is_placeholder(&h.device, h.seq)? {
            self.conn.execute("DELETE FROM entries WHERE device = ?1 AND seq = ?2", params![h.device.0.to_vec(), h.seq as i64])?;
        }
        self.conn.execute(
            "INSERT INTO entries (id, device, seq, channel, lamport, created_ms, received_ms, kind, ref_id, epoch, bytes, size, gone) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0)",
            params![
                id.0.to_vec(), h.device.0.to_vec(), h.seq as i64, h.channel.0.to_vec(), h.lamport as i64,
                h.created_ms as i64, now_ms as i64, h.kind as i64, h.refers.map(|r| r.0.to_vec()), h.epoch as i64,
                bytes, bytes.len() as i64
            ],
        )?;
        self.bump_high(&h.device, h.seq)
    }
    pub fn insert_gone_placeholder(&self, device: &DeviceKey, seq: u64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO entries (id, device, seq, channel, lamport, created_ms, received_ms, kind, ref_id, epoch, bytes, size, gone) \
             VALUES (?1, ?2, ?3, x'0000000000000000', 0, 0, 0, 0, NULL, 0, x'', 0, 1)",
            params![placeholder_id(device, seq).to_vec(), device.0.to_vec(), seq as i64],
        )?;
        // No bump_high: a placeholder is not proof the author ever reached this seq.
        Ok(())
    }
    /// high + 1 (1 for an author with no entries).
    pub fn next_seq(&self, device: &DeviceKey) -> rusqlite::Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT high FROM authors WHERE device = ?1", params![device.0.to_vec()], |r| r.get::<_, i64>(0))
            .optional()?
            .unwrap_or(0) as u64
            + 1)
    }
    /// lamport = max(current, seen); returns it.
    pub fn bump_lamport(&self, seen: u64) -> rusqlite::Result<u64> {
        let v = self.meta_u64("lamport")?.max(seen);
        self.meta_set_u64("lamport", v)?;
        Ok(v)
    }
    /// current + 1, stored.
    pub fn next_lamport(&self) -> rusqlite::Result<u64> {
        let v = self.meta_u64("lamport")?.saturating_add(1);
        self.meta_set_u64("lamport", v)?;
        Ok(v)
    }

    /// Oldest first within the page; the page is the newest `limit` rows before the cursor, ordered by
    /// (lamport, created_ms, id).
    pub fn messages_page(&self, channel: &ChannelId, before: Option<(u64, EntryId)>, limit: usize)
        -> rusqlite::Result<Vec<MessageRow>>
    {
        let mut out = Vec::new();
        match before {
            None => {
                let sql = format!(
                    "SELECT {MESSAGE_COLS} FROM messages WHERE channel = ?1 ORDER BY lamport DESC, created_ms DESC, id DESC LIMIT ?2"
                );
                let mut st = self.conn.prepare(&sql)?;
                let rows = st.query_map(params![channel.0.to_vec(), limit as i64], message_row)?;
                for r in rows { out.push(r??); }
            }
            Some((lamport, id)) => {
                let created: i64 = self
                    .conn
                    .query_row("SELECT created_ms FROM messages WHERE id = ?1", params![id.0.to_vec()], |r| r.get(0))
                    .optional()?
                    .unwrap_or(0);
                let sql = format!(
                    "SELECT {MESSAGE_COLS} FROM messages WHERE channel = ?1 AND (lamport, created_ms, id) < (?2, ?3, ?4) \
                     ORDER BY lamport DESC, created_ms DESC, id DESC LIMIT ?5"
                );
                let mut st = self.conn.prepare(&sql)?;
                let rows = st.query_map(
                    params![channel.0.to_vec(), lamport as i64, created, id.0.to_vec(), limit as i64],
                    message_row,
                )?;
                for r in rows { out.push(r??); }
            }
        }
        out.reverse();
        Ok(out)
    }
    pub fn message(&self, id: &EntryId) -> rusqlite::Result<Option<MessageRow>> {
        let sql = format!("SELECT {MESSAGE_COLS} FROM messages WHERE id = ?1");
        self.conn.query_row(&sql, params![id.0.to_vec()], message_row).optional()?.transpose()
    }

    /// (target, reaction code, how many, who) for reactions that are on.
    pub fn reactions_for(&self, targets: &[EntryId]) -> rusqlite::Result<Vec<(EntryId, String, u32, Vec<MemberKey>)>> {
        let mut out: Vec<(EntryId, String, u32, Vec<MemberKey>)> = Vec::new();
        let mut st = self
            .conn
            .prepare("SELECT emoji, member FROM reactions WHERE target = ?1 AND is_on = 1 ORDER BY emoji, member")?;
        for t in targets {
            let rows = st.query_map(params![t.0.to_vec()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
            let start = out.len();
            for r in rows {
                let (emoji, member) = r?;
                let member = MemberKey(arr(member)?);
                match out[start..].iter_mut().find(|x| x.1 == emoji) {
                    Some(x) => { x.2 += 1; x.3.push(member); }
                    None => out.push((*t, emoji, 1, vec![member])),
                }
            }
        }
        Ok(out)
    }

    pub fn mark_read(&self, channel: &ChannelId, lamport: u64) -> rusqlite::Result<()> {
        self.conn
            .execute(
                "INSERT INTO reads (channel, last_lamport) VALUES (?1, ?2) \
                 ON CONFLICT(channel) DO UPDATE SET last_lamport = MAX(last_lamport, excluded.last_lamport)",
                params![channel.0.to_vec(), lamport as i64],
            )
            .map(|_| ())
    }
    /// Local block list (never sent anywhere): block or unblock one member.
    pub fn block_set(&self, member: &MemberKey, on: bool, now_ms: u64) -> rusqlite::Result<()> {
        if on {
            self.conn.execute(
                "INSERT OR REPLACE INTO blocks (member, since_ms) VALUES (?1, ?2)",
                params![member.0.to_vec(), now_ms as i64],
            )?;
        } else {
            self.conn.execute("DELETE FROM blocks WHERE member = ?1", params![member.0.to_vec()])?;
        }
        Ok(())
    }
    pub fn is_blocked(&self, member: &MemberKey) -> bool {
        self.conn
            .query_row("SELECT 1 FROM blocks WHERE member = ?1", params![member.0.to_vec()], |_| Ok(()))
            .optional()
            .map(|r| r.is_some())
            .unwrap_or(false)
    }
    pub fn blocked(&self) -> rusqlite::Result<Vec<MemberKey>> {
        let mut st = self.conn.prepare("SELECT member FROM blocks ORDER BY since_ms")?;
        let rows = st.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
        Ok(rows.flatten().filter_map(|b| <[u8; 32]>::try_from(b).ok().map(MemberKey)).collect())
    }
    /// Messages after the last read point that are not mine and not deleted.
    pub fn unread(&self, channel: &ChannelId, me: &MemberKey) -> rusqlite::Result<u32> {
        let last: i64 = self
            .conn
            .query_row("SELECT last_lamport FROM reads WHERE channel = ?1", params![channel.0.to_vec()], |r| r.get(0))
            .optional()?
            .unwrap_or(0);
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE channel = ?1 AND lamport > ?2 AND member != ?3 AND deleted_by IS NULL",
            params![channel.0.to_vec(), last, me.0.to_vec()],
            |r| r.get(0),
        )?;
        Ok(n as u32)
    }
}
