use crate::config::{download_dir, Identity, PeerTrust, TrustStore};
use crate::crypto::{
    build_hello, decrypt, encrypt, pairing_code, session_cipher, verify_hello,
};
use crate::protocol::{
    ControlMessage, FileMeta, Hello, Offer, WireMessage, CHUNK_SIZE, MAX_ENCRYPTED_WIRE_FRAME,
    MAX_PLAINTEXT_WIRE_FRAME,
};
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chacha20poly1305::ChaCha20Poly1305;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;
use x25519_dalek::PublicKey;

pub struct SendResult {
    pub files_sent: usize,
}

pub struct ReceiveResult {
    pub saved_paths: Vec<PathBuf>,
    pub files: Vec<FileMeta>,
}

pub struct HandshakeResult {
    pub remote: Hello,
    pub cipher: ChaCha20Poly1305,
    pub pairing_code: String,
    pub needs_pairing_confirm: bool,
}

pub enum IncomingDispatch {
    Probe,
    Transfer(IncomingTransfer),
}

pub struct IncomingTransfer {
    stream: TcpStream,
    cipher: ChaCha20Poly1305,
    pub remote: Hello,
    pub needs_pairing: bool,
    pub pairing_code: String,
    expect_pairing: bool,
}

pub fn connect_and_handshake(
    identity: &Identity,
    trust: &TrustStore,
    addr: SocketAddr,
    peer_device_id: &str,
) -> Result<(TcpStream, HandshakeResult)> {
    let mut stream = TcpStream::connect(addr).with_context(|| format!("connect to {addr}"))?;
    configure_stream(&stream)?;
    let result = handshake_client(identity, trust, &mut stream, peer_device_id)?;
    Ok((stream, result))
}

pub fn send_pairing_message(
    stream: &mut TcpStream,
    cipher: &ChaCha20Poly1305,
    code: &str,
) -> Result<()> {
    write_encrypted(
        stream,
        cipher,
        &WireMessage::Control(ControlMessage::Pairing(crate::protocol::PairingRequest {
            code: code.to_string(),
        })),
        "send pairing code",
    )
}

pub fn send_files_on_connection<F>(
    stream: &mut TcpStream,
    remote: &Hello,
    cipher: &ChaCha20Poly1305,
    paths: &[PathBuf],
    mut on_progress: F,
) -> Result<SendResult>
where
    F: FnMut(f32, &str),
{
    on_progress(0.08, "Waiting for receiver to accept…");
    match read_encrypted(stream, cipher, "waiting for receiver to accept connection")? {
        WireMessage::Control(ControlMessage::Accept) => {}
        WireMessage::Control(ControlMessage::Decline) => {
            return Err(anyhow!("{} declined the connection", remote.name));
        }
        other => {
            return Err(anyhow!(
                "unexpected response while waiting for acceptance: {other:?}"
            ));
        }
    }

    let files = collect_files(paths)?;
    anyhow::ensure!(!files.is_empty(), "no files selected to send");

    on_progress(0.10, "Sending file list…");
    write_encrypted(
        stream,
        cipher,
        &WireMessage::Control(ControlMessage::Offer(Offer {
            files: files.clone(),
        })),
        "send file offer",
    )?;

    let total_bytes: u64 = files.iter().map(|f| f.size).sum::<u64>().max(1);
    let mut sent_bytes: u64 = 0;

    for (idx, file) in files.iter().enumerate() {
        let path = paths
            .iter()
            .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(file.name.as_str()))
            .ok_or_else(|| anyhow!("missing path for {}", file.name))?;
        let label = format!("Sending {} ({}/{})", file.name, idx + 1, files.len());
        send_one_file(
            stream,
            cipher,
            path,
            file,
            &mut sent_bytes,
            total_bytes,
            &label,
            &mut on_progress,
        )?;
    }

    write_encrypted(
        stream,
        cipher,
        &WireMessage::Control(ControlMessage::Done),
        "finish transfer",
    )?;

    on_progress(1.0, "Send complete");
    Ok(SendResult {
        files_sent: files.len(),
    })
}

pub fn send_files<F>(
    identity: &Identity,
    trust: &TrustStore,
    addr: SocketAddr,
    peer_device_id: &str,
    paths: &[PathBuf],
    mut on_progress: F,
) -> Result<SendResult>
where
    F: FnMut(f32, &str),
{
    on_progress(0.02, "Connecting…");
    let (mut stream, handshake) =
        connect_and_handshake(identity, trust, addr, peer_device_id)?;
    on_progress(0.05, "Handshaking…");

    if handshake.needs_pairing_confirm {
        send_pairing_message(&mut stream, &handshake.cipher, &handshake.pairing_code)?;
    }

    send_files_on_connection(
        &mut stream,
        &handshake.remote,
        &handshake.cipher,
        paths,
        &mut on_progress,
    )
}

pub fn begin_incoming(
    identity: &Identity,
    trust: &TrustStore,
    stream: TcpStream,
) -> Result<IncomingTransfer> {
    configure_stream(&stream)?;
    let (remote, cipher, needs_pairing, expect_pairing) =
        receive_handshake(identity, trust, stream.try_clone()?)?;
    finish_incoming(identity, trust, stream, remote, cipher, needs_pairing, expect_pairing)
}

/// Classify an inbound TCP connection as probe or file transfer.
pub fn dispatch_incoming(
    identity: &Identity,
    trust: &TrustStore,
    mut stream: TcpStream,
) -> Result<IncomingDispatch> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .context("set probe classify read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .context("set probe classify write timeout")?;

    let first = read_plain(&mut stream, "classify inbound connection")?;
    match first {
        WireMessage::Control(ControlMessage::Ping { .. }) => {
            crate::probe::respond_to_probe(&mut stream)?;
            Ok(IncomingDispatch::Probe)
        }
        WireMessage::Control(ControlMessage::Hello(remote)) => {
            validate_remote_hello(trust, &remote)?;
            let pairing_required = requires_pairing(trust, &remote);
            let hello = build_hello(identity, pairing_required);
            write_plain(
                &mut stream,
                &WireMessage::Control(ControlMessage::Hello(hello)),
                "send hello",
            )?;
            let remote_pk = decode_pubkey(&remote.public_key)?;
            let cipher = session_cipher(&identity.secret(), &remote_pk);
            let needs_pairing = pairing_required;
            let expect_pairing = requires_pairing(trust, &remote);
            configure_stream(&stream)?;
            let transfer = finish_incoming(
                identity,
                trust,
                stream,
                remote,
                cipher,
                needs_pairing,
                expect_pairing,
            )?;
            Ok(IncomingDispatch::Transfer(transfer))
        }
        other => Err(anyhow!("unexpected first message on inbound connection: {other:?}")),
    }
}

fn finish_incoming(
    identity: &Identity,
    trust: &TrustStore,
    stream: TcpStream,
    remote: Hello,
    cipher: ChaCha20Poly1305,
    needs_pairing: bool,
    expect_pairing: bool,
) -> Result<IncomingTransfer> {
    let _ = (identity, trust);
    let remote_pk = decode_pubkey(&remote.public_key)?;
    let code = pairing_code(&identity.public_key(), &remote_pk);
    Ok(IncomingTransfer {
        stream,
        cipher,
        remote,
        needs_pairing,
        pairing_code: code,
        expect_pairing,
    })
}

impl IncomingTransfer {
    pub fn accept_and_save<F>(
        mut self,
        trust: &mut TrustStore,
        mut on_progress: F,
    ) -> Result<ReceiveResult>
    where
        F: FnMut(f32, &str),
    {
        on_progress(0.05, "Accepting connection…");

        write_encrypted(
            &mut self.stream,
            &self.cipher,
            &WireMessage::Control(ControlMessage::Accept),
            "accept connection",
        )?;

        on_progress(0.10, "Reading file list…");
        let offer = read_offer_after_accept(
            &mut self.stream,
            &self.cipher,
            &self.pairing_code,
            self.expect_pairing,
        )?;

        trust.trust_peer(&self.remote.device_id, &self.remote.public_key);
        trust.save()?;

        let total_bytes: u64 = offer.files.iter().map(|f| f.size).sum::<u64>().max(1);
        let mut received_bytes: u64 = 0;

        let dest_root = download_dir()?;
        let mut saved = Vec::new();
        let mut current_file: Option<File> = None;
        let mut current_name = String::new();

        loop {
            let msg = read_encrypted(&mut self.stream, &self.cipher, "receive file data")?;
            match msg {
                WireMessage::Control(ControlMessage::Done) => break,
                WireMessage::Control(ControlMessage::FileComplete) => {
                    current_file = None;
                }
                WireMessage::Control(ControlMessage::FileStart { name, size: _ }) => {
                    current_name = name.clone();
                    let path = dest_root.join(sanitize_filename(&name));
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    current_file = Some(File::create(&path)?);
                    saved.push(path);
                    on_progress(
                        0.10 + 0.90 * (received_bytes as f32 / total_bytes as f32),
                        &format!("Receiving {current_name}…"),
                    );
                }
                WireMessage::Chunk(data) => {
                    if let Some(ref mut file) = current_file {
                        file.write_all(&data)?;
                        received_bytes += data.len() as u64;
                        on_progress(
                            0.10 + 0.90 * (received_bytes as f32 / total_bytes as f32),
                            &format!("Receiving {current_name}…"),
                        );
                    } else {
                        return Err(anyhow!("received file chunk before FileStart"));
                    }
                }
                other => return Err(anyhow!("unexpected message while receiving: {other:?}")),
            }
        }

        on_progress(1.0, "Receive complete");
        Ok(ReceiveResult {
            saved_paths: saved,
            files: offer.files,
        })
    }

    pub fn decline(mut self) -> Result<()> {
        write_encrypted(
            &mut self.stream,
            &self.cipher,
            &WireMessage::Control(ControlMessage::Decline),
            "decline connection",
        )
    }
}

fn requires_pairing(trust: &TrustStore, remote: &Hello) -> bool {
    !trust.is_pinned(&remote.device_id, &remote.public_key) || remote.pairing_required
}

fn validate_remote_hello(trust: &TrustStore, hello: &Hello) -> Result<()> {
    verify_hello(hello)?;
    if matches!(
        trust.check_peer(&hello.device_id, &hello.public_key),
        PeerTrust::KeyMismatch
    ) {
        return Err(anyhow!(
            "known device is using a different key — re-pair or reject"
        ));
    }
    Ok(())
}

fn read_offer_after_accept(
    stream: &mut TcpStream,
    cipher: &ChaCha20Poly1305,
    expected_pairing_code: &str,
    expect_pairing: bool,
) -> Result<Offer> {
    let mut pairing_verified = !expect_pairing;
    loop {
        let msg = read_encrypted(stream, cipher, "read file offer")?;
        match msg {
            WireMessage::Control(ControlMessage::Offer(offer)) => {
                anyhow::ensure!(
                    pairing_verified,
                    "sender did not complete pairing verification"
                );
                return Ok(offer);
            }
            WireMessage::Control(ControlMessage::Decline) => {
                return Err(anyhow!("sender cancelled before sending files"));
            }
            WireMessage::Control(ControlMessage::Pairing(req)) => {
                anyhow::ensure!(
                    req.code == expected_pairing_code,
                    "pairing verification failed"
                );
                pairing_verified = true;
            }
            other => return Err(anyhow!("expected file offer, got {other:?}")),
        }
    }
}

fn receive_handshake(
    identity: &Identity,
    trust: &TrustStore,
    mut stream: TcpStream,
) -> Result<(Hello, ChaCha20Poly1305, bool, bool)> {
    let remote_msg = read_plain(&mut stream, "read hello from peer")?;
    let remote = match remote_msg {
        WireMessage::Control(ControlMessage::Hello(h)) => h,
        other => return Err(anyhow!("expected hello, got {other:?}")),
    };
    validate_remote_hello(trust, &remote)?;

    let pairing_required = requires_pairing(trust, &remote);
    let hello = build_hello(identity, pairing_required);
    write_plain(
        &mut stream,
        &WireMessage::Control(ControlMessage::Hello(hello)),
        "send hello",
    )?;

    let remote_pk = decode_pubkey(&remote.public_key)?;
    let cipher = session_cipher(&identity.secret(), &remote_pk);
    let needs_pairing = pairing_required;
    let expect_pairing = requires_pairing(trust, &remote);
    Ok((remote, cipher, needs_pairing, expect_pairing))
}

fn handshake_client(
    identity: &Identity,
    trust: &TrustStore,
    stream: &mut TcpStream,
    peer_device_id: &str,
) -> Result<HandshakeResult> {
    let pairing_required = !trust.has_entry(peer_device_id);
    let hello = build_hello(identity, pairing_required);
    write_plain(
        stream,
        &WireMessage::Control(ControlMessage::Hello(hello)),
        "send hello",
    )?;

    let remote_msg = read_plain(stream, "read hello from peer")?;
    let remote = match remote_msg {
        WireMessage::Control(ControlMessage::Hello(h)) => h,
        other => return Err(anyhow!("expected hello, got {other:?}")),
    };
    validate_remote_hello(trust, &remote)?;

    let remote_pk = decode_pubkey(&remote.public_key)?;
    let cipher = session_cipher(&identity.secret(), &remote_pk);
    let code = pairing_code(&identity.public_key(), &remote_pk);
    let needs_pairing_confirm = requires_pairing(trust, &remote);

    Ok(HandshakeResult {
        remote,
        cipher,
        pairing_code: code,
        needs_pairing_confirm,
    })
}

fn configure_stream(stream: &TcpStream) -> Result<()> {
    stream
        .set_nonblocking(false)
        .context("set socket to blocking mode")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(300)))
        .context("set read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(300)))
        .context("set write timeout")?;
    Ok(())
}

fn collect_files(paths: &[PathBuf]) -> Result<Vec<FileMeta>> {
    let mut files = Vec::new();
    for path in paths {
        let meta = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
        if !meta.is_file() {
            continue;
        }
        files.push(FileMeta {
            name: path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string(),
            size: meta.len(),
        });
    }
    Ok(files)
}

fn send_one_file<F>(
    stream: &mut TcpStream,
    cipher: &ChaCha20Poly1305,
    path: &Path,
    meta: &FileMeta,
    sent_bytes: &mut u64,
    total_bytes: u64,
    label: &str,
    on_progress: &mut F,
) -> Result<()>
where
    F: FnMut(f32, &str),
{
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    write_encrypted(
        stream,
        cipher,
        &WireMessage::Control(ControlMessage::FileStart {
            name: meta.name.clone(),
            size: meta.size,
        }),
        &format!("start file {}", meta.name),
    )?;

    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        write_encrypted(
            stream,
            cipher,
            &WireMessage::Chunk(buf[..n].to_vec()),
            &format!("send chunk of {}", meta.name),
        )?;
        *sent_bytes += n as u64;
        on_progress(
            0.10 + 0.90 * (*sent_bytes as f32 / total_bytes as f32),
            label,
        );
    }

    write_encrypted(
        stream,
        cipher,
        &WireMessage::Control(ControlMessage::FileComplete),
        &format!("finish file {}", meta.name),
    )?;
    Ok(())
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn decode_pubkey(encoded: &str) -> Result<PublicKey> {
    let bytes = B64.decode(encoded).context("decode public key")?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow!("invalid public key length"))?;
    Ok(PublicKey::from(arr))
}

fn write_plain(stream: &mut TcpStream, msg: &WireMessage, context: &str) -> Result<()> {
    let frame = msg.encode().context("encode message")?;
    stream
        .write_all(&frame)
        .with_context(|| format!("{context}: write message"))?;
    Ok(())
}

fn read_plain(stream: &mut TcpStream, context: &str) -> Result<WireMessage> {
    let (header, payload) = read_frame(stream, context)?;
    WireMessage::decode_frame(&header, &payload).context("decode message")
}

fn write_encrypted(
    stream: &mut TcpStream,
    cipher: &ChaCha20Poly1305,
    msg: &WireMessage,
    context: &str,
) -> Result<()> {
    let plain = msg.encode().context("encode message")?;
    let encrypted = encrypt(cipher, &plain).context("encrypt message")?;
    let len = (encrypted.len() as u32).to_be_bytes();
    stream
        .write_all(&len)
        .map_err(|err| map_io_error(&format!("{context}: write frame length"), err))?;
    stream
        .write_all(&encrypted)
        .map_err(|err| map_io_error(&format!("{context}: write encrypted frame"), err))?;
    Ok(())
}

fn read_encrypted(
    stream: &mut TcpStream,
    cipher: &ChaCha20Poly1305,
    context: &str,
) -> Result<WireMessage> {
    let mut len_buf = [0u8; 4];
    read_exact(stream, &mut len_buf, context)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    anyhow::ensure!(
        len <= MAX_ENCRYPTED_WIRE_FRAME,
        "{context}: encrypted frame too large ({len} bytes)"
    );
    let mut encrypted = vec![0u8; len];
    read_exact(stream, &mut encrypted, context)?;
    let plain = decrypt(cipher, &encrypted).context("decrypt message")?;
    let header: [u8; 4] = plain[..4]
        .try_into()
        .map_err(|_| anyhow!("{context}: short decrypted frame"))?;
    WireMessage::decode_frame(&header, &plain[4..]).context("decode message")
}

fn read_frame(stream: &mut TcpStream, context: &str) -> Result<([u8; 4], Vec<u8>)> {
    let mut len_buf = [0u8; 4];
    read_exact(stream, &mut len_buf, context)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    anyhow::ensure!(
        len <= MAX_PLAINTEXT_WIRE_FRAME,
        "{context}: plaintext frame too large ({len} bytes)"
    );
    let mut payload = vec![0u8; len];
    read_exact(stream, &mut payload, context)?;
    Ok((len_buf, payload))
}

fn read_exact(stream: &mut TcpStream, buf: &mut [u8], context: &str) -> Result<()> {
    stream
        .read_exact(buf)
        .map_err(|err| map_io_error(context, err))
}

fn map_io_error(context: &str, err: std::io::Error) -> anyhow::Error {
    use std::io::ErrorKind;

    match err.kind() {
        ErrorKind::UnexpectedEof => {
            anyhow!("{context}: peer closed the connection before the transfer finished")
        }
        ErrorKind::TimedOut => anyhow!("{context}: timed out — is the other device still running?"),
        ErrorKind::WouldBlock | ErrorKind::Interrupted => {
            anyhow!("{context}: socket not ready ({err})")
        }
        _ if err.raw_os_error() == Some(35) => anyhow!(
            "{context}: connection not ready — peer may have declined or disconnected"
        ),
        _ => anyhow!("{context}: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{set_download_dir_for_tests, Identity, TrustStore};
    use crate::crypto::{build_hello, session_cipher};
    use rand::rngs::OsRng;
    use std::net::TcpListener;
    use std::thread;
    use tempfile::tempdir;
    use x25519_dalek::{PublicKey, StaticSecret};

    fn test_identity(label: &str) -> Identity {
        Identity {
            device_id: format!("test-{label}"),
            name: label.to_string(),
            secret_key: StaticSecret::random_from_rng(OsRng).to_bytes(),
        }
    }

    fn pubkey_b64(identity: &Identity) -> String {
        B64.encode(identity.public_key().as_bytes())
    }

    fn paired_ciphers() -> (ChaCha20Poly1305, ChaCha20Poly1305) {
        let secret_a = StaticSecret::random_from_rng(OsRng);
        let secret_b = StaticSecret::random_from_rng(OsRng);
        let cipher_a = session_cipher(&secret_a, &PublicKey::from(&secret_b));
        let cipher_b = session_cipher(&secret_b, &PublicKey::from(&secret_a));
        (cipher_a, cipher_b)
    }

    fn trusted_pair(sender: &Identity, receiver: &Identity) -> (TrustStore, TrustStore) {
        let mut sender_trust = TrustStore::default();
        let mut receiver_trust = TrustStore::default();
        sender_trust.trust_peer(&receiver.device_id, &pubkey_b64(receiver));
        receiver_trust.trust_peer(&sender.device_id, &pubkey_b64(sender));
        (sender_trust, receiver_trust)
    }

    #[test]
    fn encrypted_large_chunk_roundtrip_over_tcp() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let chunk: Vec<u8> = (0..CHUNK_SIZE).map(|i| (i % 256) as u8).collect();
        let expected = chunk.clone();
        let (cipher_a, cipher_b) = paired_ciphers();

        let receiver = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            configure_stream(&stream).unwrap();
            match read_encrypted(&mut stream, &cipher_b, "receive chunk").unwrap() {
                WireMessage::Chunk(restored) => assert_eq!(restored, expected),
                other => panic!("expected chunk, got {other:?}"),
            }
        });

        let mut stream = TcpStream::connect(addr).unwrap();
        configure_stream(&stream).unwrap();
        write_encrypted(
            &mut stream,
            &cipher_a,
            &WireMessage::Chunk(chunk),
            "send chunk",
        )
        .unwrap();
        receiver.join().unwrap();
    }

    #[test]
    fn sanitize_filename_strips_special_chars() {
        assert_eq!(
            sanitize_filename("El_Labirento_Del_Fauno_by_negativefix.jpg"),
            "El_Labirento_Del_Fauno_by_negativefix.jpg"
        );
        assert_eq!(sanitize_filename("bad/name?.jpg"), "bad_name_.jpg");
    }

    #[test]
    fn e2e_send_and_receive_file() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let sender = test_identity("sender");
        let receiver = test_identity("receiver");
        let (sender_trust, mut receiver_trust) = trusted_pair(&sender, &receiver);

        let send_dir = tempdir().unwrap();
        let file_path = send_dir.path().join("El_Labirento_Del_Fauno_by_negativefix.jpg");
        let payload: Vec<u8> = (0..150_000).map(|i| (i % 251) as u8).collect();
        std::fs::write(&file_path, &payload).unwrap();

        let recv_dir = tempdir().unwrap();
        set_download_dir_for_tests(recv_dir.path().to_path_buf());
        let receiver_device_id = receiver.device_id.clone();

        let receiver_handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let incoming = begin_incoming(&receiver, &receiver_trust, stream).unwrap();
            assert_eq!(incoming.remote.name, "sender");
            assert!(!incoming.needs_pairing);
            incoming
                .accept_and_save(&mut receiver_trust, |_p, _m| {})
                .unwrap()
        });

        send_files(
            &sender,
            &sender_trust,
            addr,
            &receiver_device_id,
            &[file_path.clone()],
            |_p, _m| {},
        )
        .unwrap();

        let result = receiver_handle.join().unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].name, "El_Labirento_Del_Fauno_by_negativefix.jpg");
        assert_eq!(result.saved_paths.len(), 1);

        let saved = std::fs::read(&result.saved_paths[0]).unwrap();
        assert_eq!(saved, payload);
    }

    #[test]
    fn e2e_decline_before_offer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let sender = test_identity("sender");
        let receiver = test_identity("receiver");
        let (sender_trust, receiver_trust) = trusted_pair(&sender, &receiver);
        let receiver_device_id = receiver.device_id.clone();

        let receiver_handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let incoming = begin_incoming(&receiver, &receiver_trust, stream).unwrap();
            incoming.decline().unwrap();
        });

        let result = send_files(
            &sender,
            &sender_trust,
            addr,
            &receiver_device_id,
            &[PathBuf::from("/dev/null")],
            |_p, _m| {},
        );
        receiver_handle.join().unwrap();
        match result {
            Err(err) => {
                let msg = err.to_string().to_lowercase();
                assert!(
                    msg.contains("declined") || msg.contains("closed") || msg.contains("broken pipe"),
                    "unexpected error: {err}"
                );
            }
            Ok(_) => panic!("expected decline error"),
        }
    }

    #[test]
    fn e2e_mutual_pairing_when_sender_trusts_receiver_only() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let sender = test_identity("sender");
        let receiver = test_identity("receiver");
        let mut sender_trust = TrustStore::default();
        sender_trust.trust_peer(&receiver.device_id, &pubkey_b64(&receiver));

        let send_dir = tempdir().unwrap();
        let file_path = send_dir.path().join("note.txt");
        std::fs::write(&file_path, b"pairing path").unwrap();

        let recv_dir = tempdir().unwrap();
        set_download_dir_for_tests(recv_dir.path().to_path_buf());
        let receiver_device_id = receiver.device_id.clone();

        let receiver_handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let incoming = begin_incoming(&receiver, &TrustStore::default(), stream).unwrap();
            assert!(incoming.needs_pairing);
            let mut receiver_trust = TrustStore::default();
            incoming
                .accept_and_save(&mut receiver_trust, |_p, _m| {})
                .unwrap()
        });

        send_files(
            &sender,
            &sender_trust,
            addr,
            &receiver_device_id,
            &[file_path],
            |_p, _m| {},
        )
        .unwrap();

        let result = receiver_handle.join().unwrap();
        assert_eq!(result.files.len(), 1);
        let saved = std::fs::read_to_string(&result.saved_paths[0]).unwrap();
        assert_eq!(saved, "pairing path");
    }

    #[test]
    fn e2e_rejects_pinned_peer_with_wrong_key() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let sender = test_identity("sender");
        let receiver = test_identity("receiver");
        let impostor = Identity {
            device_id: sender.device_id.clone(),
            name: "impostor".to_string(),
            secret_key: StaticSecret::random_from_rng(OsRng).to_bytes(),
        };

        let mut receiver_trust = TrustStore::default();
        receiver_trust.trust_peer(&sender.device_id, &pubkey_b64(&sender));

        let receiver_handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            begin_incoming(&receiver, &receiver_trust, stream)
        });

        let mut stream = TcpStream::connect(addr).unwrap();
        configure_stream(&stream).unwrap();
        let hello = build_hello(&impostor, false);
        write_plain(
            &mut stream,
            &WireMessage::Control(ControlMessage::Hello(hello)),
            "send hello",
        )
        .unwrap();

        let result = receiver_handle.join().unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn e2e_multi_chunk_file_integrity() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let sender = test_identity("sender-a");
        let receiver = test_identity("receiver-b");
        let (sender_trust, mut receiver_trust) = trusted_pair(&sender, &receiver);

        let send_dir = tempdir().unwrap();
        let file_path = send_dir.path().join("large.bin");
        let payload: Vec<u8> = (0..CHUNK_SIZE * 3 + 17).map(|i| (i % 256) as u8).collect();
        std::fs::write(&file_path, &payload).unwrap();

        let recv_dir = tempdir().unwrap();
        set_download_dir_for_tests(recv_dir.path().to_path_buf());
        let receiver_device_id = receiver.device_id.clone();

        let receiver_handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let incoming = begin_incoming(&receiver, &receiver_trust, stream).unwrap();
            incoming
                .accept_and_save(&mut receiver_trust, |_p, _m| {})
                .unwrap()
        });

        send_files(
            &sender,
            &sender_trust,
            addr,
            &receiver_device_id,
            &[file_path],
            |_p, _m| {},
        )
        .unwrap();

        let result = receiver_handle.join().unwrap();
        let saved = std::fs::read(&result.saved_paths[0]).unwrap();
        assert_eq!(saved, payload);
    }
}
