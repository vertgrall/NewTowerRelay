use anyhow::{Context, Result};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use rand::RngCore;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

pub const NONCE_LEN: usize = 12;

pub fn pairing_code(local: &PublicKey, remote: &PublicKey) -> String {
    let mut hasher = Sha256::new();
    hasher.update(local.as_bytes());
    hasher.update(remote.as_bytes());
    let digest = hasher.finalize();
    format!(
        "{:03}-{:03}",
        u16::from_be_bytes([digest[0], digest[1]]) % 1000,
        u16::from_be_bytes([digest[2], digest[3]]) % 1000
    )
}

pub fn session_cipher(local_secret: &StaticSecret, remote_public: &PublicKey) -> ChaCha20Poly1305 {
    let shared = local_secret.diffie_hellman(remote_public);
    let key = Sha256::digest(shared.as_bytes());
    ChaCha20Poly1305::new_from_slice(&key).expect("32-byte key")
}

pub fn encrypt(cipher: &ChaCha20Poly1305, plaintext: &[u8]) -> Result<Vec<u8>> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, Payload { msg: plaintext, aad: b"" })
        .context("encrypt")?;
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

pub fn decrypt(cipher: &ChaCha20Poly1305, data: &[u8]) -> Result<Vec<u8>> {
    anyhow::ensure!(data.len() > NONCE_LEN, "ciphertext too short");
    let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, Payload { msg: ciphertext, aad: b"" })
        .context("decrypt")
}
