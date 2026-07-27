#!/usr/bin/env bash
# Build Linux binary + .deb inside Docker (works on macOS).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/dist}"

if ! command -v docker >/dev/null 2>&1; then
  echo "Docker is required on macOS to build .deb packages." >&2
  exit 1
fi

mkdir -p "$OUT"

docker run --rm --platform linux/amd64 \
  -v "$ROOT:/src" \
  -w /src \
  ubuntu:22.04 \
  bash -c '
    set -euo pipefail
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq curl build-essential pkg-config libssl-dev \
      libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
      libxkbcommon-dev dpkg-dev file ca-certificates

    curl --proto "=https" -tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    source "$HOME/.cargo/env"
    export CARGO_TARGET_DIR=/src/target

    cargo build --release
    install -d /src/dist
    cp target/release/new_tower_relay "/src/dist/NewTowerRelay-$(grep ^version Cargo.toml | head -1 | sed "s/.*\"\\(.*\\)\".*/\\1/")-Linux-x86_64"
    chmod +x /src/dist/NewTowerRelay-*-Linux-x86_64
    ./scripts/build-deb.sh /src/dist
  '

echo "Done. .deb is in $OUT"
