#!/usr/bin/env bash
set -euo pipefail
umask 077
cd "$(dirname "$0")/.."
if ! command -v cargo >/dev/null 2>&1; then
    source "$HOME/.cargo/env"
fi
exec python3 bench/kernel_bench.py "$@"
