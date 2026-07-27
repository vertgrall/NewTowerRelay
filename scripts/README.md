# Build scripts

End users download executables from Releases — they do **not** need these scripts.

## macOS — build once, use forever

Requires [Rust](https://rustup.rs/) only for **building**, not for daily use.

```bash
chmod +x scripts/build-release.sh scripts/install-macos.sh
./scripts/build-release.sh      # builds binary + dist/NTRelay.app
./scripts/install-macos.sh      # copies to ~/Applications/NTRelay.app
```

After that, launch **NTRelay** from Finder → Applications (double-click).  
Re-run `build-release.sh` + `install-macos.sh` only when you update the app.

Output in `dist/`:

```
dist/NTRelay.app                              # double-clickable macOS app
dist/NewTowerRelay-0.1.0-macOS-Universal      # raw binary (optional)
dist/NewTowerRelay-0.1.0-Linux-x86_64        # Linux binary
dist/NewTowerRelay-0.1.0-Windows-x86_64.exe  # Windows
```

Upload release artifacts to GitHub Releases (or your download page).

## Linux — .deb installer (recommended)

On Linux, `./scripts/build-release.sh` produces both the binary and a proper `.deb`:

```bash
./scripts/build-release.sh
sudo apt install ./dist/ntrelay_0.1.0_amd64.deb
```

Users can **double-click** the `.deb` in Mint/Ubuntu, or install from the terminal as above.

**GitHub Actions** also builds the `.deb` on every release (and via **Actions → Linux .deb → Run workflow**).

On **macOS**, use Docker (recommended for valid packages):

```bash
./scripts/docker-build-deb.sh
```

Do **not** rely on macOS-only `ar`/`tar` for production `.deb` files — use Docker or CI.

Manual binary install (no package manager):

```bash
chmod +x dist/NewTowerRelay-*-Linux-x86_64
./dist/NewTowerRelay-*-Linux-x86_64
# or: sudo ./scripts/install-linux-binary.sh dist/NewTowerRelay-*-Linux-x86_64
```

## Platform notes for builders

| Build on | Produces |
|----------|----------|
| Mac (Intel) | macOS Intel binary + NTRelay.app |
| Mac (Apple Silicon) | macOS Apple Silicon binary + NTRelay.app |
| Mac (with both rust targets) | macOS Universal binary + NTRelay.app |
| Linux | Linux x86_64 binary + .desktop launcher |
| Windows | Windows x86_64 `.exe` |

Cross-compiling is possible but building on each target OS is simplest.

## macOS universal binary (optional)

```bash
rustup target add x86_64-apple-darwin aarch64-apple-darwin
./scripts/build-release.sh
```

The script detects both targets and runs `lipo` automatically.

## Tests and coverage

Run the full suite:

```bash
cargo test
```

Line coverage (installs [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) if needed):

```bash
chmod +x scripts/test-coverage.sh
./scripts/test-coverage.sh
```

Peer discovery resilience (`peer_registry`, `discovery`, `runtime`) has dedicated unit tests — run them with:

```bash
cargo test peer_registry discovery::tests runtime::tests app::peer_label
```
