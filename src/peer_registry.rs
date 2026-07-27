use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

/// How long without a fresh mDNS resolve before a peer is removed entirely.
pub const PEER_TTL: Duration = Duration::from_secs(60);
/// How long without a resolve before a peer is shown as stale (still listed).
pub const STALE_AFTER: Duration = Duration::from_secs(15);
/// How long after first discovery a peer shows the "New" badge.
pub const NEW_PEER_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerPresence {
    Online,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerSnapshot {
    pub device_id: String,
    pub name: String,
    pub addr: SocketAddr,
    pub presence: PeerPresence,
    pub is_new: bool,
}

#[derive(Debug, Clone)]
struct PeerRecord {
    device_id: String,
    name: String,
    addr: IpAddr,
    port: u16,
    first_seen: Instant,
    last_seen: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerUpsert {
    pub device_id: String,
    pub name: String,
    pub addr: IpAddr,
    pub port: u16,
    pub mdns_fullname: String,
}

#[derive(Debug, Default)]
pub struct PeerRegistry {
    peers: HashMap<String, PeerRecord>,
    /// mDNS service instance name → stable device_id
    fullname_index: HashMap<String, String>,
}

impl PeerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.peers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Insert or refresh a peer keyed by `device_id` (never by mDNS fullname).
    pub fn upsert(&mut self, peer: PeerUpsert, now: Instant) {
        self.fullname_index
            .retain(|_, device_id| device_id != &peer.device_id);
        self.fullname_index
            .insert(peer.mdns_fullname.clone(), peer.device_id.clone());

        match self.peers.get_mut(&peer.device_id) {
            Some(record) => {
                record.name = peer.name;
                record.addr = peer.addr;
                record.port = peer.port;
                record.last_seen = now;
            }
            None => {
                self.peers.insert(
                    peer.device_id.clone(),
                    PeerRecord {
                        device_id: peer.device_id,
                        name: peer.name,
                        addr: peer.addr,
                        port: peer.port,
                        first_seen: now,
                        last_seen: now,
                    },
                );
            }
        }
    }

    pub fn remove_by_fullname(&mut self, mdns_fullname: &str) -> Option<String> {
        let device_id = self.fullname_index.remove(mdns_fullname)?;
        self.peers.remove(&device_id);
        Some(device_id)
    }

    pub fn remove_by_device_id(&mut self, device_id: &str) -> bool {
        if self.peers.remove(device_id).is_some() {
            self.fullname_index
                .retain(|_, id| id != device_id);
            true
        } else {
            false
        }
    }

    /// Drop peers not seen within [`PEER_TTL`].
    pub fn evict_expired(&mut self, now: Instant) -> Vec<String> {
        let expired: Vec<String> = self
            .peers
            .iter()
            .filter(|(_, record)| now.duration_since(record.last_seen) > PEER_TTL)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired {
            self.remove_by_device_id(id);
        }
        expired
    }

    /// Visible peers sorted by name; excludes `own_device_id`.
    pub fn visible_peers(&self, now: Instant, own_device_id: &str) -> Vec<PeerSnapshot> {
        let mut out: Vec<PeerSnapshot> = self
            .peers
            .values()
            .filter(|record| record.device_id != own_device_id)
            .filter(|record| now.duration_since(record.last_seen) <= PEER_TTL)
            .map(|record| PeerSnapshot {
                device_id: record.device_id.clone(),
                name: record.name.clone(),
                addr: SocketAddr::new(record.addr, record.port),
                presence: presence_at(record.last_seen, now),
                is_new: now.duration_since(record.first_seen) <= NEW_PEER_WINDOW,
            })
            .collect();
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out
    }
}

fn presence_at(last_seen: Instant, now: Instant) -> PeerPresence {
    if now.duration_since(last_seen) > STALE_AFTER {
        PeerPresence::Stale
    } else {
        PeerPresence::Online
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr};

    fn t(secs: u64) -> Instant {
        Instant::now() - Duration::from_secs(secs)
    }

    fn upsert(
        registry: &mut PeerRegistry,
        device_id: &str,
        name: &str,
        ip: [u8; 4],
        port: u16,
        fullname: &str,
        now: Instant,
    ) {
        registry.upsert(
            PeerUpsert {
                device_id: device_id.to_string(),
                name: name.to_string(),
                addr: IpAddr::V4(Ipv4Addr::from(ip)),
                port,
                mdns_fullname: fullname.to_string(),
            },
            now,
        );
    }

    #[test]
    fn upsert_same_device_id_replaces_addr_not_duplicate() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "dev-a",
            "MacBook",
            [192, 168, 1, 10],
            9000,
            "instance-a.v1",
            now,
        );
        upsert(
            &mut registry,
            "dev-a",
            "MacBook",
            [192, 168, 1, 20],
            9001,
            "instance-a.v2",
            now,
        );
        assert_eq!(registry.len(), 1);
        let peers = registry.visible_peers(now, "self");
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].addr, SocketAddr::from(([192, 168, 1, 20], 9001)));
    }

    #[test]
    fn different_device_ids_remain_distinct() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "dev-a",
            "Alpha",
            [192, 168, 1, 2],
            9000,
            "a.local",
            now,
        );
        upsert(
            &mut registry,
            "dev-b",
            "Beta",
            [192, 168, 1, 3],
            9000,
            "b.local",
            now,
        );
        assert_eq!(registry.visible_peers(now, "self").len(), 2);
    }

    #[test]
    fn refresh_updates_last_seen_and_clears_stale() {
        let mut registry = PeerRegistry::new();
        let old = t(20);
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            old,
        );
        let now = Instant::now();
        let stale = registry.visible_peers(now, "self");
        assert_eq!(stale[0].presence, PeerPresence::Stale);

        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            now,
        );
        let fresh = registry.visible_peers(now, "self");
        assert_eq!(fresh[0].presence, PeerPresence::Online);
    }

    #[test]
    fn evict_removes_peers_past_ttl() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            t(61),
        );
        upsert(
            &mut registry,
            "dev-b",
            "PC",
            [192, 168, 1, 6],
            9000,
            "pc.local",
            now,
        );
        let removed = registry.evict_expired(now);
        assert_eq!(removed, vec!["dev-a".to_string()]);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.visible_peers(now, "self").len(), 1);
    }

    #[test]
    fn remove_by_fullname_drops_peer() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            now,
        );
        assert_eq!(registry.remove_by_fullname("mac.local"), Some("dev-a".to_string()));
        assert!(registry.is_empty());
    }

    #[test]
    fn excludes_own_device_id() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "self",
            "Me",
            [192, 168, 1, 1],
            9000,
            "self.local",
            now,
        );
        upsert(
            &mut registry,
            "other",
            "Other",
            [192, 168, 1, 2],
            9000,
            "other.local",
            now,
        );
        assert_eq!(registry.visible_peers(now, "self").len(), 1);
        assert_eq!(registry.visible_peers(now, "self")[0].device_id, "other");
    }

    #[test]
    fn new_badge_within_window() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            t(30),
        );
        assert!(registry.visible_peers(now, "self")[0].is_new);
    }

    #[test]
    fn new_badge_expires_after_window() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            t(61),
        );
        // Refresh last_seen so the peer stays listed; first_seen remains old.
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            now,
        );
        assert!(!registry.visible_peers(now, "self")[0].is_new);
    }

    #[test]
    fn online_within_stale_threshold() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            t(10),
        );
        assert_eq!(
            registry.visible_peers(now, "self")[0].presence,
            PeerPresence::Online
        );
    }

    #[test]
    fn stale_between_stale_after_and_ttl() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            t(30),
        );
        let peer = &registry.visible_peers(now, "self")[0];
        assert_eq!(peer.presence, PeerPresence::Stale);
        // Still listed until TTL evicts.
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn visible_peers_sorted_by_name() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "z",
            "Zulu",
            [192, 168, 1, 9],
            9000,
            "z.local",
            now,
        );
        upsert(
            &mut registry,
            "a",
            "Alpha",
            [192, 168, 1, 1],
            9000,
            "a.local",
            now,
        );
        let names: Vec<_> = registry
            .visible_peers(now, "self")
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, vec!["Alpha", "Zulu"]);
    }
}
