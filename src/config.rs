use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStore {
    pub trusted_peers: HashSet<String>,
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
        fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = config_dir()?.join("trusted.json");
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn trust(&mut self, device_id: &str) {
        self.trusted_peers.insert(device_id.to_string());
    }

    pub fn is_trusted(&self, device_id: &str) -> bool {
        self.trusted_peers.contains(device_id)
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
    let dir = ProjectDirs::from(APP_QUALIFIER, APP_ORG, APP_NAME)
        .map(|dirs| dirs.data_dir().join("Downloads"))
        .context("resolve data directory")?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
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
