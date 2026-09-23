mod common;

use common::temp_store;
use hikmah_kernel::decision_port::{
    admit, ask, AdmittedAnswer, DecisionEngine, DecisionRequest, EngineDescriptor, Forecast,
    NoEngine, Question, QuestionKind, RawAnswer, RawDecision, StaticEngine,
};
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::trace::{OutcomeRecord, Trace, TraceKind};
use hikmah_kernel::MemoryStore;
use std::collections::BTreeMap;

fn request() -> DecisionRequest {
    DecisionRequest::new(
        "Canary deploy to 5% of traffic with automatic rollback.",
        vec![
            Question::noul("irreversible", "Is this plan irreversible?"),
            Question {
                id: "tier".into(),
                instructions: "Which review tier does this need?".into(),
                kind: QuestionKind::Choice {
                    options: BTreeMap::from([
                        ("light".into(), "One reviewer".into()),
                        ("heavy".into(), "Change board".into()),
                    ]),
                },
                family: None,
            },
            Question {
                id: "safety".into(),
                instructions: "How safe is it?".into(),
                kind: QuestionKind::Score {
                    levels: vec!["unsafe".into(), "neutral".into(), "safe".into()],
                },
                family: Some("deploy.safety".into()),
            },
        ],
    )
    .unwrap()
}

fn engine() -> EngineDescriptor {
    EngineDescriptor {
        name: "fixture".into(),
        version: "1".into(),
    }
}

fn raw(answers: Vec<(&str, RawAnswer)>) -> RawDecision {
    RawDecision {
        request_id: request().request_id(),
        engine: engine(),
        answers: answers
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        latency_ms: 1,
    }
}

fn good_answers() -> Vec<(&'static str, RawAnswer)> {
    vec![
        ("irreversible", RawAnswer::Noul { noul: 0.05 }),
        (
            "tier",
            RawAnswer::Choice {
                choice: "light".into(),
                probabilities: Some(BTreeMap::from([
                    ("light".into(), 0.9),
                    ("heavy".into(), 0.1),
                ])),
                confidence: Some(0.9),
            },
        ),
        (
            "safety",
            RawAnswer::Score {
                score: 1.8,
                probabilities: Some(BTreeMap::from([("1".into(), 0.2), ("2".into(), 0.8)])),
                confidence: Some(0.8),
            },
        ),
    ]
}

#[test]
fn well_formed_answers_are_admitted() {
    let decision = admit(&request(), raw(good_answers()));
    assert!(decision.rejected.is_none());
    assert!(!decision.calibrated);
    assert_eq!(
        decision.answer("irreversible").unwrap().p_true(),
        Some(0.05)
    );
    match decision.answer("safety").unwrap() {
        AdmittedAnswer::Score {
            normalized, level, ..
        } => {
            assert!((normalized - 0.9).abs() < 1e-9);
            assert_eq!(*level, 2);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn any_violation_rejects_the_whole_response() {
    let mut cases: Vec<Vec<(&str, RawAnswer)>> = Vec::new();
    let mut extra = good_answers();
    extra.push(("surprise", RawAnswer::Noul { noul: 0.5 }));
    cases.push(extra);
    let mut unknown_option = good_answers();
    unknown_option[1].1 = RawAnswer::Choice {
        choice: "medium".into(),
        probabilities: None,
        confidence: None,
    };
    cases.push(unknown_option);
    let mut bad_sum = good_answers();
    bad_sum[1].1 = RawAnswer::Choice {
        choice: "light".into(),
        probabilities: Some(BTreeMap::from([
            ("light".into(), 0.9),
            ("heavy".into(), 0.9),
        ])),
        confidence: None,
    };
    cases.push(bad_sum);
    let mut out_of_range = good_answers();
    out_of_range[2].1 = RawAnswer::Score {
        score: 7.0,
        probabilities: None,
        confidence: None,
    };
    cases.push(out_of_range);
    let mut wrong_kind = good_answers();
    wrong_kind[0].1 = RawAnswer::Choice {
        choice: "light".into(),
        probabilities: None,
        confidence: None,
    };
    cases.push(wrong_kind);
    let mut malformed = good_answers();
    malformed[0].1 = RawAnswer::Malformed {
        detail: "noul was null".into(),
    };
    cases.push(malformed);
    let mut nan = good_answers();
    nan[0].1 = RawAnswer::Noul { noul: f64::NAN };
    cases.push(nan);

    for answers in cases {
        let decision = admit(&request(), raw(answers));
        assert!(decision.rejected.is_some());
        assert!(decision.answers.values().all(AdmittedAnswer::is_abstain));
    }

    let mut foreign = raw(good_answers());
    foreign.request_id = "req_other".into();
    assert!(admit(&request(), foreign).rejected.is_some());
}

#[test]
fn unanswered_questions_become_explicit_abstentions() {
    let decision = admit(
        &request(),
        raw(vec![("irreversible", RawAnswer::Noul { noul: 0.2 })]),
    );
    assert!(decision.rejected.is_none());
    assert!(decision.answer("tier").unwrap().is_abstain());
}

#[test]
fn no_engine_abstains_and_bad_requests_are_refused() {
    let decision = ask(&NoEngine, &request()).unwrap();
    assert!(decision.answers.values().all(AdmittedAnswer::is_abstain));

    let secret = DecisionRequest {
        state: "deploy with GITHUB_TOKEN=ghp_a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8".into(),
        questions: vec![Question::noul("ok", "Is this ok?")],
    };
    assert!(ask(&NoEngine, &secret).is_err());

    let one_option = DecisionRequest {
        state: "x".into(),
        questions: vec![Question {
            id: "q".into(),
            instructions: "pick".into(),
            kind: QuestionKind::Choice {
                options: BTreeMap::from([("only".into(), "Only".into())]),
            },
            family: None,
        }],
    };
    assert!(one_option.validate().is_err());
}

struct Failing;
impl DecisionEngine for Failing {
    fn descriptor(&self) -> EngineDescriptor {
        EngineDescriptor {
            name: "failing".into(),
            version: "0".into(),
        }
    }
    fn decide(&self, _: &DecisionRequest) -> hikmah_kernel::Result<RawDecision> {
        Err(hikmah_kernel::KernelError::Engine("http 529".into()))
    }
}

#[test]
fn engine_failures_become_an_all_abstain_decision() {
    let decision = ask(&Failing, &request()).unwrap();
    assert!(decision.rejected.as_deref().unwrap().contains("529"));
    assert!(decision.answers.values().all(AdmittedAnswer::is_abstain));
}

#[test]
fn predictions_are_recorded_unverified_and_calibration_is_earned_from_outcomes() {
    let path = temp_store("calibration");
    let mut store = MemoryStore::open(&path, KernelPolicy::default()).unwrap();
    let request = request();
    let mut ids = Vec::new();
    for (p, observed) in [
        (0.9, "true"),
        (0.9, "false"),
        (0.1, "false"),
        (0.1, "false"),
    ] {
        let engine = StaticEngine {
            descriptor: engine(),
            answers: BTreeMap::from([("irreversible".into(), RawAnswer::Noul { noul: p })]),
        };
        let decision = ask(&engine, &request).unwrap();
        let traces = decision.prediction_traces(&request);
        assert_eq!(traces.len(), 1);
        let trace = traces.into_iter().next().unwrap();
        assert!(trace.is_model_authored());
        assert!(!trace.provenance.verified);
        let (trace, _) = store.remember(trace).unwrap();
        ids.push((trace.id, observed));
    }

    // A model cannot resolve its own prediction.
    let mut self_graded = Trace::new(TraceKind::Outcome, "graded by model", "model:fixture@1");
    self_graded.outcome = Some(OutcomeRecord {
        prediction_id: ids[0].0.clone(),
        observed: "true".into(),
    });
    assert!(store.remember(self_graded).is_err());

    for (id, observed) in &ids {
        let mut outcome = Trace::new(TraceKind::Outcome, "observed in production", "oncall");
        outcome.outcome = Some(OutcomeRecord {
            prediction_id: id.clone(),
            observed: observed.to_string(),
        });
        store.remember(outcome).unwrap();
    }
    let report = store.calibration(None);
    assert_eq!(report.families.len(), 1);
    let family = &report.families[0];
    assert_eq!(family.family, "irreversible");
    assert_eq!(family.n, 4);
    assert!((family.brier - 0.21).abs() < 1e-9, "{family:?}");
    assert!(!family.calibrated);
    assert!((family.rate - 0.25).abs() < 1e-9);
}

#[test]
fn credentials_anywhere_in_the_request_are_refused() {
    let request = DecisionRequest {
        state: "plain state".into(),
        questions: vec![Question::noul(
            "ok",
            "Given DB_PASSWORD=hunter2hunter2, is this safe?",
        )],
    };
    assert!(ask(&NoEngine, &request).is_err());
}

#[test]
fn answers_must_agree_with_their_own_distribution() {
    let mut not_argmax = good_answers();
    not_argmax[1].1 = RawAnswer::Choice {
        choice: "light".into(),
        probabilities: Some(BTreeMap::from([
            ("light".into(), 0.0),
            ("heavy".into(), 1.0),
        ])),
        confidence: None,
    };
    let mut aliased = good_answers();
    aliased[2].1 = RawAnswer::Score {
        score: 1.0,
        probabilities: Some(BTreeMap::from([("01".into(), 1.0)])),
        confidence: None,
    };
    let mut inconsistent = good_answers();
    inconsistent[2].1 = RawAnswer::Score {
        score: 2.0,
        probabilities: Some(BTreeMap::from([("0".into(), 1.0)])),
        confidence: None,
    };
    for answers in [not_argmax, aliased, inconsistent] {
        assert!(admit(&request(), raw(answers)).rejected.is_some());
    }
}

#[test]
fn rounded_distributions_are_accepted() {
    let mut rounded = good_answers();
    rounded[1].1 = RawAnswer::Choice {
        choice: "light".into(),
        probabilities: Some(BTreeMap::from([
            ("light".into(), 0.61),
            ("heavy".into(), 0.37),
        ])),
        confidence: None,
    };
    assert!(admit(&request(), raw(rounded)).rejected.is_none());
}

#[test]
fn missing_probabilities_are_recorded_as_unknown_not_invented() {
    let mut answers = good_answers();
    answers[1].1 = RawAnswer::Choice {
        choice: "heavy".into(),
        probabilities: None,
        confidence: None,
    };
    let decision = admit(&request(), raw(answers));
    let traces = decision.prediction_traces(&request());
    let tier = traces
        .iter()
        .find(|t| t.prediction.as_ref().unwrap().question_id == "tier")
        .unwrap();
    assert_eq!(tier.prediction.as_ref().unwrap().p, None);
    assert_eq!(
        tier.prediction.as_ref().unwrap().answer_space,
        vec!["heavy", "light"]
    );
}

#[test]
fn outcomes_are_validated_and_purged_outcomes_do_not_count() {
    let path = temp_store("outcomes");
    let mut store = MemoryStore::open(&path, KernelPolicy::default()).unwrap();
    let request = request();
    let engine = StaticEngine {
        descriptor: engine(),
        answers: BTreeMap::from([("irreversible".into(), RawAnswer::Noul { noul: 0.9 })]),
    };
    let decision = ask(&engine, &request).unwrap();
    let prediction = decision.prediction_traces(&request).remove(0);
    let (prediction, _) = store.remember(prediction).unwrap();

    let mut bad = Trace::new(TraceKind::Outcome, "observed", "oncall");
    bad.outcome = Some(OutcomeRecord {
        prediction_id: prediction.id.clone(),
        observed: "yes".into(),
    });
    assert!(store.remember(bad).is_err());

    let mut good = Trace::new(TraceKind::Outcome, "observed", "oncall");
    good.outcome = Some(OutcomeRecord {
        prediction_id: prediction.id.clone(),
        observed: "true".into(),
    });
    let (good, _) = store.remember(good).unwrap();
    assert_eq!(store.calibration(None).families[0].n, 1);
    store
        .purge(&good.id, "recorded against the wrong incident")
        .unwrap();
    let report = store.calibration(None);
    assert!(report.families.is_empty());
    assert_eq!(report.unresolved_predictions, 1);
}

fn forecast(kind: &str, p: f64, value: Option<&str>, space: &[&str]) -> Forecast {
    Forecast {
        family: "hostlelo.hero.ctr".into(),
        question: "Does the plan-finder hero raise plan clicks?".into(),
        kind: kind.into(),
        p,
        value: value.map(str::to_string),
        answer_space: space.iter().map(|s| s.to_string()).collect(),
        source: "human:juber".into(),
        locator: Some("DECISIONS.md#hero".into()),
    }
}

#[test]
fn people_and_agents_record_forecasts_that_are_never_verified() {
    let mut store = MemoryStore::open(temp_store("forecast"), KernelPolicy::default()).unwrap();
    let trace = forecast("noul", 0.7, None, &[]).into_trace().unwrap();
    assert!(!trace.is_model_authored() && !trace.provenance.verified);
    let record = trace.prediction.clone().unwrap();
    assert_eq!(record.engine, "human:juber");
    assert_eq!((record.value.as_str(), record.p), ("true", Some(0.7)));
    assert_eq!(record.answer_space, vec!["true", "false"]);
    let (stored, _) = store.remember(trace.clone()).unwrap();

    // A forecast is resolved by an outcome, never by marking it verified.
    let mut verified = trace.clone();
    verified.id.clear();
    verified.provenance.verified = true;
    let refusal = |result: hikmah_kernel::Result<_>| match result {
        Err(hikmah_kernel::KernelError::Invalid(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    };
    assert!(refusal(store.remember(verified)).contains("cannot be marked verified"));
    let mut outcome = Trace::new(TraceKind::Outcome, "clicks rose 12%", "analytics");
    outcome.outcome = Some(OutcomeRecord {
        prediction_id: stored.id,
        observed: "true".into(),
    });
    store.remember(outcome).unwrap();

    // Neither kind of forecaster can pose as the other in calibration.
    let mut poses_as_engine = trace.clone();
    poses_as_engine.id.clear();
    poses_as_engine.prediction.as_mut().unwrap().engine = "jev@jev-1.13.0".into();
    assert!(refusal(store.remember(poses_as_engine)).contains("must match its source"));
    let mut poses_as_person = trace;
    poses_as_person.id.clear();
    poses_as_person.provenance.source = "model:jev@jev-1.13.0".into();
    assert!(refusal(store.remember(poses_as_person)).contains("must match its source"));

    let choice = forecast("choice", 0.6, Some("b"), &["a", "b", "c"])
        .into_trace()
        .unwrap();
    let record = choice.prediction.unwrap();
    assert_eq!((record.value.as_str(), record.p), ("b", Some(0.6)));
    assert!(
        record.probabilities.is_empty(),
        "the rest of the distribution is not invented"
    );
}

#[test]
fn malformed_forecasts_are_refused() {
    let mut engine_source = forecast("noul", 0.7, None, &[]);
    engine_source.source = "model:jev@jev-1.13.0".into();
    let mut anonymous = forecast("noul", 0.7, None, &[]);
    anonymous.source = "unknown".into();
    let mut no_family = forecast("noul", 0.7, None, &[]);
    no_family.family = " ".into();
    // A principal names its kind, so its row never reads like an engine identity.
    let unnamed_sources = ["jev@jev-1.13.0", "juber", "human:", ":juber", "a b:juber"].map(|s| {
        let mut bad = forecast("noul", 0.7, None, &[]);
        bad.source = s.into();
        bad
    });
    for bad in unnamed_sources.into_iter().chain([
        engine_source,
        anonymous,
        no_family,
        forecast("noul", 1.2, None, &[]),
        forecast("noul", f64::NAN, None, &[]),
        forecast("noul", 0.7, Some("false"), &[]),
        forecast("choice", 0.7, Some("d"), &["a", "b"]),
        forecast("choice", 0.7, None, &["a", "b"]),
        forecast("choice", 0.7, Some("a"), &["a"]),
        forecast("choice", 0.7, Some("a"), &["a", "a"]),
        forecast("score", 0.7, Some("3"), &["0", "1", "2"]),
        forecast("guess", 0.7, None, &[]),
    ]) {
        assert!(bad.clone().into_trace().is_err(), "accepted {bad:?}");
    }
    let mut agent = forecast("noul", 0.7, None, &[]);
    agent.source = "agent:planner".into();
    assert_eq!(
        agent.into_trace().unwrap().prediction.unwrap().engine,
        "agent:planner"
    );
}

#[test]
fn an_engine_version_with_surrounding_whitespace_still_records() {
    // Jev reports its own version. The source and the record's engine are built from one trimmed
    // identity, so padding cannot make them disagree and silently stop recording.
    let request = DecisionRequest::new(
        "Deploy plan for review.",
        vec![Question::noul("irreversible", "Is this irreversible?")],
    )
    .unwrap();
    let engine = StaticEngine {
        descriptor: EngineDescriptor {
            name: " jev".into(),
            version: "jev-1.13.0 \n".into(),
        },
        answers: BTreeMap::from([("irreversible".into(), RawAnswer::Noul { noul: 0.4 })]),
    };
    assert_eq!(engine.descriptor.identity(), "jev@jev-1.13.0");
    let decision = ask(&engine, &request).unwrap();
    let traces = decision.prediction_traces(&request);
    assert_eq!(traces[0].provenance.source, "model:jev@jev-1.13.0");
    assert_eq!(
        traces[0].prediction.as_ref().unwrap().engine,
        "jev@jev-1.13.0"
    );
    let mut store = MemoryStore::open(temp_store("padded"), KernelPolicy::default()).unwrap();
    assert_eq!(store.remember_many(traces).unwrap().len(), 1);
}
