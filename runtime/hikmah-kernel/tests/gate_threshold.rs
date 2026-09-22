mod common;

use common::temp_store;
use hikmah_kernel::decision_port::{EngineDescriptor, RawAnswer, StaticEngine};
use hikmah_kernel::hook::{run_stop_hook_recording, run_stop_hook_with, GATE_FAMILY};
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::trace::{OutcomeRecord, Trace, TraceKind};
use hikmah_kernel::{KernelError, MemoryStore};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

const CLEAN: &str = "Fixed the parser; all tests pass.";

fn engine(answers: BTreeMap<String, RawAnswer>) -> StaticEngine {
    StaticEngine {
        descriptor: EngineDescriptor {
            name: "fixture".into(),
            version: "1".into(),
        },
        answers,
    }
}

fn gate_engine(p: f64) -> StaticEngine {
    engine(BTreeMap::from([(
        "false_completion".to_string(),
        RawAnswer::Noul { noul: p },
    )]))
}

fn run(input: &str, engine: Option<&StaticEngine>, record: Option<&Path>) -> Vec<u8> {
    let mut out = Vec::new();
    let engine = engine.map(|e| e as &dyn hikmah_kernel::DecisionEngine);
    run_stop_hook_recording(input.as_bytes(), &mut out, engine, 0.6, record).unwrap();
    out
}

fn predictions(path: &Path) -> Vec<Trace> {
    if !path.exists() {
        return Vec::new();
    }
    MemoryStore::open_existing(path, KernelPolicy::default())
        .unwrap()
        .all()
        .filter(|e| e.trace.kind == TraceKind::Prediction)
        .map(|e| e.trace.clone())
        .collect()
}

#[test]
fn the_hook_records_engine_answers_without_changing_its_verdict() {
    let store = temp_store("hook-record");
    let input = json!({"last_assistant_message": CLEAN, "session_id": "sess-42"}).to_string();
    let high = gate_engine(0.7);

    let recorded = run(&input, Some(&high), Some(&store));
    let mut plain = Vec::new();
    run_stop_hook_with(input.as_bytes(), &mut plain, Some(&high), 0.6).unwrap();
    assert_eq!(recorded, plain, "recording must not change the output");
    assert_eq!(
        serde_json::from_slice::<Value>(&recorded).unwrap()["decision"],
        "block"
    );

    let traces = predictions(&store);
    assert_eq!(traces.len(), 1);
    let trace = &traces[0];
    let record = trace.prediction.as_ref().unwrap();
    assert_eq!(record.family, GATE_FAMILY);
    assert_eq!(record.answer_kind, "noul");
    assert_eq!(record.answer_space, vec!["true", "false"]);
    assert_eq!(record.p, Some(0.7));
    assert!(trace.is_model_authored() && !trace.provenance.verified);
    assert_eq!(trace.provenance.locator.as_deref(), Some("session:sess-42"));
    assert!(
        !trace.content.contains("parser"),
        "the message itself is not stored"
    );

    // No engine answer, no record: rules only, an abstaining engine, or a skipped event.
    run(&input, None, Some(&store));
    run(&input, Some(&engine(BTreeMap::new())), Some(&store));
    let looping = json!({"stop_hook_active": true, "last_assistant_message": CLEAN}).to_string();
    run(&looping, Some(&high), Some(&store));
    assert_eq!(predictions(&store).len(), 1);
}

#[test]
fn a_recording_failure_never_changes_the_verdict() {
    // The record path sits under a regular file, so the store cannot be created.
    let blocker = temp_store("hook-record-fail");
    std::fs::write(&blocker, b"not a directory").unwrap();
    let unusable = blocker.join("memory.jsonl");
    for (message, p) in [(CLEAN, 0.9), (CLEAN, 0.1), ("Done. TODO: add tests", 0.1)] {
        let input = json!({"last_assistant_message": message}).to_string();
        let high = gate_engine(p);
        let mut plain = Vec::new();
        run_stop_hook_with(input.as_bytes(), &mut plain, Some(&high), 0.6).unwrap();
        assert_eq!(
            run(&input, Some(&high), Some(&unusable)),
            plain,
            "{message} p={p}"
        );
    }
}

/// Record one gate prediction through the hook, then its outcome (`true` = false completion).
fn resolve(store: &Path, rows: &[(f64, bool)]) {
    let mut seen: BTreeSet<String> = predictions(store).into_iter().map(|t| t.id).collect();
    for (i, (p, false_completion)) in rows.iter().enumerate() {
        let input = json!({"last_assistant_message": CLEAN, "session_id": format!("s{i}")});
        run(&input.to_string(), Some(&gate_engine(*p)), Some(store));
        let id = predictions(store)
            .into_iter()
            .map(|t| t.id)
            .find(|id| !seen.contains(id))
            .expect("a new prediction");
        seen.insert(id.clone());
        let mut memory = MemoryStore::open_existing(store, KernelPolicy::default()).unwrap();
        let mut outcome = Trace::new(TraceKind::Outcome, "CI result for the task", "ci");
        outcome.outcome = Some(OutcomeRecord {
            prediction_id: id,
            observed: false_completion.to_string(),
        });
        memory.remember(outcome).unwrap();
    }
}

fn rows(spec: &[(f64, bool, usize)]) -> Vec<(f64, bool)> {
    spec.iter()
        .flat_map(|(p, y, count)| std::iter::repeat_n((*p, *y), *count))
        .collect()
}

#[test]
fn the_threshold_maximises_recall_within_the_false_block_budget() {
    let store = temp_store("gate-threshold");
    // 40 true completions (36 at p=0.2, 2 at 0.5, 2 at 0.7) and 20 false ones
    // (5 at 0.3, 5 at 0.6, 10 at 0.8).
    resolve(
        &store,
        &rows(&[
            (0.2, false, 36),
            (0.5, false, 2),
            (0.7, false, 2),
            (0.3, true, 5),
            (0.6, true, 5),
            (0.8, true, 10),
        ]),
    );
    let memory = MemoryStore::open_existing(&store, KernelPolicy::default()).unwrap();

    let report = memory.gate_threshold(0.10).unwrap();
    assert_eq!(
        (report.n, report.false_completions, report.true_completions),
        (60, 20, 40)
    );
    assert_eq!(report.engines, vec!["fixture@1"]);
    assert_eq!(report.threshold, Some(0.3));
    assert_eq!(report.recall, 1.0);
    assert_eq!(report.false_block_rate, 0.1);
    assert_eq!(report.false_blocks, 4);
    let [lo, hi] = report.false_block_rate_ci95;
    assert!(
        (lo - 0.0396).abs() < 5e-4 && (hi - 0.2305).abs() < 5e-4,
        "{lo} {hi}"
    );
    assert_eq!(report.at_default.recall, 0.75);
    assert_eq!(report.at_default.false_block_rate, 0.05);

    // A tighter budget gives up recall: 0.5 would block 4 true completions (0.10), so 0.6.
    let tight = memory.gate_threshold(0.05).unwrap();
    assert_eq!((tight.threshold, tight.recall), (Some(0.6), 0.75));
    let strict = memory.gate_threshold(0.0).unwrap();
    assert_eq!(
        (strict.threshold, strict.recall, strict.false_blocks),
        (Some(0.8), 0.5, 0)
    );
    assert!(memory.gate_threshold(1.5).is_err());
}

#[test]
fn too_few_outcomes_or_one_class_is_refused() {
    let store = temp_store("gate-threshold-few");
    resolve(&store, &rows(&[(0.2, false, 5), (0.8, true, 5)]));
    let memory = MemoryStore::open_existing(&store, KernelPolicy::default()).unwrap();
    match memory.gate_threshold(0.10) {
        Err(KernelError::Invalid(message)) => {
            assert!(
                message.contains("only 10") && message.contains("50"),
                "{message}"
            )
        }
        other => panic!("expected a refusal, got {other:?}"),
    }

    let store = temp_store("gate-threshold-one-class");
    resolve(&store, &rows(&[(0.2, false, 50)]));
    let memory = MemoryStore::open_existing(&store, KernelPolicy::default()).unwrap();
    assert!(
        matches!(memory.gate_threshold(0.10), Err(KernelError::Invalid(m)) if m.contains("both"))
    );
}

#[test]
fn the_cli_hook_records_nothing_without_an_engine() {
    let store = temp_store("hook-record-cli");
    let output = Command::new(env!("CARGO_BIN_EXE_hikmah"))
        .arg("hook")
        .env_remove("HIKMAH_HOOK_ENGINE")
        .env("HIKMAH_HOOK_RECORD", &store)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(br#"{"last_assistant_message": "Done. TODO: add tests"}"#)?;
            child.wait_with_output()
        })
        .unwrap();
    assert!(output.status.success());
    let verdict: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(verdict["decision"], "block");
    assert!(!store.exists(), "rules-only verdicts are not predictions");
}
