#!/usr/bin/env sh
# Hikmah Stack context launcher (SessionStart and UserPromptSubmit hooks).
#
#   sh hooks/context.sh session-start   # skill routing, working rules, due commitments
#   sh hooks/context.sh prompt          # per-prompt skill routing and recalled memory
#
# Contract: always exit 0 and never block. Output is plain text, which Claude Code and Codex add
# to the model's context for these events. hooks/context.py does the work. Without a working
# Python, session-start still prints the static hooks/session_context.md and prompt prints nothing.
#
# Security: this launcher never compiles or writes anything. context.py runs only the kernel's
# read commands (recall, commitments, conflicts), and only when a `hikmah` binary is installed
# and the project's store already exists.
set -u

MODE="${1:-}"
ROOT="${PLUGIN_ROOT:-${CLAUDE_PLUGIN_ROOT:-}}"
if [ -z "$ROOT" ]; then
  ROOT="$(cd "$(dirname "$0")/.." 2>/dev/null && pwd)" || ROOT=""
fi
INPUT="$(cat)"

case "$(printf '%s' "${HIKMAH_CONTEXT:-}" | tr 'A-Z' 'a-z')" in
  0|false|off|no) exit 0 ;;
esac

# `python3` first; `python` covers Windows, where `python3` is often a Store stub that fails.
if [ -n "$ROOT" ] && [ -f "$ROOT/hooks/context.py" ]; then
  for py in python3 python; do
    if command -v "$py" >/dev/null 2>&1; then
      if out="$(printf '%s' "$INPUT" | "$py" "$ROOT/hooks/context.py" "$MODE" 2>/dev/null)"; then
        [ -n "$out" ] && printf '%s\n' "$out"
        exit 0
      fi
    fi
  done
fi

if [ "$MODE" = "session-start" ] && [ -n "$ROOT" ] && [ -f "$ROOT/hooks/session_context.md" ]; then
  cat "$ROOT/hooks/session_context.md"
fi
exit 0
