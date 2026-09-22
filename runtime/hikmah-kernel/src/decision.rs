//! Weighted multi-criteria decisions with hard blocks and a reversibility preference.
//!
//! Semantics (3.1.0):
//! - `raw_score` averages only the criteria that have a score (evidence or model estimate);
//!   a missing criterion is *unknown*, not zero.
//! - `coverage` counts only evidence-backed criteria (`scores`). Model-estimated criteria
//!   (`model_scores`, filled through the typed decision port) contribute to `raw_score` but not to
//!   coverage, so a guess never raises confidence.
//! - `confidence_adjusted_score = raw_score × (0.5 + 0.5 × evidence_confidence × coverage)`.
//! - Blocked options always rank after admissible ones; `recommended` is `None` when every
//!   option is blocked.
//! - When the best admissible option is irreversible, evidence is weak, and a reversible option
//!   is within `REVERSIBILITY_BAND`, the reversible option is recommended instead.
//!
//! The 0.5 floor, the band, and the weak-evidence threshold are explicit heuristics.
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
                None => missing.push(criterion.id.clone()),
            }
        }
        let coverage = evidence_weight / total_weight;
        let raw_score = if scored_weight > 0.0 {
            weighted / scored_weight
        } else {
            0.0
        };
        let confidence = option.evidence_confidence * coverage;
        let confidence_adjusted_score = raw_score * (0.5 + 0.5 * confidence);
        ranking.push(RankedOption {
            name: option.name.clone(),
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

    ranking.sort_by(|a, b| {
        a.blocked
            .cmp(&b.blocked)
            .then_with(|| {
                b.confidence_adjusted_score
                    .partial_cmp(&a.confidence_adjusted_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| b.reversible.cmp(&a.reversible))
            .then_with(|| a.name.cmp(&b.name))
    });

    let mut reversibility_preferred = false;
    if let Some(top) = ranking.first() {
        if !top.blocked && !top.reversible && top.evidence_confidence < WEAK_EVIDENCE {
            let cutoff = top.confidence_adjusted_score - REVERSIBILITY_BAND;
            if let Some(index) = ranking
                .iter()
                .position(|o| !o.blocked && o.reversible && o.confidence_adjusted_score >= cutoff)
            {
                let preferred = ranking.remove(index);
                ranking.insert(0, preferred);
                reversibility_preferred = true;
            }
        }
    }

    let recommended = ranking
        .iter()
        .find(|option| !option.blocked)
        .map(|option| option.name.clone());
    Ok(DecisionResult {
        question: frame.question.clone(),
        no_admissible_option: recommended.is_none(),
        recommended,
        ranking,
        reversibility_preferred,
    })
}
