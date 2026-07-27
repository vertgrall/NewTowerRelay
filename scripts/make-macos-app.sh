#!/usr/bin/env bash
# Wrap a NewTowerRelay binary in a double-clickable macOS .app bundle.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BINARY="${1:-}"
OUT_APP="${2:-$ROOT/dist/NTRelay.app}"
ICON="${ICON:-${3:-ntr-blue}}"

if [[ -z "$BINARY" || ! -f "$BINARY" ]]; then
  echo "Usage: $0 <path-to-binary> [output.app-path] [icon-name]" >&2
  echo "  icon-name: ntr-blue (default) | satellite-dish | satellite-orbit" >&2
  exit 1
fi

case "$ICON" in
  ntr-blue|icon-ntr-blue)           ICON_PNG="$ROOT/assets/icons/icon-ntr-blue.png" ;;
  satellite-dish|icon-satellite-dish-array) ICON_PNG="$ROOT/assets/icons/icon-satellite-dish-array.png" ;;
  satellite-orbit|icon-satellite-orbit)     ICON_PNG="$ROOT/assets/icons/icon-satellite-orbit.png" ;;
  /*.png)                           ICON_PNG="$ICON" ;;
  *)                                ICON_PNG="$ROOT/assets/icons/icon-${ICON}.png" ;;
esac

if [[ ! -f "$ICON_PNG" ]]; then
  echo "Icon not found: $ICON_PNG" >&2
  exit 1
fi

VERSION="$(grep '^version' "$ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')"
CONTENTS="$OUT_APP/Contents"
MACOS="$CONTENTS/MacOS"
RESOURCES="$CONTENTS/Resources"

rm -rf "$OUT_APP"
mkdir -p "$MACOS" "$RESOURCES"

cp "$BINARY" "$MACOS/NTRelay"
chmod +x "$MACOS/NTRelay"

ICNS="$RESOURCES/AppIcon.icns"
"$ROOT/scripts/make-icon.sh" "$ICON_PNG" "$ICNS"

cat > "$CONTENTS/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>NTRelay</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundleIcons</key>
  <dict>
    <key>CFBundlePrimaryIcon</key>
    <dict>
      <key>CFBundleIconFiles</key>
      <array>
        <string>AppIcon</string>
      </array>
    </dict>
  </dict>
  <key>CFBundleIdentifier</key>
  <string>com.newtower.relay</string>
  <key>CFBundleName</key>
  <string>NTRelay</string>
  <key>CFBundleDisplayName</key>
  <string>NTRelay</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSLocalNetworkUsageDescription</key>
  <string>NTRelay discovers and connects to other devices on your local network to share files.</string>
</dict>
</plist>
EOF

echo "Created $OUT_APP (icon: $(basename "$ICON_PNG"))"
echo "Double-click NTRelay.app in Finder, or run: open \"$OUT_APP\""
