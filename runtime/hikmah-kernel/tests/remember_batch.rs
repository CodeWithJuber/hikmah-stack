mod common;

use common::{temp_store, without_agent_session};
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn hikmah(args: &[&str], env: &[(&str, &str)], stdin: Option<&[u8]>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hikmah"));
    without_agent_session(&mut command).args(args);
    for (name, value) in env {
        command.env(name, value);
    }
    if let Some(input) = stdin {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input)
            .expect("write batch to stdin");
        return child.wait_with_output().unwrap();
    }
    command.output().unwrap()
}

fn batch_file(records: &[&str]) -> std::path::PathBuf {
    let dir = temp_store("batch-input");
    let path = dir.with_extension("jsonl");
    std::fs::write(&path, records.join("\n")).unwrap();
    path
}

#[test]
fn batch_writes_many_records_in_one_process() {
    let store = temp_store("batch-ok");
    let file = batch_file(&[
        r#"{"kind":"observation","content":"deploys freeze on fridays","source":"ops","tags":["deploy"]}"#,
        r#"{"kind":"belief","content":"primary db is on 5432","source":"runbook","claim_key":"db.primary.port","claim_value":"5432","confidence":0.9}"#,
        r#"{"kind":"commitment","content":"rotate the staging key","source":"human:juber","deadline":"+2d"}"#,
    ]);
    let out = hikmah(
        &[
            "remember-batch",
            "--store",
            store.to_str().unwrap(),
            "--batch-file",
            file.to_str().unwrap(),
        ],
        &[],
        None,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let body: Value = serde_json::from_slice(&out.stdout).unwrap();
    let written = body["written"].as_array().unwrap();
    assert_eq!(written.len(), 3);
    assert_eq!(written[0]["trace"]["content"], "deploys freeze on fridays");
    assert_eq!(
        written[0]["trace"]["tags"],
        Value::Array(vec!["deploy".into()])
    );
    assert_eq!(written[1]["trace"]["claim_key"], "db.primary.port");
    assert!(!written[2]["trace"]["deadline_ms"].is_null());

    // The same records are recallable through the ordinary path.
    let recall = hikmah(
        &[
            "recall",
            "--store",
            store.to_str().unwrap(),
            "--query",
            "deploys",
        ],
        &[],
        None,
    );
    assert!(recall.status.success());
    let recall: Value = serde_json::from_slice(&recall.stdout).unwrap();
    assert!(!recall.as_array().unwrap().is_empty());
}

#[test]
fn batch_validates_everything_before_writing_anything() {
    let store = temp_store("batch-invalid");
    let file = batch_file(&[
        r#"{"kind":"observation","content":"this one is fine"}"#,
        r#"{"kind":"not-a-kind","content":"this one is not"}"#,
    ]);
    let out = hikmah(
        &[
            "remember-batch",
            "--store",
            store.to_str().unwrap(),
            "--batch-file",
            file.to_str().unwrap(),
        ],
        &[],
        None,
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("batch line 2"), "{stderr}");
    assert!(
        !store.exists(),
        "a refused batch creates no store: {stderr}"
    );
}

#[test]
fn batch_rejects_unknown_fields() {
    let store = temp_store("batch-unknown-field");
    let file = batch_file(&[r#"{"kind":"observation","content":"x","contnet":"typo"}"#]);
    let out = hikmah(
        &[
            "remember-batch",
            "--store",
            store.to_str().unwrap(),
            "--batch-file",
            file.to_str().unwrap(),
        ],
        &[],
        None,
    );
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unknown field"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn batch_reads_from_stdin() {
    let store = temp_store("batch-stdin");
    let input = "{\"kind\":\"observation\",\"content\":\"from a pipe\"}\n".as_bytes();
    let out = hikmah(
        &[
            "remember-batch",
            "--store",
            store.to_str().unwrap(),
            "--batch-file",
            "-",
        ],
        &[],
        Some(input),
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let body: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(body["written"].as_array().unwrap().len(), 1);
}

#[test]
fn batch_refuses_verified_records_inside_an_agent_session() {
    let store = temp_store("batch-agent");
    let file = batch_file(&[r#"{"kind":"belief","content":"agent claim","verified":true}"#]);
    let out = hikmah(
        &[
            "remember-batch",
            "--store",
            store.to_str().unwrap(),
            "--batch-file",
            file.to_str().unwrap(),
        ],
        &[("CLAUDECODE", "1")],
        None,
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--verified is refused"), "{stderr}");
    assert!(!store.exists(), "a refused batch writes nothing");

    // The same batch with a declared person terminal is accepted.
    let declared = hikmah(
        &[
            "remember-batch",
            "--store",
            store.to_str().unwrap(),
            "--batch-file",
            file.to_str().unwrap(),
        ],
        &[("CLAUDECODE", "1"), ("HIKMAH_AGENT_SESSION", "0")],
        None,
    );
    assert!(
        declared.status.success(),
        "{}",
        String::from_utf8_lossy(&declared.stderr)
    );
    let body: Value = serde_json::from_slice(&declared.stdout).unwrap();
    assert_eq!(body["written"][0]["trace"]["provenance"]["verified"], true);
}

#[test]
fn batch_refuses_prediction_kind_like_remember() {
    let store = temp_store("batch-prediction");
    let file = batch_file(&[r#"{"kind":"prediction","content":"it will rain"}"#]);
    let out = hikmah(
        &[
            "remember-batch",
            "--store",
            store.to_str().unwrap(),
            "--batch-file",
            file.to_str().unwrap(),
        ],
        &[],
        None,
    );
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("hikmah predict"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!store.exists());
}
