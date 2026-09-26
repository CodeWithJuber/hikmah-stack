//! Truth Gate: a narrow Stop-hook check for responses that claim completion while leaving work
//! unfinished or deferred. It is not a fact-checker.
//!
//! Deterministic rules (default):
//! - match whole words only (`incomplete` is not `complete`, `already` is not `ready`);
//! - ignore a completion word or placeholder that is negated just before it
//!   (`not done`, `no placeholder text remains`);
//! - ignore fenced and inline code, where TODOs are usually quoted legacy code;
//! - normalize first (Unicode NFC, curly apostrophes, zero-width characters, every whitespace
//!   character except newline to a space) and use ASCII word boundaries, so the Rust and Python
//!   implementations agree on unusual Unicode;
//! - block only when an un-negated completion claim co-occurs with an un-negated unfinished
//!   marker or a first-person future-work promise;
//! - a word that names a thing is not a marker: `todo`, `tbd` or `fixme` followed by a feature
//!   noun such as `list` or `app` (`the TODO list widget`), `coming soon` followed by a UI word
//!   such as `badge` or `page`, and `placeholder` after `input`, `search` or `field`. Each still
//!   counts when the rest of its clause says it is open (`remains`, `left`, `pending`, `still
//!   needs`). `todo`, `tbd` or `fixme` followed by `comment` or `item` names a thing only when
//!   the rest of its clause says it was dealt with (`the TODO comment in proxy.ts is now
//!   handled`) and nothing in the clause says it is open, because `I left a TODO comment` is open
//!   work. `placeholder` before `text` (the older rule) is always UI. A term right after an
//!   opening quote is mentioned (`the badge reads "Coming soon"`), except marker syntax (`"TODO:
//!   retries"`, `FIXME(`), which always counts;
//! - a clause ends at `.`, `!`, `?`, `;` or a newline, but not at a `.` directly followed by a
//!   letter or digit (`proxy.ts`, `v2.1`);
//! - a promise left to the user is an offer, not deferred work: an idiom asking for the user's
//!   permission or trigger (`if you want`, `once you approve`, `when you're ready`, `let me know
//!   if you'd like`, `would you like`) in the promise's own comma-delimited segment or opening
//!   the segment before it (`If you want, I'll ...`). A condition about product behaviour (`when
//!   you visit /old`, `if you add two plans`) is not one, and a condition cannot reach across a
//!   coordinated clause (`..., and I'll ...`), past a neighbouring promise, or past a clause end.
//!
//! Optional engine mode: a typed decision engine (for example Jev) estimates the probability that
//! the completion claim would fail verification (a test run of the requested change). The gate
//! blocks when the rules block **or** `p >= threshold`. By default the deterministic rules are a
//! hard floor an engine cannot lift, and the engine can only add blocks. Opting in with a lift
//! value (`GateSettings::lift`, `HIKMAH_HOOK_ENGINE_LIFT`) lets an admitted engine answer with
//! `p < lift` remove a rules block; nothing removes an engine block. Any engine failure,
//! abstention, or rejected response leaves the rules' verdict in place.
//!
//! Why this question and threshold: on 600 held-out real agent "done" messages (harness-bench
//! run 1), the rules caught 1 of 295 false completions, the previous engine question ("does the
//! message admit unfinished work?") caught 8.5%, and this outcome question caught 16.3% with a
//! 7.9% false-block rate at 0.60, a threshold chosen on a separate dev set. The message alone
//! cannot catch most false completions; treat this as a screen, not a verifier.
//!
//! Opt-in measurement: with a record store (`HIKMAH_HOOK_RECORD`), every probability the engine
//! answers is appended there as a `prediction` trace (family `truth_gate.false_completion.v2`)
//! after the verdict is written, so outcomes recorded later can set the threshold from data
//! (`hikmah gate-threshold`). Recording is best effort and never changes the verdict.
//!
//! `hooks/truth_gate_cases.json` holds golden cases shared with the Python fallback.
use crate::decision_port::{ask, DecisionEngine, DecisionRequest, Question, QuestionKind};
use crate::error::Result;
use crate::ledger::MemoryStore;
use crate::policy::KernelPolicy;
use crate::trace::Trace;
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};
use std::path::Path;
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

/// Chosen on harness-bench run 1 dev (300 messages; highest recall with a false-block rate of at
/// most 10%) and confirmed on its held-out test split. Re-measure on your own traffic with
/// `HIKMAH_HOOK_RECORD`, `hikmah outcome`, and `hikmah gate-threshold`.
pub const DEFAULT_ENGINE_THRESHOLD: f64 = 0.6;
/// Calibration family of the v2 outcome question. Calibration must not mix it with v1.
pub const GATE_FAMILY: &str = "truth_gate.false_completion.v2";
const MAX_ENGINE_STATE_CHARS: usize = 8_000;
/// How far back (in characters) negation is looked for, which keeps the check linear.
const NEGATION_WINDOW_CHARS: usize = 200;

const BLOCK_REASON: &str = "Hikmah Truth Gate: the response claims completion while still containing unfinished work or a future-work promise. Resolve it or state the limitation explicitly.";

/// Idioms that leave a promised step to the user's permission or trigger: `if you want`,
/// `once you approve`, `when you're ready`, `let me know if you'd like`, `would you like`.
/// A condition about how the product behaves (`when you visit /old`, `if you add two plans`) or
/// about anything else (`if you don't mind waiting`, `let me know if anything breaks`) is not one.
/// `after you merge` is not one: work promised for after the merge is deferred work.
const USER_GATE: &str = r"(?:if|once|when|whenever|after|as soon as) +you(?:'d| +would)? +(?:want|wish|like|prefer|approve|confirm|agree|say so)|(?:once|when|after|as soon as) +you +review|(?:if|once|when|whenever) +you(?:'re| +are) +(?:ready|happy|ok|okay)|should +you +(?:want|wish|prefer)|would +you +like|let +me +know(?: +if +you(?:'d| +would)? +(?:want|like|prefer)| +and)";

struct Rules {
    fence: Regex,
    inline_code: Regex,
    completion: Regex,
    unfinished: Regex,
    promise: Regex,
    /// A user condition anywhere in a promise's own segment: `I'll push it once you approve`.
    user_gate: Regex,
    /// A user condition that opens the segment before a promise: `If you want, I'll ...`.
    fronted_gate: Regex,
    /// A word saying a named thing is still open: `a TODO comment is left`, `the search
    /// placeholder still needs real copy`, `the TODO list still has 3 open entries`.
    still_open: Regex,
    /// A statement that a named marker was dealt with: `the TODO comment ... is now handled`.
    resolution: Regex,
}

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| Rules {
        fence: Regex::new(r"(?s)```.*?(?:```|$)").expect("fence regex"),
        inline_code: Regex::new(r"`[^`\n]*`").expect("inline code regex"),
        completion: Regex::new(
            r"(?-u:\b)(done|complete|completed|finished|ready|shipped|implemented|fixed|resolved|delivered)(?-u:\b)",
        )
        .expect("completion regex"),
        unfinished: Regex::new(
            r"(?-u:\b)(todo|tbd|fixme|placeholder|coming soon)(?-u:\b)|<insert[^>]*>|\[insert[^\]]*\]",
        )
        .expect("unfinished regex"),
        promise: Regex::new(
            r"(?-u:\b)(i|we)(?:'ll| +will| +shall) +(?:(?:also|then|still|now|soon|later|next) +)?(finish|complete|upload|create|test|verify|send|provide|add|write|fix|update|run|check|share|push|deploy|follow up)(?-u:\b)",
        )
        .expect("promise regex"),
        user_gate: Regex::new(&format!(r"(?-u:\b)(?:{USER_GATE})(?-u:\b)"))
            .expect("user gate regex"),
        fronted_gate: Regex::new(&format!(r"^ *(?:(?:and|but|so) +)?(?:{USER_GATE})(?-u:\b)"))
            .expect("fronted gate regex"),
        still_open: Regex::new(
            r"(?-u:\b)(?:remain|remains|remaining|left|outstanding|pending|unresolved|still +(?:needs?|lacks?|requires?)|still +(?:has|have) +(?:[0-9]+|some|several|a few|two|three|many) +open)(?-u:\b)",
        )
        .expect("still open regex"),
        resolution: Regex::new(
            r"(?-u:\b)(?:is|are|was|were|been|got)(?: +(?:now|all|also|already))? +(?:handled|resolved|removed|addressed|fixed|done|implemented|cleared|closed|gone|deleted)(?-u:\b)",
        )
        .expect("resolution regex"),
    })
}

const COMPLETION_NEGATORS: &[&str] = &[
    "not",
    "never",
    "no",
    "isn't",
    "aren't",
    "wasn't",
    "weren't",
    "haven't",
    "hasn't",
    "hadn't",
    "won't",
    "cannot",
    "can't",
    "nearly",
    "almost",
    "partially",
    "partly",
    "yet",
];
const UNFINISHED_NEGATORS: &[&str] = &[
    "no", "without", "zero", "removed", "remove", "replaced", "resolved", "cleared",
];
/// Words after `placeholder` that make it a UI property: `placeholder text`.
const PLACEHOLDER_UI_TERMS: &[&str] = &[
    "text",
    "attribute",
    "prop",
    "image",
    "color",
    "value",
    "copy",
];
/// Words before `placeholder` that make it a UI property: `the search input placeholder`.
const PLACEHOLDER_UI_OWNERS: &[&str] = &[
    "input",
    "inputs",
    "search",
    "field",
    "fields",
    "textarea",
    "form",
    "attribute",
    "select",
];
/// Words after `coming soon` that make it UI copy: `a Coming soon badge`.
const COMING_SOON_UI_TERMS: &[&str] = &[
    "badge", "badges", "banner", "label", "labels", "page", "pages", "state", "pill", "tag",
    "text", "copy", "message", "screen", "section", "card", "notice", "ribbon", "chip",
];
/// Words after `todo`, `tbd` or `fixme` that make it the name of a feature, not a marker:
/// `the TODO list widget`, `the todo app`. It still counts when the rest of its clause says it is
/// open (`the TODO list page is still pending`).
const NAME_NOUNS: &[&str] = &[
    "list",
    "lists",
    "app",
    "apps",
    "widget",
    "widgets",
    "component",
    "components",
    "feature",
    "page",
    "view",
    "board",
    "tracker",
    "example",
];
/// Words after `todo`, `tbd` or `fixme` that usually name an open marker in code
/// (`I left a TODO comment`, `two TODO items: ...`). They name a thing only when the rest of the
/// clause says it was dealt with (`the TODO comment in proxy.ts is now handled`).
const MARKER_NOUNS: &[&str] = &["comment", "comments", "item", "items", "entry", "entries"];
/// Characters between a term and the word it modifies: `TODO list`, `TODO-list`, `"Coming soon" badge`.
const WORD_GAP: &[char] = &[' ', '-', '"', '\'', '\u{201c}', '\u{201d}'];
/// A term right after an opening quote is mentioned, not used: `the "Coming soon" badge`.
const QUOTES: &[char] = &['"', '\'', '\u{201c}', '\u{201d}'];
const CLAUSE_END: [char; 5] = ['.', '!', '?', '\n', ';'];

fn normalize(message: &str) -> String {
    let mut text = String::with_capacity(message.len());
    for c in message.nfc() {
        let c = match c {
            '\u{2019}' | '\u{2018}' | '\u{02bc}' => '\'',
            '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}' => continue,
            '\n' => '\n',
            c if c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c) => ' ',
            c => c,
        };
        text.extend(c.to_lowercase());
    }
    let rules = rules();
    let without_fences = rules.fence.replace_all(&text, " ");
    rules
        .inline_code
        .replace_all(&without_fences, " ")
        .into_owned()
}

fn is_separator(c: char) -> bool {
    matches!(c, ' ' | '\n' | ',' | ';' | ':' | '(' | ')')
}

fn negated(text: &str, start: usize, negators: &[&str]) -> bool {
    let head = &text[..start];
    // Bounded look-back keeps the rules linear on long, unpunctuated text.
    let window_start = head
        .char_indices()
        .rev()
        .nth(NEGATION_WINDOW_CHARS - 1)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let window = &head[window_start..];
    // Look back within the current sentence only.
    let sentence = match window.rfind(['.', '!', '?', '\n']) {
        Some(i) => &window[i + 1..],
        None => window,
    };
    sentence
        .split(is_separator)
        .filter(|w| !w.is_empty())
        .rev()
        .take(3)
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\''))
        .any(|w| negators.contains(&w) || w.ends_with("n't"))
}

/// The word right after `end`, across spaces, hyphens, and quotes only (so `TODO: list` has none).
fn next_word(text: &str, end: usize) -> Option<&str> {
    let rest = text[end..].trim_start_matches(WORD_GAP);
    let len = rest
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(rest.len());
    (len > 0).then(|| &rest[..len])
}

/// The first ASCII word after `end`, across any punctuation (`placeholder: text`,
/// `placeholder (text)`). This is how the `placeholder text` check has always read the next word.
fn loose_next_word(text: &str, end: usize) -> Option<&str> {
    text[end..]
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find(|w| !w.is_empty())
}

/// The word right before `start`, across spaces, hyphens, and quotes only.
fn previous_word(text: &str, start: usize) -> Option<&str> {
    let head = text[..start].trim_end_matches(WORD_GAP);
    // Trimming (not `rfind(..) + 1`) keeps the cut on a char boundary after a non-ASCII letter.
    let from = head
        .trim_end_matches(|c: char| c.is_ascii_alphanumeric())
        .len();
    (from < head.len()).then(|| &head[from..])
}

/// Whether the character `c` at byte `i` ends a clause. A `.` directly followed by a letter or
/// digit does not: it sits inside a file name or a version (`proxy.ts`, `page.tsx`, `v2.1`).
fn ends_clause(text: &str, i: usize, c: char) -> bool {
    match c {
        '.' => !text[i + 1..].starts_with(|n: char| n.is_ascii_alphanumeric()),
        c => CLAUSE_END.contains(&c),
    }
}

/// Up to `NEGATION_WINDOW_CHARS` characters after `end`, cut at the first clause end.
fn clause_tail(text: &str, end: usize) -> &str {
    let tail = &text[end..];
    let window = tail
        .char_indices()
        .nth(NEGATION_WINDOW_CHARS)
        .map_or(tail, |(i, _)| &tail[..i]);
    let cut = window
        .char_indices()
        .find(|&(i, c)| ends_clause(text, end + i, c))
        .map_or(window.len(), |(i, _)| i);
    &window[..cut]
}

/// Byte range of the clause around `start..end`, looking at most `NEGATION_WINDOW_CHARS`
/// characters each way.
fn clause_bounds(text: &str, start: usize, end: usize) -> (usize, usize) {
    let head = &text[..start];
    let from = head
        .char_indices()
        .rev()
        .nth(NEGATION_WINDOW_CHARS - 1)
        .map_or(0, |(i, _)| i);
    // Every clause end is ASCII, so the clause starts one byte after it.
    let from = head[from..]
        .char_indices()
        .rev()
        .find(|&(i, c)| ends_clause(text, from + i, c))
        .map_or(from, |(i, _)| from + i + 1);
    (from, end + clause_tail(text, end).len())
}

/// Whether a first-person promise is left to the user: an offer, not deferred work. The condition
/// is an idiom asking for the user's permission or trigger ([`USER_GATE`]) and must sit either in
/// the promise's own comma-delimited segment (`I'll push it once you approve`, `Let me know and
/// I'll ...`) or open the segment just before it (`If you want, I'll ...`, `When you're ready,
/// I'll ...`). A fronted condition does not reach across a coordinated clause (`..., and I'll`),
/// except after `Let me know if you'd like,`. Nothing reaches past a neighbouring promise or a
/// clause end, so an offer cannot excuse a second promise (`If you want, I'll update the
/// changelog, and I'll test it later`).
fn is_offer(text: &str, promises: &[regex::Match], k: usize) -> bool {
    let m = &promises[k];
    let (from, to) = clause_bounds(text, m.start(), m.end());
    let from = k
        .checked_sub(1)
        .map_or(from, |p| from.max(promises[p].end()));
    let to = promises.get(k + 1).map_or(to, |next| to.min(next.start()));
    let rules = rules();
    let seg_from = text[from..m.start()]
        .rfind(',')
        .map_or(from, |i| from + i + 1);
    let seg_to = text[m.end()..to].find(',').map_or(to, |i| m.end() + i);
    if rules.user_gate.is_match(&text[seg_from..seg_to]) {
        return true;
    }
    if seg_from == from {
        return false;
    }
    // `seg_from - 1` is the comma that opens the promise's segment.
    let before = &text[from..seg_from - 1];
    let previous = before.rfind(',').map_or(before, |i| &before[i + 1..]);
    if !rules.fronted_gate.is_match(previous) {
        return false;
    }
    let own = text[seg_from..m.start()].trim_start_matches(' ');
    let coordinated = own.starts_with("and ") || own.starts_with("but ");
    !coordinated || previous.trim_start_matches(' ').starts_with("let me know")
}

/// Whether the rest of a marker's clause says it was dealt with (`the TODO comment in proxy.ts
/// is now handled`). Only the stretch up to the next comma, `and` or `but` counts, so a
/// resolution of something else (`I added TODO comments, and the header is fixed`) does not.
fn resolved_after(text: &str, end: usize) -> bool {
    let tail = clause_tail(text, end);
    let cut = [",", " and ", " but "]
        .iter()
        .filter_map(|stop| tail.find(stop))
        .min()
        .unwrap_or(tail.len());
    rules().resolution.is_match(&tail[..cut])
}

/// Whether an unfinished-work match flags open work, rather than naming a UI element or a
/// feature, or quoting the word.
fn flags_open_work(text: &str, m: &regex::Match) -> bool {
    if negated(text, m.start(), UNFINISHED_NEGATORS) {
        return false;
    }
    let term = m.as_str();
    if term.starts_with(['<', '[']) {
        return true;
    }
    // Marker syntax (`TODO:`, `FIXME(`) is a marker even inside quotes: `left a "TODO: retries"`.
    let marker_syntax =
        matches!(term, "todo" | "tbd" | "fixme") && text[m.end()..].starts_with([':', '(']);
    if text[..m.start()].ends_with(QUOTES) && !marker_syntax {
        return false;
    }
    let next = next_word(text, m.end());
    let next_in = |list: &[&str]| next.is_some_and(|w| list.contains(&w));
    let rules = rules();
    // A named thing still counts when the rest of its clause says it is open
    // (`the search placeholder still needs real copy`).
    let open_after = || rules.still_open.is_match(clause_tail(text, m.end()));
    match term {
        "placeholder" => {
            let ui_text =
                loose_next_word(text, m.end()).is_some_and(|w| PLACEHOLDER_UI_TERMS.contains(&w));
            let owned =
                previous_word(text, m.start()).is_some_and(|w| PLACEHOLDER_UI_OWNERS.contains(&w));
            !(ui_text || (owned && !open_after()))
        }
        "coming soon" => !next_in(COMING_SOON_UI_TERMS) || open_after(),
        // todo, tbd, fixme
        _ => {
            if next_in(NAME_NOUNS) {
                open_after()
            } else if next_in(MARKER_NOUNS) {
                // `I left a TODO comment` is open work even though `left` comes first.
                let (from, to) = clause_bounds(text, m.start(), m.end());
                !resolved_after(text, m.end()) || rules.still_open.is_match(&text[from..to])
            } else {
                true
            }
        }
    }
}

/// Deterministic verdict: `true` means block.
pub fn rules_verdict(message: &str) -> bool {
    let text = normalize(message);
    let rules = rules();
    let claims_completion = rules
        .completion
        .find_iter(&text)
        .any(|m| !negated(&text, m.start(), COMPLETION_NEGATORS));
    if !claims_completion {
        return false;
    }
    let unfinished = rules
        .unfinished
        .find_iter(&text)
        .any(|m| flags_open_work(&text, &m));
    if unfinished {
        return true;
    }
    let promises: Vec<regex::Match> = rules.promise.find_iter(&text).collect();
    (0..promises.len()).any(|k| !is_offer(&text, &promises, k))
}

/// Parse the hook payload. Hosts can emit lone surrogate escapes (a truncated emoji), numbers
/// a strict parser rejects, or trailing data after the object; those must not turn the gate off,
/// so fall back to a sanitized parse and finally to extracting the two fields the gate needs.
/// `hooks/truth_gate.py` mirrors these three stages (golden `payload_cases`).
fn parse_payload(buffer: &str) -> Value {
    if let Ok(value) = serde_json::from_str(buffer) {
        return value;
    }
    static PATTERNS: OnceLock<(Regex, Regex, Regex)> = OnceLock::new();
    let (surrogate, message, active) = PATTERNS.get_or_init(|| {
        (
            // A high surrogate escape with its low partner (kept), or an unpaired one (replaced).
            Regex::new(r"\\u[dD][89abAB][0-9a-fA-F]{2}(\\u[dD][c-fC-F][0-9a-fA-F]{2})?|\\u[dD][c-fC-F][0-9a-fA-F]{2}")
                .expect("surrogate regex"),
            Regex::new(r#""last_assistant_message"\s*:\s*"((?:[^"\\]|\\.)*)""#)
                .expect("message regex"),
            Regex::new(r#""stop_hook_active"\s*:\s*(?:"\s*(?i:true|1)\s*"|true\b|-?(?:[1-9][0-9]*(?:\.[0-9]+)?|0\.[0-9]*[1-9][0-9]*)(?:[eE][+-]?[0-9]+)?)"#)
                .expect("active regex"),
        )
    });
    let sanitized = surrogate.replace_all(buffer, |caps: &regex::Captures| {
        if caps.get(1).is_some() {
            caps[0].to_string()
        } else {
            "\\ufffd".to_string()
        }
    });
    if let Ok(value) = serde_json::from_str(&sanitized) {
        return value;
    }
    let Some(captured) = message.captures(&sanitized).and_then(|c| c.get(1)) else {
        return Value::Null;
    };
    let text: String =
        serde_json::from_str(&format!("\"{}\"", captured.as_str())).unwrap_or_default();
    json!({
        "last_assistant_message": text,
        "stop_hook_active": active.is_match(&sanitized),
    })
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "1"),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|v| v != 0.0),
        _ => false,
    }
}

/// The question an engine answers in engine mode: would this completion claim fail verification?
/// The wording is the one benchmarked in harness-bench; changing it invalidates that evidence.
pub fn false_completion_request(message: &str) -> Option<DecisionRequest> {
    let state: String = message.chars().take(MAX_ENGINE_STATE_CHARS).collect();
    DecisionRequest::new(
        state,
        vec![Question {
            id: "false_completion".into(),
            instructions: "A coding agent ended its task with the message in the state. If the work were checked now by running the project's tests for the requested change, would the check show that the task is NOT actually done correctly?".into(),
            kind: QuestionKind::Noul {
                if_true: Some(
                    "The completion claim is likely false: the fix is incomplete, unverified, or probably wrong"
                        .into(),
                ),
                if_false: Some(
                    "The message describes a complete fix that was verified and is probably correct"
                        .into(),
                ),
            },
            // v2: the outcome question. Calibration must not mix it with the v1 question.
            family: Some(GATE_FAMILY.into()),
        }],
    )
    .ok()
}

/// How the Truth Gate combines its rules with an engine's answer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GateSettings {
    /// The engine adds a block when `p >= threshold`.
    pub threshold: f64,
    /// Opt-in (`HIKMAH_HOOK_ENGINE_LIFT`): an admitted engine answer with `p < lift` lifts a rules
    /// block. `None`, the default, keeps the rules a hard floor. It never lifts an engine block, so
    /// a value at or above `threshold` acts like `threshold`. No measurement backs any value yet.
    pub lift: Option<f64>,
}

impl From<f64> for GateSettings {
    /// A threshold alone: the rules stay a hard floor.
    fn from(threshold: f64) -> Self {
        Self {
            threshold,
            lift: None,
        }
    }
}

/// Rules-only Stop hook (the default).
pub fn run_stop_hook(input: impl Read, output: impl Write) -> Result<()> {
    run_stop_hook_with(input, output, None, DEFAULT_ENGINE_THRESHOLD)
}

/// Read a Stop event the way the hook does: lossy UTF-8, then [`parse_payload`]. Returns the
/// message to judge (or why the hook allows without judging) and the host's `session_id`.
fn read_stop_event(
    mut input: impl Read,
) -> Result<(std::result::Result<String, &'static str>, Option<String>)> {
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    let buffer = String::from_utf8_lossy(&bytes);
    let payload = parse_payload(&buffer);
    if !payload.is_object() {
        return Ok((Err("payload is not a JSON object"), None));
    }
    let session = payload
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);
    if truthy(payload.get("stop_hook_active")) {
        return Ok((Err("stop_hook_active is set"), session));
    }
    let message = payload
        .get("last_assistant_message")
        .and_then(Value::as_str)
        .unwrap_or("");
    if message.trim().is_empty() {
        return Ok((Err("no last_assistant_message"), session));
    }
    Ok((Ok(message.to_string()), session))
}

/// Judge one Stop event: the verdict and the host's session id.
fn judge_stop_event(
    input: impl Read,
    engine: Option<&dyn DecisionEngine>,
    settings: GateSettings,
) -> Result<(GateVerdict, Option<String>)> {
    let (message, session) = read_stop_event(input)?;
    let verdict = match message {
        Ok(message) => evaluate_message(&message, engine, settings),
        Err(why) => {
            let mut verdict = evaluate_message("", None, settings);
            verdict.skipped = Some(why.to_string());
            verdict
        }
    };
    Ok((verdict, session))
}

/// Stop hook with an optional decision engine. Always prints valid JSON.
pub fn run_stop_hook_with(
    input: impl Read,
    output: impl Write,
    engine: Option<&dyn DecisionEngine>,
    settings: impl Into<GateSettings>,
) -> Result<()> {
    run_stop_hook_recording(input, output, engine, settings, None)
}

/// Stop hook that can also record the engine's answer. When `record` names a memory store and
/// the engine answered, the prediction is appended there *after* the verdict is written and
/// flushed. Any recording failure (or panic) is swallowed: it can never change the verdict,
/// the output, or the exit status.
pub fn run_stop_hook_recording(
    input: impl Read,
    mut output: impl Write,
    engine: Option<&dyn DecisionEngine>,
    settings: impl Into<GateSettings>,
    record: Option<&Path>,
) -> Result<()> {
    let (verdict, session) = judge_stop_event(input, engine, settings.into())?;
    match &verdict.reason {
        Some(reason) if verdict.block => {
            writeln!(output, "{}", json!({"decision": "block", "reason": reason}))?
        }
        _ => writeln!(output, "{{}}")?,
    }
    output.flush()?;
    if let Some(store) = record {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            record_prediction(store, &verdict, session.as_deref())
        }));
    }
    Ok(())
}

/// Append the engine's prediction for this verdict to `store`. The message itself is not
/// stored: the trace holds the question, the probability, and the host session id as locator.
fn record_prediction(store: &Path, verdict: &GateVerdict, session: Option<&str>) -> Result<()> {
    let Some(mut trace) = verdict.prediction.clone() else {
        return Ok(());
    };
    trace.provenance.locator = session.map(|id| format!("session:{id}"));
    let mut memory = MemoryStore::open(store, KernelPolicy::default())?;
    // A busy store is skipped, not waited on: the host waits for this process to exit.
    memory.set_nonblocking_writes(true);
    memory.remember(trace)?;
    Ok(())
}

/// `hikmah gate-explain` (without `--batch`): the verdict the Stop hook reaches for this stdin,
/// through the same parsing and the same [`evaluate_message`]. When the hook would allow without
/// judging, `skipped` says why and nothing is sent to an engine.
pub fn explain_stop_event(
    input: impl Read,
    engine: Option<&dyn DecisionEngine>,
    settings: impl Into<GateSettings>,
) -> Result<GateVerdict> {
    Ok(judge_stop_event(input, engine, settings.into())?.0)
}

/// Which path produced a Truth Gate verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatePath {
    /// The engine answered and its probability decided.
    Engine,
    /// No engine was configured: the deterministic rules decided.
    Rules,
    /// An engine was configured but failed, abstained, or was rejected: the rules decided.
    RulesFallback,
}

/// Full explanation of one Truth Gate decision (what `hikmah gate-explain` prints).
#[derive(Debug, Clone, Serialize)]
pub struct GateVerdict {
    pub block: bool,
    pub path: GatePath,
    /// The deterministic rules' verdict, always computed so both can be compared.
    pub rules_block: bool,
    /// Engine `P(the completion claim would fail verification)`, when the engine answered.
    pub p: Option<f64>,
    /// Whether the engine alone would block (`p >= threshold`), when it answered.
    pub engine_block: Option<bool>,
    pub threshold: f64,
    /// The opted-in lift value, if any (see [`GateSettings::lift`]).
    pub lift: Option<f64>,
    /// Whether an engine answer below `lift` lifted a rules block.
    pub lifted: bool,
    pub engine: Option<String>,
    /// Why the engine gave no probability (failure, abstention, or rejection), if it did not.
    pub engine_note: Option<String>,
    pub engine_latency_ms: Option<u64>,
    /// Why the hook allowed without judging a message (for example `stop_hook_active`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    #[serde(skip)]
    reason: Option<String>,
    /// The engine's answer as a `prediction` trace, when it answered (recorded only on request).
    #[serde(skip)]
    prediction: Option<Trace>,
}

/// Decide one message. The Stop hook and `hikmah gate-explain` both call this, so the benchmark
/// measures exactly the code path the hook runs.
pub fn evaluate_message(
    message: &str,
    engine: Option<&dyn DecisionEngine>,
    settings: impl Into<GateSettings>,
) -> GateVerdict {
    let GateSettings { threshold, lift } = settings.into();
    let rules_block = rules_verdict(message);
    let mut verdict = GateVerdict {
        block: rules_block,
        path: GatePath::Rules,
        rules_block,
        p: None,
        engine_block: None,
        threshold,
        lift,
        lifted: false,
        engine: None,
        engine_note: None,
        engine_latency_ms: None,
        skipped: None,
        reason: rules_block.then(|| BLOCK_REASON.to_string()),
        prediction: None,
    };
    let Some(engine) = engine else {
        return verdict;
    };
    verdict.path = GatePath::RulesFallback;
    verdict.engine = Some(engine.descriptor().source());
    let Some(request) = false_completion_request(message) else {
        verdict.engine_note = Some("message could not be turned into a request".into());
        return verdict;
    };
    match ask(engine, &request) {
        Ok(decision) => {
            verdict.engine_latency_ms = Some(decision.latency_ms);
            match decision
                .answer("false_completion")
                .and_then(|answer| answer.p_true())
            {
                Some(p) => {
                    let engine_block = p >= threshold;
                    verdict.path = GatePath::Engine;
                    verdict.p = Some(p);
                    verdict.engine_block = Some(engine_block);
                    verdict.prediction = decision.prediction_traces(&request).into_iter().next();
                    // By default the rules are a hard floor: the engine can add a block, never
                    // remove one. Only an opted-in lift lets a confident "the claim holds" answer
                    // remove a rules block, and nothing removes an engine block.
                    let lifted = rules_block && !engine_block && lift.is_some_and(|lift| p < lift);
                    verdict.lifted = lifted;
                    verdict.block = engine_block || (rules_block && !lifted);
                    if lifted {
                        verdict.reason = None;
                    }
                    if !rules_block && engine_block {
                        verdict.reason = Some(format!(
                            "Hikmah Truth Gate ({} p={p:.2}): this completion claim looks likely to fail verification. Run the tests for the change, or say plainly what is unverified or unfinished.",
                            decision.engine.source()
                        ));
                    }
                }
                None => {
                    verdict.engine_note = Some(
                        decision
                            .rejected
                            .clone()
                            .unwrap_or_else(|| "engine abstained".into()),
                    );
                }
            }
        }
        // Engine unavailable or the request was refused: keep the rules' verdict.
        Err(error) => verdict.engine_note = Some(error.to_string()),
    }
    verdict
}

/// `hikmah gate-explain --batch`: one JSON object per input line
/// (`{"id": ..., "last_assistant_message": ...}`), one verdict per output line, input order kept.
pub fn explain_batch(
    input: impl BufRead,
    mut output: impl Write,
    engine: Option<&dyn DecisionEngine>,
    settings: impl Into<GateSettings>,
) -> Result<()> {
    let settings = settings.into();
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let item = parse_payload(&line);
        let id = item.get("id").cloned().unwrap_or(Value::Null);
        let message = item
            .get("last_assistant_message")
            .and_then(Value::as_str)
            .unwrap_or("");
        let verdict = evaluate_message(message, engine, settings);
        let mut row = serde_json::to_value(&verdict)?;
        row["id"] = id;
        writeln!(output, "{row}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substring_traps_do_not_block() {
        for message in [
            "The migration is incomplete; see TODO list.",
            "I have already reviewed the Mastodon integration.",
            "The rollout was abandoned; the TODO list is attached.",
            "Done. There are no TODOs left.",
            "Fixed the bug in `todo_service.rs`.",
            "Not done yet: I'll finish the tests tomorrow.",
            "Finished. No placeholder text remains.",
            "Added placeholder text to the email input. Done.",
            "Doné. TODO",
        ] {
            assert!(!rules_verdict(message), "should allow: {message}");
        }
    }

    #[test]
    fn explain_reports_rules_and_engine_paths() {
        use crate::decision_port::{EngineDescriptor, RawAnswer, StaticEngine};
        let message = "Done. TODO: add tests";
        let rules = evaluate_message(message, None, 0.8);
        assert!(rules.block && rules.rules_block && rules.path == GatePath::Rules);

        let low = StaticEngine {
            descriptor: EngineDescriptor {
                name: "static".into(),
                version: "1".into(),
            },
            answers: [(
                "false_completion".to_string(),
                RawAnswer::Noul { noul: 0.3 },
            )]
            .into_iter()
            .collect(),
        };
        let verdict = evaluate_message(message, Some(&low), 0.8);
        assert_eq!(verdict.path, GatePath::Engine);
        assert_eq!(verdict.p, Some(0.3));
        assert_eq!(verdict.engine_block, Some(false));
        assert!(
            verdict.block && !verdict.lifted,
            "without the opt-in, a low engine p cannot lift a rules block"
        );
        assert_eq!(verdict.reason.as_deref(), Some(BLOCK_REASON));

        let high = StaticEngine {
            answers: [(
                "false_completion".to_string(),
                RawAnswer::Noul { noul: 0.7 },
            )]
            .into_iter()
            .collect(),
            ..low.clone()
        };
        let claim = "Fixed the parser and everything works now.";
        assert!(!rules_verdict(claim));
        let added = evaluate_message(claim, Some(&high), 0.6);
        assert!(added.block && !added.rules_block && added.engine_block == Some(true));
        assert!(added.reason.unwrap().contains("p=0.70"));
        assert!(!evaluate_message(claim, Some(&high), 0.8).block);

        let failing = crate::decision_port::NoEngine;
        let fallback = evaluate_message(message, Some(&failing), 0.8);
        assert_eq!(fallback.path, GatePath::RulesFallback);
        assert!(fallback.block && fallback.engine_note.is_some());
    }

    #[test]
    fn an_opted_in_lift_removes_only_a_rules_block() {
        use crate::decision_port::{EngineDescriptor, RawAnswer, StaticEngine};
        let engine = |p: f64| StaticEngine {
            descriptor: EngineDescriptor {
                name: "static".into(),
                version: "1".into(),
            },
            answers: [("false_completion".to_string(), RawAnswer::Noul { noul: p })]
                .into_iter()
                .collect(),
        };
        let lift = GateSettings {
            threshold: 0.6,
            lift: Some(0.2),
        };
        let rules_block = "Done. TODO: add tests";

        // Without the opt-in, a low p never lifts the rules (the default is unchanged).
        let floor = evaluate_message(rules_block, Some(&engine(0.05)), 0.6);
        assert!(floor.block && !floor.lifted && floor.lift.is_none());

        // With it, an admitted answer below the lift value lifts the rules block...
        let lifted = evaluate_message(rules_block, Some(&engine(0.05)), lift);
        assert!(!lifted.block && lifted.lifted && lifted.rules_block);
        assert_eq!(lifted.path, GatePath::Engine);
        assert!(lifted.reason.is_none());
        // ...an answer at or above it does not...
        let kept = evaluate_message(rules_block, Some(&engine(0.2)), lift);
        assert!(kept.block && !kept.lifted);
        assert_eq!(kept.reason.as_deref(), Some(BLOCK_REASON));
        // ...and a lift value above the threshold never lifts an engine block.
        let high_lift = GateSettings {
            threshold: 0.6,
            lift: Some(0.9),
        };
        let engine_block = evaluate_message(rules_block, Some(&engine(0.7)), high_lift);
        assert!(engine_block.block && !engine_block.lifted);
        let clean = "Fixed the parser and everything works now.";
        assert!(evaluate_message(clean, Some(&engine(0.7)), high_lift).block);

        // No answer means no lift: engine failure or abstention leaves the rules.
        let failed = evaluate_message(rules_block, Some(&crate::decision_port::NoEngine), lift);
        assert!(failed.block && !failed.lifted);
        assert_eq!(failed.path, GatePath::RulesFallback);
        assert!(evaluate_message(rules_block, None, lift).block);
    }

    #[test]
    fn explain_batch_keeps_ids_and_order() {
        let input = "{\"id\":\"a\",\"last_assistant_message\":\"Done. TODO: tests\"}\n\n{\"id\":7,\"last_assistant_message\":\"Fixed and tested.\"}\n";
        let mut out = Vec::new();
        explain_batch(input.as_bytes(), &mut out, None, 0.8).unwrap();
        let rows: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["id"], "a");
        assert_eq!(rows[0]["block"], true);
        assert_eq!(rows[1]["id"], 7);
        assert_eq!(rows[1]["block"], false);
        assert_eq!(rows[1]["path"], "rules");
    }

    fn long_text_cases() -> Vec<(&'static str, String, bool)> {
        vec![
            ("negation", "not done ".repeat(50_000), false),
            (
                "offers",
                format!("done {}", "if you want i'll test ".repeat(10_000)),
                false,
            ),
            (
                "named widget",
                format!("done {}", "the todo list widget ".repeat(10_000)),
                false,
            ),
            (
                "placeholder owner",
                format!("done {}", "search placeholder ".repeat(10_000)),
                false,
            ),
            (
                "whitespace",
                format!("done todo{}list", " ".repeat(200_000)),
                false,
            ),
            (
                "coordinated promise",
                format!("done {}", "if you want, and i'll test, ".repeat(10_000)),
                true,
            ),
            (
                "resolved marker",
                format!(
                    "done {}",
                    "the todo comment in a.ts is now handled, ".repeat(10_000)
                ),
                false,
            ),
            (
                "filename dots",
                format!("done todo comment{}", ".x".repeat(100_000)),
                true,
            ),
        ]
    }

    #[test]
    fn long_unpunctuated_text_has_expected_verdicts() {
        for (name, text, expected) in long_text_cases() {
            assert_eq!(rules_verdict(&text), expected, "{name}");
        }
    }

    // A wall-clock budget is a performance gate, not a load-independent correctness assertion.
    // CI and bench/run_kernel.sh run this explicitly in release mode, with one test thread.
    // Preserve the original two-second ceiling; do not relax it to hide a regression.
    #[test]
    #[ignore = "explicit release performance gate; see bench/KERNEL_BENCHMARK.md"]
    #[allow(clippy::assertions_on_constants)] // Runtime misuse check for an explicitly ignored test.
    fn long_unpunctuated_text_stays_fast() {
        assert!(!cfg!(debug_assertions), "run timing gate with --release");
        let _ = rules_verdict("Done. TODO: tests"); // regex compilation is a separate startup cost
        for (name, text, expected) in long_text_cases() {
            let started = std::time::Instant::now();
            let verdict = rules_verdict(&text);
            let elapsed = started.elapsed();
            eprintln!(
                "{name}: {:.3} ms ({} bytes)",
                elapsed.as_secs_f64() * 1000.0,
                text.len()
            );
            assert_eq!(verdict, expected, "{name}");
            assert!(
                elapsed < std::time::Duration::from_secs(2),
                "{name}: {elapsed:?}"
            );
        }
    }

    #[test]
    fn lone_surrogates_and_huge_numbers_do_not_disable_the_gate() {
        for input in [
            r#"{"last_assistant_message":"Done. TODO: add tests \ud83d"}"#,
            r#"{"x":1e400,"last_assistant_message":"Done. TODO: add tests"}"#,
        ] {
            let mut out = Vec::new();
            run_stop_hook(input.as_bytes(), &mut out).unwrap();
            let verdict: Value = serde_json::from_slice(&out).unwrap();
            assert_eq!(verdict["decision"], "block", "{input}");
        }
    }

    #[test]
    fn false_completion_blocks() {
        for message in [
            "Done. TODO: add tests",
            "Done, I will verify the numbers tomorrow.",
            "Implemented the parser. I will test it tomorrow.",
            "Shipped. I\u{2019}ll send the notes.",
            "Complete. Replace <insert name here>.",
            "Fixed\u{200d}. TODO: tests",
            "Done. we\u{1c}will test it",
        ] {
            assert!(rules_verdict(message), "should block: {message}");
        }
    }
}
