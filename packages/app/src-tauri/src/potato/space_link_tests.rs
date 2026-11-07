//! Potato Bot and Spaces, wired together: a line typed in a space reaches the bot's brain, the brain's answer is posted in the
//! space as the bot's, and nothing is written to the bot's own thread. With the link cut, nothing moves.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::protocol::parse_event;
use super::service_tests::Rig;
use super::space_link;
use crate::secrets::MemoryBackend;
use crate::spaces::ids::{NodeKey, SpaceId};
use crate::spaces::keystore::KeyStore;
use crate::spaces::memnet::MemoryHub;
use crate::spaces::net::SpacePermissions;
use crate::spaces::service::SpacesService;

struct Dir(std::path::PathBuf);
impl Drop for Dir {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

async fn spaces(tag: &str) -> (Arc<SpacesService>, Dir, SpaceId) {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = Dir(std::env::temp_dir().join(format!("pm-link-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst))));
    let _ = std::fs::remove_dir_all(&dir.0);
    let hub = MemoryHub::new();
    let perms = SpacePermissions::new();
    let net = hub.node(NodeKey([1; 32]), perms.clone());
    let keystore = KeyStore::with_backend(Arc::new(MemoryBackend::new()), "link");
    let svc = SpacesService::new(net, perms, keystore, dir.0.clone(), Arc::new(|| 1_700_000_000_000));
    svc.start_with(false).await.unwrap();
    let space = SpaceId::from_hex(&svc.create_space("Book club").await.unwrap()).unwrap();
    std::mem::forget(hub);
    (svc, dir, space)
}

fn general(svc: &SpacesService, space: &SpaceId) -> String {
    svc.with_open(space, |sp| sp.roster.channels[0].id.to_hex()).unwrap()
}

fn space_items(rig: &Rig) -> Vec<Value> {
    rig.inbox(0).0.into_iter().filter(|i| i["type"] == "space").collect()
}

#[tokio::test]
async fn a_line_in_the_space_reaches_the_brain_and_the_answer_is_posted_back() {
    let rig = Rig::new("link_round");
    let (svc, _dir, space) = spaces("round").await;
    space_link::connect(&rig.svc, &svc);
    let hex = space.to_hex();
    // the bot can be brought in: it is a candidate, and it joins under the name typed
    let cands = svc.bot_candidates(&hex).unwrap();
    assert_eq!(cands["candidates"][0]["id"], "nimbus");
    svc.bot_add(&hex, "nimbus", "Ren的fish").unwrap();
    let ch = general(&svc, &space);
    svc.send_text(&hex, &ch, "@Ren的fish what is the plan?", None).await.unwrap();
    let items = space_items(&rig);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["turn"], "owner");
    assert_eq!(items[0]["conversationId"], format!("app:space:{hex}:{ch}").as_str());
    rig.hello();
    let id = items[0]["id"].as_str().unwrap().to_string();
    rig.svc.ingest(
        "nimbus",
        vec![parse_event(&json!({"v":1,"type":"reply","conversationId":items[0]["conversationId"],"inReplyTo":id,"segments":["Tea at six."]})).unwrap()],
    );
    let msgs = svc.messages(&hex, &ch, None, 10).unwrap();
    let last = &msgs.as_array().unwrap()[1];
    assert_eq!((last["text"].clone(), last["isFish"].clone(), last["botName"].clone()), (json!("Tea at six."), json!(true), json!("Ren的fish")));
    // nothing went to the bot's own thread
    assert!(rig.svc.messages("nimbus", 50).unwrap().is_empty());
    // an answer that looks private is held back at the space, and the count moves
    svc.send_text(&hex, &ch, "@Ren的fish where do you live?", None).await.unwrap();
    let second = space_items(&rig).into_iter().find(|i| i["line"]["text"] == "@Ren的fish where do you live?").unwrap();
    rig.svc.ingest(
        "nimbus",
        vec![parse_event(&json!({"v":1,"type":"reply","conversationId":second["conversationId"],"inReplyTo":second["id"],"segments":["write to sam@example.org"]})).unwrap()],
    );
    assert_eq!(svc.messages(&hex, &ch, None, 10).unwrap().as_array().unwrap().len(), 3, "the two lines and the first answer, nothing more");
    assert_eq!(svc.bot_list(&hex).unwrap()["bots"][0]["heldBackToday"], 1);
}

#[tokio::test]
async fn with_the_link_cut_nothing_moves() {
    let rig = Rig::new("link_cut");
    let (svc, _dir, space) = spaces("cut").await;
    space_link::connect(&rig.svc, &svc);
    let hex = space.to_hex();
    svc.bot_add(&hex, "nimbus", "Ren的fish").unwrap();
    let ch = general(&svc, &space);
    // Spaces goes off: the service stops, and with it the port
    svc.stop().await;
    rig.svc.set_space_port(None);
    assert!(svc.bot_add(&hex, "nimbus", "Other").is_err());
    assert!(space_items(&rig).is_empty());
    let _ = ch;
    assert_eq!(rig.spawned(), 0, "no brain was asked for");
}

#[tokio::test]
async fn nothing_is_connected_by_default() {
    let rig = Rig::new("link_default");
    let (svc, _dir, space) = spaces("default").await;
    let hex = space.to_hex();
    // no link: no bot can be added and a line goes to nobody
    assert!(svc.bot_candidates(&hex).is_err());
    assert!(svc.bot_add(&hex, "nimbus", "x").is_err());
    let ch = general(&svc, &space);
    svc.send_text(&hex, &ch, "@nimbus hello", None).await.unwrap();
    assert!(space_items(&rig).is_empty());
}
