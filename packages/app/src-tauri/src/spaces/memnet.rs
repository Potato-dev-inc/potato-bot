//! In-process SpaceNet for tests: duplex pipes, permission checks, cut and relayed switches.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use tokio::sync::{mpsc, watch, Mutex};

use super::ids::NodeKey;
use super::net::{NetError, PeerHint, SpaceLink, SpaceNet, SpacePermissions, SpaceStream};

type Inbox = mpsc::UnboundedSender<Arc<dyn SpaceLink>>;
type Closed = Arc<watch::Sender<Option<u32>>>;

fn ordered(a: NodeKey, b: NodeKey) -> (NodeKey, NodeKey) { if a <= b { (a, b) } else { (b, a) } }

pub struct MemoryHub {
    nodes: StdMutex<BTreeMap<NodeKey, (Arc<SpacePermissions>, Inbox)>>,
    cut: StdMutex<BTreeSet<(NodeKey, NodeKey)>>,
    relayed: StdMutex<BTreeSet<(NodeKey, NodeKey)>>,
    open: StdMutex<Vec<(NodeKey, NodeKey, Closed)>>,
}

impl MemoryHub {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            nodes: StdMutex::new(BTreeMap::new()),
            cut: StdMutex::new(BTreeSet::new()),
            relayed: StdMutex::new(BTreeSet::new()),
            open: StdMutex::new(Vec::new()),
        })
    }

    pub fn node(self: &Arc<Self>, key: NodeKey, perms: Arc<SpacePermissions>) -> Arc<MemNet> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.nodes.lock().unwrap().insert(key, (perms, tx));
        Arc::new(MemNet { hub: self.clone(), me: key, inbox: Mutex::new(rx) })
    }

    /// Dials between a and b fail; open links between them close.
    pub fn cut(&self, a: NodeKey, b: NodeKey, on: bool) {
        let k = ordered(a, b);
        if on {
            self.cut.lock().unwrap().insert(k);
            for (x, y, closed) in self.open.lock().unwrap().iter() {
                if ordered(*x, *y) == k {
                    closed.send_if_modified(|v| if v.is_none() { *v = Some(0); true } else { false });
                }
            }
        } else {
            self.cut.lock().unwrap().remove(&k);
        }
    }

    /// New links between a and b report `relayed() == true`.
    pub fn set_relayed(&self, a: NodeKey, b: NodeKey, on: bool) {
        let k = ordered(a, b);
        let mut r = self.relayed.lock().unwrap();
        if on { r.insert(k); } else { r.remove(&k); }
    }
}

pub struct MemNet { hub: Arc<MemoryHub>, me: NodeKey, inbox: Mutex<mpsc::UnboundedReceiver<Arc<dyn SpaceLink>>> }

/// One side of a link: streams are tokio::io::duplex pairs passed over an mpsc channel.
pub struct MemLink {
    peer: NodeKey,
    relayed: bool,
    out: mpsc::UnboundedSender<SpaceStream>,
    inn: Mutex<mpsc::UnboundedReceiver<SpaceStream>>,
    closed: Closed,
}

impl MemLink {
    fn pair(a: NodeKey, b: NodeKey, relayed: bool) -> (Arc<MemLink>, Arc<MemLink>, Closed) {
        let (a_tx, a_rx) = mpsc::unbounded_channel();
        let (b_tx, b_rx) = mpsc::unbounded_channel();
        let closed: Closed = Arc::new(watch::channel(None).0);
        let dialer = Arc::new(MemLink { peer: b, relayed, out: b_tx, inn: Mutex::new(a_rx), closed: closed.clone() });
        let acceptor = Arc::new(MemLink { peer: a, relayed, out: a_tx, inn: Mutex::new(b_rx), closed: closed.clone() });
        (dialer, acceptor, closed)
    }

    async fn wait_closed(&self) -> u32 {
        let mut rx = self.closed.subscribe();
        let code = match rx.wait_for(|v| v.is_some()).await {
            Ok(v) => v.unwrap_or(0),
            Err(_) => 0,
        };
        code
    }
}

#[async_trait]
impl SpaceLink for MemLink {
    fn peer(&self) -> NodeKey { self.peer }
    fn relayed(&self) -> bool { self.relayed }
    async fn open(&self) -> Result<SpaceStream, NetError> {
        if self.closed.borrow().is_some() { return Err(NetError::Closed); }
        let (x, y) = tokio::io::duplex(1 << 20);
        let (xr, xw) = tokio::io::split(x);
        let (yr, yw) = tokio::io::split(y);
        self.out.send(SpaceStream { send: Box::new(yw), recv: Box::new(yr) }).map_err(|_| NetError::Closed)?;
        Ok(SpaceStream { send: Box::new(xw), recv: Box::new(xr) })
    }
    async fn accept_stream(&self) -> Result<SpaceStream, NetError> {
        if self.closed.borrow().is_some() { return Err(NetError::Closed); }
        let mut rx = self.inn.lock().await;
        tokio::select! {
            s = rx.recv() => s.ok_or(NetError::Closed),
            _ = self.wait_closed() => Err(NetError::Closed),
        }
    }
    fn close(&self, code: u32, _reason: &str) {
        self.closed.send_if_modified(|v| if v.is_none() { *v = Some(code); true } else { false });
    }
    async fn closed(&self) -> u32 { self.wait_closed().await }
}

#[async_trait]
impl SpaceNet for MemNet {
    fn me(&self) -> Option<NodeKey> { Some(self.me) }
    fn my_hint(&self) -> Option<PeerHint> { Some(PeerHint { node: self.me, relay: String::new(), lan: Vec::new() }) }
    async fn start(&self) -> Result<(), NetError> { Ok(()) }
    async fn stop(&self) {}
    async fn dial(&self, peer: &PeerHint) -> Result<Arc<dyn SpaceLink>, NetError> {
        let k = ordered(self.me, peer.node);
        if self.hub.cut.lock().unwrap().contains(&k) { return Err(NetError::Unreachable); }
        let (perms, inbox) = match self.hub.nodes.lock().unwrap().get(&peer.node) {
            Some((p, i)) => (p.clone(), i.clone()),
            None => return Err(NetError::Unreachable),
        };
        if !perms.allows(&self.me) { return Err(NetError::Refused); }
        let relayed = self.hub.relayed.lock().unwrap().contains(&k);
        let (mine, theirs, closed) = MemLink::pair(self.me, peer.node, relayed);
        self.hub.open.lock().unwrap().push((self.me, peer.node, closed));
        inbox.send(theirs).map_err(|_| NetError::Unreachable)?;
        Ok(mine)
    }
    async fn accept(&self) -> Option<Arc<dyn SpaceLink>> { self.inbox.lock().await.recv().await }
}
