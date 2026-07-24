# NewTowerRelay — Design Principles

## Goals

1. **Cross-platform** — macOS (Intel + Apple Silicon), Linux, and Windows from one Rust codebase.
2. **Security** — end-to-end encryption; no plaintext files on the network.
3. **Ease of use** — auto-discover peers on LAN, drag-and-drop send, explicit accept/decline.
4. **Trust** — first connection requires user approval; trusted devices are remembered.

## Architecture

```
┌─────────────┐     mDNS (_newtowerrelay._tcp)     ┌─────────────┐
│   Mac /     │ ◄────────────────────────────────► │   Linux /   │
│   Windows   │     TCP + E2E encrypted frames     │   Windows   │
└─────────────┘                                    └─────────────┘
```

### Discovery

- **mdns-sd** advertises `_newtowerrelay._tcp.local` with device name and port.
- Peers appear automatically on the same Wi‑Fi/LAN.

### Encryption

- Each device has a long-term **X25519** identity key (stored in config dir).
- On connect: ECDH → shared secret → **ChaCha20-Poly1305** for all payloads.
- **Pairing code** shown on first contact (hash of both public keys) for out-of-band verification.
- Trusted peer public keys persisted locally.

### Transfer flow

1. Sender picks files → encrypted **Offer** with filenames and sizes.
2. Receiver sees prompt → **Accept** or **Decline**.
3. Sender streams encrypted chunks; receiver writes to `Downloads/NewTowerRelay/`.

### Config locations

| OS      | Path |
|---------|------|
| macOS   | `~/Library/Application Support/NewTowerRelay/` |
| Linux   | `~/.config/newtowerrelay/` |
| Windows | `%APPDATA%\NewTowerRelay\` |

## Build targets

```bash
# macOS universal (requires both toolchains + lipo)
cargo build --release --target x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin

# Linux
cargo build --release --target x86_64-unknown-linux-gnu

# Windows
cargo build --release --target x86_64-pc-windows-msvc
```

## MVP scope (v0.1)

- [x] LAN discovery
- [x] E2E encrypted transfer
- [x] Pairing / trust store
- [x] Desktop UI (send, receive, progress)
- [ ] Internet / NAT traversal (future)
- [ ] Mobile (future)
