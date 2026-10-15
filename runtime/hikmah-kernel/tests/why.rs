mod common;

use serde_json::Value;
use std::process::Command;

fn hikmah() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hikmah"));
    common::without_agent_session(&mut command);
    command
}

fn run(args: &[&str]) -> (bool, String, String) {
    let output = hikmah().args(args).output().expect("run hikmah");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// A store with one corrected belief: `new` supersedes `old`.
fn corrected_store() -> (std::path::PathBuf, String, String) {
    let store = common::temp_store("why");
    let store_arg = store.to_str().unwrap().to_owned();

    let (ok, out, err) = run(&[
        "remember",
        "--store",
        &store_arg,
        "--kind",
        "belief",
        "--content",
        "The old runbook is current",
        "--source",
        "human:a",
    ]);
    assert!(ok, "remember old failed: {err}");
    let old_id = serde_json::from_str::<Value>(&out).unwrap()["trace"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let (ok, out, err) = run(&[
        "remember",
        "--store",
        &store_arg,
        "--kind",
        "belief",
        "--content",
        "The runbook was replaced in March",
        "--source",
        "human:a",
        "--supersedes",
        &old_id,
    ]);
    assert!(ok, "remember new failed: {err}");
    let new_id = serde_json::from_str::<Value>(&out).unwrap()["trace"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    (store, old_id, new_id)
}

#[test]
fn recall_hits_carry_a_top_level_trace_id() {
    let (store, _old_id, new_id) = corrected_store();
    let store_arg = store.to_str().unwrap();

    let (ok, out, err) = run(&[
        "recall",
        "--store",
        store_arg,
        "--query",
        "runbook replaced",
    ]);
    assert!(ok, "recall failed: {err}");
    let hits: Vec<Value> = serde_json::from_str(&out).unwrap();
    assert!(!hits.is_empty(), "expected at least one hit");
    for hit in &hits {
        let trace_id = hit["trace_id"].as_str().expect("trace_id present");
        assert_eq!(trace_id, hit["trace"]["id"].as_str().unwrap());
    }
    assert!(
        hits.iter().any(|h| h["trace_id"] == new_id),
        "the correction itself must be attributable"
    );
}

#[test]
fn why_prints_the_full_provenance_chain() {
    let (store, old_id, new_id) = corrected_store();
    let store_arg = store.to_str().unwrap();

    // `why` on the correction: the predecessor is the belief it replaced.
    let (ok, out, err) = run(&["why", "--store", store_arg, &new_id]);
    assert!(ok, "why failed: {err}");
    let chain: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(chain["trace"]["id"], new_id);
    assert_eq!(chain["trace"]["provenance"]["source"], "human:a");
    let predecessors: Vec<&str> = chain["predecessors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(predecessors, vec![old_id.as_str()]);
    assert!(chain["successor"].is_null());

    // `why` on the replaced belief: the successor is the correction.
    let (ok, out, err) = run(&["why", "--store", store_arg, &old_id]);
    assert!(ok, "why failed: {err}");
    let chain: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(chain["trace"]["id"], old_id);
    assert!(chain["predecessors"].as_array().unwrap().is_empty());
    assert_eq!(chain["successor"]["id"], new_id);
}

#[test]
fn why_lists_unresolved_conflicts() {
    let store = common::temp_store("why-conflicts");
    let store_arg = store.to_str().unwrap().to_owned();

    let claim = |content: &str, value: &str| -> String {
        let (ok, out, err) = run(&[
            "remember",
            "--store",
            &store_arg,
            "--kind",
            "belief",
            "--content",
            content,
            "--source",
            "human:a",
            "--claim-key",
            "region",
            "--claim-value",
            value,
        ]);
        assert!(ok, "remember failed: {err}");
        serde_json::from_str::<Value>(&out).unwrap()["trace"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let first = claim("Primary region is us-east", "us-east");
    claim("Primary region is eu-west", "eu-west");

    let (ok, out, err) = run(&["why", "--store", &store_arg, &first]);
    assert!(ok, "why failed: {err}");
    let chain: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(chain["conflicts"].as_array().unwrap().len(), 1);
}

#[test]
fn get_returns_one_trace_by_id() {
    let (store, old_id, _new_id) = corrected_store();
    let store_arg = store.to_str().unwrap();

    let (ok, out, err) = run(&["get", "--store", store_arg, &old_id]);
    assert!(ok, "get failed: {err}");
    let trace: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(trace["id"], old_id);
    assert_eq!(trace["content"], "The old runbook is current");
}

#[test]
fn get_and_why_fail_cleanly_on_unknown_ids() {
    let store = common::temp_store("why-missing");
    let store_arg = store.to_str().unwrap();

    // Before `init`, the store itself is missing, and the error says so.
    for subcommand in ["get", "why"] {
        let (ok, _out, err) = run(&[subcommand, "--store", store_arg, "tr_missing"]);
        assert!(!ok, "{subcommand} should fail on a missing store");
        assert!(
            err.contains("memory store not found") && !err.contains("trace not found"),
            "{subcommand} stderr should name the missing store, got: {err}"
        );
    }

    let (ok, _out, err) = run(&["init", "--store", store_arg]);
    assert!(ok, "init failed: {err}");
    for subcommand in ["get", "why"] {
        let (ok, _out, err) = run(&[subcommand, "--store", store_arg, "tr_missing"]);
        assert!(!ok, "{subcommand} should fail on an unknown id");
        assert!(
            err.contains("trace not found: tr_missing"),
            "{subcommand} stderr should name the problem, got: {err}"
        );
    }
}

#[test]
fn a_reader_that_closes_stdout_early_is_not_a_panic() {
    // `hikmah deliberate | head -0`: the reader is gone before hikmah writes.
    let (reader, writer) = std::io::pipe().expect("pipe");
    drop(reader);
    let output = hikmah()
        .args(["deliberate", "--irreversible-actions", "1"])
        .stdout(writer)
        .output()
        .expect("run hikmah");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "exit {:?}: {stderr}",
        output.status
    );
    assert!(
        stderr.is_empty(),
        "no panic or error expected, got: {stderr}"
    );
}
