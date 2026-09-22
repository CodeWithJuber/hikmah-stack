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
//! Optional engine mode: a typed decision engine (for example Jev) estimates the probability that
//! the completion claim would fail verification (a test run of the requested change). The gate
//! blocks when the rules block **or** `p >= threshold`: the deterministic rules are a hard floor an
//! engine cannot lift, and the engine can only add blocks. Any engine failure, abstention, or
//! rejected response leaves the rules' verdict in place.
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
            Regex::new(r#""stop_hook_active"\s*:\s*(?:"true"|"1"|(?:true|1)\b)"#)
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
    threshold: f64,
) -> Result<(GateVerdict, Option<String>)> {
    let (message, session) = read_stop_event(input)?;
    let verdict = match message {
        Ok(message) => evaluate_message(&message, engine, threshold),
        Err(why) => {
            let mut verdict = evaluate_message("", None, threshold);
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
    threshold: f64,
) -> Result<()> {
    run_stop_hook_recording(input, output, engine, threshold, None)
}

/// Stop hook that can also record the engine's answer. When `record` names a memory store and
/// the engine answered, the prediction is appended there *after* the verdict is written and
/// flushed. Any recording failure (or panic) is swallowed: it can never change the verdict,
/// the output, or the exit status.
pub fn run_stop_hook_recording(
    input: impl Read,
    mut output: impl Write,
    engine: Option<&dyn DecisionEngine>,
    threshold: f64,
    record: Option<&Path>,
) -> Result<()> {
    let (verdict, session) = judge_stop_event(input, engine, threshold)?;
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
    memory.remember(trace)?;
    Ok(())
}

/// `hikmah gate-explain` (without `--batch`): the verdict the Stop hook reaches for this stdin,
/// through the same parsing and the same [`evaluate_message`]. When the hook would allow without
/// judging, `skipped` says why and nothing is sent to an engine.
pub fn explain_stop_event(
    input: impl Read,
    engine: Option<&dyn DecisionEngine>,
    threshold: f64,
) -> Result<GateVerdict> {
    Ok(judge_stop_event(input, engine, threshold)?.0)
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
    threshold: f64,
) -> GateVerdict {
    let rules_block = rules_verdict(message);
    let mut verdict = GateVerdict {
        block: rules_block,
        path: GatePath::Rules,
        rules_block,
        p: None,
        engine_block: None,
        threshold,
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
                    // The rules are a hard floor: the engine can add a block, never remove one.
                    verdict.block = rules_block || engine_block;
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
    threshold: f64,
) -> Result<()> {
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
        let verdict = evaluate_message(message, engine, threshold);
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
        assert!(verdict.block, "a low engine p cannot lift a rules block");
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
