use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::ids::NodeKey;
use super::memnet::MemoryHub;
use super::net::{NetError, P2pSpaceNet, PeerHint, SpaceNet, SpacePermissions};
use crate::keys::KeyStore;
use crate::p2p::capability::{MemoryPermissions, Permissions, Registry, RoutedPermissions};
use crate::p2p::endpoint::{NodeId, CLOSE_NOT_ALLOWED};
use crate::p2p::identity::DeviceIdentity;
use crate::p2p::memory::MemoryNet;
use crate::p2p::P2p;
use crate::secrets::{MemoryBackend, SecretBackend};

const WAIT: Duration = Duration::from_secs(5);

struct Side {
    net: Arc<P2pSpaceNet>,
    perms: Arc<SpacePermissions>,
    node: NodeKey,
}

fn side(mem: &MemoryNet, name: &str) -> Side {
    let backend: Arc<dyn SecretBackend> = Arc::new(MemoryBackend::new());
    let keys = KeyStore::with_backend(backend, name);
    let node = NodeKey(DeviceIdentity::load(&keys).unwrap().id().0);
    let registry = Arc::new(Registry::new());
    let routed = RoutedPermissions::new(Arc::new(MemoryPermissions::new()));
    let p2p = Arc::new(P2p::new(keys, mem.factory(), registry, routed.clone()));
    p2p.apply_settings(&json!({ "spaces_enabled": true }));
    let perms = SpacePermissions::new();
    let net = P2pSpaceNet::register(p2p, &routed, perms.clone()).unwrap();
    Side { net, perms, node }
}

fn hint(n: NodeKey) -> PeerHint { PeerHint { node: n, relay: String::new(), lan: Vec::new() } }

async fn accepted(a: &Side) -> Option<NodeKey> {
    tokio::time::timeout(Duration::from_millis(500), a.net.accept()).await.ok().flatten().map(|l| l.peer())
}

#[tokio::test]
async fn stranger_is_refused_when_no_invite_is_open() {
    let mem = MemoryNet::new();
    let (a, b) = (side(&mem, "a"), side(&mem, "b"));
    a.net.start().await.unwrap();
    b.net.start().await.unwrap();
    if let Ok(link) = b.net.dial(&hint(a.node)).await {
        assert_eq!(tokio::time::timeout(WAIT, link.closed()).await.unwrap(), CLOSE_NOT_ALLOWED);
    }
    assert_eq!(accepted(&a).await, None);
}

#[tokio::test]
async fn stranger_is_admitted_while_an_invite_is_open() {
    let mem = MemoryNet::new();
    let (a, b) = (side(&mem, "a"), side(&mem, "b"));
    a.net.start().await.unwrap();
    b.net.start().await.unwrap();
    a.perms.set_open_invites(1);
    b.net.dial(&hint(a.node)).await.unwrap();
    assert_eq!(accepted(&a).await, Some(b.node));
}

#[tokio::test]
async fn member_node_is_admitted_without_invites() {
    let mem = MemoryNet::new();
    let (a, b) = (side(&mem, "a"), side(&mem, "b"));
    a.net.start().await.unwrap();
    b.net.start().await.unwrap();
    a.perms.set_nodes([b.node]);
    b.net.dial(&hint(a.node)).await.unwrap();
    assert_eq!(accepted(&a).await, Some(b.node));
}

#[tokio::test]
async fn closing_the_invite_refuses_new_strangers() {
    let mem = MemoryNet::new();
    let (a, c) = (side(&mem, "a"), side(&mem, "c"));
    a.net.start().await.unwrap();
    c.net.start().await.unwrap();
    a.perms.set_open_invites(1);
    a.perms.set_open_invites(0);
    if let Ok(link) = c.net.dial(&hint(a.node)).await {
        assert_eq!(tokio::time::timeout(WAIT, link.closed()).await.unwrap(), CLOSE_NOT_ALLOWED);
    }
    assert_eq!(accepted(&a).await, None);
}

#[test]
fn space_permissions_never_answer_for_other_capabilities() {
    let p = SpacePermissions::new();
    p.set_nodes([NodeKey([2; 32])]);
    assert!(!p.allowed(&NodeId([2; 32]), "desk/input"));
    assert!(p.allowed(&NodeId([2; 32]), "spaces"));
}

#[test]
fn peer_set_is_a_no_op() {
    let p = SpacePermissions::new();
    p.set(&NodeId([2; 32]), "spaces", true);
    assert!(!p.allows(&NodeKey([2; 32])));
}

#[tokio::test]
async fn memnet_refuses_unknown_and_cut_peers() {
    let hub = MemoryHub::new();
    let (ka, kb) = (NodeKey([1; 32]), NodeKey([2; 32]));
    let a = hub.node(ka, SpacePermissions::new());
    let pb = SpacePermissions::new();
    pb.set_nodes([ka]);
    let _b = hub.node(kb, pb);
    assert_eq!(a.dial(&hint(NodeKey([9; 32]))).await.err(), Some(NetError::Unreachable));
    hub.cut(ka, kb, true);
    assert_eq!(a.dial(&hint(kb)).await.err(), Some(NetError::Unreachable));
}

#[tokio::test]
async fn memnet_stream_carries_bytes_both_ways() {
    let hub = MemoryHub::new();
    let (ka, kb) = (NodeKey([1; 32]), NodeKey([2; 32]));
    let a = hub.node(ka, SpacePermissions::new());
    let pb = SpacePermissions::new();
    pb.set_nodes([ka]);
    let b = hub.node(kb, pb);
    let link = a.dial(&hint(kb)).await.unwrap();
    let theirs = b.accept().await.unwrap();
    let mut s = link.open().await.unwrap();
    s.send.write_all(b"hello").await.unwrap();
    let mut t = theirs.accept_stream().await.unwrap();
    let mut got = [0u8; 5];
    t.recv.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"hello");
    t.send.write_all(b"world").await.unwrap();
    s.recv.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"world");
}

// ---- the Labs switch decides whether anything exists at all ----

mod runtime_switch {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;

    use crate::keys::KeyStore as LinkKeys;
    use crate::p2p::memory::MemoryNet;
    use crate::secrets::{MemoryBackend, SecretBackend};
    use crate::spaces::net::AppLink;
    use crate::spaces::runtime::{self, MakeParts, Parts, Runtime, Spawner};
    use crate::spaces::keystore::KeyStore as TeamKeys;

    fn parts(mem: &MemoryNet, dir: &std::path::Path) -> Parts {
        let backend: Arc<dyn SecretBackend> = Arc::new(MemoryBackend::new());
        let link = AppLink::new(LinkKeys::with_backend(backend.clone(), "rt"), mem.factory()).unwrap();
        Parts { link, keystore: TeamKeys::with_backend(backend, "rt"), dir: dir.to_path_buf(), clock: Arc::new(|| 1_700_000_000_000) }
    }

    fn temp(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("pm-spaces-rt-{tag}-{}", std::process::id()))
    }

    /// Not a tokio test on purpose: a thread or task started by mistake has no runtime to run on and would panic.
    #[test]
    fn switch_off_builds_nothing_binds_nothing_and_starts_no_task() {
        let mem = MemoryNet::new();
        let (makes, spawns) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (m, mem2, mk) = (makes.clone(), mem.clone(), temp("off"));
        let make: MakeParts = Arc::new(move || {
            m.fetch_add(1, Ordering::SeqCst);
            Ok(parts(&mem2, &mk))
        });
        let s = spawns.clone();
        let spawn: Spawner = Arc::new(move |_f| {
            s.fetch_add(1, Ordering::SeqCst);
        });
        let rt = Runtime::new(make, spawn);
        rt.apply_settings(&json!({}));
        rt.apply_settings(&json!({ "spaces_enabled": false }));
        rt.apply_settings(&json!({ "spaces_enabled": "yes" }));
        assert_eq!(makes.load(Ordering::SeqCst), 0, "the link layer was built");
        assert_eq!(spawns.load(Ordering::SeqCst), 0, "a task was started");
        assert!(!rt.link_built() && !rt.endpoint_running() && rt.service().is_none());
        assert_eq!(mem.bind_count(), 0, "an endpoint was bound");
        // The app's own hook returns before it makes the runtime object.
        runtime::apply_settings(&json!({ "spaces_enabled": false }));
        runtime::apply_settings(&json!({}));
        assert!(!runtime::app_runtime_exists());
    }

    #[tokio::test]
    async fn switch_on_starts_one_endpoint_and_off_closes_it() {
        let mem = MemoryNet::new();
        let dir = temp("on");
        let (mem2, d) = (mem.clone(), dir.clone());
        let make: MakeParts = Arc::new(move || Ok(parts(&mem2, &d)));
        let spawn: Spawner = Arc::new(|f| {
            tokio::spawn(f);
        });
        let rt = Runtime::new(make, spawn);
        rt.apply_settings(&json!({ "spaces_enabled": true }));
        for _ in 0..400 {
            if rt.service().is_some() && rt.endpoint_running() { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(rt.service().is_some() && rt.endpoint_running());
        assert_eq!(mem.bind_count(), 1);
        rt.apply_settings(&json!({ "spaces_enabled": true }));
        assert_eq!(mem.bind_count(), 1, "a second start must not bind again");
        rt.apply_settings(&json!({ "spaces_enabled": false }));
        for _ in 0..400 {
            if rt.service().is_none() && !rt.endpoint_running() { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(rt.service().is_none() && !rt.endpoint_running());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
