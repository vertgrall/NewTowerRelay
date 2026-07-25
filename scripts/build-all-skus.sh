#!/usr/bin/env bash
# Build all NewTowerRelay SKU binaries into a drop folder.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')"
OUT="${1:-$HOME/Desktop/NewTowerBuidls}"
export CARGO_TARGET_DIR="$ROOT/target"

mkdir -p "$OUT"

echo "Building SKUs → $OUT"

# macOS Intel
if rustup target list --installed | grep -q x86_64-apple-darwin; then
  cargo build --release --target x86_64-apple-darwin
  cp target/x86_64-apple-darwin/release/new_tower_relay "$OUT/NewTowerRelay-${VERSION}-macOS-Intel"
  chmod +x "$OUT/NewTowerRelay-${VERSION}-macOS-Intel"
  echo "✓ macOS Intel"
fi

# macOS Apple Silicon
if rustup target list --installed | grep -q aarch64-apple-darwin; then
  cargo build --release --target aarch64-apple-darwin
  cp target/aarch64-apple-darwin/release/new_tower_relay "$OUT/NewTowerRelay-${VERSION}-macOS-AppleSilicon"
  chmod +x "$OUT/NewTowerRelay-${VERSION}-macOS-AppleSilicon"
  echo "✓ macOS Apple Silicon"
fi

# macOS Universal
if [[ -f "$OUT/NewTowerRelay-${VERSION}-macOS-Intel" && -f "$OUT/NewTowerRelay-${VERSION}-macOS-AppleSilicon" ]]; then
  lipo -create \
    "$OUT/NewTowerRelay-${VERSION}-macOS-Intel" \
    "$OUT/NewTowerRelay-${VERSION}-macOS-AppleSilicon" \
    -output "$OUT/NewTowerRelay-${VERSION}-macOS-Universal"
  chmod +x "$OUT/NewTowerRelay-${VERSION}-macOS-Universal"
  echo "✓ macOS Universal"
fi

# Windows (requires: brew install mingw-w64)
if command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  mkdir -p .cargo
  if ! grep -q x86_64-pc-windows-gnu .cargo/config.toml 2>/dev/null; then
    cat >> .cargo/config.toml <<'EOF'
[target.x86_64-pc-windows-gnu]
linker = "x86_64-w64-mingw32-gcc"
EOF
  fi
  rustup target add x86_64-pc-windows-gnu 2>/dev/null || true
  cargo build --release --target x86_64-pc-windows-gnu
  cp target/x86_64-pc-windows-gnu/release/new_tower_relay.exe "$OUT/NewTowerRelay-${VERSION}-Windows-x86_64.exe"
  echo "✓ Windows x86_64"
else
  echo "⚠ Windows skipped — install mingw-w64: brew install mingw-w64"
fi

# Linux — best built on Linux or via GitHub Actions (cross-GUI from macOS is unreliable)
if [[ "$(uname -s)" == "Linux" ]]; then
  cargo build --release
  cp target/release/new_tower_relay "$OUT/NewTowerRelay-${VERSION}-Linux-x86_64"
  chmod +x "$OUT/NewTowerRelay-${VERSION}-Linux-x86_64"
  echo "✓ Linux x86_64"
else
  echo "⚠ Linux skipped on macOS — use GitHub Actions release or build on a Linux machine"
fi

echo ""
echo "Drop folder contents:"
ls -lh "$OUT"/NewTowerRelay-* 2>/dev/null || true
