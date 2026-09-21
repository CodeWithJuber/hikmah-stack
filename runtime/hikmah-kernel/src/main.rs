use clap::{Parser, Subcommand};
use hikmah_kernel::council::{deliberate, DeliberationInput};
use hikmah_kernel::decision::{evaluate, DecisionFrame};
use hikmah_kernel::decision_port::{
    ask, DecisionEngine, DecisionRequest, NoEngine, Question, QuestionKind,
};
use hikmah_kernel::hook::{run_stop_hook_with, DEFAULT_ENGINE_THRESHOLD};
use hikmah_kernel::planner::{plan, PlanProblem};
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::recall::RecallQuery;
use hikmah_kernel::trace::{OutcomeRecord, PrivacyClass, Trace, TraceKind};
use hikmah_kernel::validate::validate_repo;
use hikmah_kernel::{KernelError, MemoryStore, Result};
use serde_json::json;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::str::FromStr;

const DEFAULT_STORE: &str = ".hikmah/memory.jsonl";

#[derive(Debug, Parser)]
#[command(
    name = "hikmah",
    version,
    about = "Hikmah deterministic co-model kernel"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create an empty memory store.
    Init {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
    },
    /// Append a trace.
    Remember {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        content: String,
        /// Who wrote this. Use `model:<engine>` for model output; model traces are never verified.
        #[arg(long, default_value = "unknown")]
        source: String,
        #[arg(long)]
        locator: Option<String>,
        #[arg(long, default_value_t = 0.5)]
        authority: f32,
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(long, default_value_t = 0.5)]
        salience: f32,
        #[arg(long, default_value_t = 0.5)]
        confidence: f32,
        #[arg(long, default_value = "private")]
        privacy: String,
        #[arg(long)]
        claim_key: Option<String>,
        #[arg(long)]
        claim_value: Option<String>,
        #[arg(long)]
        supersedes: Option<String>,
        /// Deadline for commitments: epoch milliseconds, `YYYY-MM-DD[THH:MM[:SS]]` (UTC), or `+<n>h` / `+<n>d`.
        #[arg(long)]
        deadline: Option<String>,
        #[arg(long)]
        verified: bool,
    },
    /// Contextual recall.
    Recall {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
        #[arg(long, default_value = "")]
        query: String,
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Restrict to these kinds (repeatable). Predictions are only returned when asked for.
        #[arg(long = "kind")]
        kinds: Vec<String>,
        #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u32).range(1..))]
        limit: u32,
    },
    /// Mark a commitment fulfilled.
    Fulfill {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
        #[arg(long)]
        id: String,
    },
    /// Tombstone a trace (it leaves recall; content stays in the append-only ledger).
    Purge {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        reason: String,
    },
    Commitments {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
        #[arg(long, default_value_t = 168)]
        within_hours: u64,
    },
    Consolidate {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
    },
    /// Verify the hash chain, the head file, and (optionally) a pinned head hash.
    VerifyLedger {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
        #[arg(long)]
        expect_head: Option<String>,
    },
    Plan {
        #[arg(long)]
        problem: PathBuf,
    },
    /// Rank a decision frame. With `--engine`, options that have a `description` get missing
    /// criteria estimated by the engine (ranked, but never counted as evidence).
    Decide {
        #[arg(long)]
        frame: PathBuf,
        #[arg(long, default_value = "none")]
        engine: String,
    },
    Deliberate {
        #[arg(long, default_value_t = 0)]
        unverified_claims: usize,
        #[arg(long, default_value_t = 0)]
        memory_conflicts: usize,
        #[arg(long, default_value_t = 0)]
        irreversible_actions: usize,
        #[arg(long, default_value_t = 0)]
        human_impact_questions: usize,
        #[arg(long, default_value_t = 0)]
        missing_acceptance_criteria: usize,
    },
    /// Ask a typed decision engine (`none` or `jev`) a request file; answers are admitted by the kernel.
    Ask {
        #[arg(long)]
        request: PathBuf,
        #[arg(long, default_value = "none")]
        engine: String,
        /// Record admitted answers as unverified `prediction` traces.
        #[arg(long)]
        record: bool,
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
    },
    /// Record the observed outcome of a prediction (from a non-model principal).
    Outcome {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
        #[arg(long)]
        prediction: String,
        /// `true`/`false` for noul, the option id for choice, the level index for score.
        #[arg(long)]
        observed: String,
        #[arg(long)]
        source: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Calibration of recorded predictions against outcomes.
    Calibration {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
        #[arg(long)]
        family: Option<String>,
    },
    /// Truth Gate Stop hook. Engine via HIKMAH_HOOK_ENGINE=jev (needs TYPESAFE_API_KEY);
    /// threshold via HIKMAH_HOOK_THRESHOLD (default 0.8).
    Hook,
    Validate {
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("hikmah: {error}");
        std::process::exit(1);
    }
}

fn policy() -> KernelPolicy {
    KernelPolicy::default()
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn select_engine(name: &str) -> Result<Box<dyn DecisionEngine>> {
    match name.trim().to_ascii_lowercase().as_str() {
        "" | "none" => Ok(Box::new(NoEngine)),
        "jev" => jev_engine(None),
        other => Err(KernelError::Invalid(format!(
            "unknown engine `{other}` (expected none or jev)"
        ))),
    }
}

#[cfg(feature = "jev")]
fn jev_engine(timeout_ms: Option<u64>) -> Result<Box<dyn DecisionEngine>> {
    let mut engine = hikmah_kernel::jev::JevEngine::from_env()
        .ok_or_else(|| KernelError::Invalid("TYPESAFE_API_KEY is not set".into()))?;
    if let Some(ms) = timeout_ms {
        if std::env::var("HIKMAH_JEV_TIMEOUT_MS").is_err() {
            engine = engine.with_timeout(std::time::Duration::from_millis(ms));
        }
    }
    Ok(Box::new(engine))
}

#[cfg(not(feature = "jev"))]
fn jev_engine(_timeout_ms: Option<u64>) -> Result<Box<dyn DecisionEngine>> {
    Err(KernelError::Invalid(
        "this hikmah binary was built without the `jev` feature".into(),
    ))
}

/// Parse a deadline: epoch ms, `+<n>h`, `+<n>d`, or `YYYY-MM-DD[THH:MM[:SS]][Z]` in UTC.
fn parse_deadline(value: &str, now_ms: u64) -> Result<u64> {
    let v = value.trim();
    let invalid = || KernelError::Invalid(format!("unrecognized deadline: {value}"));
    if let Some(rest) = v.strip_prefix('+') {
        let (number, unit) = rest.split_at(rest.len().saturating_sub(1));
        let n: u64 = number.parse().map_err(|_| invalid())?;
        let ms = match unit {
            "h" => n.saturating_mul(3_600_000),
            "d" => n.saturating_mul(86_400_000),
            _ => return Err(invalid()),
        };
        return Ok(now_ms.saturating_add(ms));
    }
    if v.chars().all(|c| c.is_ascii_digit()) {
        return v.parse().map_err(|_| invalid());
    }
    let v = v.trim_end_matches('Z');
    let (date, time) = v.split_once('T').unwrap_or((v, "00:00:00"));
    let d: Vec<i64> = date
        .split('-')
        .map(|p| p.parse().map_err(|_| invalid()))
        .collect::<Result<_>>()?;
    let t: Vec<i64> = time
        .split(':')
        .map(|p| p.parse().map_err(|_| invalid()))
        .collect::<Result<_>>()?;
    if d.len() != 3 || !(1..=3).contains(&t.len()) {
        return Err(invalid());
    }
    let (y, m, day) = (d[0], d[1], d[2]);
    let (hh, mm, ss) = (t[0], *t.get(1).unwrap_or(&0), *t.get(2).unwrap_or(&0));
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || hh > 23 || mm > 59 || ss > 60 {
        return Err(invalid());
    }
    // Days from civil (Howard Hinnant's algorithm), UTC.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hh * 3_600 + mm * 60 + ss;
    if secs < 0 {
        return Err(invalid());
    }
    Ok(secs as u64 * 1_000)
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { store } => {
            let memory = MemoryStore::open(&store, policy())?;
            print_json(&json!({"store": store, "records": memory.record_count()}))?;
        }
        Command::Remember {
            store,
            kind,
            content,
            source,
            locator,
            authority,
            tags,
            salience,
            confidence,
            privacy,
            claim_key,
            claim_value,
            supersedes,
            deadline,
            verified,
        } => {
            let mut memory = MemoryStore::open(store, policy())?;
            let mut trace = Trace::new(TraceKind::from_str(&kind)?, content, source);
            if trace.kind == TraceKind::Prediction {
                return Err(KernelError::Invalid(
                    "predictions are recorded through `hikmah ask --record`".into(),
                ));
            }
            trace.tags = tags;
            trace.salience = salience;
            trace.confidence = confidence;
            trace.privacy = PrivacyClass::from_str(&privacy)?;
            trace.claim_key = claim_key;
            trace.claim_value = claim_value;
            trace.supersedes = supersedes;
            trace.provenance.verified = verified;
            trace.provenance.authority = authority;
            trace.provenance.locator = locator;
            if let Some(deadline) = deadline {
                trace.deadline_ms = Some(parse_deadline(&deadline, trace.created_at_ms)?);
            }
            let (trace, conflicts) = memory.remember(trace)?;
            print_json(&json!({"trace": trace, "conflicts": conflicts}))?;
        }
        Command::Recall {
            store,
            query,
            tags,
            kinds,
            limit,
        } => {
            let memory = MemoryStore::open_existing(store, policy())?;
            let mut recall = RecallQuery::new(query);
            recall.tags = tags;
            recall.kinds = kinds
                .iter()
                .map(|k| TraceKind::from_str(k))
                .collect::<Result<_>>()?;
            recall.limit = limit as usize;
            print_json(&memory.recall(&recall))?;
        }
        Command::Fulfill { store, id } => {
            let mut memory = MemoryStore::open_existing(store, policy())?;
            memory.fulfill(&id)?;
            print_json(&json!({"fulfilled": id}))?;
        }
        Command::Purge { store, id, reason } => {
            let mut memory = MemoryStore::open_existing(store, policy())?;
            memory.purge(&id, reason)?;
            print_json(
                &json!({"purged": id, "note": "tombstoned; content remains in the append-only ledger"}),
            )?;
        }
        Command::Commitments {
            store,
            within_hours,
        } => {
            let memory = MemoryStore::open_existing(store, policy())?;
            let now = hikmah_kernel::trace::now_ms();
            let within_ms = within_hours.saturating_mul(3_600_000);
            print_json(&memory.commitments_due(now, within_ms))?;
        }
        Command::Consolidate { store } => {
            let memory = MemoryStore::open_existing(store, policy())?;
            print_json(&memory.consolidation_proposals())?;
        }
        Command::VerifyLedger { store, expect_head } => {
            let memory = MemoryStore::open_existing(store, policy())?;
            let report = memory.verify_report(expect_head.as_deref())?;
            print_json(&report)?;
            if !report.ok {
                std::process::exit(1);
            }
        }
        Command::Plan { problem } => {
            let text = fs::read_to_string(problem)?;
            let problem: PlanProblem = serde_json::from_str(&text)?;
            print_json(&plan(&problem)?)?;
        }
        Command::Decide { frame, engine } => {
            let text = fs::read_to_string(frame)?;
            let mut frame: DecisionFrame = serde_json::from_str(&text)?;
            if engine.trim().eq_ignore_ascii_case("none") {
                print_json(&evaluate(&frame)?)?;
            } else {
                let engine = select_engine(&engine)?;
                let estimates = estimate_missing_criteria(engine.as_ref(), &mut frame)?;
                let result = evaluate(&frame)?;
                print_json(&json!({"decision": result, "model_estimates": estimates}))?;
            }
        }
        Command::Deliberate {
            unverified_claims,
            memory_conflicts,
            irreversible_actions,
            human_impact_questions,
            missing_acceptance_criteria,
        } => {
            let result = deliberate(&DeliberationInput {
                unverified_consequential_claims: unverified_claims,
                memory_conflicts,
                irreversible_actions,
                unresolved_human_impact_questions: human_impact_questions,
                missing_acceptance_criteria,
            });
            print_json(&result)?;
        }
        Command::Ask {
            request,
            engine,
            record,
            store,
        } => {
            let text = fs::read_to_string(request)?;
            let request: DecisionRequest = serde_json::from_str(&text)?;
            let engine = select_engine(&engine)?;
            let decision = ask(engine.as_ref(), &request)?;
            let mut recorded = Vec::new();
            if record {
                let mut memory = MemoryStore::open(store, policy())?;
                for trace in decision.prediction_traces(&request) {
                    let (trace, _) = memory.remember(trace)?;
                    recorded.push(trace.id);
                }
            }
            print_json(&json!({"decision": decision, "recorded_predictions": recorded}))?;
        }
        Command::Outcome {
            store,
            prediction,
            observed,
            source,
            note,
        } => {
            let mut memory = MemoryStore::open_existing(store, policy())?;
            let content = note.unwrap_or_else(|| format!("Outcome for {prediction}: {observed}"));
            let mut trace = Trace::new(TraceKind::Outcome, content, source);
            trace.outcome = Some(OutcomeRecord {
                prediction_id: prediction,
                observed,
            });
            let (trace, _) = memory.remember(trace)?;
            print_json(&trace)?;
        }
        Command::Calibration { store, family } => {
            let memory = MemoryStore::open_existing(store, policy())?;
            print_json(&memory.calibration(family.as_deref()))?;
        }
        Command::Hook => {
            let engine_name = std::env::var("HIKMAH_HOOK_ENGINE").unwrap_or_default();
            let threshold = std::env::var("HIKMAH_HOOK_THRESHOLD")
                .ok()
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|v| (0.0..=1.0).contains(v))
                .unwrap_or(DEFAULT_ENGINE_THRESHOLD);
            // The hook must never fail closed on configuration problems: fall back to rules.
            let engine: Option<Box<dyn DecisionEngine>> =
                if engine_name.trim().eq_ignore_ascii_case("jev") {
                    jev_engine(Some(3_000)).ok()
                } else {
                    None
                };
            run_stop_hook_with(
                io::stdin().lock(),
                io::stdout().lock(),
                engine.as_deref(),
                threshold,
            )?;
        }
        Command::Validate { root } => {
            let notes = validate_repo(root)?;
            print_json(&json!({"ok": true, "checks": notes}))?;
        }
    }
    Ok(())
}

const SCORE_LEVELS: [&str; 5] = ["very poor", "poor", "fair", "good", "excellent"];

/// Ask the engine to estimate criteria an option has no evidence for, from its description.
fn estimate_missing_criteria(
    engine: &dyn DecisionEngine,
    frame: &mut DecisionFrame,
) -> Result<Vec<serde_json::Value>> {
    let mut estimates = Vec::new();
    let criteria = frame.criteria.clone();
    let question_text = frame.question.clone();
    for option in &mut frame.options {
        let Some(description) = option.description.clone() else {
            continue;
        };
        let missing: Vec<(usize, &hikmah_kernel::decision::Criterion)> = criteria
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                !option.scores.contains_key(&c.id) && !option.model_scores.contains_key(&c.id)
            })
            .collect();
        if missing.is_empty() {
            continue;
        }
        let questions = missing
            .iter()
            .map(|(index, criterion)| Question {
                id: format!("c{index}"),
                instructions: format!(
                    "Rate this option on: {} ({})",
                    criterion.description, criterion.id
                ),
                kind: QuestionKind::Score {
                    levels: SCORE_LEVELS.iter().map(|s| s.to_string()).collect(),
                },
                family: Some(format!("decide.{}", criterion.id)),
            })
            .collect();
        let request = DecisionRequest::new(
            format!(
                "Decision: {question_text}\nOption: {}\n{description}",
                option.name
            ),
            questions,
        )?;
        let decision = ask(engine, &request)?;
        for (index, criterion) in missing {
            let answer = decision.answer(&format!("c{index}"));
            match answer.and_then(|a| a.normalized_score()) {
                Some(score) => {
                    option.model_scores.insert(criterion.id.clone(), score);
                    estimates.push(json!({
                        "option": option.name,
                        "criterion": criterion.id,
                        "score": score,
                        "engine": decision.engine.source(),
                        "calibrated": false,
                    }));
                }
                None => estimates.push(json!({
                    "option": option.name,
                    "criterion": criterion.id,
                    "score": null,
                    "reason": decision.rejected.clone().unwrap_or_else(|| "abstained".into()),
                })),
            }
        }
    }
    Ok(estimates)
}
