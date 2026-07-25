# NewTowerRelay

Cross-platform encrypted file sharing for macOS, Linux, and Windows.

## Get started

1. **Download** the executable for your platform → **[SETUP.md](SETUP.md)**
2. **Run** it on two computers on the same Wi‑Fi
3. **Send** files — encrypted, with accept/decline on the receiver

No Rust or build tools needed for users.

| Platform | Download |
|----------|----------|
| macOS | `NewTowerRelay-*-macOS-Universal` |
| Linux | `NewTowerRelay-*-Linux-x86_64` |
| Windows | `NewTowerRelay-*-Windows-x86_64.exe` |

## How it works

- Auto-discovers peers on your LAN
- End-to-end encryption (X25519 + ChaCha20-Poly1305)
- Pairing code on first contact; trusted devices remembered

See [DESIGN.md](DESIGN.md) for architecture.  
See [scripts/README.md](scripts/README.md) if you need to **build** executables for release.
