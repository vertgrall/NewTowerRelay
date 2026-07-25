#!/usr/bin/env bash
# Wrap a NewTowerRelay binary in a double-clickable macOS .app bundle.
set -euo pipefail

BINARY="${1:-}"
OUT_DIR="${2:-$HOME/Desktop/NewTowerBuidls}"

if [[ -z "$BINARY" || ! -f "$BINARY" ]]; then
  echo "Usage: $0 <path-to-binary> [output-dir]" >&2
  exit 1
fi

APP="$OUT_DIR/NewTowerRelay.app"
CONTENTS="$APP/Contents"
MACOS="$CONTENTS/MacOS"

rm -rf "$APP"
mkdir -p "$MACOS"

cp "$BINARY" "$MACOS/NewTowerRelay"
chmod +x "$MACOS/NewTowerRelay"

cat > "$CONTENTS/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>NewTowerRelay</string>
  <key>CFBundleIdentifier</key>
  <string>com.newtower.relay</string>
  <key>CFBundleName</key>
  <string>NewTowerRelay</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSLocalNetworkUsageDescription</key>
  <string>NewTowerRelay discovers and connects to other devices on your local network to share files.</string>
</dict>
</plist>
EOF

echo "Created $APP"
echo "Double-click NewTowerRelay.app in Finder to run."
