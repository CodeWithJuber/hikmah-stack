//! Calibration earned from outcomes.
//!
//! Pairs every *active* `Prediction` trace (a purged or superseded prediction was retracted and
//! does not count) with the latest *active* `Outcome` trace that resolves it (written by a
//! non-model principal; purged or superseded outcomes do not count) and reports,
//! per engine and question family: Brier score, expected calibration error over 5 equal-width
//! bins, accuracy or base rate, and how many predictions carried no probability at all.
//!
//! A family is `measurable` once it has at least [`MIN_OUTCOMES`] scored predictions. Sample size
//! is necessary, not sufficient: it is reported `calibrated` only when, in addition,
//!
//! 1. Spiegelhalter's Z test does not reject calibration at alpha = 0.05:
//!    `Z = Σ (y − p)(1 − 2p) / sqrt(Σ (1 − 2p)² p (1 − p))`, `|Z| < 1.96`
//!    (two-sided p-value from the normal approximation), and
//! 2. the Brier score beats always predicting the observed base rate:
//!    `brier_skill = 1 − Brier / Brier(base rate) > 0`.
//!
//! Noul families use `p = P(true)` and `y = [observed is true]`. Choice and score families use the
//! top-label view for both checks: `p` = probability of the reported answer and `y = [observed ==
//! reported]`, with the binary Brier of that pair against the top-label accuracy base rate. When
//! Z or the skill is undefined (zero variance, for example every `p` in {0, 0.5, 1}, or a base rate
//! of 0 or 1) the family is not reported calibrated.
use crate::ledger::MemoryStore;
use crate::trace::{PredictionRecord, TraceKind, TraceStatus};
use serde::Serialize;
use std::collections::BTreeMap;

pub const MIN_OUTCOMES: usize = 50;
const BINS: usize = 5;
/// Two-sided critical value of the standard normal at alpha = 0.05.
pub const Z_CRITICAL: f64 = 1.96;

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
    /// Resolved predictions that carried a probability.
    pub n: usize,
    /// Resolved predictions without any reported probability (excluded from the metrics).
    pub unscored: usize,
    /// Noul: mean (p - y)². Choice/score: multiclass Brier over the reported distribution.
    pub brier: f64,
    /// Brier of always predicting the observed base rate (noul only; lower is better).
    pub baseline_brier: Option<f64>,
    /// Expected calibration error of the reported probability (noul: P(true); others: P(top)).
    pub ece: f64,
    /// Noul: share of outcomes that were `true`. Choice/score: accuracy of the top answer.
    pub rate: f64,
    /// Spiegelhalter's Z for the reported probability (noul: P(true); others: P(top)).
    /// `None` when its variance is zero.
    pub z: Option<f64>,
    /// Two-sided p-value of `z` (normal approximation). Small values reject calibration.
    pub p_value: Option<f64>,
    /// `1 − Brier / Brier(base rate)` (choice/score: top-label binary Brier). Positive means the
    /// probabilities beat always predicting the base rate. `None` when the base rate is 0 or 1.
    pub brier_skill: Option<f64>,
    /// At least [`MIN_OUTCOMES`] scored predictions: enough data to judge calibration.
    pub measurable: bool,
    /// Measurable, `|z| < 1.96`, and `brier_skill > 0`.
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
    p: f64,
    observed: String,
}

impl MemoryStore {
    /// Latest active outcome per prediction id, written by a non-model principal:
    /// `prediction id -> (created_at_ms, observed)`.
    pub(crate) fn latest_outcomes(&self) -> BTreeMap<&str, (u64, &str)> {
        let mut outcomes: BTreeMap<&str, (u64, &str)> = BTreeMap::new();
        for entry in self.all() {
            let trace = &entry.trace;
            if entry.status != TraceStatus::Active
                || trace.kind != TraceKind::Outcome
                || trace.is_model_authored()
            {
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
        outcomes
    }

    pub fn calibration(&self, family: Option<&str>) -> CalibrationReport {
        let outcomes = self.latest_outcomes();

        type Key = (String, String, String);
        let mut groups: BTreeMap<Key, (Vec<Pair>, usize)> = BTreeMap::new();
        let mut unresolved = 0;
        for entry in self.all() {
            // A purged or superseded prediction was retracted: it must not count toward the
            // metrics or the unresolved total (the same rule as `gate_threshold`).
            if entry.status != TraceStatus::Active {
                continue;
            }
            let trace = &entry.trace;
            let Some(prediction) = &trace.prediction else {
                continue;
            };
            if family.is_some_and(|f| f != prediction.family) {
                continue;
            }
            let Some((_, observed)) = outcomes.get(trace.id.as_str()) else {
                unresolved += 1;
                continue;
            };
            let group = groups
                .entry((
                    prediction.engine.clone(),
                    prediction.family.clone(),
                    prediction.answer_kind.clone(),
                ))
                .or_default();
            match prediction.p {
                Some(p) => group.0.push(Pair {
                    prediction,
                    p,
                    observed: observed.trim().to_string(),
                }),
                None => group.1 += 1,
            }
        }

        let families = groups
            .into_iter()
            .map(|((engine, family, answer_kind), (pairs, unscored))| {
                summarize(engine, family, answer_kind, &pairs, unscored)
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
    unscored: usize,
) -> FamilyCalibration {
    let n = pairs.len();
    let denominator = n.max(1) as f64;
    let is_noul = answer_kind == "noul";
    // (reported probability, outcome as 0/1) for the ECE / reliability view.
    let points: Vec<(f64, f64)> = pairs
        .iter()
        .map(|pair| {
            let y = if is_noul {
                pair.observed.eq_ignore_ascii_case("true")
            } else {
                pair.observed == pair.prediction.value
            };
            (pair.p, if y { 1.0 } else { 0.0 })
        })
        .collect();
    let rate = points.iter().map(|(_, y)| y).sum::<f64>() / denominator;
    let (brier, baseline_brier) = if is_noul {
        let brier = points.iter().map(|(p, y)| (p - y).powi(2)).sum::<f64>() / denominator;
        let base = points.iter().map(|(_, y)| (rate - y).powi(2)).sum::<f64>() / denominator;
        (brier, Some(base))
    } else {
        let brier = pairs
            .iter()
            .map(|pair| {
                let probabilities = &pair.prediction.probabilities;
                if probabilities.is_empty() {
                    let hit = pair.observed == pair.prediction.value;
                    (pair.p - if hit { 1.0 } else { 0.0 }).powi(2)
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
            / denominator;
        (brier, None)
    };

    let (z, p_value) = match spiegelhalter_z(&points) {
        Some(z) => (Some(z), Some(erfc(z.abs() / std::f64::consts::SQRT_2))),
        None => (None, None),
    };
    // Noul: the family Brier and its base-rate Brier. Choice/score: the binary Brier of the
    // top-label pair against the top-label accuracy base rate.
    let (skill_brier, skill_reference) = match baseline_brier {
        Some(base) => (brier, base),
        None => (
            points.iter().map(|(p, y)| (p - y).powi(2)).sum::<f64>() / denominator,
            rate * (1.0 - rate),
        ),
    };
    let brier_skill =
        (n > 0 && skill_reference > f64::EPSILON).then(|| 1.0 - skill_brier / skill_reference);
    let measurable = n >= MIN_OUTCOMES;
    let calibrated = measurable
        && z.is_some_and(|z| z.abs() < Z_CRITICAL)
        && brier_skill.is_some_and(|skill| skill > 0.0);

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
        ece += members.len() as f64 / denominator * (mean_p - observed_rate).abs();
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
        unscored,
        brier,
        baseline_brier,
        ece,
        rate,
        z,
        p_value,
        brier_skill,
        measurable,
        calibrated,
        bins,
    }
}

/// Spiegelhalter (1986) Z statistic for `(p, y)` pairs: under perfect calibration it is
/// approximately standard normal. `None` when the variance term is zero.
pub fn spiegelhalter_z(points: &[(f64, f64)]) -> Option<f64> {
    let numerator: f64 = points.iter().map(|(p, y)| (y - p) * (1.0 - 2.0 * p)).sum();
    let variance: f64 = points
        .iter()
        .map(|(p, _)| (1.0 - 2.0 * p).powi(2) * p * (1.0 - p))
        .sum();
    (variance > f64::EPSILON).then(|| numerator / variance.sqrt())
}

/// Complementary error function (Numerical Recipes `erfcc`, fractional error below 1.2e-7),
/// so the kernel needs no maths dependency for a two-sided normal p-value.
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98
                                + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let r = t * poly.exp();
    if x >= 0.0 {
        r
    } else {
        2.0 - r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_p_values_match_tables() {
        for (z, expected) in [
            (0.0, 1.0),
            (1.96, 0.049_996),
            (2.576, 0.009_995),
            (1.0, 0.317_311),
        ] {
            let p = erfc(f64::abs(z) / std::f64::consts::SQRT_2);
            assert!((p - expected).abs() < 1e-5, "z={z}: {p}");
        }
    }
}
