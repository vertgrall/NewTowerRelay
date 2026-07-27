use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use rand::rngs::OsRng;
use x25519_dalek::{PublicKey, StaticSecret};

const APP_QUALIFIER: &str = "com";
const APP_ORG: &str = "NewTower";
const APP_NAME: &str = "NewTowerRelay";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub device_id: String,
    pub name: String,
    pub secret_key: [u8; 32],
}

impl Identity {
    pub fn load_or_create() -> Result<Self> {
        let path = config_dir()?.join("identity.json");
        if path.exists() {
            let raw = fs::read_to_string(&path).context("read identity")?;
            return serde_json::from_str(&raw).context("parse identity");
        }
        let identity = Self {
            device_id: uuid_simple(),
            name: default_device_name(),
            secret_key: StaticSecret::random_from_rng(OsRng).to_bytes(),
        };
        identity.save()?;
        Ok(identity)
    }

    pub fn save(&self) -> Result<()> {
        let path = config_dir()?.join("identity.json");
        let raw = serde_json::to_string_pretty(self)?;
        fs::write(path, raw)?;
        Ok(())
    }

    pub fn secret(&self) -> StaticSecret {
        StaticSecret::from(self.secret_key)
    }

    pub fn public_key(&self) -> PublicKey {
        PublicKey::from(&self.secret())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerTrust {
    Unknown,
    Verified,
    KeyMismatch,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStore {
    /// device_id -> base64-encoded X25519 public key
    peers: HashMap<String, String>,
}

#[derive(Deserialize)]
struct LegacyTrustStore {
    trusted_peers: Vec<String>,
}

impl TrustStore {
    pub fn load() -> Self {
        let path = match config_dir() {
            Ok(dir) => dir.join("trusted.json"),
            Err(_) => return Self::default(),
        };
        if !path.exists() {
            return Self::default();
        }
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(_) => return Self::default(),
        };
        if let Ok(store) = serde_json::from_str::<TrustStore>(&raw) {
            return store;
        }
        // Legacy format stored device IDs only — discard and require re-pairing with pinned keys.
        if serde_json::from_str::<LegacyTrustStore>(&raw).is_ok() {
            return Self::default();
        }
        Self::default()
    }

    pub fn save(&self) -> Result<()> {
        let path = config_dir()?.join("trusted.json");
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn trust_peer(&mut self, device_id: &str, public_key_b64: &str) {
        self.peers
            .insert(device_id.to_string(), public_key_b64.to_string());
    }

    pub fn check_peer(&self, device_id: &str, public_key_b64: &str) -> PeerTrust {
        match self.peers.get(device_id) {
            None => PeerTrust::Unknown,
            Some(stored) if stored == public_key_b64 => PeerTrust::Verified,
            Some(_) => PeerTrust::KeyMismatch,
        }
    }

    pub fn is_pinned(&self, device_id: &str, public_key_b64: &str) -> bool {
        matches!(
            self.check_peer(device_id, public_key_b64),
            PeerTrust::Verified
        )
    }

    pub fn has_entry(&self, device_id: &str) -> bool {
        self.peers.contains_key(device_id)
    }
}

pub fn config_dir() -> Result<PathBuf> {
    ProjectDirs::from(APP_QUALIFIER, APP_ORG, APP_NAME)
        .map(|dirs| dirs.config_dir().to_path_buf())
        .context("resolve config directory")
        .map(|path| {
            fs::create_dir_all(&path).ok();
            path
        })
}

pub fn download_dir() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(dir) = test_download_override() {
        fs::create_dir_all(&dir)?;
        return Ok(dir);
    }

    if let Some(user_dirs) = directories::UserDirs::new() {
        if let Some(downloads) = user_dirs.download_dir() {
            let dir = downloads.to_path_buf();
            fs::create_dir_all(&dir)?;
            return Ok(dir);
        }
    }

    if let Some(home) = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()) {
        let dir = home.join("Downloads");
        fs::create_dir_all(&dir)?;
        return Ok(dir);
    }

    Err(anyhow::anyhow!("could not resolve Downloads folder"))
}

#[cfg(test)]
fn test_download_override() -> Option<PathBuf> {
    use std::cell::RefCell;
    thread_local! {
        static OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }
    OVERRIDE.with(|slot| slot.borrow().clone())
}

#[cfg(test)]
pub fn set_download_dir_for_tests(dir: PathBuf) {
    use std::cell::RefCell;
    thread_local! {
        static OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }
    OVERRIDE.with(|slot| *slot.borrow_mut() = Some(dir));
}

fn default_device_name() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "NewTower Device".to_string())
}

fn uuid_simple() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..16)
        .map(|_| format!("{:02x}", rng.gen::<u8>()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_store_pins_public_keys() {
        let mut store = TrustStore::default();
        store.trust_peer("dev-a", "pk-a");
        assert_eq!(store.check_peer("dev-a", "pk-a"), PeerTrust::Verified);
        assert_eq!(store.check_peer("dev-a", "pk-b"), PeerTrust::KeyMismatch);
        assert_eq!(store.check_peer("dev-b", "pk-a"), PeerTrust::Unknown);
    }
}
