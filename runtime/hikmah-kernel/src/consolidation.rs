use crate::claims::{normalize_key, normalize_value};
use crate::ledger::MemoryStore;
use crate::trace::TraceKind;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationProposal {
    pub claim_key: String,
    pub claim_value: String,
    pub support_trace_ids: Vec<String>,
    pub independent_sources: Vec<String>,
    pub conflicting_values: Vec<String>,
    pub confidence: f32,
    pub eligible_for_promotion: bool,
}

impl MemoryStore {
    /// Quiet Replay: generate evidence-preserving consolidation proposals.
    /// This never writes a new belief automatically.
    pub fn consolidation_proposals(&self) -> Vec<ConsolidationProposal> {
        let mut grouped: BTreeMap<String, BTreeMap<String, Vec<_>>> = BTreeMap::new();
        let mut display: BTreeMap<(String, String), (String, String)> = BTreeMap::new();
        for trace in self.active_traces() {
            // Model predictions are proposals, never supporting evidence for a belief.
            if trace.kind == TraceKind::Prediction || trace.is_model_authored() {
                continue;
            }
            let (Some(key), Some(value)) = (&trace.claim_key, &trace.claim_value) else {
                continue;
            };
            let (nk, nv) = (normalize_key(key), normalize_value(value));
            display
                .entry((nk.clone(), nv.clone()))
                .or_insert_with(|| (key.clone(), value.clone()));
            grouped
                .entry(nk)
                .or_default()
                .entry(nv)
                .or_default()
                .push(trace);
        }

        let mut proposals = Vec::new();
        for (key, values) in grouped {
            for (value, traces) in &values {
                // Independence is judged on normalized source names, so `Config-A` and
                // `config-a` count once.
                let sources: BTreeSet<String> = traces
                    .iter()
                    .map(|trace| trace.provenance.source.trim().to_lowercase())
                    .collect();
                let average_confidence = if traces.is_empty() {
                    0.0
                } else {
                    traces.iter().map(|trace| trace.confidence).sum::<f32>() / traces.len() as f32
                };
                let verification_ratio = if traces.is_empty() {
                    0.0
                } else {
                    traces
                        .iter()
                        .filter(|trace| trace.provenance.verified)
                        .count() as f32
                        / traces.len() as f32
                };
                let confidence =
                    (0.7 * average_confidence + 0.3 * verification_ratio).clamp(0.0, 1.0);
                let conflicting_values = values
                    .keys()
                    .filter(|other| *other != value)
                    .map(|other| {
                        display
                            .get(&(key.clone(), other.clone()))
                            .map(|(_, original)| original.clone())
                            .unwrap_or_else(|| other.clone())
                    })
                    .collect::<Vec<_>>();
                let eligible_for_promotion = traces.len()
                    >= self.policy().consolidation_min_support
                    && sources.len() >= self.policy().consolidation_min_independent_sources
                    && confidence >= self.policy().consolidation_min_confidence
                    && conflicting_values.is_empty();
                let (shown_key, shown_value) = display
                    .get(&(key.clone(), value.clone()))
                    .cloned()
                    .unwrap_or_else(|| (key.clone(), value.clone()));

                proposals.push(ConsolidationProposal {
                    claim_key: shown_key,
                    claim_value: shown_value,
                    support_trace_ids: traces.iter().map(|trace| trace.id.clone()).collect(),
                    independent_sources: sources.into_iter().collect(),
                    conflicting_values,
                    confidence,
                    eligible_for_promotion,
                });
            }
        }
        proposals.sort_by(|a, b| {
            b.eligible_for_promotion
                .cmp(&a.eligible_for_promotion)
                .then_with(|| {
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
        proposals
    }
}
