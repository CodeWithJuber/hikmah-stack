mod common;

use common::temp_store;
use hikmah_kernel::calibration::{spiegelhalter_z, FamilyCalibration, MIN_OUTCOMES};
use hikmah_kernel::decision_port::{
    ask, DecisionRequest, EngineDescriptor, Question, QuestionKind, RawAnswer, StaticEngine,
};
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::trace::{OutcomeRecord, Trace, TraceKind};
use hikmah_kernel::{KernelError, MemoryStore};
use std::collections::BTreeMap;

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

#[test]
fn purged_predictions_do_not_steer_calibration() {
    // Audit finding: `calibration` counted a purged prediction while `gate-threshold` did not.
    let request = noul_request();
    let path = temp_store("calibration-purged");
    let mut store = MemoryStore::open(&path, KernelPolicy::default()).unwrap();
    let predict = |store: &mut MemoryStore, p: f64| {
        let decision = ask(&engine(RawAnswer::Noul { noul: p }, "breaks"), &request).unwrap();
        let prediction = decision.prediction_traces(&request).remove(0);
        store.remember(prediction).unwrap().0.id
    };
    let outcome = |prediction: &str, observed: &str| {
        let mut trace = Trace::new(TraceKind::Outcome, "observed in CI", "ci");
        trace.outcome = Some(OutcomeRecord {
            prediction_id: prediction.to_string(),
            observed: observed.to_string(),
        });
        trace
    };

    let kept = predict(&mut store, 0.2);
    store.remember(outcome(&kept, "false")).unwrap();
    // Retracted after its outcome was recorded: a confident miss that must no longer count.
    let retracted = predict(&mut store, 0.95);
    store.remember(outcome(&retracted, "false")).unwrap();
    store.purge(&retracted, "recorded by mistake").unwrap();
    // Retracted before any outcome: not "unresolved" either.
    let abandoned = predict(&mut store, 0.7);
    store.purge(&abandoned, "wrong question").unwrap();
    let open = predict(&mut store, 0.4);

    let report = store.calibration(None);
    assert_eq!(report.families.len(), 1);
    let family = &report.families[0];
    assert_eq!(
        family.n, 1,
        "only the active, resolved prediction: {family:?}"
    );
    assert!((family.brier - 0.04).abs() < 1e-9, "{family:?}");
    assert_eq!(report.unresolved_predictions, 1, "only {open}");

    // A purged prediction cannot be resolved later, and nothing is written for the attempt.
    let records = store.record_count();
    match store.remember(outcome(&abandoned, "true")) {
        Err(KernelError::Invalid(message)) => assert!(message.contains("not active"), "{message}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(store.record_count(), records);

    // The same holds after reopening the store (status comes from replay, not memory).
    drop(store);
    let reopened = MemoryStore::open_existing(&path, KernelPolicy::default()).unwrap();
    assert_eq!(reopened.calibration(None).families[0].n, 1);
}
