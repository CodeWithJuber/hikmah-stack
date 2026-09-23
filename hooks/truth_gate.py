#!/usr/bin/env python3
"""Zero-install compatibility fallback for the Hikmah Truth Gate.

The primary implementation is Rust (`runtime/hikmah-kernel/src/hook.rs`). This file mirrors its
deterministic rules so a source-installed plugin keeps the same narrow Stop check on machines
without the `hikmah` binary. Both implementations are tested against `truth_gate_cases.json`.

The gate does NOT fact-check. It blocks only when an un-negated completion claim co-occurs with
an un-negated unfinished marker or a first-person future-work promise. Whole words only, negation
just before a word cancels it, and fenced or inline code is ignored. A word that names a thing is
not a marker ("the TODO list widget", "a Coming soon badge", "the search input placeholder", a
quoted "TODO"), unless the clause says a task marker is still open ("one TODO comment is left").
A promise whose clause leaves it to the user ("If you want, I'll ...", "Once you approve, we
will ...") is an offer, not deferred work. Engine mode, including the opt-in engine lift, exists
only in Rust; this fallback is rules only. Text is normalized the same
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
# A condition that leaves the promised step to the user: "If you want, I'll ...".
USER_GATE = re.compile(
    r"\b(?:(?:if|once|when|whenever|after|as soon as|should|unless) +you|let me know|would you like)\b",
    re.ASCII,
)
# A word saying a task marker is still open: "a TODO comment is left".
STILL_OPEN = re.compile(
    r"\b(?:remain|remains|remaining|left|outstanding|pending|unresolved)\b", re.ASCII
)
COMPLETION_NEGATORS = {
    "not", "never", "no", "isn't", "aren't", "wasn't", "weren't", "haven't", "hasn't", "hadn't",
    "won't", "cannot", "can't", "nearly", "almost", "partially", "partly", "yet",
}
UNFINISHED_NEGATORS = {"no", "without", "zero", "removed", "remove", "replaced", "resolved", "cleared"}
# Words after "placeholder" that make it a UI property: "placeholder text".
PLACEHOLDER_UI_TERMS = {"text", "attribute", "prop", "image", "color", "value", "copy"}
# Words before "placeholder" that make it a UI property: "the search input placeholder".
PLACEHOLDER_UI_OWNERS = {
    "input", "inputs", "search", "field", "fields", "textarea", "form", "attribute", "select",
}
# Words after "coming soon" that make it UI copy: "a Coming soon badge".
COMING_SOON_UI_TERMS = {
    "badge", "badges", "banner", "label", "labels", "page", "pages", "state", "pill", "tag",
    "text", "copy", "message", "screen", "section", "card", "notice", "ribbon", "chip",
}
# Words after todo/tbd/fixme that make it the name of a thing, not a marker ("the TODO list
# widget"). A marker still counts when the same clause says it is open.
MARKER_NOUNS = {
    "list", "lists", "app", "apps", "widget", "widgets", "item", "items", "comment", "comments",
    "component", "components", "feature", "page", "view", "board", "tracker", "example", "entry",
    "entries",
}
# Characters between a term and the word it modifies: "TODO list", "TODO-list", '"Coming soon" badge'.
WORD_GAP = " -\"'“”"
# A term right after an opening quote is mentioned, not used.
QUOTES = "\"'“”"
CLAUSE_END = ".!?\n;"
ASCII_ALNUM = frozenset("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789")
WORD_SPLIT = re.compile(r"[ \n,;:()]+")
# Payload fallback, mirrored from `parse_payload` in hook.rs.
SURROGATE_ESCAPE = re.compile(
    r"\\u[dD][89abAB][0-9a-fA-F]{2}(\\u[dD][c-fC-F][0-9a-fA-F]{2})?|\\u[dD][c-fC-F][0-9a-fA-F]{2}"
)
MESSAGE_FIELD = re.compile(r'"last_assistant_message"\s*:\s*"((?:[^"\\]|\\.)*)"')
# Same meaning as truthy(): any-case "true"/"1" strings, true, or any non-zero number.
ACTIVE_FIELD = re.compile(r'"stop_hook_active"\s*:\s*(?:"\s*(?i:true|1)\s*"|true\b|-?(?:[1-9][0-9]*(?:\.[0-9]+)?|0\.[0-9]*[1-9][0-9]*)(?:[eE][+-]?[0-9]+)?)')
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
    """The word right after `end`, across spaces, hyphens and quotes only."""
    i = end
    while i < len(text) and text[i] in WORD_GAP:
        i += 1
    j = i
    while j < len(text) and text[j] in ASCII_ALNUM:
        j += 1
    return text[i:j] or None


def previous_word(text, start):
    """The word right before `start`, across spaces, hyphens and quotes only."""
    j = start
    while j > 0 and text[j - 1] in WORD_GAP:
        j -= 1
    i = j
    while i > 0 and text[i - 1] in ASCII_ALNUM:
        i -= 1
    return text[i:j] or None


def text_after(text, end, stops):
    """Up to NEGATION_WINDOW_CHARS characters after `end`, cut at the first of `stops`."""
    window = text[end:end + NEGATION_WINDOW_CHARS]
    cuts = [i for i in (window.find(ch) for ch in stops) if i >= 0]
    return window[:min(cuts)] if cuts else window


def clause_bounds(text, start, end):
    """Index range of the clause around start..end, at most NEGATION_WINDOW_CHARS each way."""
    lo = max(0, start - NEGATION_WINDOW_CHARS)
    cut = max(text.rfind(ch, lo, start) for ch in CLAUSE_END)
    if cut >= 0:
        lo = cut + 1
    return lo, end + len(text_after(text, end, CLAUSE_END))


def is_offer(text, promises, k):
    """Whether a promise is left to the user ("If you want, I'll ..."). The condition must be in
    the promise's own clause and not past a neighbouring promise."""
    m = promises[k]
    lo, hi = clause_bounds(text, m.start(), m.end())
    if k > 0:
        lo = max(lo, promises[k - 1].end())
    if k + 1 < len(promises):
        hi = min(hi, promises[k + 1].start())
    return USER_GATE.search(text[lo:hi]) is not None


def flags_open_work(text, m):
    """Whether an unfinished-work match flags open work, rather than naming a UI element or a
    feature, or quoting the word."""
    if negated(text, m.start(), UNFINISHED_NEGATORS):
        return False
    term = m.group(0)
    if term.startswith(("<", "[")):
        return True
    if m.start() > 0 and text[m.start() - 1] in QUOTES:
        return False
    following = next_word(text, m.end())
    if term == "placeholder":
        return not (following in PLACEHOLDER_UI_TERMS
                    or previous_word(text, m.start()) in PLACEHOLDER_UI_OWNERS)
    if term == "coming soon":
        return following not in COMING_SOON_UI_TERMS
    # todo, tbd, fixme
    return (following not in MARKER_NOUNS
            or STILL_OPEN.search(text_after(text, m.end(), CLAUSE_END)) is not None)


def rules_verdict(message):
    text = normalize(message)
    if not any(not negated(text, m.start(), COMPLETION_NEGATORS) for m in COMPLETION.finditer(text)):
        return False
    if any(flags_open_work(text, m) for m in UNFINISHED.finditer(text)):
        return True
    # A promise the user has to trigger ("If you want, I'll ...") is an offer, not deferred work.
    promises = list(PROMISE.finditer(text))
    return any(not is_offer(text, promises, k) for k in range(len(promises)))


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
