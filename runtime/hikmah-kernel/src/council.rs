//! Deterministic challenge lanes.
//!
//! Each lane reads one count supplied by the caller (or by a decision engine through the typed
//! port) and reports a severity in `[0, 1]`. A lane at or above `BLOCK_AT` blocks. The risk and
//! human-impact lanes are vetoes: one irreversible action or one unresolved human-impact
//! question blocks on its own, so those concerns are never averaged away. The other lanes
//! scale with their count up to a per-lane limit.
use serde::{Deserialize, Serialize};

pub const BLOCK_AT: f32 = 0.8;

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
    pub veto: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouncilResult {
    pub can_proceed: bool,
    pub blocking_lanes: Vec<Lane>,
    pub signals: Vec<LaneSignal>,
}

pub fn deliberate(input: &DeliberationInput) -> CouncilResult {
    let signals = vec![
        LaneSignal {
            lane: Lane::Evidence,
            severity: ratio(input.unverified_consequential_claims, 3),
            veto: false,
            message: format!(
                "{} consequential claims still lack verification (blocks at 3)",
                input.unverified_consequential_claims
            ),
        },
        LaneSignal {
            lane: Lane::Memory,
            severity: ratio(input.memory_conflicts, 2),
            veto: false,
            message: format!(
                "{} unresolved memory conflicts (blocks at 2)",
                input.memory_conflicts
            ),
        },
        LaneSignal {
            lane: Lane::Risk,
            severity: veto(input.irreversible_actions),
            veto: true,
            message: format!(
                "{} irreversible actions in scope (any one blocks until a named owner accepts it)",
                input.irreversible_actions
            ),
        },
        LaneSignal {
            lane: Lane::HumanImpact,
            severity: veto(input.unresolved_human_impact_questions),
            veto: true,
            message: format!(
                "{} unresolved human-impact questions (any one blocks)",
                input.unresolved_human_impact_questions
            ),
        },
        LaneSignal {
            lane: Lane::Delivery,
            severity: ratio(input.missing_acceptance_criteria, 3),
            veto: false,
            message: format!(
                "{} acceptance criteria are still missing (blocks at 3)",
                input.missing_acceptance_criteria
            ),
        },
    ];
    let blocking_lanes: Vec<Lane> = signals
        .iter()
        .filter(|signal| signal.severity >= BLOCK_AT)
        .map(|signal| signal.lane)
        .collect();
    CouncilResult {
        can_proceed: blocking_lanes.is_empty(),
        blocking_lanes,
        signals,
    }
}

fn ratio(value: usize, blocking_at: usize) -> f32 {
    (value as f32 / blocking_at.max(1) as f32).clamp(0.0, 1.0)
}

fn veto(value: usize) -> f32 {
    if value > 0 {
        1.0
    } else {
        0.0
    }
}
