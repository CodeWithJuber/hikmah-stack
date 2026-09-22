use hikmah_kernel::decision_port::{EngineDescriptor, RawAnswer, StaticEngine};
use hikmah_kernel::hook::{rules_verdict, run_stop_hook, run_stop_hook_with};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn cases() -> Vec<(String, String)> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../hooks/truth_gate_cases.json");
    let value: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    value["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["message"].as_str().unwrap().to_string(),
                c["expect"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn hook(input: &[u8]) -> Value {
    let mut out = Vec::new();
    run_stop_hook(input, &mut out).unwrap();
    serde_json::from_slice(&out).unwrap()
}

fn engine(p: f64) -> StaticEngine {
    StaticEngine {
        descriptor: EngineDescriptor {
            name: "fixture".into(),
            version: "1".into(),
        },
        answers: BTreeMap::from([("false_completion".into(), RawAnswer::Noul { noul: p })]),
    }
}

#[test]
fn golden_cases_match_the_rules() {
    let mut failures = Vec::new();
    for (message, expect) in cases() {
        let got = if rules_verdict(&message) {
            "block"
        } else {
            "allow"
        };
        if got != expect {
            failures.push(format!("expected {expect}, got {got}: {message:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn malformed_or_looping_input_always_allows_with_valid_json() {
    for input in [
        b"".as_slice(),
        b"not json",
        b"[1,2]",
        b"null",
        br#"{"last_assistant_message": ["x"]}"#,
        br#"{"stop_hook_active": "true", "last_assistant_message": "Done. TODO"}"#,
        br#"{"stop_hook_active": true, "last_assistant_message": "Done. TODO"}"#,
    ] {
        assert_eq!(hook(input), json!({}), "{}", String::from_utf8_lossy(input));
    }
    let invalid_utf8 = b"{\"last_assistant_message\": \"Done. TODO \xff\"}";
    assert_eq!(hook(invalid_utf8)["decision"], "block");
}

#[test]
fn engine_mode_uses_the_engine_and_falls_back_to_rules() {
    let input = json!({"last_assistant_message": "The migration is incomplete; see TODO list."})
        .to_string();
    let mut out = Vec::new();
    run_stop_hook_with(input.as_bytes(), &mut out, Some(&engine(0.93)), 0.8).unwrap();
    let verdict: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(verdict["decision"], "block");
    assert!(verdict["reason"].as_str().unwrap().contains("p=0.93"));

    let input = json!({"last_assistant_message": "Done. TODO: add tests"}).to_string();
    let mut out = Vec::new();
    run_stop_hook_with(input.as_bytes(), &mut out, Some(&engine(0.2)), 0.8).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&out).unwrap(), json!({}));

    let abstaining = StaticEngine {
        descriptor: EngineDescriptor {
            name: "fixture".into(),
            version: "1".into(),
        },
        answers: BTreeMap::new(),
    };
    let mut out = Vec::new();
    run_stop_hook_with(input.as_bytes(), &mut out, Some(&abstaining), 0.8).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&out).unwrap()["decision"],
        "block"
    );
}
