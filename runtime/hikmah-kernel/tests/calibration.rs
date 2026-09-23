mod common;

use common::temp_store;
use hikmah_kernel::calibration::{
    spiegelhalter_z, FamilyCalibration, FamilyFilter, MIN_OUTCOMES, POOLED_KIND,
};
use hikmah_kernel::decision_port::{
    ask, DecisionRequest, EngineDescriptor, Forecast, Question, QuestionKind, RawAnswer,
    StaticEngine,
};
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::trace::{OutcomeRecord, Trace, TraceKind};
use hikmah_kernel::MemoryStore;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

fn engine(answer: RawAnswer, question: &str) -> StaticEngine {
    StaticEngine {
        descriptor: EngineDescriptor {
            name: "fixture".into(),
            version: "1".into(),
        },
        answers: BTreeMap::from([(question.to_string(), answer)]),
    }
}

/// Record one prediction per `(answer, observed)` pair and resolve it, then report the family.
fn resolve(
    request: &DecisionRequest,
    question: &str,
    rows: &[(RawAnswer, &str)],
) -> FamilyCalibration {
    let mut store = MemoryStore::open(temp_store("calibration"), KernelPolicy::default()).unwrap();
    for (answer, observed) in rows {
        let decision = ask(&engine(answer.clone(), question), request).unwrap();
        let prediction = decision.prediction_traces(request).remove(0);
        let (prediction, _) = store.remember(prediction).unwrap();
        let mut outcome = Trace::new(TraceKind::Outcome, "observed in CI", "ci");
        outcome.outcome = Some(OutcomeRecord {
            prediction_id: prediction.id,
            observed: observed.to_string(),
        });
        store.remember(outcome).unwrap();
    }
    let mut report = store.calibration(None);
    assert_eq!(report.families.len(), 1);
    report.families.remove(0)
}

fn noul_request() -> DecisionRequest {
    DecisionRequest::new(
        "Change set for review.",
        vec![Question::noul(
            "breaks",
            "Will this change break the build?",
        )],
    )
    .unwrap()
}

/// `count` noul predictions at `p`, of which `true_count` resolve `true`.
fn noul_rows(p: f64, count: usize, true_count: usize) -> Vec<(RawAnswer, &'static str)> {
    (0..count)
        .map(|i| {
            let observed = if i < true_count { "true" } else { "false" };
            (RawAnswer::Noul { noul: p }, observed)
        })
        .collect()
}

#[test]
fn well_calibrated_informative_predictions_are_calibrated() {
    // 50 at p=0.2 with 10 true, 50 at p=0.8 with 40 true: observed rates equal the forecasts.
    let mut rows = noul_rows(0.2, 50, 10);
    rows.extend(noul_rows(0.8, 50, 40));
    let family = resolve(&noul_request(), "breaks", &rows);
    assert_eq!(family.n, 100);
    assert!(family.measurable);
    assert!(family.z.unwrap().abs() < 1e-9, "{family:?}");
    assert!((family.p_value.unwrap() - 1.0).abs() < 1e-6);
    // Brier 0.16 against a base-rate Brier of 0.25.
    assert!((family.brier_skill.unwrap() - 0.36).abs() < 1e-9);
    assert!(family.calibrated, "{family:?}");
}

#[test]
fn fifty_confident_misses_are_measurable_but_not_calibrated() {
    // Audit probe P6: 50 predictions at p=0.95, every one wrong. n >= 50 used to be enough.
    let family = resolve(&noul_request(), "breaks", &noul_rows(0.95, MIN_OUTCOMES, 0));
    assert!(family.measurable);
    assert!(family.z.unwrap() > 20.0, "{family:?}");
    assert!(family.p_value.unwrap() < 1e-6);
    assert!((family.ece - 0.95).abs() < 1e-9);
    assert!(
        family.brier_skill.is_none(),
        "base rate 0: nothing can beat it"
    );
    assert!(!family.calibrated);
}

#[test]
fn calibrated_but_uninformative_or_small_families_are_not_calibrated() {
    // Always forecasting the base rate passes the Z test but has no skill.
    let flat = resolve(&noul_request(), "breaks", &noul_rows(0.3, 60, 18));
    assert!(flat.z.unwrap().abs() < 1e-9);
    assert!(flat.brier_skill.unwrap().abs() < 1e-9);
    assert!(!flat.calibrated, "{flat:?}");

    // Good forecasts, but below the sample-size requirement.
    let mut rows = noul_rows(0.2, 10, 2);
    rows.extend(noul_rows(0.8, 10, 8));
    let small = resolve(&noul_request(), "breaks", &rows);
    assert!(!small.measurable && !small.calibrated);
    assert!(small.brier_skill.unwrap() > 0.0);
}

#[test]
fn choice_families_use_the_top_label_for_both_checks() {
    let request = DecisionRequest::new(
        "Change set for review.",
        vec![Question {
            id: "tier".into(),
            instructions: "Which review tier does this need?".into(),
            kind: QuestionKind::Choice {
                options: BTreeMap::from([
                    ("light".into(), "One reviewer".into()),
                    ("heavy".into(), "Change board".into()),
                ]),
            },
            family: None,
        }],
    )
    .unwrap();
    let pick = |p: f64| RawAnswer::Choice {
        choice: "light".into(),
        probabilities: Some(BTreeMap::from([
            ("light".into(), p),
            ("heavy".into(), 1.0 - p),
        ])),
        confidence: None,
    };
    let mut rows = Vec::new();
    // 30 at P(top)=0.9 with 27 correct, 30 at P(top)=0.6 with 18 correct.
    for i in 0..30 {
        rows.push((pick(0.9), if i < 27 { "light" } else { "heavy" }));
        rows.push((pick(0.6), if i < 18 { "light" } else { "heavy" }));
    }
    let family = resolve(&request, "tier", &rows);
    assert_eq!(family.answer_kind, "choice");
    assert!(family.z.unwrap().abs() < 1e-9, "{family:?}");
    // Top-label Brier 0.165 against accuracy 0.75 (base-rate Brier 0.1875).
    assert!(
        (family.brier_skill.unwrap() - 0.12).abs() < 1e-9,
        "{family:?}"
    );
    assert!(family.calibrated);

    // Confident and wrong on the top label: rejected by the Z test.
    let wrong: Vec<_> = (0..60)
        .map(|i| (pick(0.9), if i < 30 { "light" } else { "heavy" }))
        .collect();
    let family = resolve(&request, "tier", &wrong);
    assert!(family.z.unwrap() > 1.96 && !family.calibrated, "{family:?}");
}

#[test]
fn spiegelhalter_z_is_undefined_without_variance() {
    assert!(spiegelhalter_z(&[(0.5, 1.0), (0.5, 0.0)]).is_none());
    assert!(spiegelhalter_z(&[(1.0, 1.0), (0.0, 0.0)]).is_none());
    assert!(spiegelhalter_z(&[]).is_none());
}

/// Store `prediction` and, when given, an outcome for it.
fn record(store: &mut MemoryStore, prediction: Trace, observed: Option<&str>) {
    let (prediction, _) = store.remember(prediction).unwrap();
    if let Some(observed) = observed {
        let mut outcome = Trace::new(TraceKind::Outcome, "observed in analytics", "analytics");
        outcome.outcome = Some(OutcomeRecord {
            prediction_id: prediction.id,
            observed: observed.to_string(),
        });
        store.remember(outcome).unwrap();
    }
}

/// The fixture engine's answer to one noul question in `family`, as a prediction trace.
fn engine_noul(family: &str, p: f64) -> Trace {
    let mut question = Question::noul("lift", "Does this raise plan clicks?");
    question.family = Some(family.into());
    let request = DecisionRequest::new("Hero layout change.", vec![question]).unwrap();
    let decision = ask(&engine(RawAnswer::Noul { noul: p }, "lift"), &request).unwrap();
    decision.prediction_traces(&request).remove(0)
}

fn human_noul(family: &str, p: f64) -> Trace {
    Forecast {
        family: family.into(),
        question: "Does this raise plan clicks?".into(),
        kind: "noul".into(),
        p,
        value: None,
        answer_space: Vec::new(),
        source: "human:juber".into(),
        locator: None,
    }
    .into_trace()
    .unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn people_and_engines_are_scored_side_by_side_per_family() {
    let mut store = MemoryStore::open(temp_store("side-by-side"), KernelPolicy::default()).unwrap();
    record(
        &mut store,
        engine_noul("hostlelo.pricing.ctr", 0.9),
        Some("true"),
    );
    record(
        &mut store,
        human_noul("hostlelo.pricing.ctr", 0.3),
        Some("false"),
    );
    record(
        &mut store,
        engine_noul("hostlelo.hero.ctr", 0.8),
        Some("true"),
    );
    record(
        &mut store,
        engine_noul("hostlelo.hero.ctr", 0.6),
        Some("false"),
    );
    record(
        &mut store,
        human_noul("hostlelo.hero.ctr", 0.7),
        Some("true"),
    );

    let report = store.calibration(None);
    let rows: Vec<(&str, &str)> = report
        .families
        .iter()
        .map(|f| (f.family.as_str(), f.engine.as_str()))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("hostlelo.hero.ctr", "fixture@1"),
            ("hostlelo.hero.ctr", "human:juber"),
            ("hostlelo.pricing.ctr", "fixture@1"),
            ("hostlelo.pricing.ctr", "human:juber"),
        ],
        "every forecaster's row for a family is adjacent"
    );
    let (engine_row, human_row) = (&report.families[0], &report.families[1]);
    assert_eq!(engine_row.n, 2);
    assert!(close(engine_row.brier, 0.2), "{engine_row:?}");
    assert_eq!(human_row.n, 1);
    assert!(close(human_row.brier, 0.09), "{human_row:?}");
    // Far below the minimum: the Brier is shown with its n, labelled anecdotal.
    for row in &report.families {
        assert_eq!(row.evidence, "anecdotal");
        assert!(!row.measurable && !row.calibrated);
        assert!(
            close(row.top_label_brier, row.brier),
            "noul: the same number"
        );
    }
    assert!(
        report.pooled.is_empty(),
        "pooling is asked for with a prefix"
    );
}

#[test]
fn a_family_prefix_pools_each_forecaster_across_families() {
    let mut store = MemoryStore::open(temp_store("pooled"), KernelPolicy::default()).unwrap();
    record(
        &mut store,
        engine_noul("hostlelo.hero.ctr", 0.8),
        Some("true"),
    );
    // Leans `false` at P(true) = 0.2 and is right: top label 0.8, hit.
    record(
        &mut store,
        human_noul("hostlelo.hero.ctr", 0.2),
        Some("false"),
    );
    record(&mut store, human_noul("hostlelo.hero.ctr", 0.9), None);
    record(&mut store, human_noul("other.team.ctr", 0.9), Some("false"));
    record(&mut store, human_noul("other.team.ctr", 0.9), None);

    // A score family: level 3 at P = 0.6, and level 3 happened.
    let request = DecisionRequest::new(
        "Hero layout change.",
        vec![Question {
            id: "perf".into(),
            instructions: "How fast is the hero on mid-range mobile?".into(),
            kind: QuestionKind::Score {
                levels: ["very poor", "poor", "fair", "good", "excellent"]
                    .map(String::from)
                    .to_vec(),
            },
            family: Some("hostlelo.perf".into()),
        }],
    )
    .unwrap();
    let answer = RawAnswer::Score {
        score: 2.6,
        probabilities: Some(BTreeMap::from([("3".into(), 0.6), ("2".into(), 0.4)])),
        confidence: None,
    };
    let decision = ask(&engine(answer, "perf"), &request).unwrap();
    record(
        &mut store,
        decision.prediction_traces(&request).remove(0),
        Some("3"),
    );

    let report = store.calibration_report(FamilyFilter::Prefix("hostlelo."));
    assert_eq!(report.family_prefix.as_deref(), Some("hostlelo."));
    assert_eq!(report.unresolved_predictions, 1, "only matching families");
    assert!(report
        .families
        .iter()
        .all(|f| f.family.starts_with("hostlelo.")));
    let perf = report
        .families
        .iter()
        .find(|f| f.family == "hostlelo.perf")
        .unwrap();
    assert!(close(perf.brier, 0.32), "multiclass: {perf:?}");
    assert!(close(perf.top_label_brier, 0.16), "{perf:?}");

    let pooled: Vec<(&str, usize)> = report
        .pooled
        .iter()
        .map(|row| (row.engine.as_str(), row.n))
        .collect();
    assert_eq!(pooled, vec![("fixture@1", 2), ("human:juber", 1)]);
    let engine_pool = &report.pooled[0];
    assert_eq!(engine_pool.family, "hostlelo.*");
    assert_eq!(engine_pool.answer_kind, POOLED_KIND);
    assert_eq!(
        engine_pool.pooled_families,
        vec!["hostlelo.hero.ctr", "hostlelo.perf"]
    );
    // Top-label pairs (0.8, hit) and (0.6, hit).
    assert!(close(engine_pool.brier, 0.1), "{engine_pool:?}");
    assert_eq!(engine_pool.evidence, "anecdotal");
    let human_pool = &report.pooled[1];
    assert!(close(human_pool.brier, 0.04), "{human_pool:?}");
    assert!(close(human_pool.rate, 1.0));
}

#[test]
fn the_policy_sets_how_many_outcomes_make_a_family_measurable() {
    let path = temp_store("min-outcomes");
    let mut store = MemoryStore::open(&path, KernelPolicy::default()).unwrap();
    for (p, count, true_count) in [(0.2, 10, 2), (0.8, 10, 8)] {
        for i in 0..count {
            let observed = if i < true_count { "true" } else { "false" };
            record(
                &mut store,
                engine_noul("hostlelo.hero.ctr", p),
                Some(observed),
            );
        }
    }
    let default = store.calibration(None);
    assert_eq!(default.min_outcomes, MIN_OUTCOMES);
    let family = &default.families[0];
    assert_eq!((family.n, family.evidence), (20, "anecdotal"));
    assert!(
        close(family.brier, 0.16),
        "reported, not hidden: {family:?}"
    );
    assert!(!family.measurable && !family.calibrated);

    let relaxed = KernelPolicy {
        calibration_min_outcomes: 20,
        ..KernelPolicy::default()
    };
    let report = MemoryStore::open_existing(&path, relaxed)
        .unwrap()
        .calibration(None);
    assert_eq!(report.min_outcomes, 20);
    let family = &report.families[0];
    assert_eq!(family.evidence, "measurable");
    assert!(family.measurable && family.calibrated, "{family:?}");
}

fn hikmah(args: &[&str]) -> (bool, Value, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_hikmah"))
        .args(args)
        .env_remove("HIKMAH_POLICY")
        .env_remove("TYPESAFE_API_KEY")
        .output()
        .unwrap();
    let stdout = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    (
        output.status.success(),
        stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn the_cli_records_forecasts_and_pools_their_calibration() {
    let store = temp_store("predict-cli");
    let store = store.to_str().unwrap();
    let predict = |extra: &[&str]| {
        let mut args = vec![
            "predict",
            "--store",
            store,
            "--question",
            "Does the plan-finder hero raise plan clicks?",
            "--source",
            "human:juber",
        ];
        args.extend_from_slice(extra);
        hikmah(&args)
    };
    let outcome = |id: &str, observed: &str| {
        let (ok, _, stderr) = hikmah(&[
            "outcome",
            "--store",
            store,
            "--prediction",
            id,
            "--observed",
            observed,
            "--source",
            "analytics",
        ]);
        assert!(ok, "{stderr}");
    };

    let (ok, noul, stderr) = predict(&[
        "--family",
        "hostlelo.hero.ctr",
        "--type",
        "noul",
        "--p",
        "0.7",
        "--locator",
        "DECISIONS.md#hero",
    ]);
    assert!(ok, "{stderr}");
    assert_eq!(noul["prediction"]["engine"], "human:juber");
    assert_eq!(noul["provenance"]["verified"], false);
    outcome(noul["id"].as_str().unwrap(), "true");
    let (ok, choice, stderr) = predict(&[
        "--family",
        "hostlelo.plan.pick",
        "--type",
        "choice",
        "--p",
        "0.6",
        "--value",
        "starter",
        "--answer-space",
        "starter,pro,business",
    ]);
    assert!(ok, "{stderr}");
    assert_eq!(
        choice["prediction"]["answer_space"],
        serde_json::json!(["starter", "pro", "business"])
    );
    outcome(choice["id"].as_str().unwrap(), "pro");

    let (ok, report, stderr) = hikmah(&[
        "calibration",
        "--store",
        store,
        "--family-prefix",
        "hostlelo.",
    ]);
    assert!(ok, "{stderr}");
    assert_eq!(report["families"].as_array().unwrap().len(), 2);
    let pooled = &report["pooled"][0];
    assert_eq!(pooled["engine"], "human:juber");
    assert_eq!(pooled["n"], 2);
    assert_eq!(pooled["evidence"], "anecdotal");
    // (0.7 − 1)² and (0.6 − 0)², averaged.
    assert!(close(pooled["brier"].as_f64().unwrap(), 0.225), "{pooled}");

    let policy = Path::new(store).with_file_name("policy.json");
    std::fs::write(&policy, r#"{"calibration_min_outcomes": 2}"#).unwrap();
    let (ok, report, stderr) = hikmah(&[
        "--policy",
        policy.to_str().unwrap(),
        "calibration",
        "--store",
        store,
        "--family-prefix",
        "hostlelo.",
    ]);
    assert!(ok, "{stderr}");
    assert_eq!(report["min_outcomes"], 2);
    assert_eq!(report["pooled"][0]["evidence"], "measurable");

    // Refusals: an engine source, a prediction through `remember`, both filters at once.
    let (ok, _, stderr) = hikmah(&[
        "predict",
        "--store",
        store,
        "--family",
        "hostlelo.hero.ctr",
        "--question",
        "Does the hero raise plan clicks?",
        "--type",
        "noul",
        "--p",
        "0.7",
        "--source",
        "model:jev@1",
    ]);
    assert!(!ok && stderr.contains("record them with"), "{stderr}");
    let (ok, _, stderr) = hikmah(&[
        "remember",
        "--store",
        store,
        "--kind",
        "prediction",
        "--content",
        "clicks rise",
    ]);
    assert!(!ok && stderr.contains("hikmah predict"), "{stderr}");
    let (ok, _, _) = hikmah(&[
        "calibration",
        "--store",
        store,
        "--family",
        "a",
        "--family-prefix",
        "b",
    ]);
    assert!(!ok);
}

#[test]
fn the_cli_refuses_to_record_a_decision_without_an_engine() {
    let store = temp_store("decide-cli");
    let frame = store.with_file_name("frame.json");
    std::fs::write(
        &frame,
        r#"{"question": "q", "criteria": [{"id": "value", "weight": 1, "description": "value"}],
            "options": [{"name": "a", "description": "An option.", "evidence_confidence": 0.5}]}"#,
    )
    .unwrap();
    let frame = frame.to_str().unwrap();
    let store = store.to_str().unwrap();
    let (ok, _, stderr) = hikmah(&["decide", "--frame", frame, "--record", "--store", store]);
    assert!(!ok && stderr.contains("--engine"), "{stderr}");
    assert!(!Path::new(store).exists(), "nothing was written");
    let (ok, result, stderr) = hikmah(&["decide", "--frame", frame]);
    assert!(ok, "{stderr}");
    assert_eq!(result["recommended"], "a");
}
