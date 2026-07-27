#!/usr/bin/env bash
# Install NTRelay from the standalone Linux binary (no .deb required).
set -euo pipefail

BINARY="${1:-}"
if [[ -z "$BINARY" || ! -f "$BINARY" ]]; then
  echo "Usage: sudo $0 /path/to/NewTowerRelay-*-Linux-x86_64" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

install -d /usr/local/bin
install -m 755 "$BINARY" /usr/local/bin/ntrelay

install -d /usr/share/applications
cat > /usr/share/applications/ntrelay.desktop <<'EOF'
[Desktop Entry]
Type=Application
Name=NTRelay
Comment=Encrypted LAN file sharing
Exec=ntrelay
Icon=ntrelay
Terminal=false
Categories=Network;FileTransfer;
StartupWMClass=new_tower_relay
EOF

ICON_SRC="$ROOT/assets/icons/icon-ntr-blue.png"
if [[ -f "$ICON_SRC" ]]; then
  install -d /usr/share/icons/hicolor/256x256/apps
  install -d /usr/share/icons/hicolor/48x48/apps
  install -m 644 "$ICON_SRC" /usr/share/icons/hicolor/256x256/apps/ntrelay.png
  cp "$ICON_SRC" /usr/share/icons/hicolor/48x48/apps/ntrelay.png
fi

if command -v systemctl >/dev/null 2>&1; then
  systemctl enable avahi-daemon >/dev/null 2>&1 || true
  systemctl start avahi-daemon >/dev/null 2>&1 || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
fi

echo "Installed. Run: ntreelay"
