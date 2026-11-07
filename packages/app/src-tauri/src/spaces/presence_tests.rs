//! Presence (D8.3): the 90-second table, the frame, and services telling each other over the in-memory net.
use std::sync::atomic::Ordering;

use super::ids::{MemberKey, SpaceId};
use super::presence::*;
use super::service_tests::{eventually, pair, trio, Node};

fn m(n: u8) -> MemberKey { MemberKey([n; 32]) }

const NOW: u64 = 1_700_000_000_000;

fn seen_by(n: &Node, space: &SpaceId, member: &MemberKey) -> String {
    let d = n.svc.detail(&space.to_hex()).unwrap();
    d["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["member"] == member.to_hex())
        .map(|x| x["presence"].as_str().unwrap().to_string())
        .unwrap_or_default()
}

fn member_of(n: &Node, space: &SpaceId) -> MemberKey { n.svc.with_open(space, |sp| sp.me.member_key()).unwrap() }

#[test]
fn fresh_report_is_online() {
    let mut t = PresenceTable::new();
    t.heard(m(1), Presence::Online, NOW, NOW);
    assert_eq!(t.state_of(&m(1), NOW), Presence::Online);
    assert_eq!(t.state_of(&m(2), NOW), Presence::Offline);
}

#[test]
fn stale_after_90_seconds() {
    let mut t = PresenceTable::new();
    t.heard(m(1), Presence::Online, NOW - 90_001, NOW);
    assert_eq!(t.state_of(&m(1), NOW), Presence::Offline);
    t.heard(m(2), Presence::Online, NOW - 90_000, NOW);
    assert_eq!(t.state_of(&m(2), NOW), Presence::Online);
}

#[test]
fn newest_report_wins() {
    let mut t = PresenceTable::new();
    t.heard(m(1), Presence::Away, NOW, NOW + 10_000);
    t.heard(m(1), Presence::Online, NOW + 5_000, NOW + 10_000);
    t.heard(m(1), Presence::Away, NOW + 1_000, NOW + 10_000);
    assert_eq!(t.state_of(&m(1), NOW + 10_000), Presence::Online);
}

#[test]
fn future_seen_is_clamped() {
    let mut t = PresenceTable::new();
    t.heard(m(1), Presence::Online, NOW + 3_600_000, NOW);
    assert_eq!(t.state_of(&m(1), NOW), Presence::Online);
    // It was recorded as "now": stale 90 s later, not an hour later.
    assert_eq!(t.state_of(&m(1), NOW + 90_001), Presence::Offline);
}

#[test]
fn relay_list_has_only_fresh() {
    let mut t = PresenceTable::new();
    for i in 1..=3 { t.heard(m(i), Presence::Online, NOW, NOW); }
    for i in 4..=5 { t.heard(m(i), Presence::Away, NOW - 200_000, NOW); }
    assert_eq!(t.relay_list(NOW).len(), 3);
}

#[test]
fn frame_round_trip() {
    let relay = vec![(m(1), Presence::Online, NOW), (m(2), Presence::Away, NOW - 5)];
    let body = encode(Presence::Away, &relay);
    assert_eq!(decode(&body).unwrap(), (Presence::Away, relay));
    // Junk is an error, not a panic.
    assert!(decode(&[]).is_err());
    assert!(decode(&[9, 0, 0]).is_err());
    assert!(decode(&[1, 0, 51]).is_err());
    let mut long = encode(Presence::Online, &[]);
    long.push(0);
    assert!(decode(&long).is_err());
}

#[tokio::test]
async fn two_hop_presence() {
    let (hub, clk, a, b, c, space) = trio().await;
    let am = member_of(&a, &space);
    hub.cut(a.key, c.key, true);
    eventually("c lost a", || c.svc.link_count() == 1 && a.svc.link_count() == 1).await;
    // Everything heard so far goes stale.
    clk.fetch_add(100_000, Ordering::SeqCst);
    assert_eq!(seen_by(&c, &space, &am), "offline");
    a.svc.presence_tick();
    eventually("b hears a", || seen_by(&b, &space, &am) == "online").await;
    assert_eq!(seen_by(&c, &space, &am), "offline", "c is not linked to a");
    // One tick later b tells c what it heard.
    clk.fetch_add(30_000, Ordering::SeqCst);
    b.svc.presence_tick();
    eventually("c sees a through b", || seen_by(&c, &space, &am) == "online").await;
}

#[tokio::test]
async fn hidden_sends_nothing() {
    let (_hub, clk, a, b, space) = pair().await;
    let am = member_of(&a, &space);
    eventually("b heard a at link time", || seen_by(&b, &space, &am) == "online").await;
    a.svc.set_show_online(false).unwrap();
    let before = a.svc.presence_written();
    a.svc.presence_tick();
    a.svc.set_my_presence("away").unwrap();
    a.svc.set_my_presence("online").unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(a.svc.presence_written(), before, "a hidden PC writes no Presence frame");
    clk.fetch_add(90_001, Ordering::SeqCst);
    assert_eq!(seen_by(&b, &space, &am), "offline");
    assert_eq!(a.svc.view()["showOnline"], false);
    // The choice is kept in spaces/settings.json (the service dir).
    let saved = std::fs::read_to_string(a.dir.0.join("settings.json")).unwrap();
    assert!(saved.contains("\"showOnline\":false"), "{saved}");
    // Turning it on tells people again.
    a.svc.set_show_online(true).unwrap();
    eventually("b hears a again", || seen_by(&b, &space, &am) == "online").await;
}

#[tokio::test]
async fn away_state_reaches_others() {
    let (_hub, _clk, a, b, space) = pair().await;
    let am = member_of(&a, &space);
    a.svc.set_my_presence("away").unwrap();
    eventually("b sees away", || seen_by(&b, &space, &am) == "away").await;
    a.svc.set_my_presence("online").unwrap();
    eventually("b sees online", || seen_by(&b, &space, &am) == "online").await;
    assert!(a.svc.set_my_presence("busy").is_err());
}

#[tokio::test]
async fn presence_about_strangers_is_ignored() {
    let (_hub, _clk, a, b, space) = pair().await;
    let (bm, stranger) = (member_of(&b, &space), m(0x77));
    let now = NOW;
    let body = encode(Presence::Online, &[(stranger, Presence::Online, now), (m(0x78), Presence::Away, now)]);
    a.svc.on_presence(&space, b.key, &body);
    assert_eq!(a.svc.presence_of(&space, &stranger, now), "offline");
    assert_eq!(a.svc.presence_of(&space, &m(0x78), now), "offline");
    assert_eq!(a.svc.presence_of(&space, &bm, now), "online");
    // A frame from a node that is not in the space says nothing at all.
    let outsider = super::ids::NodeKey([0x55; 32]);
    a.svc.on_presence(&space, outsider, &encode(Presence::Online, &[]));
    assert_eq!(a.svc.presence_of(&space, &m(0x55), now), "offline");
}

#[tokio::test]
async fn presence_is_not_written_to_the_database() {
    let (_hub, _clk, a, b, space) = pair().await;
    let am = member_of(&a, &space);
    a.svc.set_my_presence("away").unwrap();
    eventually("b sees away", || seen_by(&b, &space, &am) == "away").await;
    let tables: Vec<String> = b
        .svc
        .with_open(&space, |sp| {
            let mut st = sp.store.conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").unwrap();
            st.query_map([], |r| r.get::<_, String>(0)).unwrap().flatten().collect()
        })
        .unwrap();
    assert!(!tables.iter().any(|t| t.contains("presence")), "{tables:?}");
}
