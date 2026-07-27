#!/usr/bin/env bash
# Install NTRelay.app to ~/Applications (no admin password required).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="${1:-$ROOT/dist/NTRelay.app}"
DEST="$HOME/Applications/NTRelay.app"

if [[ ! -d "$SRC" ]]; then
  echo "NTRelay.app not found at: $SRC" >&2
  echo "Build it first: ./scripts/build-release.sh" >&2
  exit 1
fi

mkdir -p "$HOME/Applications"
rm -rf "$DEST"
cp -R "$SRC" "$DEST"

echo "Installed to $DEST"
echo ""
echo "Launch from Finder → Applications → NTRelay"
echo "Or run: open \"$DEST\""
echo ""
echo "First launch: if macOS blocks the app, right-click → Open → Open."
