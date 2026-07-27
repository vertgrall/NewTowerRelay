#!/usr/bin/env bash
# Create a .desktop launcher for the Linux binary.
set -euo pipefail

BINARY="${1:-}"
OUT_DIR="${2:-dist}"

if [[ -z "$BINARY" || ! -f "$BINARY" ]]; then
  echo "Usage: $0 <path-to-binary> [output-dir]" >&2
  exit 1
fi

ABS_BINARY="$(cd "$(dirname "$BINARY")" && pwd)/$(basename "$BINARY")"
DESKTOP="$OUT_DIR/NTRelay.desktop"

cat > "$DESKTOP" <<EOF
[Desktop Entry]
Type=Application
Name=NTRelay
Comment=Encrypted LAN file sharing
Exec=${ABS_BINARY}
Icon=folder-download
Terminal=false
Categories=Network;FileTransfer;
StartupWMClass=new_tower_relay
EOF

chmod +x "$DESKTOP"
echo "Created $DESKTOP"
echo "Copy to ~/.local/share/applications/ to pin to your app menu."
