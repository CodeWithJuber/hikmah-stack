#!/usr/bin/env python3
"""Zero-install compatibility fallback for the Hikmah Truth Gate.

The primary implementation is Rust (`runtime/hikmah-kernel/src/hook.rs`). This file mirrors its
deterministic rules so a source-installed plugin keeps the same narrow Stop check on machines
without the `hikmah` binary. Both implementations are tested against `truth_gate_cases.json`.

The gate does NOT fact-check. It blocks only when an un-negated completion claim co-occurs with
an un-negated unfinished marker or a first-person future-work promise. Whole words only, negation
just before a word cancels it, and fenced or inline code is ignored.
"""
import json
import re
import sys

BLOCK_REASON = (
    "Hikmah Truth Gate: the response claims completion while still containing unfinished work "
    "or a future-work promise. Resolve it or state the limitation explicitly."
)

FENCE = re.compile(r"```.*?(?:```|$)", re.S)
INLINE_CODE = re.compile(r"`[^`\n]*`")
COMPLETION = re.compile(
    r"\b(done|complete|completed|finished|ready|shipped|implemented|fixed|resolved|delivered)\b"
)
UNFINISHED = re.compile(r"\b(todo|tbd|fixme|placeholder|coming soon)\b|<insert[^>]*>|\[insert[^\]]*\]")
PROMISE = re.compile(
    r"\b(i|we)(?:'ll|\s+will|\s+shall)\s+(?:(?:also|then|still|now|soon|later|next)\s+)?"
    r"(finish|complete|upload|create|test|verify|send|provide|add|write|fix|update|run|check|"
    r"share|push|deploy|follow up)\b"
)
COMPLETION_NEGATORS = {
    "not", "never", "no", "isn't", "aren't", "wasn't", "weren't", "haven't", "hasn't", "hadn't",
    "won't", "cannot", "can't", "nearly", "almost", "partially", "partly", "yet",
}
UNFINISHED_NEGATORS = {"no", "without", "zero", "removed", "remove", "replaced", "resolved", "cleared"}
PLACEHOLDER_UI_TERMS = {"text", "attribute", "prop", "image", "color", "value"}
WORD_SPLIT = re.compile(r"[\s,;:()]+")


def normalize(message):
    lowered = message.lower().replace("’", "'").replace("‘", "'").replace("ʼ", "'")
    return INLINE_CODE.sub(" ", FENCE.sub(" ", lowered))


def negated(text, start, negators):
    head = text[:start]
    cut = max(head.rfind(ch) for ch in ".!?\n")
    sentence = head[cut + 1:]
    words = [w.strip("".join(c for c in w if not (c.isalnum() or c == "'"))) for w in WORD_SPLIT.split(sentence) if w]
    words = [w.strip() for w in words][-3:]
    return any(w in negators or w.endswith("n't") for w in words)


def next_word(text, end):
    match = re.search(r"[^\W_]+|[0-9]+", text[end:])
    return match.group(0) if match else None


def rules_verdict(message):
    text = normalize(message)
    if not any(not negated(text, m.start(), COMPLETION_NEGATORS) for m in COMPLETION.finditer(text)):
        return False
    for m in UNFINISHED.finditer(text):
        if negated(text, m.start(), UNFINISHED_NEGATORS):
            continue
        if m.group(0) == "placeholder" and next_word(text, m.end()) in PLACEHOLDER_UI_TERMS:
            continue
        return True
    return PROMISE.search(text) is not None


def truthy(value):
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        return value.strip().lower() in ("true", "1")
    if isinstance(value, (int, float)):
        return value != 0
    return False


def decide(raw_bytes):
    try:
        payload = json.loads(raw_bytes.decode("utf-8", errors="replace"))
    except Exception:
        return {}
    if not isinstance(payload, dict) or truthy(payload.get("stop_hook_active")):
        return {}
    message = payload.get("last_assistant_message")
    if not isinstance(message, str) or not message.strip():
        return {}
    if rules_verdict(message):
        return {"decision": "block", "reason": BLOCK_REASON}
    return {}


def main():
    try:
        verdict = decide(sys.stdin.buffer.read())
    except Exception:
        verdict = {}
    print(json.dumps(verdict))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
