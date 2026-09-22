//! Kernel policy: every tunable limit, threshold, and recall weight, as data.
//!
//! The defaults are the shipped design choices (none of them is calibrated). A JSON policy file
//! (`hikmah --policy <file>` or `HIKMAH_POLICY`) may override any subset of fields; missing fields
//! keep their defaults and unknown fields are rejected, so a typo cannot silently do nothing.
//! `hikmah policy --print-defaults` prints the full default policy.
use crate::error::{KernelError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KernelPolicy {
    pub working_set_limit: usize,
    pub recall_limit: usize,
    /// Minimum relevance (`cue`) a trace needs to be recalled.
    pub minimum_recall_score: f32,
    pub allow_sensitive_persistence: bool,
    pub consolidation_min_support: usize,
    pub consolidation_min_independent_sources: usize,
    /// Consolidation proposals below this blended confidence are never eligible for promotion.
    pub consolidation_min_confidence: f32,
    /// Recall scoring weights (see `recall.rs`).
    pub recall: RecallWeights,
}

impl Default for KernelPolicy {
    fn default() -> Self {
        Self {
            working_set_limit: 12,
            recall_limit: 8,
            minimum_recall_score: 0.12,
            allow_sensitive_persistence: false,
            consolidation_min_support: 2,
            consolidation_min_independent_sources: 2,
            consolidation_min_confidence: 0.6,
            recall: RecallWeights::default(),
        }
    }
}

/// Weights and scales of the recall score:
///
/// - `lexical = max(lexical_coverage × coverage + lexical_jaccard × jaccard, match_floor)` when
///   any query term matches;
/// - `cue = cue_lexical × lexical + cue_tag × tag` (only one of them when the query has only
///   terms or only tags);
/// - `meta = meta_recency × recency + meta_salience × salience + meta_confidence × confidence
///   + meta_provenance × provenance + meta_prospective × prospective`, clamped to [0, 1];
/// - `score = cue × (relevance_base + metadata_share × meta)`; an overdue commitment scores at
///   least `overdue_floor`; with no cue at all, `score = listing_scale × meta`.
///
/// `recency = 1 / (1 + age_days / recency_scale_days)`; `provenance = authority ×
/// (1 if verified, else unverified_provenance_factor)`; a commitment's `prospective = 1 /
/// (1 + days_left / prospective_scale_days)`, 1 when overdue, `undated_commitment_urgency`
/// without a deadline. Redundancy suppression folds a result whose word overlap with a
/// better one is at least `redundant_at` and otherwise multiplies its score by
/// `1 − redundancy_penalty × overlap`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RecallWeights {
    pub lexical_coverage: f32,
    pub lexical_jaccard: f32,
    /// Relevance given to any trace that shares at least one content term with the query.
    pub match_floor: f32,
    pub cue_lexical: f32,
    pub cue_tag: f32,
    pub meta_recency: f32,
    pub meta_salience: f32,
    pub meta_confidence: f32,
    pub meta_provenance: f32,
    pub meta_prospective: f32,
    pub relevance_base: f32,
    pub metadata_share: f32,
    pub recency_scale_days: f64,
    pub unverified_provenance_factor: f32,
    pub prospective_scale_days: f64,
    pub undated_commitment_urgency: f32,
    pub overdue_floor: f32,
    pub listing_scale: f32,
    pub redundant_at: f32,
    pub redundancy_penalty: f32,
}

impl Default for RecallWeights {
    fn default() -> Self {
        Self {
            lexical_coverage: 0.7,
            lexical_jaccard: 0.3,
            match_floor: 0.15,
            cue_lexical: 0.8,
            cue_tag: 0.2,
            meta_recency: 0.25,
            meta_salience: 0.20,
            meta_confidence: 0.20,
            meta_provenance: 0.25,
            meta_prospective: 0.10,
            relevance_base: 0.55,
            metadata_share: 0.45,
            recency_scale_days: 30.0,
            unverified_provenance_factor: 0.65,
            prospective_scale_days: 7.0,
            undated_commitment_urgency: 0.35,
            overdue_floor: 0.15,
            listing_scale: 0.5,
            redundant_at: 0.8,
            redundancy_penalty: 0.35,
        }
    }
}

impl KernelPolicy {
    /// Load a JSON policy file: missing fields keep their defaults, unknown fields are errors,
    /// and the result is validated.
    pub fn from_json_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|error| {
            KernelError::Invalid(format!("cannot read policy {}: {error}", path.display()))
        })?;
        Self::from_json(&text)
            .map_err(|error| KernelError::Invalid(format!("policy {}: {error}", path.display())))
    }

    /// Load a policy from JSON data (a `--policy` file or `HIKMAH_POLICY`). Data can tune
    /// weights and thresholds but cannot lift the sensitive-persistence hard block: the
    /// append-only reference ledger cannot delete, so enabling it belongs in code, next to an
    /// encrypted, deletion-capable storage adapter, not in an ambient file or variable.
    pub fn from_json(text: &str) -> Result<Self> {
        let policy: Self = serde_json::from_str(text)?;
        if policy.allow_sensitive_persistence {
            return Err(KernelError::Invalid(
                "policy field allow_sensitive_persistence cannot be enabled from a policy file; the append-only ledger cannot delete sensitive data. Enable it in code alongside an encrypted, deletion-capable store".into(),
            ));
        }
        policy.validate()?;
        Ok(policy)
    }

    /// Reject values that would break recall's invariants (relevance gates, bounded scores).
    pub fn validate(&self) -> Result<()> {
        let bad = |name: &str, rule: &str| {
            Err(KernelError::Invalid(format!("policy field {name} {rule}")))
        };
        if self.working_set_limit == 0 || self.recall_limit == 0 {
            return bad("working_set_limit / recall_limit", "must be at least 1");
        }
        let r = &self.recall;
        let unit = [
            ("minimum_recall_score", self.minimum_recall_score),
            (
                "consolidation_min_confidence",
                self.consolidation_min_confidence,
            ),
            ("recall.lexical_coverage", r.lexical_coverage),
            ("recall.lexical_jaccard", r.lexical_jaccard),
            ("recall.match_floor", r.match_floor),
            ("recall.cue_lexical", r.cue_lexical),
            ("recall.cue_tag", r.cue_tag),
            ("recall.meta_recency", r.meta_recency),
            ("recall.meta_salience", r.meta_salience),
            ("recall.meta_confidence", r.meta_confidence),
            ("recall.meta_provenance", r.meta_provenance),
            ("recall.meta_prospective", r.meta_prospective),
            ("recall.relevance_base", r.relevance_base),
            ("recall.metadata_share", r.metadata_share),
            (
                "recall.unverified_provenance_factor",
                r.unverified_provenance_factor,
            ),
            (
                "recall.undated_commitment_urgency",
                r.undated_commitment_urgency,
            ),
            ("recall.overdue_floor", r.overdue_floor),
            ("recall.listing_scale", r.listing_scale),
            ("recall.redundant_at", r.redundant_at),
            ("recall.redundancy_penalty", r.redundancy_penalty),
        ];
        for (name, value) in unit {
            if !(0.0..=1.0).contains(&value) {
                return bad(name, "must be between 0 and 1");
            }
        }
        if self.minimum_recall_score <= 0.0 {
            return bad(
                "minimum_recall_score",
                "must be above 0 (at 0, a trace with no matching cue would be recalled)",
            );
        }
        if self.consolidation_min_support == 0 || self.consolidation_min_independent_sources == 0 {
            return bad(
                "consolidation_min_support / consolidation_min_independent_sources",
                "must be at least 1",
            );
        }
        // Each pair blends two shares of one score; above 1 the score saturates its clamp and
        // ties are broken by recency instead of relevance.
        for (name, a, b) in [
            (
                "recall.lexical_coverage + recall.lexical_jaccard",
                r.lexical_coverage,
                r.lexical_jaccard,
            ),
            (
                "recall.cue_lexical + recall.cue_tag",
                r.cue_lexical,
                r.cue_tag,
            ),
            (
                "recall.relevance_base + recall.metadata_share",
                r.relevance_base,
                r.metadata_share,
            ),
        ] {
            if a + b > 1.0 + 1e-9 {
                return bad(name, "must sum to at most 1");
            }
        }
        if r.relevance_base <= 0.0 {
            return bad(
                "recall.relevance_base",
                "must be above 0 (relevance must always count)",
            );
        }
        for (name, value) in [
            ("recall.recency_scale_days", r.recency_scale_days),
            ("recall.prospective_scale_days", r.prospective_scale_days),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return bad(name, "must be a positive number of days");
            }
        }
        Ok(())
    }
}
