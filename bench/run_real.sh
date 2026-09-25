#!/usr/bin/env bash
set -euo pipefail
set +x
umask 077
cd "$(dirname "$0")/.."
benchmark_key=${TYPESAFE_API_KEY:-}
unset TYPESAFE_API_KEY
trap 'unset TYPESAFE_API_KEY benchmark_key' EXIT

if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi

cargo build --locked --release -p hikmah-kernel --example real_bench_port
python3 -m unittest discover -s bench -p test_real_datasets.py
python3 bench/real_datasets.py prepare

if [ -z "$benchmark_key" ]; then
  if [ ! -t 0 ]; then
    echo 'Supply TYPESAFE_API_KEY from your vault environment before this script.' >&2
    exit 1
  fi
  read -r -s -p 'TypeSafe API key (hidden): ' benchmark_key
  printf '\n'
fi
export TYPESAFE_API_KEY="$benchmark_key"
unset benchmark_key
python3 bench/real_datasets.py run --live --limit 0 --max-calls 20000 \
  --out .benchmark-results/real-live-v1 "$@"
