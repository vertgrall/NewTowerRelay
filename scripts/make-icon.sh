#!/usr/bin/env bash
# Convert a 1024x1024 PNG into a macOS .icns file.
set -euo pipefail

PNG="${1:-}"
OUT_ICNS="${2:-}"

if [[ -z "$PNG" || ! -f "$PNG" ]]; then
  echo "Usage: $0 <icon.png> [output.icns]" >&2
  exit 1
fi

if [[ -z "$OUT_ICNS" ]]; then
  OUT_ICNS="${PNG%.png}.icns"
fi

WORK="$(mktemp -d)"
ICONSET="$WORK/AppIcon.iconset"
mkdir -p "$ICONSET"

# macOS iconutil requires these exact filenames.
make_icon() {
  local size="$1"
  local name="$2"
  sips -z "$size" "$size" "$PNG" --out "$ICONSET/$name" >/dev/null
}

make_icon 16  icon_16x16.png
make_icon 32  icon_16x16@2x.png
make_icon 32  icon_32x32.png
make_icon 64  icon_32x32@2x.png
make_icon 128 icon_128x128.png
make_icon 256 icon_128x128@2x.png
make_icon 256 icon_256x256.png
make_icon 512 icon_256x256@2x.png
make_icon 512 icon_512x512.png
make_icon 1024 icon_512x512@2x.png

iconutil -c icns "$ICONSET" -o "$OUT_ICNS"
rm -rf "$WORK"

echo "Created $OUT_ICNS"
