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
//!   marker or a first-person future-work promise.
//!
//! Optional engine mode: a typed decision engine (for example Jev) answers one noul question;
//! the gate blocks when `P(false completion) >= threshold`. Any engine failure, abstention, or
//! rejected response falls back to the deterministic rules. The threshold is a configuration
//! choice, not a calibrated value; measure it with `hikmah calibration` before relying on it.
//!
//! `hooks/truth_gate_cases.json` holds golden cases shared with the Python fallback.
use crate::decision_port::{ask, DecisionEngine, DecisionRequest, Question, QuestionKind};
use crate::error::Result;
use regex::Regex;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

pub const DEFAULT_ENGINE_THRESHOLD: f64 = 0.8;
const MAX_ENGINE_STATE_CHARS: usize = 8_000;
/// How far back (in characters) negation is looked for, which keeps the check linear.
const NEGATION_WINDOW_CHARS: usize = 200;

const BLOCK_REASON: &str = "Hikmah Truth Gate: the response claims completion while still containing unfinished work or a future-work promise. Resolve it or state the limitation explicitly.";

struct Rules {
    fence: Regex,
    inline_code: Regex,
    completion: Regex,
    unfinished: Regex,
    promise: Regex,
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
const PLACEHOLDER_UI_TERMS: &[&str] = &["text", "attribute", "prop", "image", "color", "value"];

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

fn next_word(text: &str, start: usize) -> Option<&str> {
    text[start..]
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find(|w| !w.is_empty())
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
    let unfinished = rules.unfinished.find_iter(&text).any(|m| {
        if negated(&text, m.start(), UNFINISHED_NEGATORS) {
            return false;
        }
        if m.as_str() == "placeholder" {
            if let Some(next) = next_word(&text, m.end()) {
                if PLACEHOLDER_UI_TERMS.contains(&next) {
                    return false;
                }
            }
        }
        true
    });
    unfinished || rules.promise.is_match(&text)
}

/// Parse the hook payload. Hosts can emit lone surrogate escapes (a truncated emoji) or numbers
/// a strict parser rejects; those must not turn the gate off, so fall back to a sanitized parse
/// and finally to extracting the two fields the gate needs.
fn parse_payload(buffer: &str) -> Value {
    if let Ok(value) = serde_json::from_str(buffer) {
        return value;
    }
    static PATTERNS: OnceLock<(Regex, Regex, Regex)> = OnceLock::new();
    let (surrogate, message, active) = PATTERNS.get_or_init(|| {
        (
            Regex::new(r"\\u[dD][89abAB][0-9a-fA-F]{2}").expect("surrogate regex"),
            Regex::new(r#""last_assistant_message"\s*:\s*"((?:[^"\\]|\\.)*)""#)
                .expect("message regex"),
            Regex::new(r#""stop_hook_active"\s*:\s*(?:true|"true"|"1"|1)\b"#)
                .expect("active regex"),
        )
    });
    let sanitized = surrogate.replace_all(buffer, "\\ufffd");
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

/// The question an engine answers in engine mode.
pub fn false_completion_request(message: &str) -> Option<DecisionRequest> {
    let state: String = message.chars().take(MAX_ENGINE_STATE_CHARS).collect();
    DecisionRequest::new(
        state,
        vec![Question {
            id: "false_completion".into(),
            instructions: "Does this message claim the work is complete while also admitting unfinished work or promising to do remaining work later?".into(),
            kind: QuestionKind::Noul {
                if_true: Some("It claims completion but leaves work unfinished or deferred".into()),
                if_false: Some(
                    "It either honestly says the work is not done, or it is done with nothing deferred"
                        .into(),
                ),
            },
            family: Some("truth_gate.false_completion".into()),
        }],
    )
    .ok()
}

/// Rules-only Stop hook (the default).
pub fn run_stop_hook(input: impl Read, output: impl Write) -> Result<()> {
    run_stop_hook_with(input, output, None, DEFAULT_ENGINE_THRESHOLD)
}

/// Stop hook with an optional decision engine. Always prints valid JSON.
pub fn run_stop_hook_with(
    mut input: impl Read,
    mut output: impl Write,
    engine: Option<&dyn DecisionEngine>,
    threshold: f64,
) -> Result<()> {
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    let buffer = String::from_utf8_lossy(&bytes);
    let payload = parse_payload(&buffer);
    let allow = |output: &mut dyn Write| writeln!(output, "{{}}");
    if !payload.is_object() || truthy(payload.get("stop_hook_active")) {
        allow(&mut output)?;
        return Ok(());
    }
    let message = payload
        .get("last_assistant_message")
        .and_then(Value::as_str)
        .unwrap_or("");
    if message.trim().is_empty() {
        allow(&mut output)?;
        return Ok(());
    }

    if let Some(engine) = engine {
        if let Some(request) = false_completion_request(message) {
            if let Ok(decision) = ask(engine, &request) {
                if let Some(p) = decision
                    .answer("false_completion")
                    .and_then(|answer| answer.p_true())
                {
                    if p >= threshold {
                        let reason = format!(
                            "Hikmah Truth Gate ({} p={p:.2}): the response claims completion while leaving work unfinished or deferred. Resolve it or state the limitation explicitly.",
                            decision.engine.source()
                        );
                        writeln!(output, "{}", json!({"decision": "block", "reason": reason}))?;
                    } else {
                        allow(&mut output)?;
                    }
                    return Ok(());
                }
            }
        }
        // Engine unavailable, abstained, or rejected: fall through to the deterministic rules.
    }

    if rules_verdict(message) {
        writeln!(
            output,
            "{}",
            json!({"decision": "block", "reason": BLOCK_REASON})
        )?;
    } else {
        allow(&mut output)?;
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
    fn long_unpunctuated_text_stays_fast() {
        let text = "not done ".repeat(50_000);
        let started = std::time::Instant::now();
        let _ = rules_verdict(&text);
        assert!(started.elapsed().as_secs() < 2);
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
