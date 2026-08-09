#!/usr/bin/env sh
set -eu

ROOT="${PLUGIN_ROOT:-${CLAUDE_PLUGIN_ROOT:-}}"

if command -v hikmah >/dev/null 2>&1; then
  exec hikmah hook
fi

if [ -n "$ROOT" ] && [ -x "$ROOT/bin/hikmah" ]; then
  exec "$ROOT/bin/hikmah" hook
fi

if [ -n "$ROOT" ] && command -v cargo >/dev/null 2>&1 && [ -f "$ROOT/runtime/hikmah-kernel/Cargo.toml" ]; then
  exec cargo run --quiet --manifest-path "$ROOT/runtime/hikmah-kernel/Cargo.toml" -- hook
fi

# Zero-install compatibility only. Rust is the primary runtime.
if [ -n "$ROOT" ] && command -v python3 >/dev/null 2>&1 && [ -f "$ROOT/hooks/truth_gate.py" ]; then
  exec python3 "$ROOT/hooks/truth_gate.py"
fi

printf '{}\n'
