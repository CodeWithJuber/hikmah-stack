#!/usr/bin/env sh
set -eu

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust/Cargo is required to build the source checkout. Install Rust from the official Rust toolchain, then rerun." >&2
  exit 1
fi

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cargo install --path "$ROOT/runtime/hikmah-kernel" --locked
