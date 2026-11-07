//! Remove, leave, forget (D5.1) and block (D5.2) on services talking over the in-memory net: A owner, B, C.
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::frame::{read_frame, write_frame};
use super::ids::{space_tag, MemberKey, SpaceId};
use super::memnet::MemoryHub;
use super::roster::{OpBody, RemoveReason};
use super::service_tests::{clock, eventually, general, join, make, node, pair, say, texts, trio, Node};
use super::sync;

fn member_of(n: &Node, space: &SpaceId) -> MemberKey { n.svc.with_open(space, |sp| sp.me.member_key()).unwrap() }

fn state(n: &Node, space: &SpaceId) -> String {
    n.svc.detail(&space.to_hex()).map(|d| d["state"].as_str().unwrap_or("").to_string()).unwrap_or_default()
}

fn roster_info(n: &Node, space: &SpaceId) -> (usize, u32) {
    n.svc.with_open(space, |sp| (sp.roster.members.len(), sp.roster.epoch)).unwrap()
}

#[tokio::test]
async fn remove_ops_are_remove_then_rotate() {
    let (_hub, _clk, a, b, space) = pair().await;
    let bm = member_of(&b, &space);
    let (roster, me) = a.svc.with_open(&space, |sp| (sp.roster.clone(), sp.me.clone())).unwrap();
    let ops = super::moderation::remove_ops(&roster, &me, &bm, RemoveReason::Removed, &[7u8; 32], 1_700_000_000_000).unwrap();
    assert!(matches!(ops[0].body, OpBody::RemoveMember { member, reason: RemoveReason::Removed } if member == bm));
    assert!(matches!(&ops[1].body, OpBody::Rotate { epoch: 2, grants } if grants.len() == 1));
    let mut after = roster.clone();
    after.apply(&ops[0]).unwrap();
    after.apply(&ops[1]).unwrap();
    assert_eq!((after.members.len(), after.epoch), (1, 2));
}

#[tokio::test]
async fn removed_member_cannot_open_new_epoch() {
    let (_hub, _clk, a, b, c, space) = trio().await;
    let bm = member_of(&b, &space);
    a.svc.remove_member(&space.to_hex(), &bm.to_hex()).await.unwrap();
    say(&a, &space, "after").await;
    eventually("c reads after", || texts(&c, &space).contains(&"after".to_string())).await;
    eventually("b learns it", || state(&b, &space) == "removed").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!texts(&b, &space).contains(&"after".to_string()));
    assert!(b.svc.with_open(&space, |sp| sp.roster.open_epoch_key(&sp.me, 2).is_none() && !sp.keys.0.contains_key(&2)).unwrap());
    assert_eq!(roster_info(&c, &space), (2, 2));
}

#[tokio::test]
async fn removed_member_learns_it() {
    let (_hub, clk, a, b, space) = pair().await;
    a.svc.remove_member(&space.to_hex(), &member_of(&b, &space).to_hex()).await.unwrap();
    eventually("b is removed", || state(&b, &space) == "removed").await;
    eventually("links closed", || a.svc.link_count() == 0 && b.svc.link_count() == 0).await;
    clk.fetch_add(10 * 60 * 1000, Ordering::SeqCst);
    b.svc.dial_round().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(b.svc.link_count(), 0);
}

#[tokio::test]
async fn removed_node_is_refused_after_remove() {
    let (_hub, _clk, a, b, space) = pair().await;
    assert!(a.svc.allows_node(&b.key));
    a.svc.remove_member(&space.to_hex(), &member_of(&b, &space).to_hex()).await.unwrap();
    assert!(!a.svc.allows_node(&b.key));
}

#[tokio::test]
async fn member_in_another_space_keeps_a_link() {
    let (_hub, clk, a, b, space) = pair().await;
    let s2 = a.svc.create_space("Chess").await.unwrap();
    join(&a, &b, &s2).await;
    // The join dialed a second link and replaced the first: let B redial, then prove S1 syncs again.
    clk.fetch_add(10 * 60 * 1000, Ordering::SeqCst);
    b.svc.dial_round().await;
    eventually("one link again", || a.svc.link_count() == 1 && b.svc.link_count() == 1).await;
    say(&a, &space, "before").await;
    eventually("b gets before", || texts(&b, &space).contains(&"before".to_string())).await;
    a.svc.remove_member(&space.to_hex(), &member_of(&b, &space).to_hex()).await.unwrap();
    eventually("b is removed from S1", || state(&b, &space) == "removed").await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(a.svc.link_count(), 1);
    assert_eq!(b.svc.link_count(), 1);
    assert!(a.svc.allows_node(&b.key));
    // B asks A for S1 anyway: that one stream is refused with Bye(3); the link stays.
    let hello = b.svc.hello_bytes(&space).unwrap();
    let link = b.svc.link_to(&a.key).unwrap();
    let mut stream = link.open().await.unwrap();
    write_frame(&mut stream.send, sync::HELLO, &hello).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(2), read_frame(&mut stream.recv)).await.unwrap().unwrap();
    assert_eq!((reply.kind, reply.body), (sync::BYE, vec![3]));
    assert_eq!(a.svc.link_count(), 1);
}

#[tokio::test]
async fn leave_auto_rotates_on_owner() {
    let (_hub, _clk, a, b, space) = pair().await;
    b.svc.leave(&space.to_hex()).await.unwrap();
    assert_eq!(state(&b, &space), "left");
    eventually("a rotated", || roster_info(&a, &space) == (1, 2)).await;
    assert!(a.svc.view()["requests"].as_array().unwrap().is_empty());
    assert_eq!(a.svc.detail(&space.to_hex()).unwrap()["leavePending"], false);
    // B's copy stays readable and cannot write.
    assert!(b.svc.send_text(&space.to_hex(), &general(&b, &space).to_hex(), "hi", None).await.is_err());
}

#[tokio::test]
async fn owner_cannot_leave() {
    let (_hub, _clk, a, _b, space) = pair().await;
    assert_eq!(a.svc.leave(&space.to_hex()).await.unwrap_err(), "You own this space. Owners can't leave in this version.");
}

#[tokio::test]
async fn forget_needs_left_or_removed() {
    let (_hub, _clk, _a, b, space) = pair().await;
    assert_eq!(b.svc.forget(&space.to_hex()).unwrap_err(), "Leave the space first.");
}

#[tokio::test]
async fn forget_deletes_folder_and_keys() {
    let (_hub, _clk, a, b, space) = pair().await;
    let folder = b.dir.0.join(space.to_hex());
    assert!(folder.exists());
    assert!(b.keystore.epoch_key(&space_tag(&space), 1).unwrap().is_some());
    a.svc.remove_member(&space.to_hex(), &member_of(&b, &space).to_hex()).await.unwrap();
    eventually("b is removed", || state(&b, &space) == "removed").await;
    b.svc.forget(&space.to_hex()).unwrap();
    assert!(!folder.exists());
    assert_eq!(b.keystore.epoch_key(&space_tag(&space), 1).unwrap(), None);
    assert!(b.svc.view()["spaces"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn only_owner_removes() {
    let (_hub, _clk, _a, b, c, space) = trio().await;
    let err = b.svc.remove_member(&space.to_hex(), &member_of(&c, &space).to_hex()).await.unwrap_err();
    assert_eq!(err, "Only the owner can remove people.");
    assert_eq!(roster_info(&b, &space).0, 3);
}

#[tokio::test]
async fn rotation_key_is_saved_before_ops() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    let b = node(&hub, 2, &clk).await;
    let hex = a.svc.create_space("Book club").await.unwrap();
    join(&a, &b, &hex).await;
    let space = SpaceId::from_hex(&hex).unwrap();
    let bm = member_of(&b, &space).to_hex();
    // The store refuses the ops: the key for epoch 2 is already in the keystore, the roster did not change.
    a.svc.with_open(&space, |sp| {
        sp.store
            .conn
            .execute_batch("CREATE TRIGGER no_ops BEFORE INSERT ON control_ops BEGIN SELECT RAISE(ABORT, 'full'); END;")
            .unwrap();
    });
    assert!(a.svc.remove_member(&hex, &bm).await.is_err());
    assert_eq!(roster_info(&a, &space), (2, 1));
    assert!(a.keystore.epoch_key(&space_tag(&space), 2).unwrap().is_some());
    // Retry works, and after a restart the owner still opens epoch 2.
    a.svc.with_open(&space, |sp| sp.store.conn.execute_batch("DROP TRIGGER no_ops;").unwrap());
    a.svc.remove_member(&hex, &bm).await.unwrap();
    assert_eq!(roster_info(&a, &space), (1, 2));
    a.svc.stop().await;
    let Node { keystore, dir, .. } = a;
    let a2 = make(&hub, 1, &clk, Some(keystore), Some(dir));
    a2.svc.start_with(false).await.unwrap();
    assert_eq!(roster_info(&a2, &space), (1, 2));
    say(&a2, &space, "still works").await;
    assert_eq!(texts(&a2, &space), vec!["still works".to_string()]);
}

// ---- block (D5.2) -----------------------------------------------------------------------------------------------

fn blocked_flag(n: &Node, space: &SpaceId, text: &str) -> Option<bool> {
    let list = n.svc.messages(&space.to_hex(), &general(n, space).to_hex(), None, 50).unwrap();
    list.as_array().unwrap().iter().find(|m| m["text"] == text).map(|m| m["blocked"].as_bool().unwrap())
}

#[tokio::test]
async fn block_is_local_only() {
    let (_hub, _clk, a, b, c, space) = trio().await;
    say(&b, &space, "hello").await;
    eventually("a and c have it", || texts(&a, &space).len() == 1 && texts(&c, &space).len() == 1).await;
    let bm = member_of(&b, &space).to_hex();
    let before = a.svc.with_open(&space, |sp| (sp.roster.seq, sp.store.heads().unwrap())).unwrap();
    let (entries, pushes) = (super::service_tests::entry_count(&a, &space), a.svc.pushes_written());
    a.svc.block(&space.to_hex(), &bm, true).unwrap();
    assert_eq!(blocked_flag(&a, &space, "hello"), Some(true));
    assert_eq!(blocked_flag(&c, &space, "hello"), Some(false));
    assert_eq!(a.svc.detail(&space.to_hex()).unwrap()["members"].as_array().unwrap().iter()
        .filter(|m| m["blocked"] == true).count(), 1);
    // Nothing left this PC: no op, no entry, no frame; the other stores have no blocks.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(a.svc.with_open(&space, |sp| (sp.roster.seq, sp.store.heads().unwrap())).unwrap(), before);
    assert_eq!((super::service_tests::entry_count(&a, &space), a.svc.pushes_written()), (entries, pushes));
    for n in [&b, &c] {
        assert!(n.svc.with_open(&space, |sp| sp.store.blocked().unwrap().is_empty()).unwrap());
    }
    // B is not told and does not see itself as blocked.
    assert_eq!(blocked_flag(&b, &space, "hello"), Some(false));
}

#[tokio::test]
async fn blocked_entries_still_forward() {
    let (hub, _clk, a, b, c, space) = trio().await;
    hub.cut(b.key, c.key, true);
    eventually("b-c link is gone", || b.svc.link_count() == 1 && c.svc.link_count() == 1).await;
    a.svc.block(&space.to_hex(), &member_of(&b, &space).to_hex(), true).unwrap();
    say(&b, &space, "through a").await;
    eventually("c gets it through a", || texts(&c, &space).contains(&"through a".to_string())).await;
    assert_eq!(blocked_flag(&a, &space, "through a"), Some(true));
}

#[tokio::test]
async fn cannot_block_self() {
    let (_hub, _clk, a, _b, space) = pair().await;
    let err = a.svc.block(&space.to_hex(), &member_of(&a, &space).to_hex(), true).unwrap_err();
    assert_eq!(err, "You can't block yourself.");
}

#[tokio::test]
async fn unblock_shows_again() {
    let (_hub, _clk, a, b, space) = pair().await;
    say(&b, &space, "hi there").await;
    eventually("a has it", || texts(&a, &space).len() == 1).await;
    let bm = member_of(&b, &space).to_hex();
    a.svc.block(&space.to_hex(), &bm, true).unwrap();
    assert_eq!(blocked_flag(&a, &space, "hi there"), Some(true));
    a.svc.block(&space.to_hex(), &bm, false).unwrap();
    assert_eq!(blocked_flag(&a, &space, "hi there"), Some(false));
}
