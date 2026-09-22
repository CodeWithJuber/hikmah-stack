#!/usr/bin/env sh
# Hikmah Truth Gate launcher (Stop hook).
#
# Contract: always exit 0 and print one JSON object. A missing, stale, or failing binary must
# never block a turn or loop the host, so every candidate's output is checked before use and the
# final fallback is an allow ({}).
#
# Security: this launcher never compiles code. It only runs an installed `hikmah` binary (plugin
# `bin/` first, then PATH) or the Python fallback. Building from source at Stop time would run
# cargo/rustup configuration from the user's project directory, which that project controls.
set -u

ROOT="${PLUGIN_ROOT:-${CLAUDE_PLUGIN_ROOT:-}}"
INPUT="$(cat)"

try_candidate() {
  out="$(printf '%s' "$INPUT" | "$@" 2>/dev/null)" || return 1
  case "$out" in
    "{"*) printf '%s\n' "$out"; return 0 ;;
    *) return 1 ;;
  esac
}

for bin in "$ROOT/bin/hikmah" "$ROOT/bin/hikmah.exe"; do
  if [ -n "$ROOT" ] && [ -x "$bin" ]; then
    try_candidate "$bin" hook && exit 0
  fi
done

if command -v hikmah >/dev/null 2>&1; then
  try_candidate hikmah hook && exit 0
fi

# Zero-install compatibility fallback (same rules, tested against hooks/truth_gate_cases.json).
# `python3` first; `python` covers Windows, where `python3` is often a Store stub that fails.
if [ -n "$ROOT" ] && [ -f "$ROOT/hooks/truth_gate.py" ]; then
  for py in python3 python; do
    if command -v "$py" >/dev/null 2>&1; then
      try_candidate "$py" "$ROOT/hooks/truth_gate.py" && exit 0
    fi
  done
fi

printf '{}\n'
exit 0
