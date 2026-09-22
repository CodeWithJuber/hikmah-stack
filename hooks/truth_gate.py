#!/usr/bin/env python3
"""Zero-install compatibility fallback for the Hikmah Truth Gate.

The primary implementation is Rust (`runtime/hikmah-kernel/src/hook.rs`). This file mirrors its
deterministic rules so a source-installed plugin keeps the same narrow Stop check on machines
without the `hikmah` binary. Both implementations are tested against `truth_gate_cases.json`.

The gate does NOT fact-check. It blocks only when an un-negated completion claim co-occurs with
an un-negated unfinished marker or a first-person future-work promise. Whole words only, negation
just before a word cancels it, and fenced or inline code is ignored. Text is normalized the same
way as in Rust (NFC, curly apostrophes, zero-width characters removed, whitespace other than
newline mapped to a space) and word boundaries are ASCII, so both implementations agree.
Malformed payloads (lone surrogate escapes, trailing data, out-of-range numbers) go through the
same three-stage parse as Rust's `parse_payload`, so a bad payload cannot switch the gate off here
while Rust still judges it (golden `payload_cases`).
"""
import json
import re
import sys
import unicodedata

BLOCK_REASON = (
    "Hikmah Truth Gate: the response claims completion while still containing unfinished work "
    "or a future-work promise. Resolve it or state the limitation explicitly."
)

FENCE = re.compile(r"```.*?(?:```|$)", re.S)
INLINE_CODE = re.compile(r"`[^`\n]*`")
COMPLETION = re.compile(
    r"\b(done|complete|completed|finished|ready|shipped|implemented|fixed|resolved|delivered)\b",
    re.ASCII,
)
UNFINISHED = re.compile(
    r"\b(todo|tbd|fixme|placeholder|coming soon)\b|<insert[^>]*>|\[insert[^\]]*\]", re.ASCII
)
PROMISE = re.compile(
    r"\b(i|we)(?:'ll| +will| +shall) +(?:(?:also|then|still|now|soon|later|next) +)?"
    r"(finish|complete|upload|create|test|verify|send|provide|add|write|fix|update|run|check|"
    r"share|push|deploy|follow up)\b",
    re.ASCII,
)
COMPLETION_NEGATORS = {
    "not", "never", "no", "isn't", "aren't", "wasn't", "weren't", "haven't", "hasn't", "hadn't",
    "won't", "cannot", "can't", "nearly", "almost", "partially", "partly", "yet",
}
UNFINISHED_NEGATORS = {"no", "without", "zero", "removed", "remove", "replaced", "resolved", "cleared"}
PLACEHOLDER_UI_TERMS = {"text", "attribute", "prop", "image", "color", "value"}
WORD_SPLIT = re.compile(r"[ \n,;:()]+")
# Payload fallback, mirrored from `parse_payload` in hook.rs.
SURROGATE_ESCAPE = re.compile(
    r"\\u[dD][89abAB][0-9a-fA-F]{2}(\\u[dD][c-fC-F][0-9a-fA-F]{2})?|\\u[dD][c-fC-F][0-9a-fA-F]{2}"
)
MESSAGE_FIELD = re.compile(r'"last_assistant_message"\s*:\s*"((?:[^"\\]|\\.)*)"')
ACTIVE_FIELD = re.compile(r'"stop_hook_active"\s*:\s*(?:"true"|"1"|(?:true|1)\b)')
NEGATION_WINDOW_CHARS = 200
ZERO_WIDTH = {"\u200b", "\u200c", "\u200d", "\u2060", "\ufeff"}
APOSTROPHES = {"\u2019", "\u2018", "\u02bc"}


def normalize(message):
    chars = []
    for c in unicodedata.normalize("NFC", message):
        if c in APOSTROPHES:
            c = "'"
        elif c in ZERO_WIDTH:
            continue
        elif c == "\n":
            pass
        elif c.isspace():
            c = " "
        chars.append(c.lower())
    text = "".join(chars)
    return INLINE_CODE.sub(" ", FENCE.sub(" ", text))


def negated(text, start, negators):
    window = text[max(0, start - NEGATION_WINDOW_CHARS):start]
    cut = max(window.rfind(ch) for ch in ".!?\n")
    sentence = window[cut + 1:]
    words = [w for w in WORD_SPLIT.split(sentence) if w][-3:]
    for word in words:
        trimmed = word.strip("".join(c for c in word if not (c.isalnum() or c == "'")))
        if trimmed in negators or trimmed.endswith("n't"):
            return True
    return False


def next_word(text, end):
    match = re.search(r"[A-Za-z0-9]+", text[end:])
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


def _reject_constant(name):
    raise ValueError(f"non-standard JSON constant {name}")


def _finite_float(text):
    value = float(text)
    if value in (float("inf"), float("-inf")):
        raise ValueError("number out of range")
    return value


def _finite_int(text):
    value = int(text)
    float(value)  # OverflowError beyond the f64 range, like serde_json
    return value


def strict_loads(text):
    """json.loads with serde_json's strictness: no NaN/Infinity, no out-of-range numbers, no
    lone surrogates, no trailing data (json.loads already rejects trailing data)."""
    value = json.loads(
        text,
        parse_constant=_reject_constant,
        parse_float=_finite_float,
        parse_int=_finite_int,
    )
    json.dumps(value, ensure_ascii=False).encode("utf-8")  # raises on a lone surrogate
    return value


def parse_payload(text):
    """Same three stages as `parse_payload` in hook.rs: strict parse; parse again with lone
    surrogate escapes replaced; finally extract the two fields the gate needs, so trailing data,
    a truncated emoji, or an odd number cannot turn the gate off."""
    try:
        return strict_loads(text)
    except Exception:
        pass
    # Keep escaped surrogate pairs; replace unpaired surrogate escapes with U+FFFD.
    sanitized = SURROGATE_ESCAPE.sub(lambda m: m.group(0) if m.group(1) else "\\ufffd", text)
    try:
        return strict_loads(sanitized)
    except Exception:
        pass
    match = MESSAGE_FIELD.search(sanitized)
    if match is None:
        return None
    try:
        message = strict_loads('"' + match.group(1) + '"')
    except Exception:
        message = ""
    return {
        "last_assistant_message": message,
        "stop_hook_active": ACTIVE_FIELD.search(sanitized) is not None,
    }


def decide(raw_bytes):
    payload = parse_payload(raw_bytes.decode("utf-8", errors="replace"))
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
