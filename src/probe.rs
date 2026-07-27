use crate::protocol::{ControlMessage, WireMessage};
use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

pub const PROBE_TIMEOUT: Duration = Duration::from_millis(500);
pub const PROBE_INTERVAL: Duration = Duration::from_secs(15);
/// Consecutive probe failures before a peer is treated as unreachable.
pub const PROBE_FAILURES_OFFLINE: u8 = 3;
/// Recent successful probe keeps a peer marked online.
pub const PROBE_FRESH: Duration = Duration::from_secs(20);

/// Send a plaintext Ping and expect Pong within [`PROBE_TIMEOUT`].
pub fn probe_peer(addr: SocketAddr, our_device_id: &str) -> bool {
    probe_peer_with_timeout(addr, our_device_id, PROBE_TIMEOUT)
}

pub fn probe_peer_with_timeout(addr: SocketAddr, our_device_id: &str, timeout: Duration) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, timeout) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let ping = WireMessage::Control(ControlMessage::Ping {
        device_id: our_device_id.to_string(),
    });
    if write_plain(&mut stream, &ping).is_err() {
        return false;
    }

    match read_plain(&mut stream) {
        Ok(WireMessage::Control(ControlMessage::Pong)) => true,
        _ => false,
    }
}

/// Respond to an inbound probe connection (caller already read Ping).
pub fn respond_to_probe(stream: &mut TcpStream) -> Result<()> {
    write_plain(
        stream,
        &WireMessage::Control(ControlMessage::Pong),
    )
    .context("send pong")
}

fn write_plain(stream: &mut TcpStream, msg: &WireMessage) -> Result<()> {
    let frame = msg.encode().context("encode probe message")?;
    stream
        .write_all(&frame)
        .context("write probe message")?;
    Ok(())
}

fn read_plain(stream: &mut TcpStream) -> Result<WireMessage> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).context("read probe frame length")?;
    let len = u32::from_be_bytes(len_buf) as usize;
    anyhow::ensure!(len <= crate::protocol::MAX_PLAINTEXT_WIRE_FRAME, "probe frame too large");
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).context("read probe frame")?;
    WireMessage::decode_frame(&len_buf, &payload).context("decode probe message")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ControlMessage;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn probe_roundtrip_localhost() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let msg = read_plain(&mut stream).unwrap();
            match msg {
                WireMessage::Control(ControlMessage::Ping { device_id }) => {
                    assert_eq!(device_id, "probe-test");
                    respond_to_probe(&mut stream).unwrap();
                }
                other => panic!("expected ping, got {other:?}"),
            }
        });

        assert!(probe_peer_with_timeout(
            addr,
            "probe-test",
            Duration::from_secs(2)
        ));
        handle.join().unwrap();
    }

    #[test]
    fn probe_fails_on_dead_port() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        assert!(!probe_peer_with_timeout(
            addr,
            "probe-test",
            Duration::from_millis(200)
        ));
    }
}
