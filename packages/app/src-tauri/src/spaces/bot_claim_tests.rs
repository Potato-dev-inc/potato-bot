//! A member brings their own bot: the claim is a hidden signed entry, the owner's PC publishes it through the roster's own
//! rules, and every PC ends with the same roster.
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use super::bot_bridge as bb;
use bb::{BotSink, Candidate, Delivery, Refusal};
use super::diary::DiaryHome;
use super::ids::{BotId, SpaceId};
use super::log::Body;
use super::roster::{new_bot_claim, OpBody};
use super::service_tests::{entry_count, eventually, general, make, pair, texts, trio, Node};

struct Cands(Vec<&'static str>);
impl BotSink for Cands {
    fn deliver(&self, _: &Delivery) -> Result<(), Refusal> { Err(Refusal::Unavailable) }
    fn candidates(&self) -> Vec<Candidate> { self.0.iter().map(|b| Candidate { id: b.to_string(), name: b.to_string() }).collect() }
}
impl DiaryHome for Cands {
    fn vault_of(&self, _: &str) -> Option<std::path::PathBuf> { None }
}

fn with_bots(n: &Node, bots: Vec<&'static str>) { n.svc.set_bot_port(Some(Arc::new(Cands(bots)))); }

fn roster_names(n: &Node, space: &SpaceId) -> Vec<String> {
    n.svc.with_open(space, |sp| sp.roster.bots.values().map(|b| b.name.clone()).collect::<Vec<_>>()).unwrap_or_default()
}

fn bots_of_member(n: &Node, space: &SpaceId, who: &Node) -> usize {
    let m = who.svc.identity().unwrap().member_key();
    n.svc.with_open(space, |sp| sp.roster.bots_of(&m).len()).unwrap_or(0)
}

/// A claim built by hand, as a hacked PC could send it: signed by `signer`, put in an entry by `from`.
async fn forge(from: &Node, space: &SpaceId, signer: &super::identity::SpaceIdentity, bot: BotId, name: &str) {
    let OpBody::AddBot { sig, .. } = new_bot_claim(signer, space, bot, name) else { unreachable!() };
    let ch = general(from, space);
    from.svc.send_now(space, &ch, Body::BotClaim { bot, name: name.into(), sig }, None, None).unwrap();
}

async fn settle() { tokio::time::sleep(Duration::from_millis(150)).await; }

#[tokio::test]
async fn a_members_claim_is_published_by_the_owners_pc_and_a_third_member_sees_it() {
    let (_hub, _clk, a, b, c, space) = trio().await;
    with_bots(&b, vec!["kiwi", "mochi"]);
    let hex = space.to_hex();
    let before = (texts(&a, &space).len(), texts(&c, &space).len());
    let listed = b.svc.bot_add(&hex, "kiwi", "Sam的kiwi").unwrap();
    // until the roster shows it, the member's list says so; and the add is not blocked for a member
    assert!(listed["addBlocked"].is_null());
    eventually("a publishes", || roster_names(&a, &space) == vec!["Sam的kiwi".to_string()]).await;
    eventually("c sees the bot", || roster_names(&c, &space) == vec!["Sam的kiwi".to_string()]).await;
    eventually("b sees the bot", || roster_names(&b, &space) == vec!["Sam的kiwi".to_string()]).await;
    let after = b.svc.bot_list(&hex).unwrap();
    assert_eq!(after["pendingAdd"], serde_json::json!([]));
    assert_eq!(after["bots"][0]["mine"], true);
    // it is Sam's by the signature: the roster owner is b's member key on every PC
    let bm = b.svc.identity().unwrap().member_key();
    assert_eq!(c.svc.with_open(&space, |sp| sp.roster.bot_owners.values().copied().collect::<Vec<_>>()), Some(vec![bm]));
    // hidden from the feed: no message row, no unread
    assert_eq!((texts(&a, &space).len(), texts(&c, &space).len()), before);
    assert_eq!(c.svc.detail(&hex).unwrap()["channels"][0]["unread"], 0);
}

#[tokio::test]
async fn the_member_sees_the_claim_pending_until_the_roster_shows_it() {
    let (_hub, clk, a, b, space) = pair().await;
    with_bots(&b, vec!["kiwi"]);
    let hex = space.to_hex();
    a.svc.stop().await;
    let listed = b.svc.bot_add(&hex, "kiwi", "Sam的kiwi").unwrap();
    assert_eq!(listed["pendingAdd"], serde_json::json!(["Sam的kiwi"]));
    assert!(listed["bots"].as_array().unwrap().is_empty());
    // the same bot cannot be claimed twice meanwhile, and the candidates leave it out
    assert!(b.svc.bot_add(&hex, "kiwi", "Another").unwrap_err().contains("already in this space"));
    assert!(b.svc.bot_candidates(&hex).unwrap()["candidates"].as_array().unwrap().is_empty());
    // a claim nobody answered for a week is forgotten
    clk.fetch_add(7 * 24 * 60 * 60 * 1000 + 1, Ordering::SeqCst);
    b.svc.prune_all();
    assert_eq!(b.svc.bot_list(&hex).unwrap()["pendingAdd"], serde_json::json!([]));
    drop(a);
}

#[tokio::test]
async fn over_the_number_and_a_taken_name_are_told_at_once_and_a_forged_claim_is_dropped_by_the_owner() {
    let (_hub, _clk, a, b, space) = pair().await;
    with_bots(&a, vec!["nimbus"]);
    with_bots(&b, vec!["kiwi", "mochi"]);
    let hex = space.to_hex();
    a.svc.bot_add(&hex, "nimbus", "Ren的fish").unwrap();
    eventually("b has it", || roster_names(&b, &space).len() == 1).await;
    // taken name, with look-alike letters
    assert!(b.svc.bot_add(&hex, "kiwi", "ren的F1SH").unwrap_err().contains("already used"));
    b.svc.bot_add(&hex, "kiwi", "Sam的kiwi").unwrap();
    eventually("published", || bots_of_member(&a, &space, &b) == 1).await;
    // one bot each by default: the second is refused on the member's PC...
    assert!(b.svc.bot_add(&hex, "mochi", "Sam的mochi").unwrap_err().contains("as many bots"));
    // ...and a forged one that skips that check is dropped on the owner's PC
    let before = entry_count(&a, &space);
    forge(&b, &space, &b.svc.identity().unwrap(), BotId([9; 8]), "Sam的mochi").await;
    eventually("a stored the claim", || entry_count(&a, &space) > before).await;
    settle().await;
    assert_eq!(bots_of_member(&a, &space, &b), 1, "over the number");
    // a forged claim whose name only looks like a taken one
    let before = entry_count(&a, &space);
    forge(&b, &space, &b.svc.identity().unwrap(), BotId([8; 8]), "REN的flsh").await;
    eventually("a stored it", || entry_count(&a, &space) > before).await;
    settle().await;
    assert_eq!(roster_names(&a, &space).len(), 2, "taken name");
    // after the owner raises the number, a new honest claim goes through
    a.svc.bot_rules(&hex, 2, None).unwrap();
    eventually("b has the new rule", || b.svc.with_open(&space, |sp| sp.roster.bot_rules.per_person) == Some(2)).await;
    b.svc.bot_add(&hex, "mochi", "Sam的mochi").unwrap();
    eventually("third bot", || roster_names(&a, &space).len() == 3).await;
}

#[tokio::test]
async fn a_claim_signed_by_someone_elses_key_is_dropped() {
    let (_hub, _clk, a, b, space) = pair().await;
    // b puts a claim in its entry that the owner signed (so it names the owner's key), and one signed by a stranger
    let before = entry_count(&a, &space);
    forge(&b, &space, &a.svc.identity().unwrap(), BotId([5; 8]), "Fake的fish").await;
    forge(&b, &space, &super::identity::SpaceIdentity::for_test(99), BotId([6; 8]), "Out的fish").await;
    eventually("a stored them", || entry_count(&a, &space) >= before + 2).await;
    settle().await;
    assert!(roster_names(&a, &space).is_empty());
    assert!(roster_names(&b, &space).is_empty());
}

#[tokio::test]
async fn a_drop_takes_only_the_authors_own_bot() {
    let (_hub, _clk, a, b, c, space) = trio().await;
    with_bots(&a, vec!["nimbus"]);
    with_bots(&b, vec!["kiwi"]);
    let hex = space.to_hex();
    let ren = BotId::from_hex(a.svc.bot_add(&hex, "nimbus", "Ren的fish").unwrap()["bots"][0]["id"].as_str().unwrap()).unwrap();
    b.svc.bot_add(&hex, "kiwi", "Sam的kiwi").unwrap();
    eventually("both on c", || roster_names(&c, &space).len() == 2).await;
    // b tries to drop the owner's bot by hand
    let before = entry_count(&a, &space);
    b.svc.send_now(&space, &general(&b, &space), Body::BotDrop { bot: ren }, None, None).unwrap();
    eventually("a stored the drop", || entry_count(&a, &space) > before).await;
    settle().await;
    assert_eq!(roster_names(&a, &space).len(), 2);
    assert!(b.svc.bot_remove(&hex, &ren.to_hex()).unwrap_err().contains("Only the owner"));
    // b drops its own: gone everywhere, and it stops at once on b's PC
    let kiwi = c.svc.with_open(&space, |sp| sp.roster.bots.values().find(|x| x.name == "Sam的kiwi").map(|x| x.id)).flatten().unwrap();
    b.svc.bot_remove(&hex, &kiwi.to_hex()).unwrap();
    assert_eq!(b.svc.bot_list(&hex).unwrap()["bots"].as_array().unwrap().iter().find(|x| x["id"] == kiwi.to_hex()).map(|x| x["silenced"].clone()), Some(serde_json::json!(true)));
    eventually("removed on c", || roster_names(&c, &space) == vec!["Ren的fish".to_string()]).await;
    eventually("removed on a", || roster_names(&a, &space) == vec!["Ren的fish".to_string()]).await;
    // the owner may remove any bot
    let ren_hex = ren.to_hex();
    a.svc.bot_remove(&hex, &ren_hex).unwrap();
    eventually("all gone", || roster_names(&c, &space).is_empty()).await;
}

#[tokio::test]
async fn replaying_a_claim_does_nothing() {
    let (_hub, _clk, a, b, space) = pair().await;
    with_bots(&b, vec!["kiwi"]);
    let hex = space.to_hex();
    let bm = b.svc.identity().unwrap();
    let bot = BotId([4; 8]);
    forge(&b, &space, &bm, bot, "Sam的kiwi").await;
    eventually("published", || bots_of_member(&a, &space, &b) == 1).await;
    // the very same claim sent again in a new entry: the id is on the roster already
    forge(&b, &space, &bm, bot, "Sam的kiwi").await;
    settle().await;
    assert_eq!(roster_names(&a, &space).len(), 1);
    // after it was dropped, a replay of the old claim does not bring it back
    b.svc.send_now(&space, &general(&b, &space), Body::BotDrop { bot }, None, None).unwrap();
    eventually("dropped", || roster_names(&a, &space).is_empty()).await;
    forge(&b, &space, &bm, bot, "Sam的kiwi").await;
    settle().await;
    assert!(roster_names(&a, &space).is_empty(), "an id that was on the roster is never given out again");
}

#[tokio::test]
async fn a_claim_stored_while_the_owner_was_away_is_published_when_it_syncs() {
    let (hub, clk, a, b, space) = pair().await;
    with_bots(&b, vec!["kiwi"]);
    let hex = space.to_hex();
    a.svc.stop().await;
    let Node { keystore, dir, .. } = a;
    b.svc.bot_add(&hex, "kiwi", "Sam的kiwi").unwrap();
    assert_eq!(b.svc.bot_list(&hex).unwrap()["pendingAdd"], serde_json::json!(["Sam的kiwi"]));
    // the owner comes back and the two catch up
    let a2 = make(&hub, 1, &clk, Some(keystore), Some(dir));
    a2.svc.start_with(false).await.unwrap();
    clk.fetch_add(10 * 60 * 1000, Ordering::SeqCst);
    b.svc.dial_round().await;
    a2.svc.dial_round().await;
    eventually("the owner publishes", || roster_names(&a2, &space) == vec!["Sam的kiwi".to_string()]).await;
    eventually("b sees it", || b.svc.bot_list(&hex).unwrap()["pendingAdd"] == serde_json::json!([])).await;
    assert_eq!(b.svc.bot_list(&hex).unwrap()["bots"][0]["mine"], true);
}

#[tokio::test]
async fn a_claim_from_a_member_who_was_removed_is_refused() {
    let (_hub, _clk, a, b, space) = pair().await;
    let hex = space.to_hex();
    let ident = b.svc.identity().unwrap();
    a.svc.remove_member(&hex, &ident.member_key().to_hex()).await.unwrap();
    // an old entry of b's (from before the removal) reaches the owner's handler after the removal
    let OpBody::AddBot { sig, .. } = new_bot_claim(&ident, &space, BotId([3; 8]), "Late的fish") else { unreachable!() };
    a.svc.owner_bot_asks(&space, vec![(ident.device_key(), a.svc.now_for_test(), Body::BotClaim { bot: BotId([3; 8]), name: "Late的fish".into(), sig })]);
    assert!(roster_names(&a, &space).is_empty());
    assert_eq!(bots_of_member(&a, &space, &b), 0);
}
