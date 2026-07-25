use crate::protocol::SERVICE_TYPE;
use anyhow::{Context, Result};
use if_addrs::IfAddr;
use mdns_sd::{Receiver, ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    peers: Arc<Mutex<HashMap<String, DiscoveredPeer>>>,
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
            peers: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Drain mDNS events and return the current peer list.
    pub fn poll_peers(&self, own_device_id: &str) -> Result<Vec<DiscoveredPeer>> {
        while let Ok(event) = self.browse_rx.recv_timeout(Duration::from_millis(50)) {
            self.handle_event(event);
        }

        let cache = self.peers.lock().expect("peer cache lock");
        Ok(cache
            .values()
            .filter(|p| p.device_id != own_device_id)
            .cloned()
            .collect())
    }

    fn handle_event(&self, event: ServiceEvent) {
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
                    let peer = DiscoveredPeer {
                        device_id,
                        name,
                        addr,
                        port,
                    };
                    self.peers
                        .lock()
                        .expect("peer cache lock")
                        .insert(fullname, peer);
                }
            }
            ServiceEvent::ServiceRemoved(_ty, fullname) => {
                self.peers.lock().expect("peer cache lock").remove(&fullname);
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
