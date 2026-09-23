//! Data-driven Truth Gate threshold.
//!
//! The hook can record every engine probability as a `prediction` trace (`HIKMAH_HOOK_RECORD`);
//! a person or CI later records whether that completion claim was false (`hikmah outcome
//! --observed true` means it *was* a false completion). [`MemoryStore::gate_threshold`] pairs
//! the two and picks the threshold `t` (the engine blocks when `p >= t`) with the highest recall
//! of false completions whose empirical false-block rate (blocked true completions / all true
//! completions) stays within a budget. Ties in recall go to the higher threshold. Only observed
//! probabilities are candidates, because the rates change only there.
//!
//! This covers the engine path alone. The deterministic rules still block on their own, so the
//! gate's overall false-block rate can be higher by the rules' own false blocks.
use crate::calibration::{MIN_OUTCOMES, Z_CRITICAL};
use crate::error::{KernelError, Result};
use crate::hook::{DEFAULT_ENGINE_THRESHOLD, GATE_FAMILY};
use crate::ledger::MemoryStore;
use crate::trace::TraceStatus;
use serde::Serialize;
use std::collections::BTreeSet;

/// The engine-path rates at one threshold.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThresholdPoint {
    pub threshold: f64,
    /// Blocked false completions / all false completions.
    pub recall: f64,
    /// Blocked true completions / all true completions.
    pub false_block_rate: f64,
    /// True completions that would be blocked.
    pub false_blocks: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GateThresholdReport {
    pub family: String,
    pub engines: Vec<String>,
    /// Resolved predictions with a probability.
    pub n: usize,
    /// Outcomes `true`: the completion claim was false.
    pub false_completions: usize,
    /// Outcomes `false`: the completion claim held.
    pub true_completions: usize,
    pub max_false_block: f64,
    /// The chosen threshold, or `None` when no observed threshold meets the budget.
    pub threshold: Option<f64>,
    pub recall: f64,
    pub false_block_rate: f64,
    pub false_blocks: usize,
    /// Wilson 95% interval for the false-block rate at the chosen threshold.
    pub false_block_rate_ci95: [f64; 2],
    /// The same rates at the shipped default threshold, for comparison.
    pub at_default: ThresholdPoint,
    pub note: String,
}

/// Rates at threshold `t` for `(p, was_false_completion)` points.
pub fn rates_at(points: &[(f64, bool)], t: f64) -> ThresholdPoint {
    let positives = points.iter().filter(|(_, y)| *y).count();
    let negatives = points.len() - positives;
    let caught = points.iter().filter(|(p, y)| *y && *p >= t).count();
    let false_blocks = points.iter().filter(|(p, y)| !*y && *p >= t).count();
    let share = |k: usize, n: usize| if n == 0 { 0.0 } else { k as f64 / n as f64 };
    ThresholdPoint {
        threshold: t,
        recall: share(caught, positives),
        false_block_rate: share(false_blocks, negatives),
        false_blocks,
    }
}

/// Highest-recall threshold among the observed probabilities whose false-block rate is at most
/// `max_false_block`; ties in recall go to fewer false blocks, then to the higher threshold.
/// A threshold that catches no false completion only adds false blocks (turning the engine off
/// does better), so it never qualifies. `None` when none qualifies.
pub fn choose_threshold(points: &[(f64, bool)], max_false_block: f64) -> Option<ThresholdPoint> {
    let mut candidates: Vec<f64> = points.iter().map(|(p, _)| *p).collect();
    candidates.sort_by(|a, b| a.total_cmp(b));
    candidates.dedup();
    candidates
        .into_iter()
        .map(|t| rates_at(points, t))
        .filter(|point| point.recall > 0.0 && point.false_block_rate <= max_false_block)
        .max_by(|a, b| {
            a.recall
                .total_cmp(&b.recall)
                .then(b.false_blocks.cmp(&a.false_blocks))
                .then(a.threshold.total_cmp(&b.threshold))
        })
}

/// Wilson score interval for `k` successes in `n` trials at normal quantile `z`.
pub fn wilson_interval(k: usize, n: usize, z: f64) -> [f64; 2] {
    if n == 0 {
        return [0.0, 1.0];
    }
    let n = n as f64;
    let p = k as f64 / n;
    let z2 = z * z;
    let denominator = 1.0 + z2 / n;
    let center = (p + z2 / (2.0 * n)) / denominator;
    let half = z / denominator * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    [(center - half).max(0.0), (center + half).min(1.0)]
}

impl MemoryStore {
    /// Pair recorded Truth Gate engine predictions (`model:` sources only) with their outcomes and
    /// choose the engine threshold. Refuses with fewer than [`MIN_OUTCOMES`] resolved
    /// predictions, or without both classes.
    pub fn gate_threshold(&self, max_false_block: f64) -> Result<GateThresholdReport> {
        if !(0.0..=1.0).contains(&max_false_block) {
            return Err(KernelError::Invalid(
                "--max-false-block must be between 0 and 1".into(),
            ));
        }
        let outcomes = self.latest_outcomes();
        let mut points = Vec::new();
        let mut engines = BTreeSet::new();
        for entry in self.all() {
            // A purged or superseded prediction must not steer the threshold, and neither may a
            // person's or agent's forecast: this threshold applies to the engine's probability.
            if entry.status != TraceStatus::Active || !entry.trace.is_model_authored() {
                continue;
            }
            let Some(prediction) = &entry.trace.prediction else {
                continue;
            };
            if prediction.family != GATE_FAMILY || prediction.answer_kind != "noul" {
                continue;
            }
            let (Some(p), Some((_, observed))) =
                (prediction.p, outcomes.get(entry.trace.id.as_str()))
            else {
                continue;
            };
            engines.insert(prediction.engine.clone());
            points.push((p, observed.trim().eq_ignore_ascii_case("true")));
        }
        let n = points.len();
        if n < MIN_OUTCOMES {
            return Err(KernelError::Invalid(format!(
                "only {n} resolved {GATE_FAMILY} predictions with a probability; at least {MIN_OUTCOMES} are needed before choosing a threshold. Record predictions with HIKMAH_HOOK_RECORD and outcomes with `hikmah outcome --observed true|false` (true = the completion claim was false)"
            )));
        }
        let false_completions = points.iter().filter(|(_, y)| *y).count();
        let true_completions = n - false_completions;
        if false_completions == 0 || true_completions == 0 {
            return Err(KernelError::Invalid(format!(
                "the resolved predictions contain {false_completions} false and {true_completions} true completions; both are needed to trade recall against false blocks"
            )));
        }
        let chosen = choose_threshold(&points, max_false_block);
        // With no qualifying threshold the engine blocks nothing: zero recall, zero false blocks.
        let (recall, false_block_rate, false_blocks) = chosen.as_ref().map_or((0.0, 0.0, 0), |p| {
            (p.recall, p.false_block_rate, p.false_blocks)
        });
        let note = match &chosen {
            Some(point) => format!(
                "Engine path only: the rules still block on their own. Set HIKMAH_HOOK_THRESHOLD={} to use it. The false-block interval is Wilson 95% over {true_completions} true completions; re-run as outcomes accumulate.",
                point.threshold
            ),
            None => "No observed threshold keeps the false-block rate within the budget; keep the engine off (rules only) or raise the budget.".into(),
        };
        Ok(GateThresholdReport {
            family: GATE_FAMILY.into(),
            engines: engines.into_iter().collect(),
            n,
            false_completions,
            true_completions,
            max_false_block,
            threshold: chosen.as_ref().map(|p| p.threshold),
            recall,
            false_block_rate,
            false_blocks,
            false_block_rate_ci95: wilson_interval(false_blocks, true_completions, Z_CRITICAL),
            at_default: rates_at(&points, DEFAULT_ENGINE_THRESHOLD),
            note,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wilson_matches_a_textbook_value() {
        let [lo, hi] = wilson_interval(4, 40, 1.96);
        assert!(
            (lo - 0.0396).abs() < 5e-4 && (hi - 0.2305).abs() < 5e-4,
            "{lo} {hi}"
        );
        assert_eq!(wilson_interval(0, 0, 1.96), [0.0, 1.0]);
    }

    #[test]
    fn equal_recall_prefers_the_higher_threshold() {
        let points = [(0.9, true), (0.1, false), (0.5, false)];
        let chosen = choose_threshold(&points, 1.0).unwrap();
        assert_eq!(chosen.threshold, 0.9);
        assert_eq!(chosen.recall, 1.0);
        assert_eq!(chosen.false_block_rate, 0.0);
    }
}
