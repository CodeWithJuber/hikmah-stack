#![cfg(feature = "jev")]

use hikmah_kernel::decision_port::{ask, AdmittedAnswer, DecisionRequest, Question, QuestionKind};
use hikmah_kernel::jev::{JevEngine, Transport};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Default)]
struct Recorder {
    calls: Arc<Mutex<Vec<Value>>>,
    replies: Arc<Mutex<Vec<(u16, Value)>>>,
}

impl Transport for Recorder {
    fn post_json(
        &self,
        url: &str,
        api_key: &str,
        body: &Value,
        _timeout: Duration,
    ) -> Result<(u16, Value), String> {
        assert!(url.ends_with("/v1/systemone"));
        assert_eq!(api_key, "test-key");
        self.calls.lock().unwrap().push(body.clone());
        let mut replies = self.replies.lock().unwrap();
        Ok(replies.remove(0))
    }
}

fn request() -> DecisionRequest {
    DecisionRequest::new(
        "Canary deploy: rolls out to 5% of traffic, automatic rollback on error-rate alarms.",
        vec![
            Question {
                id: "safety".into(),
                instructions: "How operationally safe is this deployment plan?".into(),
                kind: QuestionKind::Score {
                    levels: ["very unsafe", "unsafe", "neutral", "safe", "very safe"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                },
                family: None,
            },
            Question::noul("bad", "Is this plan irreversible?"),
        ],
    )
    .unwrap()
}

fn engine(replies: Vec<(u16, Value)>) -> (JevEngine, Recorder) {
    let recorder = Recorder {
        calls: Arc::default(),
        replies: Arc::new(Mutex::new(replies)),
    };
    let engine = JevEngine::new("test-key")
        .with_transport(Box::new(recorder.clone()))
        .with_timeout(Duration::from_secs(5));
    (engine, recorder)
}

/// A real response captured from api.typesafe.ai (jev-1.13.0) for this request.
fn captured() -> Value {
    json!({"model":"jev-1.13.0","answers":{"safety":{"type":"score","score":3.7,"confidence":0.75,
      "legend":{"0":"very unsafe","1":"unsafe","2":"neutral","3":"safe","4":"very safe"},
      "probabilities":{"0":0.0,"1":0.0,"2":0.0,"3":0.29,"4":0.71}},
      "bad":{"type":"noul","noul":0.05}},"usage":{"input_tokens":353,"output_tokens":35}})
}

#[test]
fn maps_questions_to_the_system_one_payload() {
    let (engine, _) = engine(vec![]);
    let payload = engine.payload(&request());
    assert_eq!(payload["model"], "jev-latest");
    assert_eq!(payload["questions"]["safety"]["type"], "score");
    assert_eq!(payload["questions"]["safety"]["criteria"][4], "very safe");
    assert_eq!(payload["questions"]["bad"]["type"], "noul");
}

#[test]
fn captured_response_is_admitted() {
    let (engine, recorder) = engine(vec![(200, captured())]);
    let decision = ask(&engine, &request()).unwrap();
    assert!(decision.rejected.is_none(), "{:?}", decision.rejected);
    assert_eq!(decision.engine.version, "jev-1.13.0");
    assert_eq!(decision.answer("bad").unwrap().p_true(), Some(0.05));
    match decision.answer("safety").unwrap() {
        AdmittedAnswer::Score { normalized, .. } => assert!((normalized - 0.925).abs() < 1e-9),
        other => panic!("{other:?}"),
    }
    assert_eq!(recorder.calls.lock().unwrap().len(), 1);
}

#[test]
fn overload_is_retried_then_succeeds() {
    let (engine, recorder) = engine(vec![(529, json!({})), (200, captured())]);
    let decision = ask(&engine, &request()).unwrap();
    assert!(decision.rejected.is_none());
    assert_eq!(recorder.calls.lock().unwrap().len(), 2);
}

#[test]
fn auth_failure_becomes_abstention_without_leaking_the_key() {
    let (engine, _) = engine(vec![(
        401,
        json!({"detail":{"error_type":"authentication_error","message":"bad key"}}),
    )]);
    let decision = ask(&engine, &request()).unwrap();
    let reason = decision.rejected.unwrap();
    assert!(reason.contains("401") && reason.contains("authentication_error"));
    assert!(!reason.contains("test-key"));
    assert!(!format!("{engine:?}").contains("test-key"));
}

#[test]
fn null_noul_is_malformed_and_rejects_the_response() {
    let mut reply = captured();
    reply["answers"]["bad"] = json!({"type":"noul","noul":null});
    let (engine, _) = engine(vec![(200, reply)]);
    let decision = ask(&engine, &request()).unwrap();
    assert!(decision.rejected.unwrap().contains("malformed"));
}

#[test]
fn unknown_choice_from_the_engine_is_rejected() {
    let request = DecisionRequest::new(
        "x",
        vec![Question {
            id: "tier".into(),
            instructions: "pick".into(),
            kind: QuestionKind::Choice {
                options: BTreeMap::from([("a".into(), "A".into()), ("b".into(), "B".into())]),
            },
            family: None,
        }],
    )
    .unwrap();
    let (engine, _) = engine(vec![(
        200,
        json!({"model":"jev-1.13.0","answers":{"tier":{"type":"choice","choice":"c","confidence":1.0}}}),
    )]);
    assert!(ask(&engine, &request).unwrap().rejected.is_some());
}

/// Live check against api.typesafe.ai. Run with:
/// `HIKMAH_LIVE_JEV=1 TYPESAFE_API_KEY=... cargo test -p hikmah-kernel --test jev -- --ignored`
#[test]
#[ignore]
fn live_jev_round_trip() {
    if std::env::var("HIKMAH_LIVE_JEV").is_err() {
        return;
    }
    let engine = JevEngine::from_env().expect("TYPESAFE_API_KEY");
    let decision = ask(&engine, &request()).unwrap();
    assert!(decision.rejected.is_none(), "{:?}", decision.rejected);
    println!("{}", serde_json::to_string_pretty(&decision).unwrap());
}
