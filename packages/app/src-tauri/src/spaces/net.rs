//! The only Spaces file that names the link layer. Everything above talks to SpaceNet / SpaceLink.
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, Mutex};

use super::ids::NodeKey;
use crate::p2p::capability::{
    self, Capability, CapabilityHandler, MemoryPermissions, Permissions, RelayPolicy, Registry, RoutedPermissions,
};
use crate::p2p::endpoint::{Link, NodeId, PathKind, PeerAddr, TransportFactory};
use crate::p2p::iroh_net::IrohFactory;
use crate::p2p::{alpn, P2p};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerHint { pub node: NodeKey, pub relay: String, pub lan: Vec<String> }

pub struct SpaceStream {
    pub send: Box<dyn AsyncWrite + Send + Unpin>,
    pub recv: Box<dyn AsyncRead + Send + Unpin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError { Off, Unreachable, Refused, Closed }

#[async_trait]
pub trait SpaceLink: Send + Sync {
    fn peer(&self) -> NodeKey;
    fn relayed(&self) -> bool;
    async fn open(&self) -> Result<SpaceStream, NetError>;
    async fn accept_stream(&self) -> Result<SpaceStream, NetError>;
    fn close(&self, code: u32, reason: &str);
    async fn closed(&self) -> u32;
}

#[async_trait]
pub trait SpaceNet: Send + Sync {
    fn me(&self) -> Option<NodeKey>;
    fn my_hint(&self) -> Option<PeerHint>;
    async fn start(&self) -> Result<(), NetError>;
    async fn stop(&self);
    async fn dial(&self, peer: &PeerHint) -> Result<Arc<dyn SpaceLink>, NetError>;
    async fn accept(&self) -> Option<Arc<dyn SpaceLink>>;
}

/// Who may open a `spaces` link to this PC: member nodes of my spaces, mailboxes, and anyone while an invite is open.
pub struct SpacePermissions {
    nodes: RwLock<BTreeSet<[u8; 32]>>,
    open_invites: AtomicUsize,
}

impl SpacePermissions {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { nodes: RwLock::new(BTreeSet::new()), open_invites: AtomicUsize::new(0) })
    }
    pub fn set_nodes(&self, nodes: impl IntoIterator<Item = NodeKey>) {
        if let Ok(mut n) = self.nodes.write() {
            *n = nodes.into_iter().map(|k| k.0).collect();
        }
    }
    pub fn set_open_invites(&self, count: usize) { self.open_invites.store(count, Ordering::SeqCst); }
    pub fn is_known(&self, node: &NodeKey) -> bool { self.nodes.read().map(|n| n.contains(&node.0)).unwrap_or(false) }
    pub fn allows(&self, node: &NodeKey) -> bool { self.is_known(node) || self.open_invites.load(Ordering::SeqCst) > 0 }
}

impl Permissions for SpacePermissions {
    fn allowed(&self, peer: &NodeId, cap: &str) -> bool { cap == capability::SPACES && self.allows(&NodeKey(peer.0)) }
    /// A peer never sets permissions; Spaces changes them only through set_nodes / set_open_invites.
    fn set(&self, _peer: &NodeId, _cap: &str, _on: bool) {}
}

struct P2pLink(Arc<dyn Link>);

#[async_trait]
impl SpaceLink for P2pLink {
    fn peer(&self) -> NodeKey { NodeKey(self.0.peer().0) }
    fn relayed(&self) -> bool { self.0.path() == PathKind::Relayed }
    async fn open(&self) -> Result<SpaceStream, NetError> {
        let s = self.0.open_bi().await.map_err(|_| NetError::Closed)?;
        Ok(SpaceStream { send: s.send, recv: s.recv })
    }
    async fn accept_stream(&self) -> Result<SpaceStream, NetError> {
        let s = self.0.accept_bi().await.map_err(|_| NetError::Closed)?;
        Ok(SpaceStream { send: s.send, recv: s.recv })
    }
    fn close(&self, code: u32, reason: &str) { self.0.close(code, reason) }
    async fn closed(&self) -> u32 { self.0.closed().await }
}

struct SpaceHandler { tx: mpsc::Sender<Arc<dyn Link>> }

#[async_trait]
impl CapabilityHandler for SpaceHandler {
    async fn serve(&self, _peer: NodeId, link: Arc<dyn Link>) {
        let _ = self.tx.try_send(link);
    }
}

pub struct P2pSpaceNet {
    p2p: Arc<P2p>,
    inbox: Mutex<mpsc::Receiver<Arc<dyn Link>>>,
}

impl P2pSpaceNet {
    /// Register the `spaces` capability (relays allowed) and route its permissions to `perms`.
    pub fn register(p2p: Arc<P2p>, routed: &Arc<RoutedPermissions>, perms: Arc<SpacePermissions>)
        -> Result<Arc<Self>, String>
    {
        let (tx, rx) = mpsc::channel(64);
        let handler = Arc::new(SpaceHandler { tx });
        let cap = Capability::new(capability::SPACES, alpn::SPACES, RelayPolicy::Allowed, handler);
        p2p.registry().register(cap).map_err(|e| format!("{e:?}"))?;
        routed.route(capability::SPACES, perms);
        Ok(Arc::new(Self { p2p, inbox: Mutex::new(rx) }))
    }
}

#[async_trait]
impl SpaceNet for P2pSpaceNet {
    fn me(&self) -> Option<NodeKey> { self.p2p.node_id().map(|n| NodeKey(n.0)) }
    fn my_hint(&self) -> Option<PeerHint> {
        let node = self.me()?;
        let lan = self.p2p.local_addrs().iter().take(4).map(|a| a.to_string()).collect();
        Some(PeerHint { node, relay: self.p2p.home_relay().unwrap_or_default(), lan })
    }
    async fn start(&self) -> Result<(), NetError> {
        self.p2p.want(capability::SPACES).await.map(|_| ()).map_err(|_| NetError::Off)
    }
    async fn stop(&self) { self.p2p.unwant(capability::SPACES).await }
    async fn dial(&self, peer: &PeerHint) -> Result<Arc<dyn SpaceLink>, NetError> {
        let mut addr = PeerAddr::new(NodeId(peer.node.0)).with_relay(&peer.relay);
        addr.direct = peer.lan.iter().filter_map(|a| a.parse().ok()).collect();
        let link = self.p2p.dial(&addr, capability::SPACES).await.map_err(|_| NetError::Unreachable)?;
        Ok(Arc::new(P2pLink(link)))
    }
    async fn accept(&self) -> Option<Arc<dyn SpaceLink>> {
        let link = self.inbox.lock().await.recv().await?;
        Some(Arc::new(P2pLink(link)))
    }
}

/// The app's link layer for Spaces: one `P2p` whose permissions are wrapped in `RoutedPermissions`, with the
/// `spaces` capability registered. Building it starts nothing: no endpoint, no socket, no thread. The endpoint
/// starts only when `net().start()` runs, and `P2p` refuses that while `spaces_enabled` is off.
pub struct AppLink {
    p2p: Arc<P2p>,
    net: Arc<P2pSpaceNet>,
    perms: Arc<SpacePermissions>,
}

impl AppLink {
    /// `keys` holds the node key (`crate::keys::KeyStore`); `factory` is the network (tests pass the in-memory one).
    pub fn new(keys: crate::keys::KeyStore, factory: Arc<dyn TransportFactory>) -> Result<Self, String> {
        let registry = Arc::new(Registry::new());
        let routed = RoutedPermissions::new(Arc::new(MemoryPermissions::new()));
        let p2p = Arc::new(P2p::new(keys, factory, registry, routed.clone()));
        let perms = SpacePermissions::new();
        let net = P2pSpaceNet::register(p2p.clone(), &routed, perms.clone())?;
        Ok(Self { p2p, net, perms })
    }

    pub fn net(&self) -> Arc<dyn SpaceNet> { self.net.clone() }
    pub fn perms(&self) -> Arc<SpacePermissions> { self.perms.clone() }
    /// Tells the link layer the Labs switches. Both off closes the endpoint.
    pub fn apply_settings(&self, public: &serde_json::Value) { self.p2p.apply_settings(public) }
    pub fn running(&self) -> bool { self.p2p.is_running() }
}

/// The app's real link layer: the iroh network and the key from the OS credential store (`keys/`).
pub fn app_space_net() -> Result<AppLink, String> {
    AppLink::new(crate::keys::KeyStore::app(), Arc::new(IrohFactory))
}
