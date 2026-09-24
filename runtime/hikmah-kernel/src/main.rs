use clap::{Parser, Subcommand};
use hikmah_kernel::calibration::FamilyFilter;
use hikmah_kernel::council::{deliberate, DeliberationInput};
use hikmah_kernel::decision::{estimate_missing_criteria, evaluate, DecisionFrame};
use hikmah_kernel::decision_port::{ask, DecisionEngine, DecisionRequest, Forecast, NoEngine};
use hikmah_kernel::hook::{
    explain_batch, explain_stop_event, run_stop_hook_recording, DEFAULT_ENGINE_THRESHOLD,
};
use hikmah_kernel::planner::{plan, PlanProblem};
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::principal;
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
    /// Kernel policy JSON (limits, thresholds, recall weights). Missing fields keep their
    /// defaults; unknown fields are errors. Falls back to `HIKMAH_POLICY`, then the built-in
    /// defaults (`hikmah policy --print-defaults`).
    #[arg(long, global = true, value_name = "PATH")]
    policy: Option<PathBuf>,
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
        /// Who wrote this, as claimed by the caller (not authenticated). Use `model:<engine>` for
        /// model output; model traces are never verified. Inside a detected AI agent session the
        /// locator records `agent-session:<host>:<id>`.
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
        /// A person checked this claim. Refused inside a detected AI agent session (`CLAUDECODE`,
        /// `CLAUDE_CODE_*`, `CODEX_*`, `CURSOR_*`, `GEMINI_CLI`, `AI_AGENT`): run it from your own
        /// terminal.
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
        /// Also recall superseded traces (history); each shows the trace that replaced it.
        #[arg(long)]
        include_superseded: bool,
    },
    /// List unresolved conflicts: active traces whose claims share a key but disagree on the value.
    Conflicts {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
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
    /// Verify the hash chain, the head file, and (optionally) a pinned head hash. Records past
    /// the head that no hikmah write acknowledged are listed under `unacknowledged`.
    VerifyLedger {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
        /// Fail unless the latest record hash equals this value (kept somewhere the writer cannot
        /// reach, such as CI or git). Only checked by a plain verification, so it cannot be
        /// combined with `--accept-tail` or `--reset-head`.
        #[arg(long)]
        expect_head: Option<String>,
        /// Accept the current ledger as the new head after a deliberate repair (writes are
        /// refused while the ledger and its head file disagree). A person's decision: refused
        /// inside a detected AI agent session.
        #[arg(long, conflicts_with_all = ["accept_tail", "expect_head"])]
        reset_head: bool,
        /// Acknowledge records appended after the head file, after inspecting them: prints them
        /// and moves the head to the end. Refuses if earlier records were removed or rewritten.
        /// A person's decision: refused inside a detected AI agent session.
        #[arg(long, conflicts_with = "expect_head")]
        accept_tail: bool,
    },
    Plan {
        #[arg(long)]
        problem: PathBuf,
    },
    /// Rank a decision frame. With `--engine`, options that have a `description` and no hard
    /// block get missing criteria estimated by the engine in one request (ranked, but never
    /// counted as evidence). A rejected or timed-out request leaves every estimate in it
    /// unscored; `HIKMAH_JEV_TIMEOUT_MS` sets the engine's time budget per request.
    Decide {
        #[arg(long)]
        frame: PathBuf,
        #[arg(long, default_value = "none")]
        engine: String,
        /// Record every admitted estimate as an unverified `prediction` trace (family
        /// `decide.<criterion>`), in one batch. Needs an engine.
        #[arg(long)]
        record: bool,
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
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
    /// Record a person's or agent's forecast as a `prediction` trace, so `calibration` scores it
    /// beside any engine on the same family. It is never verified; `outcome` resolves it. Inside a
    /// detected AI agent session the locator records `agent-session:<host>:<id>`.
    Predict {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
        /// Calibration bucket, for example `site.hero.ctr` or `decide.perf`.
        #[arg(long)]
        family: String,
        /// What is being forecast, in words.
        #[arg(long)]
        question: String,
        /// `noul` (yes/no), `choice`, or `score`.
        #[arg(long = "type", value_name = "noul|choice|score")]
        kind: String,
        /// Noul: probability of `true`. Choice and score: probability of `--value`.
        #[arg(long)]
        p: f64,
        /// Choice and score: the forecast answer, one of `--answer-space`.
        #[arg(long)]
        value: Option<String>,
        /// Choice: the option ids. Score: the levels, lowest first (use `0,1,2,3,4` to share a
        /// family with engine score questions, which record level indices). Comma-separated.
        #[arg(long, value_delimiter = ',')]
        answer_space: Vec<String>,
        /// Who forecast, as `<kind>:<name>`: for example `human:alex` or `agent:planner`. Not a
        /// `model:` source.
        #[arg(long)]
        source: String,
        /// Where the forecast was made, for example `DECISIONS.md#hero`.
        #[arg(long)]
        locator: Option<String>,
    },
    /// Record the observed outcome of a prediction (from a non-model principal). Inside a
    /// detected AI agent session the locator records `agent-session:<host>:<id>`.
    Outcome {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
        #[arg(long)]
        prediction: String,
        /// One of the prediction's answer space: `true`/`false` for noul, the option id for
        /// choice, and for score the level as recorded (a level index for engine answers, the
        /// level name for a forecast recorded with named levels).
        #[arg(long)]
        observed: String,
        #[arg(long)]
        source: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Calibration of recorded predictions against outcomes, per forecaster and family. Rows
    /// below the policy's `calibration_min_outcomes` are labelled `anecdotal`.
    Calibration {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
        #[arg(long, conflicts_with = "family_prefix")]
        family: Option<String>,
        /// Every family starting with this prefix, plus one pooled row per forecaster.
        #[arg(long)]
        family_prefix: Option<String>,
    },
    /// Truth Gate Stop hook. Engine via HIKMAH_HOOK_ENGINE=jev (needs TYPESAFE_API_KEY);
    /// threshold via HIKMAH_HOOK_THRESHOLD (default 0.6, measured in harness-bench).
    /// HIKMAH_HOOK_RECORD=<store> appends each engine probability there as a prediction.
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
    /// Choose the Truth Gate engine threshold from recorded predictions and outcomes: the
    /// threshold with the highest recall whose false-block rate stays within the budget.
    GateThreshold {
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
        /// Largest acceptable false-block rate (blocked true completions / all true completions).
        #[arg(long, default_value_t = 0.10)]
        max_false_block: f64,
    },
    /// Print the effective kernel policy (after `--policy` / `HIKMAH_POLICY`), or the defaults.
    Policy {
        #[arg(long)]
        print_defaults: bool,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("hikmah: {error}");
        std::process::exit(1);
    }
}

/// The policy file named by `--policy`, else by a non-empty `HIKMAH_POLICY`.
fn policy_path(flag: Option<PathBuf>) -> Option<PathBuf> {
    flag.or_else(|| {
        std::env::var_os("HIKMAH_POLICY")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    })
}

/// Loaded only by commands that use it, so a bad policy file can never break `hook`.
fn load_policy(path: Option<&std::path::Path>) -> Result<KernelPolicy> {
    match path {
        Some(path) => KernelPolicy::from_json_file(path),
        None => Ok(KernelPolicy::default()),
    }
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
    let policy_file = policy_path(cli.policy);
    let policy = || load_policy(policy_file.as_deref());
    match cli.command {
        Command::Init { store } => {
            let memory = MemoryStore::open(&store, policy()?)?;
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
            let mut trace = Trace::new(TraceKind::from_str(&kind)?, content, source);
            if trace.kind == TraceKind::Prediction {
                return Err(KernelError::Invalid(
                    "predictions are recorded with `hikmah predict` (people and agents) or `hikmah ask --record` / `hikmah decide --record` (engines)".into(),
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
            // Before the store is opened, so a refused write creates nothing.
            if let Some(agent) = principal::detect_from_env() {
                agent.stamp(&mut trace)?;
            }
            let mut memory = MemoryStore::open(store, policy()?)?;
            let (trace, conflicts) = memory.remember(trace)?;
            print_json(&json!({"trace": trace, "conflicts": conflicts}))?;
        }
        Command::Recall {
            store,
            query,
            tags,
            kinds,
            limit,
            include_superseded,
        } => {
            let memory = MemoryStore::open_existing(store, policy()?)?;
            let mut recall = RecallQuery::new(query);
            recall.include_superseded = include_superseded;
            recall.tags = tags;
            recall.kinds = kinds
                .iter()
                .map(|k| TraceKind::from_str(k))
                .collect::<Result<_>>()?;
            recall.limit = limit as usize;
            print_json(&memory.recall(&recall))?;
        }
        Command::Conflicts { store } => {
            let memory = MemoryStore::open_existing(store, policy()?)?;
            print_json(&memory.active_conflicts())?;
        }
        Command::Fulfill { store, id } => {
            let mut memory = MemoryStore::open_existing(store, policy()?)?;
            memory.fulfill(&id)?;
            print_json(&json!({"fulfilled": id}))?;
        }
        Command::Purge { store, id, reason } => {
            let mut memory = MemoryStore::open_existing(store, policy()?)?;
            memory.purge(&id, reason)?;
            print_json(
                &json!({"purged": id, "note": "tombstoned; content remains in the append-only ledger"}),
            )?;
        }
        Command::Commitments {
            store,
            within_hours,
        } => {
            let memory = MemoryStore::open_existing(store, policy()?)?;
            let now = hikmah_kernel::trace::now_ms();
            let within_ms = within_hours.saturating_mul(3_600_000);
            print_json(&memory.commitments_due(now, within_ms))?;
        }
        Command::Consolidate { store } => {
            let memory = MemoryStore::open_existing(store, policy()?)?;
            print_json(&memory.consolidation_proposals())?;
        }
        Command::VerifyLedger {
            store,
            expect_head,
            reset_head,
            accept_tail,
        } => {
            // Before the store is opened, so a refused acceptance touches nothing. Both flags
            // accept records no hikmah write acknowledged, which may be a forged append.
            if accept_tail || reset_head {
                if let Some(agent) = principal::detect_from_env() {
                    let (command, why) = if accept_tail {
                        (
                            "hikmah verify-ledger --accept-tail",
                            "it approves ledger records that no hikmah write acknowledged, and such a record may be a forged append",
                        )
                    } else {
                        (
                            "hikmah verify-ledger --reset-head",
                            "it accepts the ledger as it is now, including records that were removed, rewritten, or appended without a hikmah write",
                        )
                    };
                    return Err(agent.refuse_person_only(command, why));
                }
            }
            let mut memory = MemoryStore::open_existing(store, policy()?)?;
            if accept_tail {
                print_json(&memory.accept_tail()?)?;
                return Ok(());
            }
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
        Command::Decide {
            frame,
            engine,
            record,
            store,
        } => {
            let text = fs::read_to_string(frame)?;
            let mut frame: DecisionFrame = serde_json::from_str(&text)?;
            if engine.trim().eq_ignore_ascii_case("none") {
                if record {
                    return Err(KernelError::Invalid(
                        "`--record` stores engine estimates; pass `--engine jev`".into(),
                    ));
                }
                print_json(&evaluate(&frame)?)?;
            } else {
                let engine = select_engine(&engine)?;
                // A bad policy file fails before the engine is called, not after.
                let record_policy = if record { Some(policy()?) } else { None };
                let estimation = estimate_missing_criteria(engine.as_ref(), &mut frame)?;
                let result = evaluate(&frame)?;
                let recorded = match record_policy {
                    Some(policy) => MemoryStore::open(store, policy)?
                        .remember_many(estimation.prediction_traces())?,
                    None => Vec::new(),
                };
                let recorded: Vec<String> = recorded.into_iter().map(|(t, _)| t.id).collect();
                print_json(&json!({
                    "decision": result,
                    "model_estimates": estimation.estimates,
                    "skipped_blocked": estimation.skipped_blocked,
                    "engine_requests": estimation.exchanges.len(),
                    "recorded_predictions": recorded,
                }))?;
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
            let recorded = if record {
                let mut memory = MemoryStore::open(store, policy()?)?;
                memory.remember_many(decision.prediction_traces(&request))?
            } else {
                Vec::new()
            };
            let recorded: Vec<String> = recorded.into_iter().map(|(t, _)| t.id).collect();
            print_json(&json!({"decision": decision, "recorded_predictions": recorded}))?;
        }
        Command::Predict {
            store,
            family,
            question,
            kind,
            p,
            value,
            answer_space,
            source,
            locator,
        } => {
            let mut trace = Forecast {
                family,
                question,
                kind,
                p,
                value,
                answer_space,
                source,
                locator,
            }
            .into_trace()?;
            // Before the store is opened, so a refused write creates nothing.
            if let Some(agent) = principal::detect_from_env() {
                agent.stamp(&mut trace)?;
            }
            let mut memory = MemoryStore::open(store, policy()?)?;
            let (trace, _) = memory.remember(trace)?;
            print_json(&trace)?;
        }
        Command::Outcome {
            store,
            prediction,
            observed,
            source,
            note,
        } => {
            let content = note.unwrap_or_else(|| format!("Outcome for {prediction}: {observed}"));
            let mut trace = Trace::new(TraceKind::Outcome, content, source);
            trace.outcome = Some(OutcomeRecord {
                prediction_id: prediction,
                observed,
            });
            if let Some(agent) = principal::detect_from_env() {
                agent.stamp(&mut trace)?;
            }
            let mut memory = MemoryStore::open_existing(store, policy()?)?;
            let (trace, _) = memory.remember(trace)?;
            print_json(&trace)?;
        }
        Command::Calibration {
            store,
            family,
            family_prefix,
        } => {
            let memory = MemoryStore::open_existing(store, policy()?)?;
            let filter = match (&family, &family_prefix) {
                (Some(family), _) => FamilyFilter::Exact(family),
                (None, Some(prefix)) => FamilyFilter::Prefix(prefix),
                (None, None) => FamilyFilter::All,
            };
            print_json(&memory.calibration_report(filter))?;
        }
        Command::Hook => {
            let (engine, threshold) = hook_settings();
            // Opt-in measurement; recording failures never change the verdict.
            let record = std::env::var_os("HIKMAH_HOOK_RECORD")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from);
            run_stop_hook_recording(
                io::stdin().lock(),
                io::stdout().lock(),
                engine.as_deref(),
                threshold,
                record.as_deref(),
            )?;
        }
        Command::GateThreshold {
            store,
            max_false_block,
        } => {
            let memory = MemoryStore::open_existing(store, policy()?)?;
            print_json(&memory.gate_threshold(max_false_block)?)?;
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
                // Same lenient parsing as `hook` (lone surrogates, trailing data, invalid UTF-8).
                print_json(&explain_stop_event(
                    io::stdin().lock(),
                    engine.as_deref(),
                    threshold,
                )?)?;
            }
        }
        Command::Validate { root } => {
            let notes = validate_repo(root)?;
            print_json(&json!({"ok": true, "checks": notes}))?;
        }
        Command::Policy { print_defaults } => {
            if print_defaults {
                print_json(&KernelPolicy::default())?;
            } else {
                print_json(&policy()?)?;
            }
        }
    }
    Ok(())
}
