use crate::peer_registry::{PeerRegistry, PeerSnapshot, PeerUpsert};
use crate::protocol::SERVICE_TYPE;
use anyhow::{Context, Result};
use if_addrs::IfAddr;
use mdns_sd::{Receiver, ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
}

impl Discovery {
    pub fn start(device_id: &str, name: &str, port: u16) -> Result<Self> {
        let daemon = ServiceDaemon::new().context("start mDNS daemon")?;
        let host = format!("{}.local.", sanitize(device_id));
        let service_name = format!("NewTowerRelay-{}", &device_id[..device_id.len().min(8)]);
        let properties = HashMap::from([
            ("device_id".to_string(), device_id.to_string()),
            ("name".to_string(), name.to_string()),
        ]);
        let my_addrs = local_ipv4_addrs();
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            &service_name,
            &host,
            my_addrs.as_slice(),
            port,
            Some(properties),
        )
        .context("build service info")?;
        daemon.register(info).context("register mDNS service")?;
        let browse_rx = daemon.browse(SERVICE_TYPE).context("browse mDNS")?;
        Ok(Self {
            daemon,
            service_name,
            browse_rx,
            registry: Arc::new(Mutex::new(PeerRegistry::new())),
        })
    }

    /// For unit tests: registry handle without starting mDNS.
    #[cfg(test)]
    pub fn registry(&self) -> Arc<Mutex<PeerRegistry>> {
        Arc::clone(&self.registry)
    }

    /// Drain mDNS events and return the current visible peer list.
    pub fn poll_peers(&self, own_device_id: &str) -> Result<Vec<PeerSnapshot>> {
        while let Ok(event) = self.browse_rx.recv_timeout(Duration::from_millis(50)) {
            self.handle_event(event);
        }

        let now = Instant::now();
        let mut registry = self.registry.lock().expect("peer registry lock");
        registry.evict_expired(now);
        Ok(registry.visible_peers(now, own_device_id))
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
                    registry.upsert(
                        PeerUpsert {
                            device_id,
                            name,
                            addr,
                            port,
                            mdns_fullname: fullname,
                        },
                        now,
                    );
                }
            }
            ServiceEvent::ServiceRemoved(_ty, fullname) => {
                registry.remove_by_fullname(&fullname);
            }
            _ => {}
        }
    }

    pub fn stop(self) -> Result<()> {
        self.daemon.unregister(&self.service_name).ok();
        Ok(())
    }
}

pub fn pick_listen_port() -> Result<(TcpListener, u16)> {
    let listener = TcpListener::bind(("0.0.0.0", 0)).context("bind tcp")?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

fn pick_addr(info: &ServiceInfo) -> Option<(IpAddr, u16)> {
    let port = info.get_port();
    info.get_addresses()
        .iter()
        .find(|ip| ip.is_ipv4())
        .map(|ip| (*ip, port))
}

fn local_ipv4_addrs() -> Vec<IpAddr> {
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
                },
                now,
            );
            assert_eq!(reg.remove_by_fullname("laptop.local"), Some("dev-1".into()));
        }
        let reg = registry.lock().unwrap();
        assert!(reg.visible_peers(now, "self").is_empty());
    }
}
