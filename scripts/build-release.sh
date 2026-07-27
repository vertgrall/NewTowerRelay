#!/usr/bin/env bash
# Build release executable(s) and a double-clickable app bundle where supported.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export CARGO_TARGET_DIR="$ROOT/target"
VERSION="$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')"
mkdir -p dist

echo "Building NTRelay v${VERSION}..."

case "$(uname -s)" in
  Darwin)
    ARCH="$(uname -m)"
    case "$ARCH" in
      x86_64) LABEL="macOS-Intel" ;;
      arm64)  LABEL="macOS-AppleSilicon" ;;
      *)      LABEL="macOS-${ARCH}" ;;
    esac

    if rustup target list --installed | grep -q x86_64-apple-darwin && \
       rustup target list --installed | grep -q aarch64-apple-darwin; then
      echo "Building universal macOS binary..."
      cargo build --release --target x86_64-apple-darwin
      cargo build --release --target aarch64-apple-darwin
      BIN="dist/NewTowerRelay-${VERSION}-macOS-Universal"
      lipo -create \
        target/x86_64-apple-darwin/release/new_tower_relay \
        target/aarch64-apple-darwin/release/new_tower_relay \
        -output "$BIN"
      chmod +x "$BIN"
      echo "→ $BIN"
    else
      cargo build --release
      BIN="dist/NewTowerRelay-${VERSION}-${LABEL}"
      cp target/release/new_tower_relay "$BIN"
      chmod +x "$BIN"
      echo "→ $BIN"
      echo "Tip: install both macOS rust targets for a universal binary."
    fi

    echo "Packaging NTRelay.app..."
    "$ROOT/scripts/make-macos-app.sh" "$BIN" "$ROOT/dist/NTRelay.app"
    echo ""
    echo "Done. To install permanently:"
    echo "  ./scripts/install-macos.sh"
    ;;
  Linux)
    cargo build --release
    OUT="dist/NewTowerRelay-${VERSION}-Linux-x86_64"
    cp target/release/new_tower_relay "$OUT"
    chmod +x "$OUT"
    echo "→ $OUT"
    "$ROOT/scripts/make-linux-desktop.sh" "$OUT" "$ROOT/dist"
    "$ROOT/scripts/build-deb.sh" "$ROOT/dist"
    echo ""
    echo "Install: sudo apt install ./dist/ntrelay_${VERSION}_amd64.deb"
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
