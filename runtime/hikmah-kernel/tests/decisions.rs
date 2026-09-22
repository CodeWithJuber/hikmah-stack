use hikmah_kernel::council::{deliberate, DeliberationInput, Lane};
use hikmah_kernel::decision::{evaluate, Criterion, DecisionFrame, DecisionOption};
use std::collections::BTreeMap;

fn criterion(id: &str, weight: f64) -> Criterion {
    Criterion {
        id: id.into(),
        weight,
        description: id.into(),
    }
}

fn option(name: &str, scores: &[(&str, f64)], confidence: f64, reversible: bool) -> DecisionOption {
    DecisionOption {
        name: name.into(),
        description: None,
        scores: scores.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        model_scores: BTreeMap::new(),
        evidence_confidence: confidence,
        hard_blocks: vec![],
        reversible,
    }
}

#[test]
fn a_missing_criterion_is_unknown_not_zero() {
    let frame = DecisionFrame {
        question: "q".into(),
        criteria: vec![criterion("safety", 0.5), criterion("cost", 0.5)],
        options: vec![
            option("a", &[("safety", 0.0), ("cost", 0.9)], 0.9, false),
            option("b", &[("cost", 0.9)], 0.9, false),
        ],
    };
    let result = evaluate(&frame).unwrap();
    let b = result.ranking.iter().find(|o| o.name == "b").unwrap();
    assert!((b.raw_score - 0.9).abs() < 1e-9);
    assert_eq!(b.missing_criteria, vec!["safety"]);
    assert!((b.coverage - 0.5).abs() < 1e-9);
}

#[test]
fn model_estimates_rank_but_never_raise_coverage() {
    let mut estimated = option("estimated", &[("cost", 0.8)], 0.9, true);
    estimated.model_scores.insert("safety".into(), 0.9);
    let frame = DecisionFrame {
        question: "q".into(),
        criteria: vec![criterion("safety", 0.5), criterion("cost", 0.5)],
        options: vec![estimated],
    };
    let result = evaluate(&frame).unwrap();
    let ranked = &result.ranking[0];
    assert!((ranked.raw_score - 0.85).abs() < 1e-9);
    assert!((ranked.coverage - 0.5).abs() < 1e-9);
    assert_eq!(ranked.model_estimated_criteria, vec!["safety"]);
}

#[test]
fn reversible_option_wins_a_close_call_on_weak_evidence() {
    let frame = DecisionFrame {
        question: "q".into(),
        criteria: vec![criterion("value", 1.0)],
        options: vec![
            option("big-bang", &[("value", 0.80)], 0.5, false),
            option("canary", &[("value", 0.79)], 0.5, true),
        ],
    };
    let result = evaluate(&frame).unwrap();
    assert_eq!(result.recommended.as_deref(), Some("canary"));
    assert!(result.reversibility_preferred);
}

#[test]
fn all_blocked_means_no_recommendation() {
    let mut a = option("a", &[("value", 0.9)], 0.9, true);
    a.hard_blocks = vec!["privacy review missing".into()];
    let frame = DecisionFrame {
        question: "q".into(),
        criteria: vec![criterion("value", 1.0)],
        options: vec![a],
    };
    let result = evaluate(&frame).unwrap();
    assert!(result.no_admissible_option);
    assert!(result.recommended.is_none());
}

#[test]
fn invalid_frames_are_rejected() {
    for criteria in [
        vec![criterion("harm", -1.0), criterion("value", 1.0)],
        vec![criterion("value", f64::INFINITY)],
        vec![criterion("value", 1.0), criterion("value", 1.0)],
        vec![criterion("a", 1e308), criterion("b", 1e308)],
    ] {
        let frame = DecisionFrame {
            question: "q".into(),
            criteria,
            options: vec![option("a", &[], 0.5, true)],
        };
        assert!(evaluate(&frame).is_err());
    }
}

#[test]
fn risk_and_human_impact_lanes_veto_on_a_single_item() {
    let result = deliberate(&DeliberationInput {
        unverified_consequential_claims: 0,
        memory_conflicts: 0,
        irreversible_actions: 1,
        unresolved_human_impact_questions: 0,
        missing_acceptance_criteria: 0,
    });
    assert!(!result.can_proceed);
    assert_eq!(result.blocking_lanes, vec![Lane::Risk]);

    let result = deliberate(&DeliberationInput {
        unverified_consequential_claims: 2,
        memory_conflicts: 1,
        irreversible_actions: 0,
        unresolved_human_impact_questions: 0,
        missing_acceptance_criteria: 2,
    });
    assert!(result.can_proceed);
}
