#!/usr/bin/env python3
"""Zero-install compatibility fallback for Hikmah Truth Gate.

The primary implementation is Rust (`runtime/hikmah-kernel`). This file remains
only so a source-installed plugin can keep its narrow Stop check on machines that
do not yet have the Hikmah binary or Rust toolchain.

Conservative Codex Stop hook for Hikmah Stack.

This hook does NOT fact-check. It only catches obvious unfinished placeholders or
future-work promises in a response that simultaneously claims completion.
"""
import json
import re
import sys


def main() -> int:
    try:
        payload = json.load(sys.stdin)
    except Exception:
        print(json.dumps({}))
        return 0

    if payload.get("stop_hook_active"):
        print(json.dumps({}))
        return 0

    text = (payload.get("last_assistant_message") or "").strip()
    if not text:
        print(json.dumps({}))
        return 0

    lower = text.lower()
    completion = re.search(r"\b(done|complete|completed|finished|ready|shipped|implemented|fixed)\b", lower)
    unfinished = re.search(r"\b(todo|tbd|fixme|placeholder|coming soon)\b|<insert[^>]*>|\[insert[^\]]*\]", lower)
    future_promise = re.search(r"\b(i(?:'ll| will)|we(?:'ll| will))\s+(finish|complete|upload|create|test|verify|send|provide)\b", lower)

    if completion and (unfinished or future_promise):
        print(json.dumps({
            "decision": "block",
            "reason": "Hikmah Truth Gate: the response claims completion but still contains an unfinished placeholder or future-work promise. Resolve it or state the limitation explicitly."
        }))
    else:
        print(json.dumps({}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
