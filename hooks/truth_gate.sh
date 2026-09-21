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

if [ -n "$ROOT" ] && [ -x "$ROOT/bin/hikmah" ]; then
  try_candidate "$ROOT/bin/hikmah" hook && exit 0
fi

if command -v hikmah >/dev/null 2>&1; then
  try_candidate hikmah hook && exit 0
fi

# Zero-install compatibility fallback (same rules, tested against hooks/truth_gate_cases.json).
if [ -n "$ROOT" ] && command -v python3 >/dev/null 2>&1 && [ -f "$ROOT/hooks/truth_gate.py" ]; then
  try_candidate python3 "$ROOT/hooks/truth_gate.py" && exit 0
fi

printf '{}\n'
exit 0
