use crate::config::{Identity, TrustStore};
use crate::discovery::{Discovery, pick_listen_port};
use crate::peer_registry::{PeerPresence, PeerSnapshot};
use crate::transfer::{
    connect_and_handshake, dispatch_incoming, send_files_on_connection, send_pairing_message,
    HandshakeResult, IncomingDispatch, IncomingTransfer,
};
use anyhow::{Context, Result};
use std::net::{SocketAddr, TcpListener, TcpStream};
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
    pub presence: PeerPresence,
    pub is_new: bool,
    pub reachable: bool,
}

pub enum RuntimeEvent {
    PeersUpdated(Vec<Peer>),
    IncomingConnection {
        remote_name: String,
        pairing_code: String,
        needs_pairing: bool,
        transfer: IncomingTransfer,
    },
    SendPairingConfirm {
        peer_name: String,
        pairing_code: String,
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
    ConfirmSendPairing,
    CancelSendPairing,
    AcceptIncoming(IncomingTransfer),
    DeclineIncoming(IncomingTransfer),
}

struct PendingSend {
    peer: Peer,
    paths: Vec<PathBuf>,
    stream: TcpStream,
    handshake: HandshakeResult,
}

pub struct RuntimeHandle {
    pub events: Receiver<RuntimeEvent>,
    cmd_tx: Sender<RuntimeCommand>,
}

impl RuntimeHandle {
    pub fn send(&self, peer: Peer, paths: Vec<PathBuf>) {
        let _ = self.cmd_tx.send(RuntimeCommand::Send { peer, paths });
    }

    pub fn confirm_send_pairing(&self) {
        let _ = self.cmd_tx.send(RuntimeCommand::ConfirmSendPairing);
    }

    pub fn cancel_send_pairing(&self) {
        let _ = self.cmd_tx.send(RuntimeCommand::CancelSendPairing);
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
    let pending_send = Arc::new(Mutex::new(None::<PendingSend>));

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
    let pending_cmd = Arc::clone(&pending_send);
    thread::spawn(move || command_loop(id_cmd, trust_cmd, pending_cmd, cmd_rx, event_tx));

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
    let mut discovery = match Discovery::start(&identity.device_id, &identity.name, port) {
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
                    .map(snapshot_to_peer)
                    .collect::<Vec<_>>();
                if peers_changed(&last, &mapped) {
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

fn peers_changed(previous: &[Peer], current: &[Peer]) -> bool {
    previous.len() != current.len()
        || previous
            .iter()
            .zip(current.iter())
            .any(|(a, b)| a != b)
        || previous
            .iter()
            .any(|p| !current.iter().any(|c| c.device_id == p.device_id))
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
                    match dispatch_incoming(&id, &trust, stream) {
                        Ok(IncomingDispatch::Probe) => {}
                        Ok(IncomingDispatch::Transfer(incoming)) => {
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
    pending_send: Arc<Mutex<Option<PendingSend>>>,
    cmd_rx: Receiver<RuntimeCommand>,
    event_tx: Sender<RuntimeEvent>,
) {
    for cmd in cmd_rx {
        match cmd {
            RuntimeCommand::Send { peer, paths } => {
                let event_tx = event_tx.clone();
                let pending_send = Arc::clone(&pending_send);
                let identity = Arc::clone(&identity);
                let mut trust = trust.lock().expect("trust lock");
                let progress = |fraction: f32, label: &str| {
                    let _ = event_tx.send(RuntimeEvent::TransferProgress {
                        fraction,
                        label: label.to_string(),
                    });
                };
                progress(0.02, "Connecting…");
                match connect_and_handshake(&identity, &trust, peer.addr, &peer.device_id) {
                    Ok((mut stream, handshake)) => {
                        progress(0.05, "Handshaking…");
                        if handshake.needs_pairing_confirm {
                            let pairing_code = handshake.pairing_code.clone();
                            *pending_send.lock().expect("pending send lock") = Some(PendingSend {
                                peer: peer.clone(),
                                paths,
                                stream,
                                handshake,
                            });
                            let _ = event_tx.send(RuntimeEvent::SendPairingConfirm {
                                peer_name: peer.name,
                                pairing_code,
                            });
                        } else {
                            let remote_id = handshake.remote.device_id.clone();
                            let remote_pk = handshake.remote.public_key.clone();
                            let result = finish_send(
                                &mut stream,
                                &handshake,
                                &paths,
                                &peer.name,
                                progress,
                            );
                            if result.is_ok() {
                                trust.trust_peer(&remote_id, &remote_pk);
                                let _ = trust.save();
                            }
                            emit_send_finished(&event_tx, &peer.name, result);
                        }
                    }
                    Err(err) => {
                        let _ = event_tx.send(RuntimeEvent::SendFinished {
                            ok: false,
                            message: format!("Send failed: {err}"),
                        });
                    }
                }
            }
            RuntimeCommand::ConfirmSendPairing => {
                let event_tx = event_tx.clone();
                let mut pending = pending_send.lock().expect("pending send lock");
                let Some(mut pending_send) = pending.take() else {
                    continue;
                };
                let peer_name = pending_send.peer.name.clone();
                let remote_id = pending_send.handshake.remote.device_id.clone();
                let remote_pk = pending_send.handshake.remote.public_key.clone();
                let progress = |fraction: f32, label: &str| {
                    let _ = event_tx.send(RuntimeEvent::TransferProgress {
                        fraction,
                        label: label.to_string(),
                    });
                };
                let result = finish_send(
                    &mut pending_send.stream,
                    &pending_send.handshake,
                    &pending_send.paths,
                    &peer_name,
                    progress,
                );
                if result.is_ok() {
                    let mut trust = trust.lock().expect("trust lock");
                    trust.trust_peer(&remote_id, &remote_pk);
                    let _ = trust.save();
                }
                emit_send_finished(&event_tx, &peer_name, result);
            }
            RuntimeCommand::CancelSendPairing => {
                *pending_send.lock().expect("pending send lock") = None;
                let _ = event_tx.send(RuntimeEvent::SendFinished {
                    ok: false,
                    message: "Send cancelled — pairing not confirmed".to_string(),
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

fn finish_send<F>(
    stream: &mut TcpStream,
    handshake: &HandshakeResult,
    paths: &[PathBuf],
    peer_name: &str,
    mut on_progress: F,
) -> Result<crate::transfer::SendResult>
where
    F: FnMut(f32, &str),
{
    if handshake.needs_pairing_confirm {
        send_pairing_message(stream, &handshake.cipher, &handshake.pairing_code)?;
    }
    send_files_on_connection(
        stream,
        &handshake.remote,
        &handshake.cipher,
        paths,
        |fraction, label| on_progress(fraction, label),
    )
    .with_context(|| format!("send to {peer_name}"))
}

fn emit_send_finished(
    event_tx: &Sender<RuntimeEvent>,
    peer_name: &str,
    result: Result<crate::transfer::SendResult>,
) {
    let msg = match &result {
        Ok(r) => format!("Sent {} file(s) to {}", r.files_sent, peer_name),
        Err(e) => format!("Send failed: {e}"),
    };
    let _ = event_tx.send(RuntimeEvent::SendFinished {
        ok: result.is_ok(),
        message: msg,
    });
}

fn snapshot_to_peer(snapshot: PeerSnapshot) -> Peer {
    Peer {
        device_id: snapshot.device_id,
        name: snapshot.name,
        addr: snapshot.addr,
        presence: snapshot.presence,
        is_new: snapshot.is_new,
        reachable: snapshot.reachable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer_registry::PeerPresence;
    use std::net::SocketAddr;

    fn sample_peer(id: &str, presence: PeerPresence, is_new: bool) -> Peer {
        Peer {
            device_id: id.to_string(),
            name: id.to_string(),
            addr: SocketAddr::from(([192, 168, 1, 1], 9000)),
            presence,
            is_new,
            reachable: presence == PeerPresence::Online,
        }
    }

    #[test]
    fn peers_changed_detects_presence_update() {
        let before = vec![sample_peer("a", PeerPresence::Online, false)];
        let after = vec![sample_peer("a", PeerPresence::Stale, false)];
        assert!(peers_changed(&before, &after));
    }

    #[test]
    fn peers_changed_detects_new_peer() {
        let before = vec![sample_peer("a", PeerPresence::Online, false)];
        let after = vec![
            sample_peer("a", PeerPresence::Online, false),
            sample_peer("b", PeerPresence::Online, true),
        ];
        assert!(peers_changed(&before, &after));
    }

    #[test]
    fn peers_changed_ignores_identical_lists() {
        let a = vec![sample_peer("a", PeerPresence::Online, false)];
        let b = vec![sample_peer("a", PeerPresence::Online, false)];
        assert!(!peers_changed(&a, &b));
    }
}
