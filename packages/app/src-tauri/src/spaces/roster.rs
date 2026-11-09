//! The control log: owner-signed, hash-linked ops. Replaying it from genesis gives the roster.
//!
//! Private-space format (founder 2026-11-07): members bring bots up to a number the owner sets, a bot's id belongs
//! to its owner's member key (the member signs the claim), and a bot's name is only a label that must be unique in
//! the space ignoring case and look-alike letters. This file holds only the signed format and the replay rules;
//! running a bot is somebody else's job.
use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::identity::{clean_name, DeviceCert, SpaceIdentity};
use super::ids::{BotId, ChannelId, DeviceKey, MemberKey, NodeKey, SpaceId};
use super::limits::*;
use super::wire::{Reader, WireError, Writer};
use crate::secure_core::grant::{open_sealed_key, seal_key_to, SealedKey};
use crate::secure_core::sign;

pub const CONTROL_DOMAIN: &[u8] = b"pm/spaces/control/v1";
/// A member signs "this bot is mine" with this domain; the owner then publishes it in an `AddBot` op.
pub const BOT_DOMAIN: &[u8] = b"pm/spaces/bot/v1";
/// A member signs their own new name with this domain (`SetName`).
pub const NAME_DOMAIN: &[u8] = b"pm/spaces/name/v1";
pub const MAGIC: &[u8; 4] = b"PMSC";
pub const MAX_MAILBOXES: usize = 2;
const MAX_BODY: usize = 60_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterError(pub String);
impl From<WireError> for RosterError {
    fn from(e: WireError) -> Self { RosterError(e.0.into()) }
}

fn fail<T>(text: &str) -> Result<T, RosterError> { Err(RosterError(text.into())) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role { Owner, Member }

/// Kept in Genesis so a public kind can be added later. Only `Private` is understood today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpaceKind { Private }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveReason { Removed, Left }

/// Channels where bots may speak.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BotChannels {
    All,
    /// Strictly ascending channel ids (one canonical encoding, so the signed bytes cannot differ).
    Only(Vec<ChannelId>),
}

/// How many bots each person may bring (0 to 10) and where they may speak.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BotRules {
    pub per_person: u32,
    pub channels: BotChannels,
}

impl Default for BotRules {
    fn default() -> Self { Self { per_person: BOTS_PER_PERSON_DEFAULT, channels: BotChannels::All } }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpBody {
    Genesis { name: String, kind: SpaceKind, owner: DeviceCert, channel: ChannelId, channel_name: String, grant: SealedKey },
    AddMember { cert: DeviceCert, epoch: u32, grant: SealedKey },
    RemoveMember { member: MemberKey, reason: RemoveReason },
    Rotate { epoch: u32, grants: Vec<SealedKey> },
    AddChannel { channel: ChannelId, name: String },
    RenameChannel { channel: ChannelId, name: String },
    ArchiveChannel { channel: ChannelId },
    RenameSpace { name: String },
    /// `sig` is the owning member's signature over `bot_claim_bytes`.
    AddBot { bot: BotId, member: MemberKey, name: String, sig: [u8; 64] },
    AddMailbox { node: NodeKey },
    RemoveMailbox { node: NodeKey },
    RemoveBot { bot: BotId },
    SetBotRules { rules: BotRules },
    /// A member renames themself. `sig` is that member's own signature over `name_claim_bytes`; the owner publishes it.
    /// Older builds do not know this op type and stop replaying the control log at it.
    SetName { member: MemberKey, name: String, sig: [u8; 64] },
}

const T_GENESIS: u8 = 1;
const T_ADD_MEMBER: u8 = 2;
const T_REMOVE_MEMBER: u8 = 3;
const T_ROTATE: u8 = 4;
const T_ADD_CHANNEL: u8 = 5;
const T_RENAME_CHANNEL: u8 = 6;
const T_ARCHIVE_CHANNEL: u8 = 7;
const T_RENAME_SPACE: u8 = 8;
const T_ADD_BOT: u8 = 9;
const T_ADD_MAILBOX: u8 = 10;
const T_REMOVE_MAILBOX: u8 = 11;
const T_REMOVE_BOT: u8 = 12;
const T_SET_BOT_RULES: u8 = 13;
const T_SET_NAME: u8 = 14;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlOp {
    pub space: SpaceId,
    pub seq: u64,
    pub prev: [u8; 32],
    pub created_ms: u64,
    pub body: OpBody,
    pub signer: MemberKey,
    pub sig: [u8; 64],
}

impl ControlOp {
    /// magic .. signer, per the layout table.
    pub fn signed_part(&self) -> Vec<u8> {
        let (t, body) = encode_body(&self.body);
        let mut w = Writer::new();
        w.fixed(MAGIC).u8(1).fixed(&self.space.0).u64(self.seq).fixed(&self.prev).u64(self.created_ms)
            .u8(t).blob(&body).fixed(&self.signer.0);
        w.finish()
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = self.signed_part();
        b.extend_from_slice(&self.sig);
        b
    }
    /// Rejects a wrong magic or version, an unknown op type and trailing bytes.
    pub fn from_bytes(b: &[u8]) -> Result<Self, RosterError> {
        let mut r = Reader::new(b);
        if r.fixed::<4>()? != *MAGIC { return fail("bad magic"); }
        if r.u8()? != 1 { return fail("bad version"); }
        let space = SpaceId(r.fixed()?);
        let seq = r.u64()?;
        let prev = r.fixed()?;
        let created_ms = r.u64()?;
        let t = r.u8()?;
        let body = decode_body(t, r.blob(MAX_BODY)?)?;
        let signer = MemberKey(r.fixed()?);
        let sig = r.fixed()?;
        r.done()?;
        Ok(Self { space, seq, prev, created_ms, body, signer, sig })
    }
    pub fn hash(&self) -> [u8; 32] { Sha256::digest(self.to_bytes()).into() }
    pub fn signature_ok(&self) -> bool {
        sign::verify(&self.signer.0, CONTROL_DOMAIN, &self.signed_part(), &self.sig)
    }
}

pub fn sealed_to_bytes(s: &SealedKey) -> Vec<u8> {
    let mut w = Writer::new();
    w.fixed(&s.device).blob(&s.enc).blob(&s.ct);
    w.finish()
}

pub fn sealed_from_bytes(b: &[u8]) -> Result<SealedKey, RosterError> {
    let mut r = Reader::new(b);
    let device = r.fixed()?;
    let enc = r.blob(64)?.to_vec();
    let ct = r.blob(128)?.to_vec();
    r.done()?;
    Ok(SealedKey { device, enc, ct })
}

pub fn encode_body(body: &OpBody) -> (u8, Vec<u8>) {
    let mut w = Writer::new();
    let t = match body {
        OpBody::Genesis { name, kind, owner, channel, channel_name, grant } => {
            let SpaceKind::Private = kind;
            w.str(name).u8(0).blob(&owner.to_bytes()).fixed(&channel.0).str(channel_name).blob(&sealed_to_bytes(grant));
            T_GENESIS
        }
        OpBody::AddMember { cert, epoch, grant } => {
            w.blob(&cert.to_bytes()).u32(*epoch).blob(&sealed_to_bytes(grant));
            T_ADD_MEMBER
        }
        OpBody::RemoveMember { member, reason } => {
            w.fixed(&member.0).u8(match reason { RemoveReason::Removed => 1, RemoveReason::Left => 2 });
            T_REMOVE_MEMBER
        }
        OpBody::Rotate { epoch, grants } => {
            w.u32(*epoch).u16(grants.len() as u16);
            for g in grants { w.blob(&sealed_to_bytes(g)); }
            T_ROTATE
        }
        OpBody::AddChannel { channel, name } => { w.fixed(&channel.0).str(name); T_ADD_CHANNEL }
        OpBody::RenameChannel { channel, name } => { w.fixed(&channel.0).str(name); T_RENAME_CHANNEL }
        OpBody::ArchiveChannel { channel } => { w.fixed(&channel.0); T_ARCHIVE_CHANNEL }
        OpBody::RenameSpace { name } => { w.str(name); T_RENAME_SPACE }
        OpBody::AddBot { bot, member, name, sig } => {
            w.fixed(&bot.0).fixed(&member.0).str(name).fixed(sig);
            T_ADD_BOT
        }
        OpBody::AddMailbox { node } => { w.fixed(&node.0); T_ADD_MAILBOX }
        OpBody::RemoveMailbox { node } => { w.fixed(&node.0); T_REMOVE_MAILBOX }
        OpBody::RemoveBot { bot } => { w.fixed(&bot.0); T_REMOVE_BOT }
        OpBody::SetBotRules { rules } => {
            w.u8(rules.per_person.min(255) as u8);
            match &rules.channels {
                BotChannels::All => { w.u8(0).u16(0); }
                BotChannels::Only(list) => {
                    w.u8(1).u16(list.len() as u16);
                    for c in list { w.fixed(&c.0); }
                }
            }
            T_SET_BOT_RULES
        }
        OpBody::SetName { member, name, sig } => { w.fixed(&member.0).str(name).fixed(sig); T_SET_NAME }
    };
    (t, w.finish())
}

pub fn decode_body(op_type: u8, body: &[u8]) -> Result<OpBody, RosterError> {
    let mut r = Reader::new(body);
    let out = match op_type {
        T_GENESIS => {
            let name = r.str(160)?;
            if r.u8()? != 0 { return fail("unsupported space kind"); }
            OpBody::Genesis {
                name,
                kind: SpaceKind::Private,
                owner: DeviceCert::from_bytes(r.blob(2048)?)?,
                channel: ChannelId(r.fixed()?),
                channel_name: r.str(64)?,
                grant: sealed_from_bytes(r.blob(256)?)?,
            }
        }
        T_ADD_MEMBER => OpBody::AddMember {
            cert: DeviceCert::from_bytes(r.blob(2048)?)?,
            epoch: r.u32()?,
            grant: sealed_from_bytes(r.blob(256)?)?,
        },
        T_REMOVE_MEMBER => {
            let member = MemberKey(r.fixed()?);
            let reason = match r.u8()? {
                1 => RemoveReason::Removed,
                2 => RemoveReason::Left,
                _ => return fail("bad reason"),
            };
            OpBody::RemoveMember { member, reason }
        }
        T_ROTATE => {
            let epoch = r.u32()?;
            let n = r.u16()? as usize;
            if n > MAX_MEMBERS * 8 { return fail("too many grants"); }
            let mut grants = Vec::with_capacity(n);
            for _ in 0..n { grants.push(sealed_from_bytes(r.blob(256)?)?); }
            OpBody::Rotate { epoch, grants }
        }
        T_ADD_CHANNEL => OpBody::AddChannel { channel: ChannelId(r.fixed()?), name: r.str(64)? },
        T_RENAME_CHANNEL => OpBody::RenameChannel { channel: ChannelId(r.fixed()?), name: r.str(64)? },
        T_ARCHIVE_CHANNEL => OpBody::ArchiveChannel { channel: ChannelId(r.fixed()?) },
        T_RENAME_SPACE => OpBody::RenameSpace { name: r.str(160)? },
        T_ADD_BOT => OpBody::AddBot {
            bot: BotId(r.fixed()?),
            member: MemberKey(r.fixed()?),
            name: r.str(160)?,
            sig: r.fixed()?,
        },
        T_ADD_MAILBOX => OpBody::AddMailbox { node: NodeKey(r.fixed()?) },
        T_REMOVE_MAILBOX => OpBody::RemoveMailbox { node: NodeKey(r.fixed()?) },
        T_REMOVE_BOT => OpBody::RemoveBot { bot: BotId(r.fixed()?) },
        T_SET_BOT_RULES => {
            let per_person = r.u8()? as u32;
            let mode = r.u8()?;
            let n = r.u16()? as usize;
            let channels = match mode {
                0 if n == 0 => BotChannels::All,
                1 if n <= MAX_CHANNELS_EVER => {
                    let mut list = Vec::with_capacity(n);
                    for _ in 0..n { list.push(ChannelId(r.fixed()?)); }
                    BotChannels::Only(list)
                }
                _ => return fail("bad bot rules"),
            };
            OpBody::SetBotRules { rules: BotRules { per_person, channels } }
        }
        T_SET_NAME => OpBody::SetName { member: MemberKey(r.fixed()?), name: r.str(160)?, sig: r.fixed()? },
        _ => return fail("unknown op type"),
    };
    r.done()?;
    Ok(out)
}

pub fn epoch_info(space: &SpaceId, epoch: u32) -> Vec<u8> {
    let mut v = b"pm/spaces/epoch/v1".to_vec();
    v.extend_from_slice(&space.0);
    v.extend_from_slice(&epoch.to_be_bytes());
    v
}

/// Channel names: lowercase, spaces become '-', only a-z 0-9 '-', 1..=32 chars, no leading/trailing '-'.
pub fn clean_channel_name(raw: &str) -> Option<String> {
    let s: String = raw.trim().to_lowercase().chars()
        .map(|c| if c.is_whitespace() { '-' } else { c })
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        .collect();
    let s = s.trim_matches('-').to_string();
    let s: String = s.chars().take(CHANNEL_NAME_MAX_CHARS).collect();
    if s.is_empty() { None } else { Some(s) }
}

/// A bot name is a label the owner types: already clean (same rules as a member's name) and not the empty fallback.
pub fn bot_name_ok(name: &str) -> bool { clean_name(name) == name && name != "Someone" }

fn fold_char(c: char) -> Option<char> {
    let c = match c as u32 {
        0xFF01..=0xFF5E => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c), // fullwidth letters and digits
        _ => c,
    };
    Some(match c {
        ' ' | '-' | '_' | '.' | '\'' | '\u{b7}' | '\u{2019}' => return None,
        _ => c,
    })
}

fn fold_lower(l: char) -> char {
    match l {
        'i' | '1' | '|' | '\u{131}' | '\u{456}' | '\u{4cf}' | '\u{3b9}' | '\u{ec}' | '\u{ed}' | '\u{ee}' | '\u{ef}'
        | '\u{142}' => 'l',
        '0' | '\u{43e}' | '\u{3bf}' | '\u{f2}' | '\u{f3}' | '\u{f4}' | '\u{f5}' | '\u{f6}' | '\u{f8}' => 'o',
        '\u{430}' | '\u{3b1}' | '\u{e0}' | '\u{e1}' | '\u{e2}' | '\u{e3}' | '\u{e4}' | '\u{e5}' => 'a',
        '3' | '\u{435}' | '\u{3b5}' | '\u{e8}' | '\u{e9}' | '\u{ea}' | '\u{eb}' => 'e',
        '\u{441}' | '\u{3f2}' | '\u{e7}' => 'c',
        '\u{440}' | '\u{3c1}' => 'p',
        '\u{445}' | '\u{3c7}' => 'x',
        '\u{443}' | '\u{fd}' | '\u{ff}' => 'y',
        '5' | '$' | '\u{455}' => 's',
        '\u{f9}' | '\u{fa}' | '\u{fb}' | '\u{fc}' | '\u{3c5}' => 'u',
        '\u{f1}' | '\u{3b7}' => 'n',
        '\u{3bd}' => 'v',
        '\u{3ba}' => 'k',
        '\u{4bb}' => 'h',
        '\u{458}' => 'j',
        '\u{501}' => 'd',
        '\u{261}' => 'g',
        '\u{51d}' => 'w',
        other => other,
    }
}

/// What two bot names must differ in to count as different: case, look-alike letters (Cyrillic and Greek twins,
/// accents, `0/o`, `1/l/i/|`, `5/s`, `3/e`, `rn/m`, `vv/w`), fullwidth forms, spaces and `-_.'` are all ignored.
pub fn bot_name_key(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars() {
        let Some(c) = fold_char(c) else { continue };
        for l in c.to_lowercase() { s.push(fold_lower(l)); }
    }
    s.replace("rn", "m").replace("vv", "w")
}

/// The bytes a member signs to claim a bot: space, bot id, member key, name.
pub fn bot_claim_bytes(space: &SpaceId, bot: &BotId, member: &MemberKey, name: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.fixed(&space.0).fixed(&bot.0).fixed(&member.0).str(name);
    w.finish()
}

/// The bytes a member signs to take a new name: space, member key, name.
pub fn name_claim_bytes(space: &SpaceId, member: &MemberKey, name: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.fixed(&space.0).fixed(&member.0).str(name);
    w.finish()
}

/// Builds a `SetName` body for `me` renaming themself. The owner of the space publishes it in a control op.
pub fn new_name_claim(me: &SpaceIdentity, space: &SpaceId, name: &str) -> OpBody {
    let member = me.member_key();
    let sig = me.sign_as_member(NAME_DOMAIN, &name_claim_bytes(space, &member, name));
    OpBody::SetName { member, name: name.to_string(), sig }
}

/// Builds an `AddBot` body for `me`'s own bot. The owner of the space publishes it in a control op.
pub fn new_bot_claim(me: &SpaceIdentity, space: &SpaceId, bot: BotId, name: &str) -> OpBody {
    let member = me.member_key();
    let sig = me.sign_as_member(BOT_DOMAIN, &bot_claim_bytes(space, &bot, &member, name));
    OpBody::AddBot { bot, member, name: name.to_string(), sig }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub member: MemberKey,
    pub name: String,
    pub role: Role,
    pub devices: Vec<DeviceCert>,
    pub joined_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel { pub id: ChannelId, pub name: String, pub archived: bool }

/// A bot on the roster. `member` owns it: the id was claimed by that member's signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bot { pub id: BotId, pub member: MemberKey, pub name: String, pub added_ms: u64 }

#[derive(Clone, Debug)]
pub struct Roster {
    pub space: SpaceId,
    pub name: String,
    pub kind: SpaceKind,
    pub owner: MemberKey,
    pub members: BTreeMap<MemberKey, Member>,
    pub channels: Vec<Channel>,
    pub epoch: u32,
    pub seq: u64,
    pub head: [u8; 32],
    pub mailboxes: Vec<NodeKey>,
    pub bots: BTreeMap<BotId, Bot>,
    /// Every bot id that was ever added, with its owner. Never shrinks: an old BotText is judged by this, not by
    /// the bots on the roster today.
    pub bot_owners: BTreeMap<BotId, MemberKey>,
    /// The last name every bot ever had, kept after the bot or its owner is removed, so old lines keep their name.
    pub bot_names: BTreeMap<BotId, String>,
    pub bot_rules: BotRules,
    /// Every grant ever published, by (epoch, device).
    pub grants: BTreeMap<(u32, DeviceKey), SealedKey>,
    /// Members removed or left, with the epoch that started right after.
    pub gone: BTreeMap<MemberKey, u32>,
    /// Devices of members who were removed or left: (member, first epoch they are no longer valid for).
    pub past: BTreeMap<DeviceKey, (MemberKey, u32)>,
    /// When (the owner's clock, from the removing op) each device in `past` was removed.
    pub past_ms: BTreeMap<DeviceKey, u64>,
    needs_rotate: bool,
}

impl Roster {
    /// Owner creates a space: genesis op + roster. `epoch_key` is the new epoch-1 key.
    pub fn genesis(owner: &SpaceIdentity, space: SpaceId, name: &str, owner_name: &str,
                   first_channel: ChannelId, epoch_key: &[u8; 32], now_ms: u64)
        -> Result<(Roster, ControlOp), RosterError>
    {
        let cert = owner.certify(&space, owner_name, now_ms);
        let grant = seal_key_to(cert.device.0, &cert.kem_b64, &epoch_info(&space, 1), epoch_key)
            .map_err(|_| RosterError("seal failed".into()))?;
        let mut op = ControlOp {
            space, seq: 1, prev: [0u8; 32], created_ms: now_ms,
            body: OpBody::Genesis {
                name: name.to_string(), kind: SpaceKind::Private, owner: cert,
                channel: first_channel, channel_name: "general".to_string(), grant,
            },
            signer: owner.member_key(), sig: [0u8; 64],
        };
        op.sig = owner.sign_as_member(CONTROL_DOMAIN, &op.signed_part());
        let r = Roster::from_genesis(space, &op)?;
        Ok((r, op))
    }

    /// Verify a whole log from genesis. Fails on the first bad op.
    pub fn replay(space: SpaceId, ops: &[ControlOp]) -> Result<Roster, RosterError> {
        let first = ops.first().ok_or_else(|| RosterError("empty log".into()))?;
        let mut r = Roster::from_genesis(space, first)?;
        for op in &ops[1..] { r.apply(op)?; }
        Ok(r)
    }

    fn from_genesis(space: SpaceId, op: &ControlOp) -> Result<Roster, RosterError> {
        if op.space != space { return fail("wrong space"); }
        if op.seq != 1 { return fail("wrong seq"); }
        if op.prev != [0u8; 32] { return fail("broken chain"); }
        let OpBody::Genesis { name, kind, owner, channel, channel_name, grant } = &op.body else {
            return fail("not a genesis");
        };
        if !op.signature_ok() { return fail("bad signature"); }
        if !owner.verify() || owner.space != space || owner.member != op.signer { return fail("bad cert"); }
        if clean_name(name) != *name || name == "Someone" { return fail("bad space name"); }
        if clean_channel_name(channel_name).as_deref() != Some("general") { return fail("bad channel name"); }
        if grant.device != owner.device.0 { return fail("grant mismatch"); }
        let mut members = BTreeMap::new();
        members.insert(op.signer, Member {
            member: op.signer, name: owner.name.clone(), role: Role::Owner,
            devices: vec![owner.clone()], joined_ms: op.created_ms,
        });
        let mut grants = BTreeMap::new();
        grants.insert((1, owner.device), grant.clone());
        Ok(Roster {
            space, name: name.clone(), kind: *kind, owner: op.signer, members,
            channels: vec![Channel { id: *channel, name: "general".into(), archived: false }],
            epoch: 1, seq: 1, head: op.hash(), mailboxes: Vec::new(),
            bots: BTreeMap::new(), bot_owners: BTreeMap::new(), bot_names: BTreeMap::new(), bot_rules: BotRules::default(),
            grants, gone: BTreeMap::new(), past: BTreeMap::new(), past_ms: BTreeMap::new(), needs_rotate: false,
        })
    }

    pub fn apply(&mut self, op: &ControlOp) -> Result<(), RosterError> {
        if op.space != self.space { return fail("wrong space"); }
        if op.seq != self.seq + 1 { return fail("wrong seq"); }
        if op.prev != self.head { return fail("broken chain"); }
        if op.signer != self.owner { return fail("not the owner"); }
        if !op.signature_ok() { return fail("bad signature"); }
        if self.needs_rotate && !matches!(op.body, OpBody::Rotate { .. }) { return fail("rotate first"); }
        self.apply_body(op)?;
        self.seq = op.seq;
        self.head = op.hash();
        Ok(())
    }

    /// Checks first, changes state only when every check passed.
    fn apply_body(&mut self, op: &ControlOp) -> Result<(), RosterError> {
        match &op.body {
            OpBody::Genesis { .. } => fail("second genesis"),
            OpBody::AddMember { cert, epoch, grant } => {
                if !cert.verify() || cert.space != self.space { return fail("bad cert"); }
                if self.members.contains_key(&cert.member) { return fail("already a member"); }
                if self.members.len() >= MAX_MEMBERS { return fail("space full"); }
                if *epoch != self.epoch { return fail("wrong epoch"); }
                if grant.device != cert.device.0 { return fail("grant mismatch"); }
                if self.device_by_node(&cert.node).is_some() { return fail("node in use"); }
                if self.device(&cert.device).is_some() { return fail("device in use"); }
                self.members.insert(cert.member, Member {
                    member: cert.member, name: cert.name.clone(), role: Role::Member,
                    devices: vec![cert.clone()], joined_ms: op.created_ms,
                });
                self.grants.insert((self.epoch, cert.device), grant.clone());
                Ok(())
            }
            OpBody::RemoveMember { member, .. } => {
                if *member == self.owner { return fail("cannot remove owner"); }
                if !self.members.contains_key(member) { return fail("not a member"); }
                if let Some(m) = self.members.remove(member) {
                    for c in &m.devices {
                        self.past.insert(c.device, (m.member, self.epoch + 1));
                        self.past_ms.insert(c.device, op.created_ms);
                    }
                }
                self.bots.retain(|_, b| b.member != *member);
                self.gone.insert(*member, self.epoch + 1);
                self.needs_rotate = true;
                Ok(())
            }
            OpBody::Rotate { epoch, grants } => {
                if *epoch != self.epoch + 1 { return fail("wrong epoch"); }
                let want: BTreeSet<[u8; 32]> = self.all_devices().iter().map(|c| c.device.0).collect();
                let got: BTreeSet<[u8; 32]> = grants.iter().map(|g| g.device).collect();
                if grants.len() != want.len() || got != want { return fail("grants do not match devices"); }
                for g in grants { self.grants.insert((*epoch, DeviceKey(g.device)), g.clone()); }
                self.epoch = *epoch;
                self.needs_rotate = false;
                Ok(())
            }
            OpBody::AddChannel { channel, name } => {
                if self.channel(channel).is_some() { return fail("channel exists"); }
                self.check_channel_name(name, None)?;
                if self.active_channels().len() >= MAX_CHANNELS || self.channels.len() >= MAX_CHANNELS_EVER {
                    return fail("too many channels");
                }
                self.channels.push(Channel { id: *channel, name: name.clone(), archived: false });
                Ok(())
            }
            OpBody::RenameChannel { channel, name } => {
                match self.channel(channel) {
                    Some(c) if !c.archived => {}
                    _ => return fail("no such channel"),
                }
                self.check_channel_name(name, Some(channel))?;
                if let Some(c) = self.channels.iter_mut().find(|c| c.id == *channel) { c.name = name.clone(); }
                Ok(())
            }
            OpBody::ArchiveChannel { channel } => {
                match self.channel(channel) {
                    Some(c) if !c.archived => {}
                    _ => return fail("no such channel"),
                }
                if self.channels.first().map(|c| c.id) == Some(*channel) {
                    return fail("cannot archive the first channel");
                }
                if let Some(c) = self.channels.iter_mut().find(|c| c.id == *channel) { c.archived = true; }
                Ok(())
            }
            OpBody::RenameSpace { name } => {
                if clean_name(name) != *name || name == "Someone" { return fail("bad space name"); }
                self.name = name.clone();
                Ok(())
            }
            OpBody::AddBot { bot, member, name, sig } => {
                if !bot_name_ok(name) { return fail("bad bot name"); }
                let claim = bot_claim_bytes(&self.space, bot, member, name);
                if !sign::verify(&member.0, BOT_DOMAIN, &claim, sig) { return fail("bad bot signature"); }
                if !self.members.contains_key(member) { return fail("not a member"); }
                if self.bots.contains_key(bot) { return fail("bot exists"); }
                // An id that was ever on the roster is never given out again: old lines are judged by its first owner.
                if self.bot_owners.contains_key(bot) { return fail("bot id used"); }
                let key = bot_name_key(name);
                if key.is_empty() || self.bots.values().any(|b| bot_name_key(&b.name) == key) {
                    return fail("bot name taken");
                }
                if self.bots_of(member).len() as u32 >= self.bot_rules.per_person { return fail("too many bots"); }
                self.bot_owners.insert(*bot, *member);
                self.bot_names.insert(*bot, name.clone());
                self.bots.insert(*bot, Bot { id: *bot, member: *member, name: name.clone(), added_ms: op.created_ms });
                Ok(())
            }
            OpBody::RemoveBot { bot } => {
                if self.bots.remove(bot).is_none() { return fail("no such bot"); }
                Ok(())
            }
            OpBody::SetBotRules { rules } => {
                if rules.per_person > BOTS_PER_PERSON_MAX { return fail("bad bot rules"); }
                if let BotChannels::Only(list) = &rules.channels {
                    if !list.windows(2).all(|w| w[0] < w[1]) { return fail("bad bot rules"); }
                    if list.iter().any(|c| self.channel(c).is_none()) { return fail("no such channel"); }
                }
                if self.members.keys().any(|m| self.bots_of(m).len() as u32 > rules.per_person) {
                    return fail("members have more bots");
                }
                self.bot_rules = rules.clone();
                Ok(())
            }
            OpBody::SetName { member, name, sig } => {
                if clean_name(name) != *name || name == "Someone" { return fail("bad member name"); }
                if !sign::verify(&member.0, NAME_DOMAIN, &name_claim_bytes(&self.space, member, name), sig) {
                    return fail("bad name signature");
                }
                match self.members.get_mut(member) {
                    Some(m) => { m.name = name.clone(); Ok(()) }
                    None => fail("not a member"),
                }
            }
            OpBody::AddMailbox { node } => {
                if self.mailboxes.contains(node) { return fail("mailbox exists"); }
                if self.mailboxes.len() >= MAX_MAILBOXES { return fail("too many mailboxes"); }
                if self.device_by_node(node).is_some() { return fail("node in use"); }
                self.mailboxes.push(*node);
                Ok(())
            }
            OpBody::RemoveMailbox { node } => {
                let Some(i) = self.mailboxes.iter().position(|n| n == node) else { return fail("no such mailbox") };
                self.mailboxes.remove(i);
                Ok(())
            }
        }
    }

    fn check_channel_name(&self, name: &str, except: Option<&ChannelId>) -> Result<(), RosterError> {
        if clean_channel_name(name).as_deref() != Some(name) { return fail("bad channel name"); }
        if self.channels.iter().any(|c| !c.archived && c.name == name && Some(&c.id) != except) {
            return fail("name taken");
        }
        Ok(())
    }

    /// Owner builds the next op; it is checked by applying it to a clone first.
    pub fn next_op(&self, owner: &SpaceIdentity, body: OpBody, now_ms: u64) -> Result<ControlOp, RosterError> {
        let mut op = ControlOp {
            space: self.space, seq: self.seq + 1, prev: self.head, created_ms: now_ms,
            body, signer: owner.member_key(), sig: [0u8; 64],
        };
        if encode_body(&op.body).1.len() > MAX_BODY { return fail("op too large"); }
        op.sig = owner.sign_as_member(CONTROL_DOMAIN, &op.signed_part());
        self.clone().apply(&op)?;
        Ok(op)
    }

    pub fn device(&self, d: &DeviceKey) -> Option<(&Member, &DeviceCert)> {
        self.members.values().find_map(|m| m.devices.iter().find(|c| c.device == *d).map(|c| (m, c)))
    }
    /// Author of a device: (member, None) if current; (member, Some(e)) if removed, valid only for epochs < e.
    pub fn author(&self, d: &DeviceKey) -> Option<(MemberKey, Option<u32>)> {
        if let Some((m, _)) = self.device(d) { return Some((m.member, None)); }
        self.past.get(d).map(|(m, e)| (*m, Some(*e)))
    }
    pub fn device_by_node(&self, n: &NodeKey) -> Option<(&Member, &DeviceCert)> {
        self.members.values().find_map(|m| m.devices.iter().find(|c| c.node == *n).map(|c| (m, c)))
    }
    pub fn all_devices(&self) -> Vec<&DeviceCert> {
        self.members.values().flat_map(|m| m.devices.iter()).collect()
    }
    /// Every device node of every current member.
    pub fn member_nodes(&self) -> Vec<NodeKey> { self.all_devices().iter().map(|c| c.node).collect() }
    pub fn active_channels(&self) -> Vec<&Channel> { self.channels.iter().filter(|c| !c.archived).collect() }
    pub fn channel(&self, id: &ChannelId) -> Option<&Channel> { self.channels.iter().find(|c| c.id == *id) }
    pub fn is_owner(&self, m: &MemberKey) -> bool { *m == self.owner }
    pub fn bots_of(&self, member: &MemberKey) -> Vec<&Bot> { self.bots.values().filter(|b| b.member == *member).collect() }
    /// May this bot post in this channel under the current rules? (The roster half of a `BotText` check.)
    pub fn bot_may_speak(&self, bot: &BotId, channel: &ChannelId) -> bool {
        let Some(b) = self.bots.get(bot) else { return false };
        if !self.members.contains_key(&b.member) { return false; }
        match self.channel(channel) {
            Some(c) if !c.archived => {}
            _ => return false,
        }
        match &self.bot_rules.channels {
            BotChannels::All => true,
            BotChannels::Only(list) => list.contains(channel),
        }
    }
    pub fn open_epoch_key(&self, me: &SpaceIdentity, epoch: u32) -> Option<[u8; 32]> {
        let g = self.grants.get(&(epoch, me.device_key()))?;
        open_sealed_key(g, me.kem(), &epoch_info(&self.space, epoch))
    }
    /// Grants of `key` for every current device (owner uses this for Rotate).
    pub fn grants_for_all(&self, epoch: u32, key: &[u8; 32]) -> Result<Vec<SealedKey>, RosterError> {
        self.all_devices().iter()
            .map(|c| seal_key_to(c.device.0, &c.kem_b64, &epoch_info(&self.space, epoch), key)
                .map_err(|_| RosterError("seal failed".into())))
            .collect()
    }
}
