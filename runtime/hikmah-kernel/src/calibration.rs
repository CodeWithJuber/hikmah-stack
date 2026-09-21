//! Calibration earned from outcomes.
//!
//! Pairs every recorded `Prediction` trace with the latest `Outcome` trace that resolves it
//! (written by a non-model principal) and reports, per engine and question family:
//! Brier score, expected calibration error over 5 equal-width bins, accuracy, and base rate.
//! A family counts as calibrated only once it has at least [`MIN_OUTCOMES`] resolved predictions.
use crate::ledger::MemoryStore;
use crate::trace::{PredictionRecord, TraceKind};
use serde::Serialize;
use std::collections::BTreeMap;

pub const MIN_OUTCOMES: usize = 50;
const BINS: usize = 5;

#[derive(Debug, Clone, Serialize)]
pub struct CalibrationBin {
    pub range: [f64; 2],
    pub n: usize,
    pub mean_p: f64,
    pub observed_rate: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FamilyCalibration {
    pub engine: String,
    pub family: String,
    pub answer_kind: String,
    pub n: usize,
    /// Noul: mean (p - y)². Choice/score: multiclass Brier over the reported distribution.
    pub brier: f64,
    /// Brier of always predicting the observed base rate (noul only; lower is better).
    pub baseline_brier: Option<f64>,
    /// Expected calibration error of the reported probability (noul: P(true); others: P(top)).
    pub ece: f64,
    /// Noul: share of outcomes that were `true`. Choice/score: accuracy of the top answer.
    pub rate: f64,
    pub calibrated: bool,
    pub bins: Vec<CalibrationBin>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CalibrationReport {
    pub min_outcomes: usize,
    pub unresolved_predictions: usize,
    pub families: Vec<FamilyCalibration>,
}

struct Pair<'a> {
    prediction: &'a PredictionRecord,
    observed: String,
}

impl MemoryStore {
    pub fn calibration(&self, family: Option<&str>) -> CalibrationReport {
        // Latest outcome per prediction id.
        let mut outcomes: BTreeMap<&str, (u64, &str)> = BTreeMap::new();
        for entry in self.all() {
            let trace = &entry.trace;
            if trace.kind != TraceKind::Outcome || trace.is_model_authored() {
                continue;
            }
            if let Some(outcome) = &trace.outcome {
                let slot = outcomes
                    .entry(outcome.prediction_id.as_str())
                    .or_insert((0, ""));
                if trace.created_at_ms >= slot.0 {
                    *slot = (trace.created_at_ms, outcome.observed.as_str());
                }
            }
        }

        let mut groups: BTreeMap<(String, String, String), Vec<Pair>> = BTreeMap::new();
        let mut unresolved = 0;
        for entry in self.all() {
            let trace = &entry.trace;
            let Some(prediction) = &trace.prediction else {
                continue;
            };
            if family.is_some_and(|f| f != prediction.family) {
                continue;
            }
            match outcomes.get(trace.id.as_str()) {
                Some((_, observed)) => groups
                    .entry((
                        prediction.engine.clone(),
                        prediction.family.clone(),
                        prediction.answer_kind.clone(),
                    ))
                    .or_default()
                    .push(Pair {
                        prediction,
                        observed: observed.trim().to_string(),
                    }),
                None => unresolved += 1,
            }
        }

        let families = groups
            .into_iter()
            .map(|((engine, family, answer_kind), pairs)| {
                summarize(engine, family, answer_kind, &pairs)
            })
            .collect();
        CalibrationReport {
            min_outcomes: MIN_OUTCOMES,
            unresolved_predictions: unresolved,
            families,
        }
    }
}

fn summarize(
    engine: String,
    family: String,
    answer_kind: String,
    pairs: &[Pair],
) -> FamilyCalibration {
    let n = pairs.len();
    let is_noul = answer_kind == "noul";
    // (reported probability, outcome as 0/1) for the ECE / reliability view.
    let points: Vec<(f64, f64)> = pairs
        .iter()
        .map(|pair| {
            if is_noul {
                let y = if pair.observed.eq_ignore_ascii_case("true") {
                    1.0
                } else {
                    0.0
                };
                (pair.prediction.p, y)
            } else {
                let y = if pair.observed == pair.prediction.value {
                    1.0
                } else {
                    0.0
                };
                (pair.prediction.p, y)
            }
        })
        .collect();
    let rate = points.iter().map(|(_, y)| y).sum::<f64>() / n.max(1) as f64;
    let (brier, baseline_brier) = if is_noul {
        let brier = points.iter().map(|(p, y)| (p - y).powi(2)).sum::<f64>() / n.max(1) as f64;
        let base = points.iter().map(|(_, y)| (rate - y).powi(2)).sum::<f64>() / n.max(1) as f64;
        (brier, Some(base))
    } else {
        let brier = pairs
            .iter()
            .map(|pair| {
                let probabilities = &pair.prediction.probabilities;
                if probabilities.is_empty() {
                    let hit = pair.observed == pair.prediction.value;
                    (pair.prediction.p - if hit { 1.0 } else { 0.0 }).powi(2)
                } else {
                    let mut sum: f64 = probabilities
                        .iter()
                        .map(|(k, p)| (p - if *k == pair.observed { 1.0 } else { 0.0 }).powi(2))
                        .sum();
                    if !probabilities.contains_key(&pair.observed) {
                        sum += 1.0;
                    }
                    sum
                }
            })
            .sum::<f64>()
            / n.max(1) as f64;
        (brier, None)
    };

    let mut bins = Vec::new();
    let mut ece = 0.0;
    for b in 0..BINS {
        let lo = b as f64 / BINS as f64;
        let hi = (b + 1) as f64 / BINS as f64;
        let members: Vec<&(f64, f64)> = points
            .iter()
            .filter(|(p, _)| *p >= lo && (*p < hi || (b == BINS - 1 && *p <= hi)))
            .collect();
        if members.is_empty() {
            continue;
        }
        let mean_p = members.iter().map(|(p, _)| p).sum::<f64>() / members.len() as f64;
        let observed_rate = members.iter().map(|(_, y)| y).sum::<f64>() / members.len() as f64;
        ece += members.len() as f64 / n as f64 * (mean_p - observed_rate).abs();
        bins.push(CalibrationBin {
            range: [lo, hi],
            n: members.len(),
            mean_p,
            observed_rate,
        });
    }

    FamilyCalibration {
        engine,
        family,
        answer_kind,
        n,
        brier,
        baseline_brier,
        ece,
        rate,
        calibrated: n >= MIN_OUTCOMES,
        bins,
    }
}
