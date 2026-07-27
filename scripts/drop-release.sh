#!/usr/bin/env bash
# Build release artifacts into the drop folder (default: ~/Desktop/NewTowerBuilds).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=scripts/drop-dir.sh
source "$ROOT/scripts/drop-dir.sh"

VERSION="$(grep '^version' "$ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')"

echo "Drop folder: $DROP_DIR"
mkdir -p "$DROP_DIR"

"$ROOT/scripts/build-all-skus.sh" "$DROP_DIR"

UNIVERSAL="$DROP_DIR/NewTowerRelay-${VERSION}-macOS-Universal"
if [[ -f "$UNIVERSAL" ]]; then
  echo "Packaging NewTowerRelay.app..."
  "$ROOT/scripts/make-macos-app.sh" "$UNIVERSAL" "$DROP_DIR/NewTowerRelay.app"
  echo "→ $DROP_DIR/NewTowerRelay.app"
elif [[ -f "$DROP_DIR/NewTowerRelay-${VERSION}-macOS-AppleSilicon" ]]; then
  echo "Packaging NewTowerRelay.app (Apple Silicon)..."
  "$ROOT/scripts/make-macos-app.sh" \
    "$DROP_DIR/NewTowerRelay-${VERSION}-macOS-AppleSilicon" \
    "$DROP_DIR/NewTowerRelay.app"
  echo "→ $DROP_DIR/NewTowerRelay.app"
elif [[ -f "$DROP_DIR/NewTowerRelay-${VERSION}-macOS-Intel" ]]; then
  echo "Packaging NewTowerRelay.app (Intel)..."
  "$ROOT/scripts/make-macos-app.sh" \
    "$DROP_DIR/NewTowerRelay-${VERSION}-macOS-Intel" \
    "$DROP_DIR/NewTowerRelay.app"
  echo "→ $DROP_DIR/NewTowerRelay.app"
fi

if command -v docker >/dev/null 2>&1; then
  echo "Building Linux .deb via Docker..."
  "$ROOT/scripts/docker-build-deb.sh" "$DROP_DIR"
else
  echo "⚠ Docker not available — Linux .deb skipped (build on Linux or install Docker)"
fi

echo ""
echo "Drop folder contents:"
ls -lh "$DROP_DIR"/NewTowerRelay-* "$DROP_DIR"/*.deb "$DROP_DIR"/*.app 2>/dev/null || ls -lh "$DROP_DIR" 2>/dev/null || true
