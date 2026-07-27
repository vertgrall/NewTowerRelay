mod app;
mod config;
mod crypto;
mod discovery;
mod peer_registry;
mod probe;
mod protocol;
mod runtime;
mod transfer;

use anyhow::Result;
use config::{Identity, TrustStore};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("new_tower_relay=info".parse()?))
        .init();

    let identity = Identity::load_or_create()?;
    let trust = TrustStore::load();
    app::run(identity, trust).map_err(|e| anyhow::anyhow!("{e}"))
}
