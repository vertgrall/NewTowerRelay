#!/usr/bin/env bash
# Build a release executable for the current platform into dist/
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Use project-local target dir (avoid sandbox or global CARGO_TARGET_DIR overrides)
export CARGO_TARGET_DIR="$ROOT/target"

VERSION="$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')"
mkdir -p dist

echo "Building NewTowerRelay v${VERSION}..."

case "$(uname -s)" in
  Darwin)
    ARCH="$(uname -m)"
    case "$ARCH" in
      x86_64) LABEL="macOS-Intel" ;;
      arm64)  LABEL="macOS-AppleSilicon" ;;
      *)      LABEL="macOS-${ARCH}" ;;
    esac

    # Universal binary when both macOS targets are installed
    if rustup target list --installed | grep -q x86_64-apple-darwin && \
       rustup target list --installed | grep -q aarch64-apple-darwin; then
      echo "Building universal macOS binary..."
      cargo build --release --target x86_64-apple-darwin
      cargo build --release --target aarch64-apple-darwin
      OUT="dist/NewTowerRelay-${VERSION}-macOS-Universal"
      lipo -create \
        target/x86_64-apple-darwin/release/new_tower_relay \
        target/aarch64-apple-darwin/release/new_tower_relay \
        -output "$OUT"
      chmod +x "$OUT"
      echo "→ $OUT"
    else
      cargo build --release
      OUT="dist/NewTowerRelay-${VERSION}-${LABEL}"
      cp target/release/new_tower_relay "$OUT"
      chmod +x "$OUT"
      echo "→ $OUT"
      echo "Tip: install both macOS rust targets for a universal binary."
    fi
    ;;
  Linux)
    cargo build --release
    OUT="dist/NewTowerRelay-${VERSION}-Linux-x86_64"
    cp target/release/new_tower_relay "$OUT"
    chmod +x "$OUT"
    echo "→ $OUT"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    cargo build --release
    OUT="dist/NewTowerRelay-${VERSION}-Windows-x86_64.exe"
    cp target/release/new_tower_relay.exe "$OUT"
    echo "→ $OUT"
    ;;
  *)
    echo "Unsupported platform: $(uname -s)" >&2
    exit 1
    ;;
esac

echo "Done."
