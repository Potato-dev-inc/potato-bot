//! Real network, one process: two Spaces service nodes talk over real iroh endpoints on this PC's loopback.
//! No relay and no public server: the factory below forces `relays = false` and loopback-only binding, and
//! the peer is reached by the 127.0.0.1 address in its invite. Everything secret is in memory.
//!
//! Run by hand (it binds real UDP sockets on 127.0.0.1 with random ports):
//!   cargo test --lib spaces::local_net_tests -- --ignored --nocapture
//! This does not replace the two-computer test D0.4.
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::json;

use super::net::AppLink;
use super::service::SpacesService;
use super::service_tests::TempDir;
use crate::keys::KeyStore as LinkKeys;
use crate::p2p::endpoint::{BindSpec, P2pError, Transport, TransportFactory};
use crate::p2p::identity::DeviceIdentity;
use crate::p2p::iroh_net::IrohFactory;
use crate::secrets::{MemoryBackend, SecretBackend};
use crate::spaces::keystore::KeyStore as TeamKeys;

/// The real iroh transport, with relays off and the socket on 127.0.0.1 only.
struct LocalIroh;

#[async_trait]
impl TransportFactory for LocalIroh {
    async fn bind(&self, identity: &DeviceIdentity, spec: &BindSpec) -> Result<Arc<dyn Transport>, P2pError> {
        let mut local = spec.clone();
        local.relays = false;
        local.loopback_only = true;
        IrohFactory.bind(identity, &local).await
    }
}

fn now_ms() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0) }

struct Side {
    svc: Arc<SpacesService>,
    link: AppLink,
    _dir: TempDir,
}

async fn side(name: &str) -> Side {
    let backend: Arc<dyn SecretBackend> = Arc::new(MemoryBackend::new());
    let link = AppLink::new(LinkKeys::with_backend(backend.clone(), name), Arc::new(LocalIroh)).unwrap();
    link.apply_settings(&json!({ "spaces_enabled": true }));
    let dir = TempDir::new(&format!("iroh-{name}"));
    let svc = SpacesService::new(
        link.net(), link.perms(), TeamKeys::with_backend(backend, name), dir.0.clone(), Arc::new(now_ms),
    );
    svc.start().await.expect("the endpoint should start on loopback");
    Side { svc, link, _dir: dir }
}

async fn wait_for(what: &str, secs: u64, f: impl Fn() -> bool) {
    let end = tokio::time::Instant::now() + Duration::from_secs(secs);
    while tokio::time::Instant::now() < end {
        if f() { return; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out after {secs}s waiting for: {what}");
}

fn first_channel(s: &Side, space: &str) -> String {
    s.svc.detail(space).unwrap()["channels"][0]["id"].as_str().unwrap().to_string()
}

fn texts(s: &Side, space: &str) -> Vec<String> {
    let ch = first_channel(s, space);
    s.svc
        .messages(space, &ch, None, 50)
        .map(|v| v.as_array().unwrap().iter().map(|m| m["text"].as_str().unwrap_or("").to_string()).collect())
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "binds real iroh endpoints on 127.0.0.1; run by hand with --ignored"]
async fn two_nodes_talk_over_real_iroh_on_loopback() {
    let (a, b) = (side("a").await, side("b").await);
    assert!(a.link.running() && b.link.running(), "both endpoints are up");

    // create, invite, join
    let space = a.svc.create_space("Local proof").await.unwrap();
    let invite = a.svc.create_invite(&space).await.unwrap();
    let link_text = invite["link"].as_str().unwrap().to_string();
    let words: Vec<String> = invite["words"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect();
    b.svc.start_join(&link_text, &words.join(" ")).await.unwrap();
    wait_for("a join request on a", 30, || !a.svc.view()["requests"].as_array().unwrap().is_empty()).await;
    let request = a.svc.view()["requests"][0]["id"].as_i64().unwrap();
    a.svc.answer_join(&space, request, true).await.unwrap();
    wait_for("b is in the space", 30, || b.svc.view()["joining"][0]["state"] == "joined").await;
    wait_for("a live link on both sides", 30, || a.svc.link_count() == 1 && b.svc.link_count() == 1).await;
    // The link never touched a relay.
    assert_eq!(a.svc.view()["relayed"], false);
    assert_eq!(b.svc.view()["relayed"], false);

    // send and receive, both ways
    let (ach, bch) = (first_channel(&a, &space), first_channel(&b, &space));
    a.svc.send_text(&space, &ach, "hello from a", None).await.unwrap();
    wait_for("b gets a's message", 30, || texts(&b, &space).contains(&"hello from a".to_string())).await;
    b.svc.send_text(&space, &bch, "hello from b", None).await.unwrap();
    wait_for("a gets b's message", 30, || texts(&a, &space).contains(&"hello from b".to_string())).await;

    // one file: offered by a, fetched by b only when asked, every chunk checked
    let tmp = TempDir::new("iroh-file");
    std::fs::create_dir_all(&tmp.0).unwrap();
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let path = tmp.0.join("proof.bin");
    std::fs::write(&path, &data).unwrap();
    let offered = a.svc.offer_file(&space, &ach, &path.to_string_lossy()).await.unwrap();
    let sha = offered["file"]["sha"].as_str().unwrap().to_string();
    wait_for("b sees the file card", 30, || {
        b.svc.messages(&space, &bch, None, 50).map(|v| v.as_array().unwrap().iter().any(|m| m["file"]["sha"] == sha.as_str())).unwrap_or(false)
    })
    .await;
    let before = b.svc.messages(&space, &bch, None, 50).unwrap();
    let card = before.as_array().unwrap().iter().find(|m| !m["file"].is_null()).unwrap();
    assert_eq!(card["file"]["have"], false, "nothing is downloaded by itself");
    b.svc.get_file(&space, &sha).await.unwrap();
    let downloads = tmp.0.join("Downloads");
    let saved = b.svc.save_file(&space, &sha, &downloads).unwrap();
    assert_eq!(saved["name"], "proof.bin");
    assert_eq!(std::fs::read(downloads.join("proof.bin")).unwrap(), data);

    a.svc.stop().await;
    b.svc.stop().await;
    let _ = std::fs::remove_dir_all(&tmp.0);
}
