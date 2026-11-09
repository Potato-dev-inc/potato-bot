//! Services talk to each other over the in-memory net (`MemoryHub`): join, send, catch up, refuse.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::frame::write_frame;
use super::ids::{BotId, ChannelId, NodeKey, SpaceId};
use super::log::{Body, Entry, EntryHeader, KIND_BOT};
use super::memnet::MemoryHub;
use super::net::{NetError, PeerHint, SpaceNet, SpacePermissions};
use super::service::SpacesService;
use super::sync;
use crate::secrets::MemoryBackend;
use crate::spaces::keystore::KeyStore;

const T0: u64 = 1_700_000_000_000;

pub(crate) struct TempDir(pub(crate) PathBuf);

impl TempDir {
    pub(crate) fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("pm-svc-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&p);
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

pub(crate) struct Node {
    pub(crate) svc: Arc<SpacesService>,
    pub(crate) key: NodeKey,
    pub(crate) keystore: KeyStore,
    pub(crate) dir: TempDir,
}

fn hint(n: NodeKey) -> PeerHint { PeerHint { node: n, relay: String::new(), lan: Vec::new() } }

pub(crate) fn make(hub: &Arc<MemoryHub>, n: u8, clock: &Arc<AtomicU64>, keystore: Option<KeyStore>, dir: Option<TempDir>) -> Node {
    let node = make_unnamed(hub, n, clock, keystore, dir);
    if node.svc.display_name().is_none() { node.svc.set_display_name("Me").unwrap(); }
    node
}

/// A PC that has not picked its Spaces name yet (create and join must ask for one).
pub(crate) fn make_unnamed(hub: &Arc<MemoryHub>, n: u8, clock: &Arc<AtomicU64>, keystore: Option<KeyStore>, dir: Option<TempDir>) -> Node {
    let key = NodeKey([n; 32]);
    let perms = SpacePermissions::new();
    let net = hub.node(key, perms.clone());
    let keystore = keystore.unwrap_or_else(|| KeyStore::with_backend(Arc::new(MemoryBackend::new()), &format!("node{n}")));
    let dir = dir.unwrap_or_else(|| TempDir::new(&format!("n{n}")));
    let c = clock.clone();
    let svc = SpacesService::new(net, perms, keystore.clone(), dir.0.clone(), Arc::new(move || c.load(Ordering::SeqCst)));
    Node { svc, key, keystore, dir }
}

pub(crate) async fn node(hub: &Arc<MemoryHub>, n: u8, clock: &Arc<AtomicU64>) -> Node {
    let node = make(hub, n, clock, None, None);
    node.svc.start_with(false).await.unwrap();
    node
}

pub(crate) async fn eventually(what: &str, f: impl Fn() -> bool) {
    for _ in 0..400 {
        if f() { return; }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("timed out waiting for: {what}");
}

pub(crate) fn clock() -> Arc<AtomicU64> { Arc::new(AtomicU64::new(T0)) }

pub(crate) fn general(n: &Node, space: &SpaceId) -> ChannelId {
    n.svc.with_open(space, |sp| sp.roster.channels[0].id).unwrap()
}

pub(crate) fn texts(n: &Node, space: &SpaceId) -> Vec<String> {
    let ch = general(n, space);
    n.svc
        .with_open(space, |sp| sp.store.messages_page(&ch, None, 200).unwrap().into_iter().map(|m| m.text).collect())
        .unwrap_or_default()
}

pub(crate) fn entry_count(n: &Node, space: &SpaceId) -> i64 {
    n.svc.with_open(space, |sp| sp.store.conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0)).unwrap()).unwrap_or(0)
}

pub(crate) async fn say(n: &Node, space: &SpaceId, t: &str) {
    let ch = general(n, space);
    n.svc.send(space, &ch, Body::Text { text: t.into(), mentions: vec![] }, None).await.unwrap();
}

fn joining_state(n: &Node) -> String {
    n.svc.view()["joining"][0]["state"].as_str().unwrap_or("").to_string()
}

fn has_space(n: &Node) -> bool { !n.svc.view()["spaces"].as_array().unwrap().is_empty() }

/// `owner` invites, `joiner` joins, the owner lets them in. Returns the space id.
pub(crate) async fn join(owner: &Node, joiner: &Node, space_hex: &str) {
    let invite = owner.svc.create_invite(space_hex).await.unwrap();
    let words: Vec<String> = invite["words"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect();
    joiner.svc.start_join(invite["link"].as_str().unwrap(), &words.join(" ")).await.unwrap();
    eventually("a join request", || !owner.svc.view()["requests"].as_array().unwrap().is_empty()).await;
    let id = owner.svc.view()["requests"][0]["id"].as_i64().unwrap();
    owner.svc.answer_join(space_hex, id, true).await.unwrap();
    eventually("the joiner is in", || has_space(joiner) && joining_state(joiner) == "joined").await;
}

pub(crate) async fn pair() -> (Arc<MemoryHub>, Arc<AtomicU64>, Node, Node, SpaceId) {
    pair_with(clock()).await
}

/// `pair` on a clock the caller made (the Potato loop test uses the real time, because Potato's own clock is real).
pub(crate) async fn pair_with(clk: Arc<AtomicU64>) -> (Arc<MemoryHub>, Arc<AtomicU64>, Node, Node, SpaceId) {
    let hub = MemoryHub::new();
    let (a, b) = (node(&hub, 1, &clk).await, node(&hub, 2, &clk).await);
    let hex = a.svc.create_space("Book club").await.unwrap();
    join(&a, &b, &hex).await;
    eventually("a link", || a.svc.link_count() == 1 && b.svc.link_count() == 1).await;
    (hub, clk, a, b, SpaceId::from_hex(&hex).unwrap())
}

pub(crate) async fn trio() -> (Arc<MemoryHub>, Arc<AtomicU64>, Node, Node, Node, SpaceId) {
    let (hub, clk, a, b, space) = pair().await;
    let c = node(&hub, 3, &clk).await;
    join(&a, &c, &space.to_hex()).await;
    eventually("b knows c", || b.svc.with_open(&space, |sp| sp.roster.members.len()) == Some(3)).await;
    // c's first dial may have come before b knew c: let the backoff pass, then dial again.
    clk.fetch_add(10 * 60 * 1000, Ordering::SeqCst);
    c.svc.dial_round().await;
    eventually("c linked to a and b", || c.svc.link_count() == 2 && b.svc.link_count() == 2).await;
    (hub, clk, a, b, c, space)
}

#[tokio::test]
async fn create_space_makes_owner_and_general() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    assert_eq!(a.svc.create_space("   ").await.err().as_deref(), Some("Give the space a name."));
    let hex = a.svc.create_space("Book club").await.unwrap();
    let v = a.svc.view();
    assert_eq!(v["spaces"].as_array().unwrap().len(), 1);
    assert_eq!(v["spaces"][0]["isOwner"], true);
    assert_eq!(v["spaces"][0]["name"], "Book club");
    let space = SpaceId::from_hex(&hex).unwrap();
    assert_eq!(a.svc.with_open(&space, |sp| sp.store.control_ops().unwrap().len()), Some(1));
    assert_eq!(a.svc.with_open(&space, |sp| sp.roster.channels[0].name.clone()).as_deref(), Some("general"));
}

#[tokio::test]
async fn full_join_through_services() {
    let (_hub, _clk, a, b, space) = pair().await;
    assert_eq!(joining_state(&b), "joined");
    assert_eq!(b.svc.view()["spaces"][0]["members"], 2);
    assert_eq!(b.svc.view()["spaces"][0]["isOwner"], false);
    assert_eq!(b.svc.with_open(&space, |sp| sp.store.control_ops().unwrap().len()), Some(2));
    assert_eq!(a.svc.view()["spaces"][0]["members"], 2);
    // The invite is used up and the request answered.
    assert!(a.svc.view()["requests"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn two_members_see_each_others_messages() {
    let (_hub, _clk, a, b, space) = pair().await;
    say(&a, &space, "hi").await;
    say(&b, &space, "yo").await;
    eventually("both messages on both", || texts(&a, &space).len() == 2 && texts(&b, &space).len() == 2).await;
    for n in [&a, &b] {
        let mut t = texts(n, &space);
        t.sort();
        assert_eq!(t, vec!["hi".to_string(), "yo".to_string()]);
    }
}

#[tokio::test]
async fn a_member_who_was_offline_catches_up_including_what_was_sent_while_away() {
    let (hub, _clk, a, b, space) = pair().await;
    say(&a, &space, "before").await;
    eventually("b has before", || texts(&b, &space).len() == 1).await;
    // b goes offline.
    hub.cut(a.key, b.key, true);
    eventually("links gone", || a.svc.link_count() == 0 && b.svc.link_count() == 0).await;
    say(&a, &space, "while you were away").await;
    say(&b, &space, "typed offline").await;
    assert_eq!(texts(&b, &space).len(), 2);
    assert_eq!(texts(&a, &space).len(), 2);
    // b comes back and dials.
    hub.cut(a.key, b.key, false);
    b.svc.dial_round().await;
    eventually("both caught up", || texts(&a, &space).len() == 3 && texts(&b, &space).len() == 3).await;
    let mut t = texts(&b, &space);
    t.sort();
    assert_eq!(t, vec!["before".to_string(), "typed offline".to_string(), "while you were away".to_string()]);
}

#[tokio::test]
async fn offline_member_catches_up_from_a_third_member() {
    let (hub, clk, a, b, c, space) = trio().await;
    // c goes offline from both; b stays with a.
    hub.cut(a.key, c.key, true);
    hub.cut(b.key, c.key, true);
    eventually("c has no links", || c.svc.link_count() == 0).await;
    for i in 0..20 {
        // Ten messages in ten seconds is the most one person may send: space them out.
        clk.fetch_add(2_000, Ordering::SeqCst);
        say(&a, &space, &format!("m{i}")).await;
    }
    eventually("b has 20", || texts(&b, &space).len() == 20).await;
    assert_eq!(texts(&c, &space).len(), 0);
    // a goes away for good; c can only reach b.
    a.svc.stop().await;
    hub.cut(b.key, c.key, false);
    c.svc.dial_round().await;
    eventually("c has 20 from a", || texts(&c, &space).len() == 20).await;
}

#[tokio::test]
async fn push_is_forwarded_once() {
    let (hub, _clk, a, b, c, space) = trio().await;
    hub.cut(a.key, c.key, true);
    eventually("chain a-b-c", || a.svc.link_count() == 1 && c.svc.link_count() == 1 && b.svc.link_count() == 2).await;
    let before = b.svc.pushes_written();
    say(&a, &space, "one").await;
    eventually("c has it", || texts(&c, &space).len() == 1).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(texts(&c, &space).len(), 1);
    assert_eq!(entry_count(&c, &space), 1);
    assert_eq!(b.svc.pushes_written() - before, 1);
}

#[tokio::test]
async fn stranger_first_frame_must_be_join_request() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    let hex = a.svc.create_space("Book club").await.unwrap();
    let space = SpaceId::from_hex(&hex).unwrap();
    a.svc.create_invite(&hex).await.unwrap();
    let d = hub.node(NodeKey([9; 32]), SpacePermissions::new());
    let link = d.dial(&hint(a.key)).await.expect("an open invite lets a stranger knock");
    let mut s = link.open().await.unwrap();
    write_frame(&mut s.send, sync::HELLO, &[1, 2, 3]).await.unwrap();
    let code = tokio::time::timeout(Duration::from_secs(2), link.closed()).await.unwrap();
    assert_eq!(code, 2);
    assert_eq!(entry_count(&a, &space), 0);
    assert_eq!(a.svc.view()["spaces"][0]["members"], 1);
}

#[tokio::test]
async fn permissions_follow_roster_and_invites() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    let hex = a.svc.create_space("Book club").await.unwrap();
    let d = hub.node(NodeKey([9; 32]), SpacePermissions::new());
    assert_eq!(d.dial(&hint(a.key)).await.err(), Some(NetError::Refused));
    let invite = a.svc.create_invite(&hex).await.unwrap();
    assert!(d.dial(&hint(a.key)).await.is_ok());
    a.svc.cancel_invite(&hex, invite["id"].as_str().unwrap()).unwrap();
    assert_eq!(d.dial(&hint(a.key)).await.err(), Some(NetError::Refused));
}

#[tokio::test]
async fn owner_no_answer_expires_after_thirty_minutes() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let (a, b) = (node(&hub, 1, &clk).await, node(&hub, 2, &clk).await);
    let hex = a.svc.create_space("Book club").await.unwrap();
    let invite = a.svc.create_invite(&hex).await.unwrap();
    let words: Vec<String> = invite["words"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect();
    b.svc.start_join(invite["link"].as_str().unwrap(), &words.join(" ")).await.unwrap();
    eventually("a request", || !a.svc.view()["requests"].as_array().unwrap().is_empty()).await;
    eventually("b waits", || joining_state(&b) == "waiting").await;
    clk.fetch_add(30 * 60 * 1000 + 61_000, Ordering::SeqCst);
    a.svc.tick();
    eventually("b hears no answer", || joining_state(&b) == "noAnswer").await;
    assert_eq!(a.svc.view()["invites"][0]["state"], "dead");
    assert!(a.svc.view()["requests"].as_array().unwrap().is_empty());
    assert!(!has_space(&b));
}

#[tokio::test]
async fn duplicate_links_keep_one() {
    let (hub, _clk, a, b, space) = pair().await;
    hub.cut(a.key, b.key, true);
    eventually("links gone", || a.svc.link_count() == 0 && b.svc.link_count() == 0).await;
    hub.cut(a.key, b.key, false);
    tokio::join!(a.svc.dial_round(), b.svc.dial_round());
    eventually("one link each", || a.svc.link_count() == 1 && b.svc.link_count() == 1).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!((a.svc.link_count(), b.svc.link_count()), (1, 1));
    // The link that stayed works.
    say(&a, &space, "still here").await;
    eventually("b hears a", || texts(&b, &space) == vec!["still here".to_string()]).await;
}

#[tokio::test]
async fn restart_reloads_spaces() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    let hex = a.svc.create_space("Book club").await.unwrap();
    let space = SpaceId::from_hex(&hex).unwrap();
    say(&a, &space, "kept").await;
    a.svc.stop().await;
    let Node { keystore, dir, .. } = a;
    let a2 = make(&hub, 1, &clk, Some(keystore), Some(dir));
    a2.svc.start_with(false).await.unwrap();
    let v = a2.svc.view();
    assert_eq!(v["spaces"].as_array().unwrap().len(), 1);
    assert_eq!(v["spaces"][0]["name"], "Book club");
    assert_eq!(a2.svc.with_open(&space, |sp| sp.roster.members.len()), Some(1));
    assert_eq!(texts(&a2, &space), vec!["kept".to_string()]);
    // The key came back too: it can still write.
    say(&a2, &space, "and more").await;
    assert_eq!(texts(&a2, &space).len(), 2);
}

/// A bot answer from a bot that is not on the roster, signed by a real member.
fn bad_bot_entry(n: &Node, space: &SpaceId) -> Entry {
    let ch = general(n, space);
    n.svc
        .with_open(space, |sp| {
            let device = sp.me.device_key();
            let header = EntryHeader {
                space: *space, channel: ch, device, seq: sp.store.next_seq(&device).unwrap(),
                lamport: sp.store.next_lamport().unwrap(), created_ms: T0, epoch: sp.roster.epoch, kind: KIND_BOT,
                refers: None, bot: Some(BotId([5; 8])),
            };
            let body = Body::BotText { text: "I am a bot".into(), called_by: None, hops: 0 };
            Entry::seal(&sp.me, header, &body, &sp.keys.0[&sp.roster.epoch]).unwrap()
        })
        .unwrap()
}

#[tokio::test]
async fn bot_text_from_a_bot_not_on_the_roster_is_refused() {
    let (_hub, _clk, a, b, space) = pair().await;
    // 1. It arrives as a live push: the other side stores nothing, and the link keeps working.
    let bad = bad_bot_entry(&a, &space);
    a.svc.forward(&space, None, &bad.to_bytes());
    say(&a, &space, "after the push").await;
    eventually("b has the normal one", || texts(&b, &space) == vec!["after the push".to_string()]).await;
    let device = a.svc.with_open(&space, |sp| sp.me.device_key()).unwrap();
    assert!(b.svc.with_open(&space, |sp| sp.store.conn
        .query_row("SELECT COUNT(*) FROM messages WHERE bot IS NOT NULL", [], |r| r.get::<_, i64>(0)).unwrap()) == Some(0));

    // 2. It sits in a's log (put there past the checks) and b asks for it while catching up: refused, no loop.
    let hole = bad_bot_entry(&a, &space);
    let hole_seq = hole.header.seq;
    a.svc.with_open(&space, |sp| sp.store.insert_entry(&hole, &hole.id(), &hole.to_bytes(), T0).unwrap());
    say(&a, &space, "after the hole").await;
    eventually("b has both normal ones", || texts(&b, &space).len() == 2).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(b.svc.with_open(&space, |sp| sp.store.has_seq(&device, hole_seq).unwrap()), Some(false));
    // The stream is still healthy.
    say(&a, &space, "and one more").await;
    eventually("b still receives", || texts(&b, &space).len() == 3).await;
    let ch = general(&b, &space);
    assert_eq!(b.svc.with_open(&space, |sp| sp.store.messages_page(&ch, None, 50).unwrap().iter().all(|m| m.bot.is_none())), Some(true));
}

/// A text from `n`'s own device that is `age_ms` old, put in its log past the receive checks (as if it had been
/// stored when it was young).
fn put_old_text(n: &Node, space: &SpaceId, age_ms: u64) {
    let ch = general(n, space);
    n.svc
        .with_open(space, |sp| {
            let device = sp.me.device_key();
            let seq = sp.store.next_seq(&device).unwrap();
            let header = EntryHeader {
                space: *space, channel: ch, device, seq, lamport: seq, created_ms: T0 - age_ms, epoch: sp.roster.epoch,
                kind: super::log::KIND_TEXT, refers: None, bot: None,
            };
            let body = Body::Text { text: "ancient".into(), mentions: vec![] };
            let e = Entry::seal(&sp.me, header, &body, &sp.keys.0[&sp.roster.epoch]).unwrap();
            sp.store.insert_entry(&e, &e.id(), &e.to_bytes(), T0).unwrap();
        })
        .unwrap();
}

const NINETY_ONE_DAYS: u64 = 91 * 24 * 60 * 60 * 1000;

#[tokio::test]
async fn old_entries_are_pruned_when_the_service_starts() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    let hex = a.svc.create_space("Book club").await.unwrap();
    let space = SpaceId::from_hex(&hex).unwrap();
    put_old_text(&a, &space, NINETY_ONE_DAYS);
    assert_eq!(entry_count(&a, &space), 1);
    a.svc.stop().await;
    let Node { keystore, dir, .. } = a;
    let a2 = make(&hub, 1, &clk, Some(keystore), Some(dir));
    a2.svc.start_with(false).await.unwrap();
    assert_eq!(entry_count(&a2, &space), 0);
    let device = a2.svc.with_open(&space, |sp| sp.me.device_key()).unwrap();
    assert_eq!(a2.svc.with_open(&space, |sp| sp.store.author_floor(&device).unwrap()), Some(1));
}

#[tokio::test]
async fn prune_runs_after_every_thousand_stored_entries() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    let hex = a.svc.create_space("Book club").await.unwrap();
    let space = SpaceId::from_hex(&hex).unwrap();
    put_old_text(&a, &space, NINETY_ONE_DAYS);
    for i in 0..999 {
        clk.fetch_add(7_000, Ordering::SeqCst); // under the 10-per-10-seconds and 600-per-hour limits
        say(&a, &space, &format!("m{i}")).await;
    }
    assert_eq!(entry_count(&a, &space), 1000, "999 new ones and the old one");
    clk.fetch_add(7_000, Ordering::SeqCst);
    say(&a, &space, "the thousandth").await;
    assert_eq!(entry_count(&a, &space), 1000, "the old one went");
}

// ---- D4.1 rules: send, channels -------------------------------------------------------------------------------

#[tokio::test]
async fn send_text_rules() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    let hex = a.svc.create_space("Book club").await.unwrap();
    let space = SpaceId::from_hex(&hex).unwrap();
    let ch = general(&a, &space).to_hex();
    assert_eq!(a.svc.send_text(&hex, &ch, "   ", None).await.unwrap_err(), "Write something first.");
    let long = "x".repeat(4001);
    assert_eq!(
        a.svc.send_text(&hex, &ch, &long, None).await.unwrap_err(),
        "This message is too long. The limit is 4,000 characters."
    );
    assert!(a.svc.send_text(&hex, &ch, &"x".repeat(4000), None).await.is_ok());
    let detail = a.svc.channel_create(&hex, "books").await.unwrap();
    let books = detail["channels"].as_array().unwrap().iter().find(|c| c["name"] == "books").unwrap()["id"]
        .as_str().unwrap().to_string();
    a.svc.channel_archive(&hex, &books).await.unwrap();
    assert_eq!(a.svc.send_text(&hex, &books, "hi", None).await.unwrap_err(), "This channel is archived.");
}

#[tokio::test]
async fn channel_create_rules() {
    let (_hub, _clk, a, b, space) = pair().await;
    let hex = space.to_hex();
    assert_eq!(b.svc.channel_create(&hex, "books").await.unwrap_err(), "Only the owner can add channels.");
    let detail = a.svc.channel_create(&hex, "Books!").await.unwrap();
    assert!(detail["channels"].as_array().unwrap().iter().any(|c| c["name"] == "books"));
    // general + books = 2; add up to 20 active channels, then the next is refused.
    for i in 2..20 {
        a.svc.channel_create(&hex, &format!("c{i}")).await.unwrap();
    }
    assert_eq!(
        a.svc.channel_create(&hex, "one-more").await.unwrap_err(),
        "This space has 20 channels. Archive one first."
    );
}

#[tokio::test]
async fn new_channel_reaches_members() {
    let (_hub, _clk, a, b, space) = pair().await;
    a.svc.channel_create(&space.to_hex(), "books").await.unwrap();
    eventually("b lists books", || {
        b.svc.detail(&space.to_hex()).map(|d| d["channels"].as_array().unwrap().iter().any(|c| c["name"] == "books")).unwrap_or(false)
    })
    .await;
}

// ---- D5.3: react and delete ------------------------------------------------------------------------------------

fn msg_id(n: &Node, space: &SpaceId, text: &str) -> String {
    let ch = general(n, space);
    n.svc
        .with_open(space, |sp| sp.store.messages_page(&ch, None, 200).unwrap().into_iter().find(|m| m.text == text).map(|m| m.id.to_hex()))
        .flatten()
        .unwrap_or_default()
}

fn message(n: &Node, space: &SpaceId, id: &str) -> serde_json::Value {
    let list = n.svc.messages(&space.to_hex(), &general(n, space).to_hex(), None, 100).unwrap();
    list.as_array().unwrap().iter().find(|m| m["id"] == id).cloned().unwrap_or(serde_json::Value::Null)
}

fn heart_count(n: &Node, space: &SpaceId, id: &str) -> u64 {
    message(n, space, id)["reactions"].as_array().map(|r| {
        r.iter().filter(|x| x["code"] == "heart").map(|x| x["count"].as_u64().unwrap()).sum()
    }).unwrap_or(0)
}

#[tokio::test]
async fn reaction_reaches_others_with_count() {
    let (_hub, _clk, a, b, c, space) = trio().await;
    say(&b, &space, "hi").await;
    eventually("a and c have it", || !msg_id(&a, &space, "hi").is_empty() && !msg_id(&c, &space, "hi").is_empty()).await;
    let id = msg_id(&b, &space, "hi");
    a.svc.react(&space.to_hex(), &id, "heart", true).await.unwrap();
    c.svc.react(&space.to_hex(), &id, "heart", true).await.unwrap();
    eventually("b sees two hearts", || heart_count(&b, &space, &id) == 2).await;
    assert_eq!(message(&b, &space, &id)["reactions"][0]["mine"], false);
}

#[tokio::test]
async fn reaction_off_removes_mine() {
    let (_hub, _clk, a, b, space) = pair().await;
    say(&a, &space, "hi").await;
    eventually("b has it", || !msg_id(&b, &space, "hi").is_empty()).await;
    let id = msg_id(&a, &space, "hi");
    a.svc.react(&space.to_hex(), &id, "heart", true).await.unwrap();
    eventually("b sees one heart", || heart_count(&b, &space, &id) == 1).await;
    a.svc.react(&space.to_hex(), &id, "heart", false).await.unwrap();
    eventually("zero everywhere", || heart_count(&a, &space, &id) == 0 && heart_count(&b, &space, &id) == 0).await;
}

#[tokio::test]
async fn unknown_reaction_is_refused() {
    let (_hub, _clk, a, _b, space) = pair().await;
    say(&a, &space, "hi").await;
    let id = msg_id(&a, &space, "hi");
    assert_eq!(a.svc.react(&space.to_hex(), &id, "fire", true).await.unwrap_err(), "That reaction isn't available.");
}

#[tokio::test]
async fn author_deletes_for_everyone() {
    let (_hub, _clk, a, b, c, space) = trio().await;
    say(&b, &space, "oops").await;
    eventually("a and c have it", || !msg_id(&a, &space, "oops").is_empty() && !msg_id(&c, &space, "oops").is_empty()).await;
    let id = msg_id(&b, &space, "oops");
    let reply = b.svc.delete(&space.to_hex(), &id).await.unwrap();
    assert_eq!(reply["text"], "");
    for n in [&a, &c] {
        eventually("deleted by b", || {
            let m = message(n, &space, &id);
            m["deleted"]["byOwner"] == false && m["text"] == ""
        })
        .await;
    }
}

#[tokio::test]
async fn owner_deletes_anyones() {
    let (_hub, _clk, a, b, space) = pair().await;
    say(&b, &space, "rude").await;
    eventually("a has it", || !msg_id(&a, &space, "rude").is_empty()).await;
    let id = msg_id(&a, &space, "rude");
    let reply = a.svc.delete(&space.to_hex(), &id).await.unwrap();
    assert_eq!(reply["deleted"]["byOwner"], true);
    eventually("b shows it deleted by the owner", || message(&b, &space, &id)["deleted"]["byOwner"] == true).await;
}

#[tokio::test]
async fn member_cannot_delete_others() {
    let (_hub, _clk, _a, b, c, space) = trio().await;
    say(&b, &space, "mine").await;
    eventually("c has it", || !msg_id(&c, &space, "mine").is_empty()).await;
    let id = msg_id(&c, &space, "mine");
    let before = entry_count(&c, &space);
    assert_eq!(c.svc.delete(&space.to_hex(), &id).await.unwrap_err(), "You can only delete your own messages.");
    assert_eq!(entry_count(&c, &space), before);
}

#[tokio::test]
async fn forged_delete_from_member_is_ignored() {
    let (_hub, _clk, a, b, c, space) = trio().await;
    say(&b, &space, "keep me").await;
    eventually("a and c have it", || !msg_id(&a, &space, "keep me").is_empty() && !msg_id(&c, &space, "keep me").is_empty()).await;
    let id = msg_id(&c, &space, "keep me");
    // C skips the service rule and sends a Delete for B's message anyway.
    let target = super::ids::EntryId::from_hex(&id).unwrap();
    let (a_before, b_before) = (entry_count(&a, &space), entry_count(&b, &space));
    c.svc.send(&space, &general(&c, &space), Body::Delete, Some(target)).await.unwrap();
    eventually("the forged entry reached a and b", || entry_count(&a, &space) > a_before && entry_count(&b, &space) > b_before).await;
    for n in [&a, &b] {
        assert_eq!(message(n, &space, &id)["text"], "keep me");
        assert!(message(n, &space, &id)["deleted"].is_null());
    }
}

#[tokio::test]
async fn sending_as_a_bot_that_may_not_speak_is_refused_before_it_is_signed() {
    let (_hub, _clk, a, _b, space) = pair().await;
    let ch = general(&a, &space);
    let body = Body::BotText { text: "hi".into(), called_by: None, hops: 0 };
    let err = a.svc.send_with_bot(&space, &ch, body, None, Some(BotId([5; 8]))).await.unwrap_err();
    assert_eq!(err, "That bot may not speak here.");
}

#[tokio::test]
async fn a_refused_join_closes_the_link() {
    use super::identity::SpaceIdentity;
    use super::invite::InviteTicket;
    use super::join::{JoinRequestFrame, JOIN_REFUSED, JOIN_REQUEST};
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    let hex = a.svc.create_space("Book club").await.unwrap();
    let invite = a.svc.create_invite(&hex).await.unwrap();
    let (ticket, _) = InviteTicket::parse(invite["link"].as_str().unwrap()).unwrap();
    let stranger = SpaceIdentity::for_test(9);
    let d = hub.node(stranger.node(), SpacePermissions::new());
    let link = d.dial(&hint(a.key)).await.unwrap();
    let mut s = link.open().await.unwrap();
    let req = JoinRequestFrame::new(ticket.space, ticket.invite, stranger.certify(&ticket.space, "X", T0), [0u8; 32], None);
    write_frame(&mut s.send, JOIN_REQUEST, &req.to_bytes()).await.unwrap();
    let reply = super::frame::read_frame(&mut s.recv).await.unwrap();
    assert_eq!(reply.kind, JOIN_REFUSED);
    tokio::time::timeout(Duration::from_secs(3), link.closed()).await.expect("the owner closes a refused link");
    eventually("the owner keeps no link", || a.svc.link_count() == 0).await;
}

#[tokio::test]
async fn strangers_cannot_fill_every_link_slot() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    let hex = a.svc.create_space("Book club").await.unwrap();
    a.svc.create_invite(&hex).await.unwrap();
    let mut held = Vec::new();
    for n in 0..(super::limits::MAX_LINKS as u8 + 3) {
        let d = hub.node(NodeKey([100 + n; 32]), SpacePermissions::new());
        if let Ok(link) = d.dial(&hint(a.key)).await { held.push((d, link)); }
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(a.svc.link_count() <= super::limits::MAX_LINKS, "{} links", a.svc.link_count());
}

#[tokio::test]
async fn invite_secrets_are_forgotten_once_the_invite_is_used_or_dead() {
    use super::ids::space_tag;
    let (hub, clk) = (MemoryHub::new(), clock());
    let (a, b) = (node(&hub, 1, &clk).await, node(&hub, 2, &clk).await);
    let hex = a.svc.create_space("Book club").await.unwrap();
    let tag = space_tag(&SpaceId::from_hex(&hex).unwrap());
    let kept = |n: &Node, inv: &serde_json::Value| n.keystore.invite_code(&tag, inv["id"].as_str().unwrap()).unwrap().is_some();
    // Used: the owner answered.
    let first = a.svc.create_invite(&hex).await.unwrap();
    assert!(kept(&a, &first));
    let words: Vec<String> = first["words"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect();
    b.svc.start_join(first["link"].as_str().unwrap(), &words.join(" ")).await.unwrap();
    eventually("a request", || !a.svc.view()["requests"].as_array().unwrap().is_empty()).await;
    let id = a.svc.view()["requests"][0]["id"].as_i64().unwrap();
    a.svc.answer_join(&hex, id, true).await.unwrap();
    assert!(!kept(&a, &first), "the answered invite keeps no secret");
    // Dead: the time ran out.
    let second = a.svc.create_invite(&hex).await.unwrap();
    assert!(kept(&a, &second));
    clk.fetch_add(25 * 60 * 60 * 1000, Ordering::SeqCst);
    a.svc.tick();
    assert!(!kept(&a, &second), "the expired invite keeps no secret");
}

#[tokio::test]
async fn control_ops_reach_the_disk_before_the_roster_in_memory() {
    let (_hub, _clk, a, _b, space) = pair().await;
    let (seq, bad) = a.svc.with_open(&space, |sp| {
        // The log on disk gets one op the roster in memory does not have (as after a crash between the two).
        let ahead = sp.roster.next_op(&sp.me, super::roster::OpBody::AddChannel { channel: ChannelId([77; 8]), name: "ahead".into() }, T0).unwrap();
        assert_eq!(sp.store.control_append(&ahead).unwrap(), true);
        let other = sp.roster.next_op(&sp.me, super::roster::OpBody::AddChannel { channel: ChannelId([78; 8]), name: "other".into() }, T0).unwrap();
        (sp.roster.seq, other)
    }).unwrap();
    // The disk refuses this op (not the next in its log), so memory must not take it either.
    let ok = a.svc.apply_control(&space, NodeKey([2; 32]), vec![bad]);
    assert!(!ok);
    let after = a.svc.with_open(&space, |sp| (sp.roster.seq, sp.roster.channel(&ChannelId([78; 8])).is_some())).unwrap();
    assert_eq!(after, (seq, false));
}

// ---- my name in Spaces ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn create_and_join_need_a_display_name_first() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    let hex = a.svc.create_space("Book club").await.unwrap();
    let invite = a.svc.create_invite(&hex).await.unwrap();
    let words: Vec<String> = invite["words"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect();
    let b = make_unnamed(&hub, 2, &clk, None, None);
    b.svc.start_with(false).await.unwrap();
    assert_eq!(b.svc.display_name(), None);
    assert_eq!(b.svc.create_space("Mine").await.err().as_deref(), Some("needs_display_name"));
    assert_eq!(b.svc.start_join(invite["link"].as_str().unwrap(), &words.join(" ")).await.err().as_deref(), Some("needs_display_name"));
    assert!(b.svc.view()["spaces"].as_array().unwrap().is_empty());
    assert!(b.svc.view()["joining"].as_array().unwrap().is_empty());
    assert!(b.svc.set_display_name("   ").is_err());
    assert_eq!(b.svc.set_display_name("  Sam ").unwrap(), "Sam");
    assert_eq!(b.svc.view()["displayName"], "Sam");
    let mine = b.svc.create_space("Mine").await.unwrap();
    let who = b.svc.with_open(&SpaceId::from_hex(&mine).unwrap(), |sp| {
        sp.roster.members.values().map(|m| m.name.clone()).collect::<Vec<_>>()
    });
    assert_eq!(who, Some(vec!["Sam".to_string()]));
}

#[tokio::test]
async fn the_display_name_survives_a_restart_and_keeps_the_other_settings() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = make_unnamed(&hub, 1, &clk, None, None);
    a.svc.set_show_online(false).unwrap();
    a.svc.set_display_name("Ren").unwrap();
    let b = make_unnamed(&hub, 1, &clk, Some(a.keystore.clone()), Some(TempDir(a.dir.0.clone())));
    std::mem::forget(a);
    assert_eq!(b.svc.display_name().as_deref(), Some("Ren"));
    assert!(!b.svc.show_online());
}

#[tokio::test]
async fn joining_uses_my_name_and_renaming_reaches_every_space() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    a.svc.set_display_name("Alice").unwrap();
    let hex = a.svc.create_space("Book club").await.unwrap();
    let space = SpaceId::from_hex(&hex).unwrap();
    let b = make_unnamed(&hub, 2, &clk, None, None);
    b.svc.set_display_name("Bob").unwrap();
    b.svc.start_with(false).await.unwrap();
    join(&a, &b, &hex).await;
    eventually("a link", || a.svc.link_count() == 1 && b.svc.link_count() == 1).await;
    let names = |n: &Node| n.svc.with_open(&space, |sp| sp.roster.members.values().map(|m| m.name.clone()).collect::<Vec<_>>()).unwrap();
    assert!(names(&a).contains(&"Bob".to_string()));
    assert!(names(&a).contains(&"Alice".to_string()));
    // a member renames: the owner's PC publishes it, both PCs show it
    b.svc.set_display_name("Robert").unwrap();
    eventually("a sees Robert", || names(&a).contains(&"Robert".to_string())).await;
    eventually("b sees Robert", || names(&b).contains(&"Robert".to_string())).await;
    assert!(!names(&a).contains(&"Bob".to_string()));
    // the owner renames: the member's PC learns it from the control op
    a.svc.set_display_name("Alicia").unwrap();
    eventually("b sees Alicia", || names(&b).contains(&"Alicia".to_string())).await;
}
