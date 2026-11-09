//! Spaces service: owns open spaces, links, sync sessions and joins. One per app, started by `runtime.rs` only
//! while the Labs switch `spaces_enabled` is on. Never holds a lock across an `.await`.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use rusqlite::params;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use super::frame::{read_frame, read_frame_max, write_frame, Frame};
use super::identity::{clean_name, DeviceCert, SpaceIdentity};
use super::ids::{from_hex, space_tag, to_hex, BotId, ChannelId, DeviceKey, EntryId, InviteId, MemberKey, NodeKey, SpaceId};
use super::ingest::{ingest_entry, EpochKeys, Ingest};
use super::invite::{new_invite, qr_text, InviteRecord, InviteState, InviteTicket};
use super::join::{self, Answer, JoinHost, JoinOutcome, JoinRequestFrame, Lockouts, RefuseReason, Verdict};
use super::limits::*;
use super::links::{LinkTable, Offer, Outgoing};
use super::log::{clean_text, kind_of, Body, Entry, EntryHeader, KIND_BOT, KIND_BOT_CLAIM, KIND_BOT_DROP, KIND_DELETE, KIND_LEAVE, KIND_NAME_CLAIM, KIND_TEXT, MAX_MENTIONS, MAX_TEXT_BYTES, REACTIONS};
use super::moderation::remove_ops;
use super::net::{NetError, PeerHint, SpaceLink, SpaceNet, SpacePermissions, SpaceStream};
use super::ratelimit::{author_over_limit, LinkMeter, Meter, Over};
use super::retention;
use super::roster::{clean_channel_name, ControlOp, OpBody, RemoveReason, Role, Roster};
use super::store::{MessageRow, SpaceStore};
use super::sync::{self, Out, SyncSession, SyncView};
use super::words::{self, SECRET_BYTES};
use crate::secure_core::crypto::random_bytes;
use crate::spaces::keystore::KeyStore;

mod bots;
mod files;
mod names;
mod presence;
pub use bots::{BotPatch, BotPort};
use files::{file_value, FileJobs};
use super::presence::Presence;

pub type Notifier = Arc<dyn Fn(&Value) + Send + Sync>;
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// What create and join answer with when this PC has no Spaces display name yet; the screen asks for one first.
const NEEDS_NAME: &str = "needs_display_name";
const FIRST_FRAME_WAIT: Duration = Duration::from_secs(10);
const DIAL_EVERY: Duration = Duration::from_secs(30);
const HOUSEKEEPING_EVERY: Duration = Duration::from_secs(60);
/// Hints sent to a new member in the welcome (the owner's own plus the peers it knows).
const WELCOME_HINTS: usize = 12;
/// Prune again after this many entries were stored, and at least this often.
const PRUNE_EVERY_ENTRIES: usize = 1000;
const PRUNE_EVERY_MS: u64 = 6 * 60 * 60 * 1000;
/// A removed person's links are closed this long after the removal was queued, so the news reaches them first.
const REMOVED_CLOSE_DELAY: Duration = Duration::from_millis(300);
/// A stranger's link is closed this long after its join was answered, so the refusal reaches it first.
const REFUSED_CLOSE_DELAY: Duration = Duration::from_millis(500);
/// A person who left keeps dialing this long if the Leave could not be sent at once.
const LEAVE_DIAL_MS: u64 = 24 * 60 * 60 * 1000;

pub struct OpenSpace {
    pub id: SpaceId,
    pub store: SpaceStore,
    pub roster: Roster,
    pub keys: EpochKeys,
    pub me: Arc<SpaceIdentity>,
}

impl OpenSpace {
    fn is_owner(&self) -> bool { self.roster.is_owner(&self.me.member_key()) }
}

/// My place in a space: "active", "left" (I sent a Leave) or "removed" (the owner removed me).
fn space_state(sp: &OpenSpace) -> &'static str {
    match sp.store.meta_get("role").ok().flatten().as_deref() {
        Some(b"left") => "left",
        Some(b"removed") => "removed",
        _ if !sp.roster.members.contains_key(&sp.me.member_key()) => "removed",
        _ => "active",
    }
}

/// May this PC talk to others about the space? Active yes; left only while its Leave is still to be delivered.
fn dialable(sp: &OpenSpace, now: u64) -> bool {
    match space_state(sp) {
        "active" => true,
        "left" => sp.store.meta_u64("leave_until").unwrap_or(0) > now,
        _ => false,
    }
}

/// Owner side: a member's Leave entry is stored but their RemoveMember is not (yet).
fn leave_pending(sp: &OpenSpace) -> bool {
    if !sp.is_owner() { return false; }
    let Ok(mut st) = sp.store.conn.prepare("SELECT DISTINCT device FROM entries WHERE kind = ?1 AND gone = 0") else { return false };
    let Ok(rows) = st.query_map(params![KIND_LEAVE as i64], |r| r.get::<_, Vec<u8>>(0)) else { return false };
    let found = rows.flatten().any(|d| {
        <[u8; 32]>::try_from(d).ok()
            .and_then(|d| sp.roster.author(&DeviceKey(d)))
            .map(|(m, until)| until.is_none() && m != sp.roster.owner)
            .unwrap_or(false)
    });
    found
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoiningState {
    pub space_name: String,
    pub state: &'static str,
    pub tries_left: u8,
    pub answer_by_ms: Option<u64>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> { m.lock().unwrap_or_else(|p| p.into_inner()) }

struct View<'a> {
    sp: &'a OpenSpace,
    hint: PeerHint,
}

impl SyncView for View<'_> {
    fn space(&self) -> SpaceId { self.sp.id }
    fn control_seq(&self) -> u64 { self.sp.roster.seq }
    fn control_head(&self) -> [u8; 32] { self.sp.roster.head }
    fn control_hash_at(&self, seq: u64) -> Option<[u8; 32]> { self.sp.store.control_hash_at(seq).ok().flatten() }
    fn control_ops_after(&self, seq: u64) -> Vec<ControlOp> { self.sp.store.control_ops_after(seq).unwrap_or_default() }
    fn heads(&self) -> Vec<(DeviceKey, u64, u64)> { self.sp.store.heads().unwrap_or_default() }
    fn floor(&self, device: &DeviceKey) -> u64 { self.sp.store.author_floor(device).unwrap_or(0) }
    fn knows_device(&self, device: &DeviceKey) -> bool { self.sp.roster.author(device).is_some() }
    fn missing(&self, device: &DeviceKey, from: u64, to: u64, max: usize) -> Vec<u64> {
        self.sp.store.missing(device, from, to, max).unwrap_or_default()
    }
    fn entries_for(&self, device: &DeviceKey, seqs: &[u64]) -> (Vec<Vec<u8>>, Vec<u64>) {
        self.sp.store.entries_for(device, seqs).unwrap_or_default()
    }
    fn my_hint(&self) -> PeerHint { self.hint.clone() }
}

pub struct SpacesService {
    net: Arc<dyn SpaceNet>,
    perms: Arc<SpacePermissions>,
    keystore: KeyStore,
    dir: PathBuf,
    clock: Clock,
    me: Mutex<Option<Arc<SpaceIdentity>>>,
    spaces: Mutex<BTreeMap<SpaceId, OpenSpace>>,
    links: Mutex<LinkTable>,
    lockouts: Mutex<Lockouts>,
    joining: Mutex<BTreeMap<SpaceId, JoiningState>>,
    answers_tx: Mutex<BTreeMap<(SpaceId, i64), oneshot::Sender<Answer>>>,
    answers_rx: Mutex<BTreeMap<(SpaceId, i64), oneshot::Receiver<Answer>>>,
    notifier: Mutex<Option<Notifier>>,
    message_notifier: Mutex<Option<Notifier>>,
    /// A second listener beside the desktop's two: the phone server wakes its change feed. Set once by the runtime.
    observer: Mutex<Option<Notifier>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    started: AtomicBool,
    stopped: AtomicBool,
    next_id: AtomicU64,
    last_prune_ms: AtomicU64,
    stored_since_prune: AtomicUsize,
    /// `Push` frames written to any stream (tests count them to prove an entry is forwarded once).
    pushes_written: Arc<AtomicUsize>,
    /// Files being fetched (or whose fetch failed), by (space, file hash).
    file_jobs: Mutex<FileJobs>,
    /// Fresh presence reports per space. Memory only.
    presence: Mutex<BTreeMap<SpaceId, super::presence::PresenceTable>>,
    my_presence: Mutex<Presence>,
    show_online: AtomicBool,
    /// `Presence` frames written to any stream (tests count them to prove a hidden PC sends none).
    presence_written: Arc<AtomicUsize>,
    uploads: Arc<tokio::sync::Semaphore>,
    downloads: Arc<tokio::sync::Semaphore>,
    /// What runs this PC's bots (Potato Bot), when it is up. None: no line is handed to a bot.
    bots_port: Mutex<Option<Arc<dyn BotPort>>>,
    /// Lines handed to a bot and not answered yet, by delivery id.
    bot_pending: Mutex<BTreeMap<String, bots::Pending>>,
    /// Why the last line handed to one of my bots was refused by the one running it (off, no key, over its money limit,
    /// busy). Cleared when a later line is taken. The screen reads it so a call that nobody answers is explained.
    bot_refused: Mutex<BTreeMap<BotId, &'static str>>,
    /// My name in every space (kept in `settings.json`). None until the person picks one.
    display_name: Mutex<Option<String>>,
}

fn space_of(hex: &str) -> Result<SpaceId, String> { SpaceId::from_hex(hex).ok_or_else(|| "That space is not known.".to_string()) }

fn db(e: rusqlite::Error) -> String { format!("Could not save: {e}") }

impl SpacesService {
    pub fn new(net: Arc<dyn SpaceNet>, perms: Arc<SpacePermissions>, keystore: KeyStore, dir: PathBuf, clock: Clock)
        -> Arc<Self>
    {
        let show_online = presence::load_show_online(&dir);
        let display_name = presence::load_display_name(&dir);
        Arc::new(Self {
            net,
            perms,
            keystore,
            dir,
            clock,
            me: Mutex::new(None),
            spaces: Mutex::new(BTreeMap::new()),
            links: Mutex::new(LinkTable::new()),
            lockouts: Mutex::new(Lockouts::new()),
            joining: Mutex::new(BTreeMap::new()),
            answers_tx: Mutex::new(BTreeMap::new()),
            answers_rx: Mutex::new(BTreeMap::new()),
            notifier: Mutex::new(None),
            message_notifier: Mutex::new(None),
            observer: Mutex::new(None),
            tasks: Mutex::new(Vec::new()),
            started: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
            last_prune_ms: AtomicU64::new(0),
            stored_since_prune: AtomicUsize::new(0),
            pushes_written: Arc::new(AtomicUsize::new(0)),
            show_online: AtomicBool::new(show_online),
            presence: Mutex::new(BTreeMap::new()),
            my_presence: Mutex::new(Presence::Online),
            presence_written: Arc::new(AtomicUsize::new(0)),
            file_jobs: Mutex::new(BTreeMap::new()),
            uploads: Arc::new(tokio::sync::Semaphore::new(super::files::MAX_UPLOADS)),
            downloads: Arc::new(tokio::sync::Semaphore::new(super::files::MAX_DOWNLOADS)),
            bots_port: Mutex::new(None),
            bot_pending: Mutex::new(BTreeMap::new()),
            bot_refused: Mutex::new(BTreeMap::new()),
            display_name: Mutex::new(display_name),
        })
    }

    pub fn set_notifier(&self, n: Notifier) { *lock(&self.notifier) = Some(n); }
    pub fn set_message_notifier(&self, n: Notifier) { *lock(&self.message_notifier) = Some(n); }
    pub fn set_observer(&self, n: Notifier) { *lock(&self.observer) = Some(n); }

    fn now(&self) -> u64 { (self.clock)() }

    pub(crate) fn identity(&self) -> Result<Arc<SpaceIdentity>, String> {
        lock(&self.me).clone().ok_or_else(|| "Spaces is not running.".to_string())
    }

    fn my_hint(&self, fallback: NodeKey) -> PeerHint {
        self.net.my_hint().unwrap_or(PeerHint { node: fallback, relay: String::new(), lan: Vec::new() })
    }

    /// Opens every `dir/<hex>/space.sqlite`, replays control logs, loads epoch keys from the keystore, updates
    /// permissions, starts the net, spawns the accept loop, the dial loop (every 30 s) and the housekeeping loop
    /// (every 60 s).
    pub async fn start(self: &Arc<Self>) -> Result<(), String> { self.start_with(true).await }

    /// `loops = false` leaves the periodic dial and housekeeping loops off (tests drive `dial_round` and `tick`).
    pub(crate) async fn start_with(self: &Arc<Self>, loops: bool) -> Result<(), String> {
        self.net.start().await.map_err(|_| "Spaces could not reach the network.".to_string())?;
        let node = self.net.me().ok_or_else(|| "Spaces could not reach the network.".to_string())?;
        let me = Arc::new(SpaceIdentity::load(&self.keystore, node)?);
        *lock(&self.me) = Some(me);
        self.stopped.store(false, Ordering::SeqCst);
        self.started.store(true, Ordering::SeqCst);
        self.load_spaces();
        self.prune_all();
        self.refresh_permissions();
        let mut tasks = vec![tokio::spawn(self.clone().accept_loop())];
        if loops {
            tasks.push(tokio::spawn(self.clone().dial_loop()));
            tasks.push(tokio::spawn(self.clone().housekeeping_loop()));
            tasks.push(tokio::spawn(self.clone().presence_loop()));
        }
        lock(&self.tasks).extend(tasks);
        self.changed();
        Ok(())
    }

    pub async fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.started.store(false, Ordering::SeqCst);
        let tasks: Vec<_> = lock(&self.tasks).drain(..).collect();
        for t in tasks { t.abort(); }
        lock(&self.links).close_all();
        lock(&self.spaces).clear();
        *lock(&self.bots_port) = None;
        lock(&self.bot_pending).clear();
        lock(&self.bot_refused).clear();
        self.net.stop().await;
    }

    fn load_spaces(&self) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return };
        let Ok(me) = self.identity() else { return };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(id) = SpaceId::from_hex(&name) else { continue };
            let file = entry.path().join("space.sqlite");
            if !file.exists() { continue; }
            let Ok(store) = SpaceStore::open(&file) else { continue };
            if store.meta_get("role").ok().flatten().as_deref() == Some(b"joining") { continue; }
            let Ok(ops) = store.control_ops() else { continue };
            let Ok(roster) = Roster::replay(id, &ops) else { continue };
            let mut keys = EpochKeys::default();
            for e in 1..=roster.epoch {
                if let Ok(Some(k)) = self.keystore.epoch_key(&space_tag(&id), e as u64) { keys.0.insert(e, k); }
            }
            lock(&self.spaces).insert(id, OpenSpace { id, store, roster, keys, me: me.clone() });
        }
    }

    // ---- views -------------------------------------------------------------------------------------------------

    /// Runs `f` on one open space while the lock is held. Never call `.await` inside `f`.
    pub(crate) fn with_open<T>(&self, space: &SpaceId, f: impl FnOnce(&OpenSpace) -> T) -> Option<T> {
        lock(&self.spaces).get(space).map(f)
    }

    pub fn link_count(&self) -> usize { lock(&self.links).len() }
    pub fn pushes_written(&self) -> usize { self.pushes_written.load(Ordering::SeqCst) }
    pub fn presence_written(&self) -> usize { self.presence_written.load(Ordering::SeqCst) }

    /// `SpacesView` JSON, camelCase.
    pub fn view(&self) -> Value {
        let now = self.now();
        let linked = lock(&self.links).peers();
        let (mut spaces, mut requests, mut invites) = (Vec::new(), Vec::new(), Vec::new());
        for sp in lock(&self.spaces).values() {
            let me = sp.me.member_key();
            let unread: u32 = sp.roster.active_channels().iter().map(|c| sp.store.unread(&c.id, &me).unwrap_or(0)).sum();
            let online = sp.roster.member_nodes().iter().filter(|n| linked.contains(n)).count();
            spaces.push(json!({
                "id": sp.id.to_hex(), "name": sp.roster.name, "isOwner": sp.is_owner(), "unread": unread,
                "members": sp.roster.members.len(), "online": online,
            }));
            if !sp.is_owner() { continue; }
            for r in sp.store.join_requests_waiting().unwrap_or_default() {
                requests.push(json!({
                    "id": r.id, "space": sp.id.to_hex(), "spaceName": sp.roster.name, "name": r.name,
                    "short": r.joiner.short(), "receivedAtMs": r.received_ms, "answerByMs": r.answer_by_ms,
                }));
            }
            for i in sp.store.invites().unwrap_or_default() {
                let state = match i.state {
                    InviteState::Open | InviteState::Pending(_) if now < i.expires_ms => "open",
                    InviteState::Dead | InviteState::Open | InviteState::Pending(_) => "dead",
                    _ => continue,
                };
                invites.push(json!({
                    "id": i.id.to_hex(), "space": sp.id.to_hex(), "link": "", "words": [], "qr": "",
                    "expiresAtMs": i.expires_ms, "state": state,
                }));
            }
        }
        let joining: Vec<Value> = lock(&self.joining)
            .iter()
            .map(|(id, j)| json!({
                "space": id.to_hex(), "spaceName": j.space_name, "state": j.state, "triesLeft": j.tries_left,
                "answerByMs": j.answer_by_ms.unwrap_or(0),
            }))
            .collect();
        json!({
            "spaces": spaces, "requests": requests, "joining": joining, "invites": invites,
            "online": self.started.load(Ordering::SeqCst), "relayed": lock(&self.links).all_relayed(),
            "showOnline": self.show_online(), "displayName": self.display_name(),
        })
    }

    fn changed(&self) {
        let n = lock(&self.notifier).clone();
        if let Some(n) = n { n(&self.view()); }
        self.observe(&Value::Null);
    }

    fn observe(&self, v: &Value) {
        let o = lock(&self.observer).clone();
        if let Some(o) = o { o(v); }
    }

    /// `channel` is None when the entries of several channels arrived together (the UI then refetches).
    fn messages_changed(&self, space: &SpaceId, channel: Option<&ChannelId>) {
        let mut v = json!({ "space": space.to_hex() });
        if let Some(c) = channel { v["channel"] = json!(c.to_hex()); }
        let n = lock(&self.message_notifier).clone();
        if let Some(n) = n { n(&v); }
        self.observe(&v);
    }

    /// set_nodes(all member nodes + mailboxes), set_open_invites(n).
    fn refresh_permissions(&self) {
        let now = self.now();
        let (mut nodes, mut open) = (Vec::new(), 0usize);
        for sp in lock(&self.spaces).values() {
            nodes.extend(sp.roster.member_nodes());
            nodes.extend(sp.roster.mailboxes.iter().copied());
            if sp.is_owner() {
                open += sp.store.invites().unwrap_or_default().iter().filter(|i| i.is_open(now)).count();
            }
        }
        self.perms.set_nodes(nodes);
        self.perms.set_open_invites(open);
    }

    // ---- create, invite ----------------------------------------------------------------------------------------

    pub async fn create_space(&self, name: &str) -> Result<String, String> {
        // `clean_name` turns an empty name into "Someone", which a space may not use: an empty name is refused first.
        let name = clean_name(name);
        if name == "Someone" { return Err("Give the space a name.".into()); }
        let my_name = self.require_display_name()?;
        let me = self.identity()?;
        let (id, channel) = (SpaceId::random(), ChannelId::random());
        let key = random_bytes::<32>();
        let now = self.now();
        self.keystore.set_epoch_key(&space_tag(&id), 1, &key)?;
        let (roster, op) = Roster::genesis(&me, id, &name, &my_name, channel, &key, now).map_err(|e| e.0)?;
        let dir = self.dir.join(id.to_hex());
        std::fs::create_dir_all(&dir).map_err(|_| "Could not make a folder for the space.".to_string())?;
        let store = SpaceStore::open(&dir.join("space.sqlite")).map_err(db)?;
        store.control_append(&op).map_err(db)?;
        store.meta_set("role", b"owner").map_err(db)?;
        store.meta_set("space_id", id.to_hex().as_bytes()).map_err(db)?;
        store.meta_set("my_member", &me.member_key().0).map_err(db)?;
        store.meta_set("my_device", &me.device_key().0).map_err(db)?;
        store.meta_set("name_in_space", my_name.as_bytes()).map_err(db)?;
        let mut keys = EpochKeys::default();
        keys.0.insert(1, key);
        lock(&self.spaces).insert(id, OpenSpace { id, store, roster, keys, me });
        self.refresh_permissions();
        self.changed();
        Ok(id.to_hex())
    }

    pub async fn create_invite(&self, space: &str) -> Result<Value, String> {
        let id = space_of(space)?;
        let now = self.now();
        let me = self.identity()?;
        let hint = self.my_hint(me.node());
        let invite = {
            let spaces = lock(&self.spaces);
            let sp = spaces.get(&id).ok_or("That space is not known.")?;
            if !sp.is_owner() { return Err("Only the owner can invite people.".into()); }
            let open = sp.store.invites().map_err(db)?.iter().filter(|i| i.is_open(now)).count();
            if open >= MAX_OPEN_INVITES { return Err("Three invites are already open. Cancel one first.".into()); }
            let new = new_invite(id, me.node(), &hint.relay, &hint.lan, &sp.roster.name, now);
            self.keystore.set_invite_code(&space_tag(&id), &new.ticket.invite.to_hex(), &to_hex(&new.secret))?;
            sp.store.invite_put(&InviteRecord::new(new.ticket.invite, now)).map_err(db)?;
            new
        };
        self.refresh_permissions();
        self.changed();
        Ok(json!({
            "id": invite.ticket.invite.to_hex(), "space": id.to_hex(), "link": invite.ticket.to_link(),
            "words": words::encode(&invite.secret), "qr": qr_text(&invite.ticket, &invite.secret),
            "expiresAtMs": invite.ticket.expires_ms, "state": "open",
        }))
    }

    pub fn cancel_invite(&self, space: &str, invite: &str) -> Result<(), String> {
        let id = space_of(space)?;
        let inv = InviteId::from_hex(invite).ok_or("That invite is not known.")?;
        {
            let spaces = lock(&self.spaces);
            let sp = spaces.get(&id).ok_or("That space is not known.")?;
            if !sp.is_owner() { return Err("Only the owner can cancel an invite.".into()); }
            let mut rec = sp.store.invite_get(&inv).map_err(db)?.ok_or("That invite is not known.")?;
            rec.cancel();
            sp.store.invite_put(&rec).map_err(db)?;
        }
        let _ = self.keystore.forget_invite(&space_tag(&id), invite);
        self.refresh_permissions();
        self.changed();
        Ok(())
    }

    // ---- joining (joiner side) ---------------------------------------------------------------------------------

    pub async fn start_join(self: &Arc<Self>, text: &str, words_typed: &str) -> Result<(), String> {
        let (ticket, secret) = InviteTicket::parse(text).map_err(|_| "That is not a Spaces invite link.".to_string())?;
        let secret: [u8; SECRET_BYTES] = match secret {
            Some(s) => s,
            None => words::parse_typed(words_typed).map_err(|_| "Those 8 words are not right. Check them and try again.".to_string())?,
        };
        ticket.check_not_expired(self.now()).map_err(|_| "This invite has run out. Ask for a new one.".to_string())?;
        if lock(&self.spaces).contains_key(&ticket.space) { return Err("You are already in this space.".into()); }
        let my_name = self.require_display_name()?;
        let me = self.identity()?;
        lock(&self.joining).insert(
            ticket.space,
            JoiningState { space_name: ticket.space_name.clone(), state: "connecting", tries_left: 0, answer_by_ms: None },
        );
        self.changed();
        let svc = self.clone();
        tokio::spawn(async move { svc.run_join(ticket, secret, me, my_name).await });
        Ok(())
    }

    pub fn cancel_join(&self, space: &str) {
        if let Some(id) = SpaceId::from_hex(space) { lock(&self.joining).remove(&id); }
        self.changed();
    }

    fn set_joining(&self, space: SpaceId, state: &'static str, tries: u8, by: Option<u64>) {
        if let Some(j) = lock(&self.joining).get_mut(&space) {
            j.state = state;
            j.tries_left = tries;
            if by.is_some() { j.answer_by_ms = by; }
        }
        self.changed();
    }

    async fn run_join(self: Arc<Self>, ticket: InviteTicket, secret: [u8; SECRET_BYTES], me: Arc<SpaceIdentity>, my_name: String) {
        let space = ticket.space;
        let svc = self.clone();
        let on_wait = move |by: u64| svc.set_joining(space, "waiting", 0, Some(by));
        let outcome = join::run_joiner(&*self.net, &ticket, &secret, &me, &my_name, self.now(), None, &on_wait).await;
        match outcome {
            Ok(JoinOutcome::Joined { roster, epoch_key, hints, ops }) => {
                match self.finish_join(me, roster, epoch_key, hints, ops, &my_name) {
                    Ok(()) => {
                        self.set_joining(space, "joined", 0, None);
                        self.dial_round().await;
                    }
                    Err(_) => self.set_joining(space, "bad", 0, None),
                }
            }
            Ok(JoinOutcome::Refused(reason, left)) => {
                let state = match reason {
                    RefuseReason::WrongWords => "wrongWords",
                    RefuseReason::Gone => "gone",
                    RefuseReason::No => "no",
                    RefuseReason::LockedOut => "lockedOut",
                    RefuseReason::Busy => "busy",
                    RefuseReason::Full => "full",
                    RefuseReason::NoAnswer => "noAnswer",
                    RefuseReason::Bad => "bad",
                };
                self.set_joining(space, state, left, None);
            }
            Err(join::JoinError::BadWelcome(_)) => self.set_joining(space, "bad", 0, None),
            Err(_) => self.set_joining(space, "unreachable", 0, None),
        }
    }

    fn finish_join(&self, me: Arc<SpaceIdentity>, roster: Roster, epoch_key: [u8; 32], hints: Vec<PeerHint>, ops: Vec<ControlOp>, my_name: &str)
        -> Result<(), String>
    {
        let id = roster.space;
        let dir = self.dir.join(id.to_hex());
        std::fs::create_dir_all(&dir).map_err(|_| "Could not make a folder for the space.".to_string())?;
        let store = SpaceStore::open(&dir.join("space.sqlite")).map_err(db)?;
        for op in &ops { store.control_append(op).map_err(db)?; }
        store.meta_set("role", b"member").map_err(db)?;
        store.meta_set("space_id", id.to_hex().as_bytes()).map_err(db)?;
        store.meta_set("my_member", &me.member_key().0).map_err(db)?;
        store.meta_set("my_device", &me.device_key().0).map_err(db)?;
        store.meta_set("name_in_space", my_name.as_bytes()).map_err(db)?;
        let mut keys = EpochKeys::default();
        for e in 1..=roster.epoch {
            let key = if e == roster.epoch { Some(epoch_key) } else { roster.open_epoch_key(&me, e) };
            if let Some(k) = key {
                self.keystore.set_epoch_key(&space_tag(&id), e as u64, &k)?;
                keys.0.insert(e, k);
            }
        }
        for h in &hints { save_peer(&store, h, self.now()); }
        lock(&self.spaces).insert(id, OpenSpace { id, store, roster, keys, me });
        self.refresh_permissions();
        Ok(())
    }

    // ---- joining (owner side) ----------------------------------------------------------------------------------

    pub async fn answer_join(&self, space: &str, request: i64, allow: bool) -> Result<(), String> {
        let id = space_of(space)?;
        let now = self.now();
        let (answer, new_op) = {
            let mut spaces = lock(&self.spaces);
            let sp = spaces.get_mut(&id).ok_or("That space is not known.")?;
            if !sp.is_owner() { return Err("Only the owner can answer.".into()); }
            let row = sp.store.join_request(request).map_err(db)?.ok_or("That request is not known.")?;
            if row.state != "waiting" { return Err("That request was already answered.".into()); }
            let mut invite = sp.store.invite_get(&row.invite).map_err(db)?;
            let result = if allow {
                let cert = DeviceCert::from_bytes(&row.cert).map_err(|e| e.0.to_string())?;
                let key = sp.keys.0.get(&sp.roster.epoch).copied().ok_or("This computer does not have the key.")?;
                let op = join::owner_let_in(&sp.roster, &sp.me, &cert, &key, now).map_err(|e| e.0)?;
                // The op was checked on a copy by `owner_let_in`; disk first, then memory.
                if !sp.store.control_append(&op).map_err(db)? { return Err("Could not save: the log is out of step.".into()); }
                sp.roster.apply(&op).map_err(|e| e.0)?;
                sp.store.join_request_set(request, "let_in").map_err(db)?;
                let mut hints = vec![self.my_hint(sp.me.node())];
                hints.extend(peer_hints(&sp.store).into_iter().filter(|h| h.node != cert.node).take(WELCOME_HINTS - 1));
                let ops = sp.store.control_ops().map_err(db)?;
                (Answer::LetIn { ops, hints }, Some(op))
            } else {
                sp.store.join_request_set(request, "refused").map_err(db)?;
                (Answer::No, None)
            };
            if let Some(inv) = invite.as_mut() {
                inv.answered();
                sp.store.invite_put(inv).map_err(db)?;
                let _ = self.keystore.forget_invite(&space_tag(&id), &inv.id.to_hex());
            }
            result
        };
        if let Some(tx) = lock(&self.answers_tx).remove(&(id, request)) { let _ = tx.send(answer); }
        if let Some(op) = new_op {
            lock(&self.links).send_to_space(&id, None, sync::CONTROL_OPS, &sync::encode_ops(&[op]));
        }
        self.refresh_permissions();
        self.changed();
        Ok(())
    }

    // ---- sending -----------------------------------------------------------------------------------------------

    pub async fn send(&self, space: &SpaceId, channel: &ChannelId, body: Body, refers: Option<EntryId>)
        -> Result<EntryId, String>
    {
        self.send_with_bot(space, channel, body, refers, None).await
    }

    /// Like `send`, with the bot id for a `BotText`. The sender checks its own entry like any receiver does.
    pub async fn send_with_bot(&self, space: &SpaceId, channel: &ChannelId, body: Body, refers: Option<EntryId>,
                               bot: Option<BotId>) -> Result<EntryId, String>
    {
        self.send_now(space, channel, body, refers, bot)
    }

    /// The whole send, with nothing to wait for (a bot's answer comes from a thread that is not async).
    pub(crate) fn send_now(&self, space: &SpaceId, channel: &ChannelId, body: Body, refers: Option<EntryId>,
                           bot: Option<BotId>) -> Result<EntryId, String>
    {
        let body = match body {
            Body::Text { text, mentions } => {
                let text = clean_text(&text);
                if text.trim().is_empty() { return Err("Write something first.".into()); }
                Body::Text { text, mentions }
            }
            other => other,
        };
        let now = self.now();
        let (id, bytes) = {
            let spaces = lock(&self.spaces);
            let sp = spaces.get(space).ok_or("That space is not known.")?;
            match sp.roster.channel(channel) {
                Some(c) if !c.archived => {}
                _ => return Err("That channel is closed.".into()),
            }
            if space_state(sp) != "active" { return Err("You can't write in this space any more.".into()); }
            // Removal of a bot, the bot rules and a closed channel are checked here, not when history is received.
            if kind_of(&body) == KIND_BOT && !bot.is_some_and(|b| sp.roster.bot_may_speak(&b, channel)) {
                return Err("That bot may not speak here.".into());
            }
            match author_over_limit(&sp.store, &sp.me.device_key(), now).map_err(db)? {
                Over::Burst => return Err("You're sending messages too fast. Wait a few seconds.".into()),
                Over::Hour => return Err("You've sent a lot of messages this hour. Try again later.".into()),
                Over::No => {}
            }
            let epoch = sp.roster.epoch;
            let key = sp.keys.0.get(&epoch).ok_or("This computer does not have the key yet.")?;
            let device = sp.me.device_key();
            let header = EntryHeader {
                space: *space, channel: *channel, device, seq: sp.store.next_seq(&device).map_err(db)?,
                lamport: sp.store.next_lamport().map_err(db)?, created_ms: now, epoch, kind: kind_of(&body), refers,
                bot: if kind_of(&body) == KIND_BOT { bot } else { None },
            };
            let entry = Entry::seal(&sp.me, header, &body, key).map_err(|e| e.0.to_string())?;
            let bytes = entry.to_bytes();
            match ingest_entry(&sp.store, &sp.roster, &sp.keys, &bytes, now) {
                Ingest::Stored { id, .. } => (id, bytes),
                Ingest::Rejected(why) => return Err(format!("This message was not accepted: {why}.")),
                Ingest::Duplicate | Ingest::Old => return Err("This message was not accepted.".into()),
            }
        };
        self.forward(space, None, &bytes);
        self.note_stored(space, 1);
        self.messages_changed(space, Some(channel));
        self.changed();
        // This PC's bots hear its own lines too, and a bot's own answer goes into its diary.
        match kind_of(&body) {
            KIND_TEXT | KIND_BOT => self.after_stored(space, &[id], &[]),
            KIND_DELETE => if let Some(t) = refers { self.after_stored(space, &[], &[t]) },
            _ => {}
        }
        Ok(id)
    }

    /// Sends a `Push` for `bytes` to every live sync stream of the space except `from`.
    pub(crate) fn forward(&self, space: &SpaceId, from: Option<NodeKey>, bytes: &[u8]) -> usize {
        if let Out::Send(kind, body) = SyncSession::push_frame(bytes) {
            return lock(&self.links).send_to_space(space, from, kind, &body);
        }
        0
    }

    // ---- links -------------------------------------------------------------------------------------------------

    async fn accept_loop(self: Arc<Self>) {
        while let Some(link) = self.net.accept().await {
            if self.stopped.load(Ordering::SeqCst) { break; }
            self.adopt(link, false);
        }
    }

    /// Puts a link in the table (one per peer). Returns true when it was kept.
    fn adopt(self: &Arc<Self>, link: Arc<dyn SpaceLink>, dialed: bool) -> bool {
        let Some(me) = self.net.me() else { return false };
        let offer = lock(&self.links).offer(me, link.clone(), dialed);
        let Offer::Kept(id) = offer else { return false };
        let peer = link.peer();
        let svc = self.clone();
        let watched = link.clone();
        tokio::spawn(async move {
            watched.closed().await;
            lock(&svc.links).forget(&peer, id);
            svc.changed();
        });
        tokio::spawn(self.clone().serve_link(link));
        self.changed();
        true
    }

    async fn close_if_stranger(self: Arc<Self>, link: Arc<dyn SpaceLink>) {
        tokio::time::sleep(REFUSED_CLOSE_DELAY).await;
        let peer = link.peer();
        let member = lock(&self.spaces).values().any(|sp| sp.roster.device_by_node(&peer).is_some());
        if !member { link.close(0, "done"); }
    }

    async fn serve_link(self: Arc<Self>, link: Arc<dyn SpaceLink>) {
        while let Ok(stream) = link.accept_stream().await {
            tokio::spawn(self.clone().serve_first_frame(link.clone(), stream));
        }
    }

    async fn serve_first_frame(self: Arc<Self>, link: Arc<dyn SpaceLink>, mut stream: SpaceStream) {
        let now = self.now();
        let peer = link.peer();
        let first = match tokio::time::timeout(FIRST_FRAME_WAIT, read_frame_max(&mut stream.recv, FIRST_FRAME_MAX_BYTES)).await {
            Ok(Ok(f)) => f,
            _ => return link.close(2, "not allowed"),
        };
        match first.kind {
            sync::HELLO => {
                let asked = sync::decode_hello(&first.body).ok().map(|h| h.space);
                let member_of = asked.filter(|s| {
                    self.with_open(s, |sp| sp.roster.device_by_node(&peer).is_some() && dialable(sp, now)).unwrap_or(false)
                });
                match member_of {
                    Some(space) => self.run_sync(link, stream, space, Some(first)).await,
                    None => {
                        // Someone who is still in another space of mine keeps the link; only this stream is refused.
                        let shares = lock(&self.spaces).values().any(|sp| sp.roster.device_by_node(&peer).is_some());
                        if asked.is_some() && shares {
                            let _ = write_frame(&mut stream.send, sync::BYE, &[3]).await;
                        } else {
                            link.close(2, "not allowed");
                        }
                    }
                }
            }
            join::JOIN_REQUEST => {
                let open_invite = JoinRequestFrame::from_bytes(&first.body).ok().filter(|req| {
                    self.with_open(&req.space, |sp| {
                        sp.is_owner() && sp.store.invites().unwrap_or_default().iter().any(|i| i.is_open(now))
                    })
                    .unwrap_or(false)
                });
                match open_invite {
                    Some(req) => {
                        let host = ServiceJoinHost { svc: self.clone(), space: req.space };
                        let _ = join::serve_join(stream, peer, first, &host, now).await;
                        // A refused stranger keeps no link (it holds one of MAX_LINKS). A person who was let in
                        // is a member by now and keeps theirs.
                        tokio::spawn(self.clone().close_if_stranger(link));
                    }
                    None => link.close(2, "not allowed"),
                }
            }
            super::files::FILE_WANT => {
                let wanted = super::files::parse_want(&first.body).ok().filter(|(space, ..)| {
                    self.with_open(space, |sp| sp.roster.device_by_node(&peer).is_some() && dialable(sp, now)).unwrap_or(false)
                });
                match wanted {
                    Some((space, sha, from)) => self.serve_file_stream(stream, space, sha, from).await,
                    None => link.close(2, "not allowed"),
                }
            }
            _ => link.close(2, "not allowed"),
        }
    }

    // ---- sync --------------------------------------------------------------------------------------------------

    /// Runs `f` with a read view of one space (lock held, so `f` must not await). None if the space is gone.
    fn drive<T>(&self, space: &SpaceId, f: impl FnOnce(&dyn SyncView) -> T) -> Option<T> {
        let spaces = lock(&self.spaces);
        let sp = spaces.get(space)?;
        let v = View { sp, hint: self.my_hint(sp.me.node()) };
        Some(f(&v))
    }

    async fn run_sync(self: Arc<Self>, link: Arc<dyn SpaceLink>, stream: SpaceStream, space: SpaceId, first: Option<Frame>) {
        let peer = link.peer();
        let SpaceStream { mut send, mut recv } = stream;
        let (tx, mut rx) = mpsc::unbounded_channel::<Outgoing>();
        let pushes = self.pushes_written.clone();
        let presence_written = self.presence_written.clone();
        let writer = tokio::spawn(async move {
            while let Some((kind, body)) = rx.recv().await {
                if kind == sync::PUSH { pushes.fetch_add(1, Ordering::SeqCst); }
                if kind == sync::PRESENCE { presence_written.fetch_add(1, Ordering::SeqCst); }
                if write_frame(&mut send, kind, &body).await.is_err() { break; }
            }
            let _ = send.shutdown().await;
        });
        let mut session = SyncSession::new(peer);
        let writer_id = self.next_id.fetch_add(1, Ordering::SeqCst);
        // Hello is queued before the writer is registered, so no push can be written ahead of it.
        let Some(start) = self.drive(&space, |v| session.start(v)) else { return };
        self.exec(&space, peer, &tx, start);
        lock(&self.links).add_writer(space, peer, writer_id, tx.clone());
        // A new link hears my state at once instead of after the next 30 s tick.
        self.presence_tick();
        let mut pending = first;
        let mut told = false;
        let mut meter = LinkMeter::new();
        loop {
            let frame = match pending.take() {
                Some(f) => f,
                None => tokio::select! {
                    f = read_frame(&mut recv) => match f { Ok(f) => f, Err(_) => break },
                    _ = link.closed() => break,
                },
            };
            // A stream that floods frames is closed (the second layer after the per-author entry limit).
            if meter.frame(self.now()) == Meter::Close {
                let _ = tx.send((sync::BYE, vec![4]));
                break;
            }
            // The peer was removed (or left) while this stream was open: it gets no more of this space.
            if !self.with_open(&space, |sp| sp.roster.device_by_node(&peer).is_some()).unwrap_or(false) {
                let _ = tx.send((sync::BYE, vec![3]));
                break;
            }
            if frame.kind == sync::PRESENCE {
                self.on_presence(&space, peer, &frame.body);
                continue;
            }
            let Some(outs) = self.drive(&space, |v| session.on_frame(v, &frame)) else { break };
            if self.exec(&space, peer, &tx, outs).is_some() { break; }
            if !told {
                // The first answer from a peer: my Leave (if any) has been offered to it, so stop dialing for it.
                told = true;
                self.with_open(&space, |sp| {
                    if sp.store.meta_u64("leave_until").unwrap_or(0) > 0 { let _ = sp.store.meta_set_u64("leave_until", 0); }
                });
            }
            if matches!(frame.kind, sync::CONTROL_OPS | sync::ENTRIES | sync::PUSH) {
                let Some(more) = self.drive(&space, |v| session.next_round(v)) else { break };
                if self.exec(&space, peer, &tx, more).is_some() { break; }
            }
        }
        lock(&self.links).remove_writer(space, peer, writer_id);
        drop(tx);
        let _ = writer.await;
    }

    /// Carries out what a session decided. Returns the close code when the stream must end.
    fn exec(&self, space: &SpaceId, peer: NodeKey, tx: &mpsc::UnboundedSender<Outgoing>, outs: Vec<Out>) -> Option<u8> {
        for out in outs {
            match out {
                Out::Send(kind, body) => { let _ = tx.send((kind, body)); }
                Out::Ingest(list) => self.ingest_many(space, peer, list),
                Out::Gone(list) => {
                    if let Some(sp) = lock(&self.spaces).get(space) {
                        for (d, s) in list { let _ = sp.store.insert_gone_placeholder(&d, s); }
                    }
                }
                Out::ApplyControl(ops) => {
                    if !self.apply_control(space, peer, ops) {
                        let _ = tx.send((sync::BYE, vec![2]));
                        return Some(2);
                    }
                }
                Out::Hint(h) => {
                    if let Some(sp) = lock(&self.spaces).get(space) {
                        if h.node != sp.me.node() { save_peer(&sp.store, &h, self.now()); }
                    }
                }
                Out::Close(code) => {
                    if code != 0 { let _ = tx.send((sync::BYE, vec![code])); }
                    return Some(code);
                }
            }
        }
        None
    }

    fn ingest_many(&self, space: &SpaceId, from: NodeKey, list: Vec<Vec<u8>>) {
        let now = self.now();
        let (mut stored, mut shown) = (Vec::new(), false);
        let mut leavers: Vec<MemberKey> = Vec::new();
        let (mut lines, mut deletes): (Vec<EntryId>, Vec<EntryId>) = (Vec::new(), Vec::new());
        let mut asks: Vec<(DeviceKey, u64, Body)> = Vec::new();
        {
            let spaces = lock(&self.spaces);
            let Some(sp) = spaces.get(space) else { return };
            for bytes in list {
                // Only a Stored entry is forwarded: a Duplicate, Old or Rejected one stops here.
                if let Ingest::Stored { id: stored_id, shown: s } = ingest_entry(&sp.store, &sp.roster, &sp.keys, &bytes, now) {
                    shown |= s;
                    if let Ok(e) = Entry::from_bytes(&bytes) {
                        match e.header.kind {
                            KIND_TEXT | KIND_BOT if s => lines.push(stored_id),
                            KIND_DELETE => deletes.extend(e.header.refers),
                            KIND_BOT_CLAIM | KIND_BOT_DROP | KIND_NAME_CLAIM if sp.is_owner() => {
                                if let Some(body) = sp.keys.0.get(&e.header.epoch).and_then(|k| e.open(k).ok()) {
                                    asks.push((e.header.device, e.header.created_ms, body));
                                }
                            }
                            _ => {}
                        }
                    }
                    if sp.is_owner() {
                        if let Ok(e) = Entry::from_bytes(&bytes) {
                            if e.header.kind == KIND_LEAVE {
                                if let Some((m, None)) = sp.roster.author(&e.header.device) {
                                    if m != sp.roster.owner && !leavers.contains(&m) { leavers.push(m); }
                                }
                            }
                        }
                    }
                    stored.push(bytes);
                }
            }
        }
        for bytes in &stored { self.forward(space, Some(from), bytes); }
        self.note_stored(space, stored.len());
        if shown {
            self.messages_changed(space, None);
            self.changed();
        }
        self.after_stored(space, &lines, &deletes);
        let (name_asks, asks): (Vec<_>, Vec<_>) = asks.into_iter().partition(|a| matches!(a.2, Body::NameClaim { .. }));
        if !asks.is_empty() { self.owner_bot_asks(space, asks); }
        if !name_asks.is_empty() {
            let list = name_asks.into_iter().filter_map(|(d, at, b)| match b { Body::NameClaim { name, sig } => Some((d, at, name, sig)), _ => None }).collect();
            self.owner_name_asks(space, list);
        }
        // The owner's PC answers a stored Leave at once: RemoveMember + Rotate, no question asked.
        for m in leavers { let _ = self.remove_locked(space, &m, RemoveReason::Left); }
    }

    /// Applies control ops in order. False = a bad op (fork, wrong signature, wrong order): the stream ends.
    pub(crate) fn apply_control(&self, space: &SpaceId, from: NodeKey, ops: Vec<ControlOp>) -> bool {
        let mut applied = Vec::new();
        let mut removed_me = false;
        {
            let mut spaces = lock(&self.spaces);
            let Some(sp) = spaces.get_mut(space) else { return false };
            let was_in = sp.roster.members.contains_key(&sp.me.member_key());
            for op in ops {
                if op.seq <= sp.roster.seq {
                    // Already have it: it must be the same op.
                    if sp.store.control_hash_at(op.seq).ok().flatten() == Some(op.hash()) { continue; }
                    return false;
                }
                // Checked on a copy, written to disk, and only then taken into memory: a failed write leaves the
                // roster as it was.
                let mut next = sp.roster.clone();
                if next.apply(&op).is_err() { return false; }
                if !matches!(sp.store.control_append(&op), Ok(true)) { return false; }
                sp.roster = next;
                applied.push(op);
            }
            for e in 1..=sp.roster.epoch {
                if sp.keys.0.contains_key(&e) { continue; }
                if let Some(k) = sp.roster.open_epoch_key(&sp.me, e) {
                    let _ = self.keystore.set_epoch_key(&space_tag(space), e as u64, &k);
                    sp.keys.0.insert(e, k);
                }
            }
            if was_in && !sp.roster.members.contains_key(&sp.me.member_key()) {
                removed_me = true;
                if space_state(sp) == "active" { let _ = sp.store.meta_set("role", b"removed"); }
            }
        }
        if removed_me { lock(&self.links).drop_space_writers(space); }
        if !applied.is_empty() {
            // Pass them on to the other members this PC is linked with.
            lock(&self.links).send_to_space(space, Some(from), sync::CONTROL_OPS, &sync::encode_ops(&applied));
            self.refresh_permissions();
            self.bots_roster_changed(space);
            self.changed();
        }
        true
    }

    // ---- dialing -----------------------------------------------------------------------------------------------

    async fn dial_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(DIAL_EVERY);
        loop {
            tick.tick().await;
            self.dial_round().await;
        }
    }

    async fn housekeeping_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(HOUSEKEEPING_EVERY);
        loop {
            tick.tick().await;
            self.tick();
        }
    }

    /// Member nodes of my spaces except mine: the owner first, then the most recently seen.
    fn dial_candidates(&self, me: NodeKey) -> Vec<PeerHint> {
        let mut out: Vec<(bool, u64, PeerHint)> = Vec::new();
        let now = self.now();
        for sp in lock(&self.spaces).values() {
            if !dialable(sp, now) { continue; }
            let seen = peer_rows(&sp.store);
            for m in sp.roster.members.values() {
                for cert in &m.devices {
                    if cert.node == me || out.iter().any(|(_, _, h)| h.node == cert.node) { continue; }
                    let row = seen.iter().find(|(h, _)| h.node == cert.node);
                    let hint = row.map(|(h, _)| h.clone()).unwrap_or(PeerHint { node: cert.node, relay: String::new(), lan: Vec::new() });
                    out.push((sp.roster.is_owner(&m.member), row.map(|(_, t)| *t).unwrap_or(0), hint));
                }
            }
        }
        out.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        out.into_iter().map(|(_, _, h)| h).collect()
    }

    /// One pass over the people I am not linked to. Failed nodes wait 5 s, doubling, at most 5 minutes.
    pub async fn dial_round(self: &Arc<Self>) {
        if self.stopped.load(Ordering::SeqCst) { return; }
        let Some(me) = self.net.me() else { return };
        let now = self.now();
        let mut tries = Vec::new();
        for hint in self.dial_candidates(me) {
            {
                let links = lock(&self.links);
                if links.full() { break; }
                if links.has(&hint.node) || !links.may_dial(&hint.node, now) { continue; }
            }
            let svc = self.clone();
            tries.push(tokio::spawn(async move {
                match svc.net.dial(&hint).await {
                    Ok(link) => {
                        lock(&svc.links).dial_ok(&hint.node);
                        if svc.adopt(link.clone(), true) { svc.sync_all(link).await; }
                    }
                    Err(_) => lock(&svc.links).dial_failed(hint.node, svc.now()),
                }
            }));
        }
        for t in tries { let _ = t.await; }
    }

    /// One stream per space that both of us are in.
    async fn sync_all(self: &Arc<Self>, link: Arc<dyn SpaceLink>) {
        let peer = link.peer();
        let now = self.now();
        let common: Vec<SpaceId> = lock(&self.spaces)
            .values()
            .filter(|sp| sp.roster.device_by_node(&peer).is_some() && dialable(sp, now))
            .map(|sp| sp.id)
            .collect();
        for space in common {
            match link.open().await {
                Ok(stream) => { tokio::spawn(self.clone().run_sync(link.clone(), stream, space, None)); }
                Err(NetError::Closed) | Err(_) => break,
            }
        }
    }

    // ---- housekeeping ------------------------------------------------------------------------------------------

    /// Every 60 s: invites past their time die; join requests nobody answered in time expire.
    pub fn tick(&self) {
        let now = self.now();
        let mut expired = Vec::new();
        for sp in lock(&self.spaces).values() {
            if !sp.is_owner() { continue; }
            for mut rec in sp.store.invites().unwrap_or_default() {
                if matches!(rec.state, InviteState::Open | InviteState::Pending(_)) && now >= rec.expires_ms {
                    rec.timed_out();
                    let _ = sp.store.invite_put(&rec);
                    let _ = self.keystore.forget_invite(&space_tag(&sp.id), &rec.id.to_hex());
                }
            }
            for jr in sp.store.join_requests_waiting().unwrap_or_default() {
                if now < jr.answer_by_ms { continue; }
                let _ = sp.store.join_request_set(jr.id, "expired");
                if let Ok(Some(mut inv)) = sp.store.invite_get(&jr.invite) {
                    inv.timed_out();
                    let _ = sp.store.invite_put(&inv);
                    let _ = self.keystore.forget_invite(&space_tag(&sp.id), &inv.id.to_hex());
                }
                expired.push((sp.id, jr.id));
            }
        }
        for key in expired {
            if let Some(tx) = lock(&self.answers_tx).remove(&key) { let _ = tx.send(Answer::NoAnswer); }
        }
        if now.saturating_sub(self.last_prune_ms.load(Ordering::SeqCst)) >= PRUNE_EVERY_MS { self.prune_all(); }
        self.refresh_permissions();
        self.changed();
    }

    // ---- retention (90 days or 200 MB) -------------------------------------------------------------------------

    fn prune_one(&self, sp: &OpenSpace, now: u64) {
        let files = self.dir.join(sp.id.to_hex()).join("files");
        let _ = retention::prune(&sp.store, now, &|sha| { let _ = std::fs::remove_file(files.join(to_hex(sha))); });
        // Bot caps and handled-line marks that can no longer matter.
        self.bots_prune(sp, now);
    }

    /// At start and every 6 hours.
    pub(crate) fn prune_all(&self) {
        let now = self.now();
        self.last_prune_ms.store(now, Ordering::SeqCst);
        self.stored_since_prune.store(0, Ordering::SeqCst);
        for sp in lock(&self.spaces).values() { self.prune_one(sp, now); }
    }

    // ---- detail, messages, send, read marks, channels (D4.1) ---------------------------------------------------

    /// `SpaceDetail` JSON for one space.
    pub fn detail(&self, space: &str) -> Result<Value, String> {
        let id = space_of(space)?;
        let now = self.now();
        let linked = lock(&self.links).peers();
        let relayed = lock(&self.links).all_relayed();
        self.with_open(&id, |sp| {
            let me = sp.me.member_key();
            let channels: Vec<Value> = sp.roster.channels.iter().map(|c| json!({
                "id": c.id.to_hex(), "name": c.name, "archived": c.archived,
                "unread": sp.store.unread(&c.id, &me).unwrap_or(0),
            })).collect();
            let members: Vec<Value> = sp.roster.members.values().map(|m| {
                let h = m.member.to_hex().to_uppercase();
                let is_me = m.member == me;
                json!({
                    "member": m.member.to_hex(), "name": m.name,
                    "role": match m.role { Role::Owner => "owner", Role::Member => "member" },
                    "presence": if is_me { "online" } else { self.presence_of(&id, &m.member, now) },
                    "blocked": !is_me && sp.store.is_blocked(&m.member), "isMe": is_me, "short": format!("{}-{}", &h[0..4], &h[4..8]),
                })
            }).collect();
            let open_invites = if sp.is_owner() {
                sp.store.invites().unwrap_or_default().iter().filter(|i| i.is_open(now)).count()
            } else {
                0
            };
            let any_online = sp.roster.member_nodes().iter().any(|n| linked.contains(n));
            let owner_name = sp.roster.members.get(&sp.roster.owner).map(|m| m.name.clone()).unwrap_or_default();
            let (bots, bot_rules) = self.bots_in_detail(sp);
            json!({
                "bots": bots, "botRules": bot_rules,
                "id": id.to_hex(), "name": sp.roster.name, "isOwner": sp.is_owner(), "myMember": me.to_hex(),
                "ownerName": owner_name, "channels": channels, "members": members, "openInvites": open_invites,
                "relayed": relayed, "anyoneOnline": any_online,
                "state": space_state(sp), "leavePending": leave_pending(sp),
            })
        })
        .ok_or_else(|| "That space is not known.".to_string())
    }

    /// Newest `limit` messages before `before` (a message id), oldest first. `limit` is clamped to 1..=100.
    pub fn messages(&self, space: &str, channel: &str, before: Option<&str>, limit: u32) -> Result<Value, String> {
        let id = space_of(space)?;
        let ch = channel_of(channel)?;
        let limit = limit.clamp(1, 100) as usize;
        let before = before.map(|b| EntryId::from_hex(b).ok_or("That message is not known.")).transpose()?;
        let jobs = lock(&self.file_jobs).clone();
        self.with_open(&id, |sp| -> Result<Value, String> {
            if sp.roster.channel(&ch).is_none() { return Err("That channel is not known.".into()); }
            let cursor = match before {
                Some(b) => {
                    let row = sp.store.message(&b).map_err(db)?.ok_or("That message is not known.")?;
                    Some((row.lamport, b))
                }
                None => None,
            };
            let mut rows = sp.store.messages_page(&ch, cursor, limit).map_err(db)?;
            // A bot the viewer muted on this PC is not shown.
            let muted = self.muted_bots(sp);
            rows.retain(|r| !r.bot.is_some_and(|b| muted.contains(&b)));
            let me = sp.me.member_key();
            let ids: Vec<EntryId> = rows.iter().map(|r| r.id).collect();
            let reacts = sp.store.reactions_for(&ids).map_err(db)?;
            Ok(Value::Array(rows.iter().map(|r| message_json(sp, r, &reacts, &me, &jobs, &self.dir.join(id.to_hex()))).collect()))
        })
        .unwrap_or_else(|| Err("That space is not known.".to_string()))
    }

    /// Stores a text message from me and returns its `SpaceMessage` JSON.
    pub async fn send_text(&self, space: &str, channel: &str, text: &str, reply: Option<&str>) -> Result<Value, String> {
        let id = space_of(space)?;
        let ch = channel_of(channel)?;
        if text.trim().is_empty() { return Err("Write something first.".into()); }
        if text.chars().count() > MAX_TEXT_CHARS || text.len() > MAX_TEXT_BYTES {
            return Err("This message is too long. The limit is 4,000 characters.".into());
        }
        let cleaned = clean_text(text);
        if cleaned.trim().is_empty() { return Err("Write something first.".into()); }
        let reply = reply.and_then(|r| EntryId::from_hex(r));
        let (refers, mentions) = self.with_open(&id, |sp| -> Result<(Option<EntryId>, Vec<MemberKey>), String> {
            match sp.roster.channel(&ch) {
                None => return Err("That channel is not known.".into()),
                Some(c) if c.archived => return Err("This channel is archived.".into()),
                Some(_) => {}
            }
            let refers = reply.filter(|r| sp.store.message(r).ok().flatten().is_some());
            let mut mentions: Vec<MemberKey> = Vec::new();
            for word in cleaned.split_whitespace() {
                let Some(name) = word.strip_prefix('@') else { continue };
                if let Some(m) = sp.roster.members.values().find(|m| m.name.eq_ignore_ascii_case(name)) {
                    if !mentions.contains(&m.member) && mentions.len() < MAX_MENTIONS { mentions.push(m.member); }
                }
            }
            Ok((refers, mentions))
        })
        .ok_or_else(|| "That space is not known.".to_string())??;
        let entry = self.send(&id, &ch, Body::Text { text: cleaned, mentions }, refers).await?;
        let jobs = lock(&self.file_jobs).clone();
        self.with_open(&id, |sp| {
            let row = sp.store.message(&entry).ok().flatten()?;
            let reacts = sp.store.reactions_for(&[entry]).unwrap_or_default();
            Some(message_json(sp, &row, &reacts, &sp.me.member_key(), &jobs, &self.dir.join(id.to_hex())))
        })
        .flatten()
        .ok_or_else(|| "That message was not saved.".to_string())
    }

    /// `SpaceMessage` JSON for one stored message.
    fn message_value(&self, id: &SpaceId, entry: EntryId) -> Result<Value, String> {
        let jobs = lock(&self.file_jobs).clone();
        self.with_open(id, |sp| {
            let row = sp.store.message(&entry).ok().flatten()?;
            let reacts = sp.store.reactions_for(&[entry]).unwrap_or_default();
            Some(message_json(sp, &row, &reacts, &sp.me.member_key(), &jobs, &self.dir.join(id.to_hex())))
        })
        .flatten()
        .ok_or_else(|| "That message is gone.".to_string())
    }

    /// React to a shown message, or take my reaction back (`on` false). Returns the updated message.
    pub async fn react(&self, space: &str, target: &str, code: &str, on: bool) -> Result<Value, String> {
        let id = space_of(space)?;
        if !REACTIONS.contains(&code) { return Err("That reaction isn't available.".into()); }
        let target = EntryId::from_hex(target).ok_or("That message is gone.")?;
        let channel = self
            .with_open(&id, |sp| match sp.store.message(&target) {
                Ok(Some(row)) if row.deleted_by.is_none() => Ok(row.channel),
                _ => Err("That message is gone.".to_string()),
            })
            .ok_or("That space is not known.")??;
        self.send(&id, &channel, Body::React { code: code.to_string(), on }, Some(target)).await?;
        self.message_value(&id, target)
    }

    /// Delete a message: its author may, and so may the owner. Returns the message (now deleted).
    pub async fn delete(&self, space: &str, target: &str) -> Result<Value, String> {
        let id = space_of(space)?;
        let target = EntryId::from_hex(target).ok_or("That message is gone.")?;
        let channel = self
            .with_open(&id, |sp| match sp.store.message(&target) {
                Ok(Some(row)) if row.deleted_by.is_some() => Err("That message is gone.".to_string()),
                Ok(Some(row)) if row.member == sp.me.member_key() || sp.is_owner() => Ok(row.channel),
                Ok(Some(_)) => Err("You can only delete your own messages.".to_string()),
                _ => Err("That message is gone.".to_string()),
            })
            .ok_or("That space is not known.")??;
        self.send(&id, &channel, Body::Delete, Some(target)).await?;
        self.message_value(&id, target)
    }

    /// Marks the channel read up to its newest message, then sends the new unread counts.
    pub fn mark_read(&self, space: &str, channel: &str) -> Result<(), String> {
        let id = space_of(space)?;
        let ch = channel_of(channel)?;
        self.with_open(&id, |sp| -> Result<(), String> {
            if sp.roster.channel(&ch).is_none() { return Err("That channel is not known.".into()); }
            let newest = sp.store.messages_page(&ch, None, 1).map_err(db)?;
            if let Some(row) = newest.last() { sp.store.mark_read(&ch, row.lamport).map_err(db)?; }
            Ok(())
        })
        .ok_or_else(|| "That space is not known.".to_string())??;
        self.changed();
        Ok(())
    }

    /// Owner adds a channel. Returns `SpaceDetail` JSON.
    pub async fn channel_create(&self, space: &str, name: &str) -> Result<Value, String> {
        let id = space_of(space)?;
        let name = clean_channel_name(name).ok_or("Use letters, numbers and dashes.")?;
        let now = self.now();
        let op = {
            let mut spaces = lock(&self.spaces);
            let sp = spaces.get_mut(&id).ok_or("That space is not known.")?;
            if !sp.is_owner() { return Err("Only the owner can add channels.".into()); }
            if sp.roster.active_channels().len() >= MAX_CHANNELS {
                return Err(format!("This space has {MAX_CHANNELS} channels. Archive one first."));
            }
            if sp.roster.active_channels().iter().any(|c| c.name == name) {
                return Err(format!("There is already a channel called #{name}."));
            }
            owner_op(sp, OpBody::AddChannel { channel: ChannelId::random(), name }, now)?
        };
        self.push_control(&id, &op);
        self.changed();
        self.detail(space)
    }

    /// Owner renames a channel. Returns `SpaceDetail` JSON.
    pub async fn channel_rename(&self, space: &str, channel: &str, name: &str) -> Result<Value, String> {
        let id = space_of(space)?;
        let ch = channel_of(channel)?;
        let name = clean_channel_name(name).ok_or("Use letters, numbers and dashes.")?;
        let now = self.now();
        let op = {
            let mut spaces = lock(&self.spaces);
            let sp = spaces.get_mut(&id).ok_or("That space is not known.")?;
            if !sp.is_owner() { return Err("Only the owner can rename channels.".into()); }
            match sp.roster.channel(&ch) {
                None => return Err("That channel is not known.".into()),
                Some(c) if c.archived => return Err("This channel is archived.".into()),
                Some(_) => {}
            }
            if sp.roster.active_channels().iter().any(|c| c.name == name && c.id != ch) {
                return Err(format!("There is already a channel called #{name}."));
            }
            owner_op(sp, OpBody::RenameChannel { channel: ch, name }, now)?
        };
        self.push_control(&id, &op);
        self.changed();
        self.detail(space)
    }

    /// Owner archives a channel. The first channel cannot be archived. Returns `SpaceDetail` JSON.
    pub async fn channel_archive(&self, space: &str, channel: &str) -> Result<Value, String> {
        let id = space_of(space)?;
        let ch = channel_of(channel)?;
        let now = self.now();
        let op = {
            let mut spaces = lock(&self.spaces);
            let sp = spaces.get_mut(&id).ok_or("That space is not known.")?;
            if !sp.is_owner() { return Err("Only the owner can archive channels.".into()); }
            match sp.roster.channel(&ch) {
                None => return Err("That channel is not known.".into()),
                Some(c) if c.archived => return Err("This channel is archived.".into()),
                Some(_) => {}
            }
            if sp.roster.channels.first().map(|c| c.id) == Some(ch) {
                return Err("The first channel can't be archived.".into());
            }
            owner_op(sp, OpBody::ArchiveChannel { channel: ch }, now)?
        };
        self.push_control(&id, &op);
        self.changed();
        self.detail(space)
    }

    // ---- moderation (D5.1) -------------------------------------------------------------------------------------

    /// Owner removes a member: RemoveMember + Rotate in one step, signed and pushed at once.
    pub async fn remove_member(&self, space: &str, member: &str) -> Result<(), String> {
        let id = space_of(space)?;
        let target = MemberKey::from_hex(member).ok_or("That person isn't in this space.")?;
        self.remove_locked(&id, &target, RemoveReason::Removed)
    }

    /// The whole removal. The new key is saved first; both ops go into the store in one transaction.
    fn remove_locked(&self, id: &SpaceId, target: &MemberKey, reason: RemoveReason) -> Result<(), String> {
        let now = self.now();
        let (ops, nodes) = {
            let mut spaces = lock(&self.spaces);
            let sp = spaces.get_mut(id).ok_or("That space is not known.")?;
            if !sp.is_owner() { return Err("Only the owner can remove people.".into()); }
            if *target == sp.me.member_key() { return Err("You can't remove yourself.".into()); }
            let member = sp.roster.members.get(target).ok_or("That person isn't in this space.")?;
            let nodes: Vec<NodeKey> = member.devices.iter().map(|c| c.node).collect();
            let new_key = random_bytes::<32>();
            let ops = remove_ops(&sp.roster, &sp.me, target, reason, &new_key, now).map_err(|e| e.0)?;
            let epoch = sp.roster.epoch + 1;
            // First the key, then the ops: a crash in between loses nothing (a retry makes a new key).
            self.keystore.set_epoch_key(&space_tag(id), epoch as u64, &new_key)?;
            sp.store
                .in_tx(|s| {
                    for op in &ops {
                        if !s.control_append(op)? { return Err(rusqlite::Error::InvalidQuery); }
                    }
                    Ok(())
                })
                .map_err(db)?;
            for op in &ops { sp.roster.apply(op).map_err(|e| e.0)?; }
            sp.keys.0.insert(epoch, new_key);
            (ops, nodes)
        };
        // The removed person's streams get the news, then no more of this space.
        for op in &ops { self.push_control(id, op); }
        let still_shared: BTreeSet<NodeKey> = lock(&self.spaces)
            .values()
            .filter(|sp| sp.id != *id)
            .flat_map(|sp| sp.roster.member_nodes())
            .collect();
        let mut to_close: Vec<Arc<dyn SpaceLink>> = Vec::new();
        {
            let mut links = lock(&self.links);
            for n in &nodes {
                links.drop_writer(id, n);
                if !still_shared.contains(n) {
                    if let Some(l) = links.link(n) { to_close.push(l); }
                }
            }
        }
        if !to_close.is_empty() {
            tokio::spawn(async move {
                tokio::time::sleep(REMOVED_CLOSE_DELAY).await;
                for l in to_close { l.close(0, "removed"); }
            });
        }
        self.refresh_permissions();
        self.messages_changed(id, None);
        self.changed();
        Ok(())
    }

    /// A member leaves: a Leave entry goes out, the space turns read-only here, the owner's PC rotates the key.
    pub async fn leave(&self, space: &str) -> Result<(), String> {
        let id = space_of(space)?;
        let (first, owner, state) = self
            .with_open(&id, |sp| (sp.roster.channels.first().map(|c| c.id), sp.is_owner(), space_state(sp)))
            .ok_or("That space is not known.")?;
        if owner { return Err("You own this space. Owners can't leave in this version.".into()); }
        if state != "active" { return Err("You are not in this space any more.".into()); }
        let channel = first.ok_or("That channel is not known.")?;
        self.send(&id, &channel, Body::Leave, None).await?;
        let delivered = lock(&self.links).has_writer(&id);
        let until = if delivered { 0 } else { self.now() + LEAVE_DIAL_MS };
        self.with_open(&id, |sp| -> Result<(), String> {
            sp.store.meta_set_u64("leave_until", until).map_err(db)?;
            sp.store.meta_set("role", b"left").map_err(db)
        })
        .ok_or("That space is not known.")??;
        self.messages_changed(&id, None);
        self.changed();
        Ok(())
    }

    /// Block or unblock a member on this PC only. Nothing is sent: they are not told, and their entries still sync.
    pub fn block(&self, space: &str, member: &str, on: bool) -> Result<Value, String> {
        let id = space_of(space)?;
        let target = MemberKey::from_hex(member).ok_or("That person isn't in this space.")?;
        let now = self.now();
        self.with_open(&id, |sp| -> Result<(), String> {
            if target == sp.me.member_key() { return Err("You can't block yourself.".into()); }
            if !sp.roster.members.contains_key(&target) && !sp.roster.gone.contains_key(&target) {
                return Err("That person isn't in this space.".into());
            }
            sp.store.block_set(&target, on, now).map_err(db)
        })
        .ok_or("That space is not known.")??;
        self.messages_changed(&id, None);
        self.changed();
        self.detail(space)
    }

    /// Deletes this PC's copy of a space I left or was removed from, and forgets its keys.
    pub fn forget(&self, space: &str) -> Result<(), String> {
        let id = space_of(space)?;
        let epoch = {
            let mut spaces = lock(&self.spaces);
            let sp = spaces.get(&id).ok_or("That space is not known.")?;
            if space_state(sp) == "active" { return Err("Leave the space first.".into()); }
            self.wipe_diaries(sp);
            let epoch = sp.roster.epoch;
            spaces.remove(&id);
            epoch
        };
        lock(&self.links).drop_space_writers(&id);
        lock(&self.joining).remove(&id);
        let _ = std::fs::remove_dir_all(self.dir.join(id.to_hex()));
        self.keystore.forget_team(&space_tag(&id), epoch as u64);
        self.refresh_permissions();
        self.changed();
        Ok(())
    }

    /// Test hooks: the live link to a peer, and the Hello this PC would send for a space.
    #[cfg(test)]
    pub(crate) fn link_to(&self, peer: &NodeKey) -> Option<Arc<dyn SpaceLink>> { lock(&self.links).link(peer) }
    #[cfg(test)]
    pub(crate) fn hello_bytes(&self, space: &SpaceId) -> Option<Vec<u8>> { self.drive(space, |v| sync::encode_hello(v)) }

    /// Tests ask whether a node may open a link to this PC.
    #[cfg(test)]
    pub(crate) fn allows_node(&self, node: &NodeKey) -> bool { self.perms.allows(node) }

    /// Writes one control op to every live sync stream of the space (frame 0x02).
    fn push_control(&self, space: &SpaceId, op: &ControlOp) {
        lock(&self.links).send_to_space(space, None, sync::CONTROL_OPS, &sync::encode_ops(std::slice::from_ref(op)));
    }

    /// After every 1000 stored entries.
    fn note_stored(&self, space: &SpaceId, n: usize) {
        if n == 0 { return; }
        if self.stored_since_prune.fetch_add(n, Ordering::SeqCst) + n < PRUNE_EVERY_ENTRIES { return; }
        self.stored_since_prune.store(0, Ordering::SeqCst);
        if let Some(sp) = lock(&self.spaces).get(space) { self.prune_one(sp, self.now()); }
    }
}

struct ServiceJoinHost {
    svc: Arc<SpacesService>,
    space: SpaceId,
}

#[async_trait]
impl JoinHost for ServiceJoinHost {
    fn check(&self, req: &JoinRequestFrame, joiner: NodeKey, now_ms: u64) -> Verdict {
        let svc = &self.svc;
        let verdict = {
            let spaces = lock(&svc.spaces);
            let Some(sp) = spaces.get(&self.space).filter(|sp| sp.is_owner()) else {
                return Verdict::Refuse(RefuseReason::Gone, 0);
            };
            let mut invite = sp.store.invite_get(&req.invite).ok().flatten();
            let secret = svc
                .keystore
                .invite_code(&space_tag(&self.space), &req.invite.to_hex())
                .ok()
                .flatten()
                .and_then(|h| from_hex::<SECRET_BYTES>(&h));
            let checked = join::owner_check(&sp.roster, invite.as_mut(), secret, &mut lock(&svc.lockouts), req, joiner, now_ms);
            if let Some(inv) = &invite {
                let _ = sp.store.invite_put(inv);
                // Used up or dead (too many wrong tries): the secret has no use any more.
                if matches!(inv.state, InviteState::Used | InviteState::Dead | InviteState::Cancelled) {
                    let _ = svc.keystore.forget_invite(&space_tag(&self.space), &inv.id.to_hex());
                }
            }
            match checked {
                Err((reason, left)) => Verdict::Refuse(reason, left),
                Ok(()) => {
                    let answer_by = now_ms + JOIN_WAIT_MS;
                    match sp.store.join_request_add(&req.invite, &joiner, &req.cert.to_bytes(), &req.cert.name, now_ms, answer_by) {
                        Ok(request_id) => {
                            let (tx, rx) = oneshot::channel();
                            lock(&svc.answers_tx).insert((self.space, request_id), tx);
                            lock(&svc.answers_rx).insert((self.space, request_id), rx);
                            Verdict::Wait { request_id, answer_by_ms: answer_by }
                        }
                        Err(_) => Verdict::Refuse(RefuseReason::Bad, 0),
                    }
                }
            }
        };
        svc.refresh_permissions();
        svc.changed();
        verdict
    }

    async fn wait_answer(&self, request_id: i64) -> Answer {
        let rx = lock(&self.svc.answers_rx).remove(&(self.space, request_id));
        match rx {
            Some(rx) => rx.await.unwrap_or(Answer::NoAnswer),
            None => Answer::NoAnswer,
        }
    }
}

// ---- the `peers` table: where each member was last reachable ------------------------------------------------

fn save_peer(store: &SpaceStore, h: &PeerHint, now_ms: u64) {
    let _ = store.conn.execute(
        "INSERT INTO peers (node, relay, lan, last_seen_ms) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(node) DO UPDATE SET relay = excluded.relay, lan = excluded.lan, last_seen_ms = excluded.last_seen_ms",
        params![h.node.0.to_vec(), h.relay, h.lan.join(","), now_ms as i64],
    );
}

fn peer_rows(store: &SpaceStore) -> Vec<(PeerHint, u64)> {
    let Ok(mut st) = store.conn.prepare("SELECT node, relay, lan, last_seen_ms FROM peers ORDER BY last_seen_ms DESC") else {
        return Vec::new();
    };
    let rows = st.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?)));
    let Ok(rows) = rows else { return Vec::new() };
    rows.flatten()
        .filter_map(|(node, relay, lan, seen)| {
            let node = NodeKey(node.try_into().ok()?);
            let lan = lan.split(',').filter(|a| !a.is_empty()).map(str::to_string).collect();
            Some((PeerHint { node, relay, lan }, seen as u64))
        })
        .collect()
}

fn peer_hints(store: &SpaceStore) -> Vec<PeerHint> { peer_rows(store).into_iter().map(|(h, _)| h).collect() }

fn channel_of(hex: &str) -> Result<ChannelId, String> {
    ChannelId::from_hex(hex).ok_or_else(|| "That channel is not known.".to_string())
}

/// Owner builds, checks, applies and stores one control op. Nothing is sent here.
fn owner_op(sp: &mut OpenSpace, body: OpBody, now: u64) -> Result<ControlOp, String> {
    let op = sp.roster.next_op(&sp.me, body, now).map_err(|e| e.0)?;
    sp.roster.apply(&op).map_err(|e| e.0)?;
    sp.store.control_append(&op).map_err(db)?;
    Ok(op)
}

fn member_name(sp: &OpenSpace, m: &MemberKey) -> String {
    sp.roster.members.get(m).map(|x| x.name.clone()).unwrap_or_else(|| "Someone".to_string())
}

/// One `SpaceMessage` as the chat UI shows it. A deleted message has empty text. A file message carries `file` and its text is the file name.
fn message_json(sp: &OpenSpace, row: &MessageRow, reacts: &[(EntryId, String, u32, Vec<MemberKey>)], me: &MemberKey,
                jobs: &FileJobs, space_dir: &std::path::Path) -> Value
{
    let is_fish = row.kind == KIND_BOT;
    let author = member_name(sp, &row.member);
    // A removed bot keeps the last name it had; "Bot" only for an id this PC never heard a name for.
    let bot_name = row.bot.and_then(|b| sp.roster.bots.get(&b).map(|x| x.name.clone()).or_else(|| sp.roster.bot_names.get(&b).cloned()));
    let author_name = if is_fish { bot_name.clone().unwrap_or_else(|| "Bot".to_string()) } else { author.clone() };
    let deleted = match row.deleted_by {
        Some(by) => json!({ "byName": member_name(sp, &by), "byOwner": by == sp.roster.owner }),
        None => Value::Null,
    };
    let reply_preview = row.reply_to.and_then(|r| sp.store.message(&r).ok().flatten()).map(|p| json!({
        "authorName": member_name(sp, &p.member),
        "text": if p.deleted_by.is_some() { String::new() } else { p.text.chars().take(80).collect::<String>() },
    }));
    let reactions: Vec<Value> = reacts
        .iter()
        .filter(|(t, ..)| *t == row.id)
        .map(|(_, code, count, who)| json!({ "code": code, "count": count, "mine": who.contains(me) }))
        .collect();
    let file = file_value(space_dir, sp, row, jobs);
    let text = if row.deleted_by.is_some() {
        String::new()
    } else if let Some(name) = file["name"].as_str() {
        name.to_string()
    } else {
        row.text.clone()
    };
    json!({
        "id": row.id.to_hex(), "channel": row.channel.to_hex(), "member": row.member.to_hex(),
        "authorName": author_name, "isFish": is_fish, "botId": row.bot.map(|b| b.to_hex()), "botName": bot_name,
        // A bot line is signed by its owner's member key, but it is the bot talking: never "mine" on the screen.
        "isMine": !is_fish && row.member == *me, "ownerName": if is_fish { json!(author) } else { Value::Null },
        "ownedByMe": is_fish && row.member == *me,
        "createdAtMs": row.created_ms,
        "text": text,
        "replyTo": row.reply_to.map(|r| r.to_hex()), "replyPreview": reply_preview,
        "deleted": deleted, "reactions": reactions, "file": file,
        "blocked": row.member != *me && sp.store.is_blocked(&row.member),
    })
}
