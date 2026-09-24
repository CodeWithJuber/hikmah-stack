//! `decide --engine`: which options reach the engine, how questions are batched into requests,
//! and how the admitted estimates are recorded. The engine is a mock that records every request.
mod common;

use common::{line_count, temp_store};
use hikmah_kernel::decision::{
    estimate_missing_criteria, evaluate, Criterion, DecisionFrame, DecisionOption,
};
use hikmah_kernel::decision_port::{
    DecisionEngine, DecisionRequest, EngineDescriptor, RawAnswer, RawDecision, MAX_QUESTIONS,
    MAX_STATE_CHARS,
};
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::trace::TraceKind;
use hikmah_kernel::MemoryStore;
use std::cell::RefCell;
use std::collections::BTreeMap;

/// Answers every score question with level 3 of 0..=4 ("good", normalized 0.75) and keeps a
/// copy of every request it was sent.
#[derive(Default)]
struct Recording {
    requests: RefCell<Vec<DecisionRequest>>,
}

impl DecisionEngine for Recording {
    fn descriptor(&self) -> EngineDescriptor {
        EngineDescriptor {
            name: "fixture".into(),
            version: "1".into(),
        }
    }

    fn decide(&self, request: &DecisionRequest) -> hikmah_kernel::Result<RawDecision> {
        self.requests.borrow_mut().push(request.clone());
        Ok(RawDecision {
            request_id: request.request_id(),
            engine: self.descriptor(),
            answers: request
                .questions
                .iter()
                .map(|q| {
                    (
                        q.id.clone(),
                        RawAnswer::Score {
                            score: 3.0,
                            probabilities: Some(BTreeMap::from([("3".into(), 1.0)])),
                            confidence: None,
                        },
                    )
                })
                .collect(),
            latency_ms: 0,
        })
    }
}

fn criterion(id: &str, weight: f64) -> Criterion {
    Criterion {
        id: id.into(),
        weight,
        description: format!("{id} of the layout"),
    }
}

fn option(name: &str, description: Option<&str>, scores: &[(&str, f64)]) -> DecisionOption {
    DecisionOption {
        name: name.into(),
        description: description.map(str::to_string),
        scores: scores.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        model_scores: BTreeMap::new(),
        evidence_confidence: 0.6,
        hard_blocks: vec![],
        reversible: true,
    }
}

/// The review's HostLelo frame: one partly scored option, one fully scored option, and one
/// hard-blocked option with nothing scored.
fn hero_frame() -> DecisionFrame {
    let mut video = option(
        "video-hero",
        Some("Autoplay background video with a single CTA."),
        &[],
    );
    video.hard_blocks = vec!["autoplay video violates the motion rules".into()];
    DecisionFrame {
        question: "Which hero layout should ship?".into(),
        criteria: vec![
            criterion("clarity", 0.4),
            criterion("perf", 0.3),
            criterion("brand", 0.3),
        ],
        options: vec![
            option(
                "plan-finder-hero",
                Some("Hero with an inline 3-question plan finder."),
                &[("clarity", 0.7)],
            ),
            option(
                "price-table-hero",
                Some("Hero showing the three cheapest plans."),
                &[("clarity", 0.8), ("perf", 0.6), ("brand", 0.7)],
            ),
            video,
        ],
    }
}

#[test]
fn hard_blocked_options_are_never_sent_and_the_rest_share_one_request() {
    let engine = Recording::default();
    let mut frame = hero_frame();
    let estimation = estimate_missing_criteria(&engine, &mut frame).unwrap();

    let requests = engine.requests.borrow();
    assert_eq!(requests.len(), 1, "one request for every option");
    let ids: Vec<&str> = requests[0]
        .questions
        .iter()
        .map(|q| q.id.as_str())
        .collect();
    assert_eq!(ids, vec!["o0_c1", "o0_c2"]);
    assert!(
        !requests[0].state.contains("video-hero")
            && requests[0]
                .questions
                .iter()
                .all(|q| !q.instructions.contains("video-hero")),
        "a hard-blocked option can never be recommended, so it is never asked about"
    );
    assert!(requests[0].state.contains("plan-finder-hero"));
    assert_eq!(estimation.skipped_blocked, vec!["video-hero"]);
    assert_eq!(estimation.exchanges.len(), 1);

    assert_eq!(estimation.estimates.len(), 2);
    for estimate in &estimation.estimates {
        assert_eq!(estimate.option, "plan-finder-hero");
        assert_eq!(estimate.score, Some(0.75));
        assert_eq!(estimate.engine, "model:fixture@1");
        assert!(!estimate.calibrated && estimate.reason.is_none());
    }
    assert_eq!(
        frame.options[0].model_scores,
        BTreeMap::from([("brand".to_string(), 0.75), ("perf".to_string(), 0.75)])
    );
    assert!(frame.options[2].model_scores.is_empty());

    let result = evaluate(&frame).unwrap();
    let blocked = result
        .ranking
        .iter()
        .find(|o| o.name == "video-hero")
        .unwrap();
    assert!(blocked.blocked && blocked.model_estimated_criteria.is_empty());

    // Recorded as unverified engine predictions, one family per criterion.
    let traces = estimation.prediction_traces();
    let families: Vec<&str> = traces
        .iter()
        .map(|t| t.prediction.as_ref().unwrap().family.as_str())
        .collect();
    assert_eq!(families, vec!["decide.perf", "decide.brand"]);
    assert!(traces
        .iter()
        .all(|t| t.is_model_authored() && !t.provenance.verified));
}

#[test]
fn requests_are_split_only_at_the_question_limit() {
    let criteria: Vec<Criterion> = (0..12).map(|j| criterion(&format!("k{j}"), 1.0)).collect();
    let options: Vec<DecisionOption> = (0..3)
        .map(|i| {
            let name = format!("layout-{i}");
            let description = format!("Candidate layout number {i}.");
            option(&name, Some(&description), &[])
        })
        .collect();
    let mut frame = DecisionFrame {
        question: "Which layout?".into(),
        criteria,
        options,
    };
    let engine = Recording::default();
    let estimation = estimate_missing_criteria(&engine, &mut frame).unwrap();

    let requests = engine.requests.borrow();
    let sizes: Vec<usize> = requests.iter().map(|r| r.questions.len()).collect();
    assert_eq!(sizes, vec![MAX_QUESTIONS, 36 - MAX_QUESTIONS]);
    // The second request only carries the option its questions are about.
    assert!(requests[1].state.contains("layout-2") && !requests[1].state.contains("layout-0"));
    assert!(requests[1]
        .questions
        .iter()
        .all(|q| q.id.starts_with("o2_")));
    assert_eq!(estimation.estimates.len(), 36);
    assert!(estimation.estimates.iter().all(|e| e.score == Some(0.75)));
    assert_eq!(estimation.prediction_traces().len(), 36);
}

#[test]
fn requests_are_split_where_the_state_would_pass_its_limit() {
    // Two long descriptions cannot share one request's state; a short one still joins the second.
    let long = |i: usize| {
        let sentence = format!("Layout {i} keeps the plan finder above the fold. ");
        sentence.repeat(MAX_STATE_CHARS / 2 / sentence.len() + 1)
    };
    let (first, second) = (long(0), long(1));
    assert!(first.chars().count() < MAX_STATE_CHARS / 2 + 100);
    let mut frame = DecisionFrame {
        question: "Which layout?".into(),
        criteria: vec![criterion("clarity", 0.5), criterion("perf", 0.5)],
        options: vec![
            option("layout-0", Some(&first), &[]),
            option("layout-1", Some(&second), &[]),
            option("layout-2", Some("A short third layout."), &[]),
        ],
    };
    let engine = Recording::default();
    let estimation = estimate_missing_criteria(&engine, &mut frame).unwrap();

    let requests = engine.requests.borrow();
    let ids: Vec<Vec<&str>> = requests
        .iter()
        .map(|r| r.questions.iter().map(|q| q.id.as_str()).collect())
        .collect();
    assert_eq!(
        ids,
        vec![
            vec!["o0_c0", "o0_c1"],
            vec!["o1_c0", "o1_c1", "o2_c0", "o2_c1"]
        ],
        "split only where the next option's line would not fit"
    );
    for request in requests.iter() {
        request.validate().unwrap();
        assert!(request.state.chars().count() <= MAX_STATE_CHARS);
    }
    // Each request's state carries only the options it asks about.
    assert!(requests[0].state.contains("layout-0") && !requests[0].state.contains("layout-1"));
    assert!(!requests[1].state.contains("layout-0"));
    assert!(requests[1].state.contains("layout-1") && requests[1].state.contains("layout-2"));
    assert_eq!(estimation.estimates.len(), 6);
    assert_eq!(estimation.exchanges.len(), 2);
}

#[test]
fn nothing_is_sent_for_an_invalid_frame_or_when_nothing_is_missing() {
    let engine = Recording::default();

    let mut complete = hero_frame();
    complete.options.retain(|o| o.name == "price-table-hero");
    let estimation = estimate_missing_criteria(&engine, &mut complete).unwrap();
    assert!(estimation.estimates.is_empty() && estimation.exchanges.is_empty());

    let mut duplicate = hero_frame();
    duplicate.options[1].name = "plan-finder-hero".into();
    assert!(estimate_missing_criteria(&engine, &mut duplicate).is_err());

    assert!(engine.requests.borrow().is_empty());
}

#[test]
fn recorded_estimates_are_written_in_one_batch() {
    let engine = Recording::default();
    let mut frame = hero_frame();
    let estimation = estimate_missing_criteria(&engine, &mut frame).unwrap();

    let path = temp_store("decide-record");
    let mut store = MemoryStore::open(&path, KernelPolicy::default()).unwrap();
    let recorded = store.remember_many(estimation.prediction_traces()).unwrap();
    assert_eq!(recorded.len(), 2);
    assert_ne!(recorded[0].0.id, recorded[1].0.id);
    assert_eq!(store.record_count(), 2);
    assert_eq!(line_count(&path), 2);

    let reopened = MemoryStore::open_existing(&path, KernelPolicy::default()).unwrap();
    assert_eq!(
        reopened
            .all()
            .filter(|e| e.trace.kind == TraceKind::Prediction)
            .count(),
        2
    );
}
