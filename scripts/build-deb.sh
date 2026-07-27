#!/usr/bin/env bash
# Build ntreelay_*_amd64.deb for Debian / Ubuntu / Mint.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export CARGO_TARGET_DIR="$ROOT/target"
VERSION="$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')"
OUT_DIR="${1:-$ROOT/dist}"
BINARY="${BINARY:-}"

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

if [[ -z "$BINARY" ]]; then
  for candidate in \
    "$OUT_DIR/NewTowerRelay-${VERSION}-Linux-x86_64" \
    "$ROOT/target/x86_64-unknown-linux-gnu/release/new_tower_relay" \
    "$ROOT/target/release/new_tower_relay"; do
    if [[ -f "$candidate" ]]; then
      BINARY="$candidate"
      break
    fi
  done
fi

if [[ -z "$BINARY" || ! -f "$BINARY" ]]; then
  echo "Linux binary not found. Build first:" >&2
  echo "  ./scripts/build-release.sh   (on Linux)" >&2
  echo "  ./scripts/docker-build-deb.sh  (on macOS via Docker)" >&2
  exit 1
fi

if ! file "$BINARY" | grep -q "ELF.*x86-64"; then
  echo "Refusing to package non-Linux x86_64 binary: $BINARY" >&2
  file "$BINARY" >&2
  exit 1
fi

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE/DEBIAN"
mkdir -p "$STAGE/usr/bin"
mkdir -p "$STAGE/usr/share/applications"
mkdir -p "$STAGE/usr/share/icons/hicolor/256x256/apps"
mkdir -p "$STAGE/usr/share/icons/hicolor/48x48/apps"

install -m 755 "$BINARY" "$STAGE/usr/bin/ntrelay"

cat > "$STAGE/usr/share/applications/ntrelay.desktop" <<'EOF'
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
  if command -v convert >/dev/null 2>&1; then
    convert "$ICON_SRC" -resize 256x256 "$STAGE/usr/share/icons/hicolor/256x256/apps/ntrelay.png"
    convert "$ICON_SRC" -resize 48x48 "$STAGE/usr/share/icons/hicolor/48x48/apps/ntrelay.png"
  elif command -v sips >/dev/null 2>&1; then
    cp "$ICON_SRC" "$STAGE/usr/share/icons/hicolor/256x256/apps/ntrelay.png"
    sips -z 48 48 "$ICON_SRC" --out "$STAGE/usr/share/icons/hicolor/48x48/apps/ntrelay.png" >/dev/null
  else
    cp "$ICON_SRC" "$STAGE/usr/share/icons/hicolor/256x256/apps/ntrelay.png"
    cp "$ICON_SRC" "$STAGE/usr/share/icons/hicolor/48x48/apps/ntrelay.png"
  fi
fi

cat > "$STAGE/DEBIAN/control" <<EOF
Package: ntrelay
Version: ${VERSION}
Section: net
Priority: optional
Architecture: amd64
Depends: avahi-daemon, libxcb1, libxkbcommon0, libssl3t64 | libssl3 | libssl1.1
Maintainer: NTRelay <noreply@local>
Homepage: https://github.com/vertgrall/NewTowerRelay
Description: Encrypted LAN file sharing
 NTRelay discovers devices on your local network and transfers
 files with encryption and pairing verification.
EOF

cat > "$STAGE/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if command -v systemctl >/dev/null 2>&1; then
  systemctl enable avahi-daemon >/dev/null 2>&1 || true
  systemctl start avahi-daemon >/dev/null 2>&1 || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi
EOF
chmod 755 "$STAGE/DEBIAN/postinst"

cat > "$STAGE/DEBIAN/prerm" <<'EOF'
#!/bin/sh
set -e
exit 0
EOF
chmod 755 "$STAGE/DEBIAN/prerm"

DEB="$OUT_DIR/ntrelay_${VERSION}_amd64.deb"
rm -f "$DEB"

if command -v dpkg-deb >/dev/null 2>&1; then
  dpkg-deb --build --root-owner-group "$STAGE" "$DEB"
else
  BUILD_DIR="$(mktemp -d)"
  trap 'rm -rf "$STAGE" "$BUILD_DIR"' EXIT
  echo "2.0" > "$BUILD_DIR/debian-binary"
  # macOS bsdtar emits PAX headers Linux dpkg rejects; build tars with Python USTAR.
  python3 - "$STAGE" "$BUILD_DIR" <<'PY'
import os, sys, tarfile, pathlib
stage = pathlib.Path(sys.argv[1])
build = sys.argv[2]
fmt = tarfile.USTAR_FORMAT

control = os.path.join(build, "control.tar.gz")
with tarfile.open(control, "w:gz", format=fmt) as tar:
    for name in ("control", "postinst", "prerm"):
        p = stage / "DEBIAN" / name
        tar.add(p, arcname="./" + name, recursive=False)

data = os.path.join(build, "data.tar.gz")
with tarfile.open(data, "w:gz", format=fmt) as tar:
    for path in sorted(stage.rglob("*")):
        if path.is_dir() or "DEBIAN" in path.parts:
            continue
        rel = path.relative_to(stage)
        tar.add(path, arcname="./" + rel.as_posix(), recursive=False)
PY
  python3 "$ROOT/scripts/make-deb-ar.py" "$DEB" \
    debian-binary "$BUILD_DIR/debian-binary" \
    control.tar.gz "$BUILD_DIR/control.tar.gz" \
    data.tar.gz "$BUILD_DIR/data.tar.gz"
fi

echo "→ $DEB"
ls -lh "$DEB"
