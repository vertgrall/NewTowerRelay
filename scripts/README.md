# Build scripts

End users download executables from Releases — they do **not** need these scripts.

## Create release executables

Requires [Rust](https://rustup.rs/) on the machine doing the build.

```bash
chmod +x scripts/build-release.sh
./scripts/build-release.sh
```

Output in `dist/`:

```
dist/NewTowerRelay-0.1.0-macOS-Universal      # macOS (both archs, if toolchains installed)
dist/NewTowerRelay-0.1.0-macOS-Intel         # macOS Intel-only build
dist/NewTowerRelay-0.1.0-Linux-x86_64        # Linux
dist/NewTowerRelay-0.1.0-Windows-x86_64.exe  # Windows
```

Upload the `dist/` files to GitHub Releases (or your download page).

## Platform notes for builders

| Build on | Produces |
|----------|----------|
| Mac (Intel) | macOS Intel binary |
| Mac (Apple Silicon) | macOS Apple Silicon binary |
| Mac (with both rust targets) | macOS Universal binary |
| Linux | Linux x86_64 binary |
| Windows | Windows x86_64 `.exe` |

Cross-compiling is possible but building on each target OS is simplest.

## macOS universal binary (optional)

```bash
rustup target add x86_64-apple-darwin aarch64-apple-darwin
./scripts/build-release.sh
```

The script detects both targets and runs `lipo` automatically.
