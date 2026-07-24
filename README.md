# NewTowerRelay

Cross-platform encrypted file sharing for macOS, Linux, and Windows.

## Principles

- **Cross-platform** — one Rust codebase, native binaries per OS
- **Encrypted** — end-to-end ChaCha20-Poly1305 after X25519 key exchange
- **Easy** — auto-discover peers on LAN, drag-and-drop, accept/decline
- **Trust** — pairing code on first contact; trusted devices remembered

## Run (development)

```bash
cargo run
```

## Build release

```bash
# macOS (native arch)
cargo build --release

# Linux
cargo build --release --target x86_64-unknown-linux-gnu

# Windows (from Linux/macOS cross-compile or on Windows)
cargo build --release --target x86_64-pc-windows-msvc
```

## Usage

1. Launch **NewTowerRelay** on both computers (same Wi‑Fi/LAN).
2. Wait for the other device to appear under **Nearby devices**.
3. Select files (or drag-and-drop), pick a peer, click **Send encrypted**.
4. On the receiver, verify the **pairing code** (first time only), then **Accept**.

Received files are saved to the app data `Downloads/` folder (see `DESIGN.md`).

## Project layout

```
src/
  app.rs        — UI
  runtime.rs    — background discovery, listener, commands
  transfer.rs   — encrypted send/receive
  crypto.rs     — X25519 + ChaCha20-Poly1305
  discovery.rs  — mDNS LAN discovery
  protocol.rs   — wire messages
  config.rs     — identity + trust store
```

See [DESIGN.md](DESIGN.md) for architecture details.
