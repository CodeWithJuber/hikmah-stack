use crate::ledger::MemoryStore;
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
        for trace in self.active_traces() {
            let (Some(key), Some(value)) = (&trace.claim_key, &trace.claim_value) else {
                continue;
            };
            grouped
                .entry(normalize(key))
                .or_default()
                .entry(normalize(value))
                .or_default()
                .push(trace);
        }

        let mut proposals = Vec::new();
        for (key, values) in grouped {
            for (value, traces) in &values {
                let sources: BTreeSet<String> = traces
                    .iter()
                    .map(|trace| trace.provenance.source.clone())
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
                    .cloned()
                    .collect::<Vec<_>>();
                let eligible_for_promotion = traces.len()
                    >= self.policy().consolidation_min_support
                    && sources.len() >= self.policy().consolidation_min_independent_sources
                    && conflicting_values.is_empty();

                proposals.push(ConsolidationProposal {
                    claim_key: key.clone(),
                    claim_value: value.clone(),
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

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
