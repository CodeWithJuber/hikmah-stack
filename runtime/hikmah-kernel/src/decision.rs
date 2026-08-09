use crate::error::{KernelError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    pub scores: BTreeMap<String, f64>,
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
    pub evidence_confidence: f64,
    pub reversible: bool,
    pub blocked: bool,
    pub hard_blocks: Vec<String>,
    pub missing_criteria: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionResult {
    pub question: String,
    pub ranking: Vec<RankedOption>,
}

pub fn evaluate(frame: &DecisionFrame) -> Result<DecisionResult> {
    if frame.criteria.is_empty() || frame.options.is_empty() {
        return Err(KernelError::Invalid(
            "decision frame requires at least one criterion and one option".into(),
        ));
    }
    let total_weight: f64 = frame.criteria.iter().map(|c| c.weight.max(0.0)).sum();
    if total_weight <= f64::EPSILON {
        return Err(KernelError::Invalid(
            "decision criteria must contain positive weight".into(),
        ));
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
        let mut observed_weight = 0.0;
        let mut missing = Vec::new();
        for criterion in &frame.criteria {
            match option.scores.get(&criterion.id) {
                Some(score) if (0.0..=1.0).contains(score) => {
                    weighted += score * criterion.weight.max(0.0);
                    observed_weight += criterion.weight.max(0.0);
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
        let coverage = observed_weight / total_weight;
        let raw_score = weighted / total_weight;
        let confidence = option.evidence_confidence * coverage;
        let confidence_adjusted_score = raw_score * (0.5 + 0.5 * confidence);
        ranking.push(RankedOption {
            name: option.name.clone(),
            raw_score,
            confidence_adjusted_score,
            evidence_confidence: confidence,
            reversible: option.reversible,
            blocked: !option.hard_blocks.is_empty(),
            hard_blocks: option.hard_blocks.clone(),
            missing_criteria: missing,
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
    });

    Ok(DecisionResult {
        question: frame.question.clone(),
        ranking,
    })
}
