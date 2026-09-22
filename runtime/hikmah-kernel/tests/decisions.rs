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

fn four_criteria() -> Vec<Criterion> {
    ["safety", "cost", "speed", "quality"]
        .iter()
        .map(|id| criterion(id, 0.25))
        .collect()
}

#[test]
fn one_good_score_and_three_unknowns_cannot_beat_full_evidence() {
    // Audit probe P4: A is 0.6 on all four criteria; B has only speed = 1.0 (safety unknown).
    // B used to win with an adjusted 0.625 against 0.6.
    let frame = DecisionFrame {
        question: "q".into(),
        criteria: four_criteria(),
        options: vec![
            option(
                "a",
                &[
                    ("safety", 0.6),
                    ("cost", 0.6),
                    ("speed", 0.6),
                    ("quality", 0.6),
                ],
                0.9,
                false,
            ),
            option("b", &[("speed", 1.0)], 0.9, false),
        ],
    };
    let result = evaluate(&frame).unwrap();
    assert_eq!(result.recommended.as_deref(), Some("a"));
    let a = &result.ranking[0];
    let b = &result.ranking[1];
    assert_eq!(a.score_interval, [0.6, 0.6]);
    assert!((b.score_interval[0] - 0.25).abs() < 1e-9);
    assert!((b.score_interval[1] - 1.0).abs() < 1e-9);
    assert_eq!(b.missing_criteria, vec!["safety", "cost", "quality"]);
    // B could still turn out better once its unknowns are measured.
    assert!(!result.decisive);
}

#[test]
fn decisive_only_when_no_unknown_could_change_the_winner() {
    let frame = DecisionFrame {
        question: "q".into(),
        criteria: four_criteria(),
        options: vec![
            option(
                "strong",
                &[
                    ("safety", 0.9),
                    ("cost", 0.9),
                    ("speed", 0.8),
                    ("quality", 0.8),
                ],
                0.9,
                true,
            ),
            // Best case 0.25 * 0.2 + 0.75 = 0.8 < 0.85.
            option("weak", &[("safety", 0.2)], 0.9, true),
        ],
    };
    let result = evaluate(&frame).unwrap();
    assert_eq!(result.recommended.as_deref(), Some("strong"));
    assert!(result.decisive);

    // Ties on the lower bound go to the wider upper bound; blocked options never count.
    let mut blocked = option(
        "blocked",
        &[
            ("safety", 1.0),
            ("cost", 1.0),
            ("speed", 1.0),
            ("quality", 1.0),
        ],
        1.0,
        true,
    );
    blocked.hard_blocks = vec!["no consent".into()];
    let frame = DecisionFrame {
        question: "q".into(),
        criteria: four_criteria(),
        options: vec![
            option("narrow", &[("safety", 0.4), ("cost", 0.4)], 0.9, true),
            option("wide", &[("safety", 0.8)], 0.9, true),
            blocked,
        ],
    };
    let result = evaluate(&frame).unwrap();
    let names: Vec<&str> = result.ranking.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, vec!["wide", "narrow", "blocked"]);
    assert!(!result.decisive);
}

#[test]
fn model_estimates_are_points_in_the_interval_not_evidence() {
    let mut estimated = option("estimated", &[("safety", 0.8)], 0.9, true);
    estimated.model_scores.insert("cost".into(), 0.6);
    let frame = DecisionFrame {
        question: "q".into(),
        criteria: vec![
            criterion("safety", 0.5),
            criterion("cost", 0.3),
            criterion("speed", 0.2),
        ],
        options: vec![estimated],
    };
    let ranked = &evaluate(&frame).unwrap().ranking[0];
    assert!((ranked.score_interval[0] - 0.58).abs() < 1e-9);
    assert!((ranked.score_interval[1] - 0.78).abs() < 1e-9);
    assert!((ranked.coverage - 0.5).abs() < 1e-9);
}
