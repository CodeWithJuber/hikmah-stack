//! Calibration earned from outcomes.
//!
//! Pairs every *active* `Prediction` trace (a purged or superseded prediction was retracted and
//! does not count) with the latest *active* `Outcome` trace that resolves it (written by a
//! non-model principal; purged or superseded outcomes do not count) and reports, per forecaster
//! (an engine such as `jev@jev-1.13.0`, or a person or agent such as `human:alex`) and question
//! family: Brier score, expected calibration error over 5 equal-width bins, accuracy or base
//! rate, and how many predictions carried no probability at all. Rows are ordered by family,
//! then answer kind, then forecaster, so every forecaster's row for one family sits next to the
//! others.
//!
//! A forecaster is its class and its name. The class comes from the trace's source, not from
//! the name it records: `engine` for a `model:` source, `principal` for anyone else
//! (`forecaster_kind`). A person who records forecasts under an engine's name therefore gets a
//! row of their own, never a share of the engine's.
//!
//! A family is `measurable` once it has at least `calibration_min_outcomes` scored predictions
//! (a [`crate::policy::KernelPolicy`] field, default [`MIN_OUTCOMES`]). Below that its scores are
//! still reported, labelled `evidence: "anecdotal"` next to their `n`: they describe the
//! outcomes so far but support no verdict. Sample size is necessary, not sufficient: a family is
//! reported `calibrated` only when it has at least [`MIN_OUTCOMES`] scored predictions whatever
//! the policy says (a policy can raise that floor through `calibration_min_outcomes`, never
//! lower it), and in addition
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
//!
//! With [`FamilyFilter::Prefix`], the report also has one pooled row per forecaster over every
//! matching family, in the top-label view (for a noul forecast, the probability of the side it
//! leaned to). Pooling trades family-level detail for sample size.
use crate::ledger::MemoryStore;
use crate::trace::{PredictionRecord, TraceKind, TraceStatus};
use serde::Serialize;
use std::collections::BTreeMap;

/// Default of `KernelPolicy::calibration_min_outcomes`, and the minimum number of resolved
/// predictions `hikmah gate-threshold` needs.
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

/// Which families a calibration report covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FamilyFilter<'a> {
    All,
    Exact(&'a str),
    /// Every family that starts with the prefix, plus one pooled row per forecaster.
    Prefix(&'a str),
}

impl FamilyFilter<'_> {
    fn matches(&self, family: &str) -> bool {
        match self {
            Self::All => true,
            Self::Exact(name) => family == *name,
            Self::Prefix(prefix) => family.starts_with(prefix),
        }
    }
}

/// Answer kind of a pooled row: every prediction is scored on its top label.
pub const POOLED_KIND: &str = "top_label";
/// `forecaster_kind` of a row whose predictions have a `model:` source.
pub const ENGINE_FORECASTER: &str = "engine";
/// `forecaster_kind` of a row whose predictions come from a person or agent.
pub const PRINCIPAL_FORECASTER: &str = "principal";

fn forecaster_kind(model_authored: bool) -> &'static str {
    if model_authored {
        ENGINE_FORECASTER
    } else {
        PRINCIPAL_FORECASTER
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FamilyCalibration {
    /// Who forecast: an engine (`jev@jev-1.13.0`) or a person or agent (`human:alex`).
    pub engine: String,
    /// [`ENGINE_FORECASTER`] when the predictions have a `model:` source, otherwise
    /// [`PRINCIPAL_FORECASTER`]. Part of the row's identity, so the same `engine` name from the
    /// two classes gives two rows.
    pub forecaster_kind: &'static str,
    /// The family, or `<prefix>*` for a pooled row.
    pub family: String,
    /// `noul`, `choice`, `score`, or [`POOLED_KIND`] for a pooled row.
    pub answer_kind: String,
    /// Families in a pooled row (empty otherwise).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pooled_families: Vec<String>,
    /// Resolved predictions that carried a probability.
    pub n: usize,
    /// Resolved predictions without any reported probability (excluded from the metrics).
    pub unscored: usize,
    /// `anecdotal` while `n` is below `min_outcomes`: the scores are shown, but they support no
    /// verdict either way. `measurable` from `min_outcomes` on.
    pub evidence: &'static str,
    /// Noul and pooled rows: mean (p - y)². Choice/score: multiclass Brier over the reported
    /// distribution (a forecast with only a top-label probability adds its binary Brier).
    pub brier: f64,
    /// Binary Brier of the probability the verdict tests (noul: P(true); others: P(reported
    /// answer)). The same as `brier` for noul; comparable across forecasters and answer kinds.
    pub top_label_brier: f64,
    /// Brier of always predicting the observed base rate (noul and pooled rows; lower is better).
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
    /// At least `min_outcomes` scored predictions: the scores are evidence, not an anecdote.
    pub measurable: bool,
    /// Measurable, at least [`MIN_OUTCOMES`] scored predictions whatever the policy,
    /// `|z| < 1.96`, and `brier_skill > 0`.
    pub calibrated: bool,
    pub bins: Vec<CalibrationBin>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CalibrationReport {
    /// `calibration_min_outcomes` of the policy in effect.
    pub min_outcomes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family_prefix: Option<String>,
    pub unresolved_predictions: usize,
    pub families: Vec<FamilyCalibration>,
    /// One row per forecaster over every family matching `family_prefix`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pooled: Vec<FamilyCalibration>,
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

    /// Calibration of every family, or of one family.
    pub fn calibration(&self, family: Option<&str>) -> CalibrationReport {
        self.calibration_report(family.map_or(FamilyFilter::All, FamilyFilter::Exact))
    }

    /// Calibration of the families `filter` selects; a prefix filter adds pooled rows.
    pub fn calibration_report(&self, filter: FamilyFilter) -> CalibrationReport {
        let min_outcomes = self.policy().calibration_min_outcomes;
        let outcomes = self.latest_outcomes();

        // (family, answer kind, forecaster class, forecaster): each forecaster's row for a family
        // is adjacent, and the class keeps a principal out of an engine's row of the same name.
        type Key = (String, String, &'static str, String);
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
            if !filter.matches(&prediction.family) {
                continue;
            }
            let Some((_, observed)) = outcomes.get(trace.id.as_str()) else {
                unresolved += 1;
                continue;
            };
            let group = groups
                .entry((
                    prediction.family.clone(),
                    prediction.answer_kind.clone(),
                    forecaster_kind(trace.is_model_authored()),
                    prediction.engine.clone(),
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

        let pooled = match filter {
            FamilyFilter::Prefix(prefix) => {
                type Pool<'p> = (Vec<&'p Pair<'p>>, usize, Vec<String>);
                let mut by_engine: BTreeMap<(&str, &str), Pool> = BTreeMap::new();
                for ((family, _, kind, engine), (pairs, unscored)) in &groups {
                    let slot = by_engine.entry((kind, engine.as_str())).or_default();
                    slot.0.extend(pairs.iter());
                    slot.1 += unscored;
                    if !slot.2.contains(family) {
                        slot.2.push(family.clone());
                    }
                }
                by_engine
                    .into_iter()
                    .map(|((kind, engine), (pairs, unscored, families))| {
                        let points: Vec<(f64, f64)> = pairs.iter().map(|p| top_label(p)).collect();
                        let brier = mean_squared_error(&points);
                        let mut row = finish(
                            (kind, engine.to_string()),
                            format!("{prefix}*"),
                            POOLED_KIND.to_string(),
                            points,
                            brier,
                            true,
                            unscored,
                            min_outcomes,
                        );
                        row.pooled_families = families;
                        row
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        let families = groups
            .into_iter()
            .map(|((family, answer_kind, kind, engine), (pairs, unscored))| {
                summarize(
                    (kind, engine),
                    family,
                    answer_kind,
                    &pairs,
                    unscored,
                    min_outcomes,
                )
            })
            .collect();
        CalibrationReport {
            min_outcomes,
            family_prefix: match filter {
                FamilyFilter::Prefix(prefix) => Some(prefix.to_string()),
                _ => None,
            },
            unresolved_predictions: unresolved,
            families,
            pooled,
        }
    }
}

/// The top-label pair of a prediction: the probability of the answer it leaned to, and whether
/// that answer happened. For noul, the lean is `true` when `P(true) >= 0.5`.
fn top_label(pair: &Pair) -> (f64, f64) {
    let record = pair.prediction;
    let (p, hit) = if record.answer_kind == "noul" {
        let says_true = record.value.eq_ignore_ascii_case("true");
        let happened = pair.observed.eq_ignore_ascii_case("true");
        (
            if says_true { pair.p } else { 1.0 - pair.p },
            says_true == happened,
        )
    } else {
        (pair.p, pair.observed == record.value)
    };
    (p, if hit { 1.0 } else { 0.0 })
}

fn mean_squared_error(points: &[(f64, f64)]) -> f64 {
    points.iter().map(|(p, y)| (p - y).powi(2)).sum::<f64>() / points.len().max(1) as f64
}

/// A row's forecaster: its class ([`ENGINE_FORECASTER`] or [`PRINCIPAL_FORECASTER`]) and name.
type Forecaster = (&'static str, String);

fn summarize(
    forecaster: Forecaster,
    family: String,
    answer_kind: String,
    pairs: &[Pair],
    unscored: usize,
    min_outcomes: usize,
) -> FamilyCalibration {
    let is_noul = answer_kind == "noul";
    // (reported probability, outcome as 0/1) for the verdict and the reliability view.
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
    let brier = if is_noul {
        mean_squared_error(&points)
    } else {
        pairs
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
            / pairs.len().max(1) as f64
    };
    finish(
        forecaster,
        family,
        answer_kind,
        points,
        brier,
        is_noul,
        unscored,
        min_outcomes,
    )
}

/// Verdict, bins, and ECE over `(p, y)` points. `brier` is the row's reported Brier;
/// `report_baseline` shows the base-rate Brier (rows whose `brier` is binary).
#[allow(clippy::too_many_arguments)]
fn finish(
    (forecaster_kind, engine): Forecaster,
    family: String,
    answer_kind: String,
    points: Vec<(f64, f64)>,
    brier: f64,
    report_baseline: bool,
    unscored: usize,
    min_outcomes: usize,
) -> FamilyCalibration {
    let n = points.len();
    let denominator = n.max(1) as f64;
    let rate = points.iter().map(|(_, y)| y).sum::<f64>() / denominator;
    let top_label_brier = mean_squared_error(&points);
    let base = points.iter().map(|(_, y)| (rate - y).powi(2)).sum::<f64>() / denominator;

    let (z, p_value) = match spiegelhalter_z(&points) {
        Some(z) => (Some(z), Some(erfc(z.abs() / std::f64::consts::SQRT_2))),
        None => (None, None),
    };
    // Binary Brier of the tested pair against always predicting its base rate.
    let brier_skill = (n > 0 && base > f64::EPSILON).then(|| 1.0 - top_label_brier / base);
    let measurable = n >= min_outcomes;
    // The verdict keeps its own sample floor: a policy may ask for more outcomes before a row is
    // measurable, but it cannot let the Z test certify fewer than MIN_OUTCOMES.
    let calibrated = measurable
        && n >= MIN_OUTCOMES
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
        forecaster_kind,
        family,
        answer_kind,
        pooled_families: Vec::new(),
        n,
        unscored,
        evidence: if measurable {
            "measurable"
        } else {
            "anecdotal"
        },
        brier,
        top_label_brier,
        baseline_brier: report_baseline.then_some(base),
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
