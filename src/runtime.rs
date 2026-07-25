use crate::config::{Identity, TrustStore};
use crate::discovery::{DiscoveredPeer, Discovery, pick_listen_port};
use crate::transfer::{begin_incoming, send_files, IncomingTransfer};
use anyhow::Result;
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub device_id: String,
    pub name: String,
    pub addr: SocketAddr,
}

pub enum RuntimeEvent {
    PeersUpdated(Vec<Peer>),
    IncomingConnection {
        remote_name: String,
        pairing_code: String,
        needs_pairing: bool,
        transfer: IncomingTransfer,
    },
    SendFinished {
        ok: bool,
        message: String,
    },
    ReceiveFinished {
        ok: bool,
        message: String,
    },
    TransferProgress {
        fraction: f32,
        label: String,
    },
    Log(String),
}

pub enum RuntimeCommand {
    Send {
        peer: Peer,
        paths: Vec<PathBuf>,
    },
    AcceptIncoming(IncomingTransfer),
    DeclineIncoming(IncomingTransfer),
}

pub struct RuntimeHandle {
    pub events: Receiver<RuntimeEvent>,
    cmd_tx: Sender<RuntimeCommand>,
}

impl RuntimeHandle {
    pub fn send(&self, peer: Peer, paths: Vec<PathBuf>) {
        let _ = self.cmd_tx.send(RuntimeCommand::Send { peer, paths });
    }

    pub fn accept(&self, transfer: IncomingTransfer) {
        let _ = self
            .cmd_tx
            .send(RuntimeCommand::AcceptIncoming(transfer));
    }

    pub fn decline(&self, transfer: IncomingTransfer) {
        let _ = self
            .cmd_tx
            .send(RuntimeCommand::DeclineIncoming(transfer));
    }
}

pub fn spawn(identity: Identity, trust: TrustStore) -> Result<RuntimeHandle> {
    let (event_tx, event_rx) = mpsc::channel();
    let (cmd_tx, cmd_rx) = mpsc::channel();

    let identity = Arc::new(identity);
    let trust = Arc::new(Mutex::new(trust));

    let (listener, port) = pick_listen_port()?;
    listener.set_nonblocking(true)?;

    let id_bg = Arc::clone(&identity);
    let trust_bg = Arc::clone(&trust);
    let event_peers = event_tx.clone();
    thread::spawn(move || peer_loop(id_bg, trust_bg, port, event_peers));

    let id_listener = Arc::clone(&identity);
    let trust_listener = Arc::clone(&trust);
    let event_listener = event_tx.clone();
    thread::spawn(move || listen_loop(listener, id_listener, trust_listener, event_listener));

    let id_cmd = Arc::clone(&identity);
    let trust_cmd = Arc::clone(&trust);
    thread::spawn(move || command_loop(id_cmd, trust_cmd, cmd_rx, event_tx));

    Ok(RuntimeHandle {
        events: event_rx,
        cmd_tx,
    })
}

fn peer_loop(
    identity: Arc<Identity>,
    _trust: Arc<Mutex<TrustStore>>,
    port: u16,
    event_tx: Sender<RuntimeEvent>,
) {
    let discovery = match Discovery::start(&identity.device_id, &identity.name, port) {
        Ok(d) => d,
        Err(err) => {
            let _ = event_tx.send(RuntimeEvent::Log(format!("discovery error: {err}")));
            return;
        }
    };
    let mut last: Vec<Peer> = Vec::new();
    loop {
        thread::sleep(Duration::from_secs(2));
        match discovery.poll_peers(&identity.device_id) {
            Ok(peers) => {
                let mapped = peers
                    .into_iter()
                    .map(discovered_to_peer)
                    .collect::<Vec<_>>();
                if mapped.len() != last.len() || mapped.iter().any(|p| !last.contains(p)) {
                    last = mapped.clone();
                    let _ = event_tx.send(RuntimeEvent::PeersUpdated(mapped));
                }
            }
            Err(err) => {
                let _ = event_tx.send(RuntimeEvent::Log(format!("browse error: {err}")));
            }
        }
    }
}

fn listen_loop(
    listener: TcpListener,
    identity: Arc<Identity>,
    trust: Arc<Mutex<TrustStore>>,
    event_tx: Sender<RuntimeEvent>,
) {
    loop {
        match listener.accept() {
            Ok((stream, _addr)) => {
                let _ = stream.set_nonblocking(false);
                let id = Arc::clone(&identity);
                let trust = Arc::clone(&trust);
                let event_tx = event_tx.clone();
                thread::spawn(move || {
                    let trust = trust.lock().expect("trust lock");
                    match begin_incoming(&id, &trust, stream) {
                        Ok(incoming) => {
                            let _ = event_tx.send(RuntimeEvent::IncomingConnection {
                                remote_name: incoming.remote.name.clone(),
                                pairing_code: incoming.pairing_code.clone(),
                                needs_pairing: incoming.needs_pairing,
                                transfer: incoming,
                            });
                        }
                        Err(err) => {
                            let _ = event_tx.send(RuntimeEvent::Log(format!(
                                "Incoming connection failed: {err}"
                            )));
                        }
                    }
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(200));
            }
            Err(err) => {
                let _ = event_tx.send(RuntimeEvent::Log(format!("accept error: {err}")));
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

fn command_loop(
    identity: Arc<Identity>,
    trust: Arc<Mutex<TrustStore>>,
    cmd_rx: Receiver<RuntimeCommand>,
    event_tx: Sender<RuntimeEvent>,
) {
    for cmd in cmd_rx {
        match cmd {
            RuntimeCommand::Send { peer, paths } => {
                let trust = trust.lock().expect("trust lock");
                let event_tx = event_tx.clone();
                let result = send_files(&identity, &trust, peer.addr, &paths, |fraction, label| {
                    let _ = event_tx.send(RuntimeEvent::TransferProgress {
                        fraction,
                        label: label.to_string(),
                    });
                });
                let msg = match &result {
                    Ok(r) => format!("Sent {} file(s) to {}", r.files_sent, peer.name),
                    Err(e) => format!("Send failed: {e}"),
                };
                let _ = event_tx.send(RuntimeEvent::SendFinished {
                    ok: result.is_ok(),
                    message: msg,
                });
            }
            RuntimeCommand::AcceptIncoming(transfer) => {
                let mut trust = trust.lock().expect("trust lock");
                let event_tx = event_tx.clone();
                let result = transfer.accept_and_save(&mut trust, |fraction, label| {
                    let _ = event_tx.send(RuntimeEvent::TransferProgress {
                        fraction,
                        label: label.to_string(),
                    });
                });
                let msg = match &result {
                    Ok(r) => format!(
                        "Received {} file(s): {}",
                        r.saved_paths.len(),
                        r.files
                            .iter()
                            .map(|f| f.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    Err(e) => format!("Receive failed: {e}"),
                };
                let _ = event_tx.send(RuntimeEvent::ReceiveFinished {
                    ok: result.is_ok(),
                    message: msg,
                });
            }
            RuntimeCommand::DeclineIncoming(transfer) => {
                let _ = transfer.decline();
                let _ = event_tx.send(RuntimeEvent::ReceiveFinished {
                    ok: true,
                    message: "Transfer declined".to_string(),
                });
            }
        }
    }
}

fn discovered_to_peer(p: DiscoveredPeer) -> Peer {
    Peer {
        device_id: p.device_id,
        name: p.name,
        addr: SocketAddr::new(p.addr, p.port),
    }
}
