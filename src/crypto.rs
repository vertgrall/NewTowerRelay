use crate::config::Identity;
use crate::protocol::Hello;
use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

pub const NONCE_LEN: usize = 12;

pub fn pairing_code(local: &PublicKey, remote: &PublicKey) -> String {
    let (first, second) = if local.as_bytes() <= remote.as_bytes() {
        (local, remote)
    } else {
        (remote, local)
    };
    let mut hasher = Sha256::new();
    hasher.update(first.as_bytes());
    hasher.update(second.as_bytes());
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

pub fn signing_key(identity: &Identity) -> SigningKey {
    let seed = Sha256::digest([b"ntrelay-ed25519-v1", identity.secret_key.as_slice()].concat());
    SigningKey::from_bytes(&seed.into())
}

pub fn hello_sign_bytes(hello: &Hello) -> Vec<u8> {
    format!(
        "{}|{}|{}|{}|{}",
        hello.device_id,
        hello.name,
        hello.public_key,
        hello.sign_key,
        hello.pairing_required
    )
    .into_bytes()
}

pub fn build_hello(identity: &Identity, pairing_required: bool) -> Hello {
    let signing = signing_key(identity);
    let sign_key = signing.verifying_key().to_bytes();
    let public_key =
        base64::engine::general_purpose::STANDARD.encode(identity.public_key().as_bytes());
    let sign_key_b64 = base64::engine::general_purpose::STANDARD.encode(sign_key);
    let hello = Hello {
        device_id: identity.device_id.clone(),
        name: identity.name.clone(),
        public_key,
        sign_key: sign_key_b64,
        signature: String::new(),
        pairing_required,
    };
    let signature = signing.sign(&hello_sign_bytes(&hello));
    Hello {
        signature: base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
        ..hello
    }
}

pub fn verify_hello(hello: &Hello) -> Result<()> {
    let sign_key_bytes = base64::engine::general_purpose::STANDARD
        .decode(&hello.sign_key)
        .context("decode hello sign key")?;
    let sign_key_arr: [u8; 32] = sign_key_bytes
        .try_into()
        .map_err(|_| anyhow!("invalid hello sign key length"))?;
    let verifying_key =
        VerifyingKey::from_bytes(&sign_key_arr).map_err(|err| anyhow!("invalid sign key: {err}"))?;
    let signature_bytes = base64::engine::general_purpose::STANDARD
        .decode(&hello.signature)
        .context("decode hello signature")?;
    let signature = Signature::from_slice(&signature_bytes).context("invalid hello signature")?;
    verifying_key
        .verify(&hello_sign_bytes(hello), &signature)
        .map_err(|_| anyhow!("hello signature verification failed"))
}

pub fn encrypt(cipher: &ChaCha20Poly1305, plaintext: &[u8]) -> Result<Vec<u8>> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
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

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;
    use x25519_dalek::{PublicKey, StaticSecret};

    fn test_identity(label: &str) -> Identity {
        Identity {
            device_id: format!("test-{label}"),
            name: label.to_string(),
            secret_key: StaticSecret::random_from_rng(OsRng).to_bytes(),
        }
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let secret = StaticSecret::random_from_rng(OsRng);
        let peer_secret = StaticSecret::random_from_rng(OsRng);
        let cipher = session_cipher(&secret, &PublicKey::from(&peer_secret));
        let plain = b"hello encrypted world";
        let encrypted = encrypt(&cipher, plain).unwrap();
        let restored = decrypt(&cipher, &encrypted).unwrap();
        assert_eq!(restored, plain);
    }

    #[test]
    fn pairing_code_is_symmetric() {
        let a = StaticSecret::random_from_rng(OsRng);
        let b = StaticSecret::random_from_rng(OsRng);
        let pk_a = PublicKey::from(&a);
        let pk_b = PublicKey::from(&b);
        assert_eq!(pairing_code(&pk_a, &pk_b), pairing_code(&pk_b, &pk_a));
    }

    #[test]
    fn hello_signature_roundtrip() {
        let identity = test_identity("alice");
        let hello = build_hello(&identity, true);
        verify_hello(&hello).unwrap();
    }

    #[test]
    fn hello_signature_rejects_tampered_public_key() {
        let identity = test_identity("bob");
        let mut hello = build_hello(&identity, false);
        hello.public_key = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
        assert!(verify_hello(&hello).is_err());
    }
}
