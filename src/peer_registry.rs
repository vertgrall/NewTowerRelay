use crate::probe::{PROBE_FAILURES_OFFLINE, PROBE_FRESH, PROBE_INTERVAL};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

/// How long without a fresh mDNS resolve before a peer is removed entirely.
pub const PEER_TTL: Duration = Duration::from_secs(60);
/// How long without a resolve before a peer is shown as stale (still listed).
pub const STALE_AFTER: Duration = Duration::from_secs(15);
/// Grace period after mDNS ServiceRemoved before evicting a peer.
pub const REMOVAL_GRACE: Duration = STALE_AFTER;
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
    /// True when a recent TCP probe succeeded.
    pub reachable: bool,
}

#[derive(Debug, Clone)]
struct PeerRecord {
    device_id: String,
    name: String,
    addr: IpAddr,
    port: u16,
    session_epoch: u64,
    first_seen: Instant,
    last_seen: Instant,
    last_probe_ok: Option<Instant>,
    probe_failures: u8,
    /// Set when mDNS reports ServiceRemoved; peer evicted after [`REMOVAL_GRACE`].
    removed_at: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerUpsert {
    pub device_id: String,
    pub name: String,
    pub addr: IpAddr,
    pub port: u16,
    pub mdns_fullname: String,
    pub session_epoch: u64,
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

    pub fn clear(&mut self) {
        self.peers.clear();
        self.fullname_index.clear();
    }

    /// Insert or refresh a peer keyed by `device_id` (never by mDNS fullname).
    /// Ignores announcements with an older [`PeerUpsert::session_epoch`].
    pub fn upsert(&mut self, peer: PeerUpsert, now: Instant) -> bool {
        if let Some(existing) = self.peers.get(&peer.device_id) {
            if peer.session_epoch < existing.session_epoch {
                return false;
            }
        }

        self.fullname_index
            .retain(|_, device_id| device_id != &peer.device_id);
        self.fullname_index
            .insert(peer.mdns_fullname.clone(), peer.device_id.clone());

        match self.peers.get_mut(&peer.device_id) {
            Some(record) => {
                record.name = peer.name;
                record.addr = peer.addr;
                record.port = peer.port;
                record.session_epoch = peer.session_epoch;
                record.last_seen = now;
                record.removed_at = None;
                record.probe_failures = 0;
            }
            None => {
                self.peers.insert(
                    peer.device_id.clone(),
                    PeerRecord {
                        device_id: peer.device_id,
                        name: peer.name,
                        addr: peer.addr,
                        port: peer.port,
                        session_epoch: peer.session_epoch,
                        first_seen: now,
                        last_seen: now,
                        last_probe_ok: None,
                        probe_failures: 0,
                        removed_at: None,
                    },
                );
            }
        }
        true
    }

    /// Mark a peer for deferred removal after mDNS ServiceRemoved (transient re-registrations).
    pub fn mark_removed_by_fullname(&mut self, mdns_fullname: &str, now: Instant) -> bool {
        let Some(device_id) = self.fullname_index.get(mdns_fullname).cloned() else {
            return false;
        };
        if let Some(record) = self.peers.get_mut(&device_id) {
            record.removed_at = Some(now);
            return true;
        }
        false
    }

    pub fn remove_by_fullname(&mut self, mdns_fullname: &str) -> Option<String> {
        let device_id = self.fullname_index.remove(mdns_fullname)?;
        self.peers.remove(&device_id);
        Some(device_id)
    }

    pub fn remove_by_device_id(&mut self, device_id: &str) -> bool {
        if self.peers.remove(device_id).is_some() {
            self.fullname_index.retain(|_, id| id != device_id);
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

    /// Drop peers marked removed by mDNS that have exceeded [`REMOVAL_GRACE`].
    pub fn evict_pending_removals(&mut self, now: Instant) -> Vec<String> {
        let expired: Vec<String> = self
            .peers
            .iter()
            .filter(|(_, record)| {
                record
                    .removed_at
                    .is_some_and(|t| now.duration_since(t) > REMOVAL_GRACE)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired {
            self.remove_by_device_id(id);
        }
        expired
    }

    pub fn record_probe_result(&mut self, device_id: &str, ok: bool, now: Instant) {
        let Some(record) = self.peers.get_mut(device_id) else {
            return;
        };
        if ok {
            record.last_probe_ok = Some(now);
            record.probe_failures = 0;
        } else {
            record.probe_failures = record.probe_failures.saturating_add(1);
        }
    }

    /// Peers due for a TCP liveness probe.
    pub fn peers_for_probe(&self, now: Instant, own_device_id: &str) -> Vec<(String, SocketAddr)> {
        self.peers
            .values()
            .filter(|r| r.device_id != own_device_id)
            .filter(|r| now.duration_since(r.last_seen) <= PEER_TTL)
            .filter(|r| {
                r.last_probe_ok
                    .map(|t| now.duration_since(t) > PROBE_INTERVAL)
                    .unwrap_or(true)
            })
            .map(|r| {
                (
                    r.device_id.clone(),
                    SocketAddr::new(r.addr, r.port),
                )
            })
            .collect()
    }

    /// Visible peers sorted by name; excludes `own_device_id`.
    pub fn visible_peers(&self, now: Instant, own_device_id: &str) -> Vec<PeerSnapshot> {
        let mut out: Vec<PeerSnapshot> = self
            .peers
            .values()
            .filter(|record| record.device_id != own_device_id)
            .filter(|record| now.duration_since(record.last_seen) <= PEER_TTL)
            .map(|record| {
                let reachable = record
                    .last_probe_ok
                    .map(|t| now.duration_since(t) <= PROBE_FRESH)
                    .unwrap_or(false);
                PeerSnapshot {
                    device_id: record.device_id.clone(),
                    name: record.name.clone(),
                    addr: SocketAddr::new(record.addr, record.port),
                    presence: presence_at(record, now),
                    is_new: now.duration_since(record.first_seen) <= NEW_PEER_WINDOW,
                    reachable,
                }
            })
            .collect();
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out
    }
}

fn presence_at(record: &PeerRecord, now: Instant) -> PeerPresence {
    if record.probe_failures >= PROBE_FAILURES_OFFLINE {
        return PeerPresence::Stale;
    }
    if record
        .last_probe_ok
        .is_some_and(|t| now.duration_since(t) <= PROBE_FRESH)
    {
        return PeerPresence::Online;
    }
    if now.duration_since(record.last_seen) <= STALE_AFTER {
        return PeerPresence::Online;
    }
    PeerPresence::Stale
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
        session_epoch: u64,
        now: Instant,
    ) {
        registry.upsert(
            PeerUpsert {
                device_id: device_id.to_string(),
                name: name.to_string(),
                addr: IpAddr::V4(Ipv4Addr::from(ip)),
                port,
                mdns_fullname: fullname.to_string(),
                session_epoch,
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
            1,
            now,
        );
        upsert(
            &mut registry,
            "dev-a",
            "MacBook",
            [192, 168, 1, 20],
            9001,
            "instance-a.v2",
            2,
            now,
        );
        assert_eq!(registry.len(), 1);
        let peers = registry.visible_peers(now, "self");
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].addr, SocketAddr::from(([192, 168, 1, 20], 9001)));
    }

    #[test]
    fn ignores_older_session_epoch() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "dev-a",
            "MacBook",
            [192, 168, 1, 20],
            9001,
            "new.local",
            5,
            now,
        );
        upsert(
            &mut registry,
            "dev-a",
            "Ghost",
            [192, 168, 1, 99],
            9000,
            "old.local",
            2,
            now,
        );
        let peers = registry.visible_peers(now, "self");
        assert_eq!(peers[0].addr.port(), 9001);
        assert_eq!(peers[0].name, "MacBook");
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
            1,
            now,
        );
        upsert(
            &mut registry,
            "dev-b",
            "Beta",
            [192, 168, 1, 3],
            9000,
            "b.local",
            1,
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
            1,
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
            1,
            now,
        );
        let fresh = registry.visible_peers(now, "self");
        assert_eq!(fresh[0].presence, PeerPresence::Online);
    }

    #[test]
    fn probe_failures_mark_stale() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            1,
            now,
        );
        for _ in 0..PROBE_FAILURES_OFFLINE {
            registry.record_probe_result("dev-a", false, now);
        }
        let peer = &registry.visible_peers(now, "self")[0];
        assert_eq!(peer.presence, PeerPresence::Stale);
        assert!(!peer.reachable);
    }

    #[test]
    fn successful_probe_marks_reachable() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            1,
            now,
        );
        registry.record_probe_result("dev-a", true, now);
        let peer = &registry.visible_peers(now, "self")[0];
        assert!(peer.reachable);
        assert_eq!(peer.presence, PeerPresence::Online);
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
            1,
            t(61),
        );
        upsert(
            &mut registry,
            "dev-b",
            "PC",
            [192, 168, 1, 6],
            9000,
            "pc.local",
            1,
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
            1,
            now,
        );
        assert_eq!(
            registry.remove_by_fullname("mac.local"),
            Some("dev-a".to_string())
        );
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
            1,
            now,
        );
        upsert(
            &mut registry,
            "other",
            "Other",
            [192, 168, 1, 2],
            9000,
            "other.local",
            1,
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
            1,
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
            1,
            t(61),
        );
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            1,
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
            1,
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
            1,
            t(30),
        );
        let peer = &registry.visible_peers(now, "self")[0];
        assert_eq!(peer.presence, PeerPresence::Stale);
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
            1,
            now,
        );
        upsert(
            &mut registry,
            "a",
            "Alpha",
            [192, 168, 1, 1],
            9000,
            "a.local",
            1,
            now,
        );
        let names: Vec<_> = registry
            .visible_peers(now, "self")
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, vec!["Alpha", "Zulu"]);
    }

    #[test]
    fn upsert_resets_probe_failures() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            1,
            now,
        );
        for _ in 0..PROBE_FAILURES_OFFLINE {
            registry.record_probe_result("dev-a", false, now);
        }
        assert_eq!(
            registry.visible_peers(now, "self")[0].presence,
            PeerPresence::Stale
        );

        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            1,
            now,
        );
        assert_eq!(
            registry.visible_peers(now, "self")[0].presence,
            PeerPresence::Online
        );
    }

    #[test]
    fn mark_removed_defers_eviction_until_grace_expires() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            1,
            now,
        );
        let recent_mark = Instant::now() - Duration::from_secs(5);
        assert!(registry.mark_removed_by_fullname("mac.local", recent_mark));
        assert_eq!(registry.visible_peers(now, "self").len(), 1);
        registry.evict_pending_removals(Instant::now());
        assert_eq!(registry.visible_peers(now, "self").len(), 1);

        let stale_mark = Instant::now() - REMOVAL_GRACE - Duration::from_secs(1);
        registry.mark_removed_by_fullname("mac.local", stale_mark);
        registry.evict_pending_removals(Instant::now());
        assert!(registry.visible_peers(Instant::now(), "self").is_empty());
    }

    #[test]
    fn upsert_clears_pending_removal() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            1,
            now,
        );
        registry.mark_removed_by_fullname("mac.local", t(10));
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            1,
            now,
        );
        registry.evict_pending_removals(t(20));
        assert_eq!(registry.visible_peers(now, "self").len(), 1);
    }

    #[test]
    fn clear_removes_all_peers() {
        let mut registry = PeerRegistry::new();
        let now = Instant::now();
        upsert(
            &mut registry,
            "dev-a",
            "Mac",
            [192, 168, 1, 5],
            9000,
            "mac.local",
            1,
            now,
        );
        registry.clear();
        assert!(registry.is_empty());
    }
}
