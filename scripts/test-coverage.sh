#!/usr/bin/env bash
# Run the full test suite with line coverage (requires cargo-llvm-cov).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  echo "Installing cargo-llvm-cov…"
  cargo install cargo-llvm-cov
fi

echo "Running tests with coverage…"
cargo llvm-cov --all-features --workspace --lcov --output-path lcov.info
cargo llvm-cov report --summary-only

echo ""
echo "Detailed HTML report: cargo llvm-cov report --html && open target/llvm-cov/html/index.html"
