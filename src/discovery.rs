use crate::peer_registry::{PeerRegistry, PeerSnapshot, PeerUpsert};
use crate::probe;
use crate::protocol::SERVICE_TYPE;
use anyhow::{Context, Result};
use if_addrs::IfAddr;
use mdns_sd::{Receiver, ServiceDaemon, ServiceEvent, ServiceInfo};
use rand::Rng;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Debounce window before flushing peers after a network interface change.
const NETWORK_CHANGE_DEBOUNCE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiscoveredPeer {
    pub device_id: String,
    pub name: String,
    pub addr: IpAddr,
    pub port: u16,
}

pub struct Discovery {
    daemon: ServiceDaemon,
    service_name: String,
    browse_rx: Receiver<ServiceEvent>,
    registry: Arc<Mutex<PeerRegistry>>,
    device_id: String,
    display_name: String,
    port: u16,
    session_epoch: u64,
    last_addrs: Vec<IpAddr>,
    network_change_at: Option<Instant>,
}

impl Discovery {
    pub fn start(device_id: &str, name: &str, port: u16) -> Result<Self> {
        let session_epoch = rand::thread_rng().gen::<u64>();
        let daemon = ServiceDaemon::new().context("start mDNS daemon")?;
        let service_name = format!("NewTowerRelay-{}", &device_id[..device_id.len().min(8)]);
        let last_addrs = local_ipv4_addrs();
        let info = build_service_info(
            device_id,
            name,
            port,
            session_epoch,
            last_addrs.as_slice(),
            &service_name,
        )?;
        daemon.register(info).context("register mDNS service")?;
        let browse_rx = daemon.browse(SERVICE_TYPE).context("browse mDNS")?;
        Ok(Self {
            daemon,
            service_name,
            browse_rx,
            registry: Arc::new(Mutex::new(PeerRegistry::new())),
            device_id: device_id.to_string(),
            display_name: name.to_string(),
            port,
            session_epoch,
            last_addrs,
            network_change_at: None,
        })
    }

    /// For unit tests: registry handle without starting mDNS.
    #[cfg(test)]
    pub fn registry(&self) -> Arc<Mutex<PeerRegistry>> {
        Arc::clone(&self.registry)
    }

    #[cfg(test)]
    pub fn session_epoch(&self) -> u64 {
        self.session_epoch
    }

    /// Drain mDNS events, run probes, and return the current visible peer list.
    pub fn poll_peers(&mut self, own_device_id: &str) -> Result<Vec<PeerSnapshot>> {
        self.handle_network_change()?;

        while let Ok(event) = self.browse_rx.recv_timeout(Duration::from_millis(50)) {
            self.handle_event(event);
        }

        let now = Instant::now();
        self.run_probes(own_device_id, now);

        let mut registry = self.registry.lock().expect("peer registry lock");
        registry.evict_expired(now);
        registry.evict_pending_removals(now);
        Ok(registry.visible_peers(now, own_device_id))
    }

    fn handle_network_change(&mut self) -> Result<()> {
        let current = local_ipv4_addrs();
        if current == self.last_addrs {
            self.network_change_at = None;
            return Ok(());
        }

        let now = Instant::now();
        if self.network_change_at.is_none() {
            self.network_change_at = Some(now);
            return Ok(());
        }

        if now.duration_since(self.network_change_at.unwrap()) < NETWORK_CHANGE_DEBOUNCE {
            return Ok(());
        }

        self.last_addrs = current.clone();
        self.network_change_at = None;

        self.reregister(&current)
    }

    pub fn rescan(&mut self) -> Result<()> {
        {
            let mut registry = self.registry.lock().expect("peer registry lock");
            registry.clear();
        }
        self.last_addrs = local_ipv4_addrs();
        self.network_change_at = None;
        self.session_epoch = rand::thread_rng().gen();
        self.reregister(&self.last_addrs.clone())?;
        while let Ok(event) = self.browse_rx.recv_timeout(Duration::from_millis(50)) {
            self.handle_event(event);
        }
        Ok(())
    }

    fn reregister(&mut self, addrs: &[IpAddr]) -> Result<()> {
        self.daemon.unregister(&self.service_name).ok();
        let info = build_service_info(
            &self.device_id,
            &self.display_name,
            self.port,
            self.session_epoch,
            addrs,
            &self.service_name,
        )?;
        self.daemon.register(info).context("re-register mDNS service")?;
        Ok(())
    }

    fn run_probes(&self, own_device_id: &str, now: Instant) {
        let targets = {
            let registry = self.registry.lock().expect("peer registry lock");
            registry.peers_for_probe(now, own_device_id)
        };
        for (device_id, addr) in targets {
            let ok = probe::probe_peer(addr, own_device_id);
            let mut registry = self.registry.lock().expect("peer registry lock");
            registry.record_probe_result(&device_id, ok, now);
        }
    }

    fn handle_event(&self, event: ServiceEvent) {
        let now = Instant::now();
        let mut registry = self.registry.lock().expect("peer registry lock");
        match event {
            ServiceEvent::ServiceResolved(info) => {
                if let Some((addr, port)) = pick_addr(&info) {
                    let fullname = info.get_fullname().to_string();
                    let device_id = info
                        .get_property("device_id")
                        .map(|v| v.val_str().to_string())
                        .unwrap_or_else(|| fullname.clone());
                    let name = info
                        .get_property("name")
                        .map(|v| v.val_str().to_string())
                        .unwrap_or_else(|| fullname.clone());
                    let session_epoch = info
                        .get_property("session_epoch")
                        .and_then(|v| v.val_str().parse().ok())
                        .unwrap_or(0);
                    registry.upsert(
                        PeerUpsert {
                            device_id,
                            name,
                            addr,
                            port,
                            mdns_fullname: fullname,
                            session_epoch,
                        },
                        now,
                    );
                }
            }
            ServiceEvent::ServiceRemoved(_ty, fullname) => {
                registry.mark_removed_by_fullname(&fullname, now);
            }
            _ => {}
        }
    }

    pub fn stop(self) -> Result<()> {
        self.daemon.unregister(&self.service_name).ok();
        Ok(())
    }
}

fn build_service_info(
    device_id: &str,
    name: &str,
    port: u16,
    session_epoch: u64,
    addrs: &[IpAddr],
    service_name: &str,
) -> Result<ServiceInfo> {
    let host = format!("{}.local.", sanitize(device_id));
    let properties = HashMap::from([
        ("device_id".to_string(), device_id.to_string()),
        ("name".to_string(), name.to_string()),
        ("session_epoch".to_string(), session_epoch.to_string()),
    ]);
    ServiceInfo::new(
        SERVICE_TYPE,
        service_name,
        &host,
        addrs,
        port,
        Some(properties),
    )
    .context("build service info")
}

pub fn pick_listen_port() -> Result<(TcpListener, u16)> {
    let listener = TcpListener::bind(("0.0.0.0", 0)).context("bind tcp")?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

fn pick_addr(info: &ServiceInfo) -> Option<(IpAddr, u16)> {
    let port = info.get_port();
    let addrs: Vec<IpAddr> = info
        .get_addresses()
        .iter()
        .copied()
        .filter(IpAddr::is_ipv4)
        .collect();
    if addrs.is_empty() {
        return None;
    }
    if let Some(addr) = prefer_local_subnet(&addrs, &local_ipv4_addrs()) {
        return Some((addr, port));
    }
    Some((addrs[0], port))
}

fn prefer_local_subnet(candidates: &[IpAddr], local: &[IpAddr]) -> Option<IpAddr> {
    for local_ip in local {
        let IpAddr::V4(local_v4) = local_ip else {
            continue;
        };
        let local_prefix = &local_v4.octets()[..3];
        for candidate in candidates {
            if let IpAddr::V4(candidate_v4) = candidate {
                if &candidate_v4.octets()[..3] == local_prefix {
                    return Some(*candidate);
                }
            }
        }
    }
    None
}

pub fn local_ipv4_addrs() -> Vec<IpAddr> {
    let mut addrs = Vec::new();
    if let Ok(interfaces) = if_addrs::get_if_addrs() {
        for iface in interfaces {
            if iface.is_loopback() {
                continue;
            }
            if let IfAddr::V4(v4) = iface.addr {
                addrs.push(IpAddr::V4(v4.ip));
            }
        }
    }
    addrs.sort();
    if addrs.is_empty() {
        addrs.push(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
    }
    addrs
}

fn sanitize(input: &str) -> String {
    input
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr};

    #[test]
    fn registry_dedupes_by_device_id_via_shared_state() {
        let registry = Arc::new(Mutex::new(PeerRegistry::new()));
        let now = Instant::now();
        {
            let mut reg = registry.lock().unwrap();
            reg.upsert(
                PeerUpsert {
                    device_id: "dev-1".into(),
                    name: "Laptop".into(),
                    addr: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
                    port: 8000,
                    mdns_fullname: "old-instance.local".into(),
                    session_epoch: 1,
                },
                now,
            );
            reg.upsert(
                PeerUpsert {
                    device_id: "dev-1".into(),
                    name: "Laptop".into(),
                    addr: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
                    port: 8001,
                    mdns_fullname: "new-instance.local".into(),
                    session_epoch: 2,
                },
                now,
            );
        }
        let reg = registry.lock().unwrap();
        let peers = reg.visible_peers(now, "self");
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].addr, SocketAddr::from(([10, 0, 0, 2], 8001)));
    }

    #[test]
    fn remove_by_fullname_clears_device() {
        let registry = Arc::new(Mutex::new(PeerRegistry::new()));
        let now = Instant::now();
        {
            let mut reg = registry.lock().unwrap();
            reg.upsert(
                PeerUpsert {
                    device_id: "dev-1".into(),
                    name: "Laptop".into(),
                    addr: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
                    port: 8000,
                    mdns_fullname: "laptop.local".into(),
                    session_epoch: 1,
                },
                now,
            );
            assert_eq!(
                reg.remove_by_fullname("laptop.local"),
                Some("dev-1".into())
            );
        }
        let reg = registry.lock().unwrap();
        assert!(reg.visible_peers(now, "self").is_empty());
    }

    #[test]
    fn local_addrs_are_sorted_for_stable_comparison() {
        let a = local_ipv4_addrs();
        let mut b = a.clone();
        b.sort();
        assert_eq!(a, b);
    }

    #[test]
    fn prefer_local_subnet_picks_lan_address() {
        let candidates = vec![
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
        ];
        let local = vec![IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))];
        assert_eq!(
            prefer_local_subnet(&candidates, &local),
            Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)))
        );
    }

    #[test]
    fn rescan_clears_registry_and_bumps_epoch() {
        let (_, port) = pick_listen_port().expect("pick port");
        let mut discovery =
            Discovery::start("test-rescan-device", "RescanTest", port).expect("start discovery");
        let epoch_before = discovery.session_epoch();

        {
            let registry = discovery.registry();
            let mut reg = registry.lock().unwrap();
            let now = Instant::now();
            reg.upsert(
                PeerUpsert {
                    device_id: "peer-1".into(),
                    name: "Peer".into(),
                    addr: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
                    port: 9000,
                    mdns_fullname: "peer.local".into(),
                    session_epoch: 1,
                },
                now,
            );
            assert_eq!(reg.len(), 1);
        }

        discovery.rescan().expect("rescan");

        assert_ne!(discovery.session_epoch(), epoch_before);
        let peers = discovery
            .registry()
            .lock()
            .unwrap()
            .visible_peers(Instant::now(), "test-rescan-device");
        assert!(
            !peers.iter().any(|p| p.device_id == "peer-1"),
            "rescan should clear manually inserted peer; remaining: {:?}",
            peers.iter().map(|p| &p.device_id).collect::<Vec<_>>()
        );
    }
}
