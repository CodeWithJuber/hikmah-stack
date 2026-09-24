mod common;

use common::temp_store;
use hikmah_kernel::policy::{KernelPolicy, RecallWeights};
use hikmah_kernel::recall::RecallQuery;
use hikmah_kernel::trace::{Trace, TraceKind};
use hikmah_kernel::MemoryStore;
use serde_json::Value;
use std::process::Command;

#[test]
fn defaults_are_the_constants_recall_used_before() {
    let policy = KernelPolicy::default();
    assert_eq!(
        policy.recall,
        RecallWeights {
            lexical_coverage: 0.7,
            lexical_jaccard: 0.3,
            match_floor: 0.15,
            cue_lexical: 0.8,
            cue_tag: 0.2,
            meta_recency: 0.25,
            meta_salience: 0.20,
            meta_confidence: 0.20,
            meta_provenance: 0.25,
            meta_prospective: 0.10,
            relevance_base: 0.55,
            metadata_share: 0.45,
            recency_scale_days: 30.0,
            unverified_provenance_factor: 0.65,
            prospective_scale_days: 7.0,
            undated_commitment_urgency: 0.35,
            overdue_floor: 0.15,
            listing_scale: 0.5,
            redundant_at: 0.8,
            redundancy_penalty: 0.35,
        }
    );
    assert_eq!(policy.recall_limit, 8);
    assert_eq!(policy.working_set_limit, 12);
    assert_eq!(policy.minimum_recall_score, 0.12);
    assert!(!policy.allow_sensitive_persistence);
    assert_eq!(
        policy.calibration_min_outcomes,
        hikmah_kernel::calibration::MIN_OUTCOMES
    );
    policy.validate().unwrap();
}

#[test]
fn missing_fields_keep_defaults_and_bad_fields_are_rejected() {
    assert_eq!(
        KernelPolicy::from_json("{}").unwrap(),
        KernelPolicy::default()
    );
    let partial = KernelPolicy::from_json(r#"{"recall": {"match_floor": 0.3}}"#).unwrap();
    assert_eq!(partial.recall.match_floor, 0.3);
    let mut expected = KernelPolicy::default();
    expected.recall.match_floor = 0.3;
    assert_eq!(partial, expected);
    // The full default policy round-trips.
    let printed = serde_json::to_string(&KernelPolicy::default()).unwrap();
    assert_eq!(
        KernelPolicy::from_json(&printed).unwrap(),
        KernelPolicy::default()
    );

    for bad in [
        r#"{"recal_limit": 3}"#,
        r#"{"recall": {"match_flor": 0.2}}"#,
        r#"{"recall": {"match_floor": 1.5}}"#,
        r#"{"recall": {"relevance_base": 0}}"#,
        r#"{"recall": {"recency_scale_days": 0}}"#,
        r#"{"working_set_limit": 0}"#,
        r#"{"minimum_recall_score": -0.1}"#,
        // At 0, a trace with no matching cue would be recalled.
        r#"{"minimum_recall_score": 0}"#,
        // Blended shares above 1 saturate the score clamp.
        r#"{"recall": {"lexical_coverage": 1, "lexical_jaccard": 1}}"#,
        r#"{"recall": {"cue_lexical": 0.9, "cue_tag": 0.2}}"#,
        r#"{"recall": {"relevance_base": 1, "metadata_share": 1}}"#,
        r#"{"consolidation_min_support": 0}"#,
        // At 0, a family with no outcomes would be measurable.
        r#"{"calibration_min_outcomes": 0}"#,
        r#"{"consolidation_min_independent_sources": 0}"#,
        // Data cannot lift the sensitive-persistence hard block.
        r#"{"allow_sensitive_persistence": true}"#,
    ] {
        assert!(KernelPolicy::from_json(bad).is_err(), "accepted {bad}");
    }
}

fn billing_store(policy: KernelPolicy) -> (MemoryStore, String, String) {
    let mut store = MemoryStore::open(temp_store("policy"), policy).unwrap();
    let mut verified = Trace::new(TraceKind::Belief, "billing database is Postgres", "dba");
    verified.provenance.verified = true;
    verified.provenance.authority = 0.5;
    let verified = store.remember(verified).unwrap().0.id;
    let mut loud = Trace::new(TraceKind::Belief, "billing database is MySQL", "chat");
    loud.provenance.authority = 1.0;
    let loud = store.remember(loud).unwrap().0.id;
    (store, verified, loud)
}

#[test]
fn a_policy_file_changes_recall_behaviour() {
    let query = RecallQuery::new("billing database");
    let (store, verified, loud) = billing_store(KernelPolicy::default());
    let default_order: Vec<String> = store
        .recall(&query)
        .into_iter()
        .map(|r| r.trace.id)
        .collect();
    assert_eq!(default_order, vec![loud.clone(), verified.clone()]);

    let path = temp_store("policy-file").with_file_name("policy.json");
    std::fs::write(
        &path,
        r#"{"recall": {"unverified_provenance_factor": 0.3}}"#,
    )
    .unwrap();
    let strict = KernelPolicy::from_json_file(&path).unwrap();
    let (store, verified, loud) = billing_store(strict);
    let order: Vec<String> = store
        .recall(&query)
        .into_iter()
        .map(|r| r.trace.id)
        .collect();
    assert_eq!(
        order,
        vec![verified, loud],
        "unverified sources now count for less"
    );
}

fn hikmah(args: &[&str], policy_env: Option<&str>) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hikmah"));
    command.args(args).env_remove("HIKMAH_POLICY");
    if let Some(path) = policy_env {
        command.env("HIKMAH_POLICY", path);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn the_cli_reads_policy_from_the_flag_and_the_environment() {
    let store = temp_store("policy-cli");
    let store = store.to_str().unwrap();
    let remember = |content: &str, extra: &[&str]| {
        let mut args = vec![
            "remember",
            "--store",
            store,
            "--kind",
            "belief",
            "--content",
            content,
        ];
        args.extend_from_slice(extra);
        hikmah(&args, None)["trace"]["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let verified = remember(
        "billing database is Postgres",
        &["--source", "dba", "--authority", "0.5", "--verified"],
    );
    let loud = remember(
        "billing database is MySQL",
        &["--source", "chat", "--authority", "1.0"],
    );
    let policy = std::path::Path::new(store).with_file_name("policy.json");
    std::fs::write(
        &policy,
        r#"{"recall": {"unverified_provenance_factor": 0.3}}"#,
    )
    .unwrap();
    let policy = policy.to_str().unwrap();

    let first = |value: Value| value[0]["trace"]["id"].as_str().unwrap().to_string();
    let recall = ["recall", "--store", store, "--query", "billing database"];
    assert_eq!(first(hikmah(&recall, None)), loud);
    let mut with_flag = vec!["--policy", policy];
    with_flag.extend_from_slice(&recall);
    assert_eq!(first(hikmah(&with_flag, None)), verified);
    assert_eq!(first(hikmah(&recall, Some(policy))), verified);

    let defaults = hikmah(&["policy", "--print-defaults"], Some(policy));
    let defaults: KernelPolicy = serde_json::from_value(defaults).unwrap();
    assert_eq!(
        defaults,
        KernelPolicy::default(),
        "the env policy does not leak in"
    );
    let effective = hikmah(&["policy"], Some(policy));
    assert_eq!(effective["recall"]["unverified_provenance_factor"], 0.3);
}
