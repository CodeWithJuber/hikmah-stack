use clap::{Parser, Subcommand};
use hikmah_kernel::council::{deliberate, DeliberationInput};
use hikmah_kernel::decision::{evaluate, DecisionFrame};
use hikmah_kernel::hook::run_stop_hook;
use hikmah_kernel::planner::{plan, PlanProblem};
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::recall::RecallQuery;
use hikmah_kernel::trace::{PrivacyClass, Trace, TraceKind};
use hikmah_kernel::validate::validate_repo;
use hikmah_kernel::{MemoryStore, Result};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::str::FromStr;

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
    Init {
        #[arg(long, default_value = ".hikmah/memory.jsonl")]
        store: PathBuf,
    },
    Remember {
        #[arg(long, default_value = ".hikmah/memory.jsonl")]
        store: PathBuf,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        content: String,
        #[arg(long, default_value = "user")]
        source: String,
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
        #[arg(long)]
        verified: bool,
    },
    Recall {
        #[arg(long, default_value = ".hikmah/memory.jsonl")]
        store: PathBuf,
        #[arg(long)]
        query: String,
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(long, default_value_t = 8)]
        limit: usize,
    },
    Commitments {
        #[arg(long, default_value = ".hikmah/memory.jsonl")]
        store: PathBuf,
        #[arg(long, default_value_t = 168)]
        within_hours: u64,
    },
    Consolidate {
        #[arg(long, default_value = ".hikmah/memory.jsonl")]
        store: PathBuf,
    },
    VerifyLedger {
        #[arg(long, default_value = ".hikmah/memory.jsonl")]
        store: PathBuf,
    },
    Plan {
        #[arg(long)]
        problem: PathBuf,
    },
    Decide {
        #[arg(long)]
        frame: PathBuf,
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

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { store } => {
            let memory = MemoryStore::open(&store, KernelPolicy::default())?;
            println!(
                "{}",
                serde_json::json!({"store": store, "records": memory.record_count()})
            );
        }
        Command::Remember {
            store,
            kind,
            content,
            source,
            tags,
            salience,
            confidence,
            privacy,
            claim_key,
            claim_value,
            supersedes,
            verified,
        } => {
            let mut memory = MemoryStore::open(store, KernelPolicy::default())?;
            let mut trace = Trace::new(TraceKind::from_str(&kind)?, content, source);
            trace.tags = tags;
            trace.salience = salience;
            trace.confidence = confidence;
            trace.privacy = PrivacyClass::from_str(&privacy)?;
            trace.claim_key = claim_key;
            trace.claim_value = claim_value;
            trace.supersedes = supersedes;
            trace.provenance.verified = verified;
            let (trace, conflicts) = memory.remember(trace)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "trace": trace,
                    "conflicts": conflicts
                }))?
            );
        }
        Command::Recall {
            store,
            query,
            tags,
            limit,
        } => {
            let memory = MemoryStore::open(store, KernelPolicy::default())?;
            let mut recall = RecallQuery::new(query);
            recall.tags = tags;
            recall.limit = limit;
            println!("{}", serde_json::to_string_pretty(&memory.recall(&recall))?);
        }
        Command::Commitments {
            store,
            within_hours,
        } => {
            let memory = MemoryStore::open(store, KernelPolicy::default())?;
            let now = hikmah_kernel::trace::now_ms();
            let within_ms = within_hours.saturating_mul(3_600_000);
            println!(
                "{}",
                serde_json::to_string_pretty(&memory.commitments_due(now, within_ms))?
            );
        }
        Command::Consolidate { store } => {
            let memory = MemoryStore::open(store, KernelPolicy::default())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&memory.consolidation_proposals())?
            );
        }
        Command::VerifyLedger { store } => {
            let memory = MemoryStore::open(store, KernelPolicy::default())?;
            memory.verify()?;
            println!(
                "{}",
                serde_json::json!({"ok": true, "records": memory.record_count()})
            );
        }
        Command::Plan { problem } => {
            let text = fs::read_to_string(problem)?;
            let problem: PlanProblem = serde_json::from_str(&text)?;
            println!("{}", serde_json::to_string_pretty(&plan(&problem)?)?);
        }
        Command::Decide { frame } => {
            let text = fs::read_to_string(frame)?;
            let frame: DecisionFrame = serde_json::from_str(&text)?;
            println!("{}", serde_json::to_string_pretty(&evaluate(&frame)?)?);
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
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Hook => run_stop_hook(io::stdin().lock(), io::stdout().lock())?,
        Command::Validate { root } => {
            let notes = validate_repo(root)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({"ok": true, "checks": notes}))?
            );
        }
    }
    Ok(())
}
