use crate::config::{download_dir, Identity, TrustStore};
use crate::crypto::{decrypt, encrypt, pairing_code, session_cipher};
use crate::protocol::{ControlMessage, FileMeta, Hello, Offer, WireMessage, CHUNK_SIZE};
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

pub struct IncomingTransfer {
    stream: TcpStream,
    cipher: ChaCha20Poly1305,
    pub remote: Hello,
    pub needs_pairing: bool,
    pub pairing_code: String,
}

pub fn send_files<F>(
    identity: &Identity,
    trust: &TrustStore,
    addr: SocketAddr,
    paths: &[PathBuf],
    mut on_progress: F,
) -> Result<SendResult>
where
    F: FnMut(f32, &str),
{
    on_progress(0.02, "Connecting…");
    let mut stream = TcpStream::connect(addr).with_context(|| format!("connect to {addr}"))?;
    configure_stream(&stream)?;

    on_progress(0.05, "Handshaking…");
    let (remote, cipher) = handshake_client(identity, trust, &mut stream)?;

    on_progress(0.08, "Waiting for receiver to accept…");
    // Wait for receiver to approve the connection before sending the offer.
    match read_encrypted(&mut stream, &cipher, "waiting for receiver to accept connection")? {
        WireMessage::Control(ControlMessage::Accept) => {}
        WireMessage::Control(ControlMessage::Decline) => {
            return Err(anyhow!(
                "{} declined the connection",
                remote.name
            ));
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
        &mut stream,
        &cipher,
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
        send_one_file(&mut stream, &cipher, path, file, &mut sent_bytes, total_bytes, &label, &mut on_progress)?;
    }

    write_encrypted(
        &mut stream,
        &cipher,
        &WireMessage::Control(ControlMessage::Done),
        "finish transfer",
    )?;

    on_progress(1.0, "Send complete");
    Ok(SendResult {
        files_sent: files.len(),
    })
}

pub fn begin_incoming(
    identity: &Identity,
    trust: &TrustStore,
    stream: TcpStream,
) -> Result<IncomingTransfer> {
    configure_stream(&stream)?;
    let (remote, cipher, needs_pairing) = receive_handshake(identity, trust, stream.try_clone()?)?;
    let remote_pk = decode_pubkey(&remote.public_key)?;
    let code = pairing_code(&identity.public_key(), &remote_pk);
    Ok(IncomingTransfer {
        stream,
        cipher,
        remote,
        needs_pairing,
        pairing_code: code,
    })
}

impl IncomingTransfer {
    pub fn accept_and_save<F>(mut self, trust: &mut TrustStore, mut on_progress: F) -> Result<ReceiveResult>
    where
        F: FnMut(f32, &str),
    {
        on_progress(0.05, "Accepting connection…");
        trust.trust(&self.remote.device_id);
        trust.save()?;

        write_encrypted(
            &mut self.stream,
            &self.cipher,
            &WireMessage::Control(ControlMessage::Accept),
            "accept connection",
        )?;

        on_progress(0.10, "Reading file list…");
        let offer_msg = read_encrypted(&mut self.stream, &self.cipher, "read file offer")?;
        let offer = match offer_msg {
            WireMessage::Control(ControlMessage::Offer(offer)) => offer,
            WireMessage::Control(ControlMessage::Decline) => {
                return Err(anyhow!("sender cancelled before sending files"));
            }
            other => return Err(anyhow!("expected file offer, got {other:?}")),
        };

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
                WireMessage::Chunk(data) => {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&data) {
                        if v.get("kind").and_then(|k| k.as_str()) == Some("file_start") {
                            let name = v
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("file");
                            current_name = name.to_string();
                            let path = dest_root.join(sanitize_filename(name));
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
                    } else if let Some(ref mut file) = current_file {
                        file.write_all(&data)?;
                        received_bytes += data.len() as u64;
                        on_progress(
                            0.10 + 0.90 * (received_bytes as f32 / total_bytes as f32),
                            &format!("Receiving {current_name}…"),
                        );
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

fn receive_handshake(
    identity: &Identity,
    trust: &TrustStore,
    mut stream: TcpStream,
) -> Result<(Hello, ChaCha20Poly1305, bool)> {
    let remote_msg = read_plain(&mut stream, "read hello from peer")?;
    let remote = match remote_msg {
        WireMessage::Control(ControlMessage::Hello(h)) => h,
        other => return Err(anyhow!("expected hello, got {other:?}")),
    };

    let hello = Hello {
        device_id: identity.device_id.clone(),
        name: identity.name.clone(),
        public_key: B64.encode(identity.public_key().as_bytes()),
    };
    write_plain(
        &mut stream,
        &WireMessage::Control(ControlMessage::Hello(hello)),
        "send hello",
    )?;

    let remote_pk = decode_pubkey(&remote.public_key)?;
    let cipher = session_cipher(&identity.secret(), &remote_pk);
    let needs_pairing = !trust.is_trusted(&remote.device_id);
    Ok((remote, cipher, needs_pairing))
}

fn handshake_client(
    identity: &Identity,
    trust: &TrustStore,
    stream: &mut TcpStream,
) -> Result<(Hello, ChaCha20Poly1305)> {
    let hello = Hello {
        device_id: identity.device_id.clone(),
        name: identity.name.clone(),
        public_key: B64.encode(identity.public_key().as_bytes()),
    };
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

    let remote_pk = decode_pubkey(&remote.public_key)?;
    let cipher = session_cipher(&identity.secret(), &remote_pk);

    if !trust.is_trusted(&remote.device_id) {
        let code = pairing_code(&identity.public_key(), &remote_pk);
        write_encrypted(
            stream,
            &cipher,
            &WireMessage::Control(ControlMessage::Pairing(crate::protocol::PairingRequest {
                code,
            })),
            "send pairing code",
        )?;
    }

    Ok((remote, cipher))
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
    let header = serde_json::json!({
        "kind": "file_start",
        "name": meta.name,
        "size": meta.size,
    });
    write_encrypted(
        stream,
        cipher,
        &WireMessage::Chunk(header.to_string().into_bytes()),
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
        .with_context(|| format!("{context}: write frame length"))?;
    stream
        .write_all(&encrypted)
        .with_context(|| format!("{context}: write encrypted frame"))?;
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
        _ if err.raw_os_error() == Some(35) => {
            anyhow!(
                "{context}: connection not ready — peer may have declined or disconnected"
            )
        }
        _ => anyhow!("{context}: {err}"),
    }
}
