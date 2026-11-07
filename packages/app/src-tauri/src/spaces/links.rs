//! Live links: at most one per peer node, plus the per-(space, peer) writers that pushes go through, and the
//! redial backoff. Plain data behind the service's mutex; no I/O here.
use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use super::ids::{NodeKey, SpaceId};
use super::limits::MAX_LINKS;
use super::net::SpaceLink;

/// A frame waiting to be written on a sync stream: (type, body).
pub type Outgoing = (u8, Vec<u8>);

const BACKOFF_FIRST_MS: u64 = 5_000;
const BACKOFF_MAX_MS: u64 = 5 * 60 * 1000;

struct Held {
    link: Arc<dyn SpaceLink>,
    dialed: bool,
    id: u64,
}

struct Writer {
    id: u64,
    tx: UnboundedSender<Outgoing>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// Kept; the id names this link in `forget`.
    Kept(u64),
    /// Another link to this peer wins; the offered one was closed.
    Dropped,
}

#[derive(Default)]
pub struct LinkTable {
    links: BTreeMap<NodeKey, Held>,
    writers: BTreeMap<(SpaceId, NodeKey), Writer>,
    /// node -> (do not dial before, last delay)
    backoff: BTreeMap<NodeKey, (u64, u64)>,
    next_id: u64,
}

impl LinkTable {
    pub fn new() -> Self { Self::default() }

    pub fn len(&self) -> usize { self.links.len() }
    pub fn has(&self, peer: &NodeKey) -> bool { self.links.contains_key(peer) }
    pub fn full(&self) -> bool { self.links.len() >= MAX_LINKS }
    pub fn peers(&self) -> Vec<NodeKey> { self.links.keys().copied().collect() }
    pub fn all_relayed(&self) -> bool { !self.links.is_empty() && self.links.values().all(|h| h.link.relayed()) }
    pub fn link(&self, peer: &NodeKey) -> Option<Arc<dyn SpaceLink>> { self.links.get(peer).map(|h| h.link.clone()) }

    /// At most one link per peer. If a second arrives, keep the one dialed by the smaller node key (both sides apply
    /// the same rule, so they agree). Two links dialed from the same side: the newer one replaces the older.
    pub fn offer(&mut self, me: NodeKey, link: Arc<dyn SpaceLink>, dialed: bool) -> Offer {
        let peer = link.peer();
        self.next_id += 1;
        let id = self.next_id;
        if !self.links.contains_key(&peer) && self.full() {
            link.close(0, "full");
            return Offer::Dropped;
        }
        if let Some(old) = self.links.get(&peer) {
            let dialer_of = |d: bool| if d { me } else { peer };
            let (old_dialer, new_dialer) = (dialer_of(old.dialed), dialer_of(dialed));
            let keep_new = old_dialer == new_dialer || new_dialer < old_dialer;
            if !keep_new {
                link.close(0, "duplicate");
                return Offer::Dropped;
            }
            old.link.close(0, "duplicate");
            self.writers.retain(|(_, p), _| *p != peer);
        }
        self.links.insert(peer, Held { link, dialed, id });
        Offer::Kept(id)
    }

    /// The link `id` to `peer` ended. Does nothing if a newer link replaced it.
    pub fn forget(&mut self, peer: &NodeKey, id: u64) {
        if self.links.get(peer).map(|h| h.id) == Some(id) {
            self.links.remove(peer);
            self.writers.retain(|(_, p), _| p != peer);
        }
    }

    pub fn close_all(&mut self) {
        for h in self.links.values() { h.link.close(0, "stopping"); }
        self.links.clear();
        self.writers.clear();
    }

    pub fn add_writer(&mut self, space: SpaceId, peer: NodeKey, id: u64, tx: UnboundedSender<Outgoing>) {
        self.writers.insert((space, peer), Writer { id, tx });
    }

    pub fn remove_writer(&mut self, space: SpaceId, peer: NodeKey, id: u64) {
        if self.writers.get(&(space, peer)).map(|w| w.id) == Some(id) {
            self.writers.remove(&(space, peer));
        }
    }

    /// Stop pushing this space to `peer` (frames already queued are still written). The link itself stays.
    pub fn drop_writer(&mut self, space: &SpaceId, peer: &NodeKey) { self.writers.remove(&(*space, *peer)); }

    pub fn has_writer(&self, space: &SpaceId) -> bool { self.writers.keys().any(|(s, _)| s == space) }

    /// Stop pushing this space to anyone.
    pub fn drop_space_writers(&mut self, space: &SpaceId) { self.writers.retain(|(s, _), _| s != space); }

    /// Queue a frame for every live writer of `space` except `except`. Returns how many got it.
    pub fn send_to_space(&self, space: &SpaceId, except: Option<NodeKey>, kind: u8, body: &[u8]) -> usize {
        let mut n = 0;
        for ((s, p), w) in &self.writers {
            if s == space && Some(*p) != except && w.tx.send((kind, body.to_vec())).is_ok() { n += 1; }
        }
        n
    }

    pub fn may_dial(&self, node: &NodeKey, now_ms: u64) -> bool {
        self.backoff.get(node).map(|(until, _)| now_ms >= *until).unwrap_or(true)
    }

    /// 5 s, doubling, at most 5 minutes.
    pub fn dial_failed(&mut self, node: NodeKey, now_ms: u64) {
        let delay = match self.backoff.get(&node) {
            Some((_, last)) => (last * 2).min(BACKOFF_MAX_MS),
            None => BACKOFF_FIRST_MS,
        };
        self.backoff.insert(node, (now_ms + delay, delay));
    }

    pub fn dial_ok(&mut self, node: &NodeKey) { self.backoff.remove(node); }
}
