use serde::{Deserialize, Serialize};
use std::thread;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliberationInput {
    pub unverified_consequential_claims: usize,
    pub memory_conflicts: usize,
    pub irreversible_actions: usize,
    pub unresolved_human_impact_questions: usize,
    pub missing_acceptance_criteria: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Evidence,
    Memory,
    Risk,
    HumanImpact,
    Delivery,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaneSignal {
    pub lane: Lane,
    pub severity: f32,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouncilResult {
    pub can_proceed: bool,
    pub signals: Vec<LaneSignal>,
}

pub fn deliberate(input: &DeliberationInput) -> CouncilResult {
    let signals = thread::scope(|scope| {
        let evidence = scope.spawn(|| LaneSignal {
            lane: Lane::Evidence,
            severity: ratio(input.unverified_consequential_claims, 3),
            message: format!(
                "{} consequential claims still lack verification",
                input.unverified_consequential_claims
            ),
        });
        let memory = scope.spawn(|| LaneSignal {
            lane: Lane::Memory,
            severity: ratio(input.memory_conflicts, 2),
            message: format!("{} unresolved memory conflicts", input.memory_conflicts),
        });
        let risk = scope.spawn(|| LaneSignal {
            lane: Lane::Risk,
            severity: ratio(input.irreversible_actions, 2),
            message: format!(
                "{} irreversible actions in scope",
                input.irreversible_actions
            ),
        });
        let human_impact = scope.spawn(|| LaneSignal {
            lane: Lane::HumanImpact,
            severity: ratio(input.unresolved_human_impact_questions, 2),
            message: format!(
                "{} unresolved human-impact questions",
                input.unresolved_human_impact_questions
            ),
        });
        let delivery = scope.spawn(|| LaneSignal {
            lane: Lane::Delivery,
            severity: ratio(input.missing_acceptance_criteria, 3),
            message: format!(
                "{} acceptance criteria are still missing",
                input.missing_acceptance_criteria
            ),
        });

        vec![
            evidence.join().expect("evidence lane panicked"),
            memory.join().expect("memory lane panicked"),
            risk.join().expect("risk lane panicked"),
            human_impact.join().expect("human-impact lane panicked"),
            delivery.join().expect("delivery lane panicked"),
        ]
    });

    let can_proceed = signals.iter().all(|signal| signal.severity < 0.8);
    CouncilResult {
        can_proceed,
        signals,
    }
}

fn ratio(value: usize, blocking_at: usize) -> f32 {
    (value as f32 / blocking_at.max(1) as f32).clamp(0.0, 1.0)
}
