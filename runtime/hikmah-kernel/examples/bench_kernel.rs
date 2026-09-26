//! Offline measurements against the real kernel. Synthetic workloads are explicitly labelled.
use clap::{Parser, Subcommand};
use hikmah_kernel::decision::{evaluate, Criterion, DecisionFrame, DecisionOption};
use hikmah_kernel::hook::rules_verdict;
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::recall::RecallQuery;
use hikmah_kernel::trace::{Trace, TraceKind};
use hikmah_kernel::MemoryStore;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const CLOCK: u64 = 1_700_000_000_000;

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Mode,
}

#[derive(Subcommand)]
enum Mode {
    Scale {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long, default_value_t = 1000)]
        records: usize,
        #[arg(long, default_value_t = 30)]
        queries: usize,
    },
    Decisions {
        #[arg(long, default_value_t = 10000)]
        cases: usize,
        #[arg(long, default_value_t = 20260926)]
        seed: u64,
    },
    Recovery {
        #[arg(long)]
        dir: PathBuf,
    },
    /// Internal process killed after acknowledging a committed batch, not during a write.
    #[command(hide = true)]
    CommittedChild {
        #[arg(long)]
        dir: PathBuf,
    },
    /// JSONL {id, message} -> {id, blocked, ms}; no labels enter the kernel.
    Gate,
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn quantiles(mut values: Vec<f64>) -> Value {
    values.sort_by(f64::total_cmp);
    let q = |p: f64| {
        if values.is_empty() {
            return None;
        }
        let x = (values.len() - 1) as f64 * p;
        let lo = x.floor() as usize;
        let hi = x.ceil() as usize;
        Some(values[lo] + (values[hi] - values[lo]) * x.fract())
    };
    json!({"n": values.len(), "p50": q(0.5), "p95": q(0.95), "p99": q(0.99)})
}

fn rss() -> Value {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |key: &str| {
        status.lines().find_map(|line| {
            line.strip_prefix(key)
                .and_then(|s| s.split_whitespace().next())
                .and_then(|n| n.parse::<u64>().ok())
        })
    };
    json!({"rss_kib": field("VmRSS:"), "process_peak_rss_kib": field("VmHWM:")})
}

fn token(mut n: usize) -> String {
    let mut out = String::from("event");
    for _ in 0..8 {
        out.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
    }
    out
}

fn trace(i: usize) -> Trace {
    let mut t = Trace::new(
        TraceKind::Observation,
        format!("{} deployment observation", token(i)),
        "synthetic-kernel-benchmark",
    );
    t.id = format!("sample-{i}");
    t.created_at_ms = CLOCK;
    t.provenance.observed_at_ms = CLOCK;
    t
}

fn fresh(dir: &Path) -> Result<PathBuf> {
    // Never open or modify an existing user's memory, even when it is empty.
    fs::create_dir(dir)?;
    Ok(dir.join("memory.jsonl"))
}

fn scale(dir: &Path, records: usize, queries: usize) -> Result<Value> {
    if records == 0 || queries == 0 {
        return Err("records and queries must be positive".into());
    }
    let path = fresh(dir)?;
    let mut store = MemoryStore::open(&path, KernelPolicy::default())?;
    let single_count = records.min(50);
    let batched = records - single_count;
    let mut batches = Vec::new();
    let seed_started = Instant::now();
    for begin in (0..batched).step_by(1000) {
        let traces = (begin..(begin + 1000).min(batched)).map(trace).collect();
        let start = Instant::now();
        store.remember_many(traces)?;
        batches.push(ms(start));
    }
    let mut appends = Vec::new();
    for i in batched..records {
        let t = trace(i);
        let start = Instant::now();
        store.remember(t)?;
        appends.push(ms(start));
    }
    let seed_ms = ms(seed_started);
    let memory_after_seed = rss();
    let mut recall_ms = Vec::new();
    let mut hits = 0;
    let mut returned = 0;
    let mut result_ids = Vec::new();
    // Warm up with an exact cue; quality here is a synthetic exact-cue check, not field recall.
    std::hint::black_box(store.recall(&RecallQuery::new(token(0))));
    for i in 0..queries {
        let selected = i * records / queries;
        let mut query = RecallQuery::new(token(selected));
        query.now_ms = CLOCK;
        query.limit = 1;
        let start = Instant::now();
        let found = store.recall(&query);
        recall_ms.push(ms(start));
        returned += found.len();
        hits += usize::from(
            found
                .first()
                .is_some_and(|r| r.trace.id == format!("sample-{selected}")),
        );
        result_ids.push(found.iter().map(|r| r.trace.id.clone()).collect::<Vec<_>>());
    }
    let start = Instant::now();
    store.verify()?;
    let verify_ms = ms(start);
    let before = store.head();
    drop(store);
    let start = Instant::now();
    let store = MemoryStore::open_existing(&path, KernelPolicy::default())?;
    let replay_ms = ms(start);
    store.verify()?;
    let retained = store
        .all()
        .filter(|e| {
            e.trace.provenance.source == "synthetic-kernel-benchmark"
                && e.trace.provenance.observed_at_ms == CLOCK
                && !e.trace.provenance.verified
        })
        .count();
    let mut replay_matches = 0;
    for (i, expected) in result_ids.iter().enumerate() {
        let mut query = RecallQuery::new(token(i * records / queries));
        query.now_ms = CLOCK;
        query.limit = 1;
        let ids: Vec<_> = store
            .recall(&query)
            .into_iter()
            .map(|r| r.trace.id)
            .collect();
        replay_matches += usize::from(ids == *expected);
    }
    let head_same = serde_json::to_value(before)? == serde_json::to_value(store.head())?;
    let ok = store.record_count() == records
        && retained == records
        && hits == queries
        && replay_matches == queries
        && head_same;
    Ok(json!({
        "kind": "synthetic_scale", "ok": ok, "records": records, "queries": queries,
        "workload": "unclaimed observation traces, unique exact lexical cues, fixed clock",
        "seed_ms": seed_ms, "batch_size": 1000, "batched_records": batched,
        "batch_append_ms": quantiles(batches), "single_append_ms": quantiles(appends),
        "recall_ms": quantiles(recall_ms), "verify_ms": verify_ms, "replay_ms": replay_ms,
        "exact_cue_hits": hits, "returned": returned, "recall_at_1": hits as f64 / queries as f64,
        "provenance_retained": retained, "replayed_records": store.record_count(),
        "recall_replay_matches": replay_matches, "head_unchanged": head_same,
        "ledger_bytes": fs::metadata(path)?.len(), "memory_after_seed": memory_after_seed,
        "memory_after_replay": rss(), "policy": store.policy(),
        "limits": ["Batched seeding does not measure per-record durable append latency.",
          "Warm filesystem cache; reopening in the same process is not a cold-start measurement.",
          "No structured claims in this workload; conflict-heavy stores need a separate workload.",
          "Synthetic exact-cue quality is not semantic or real-world retrieval accuracy.",
          "RSS is process-wide Linux /proc data; unavailable platforms report null."]
    }))
}

// Specified, platform-independent SplitMix64; never a cryptographic RNG.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut x = self.0;
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        x ^ (x >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() % 10001) as f64 / 10000.0
    }
}

fn option(name: &str, a: f64, b: f64) -> DecisionOption {
    DecisionOption {
        name: name.into(),
        description: None,
        scores: BTreeMap::from([("a".into(), a), ("b".into(), b)]),
        model_scores: BTreeMap::new(),
        evidence_confidence: 1.0,
        hard_blocks: vec![],
        reversible: false,
    }
}

fn decisions(cases: usize, seed: u64) -> Result<Value> {
    if cases < 6 {
        return Err("at least 6 cases required to exercise every family".into());
    }
    let mut rng = Rng(seed);
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut violations = 0;
    let start = Instant::now();
    for i in 0..cases {
        let mut a = option("a", rng.unit(), rng.unit());
        let mut b = option("b", rng.unit(), rng.unit());
        let family;
        let mut expected: Option<Option<String>> = None;
        match i % 6 {
            0 => {
                family = "high_score_hard_block";
                a.scores.values_mut().for_each(|s| *s = 1.0);
                a.hard_blocks.push("no authorization".into());
                expected = Some(Some("b".into()));
            }
            1 => {
                family = "all_blocked";
                a.hard_blocks.push("privacy".into());
                b.hard_blocks.push("consent".into());
                expected = Some(None);
            }
            2 => {
                family = "complete_score_oracle";
                let sum_a: f64 = a.scores.values().sum();
                let sum_b: f64 = b.scores.values().sum();
                expected = Some(Some(if sum_a >= sum_b { "a" } else { "b" }.into()));
            }
            3 => {
                family = "unknowns_and_model_estimates";
                a.scores.clear();
                a.model_scores.insert("a".into(), rng.unit());
            }
            4 => {
                family = "reversibility_band";
                let top = 0.5 + rng.unit() * 0.4;
                let gap = if rng.next().is_multiple_of(2) {
                    0.01
                } else {
                    0.03
                };
                a.scores.values_mut().for_each(|s| *s = top);
                a.evidence_confidence = 0.1;
                b.scores.values_mut().for_each(|s| *s = top - gap);
                b.reversible = true;
                expected = Some(Some(if gap < 0.02 { "b" } else { "a" }.into()));
            }
            _ => {
                family = "invalid_numeric_input";
                a.scores.insert("a".into(), 1.01 + rng.unit());
            }
        }
        *counts.entry(family).or_default() += 1;
        let frame = DecisionFrame {
            question: "synthetic decision".into(),
            criteria: ["a", "b"]
                .into_iter()
                .map(|id| Criterion {
                    id: id.into(),
                    weight: 1.0,
                    description: id.into(),
                })
                .collect(),
            options: vec![a, b],
        };
        let actual = evaluate(&frame);
        let mut reasons = Vec::new();
        if family == "invalid_numeric_input" {
            if actual.is_ok() {
                reasons.push("invalid input admitted");
            }
        } else if let Ok(answer) = &actual {
            if let Some(expected) = expected {
                if answer.recommended != expected {
                    reasons.push("wrong recommendation");
                }
            }
            if let Some(name) = &answer.recommended {
                if frame
                    .options
                    .iter()
                    .any(|o| &o.name == name && !o.hard_blocks.is_empty())
                {
                    reasons.push("hard-blocked option selected");
                }
            }
            if family == "unknowns_and_model_estimates" {
                let ranked = answer.ranking.iter().find(|o| o.name == "a").unwrap();
                if ranked.coverage != 0.0
                    || ranked.evidence_interval != [0.0, 1.0]
                    || ranked.missing_criteria != ["b"]
                    || answer.decisive
                {
                    reasons.push("unknown evidence hidden or model guess counted as evidence");
                }
            }
            let mut permuted = frame.clone();
            permuted.options.reverse();
            permuted.question = "irrelevant wording changed".into();
            let other = evaluate(&permuted)?;
            if answer.recommended != other.recommended
                || serde_json::to_value(&answer.ranking)? != serde_json::to_value(&other.ranking)?
            {
                reasons.push("order or irrelevant text changed result");
            }
        } else {
            reasons.push("valid frame rejected");
        }
        if !reasons.is_empty() {
            violations += 1;
            if failures.len() < 20 {
                failures.push(
                    json!({"case_index": i, "family": family, "reasons": reasons,
                    "frame": frame, "actual": actual.as_ref().ok()}),
                );
            }
        }
    }
    Ok(
        json!({"kind": "synthetic_decision_invariants", "seed": seed, "rng": "splitmix64",
        "cases": cases, "families": counts, "violations": violations,
        "violation_rate": violations as f64 / cases as f64, "ok": violations == 0,
        "elapsed_ms": ms(start), "counterexamples": failures,
        "limits": ["Generated families are not real decision-quality labels or a proof over all inputs.",
          "Metamorphic checks keep criteria fixed; weight perturbations require separate expectations."]}),
    )
}

fn committed_child(dir: &Path) -> Result<()> {
    let path = fresh(dir)?;
    let mut store = MemoryStore::open(path, KernelPolicy::default())?;
    store.remember_many((0..50).map(trace).collect())?;
    println!("committed:50");
    io::stdout().flush()?;
    // Parent kills us after the acknowledged commit; this is not an in-flight crash claim.
    loop {
        std::thread::park();
    }
}

fn recovery(dir: &Path) -> Result<Value> {
    fs::create_dir(dir)?;
    let child_dir = dir.join("committed");
    let mut child = Command::new(std::env::current_exe()?)
        .args(["committed-child", "--dir"])
        .arg(&child_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut ack = String::new();
    let read_result = io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut ack);
    let kill_result = child.kill();
    let wait_result = child.wait();
    read_result?;
    kill_result?;
    wait_result?;
    if ack.trim() != "committed:50" {
        return Err("child did not acknowledge commit".into());
    }
    let path = child_dir.join("memory.jsonl");
    let start = Instant::now();
    let store = MemoryStore::open_existing(&path, KernelPolicy::default())?;
    store.verify()?;
    let restart_ms = ms(start);
    let acknowledged_survived =
        store.record_count() == 50 && (0..50).all(|i| store.get(&format!("sample-{i}")).is_some());
    let head_path = store.head_path();
    drop(store);
    let original = fs::read(&path)?;
    let original_head = fs::read(head_path)?;
    let copied_store = |name: &str, bytes: &[u8]| -> Result<PathBuf> {
        let target = fresh(&dir.join(name))?;
        let store = MemoryStore::open(&target, KernelPolicy::default())?;
        let target_head = store.head_path();
        drop(store);
        fs::write(&target, bytes)?;
        fs::write(target_head, &original_head)?;
        Ok(target)
    };
    let torn = copied_store("torn", &original)?;
    OpenOptions::new()
        .append(true)
        .open(&torn)?
        .write_all(b"{\"seq\":")?;
    let mut store = MemoryStore::open_existing(&torn, KernelPolicy::default())?;
    store.remember(trace(50))?;
    store.verify()?;
    let torn_repaired = store.record_count() == 51;
    let mut edited = String::from_utf8(original.clone())?;
    edited = edited.replacen("deployment observation", "deployment fabrication", 1);
    let tampered = copied_store("edited", edited.as_bytes())?;
    let rejected = |path: &Path| match MemoryStore::open_existing(path, KernelPolicy::default()) {
        Err(_) => true,
        Ok(store) => store.verify().is_err(),
    };
    let edit_detected = rejected(&tampered);
    let last = original[..original.len() - 1]
        .iter()
        .rposition(|b| *b == b'\n')
        .unwrap()
        + 1;
    let truncated = copied_store("truncated", &original[..last])?;
    let truncation_detected = rejected(&truncated);
    Ok(
        json!({"kind": "synthetic_recovery", "acknowledged_records": 50,
        "acknowledged_survived": acknowledged_survived, "restart_ms": restart_ms,
        "torn_tail_repaired": torn_repaired, "edit_detected": edit_detected,
        "truncation_detected": truncation_detected,
        "ok": acknowledged_survived && torn_repaired && edit_detected && truncation_detected,
        "limits": ["Process killed after an acknowledged commit, not during fsync or head replacement.",
          "Torn tail, edit and truncation are injected into separate copies.",
          "This does not simulate power loss or an attacker rewriting both ledger and head."]}),
    )
}

fn gate() -> Result<()> {
    let mut out = io::BufWriter::new(io::stdout().lock());
    // Exclude one-time regex compilation from per-message timings.
    std::hint::black_box(rules_verdict("Done. TODO: tests"));
    for line in io::stdin().lock().lines() {
        let input: Value = serde_json::from_str(&line?)?;
        let id = input["id"].as_str().ok_or("id must be a string")?;
        let message = input["message"]
            .as_str()
            .ok_or("message must be a string")?;
        let start = Instant::now();
        let blocked = rules_verdict(message);
        writeln!(
            out,
            "{}",
            json!({"id": id, "blocked": blocked, "ms": ms(start)})
        )?;
    }
    out.flush()?;
    Ok(())
}

fn main() -> Result<()> {
    let result = match Args::parse().command {
        Mode::Scale {
            dir,
            records,
            queries,
        } => scale(&dir, records, queries)?,
        Mode::Decisions { cases, seed } => decisions(cases, seed)?,
        Mode::Recovery { dir } => recovery(&dir)?,
        Mode::CommittedChild { dir } => return committed_child(&dir),
        Mode::Gate => return gate(),
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    if result["ok"] != true {
        std::process::exit(1);
    }
    Ok(())
}
