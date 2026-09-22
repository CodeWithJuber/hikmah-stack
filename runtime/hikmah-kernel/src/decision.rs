//! Weighted multi-criteria decisions with hard blocks and a reversibility preference.
//!
//! Semantics:
//! - A criterion with no score (neither evidence nor a model estimate) is *unknown*: it could
//!   be anywhere on the score scale [0, 1]. It is not zero, and it is not the average of the
//!   known criteria (that would be a guess).
//! - `score_interval = [lo, hi]` is interval arithmetic over the weighted mean of every
//!   criterion: `lo` puts each unknown criterion at the scale minimum (0) and `hi` at the scale
//!   maximum (1), so `hi − lo` is the weight share still unknown. No prior is invented.
//! - Admissible options rank by `lo` (the score they are guaranteed on the stated scores), then
//!   by `hi`, then reversible first, then name. An option with one excellent score and three
//!   unknowns therefore cannot outrank a fully evidenced option whose guaranteed score is higher.
//! - `evidence_interval` is the same interval over evidence alone: model-estimated criteria
//!   count as unknown there.
//! - `decisive` is true only when the recommended option's evidence `lo` is strictly greater
//!   than every other admissible option's evidence `hi`: no resolution of the unknowns, and no
//!   engine estimate proving wrong, could change the winner. Exact ties are not decisive.
//! - `coverage` counts only evidence-backed criteria (`scores`). Model-estimated criteria
//!   (`model_scores`, filled through the typed decision port) are point values in
//!   `score_interval`, so they can reorder the ranking, but they never raise coverage and never
//!   make a result decisive.
//! - `raw_score` (mean over scored criteria) and
//!   `confidence_adjusted_score = raw_score × (0.5 + 0.5 × evidence_confidence × coverage)` are
//!   still reported for comparison with 3.1.0; they no longer order the ranking.
//! - Blocked options always rank after admissible ones; `recommended` is `None` when every
//!   option is blocked.
//! - When the best admissible option is irreversible, evidence is weak, and a reversible option's
//!   `lo` is within `REVERSIBILITY_BAND` of the best `lo`, the reversible option is recommended.
//!
//! The band and the weak-evidence threshold are explicit heuristics.
use crate::error::{KernelError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const REVERSIBILITY_BAND: f64 = 0.02;
pub const WEAK_EVIDENCE: f64 = 0.7;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionFrame {
    pub question: String,
    pub criteria: Vec<Criterion>,
    pub options: Vec<DecisionOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Criterion {
    pub id: String,
    pub weight: f64,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionOption {
    pub name: String,
    /// Optional free-text description; a decision engine can estimate missing criteria from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub scores: BTreeMap<String, f64>,
    /// Scores estimated by a decision engine. Used for ranking, never for coverage.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub model_scores: BTreeMap<String, f64>,
    pub evidence_confidence: f64,
    #[serde(default)]
    pub hard_blocks: Vec<String>,
    #[serde(default)]
    pub reversible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedOption {
    pub name: String,
    /// `[lo, hi]`: the weighted score with every unscored criterion at 0 and at 1.
    pub score_interval: [f64; 2],
    /// `[lo, hi]` over evidence alone: model-estimated criteria are unknown here, like unscored
    /// ones. `decisive` is computed from this interval, so an engine guess can reorder options
    /// but can never make a result look settled.
    #[serde(default)]
    pub evidence_interval: [f64; 2],
    pub raw_score: f64,
    pub confidence_adjusted_score: f64,
    /// `evidence_confidence × coverage`.
    pub evidence_confidence: f64,
    pub coverage: f64,
    pub reversible: bool,
    pub blocked: bool,
    pub hard_blocks: Vec<String>,
    pub missing_criteria: Vec<String>,
    #[serde(default)]
    pub model_estimated_criteria: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionResult {
    pub question: String,
    pub ranking: Vec<RankedOption>,
    /// Best admissible option, or `None` when every option is blocked.
    pub recommended: Option<String>,
    pub no_admissible_option: bool,
    /// True when the reversibility preference changed the recommendation.
    pub reversibility_preferred: bool,
    /// True only when the recommended option's lower bound is at least every other admissible
    /// option's upper bound, so no value of the unknown criteria could change the winner.
    pub decisive: bool,
}

pub fn evaluate(frame: &DecisionFrame) -> Result<DecisionResult> {
    if frame.criteria.is_empty() || frame.options.is_empty() {
        return Err(KernelError::Invalid(
            "decision frame requires at least one criterion and one option".into(),
        ));
    }
    let mut ids = BTreeSet::new();
    for criterion in &frame.criteria {
        if !criterion.weight.is_finite() || criterion.weight < 0.0 {
            return Err(KernelError::Invalid(format!(
                "criterion {} needs a finite, non-negative weight",
                criterion.id
            )));
        }
        if !ids.insert(criterion.id.as_str()) {
            return Err(KernelError::Invalid(format!(
                "duplicate criterion id: {}",
                criterion.id
            )));
        }
    }
    let total_weight: f64 = frame.criteria.iter().map(|c| c.weight).sum();
    if !total_weight.is_finite() || total_weight <= f64::EPSILON {
        return Err(KernelError::Invalid(
            "decision criteria must contain positive, finite total weight".into(),
        ));
    }
    let mut names = BTreeSet::new();
    for option in &frame.options {
        if !names.insert(option.name.as_str()) {
            return Err(KernelError::Invalid(format!(
                "duplicate option name: {}",
                option.name
            )));
        }
        for id in option.scores.keys().chain(option.model_scores.keys()) {
            if !ids.contains(id.as_str()) {
                return Err(KernelError::Invalid(format!(
                    "option {} scores unknown criterion {id}",
                    option.name
                )));
            }
        }
    }

    let mut ranking = Vec::new();
    for option in &frame.options {
        if !(0.0..=1.0).contains(&option.evidence_confidence) {
            return Err(KernelError::Invalid(format!(
                "evidence_confidence for {} must be between 0 and 1",
                option.name
            )));
        }
        let mut weighted = 0.0;
        let mut scored_weight = 0.0;
        let mut evidence_weight = 0.0;
        let mut evidence_sum = 0.0;
        let mut missing_weight = 0.0;
        let mut missing = Vec::new();
        let mut estimated = Vec::new();
        for criterion in &frame.criteria {
            let (score, is_evidence) = match (
                option.scores.get(&criterion.id),
                option.model_scores.get(&criterion.id),
            ) {
                (Some(score), _) => (Some(*score), true),
                (None, Some(score)) => (Some(*score), false),
                (None, None) => (None, false),
            };
            match score {
                Some(score) if (0.0..=1.0).contains(&score) => {
                    weighted += score * criterion.weight;
                    scored_weight += criterion.weight;
                    if is_evidence {
                        evidence_weight += criterion.weight;
                        evidence_sum += score * criterion.weight;
                    } else {
                        estimated.push(criterion.id.clone());
                    }
                }
                Some(_) => {
                    return Err(KernelError::Invalid(format!(
                        "score for {} / {} must be between 0 and 1",
                        option.name, criterion.id
                    )));
                }
                None => {
                    missing_weight += criterion.weight;
                    missing.push(criterion.id.clone());
                }
            }
        }
        let coverage = evidence_weight / total_weight;
        let score_interval = [
            (weighted / total_weight).clamp(0.0, 1.0),
            ((weighted + missing_weight) / total_weight).clamp(0.0, 1.0),
        ];
        let evidence_interval = [
            (evidence_sum / total_weight).clamp(0.0, 1.0),
            ((evidence_sum + total_weight - evidence_weight) / total_weight).clamp(0.0, 1.0),
        ];
        let raw_score = if scored_weight > 0.0 {
            weighted / scored_weight
        } else {
            0.0
        };
        let confidence = option.evidence_confidence * coverage;
        let confidence_adjusted_score = raw_score * (0.5 + 0.5 * confidence);
        ranking.push(RankedOption {
            name: option.name.clone(),
            score_interval,
            evidence_interval,
            raw_score,
            confidence_adjusted_score,
            evidence_confidence: confidence,
            coverage,
            reversible: option.reversible,
            blocked: !option.hard_blocks.is_empty(),
            hard_blocks: option.hard_blocks.clone(),
            missing_criteria: missing,
            model_estimated_criteria: estimated,
        });
    }

    let by = |x: f64, y: f64| y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal);
    ranking.sort_by(|a, b| {
        a.blocked
            .cmp(&b.blocked)
            .then_with(|| by(a.score_interval[0], b.score_interval[0]))
            .then_with(|| by(a.score_interval[1], b.score_interval[1]))
            .then_with(|| b.reversible.cmp(&a.reversible))
            .then_with(|| a.name.cmp(&b.name))
    });

    let mut reversibility_preferred = false;
    if let Some(top) = ranking.first() {
        if !top.blocked && !top.reversible && top.evidence_confidence < WEAK_EVIDENCE {
            let cutoff = top.score_interval[0] - REVERSIBILITY_BAND;
            if let Some(index) = ranking
                .iter()
                .position(|o| !o.blocked && o.reversible && o.score_interval[0] >= cutoff)
            {
                let preferred = ranking.remove(index);
                ranking.insert(0, preferred);
                reversibility_preferred = true;
            }
        }
    }

    let top = ranking.iter().find(|option| !option.blocked);
    let recommended = top.map(|option| option.name.clone());
    let decisive = top.is_some_and(|top| {
        ranking
            .iter()
            .filter(|other| !other.blocked && other.name != top.name)
            // Strict: on an exact tie only the name tie-break separates them.
            .all(|other| top.evidence_interval[0] > other.evidence_interval[1])
    });
    Ok(DecisionResult {
        question: frame.question.clone(),
        no_admissible_option: recommended.is_none(),
        recommended,
        ranking,
        reversibility_preferred,
        decisive,
    })
}
