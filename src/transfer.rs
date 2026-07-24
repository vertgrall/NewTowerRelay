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
use x25519_dalek::PublicKey;

pub struct SendResult {
    pub files_sent: usize,
}

pub struct ReceiveResult {
    pub saved_paths: Vec<PathBuf>,
}

pub struct IncomingTransfer {
    stream: TcpStream,
    cipher: ChaCha20Poly1305,
    pub remote: Hello,
    pub offer: Offer,
    pub needs_pairing: bool,
    pub pairing_code: String,
}

pub fn send_files(
    identity: &Identity,
    trust: &TrustStore,
    addr: SocketAddr,
    paths: &[PathBuf],
) -> Result<SendResult> {
    let mut stream = TcpStream::connect(addr).context("connect to peer")?;
    let (_remote, cipher) = handshake_client(identity, trust, &mut stream)?;
    let files = collect_files(paths)?;
    write_encrypted(
        &mut stream,
        &cipher,
        &WireMessage::Control(ControlMessage::Offer(Offer {
            files: files.clone(),
        })),
    )?;

    match read_encrypted(&mut stream, &cipher)? {
        WireMessage::Control(ControlMessage::Accept) => {}
        WireMessage::Control(ControlMessage::Decline) => {
            return Err(anyhow!("transfer declined by peer"));
        }
        other => return Err(anyhow!("unexpected response: {:?}", other)),
    }

    for file in &files {
        let path = paths
            .iter()
            .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(file.name.as_str()))
            .ok_or_else(|| anyhow!("missing path for {}", file.name))?;
        send_one_file(&mut stream, &cipher, path, file)?;
    }

    write_encrypted(
        &mut stream,
        &cipher,
        &WireMessage::Control(ControlMessage::Done),
    )?;
    Ok(SendResult {
        files_sent: files.len(),
    })
}

pub fn begin_incoming(
    identity: &Identity,
    trust: &mut TrustStore,
    stream: TcpStream,
) -> Result<IncomingTransfer> {
    let (remote, offer, cipher, needs_pairing) =
        receive_connection(identity, trust, stream.try_clone()?)?;
    let remote_pk = decode_pubkey(&remote.public_key)?;
    let code = pairing_code(&identity.public_key(), &remote_pk);
    Ok(IncomingTransfer {
        stream,
        cipher,
        remote,
        offer,
        needs_pairing,
        pairing_code: code,
    })
}

impl IncomingTransfer {
    pub fn accept_and_save(mut self, trust: &mut TrustStore) -> Result<ReceiveResult> {
        trust.trust(&self.remote.device_id);
        trust.save()?;
        write_encrypted(
            &mut self.stream,
            &self.cipher,
            &WireMessage::Control(ControlMessage::Accept),
        )?;
        let dest_root = download_dir()?;
        let mut saved = Vec::new();
        let mut current_file: Option<File> = None;

        loop {
            let msg = read_encrypted(&mut self.stream, &self.cipher)?;
            match msg {
                WireMessage::Control(ControlMessage::Done) => break,
                WireMessage::Control(ControlMessage::FileComplete) => {
                    current_file = None;
                }
                WireMessage::Chunk(data) => {
                    if data.starts_with(b"FILE:") {
                        let text = String::from_utf8_lossy(&data);
                        let parts: Vec<_> = text.splitn(3, ':').collect();
                        if parts.len() == 3 {
                            let name = sanitize_filename(parts[1]);
                            let path = dest_root.join(&name);
                            if let Some(parent) = path.parent() {
                                fs::create_dir_all(parent)?;
                            }
                            current_file = Some(File::create(&path)?);
                            saved.push(path);
                        }
                    } else if let Some(ref mut file) = current_file {
                        file.write_all(&data)?;
                    }
                }
                other => return Err(anyhow!("unexpected: {:?}", other)),
            }
        }
        Ok(ReceiveResult { saved_paths: saved })
    }

    pub fn decline(mut self) -> Result<()> {
        write_encrypted(
            &mut self.stream,
            &self.cipher,
            &WireMessage::Control(ControlMessage::Decline),
        )
    }
}

fn receive_connection(
    identity: &Identity,
    trust: &TrustStore,
    mut stream: TcpStream,
) -> Result<(Hello, Offer, ChaCha20Poly1305, bool)> {
    let remote_msg = read_plain(&mut stream)?;
    let remote = match remote_msg {
        WireMessage::Control(ControlMessage::Hello(h)) => h,
        other => return Err(anyhow!("expected hello, got {:?}", other)),
    };
    let hello = Hello {
        device_id: identity.device_id.clone(),
        name: identity.name.clone(),
        public_key: B64.encode(identity.public_key().as_bytes()),
    };
    write_plain(
        &mut stream,
        &WireMessage::Control(ControlMessage::Hello(hello)),
    )?;
    let remote_pk = decode_pubkey(&remote.public_key)?;
    let cipher = session_cipher(&identity.secret(), &remote_pk);
    let needs_pairing = !trust.is_trusted(&remote.device_id);
    let offer_msg = read_encrypted(&mut stream, &cipher)?;
    let offer = match offer_msg {
        WireMessage::Control(ControlMessage::Offer(offer)) => offer,
        other => return Err(anyhow!("expected offer, got {:?}", other)),
    };
    Ok((remote, offer, cipher, needs_pairing))
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
    )?;
    let remote_msg = read_plain(stream)?;
    let remote = match remote_msg {
        WireMessage::Control(ControlMessage::Hello(h)) => h,
        other => return Err(anyhow!("expected hello, got {:?}", other)),
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
        )?;
    }
    Ok((remote, cipher))
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

fn send_one_file(
    stream: &mut TcpStream,
    cipher: &ChaCha20Poly1305,
    path: &Path,
    meta: &FileMeta,
) -> Result<()> {
    let mut file = File::open(path)?;
    let header = format!("FILE:{}:{}", meta.name, meta.size);
    write_encrypted(stream, cipher, &WireMessage::Chunk(header.into_bytes()))?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        write_encrypted(stream, cipher, &WireMessage::Chunk(buf[..n].to_vec()))?;
    }
    write_encrypted(
        stream,
        cipher,
        &WireMessage::Control(ControlMessage::FileComplete),
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
    let arr: [u8; 32] = bytes.try_into().map_err(|_| anyhow!("bad key length"))?;
    Ok(PublicKey::from(arr))
}

fn write_plain(stream: &mut TcpStream, msg: &WireMessage) -> Result<()> {
    let frame = msg.encode()?;
    stream.write_all(&frame)?;
    Ok(())
}

fn read_plain(stream: &mut TcpStream) -> Result<WireMessage> {
    let (header, payload) = read_frame(stream)?;
    WireMessage::decode_frame(&header, &payload)
}

fn write_encrypted(
    stream: &mut TcpStream,
    cipher: &ChaCha20Poly1305,
    msg: &WireMessage,
) -> Result<()> {
    let plain = msg.encode()?;
    let encrypted = encrypt(cipher, &plain)?;
    let len = (encrypted.len() as u32).to_be_bytes();
    stream.write_all(&len)?;
    stream.write_all(&encrypted)?;
    Ok(())
}

fn read_encrypted(stream: &mut TcpStream, cipher: &ChaCha20Poly1305) -> Result<WireMessage> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut encrypted = vec![0u8; len];
    stream.read_exact(&mut encrypted)?;
    let plain = decrypt(cipher, &encrypted)?;
    let header: [u8; 4] = plain[..4].try_into().unwrap();
    WireMessage::decode_frame(&header, &plain[4..])
}

fn read_frame(stream: &mut TcpStream) -> Result<([u8; 4], Vec<u8>)> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload)?;
    Ok((len_buf, payload))
}
