use clap::{Parser, Subcommand};
use hikmah_kernel::council::{deliberate, DeliberationInput};
use hikmah_kernel::decision::{evaluate, DecisionFrame};
use hikmah_kernel::decision_port::{
    ask, DecisionEngine, DecisionRequest, NoEngine, Question, QuestionKind,
};
use hikmah_kernel::hook::{
    evaluate_message, explain_batch, run_stop_hook_with, DEFAULT_ENGINE_THRESHOLD,
};
use hikmah_kernel::planner::{plan, PlanProblem};
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::recall::RecallQuery;
use hikmah_kernel::trace::{parse_deadline, OutcomeRecord, PrivacyClass, Trace, TraceKind};
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
        /// Accept the current ledger as the new head after a deliberate repair (writes are
        /// refused while the ledger and its head file disagree).
        #[arg(long)]
        reset_head: bool,
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
    /// threshold via HIKMAH_HOOK_THRESHOLD (default 0.6, measured in harness-bench).
    Hook,
    /// Explain the Truth Gate decision (rules verdict, engine probability, path) for a stop event
    /// on stdin, or for JSON lines with `--batch`. Same engine settings and code path as `hook`.
    GateExplain {
        #[arg(long)]
        batch: bool,
    },
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
    if let Some(cap_ms) = timeout_ms {
        // The hook must finish well inside the host's timeout, whatever the environment says.
        let capped = engine
            .timeout()
            .min(std::time::Duration::from_millis(cap_ms));
        engine = engine.with_timeout(capped);
    }
    Ok(Box::new(engine))
}

/// Engine and threshold for `hook` and `gate-explain`. Configuration problems never fail closed:
/// an unusable engine setting means rules only.
fn hook_settings() -> (Option<Box<dyn DecisionEngine>>, f64) {
    let engine_name = std::env::var("HIKMAH_HOOK_ENGINE").unwrap_or_default();
    let threshold = std::env::var("HIKMAH_HOOK_THRESHOLD")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| (0.0..=1.0).contains(v))
        .unwrap_or(DEFAULT_ENGINE_THRESHOLD);
    let engine = if engine_name.trim().eq_ignore_ascii_case("jev") {
        jev_engine(Some(3_000)).ok()
    } else {
        None
    };
    (engine, threshold)
}

#[cfg(not(feature = "jev"))]
fn jev_engine(_timeout_ms: Option<u64>) -> Result<Box<dyn DecisionEngine>> {
    Err(KernelError::Invalid(
        "this hikmah binary was built without the `jev` feature".into(),
    ))
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
        Command::VerifyLedger {
            store,
            expect_head,
            reset_head,
        } => {
            let mut memory = MemoryStore::open_existing(store, policy())?;
            if reset_head {
                let head = memory.reset_head()?;
                print_json(&json!({"head_reset": head}))?;
                return Ok(());
            }
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
            let (engine, threshold) = hook_settings();
            run_stop_hook_with(
                io::stdin().lock(),
                io::stdout().lock(),
                engine.as_deref(),
                threshold,
            )?;
        }
        Command::GateExplain { batch } => {
            let (engine, threshold) = hook_settings();
            if batch {
                explain_batch(
                    io::stdin().lock(),
                    io::stdout().lock(),
                    engine.as_deref(),
                    threshold,
                )?;
            } else {
                let mut buffer = String::new();
                io::Read::read_to_string(&mut io::stdin().lock(), &mut buffer)?;
                let payload: serde_json::Value =
                    serde_json::from_str(&buffer).unwrap_or(serde_json::Value::Null);
                let message = payload
                    .get("last_assistant_message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                print_json(&evaluate_message(message, engine.as_deref(), threshold))?;
            }
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
