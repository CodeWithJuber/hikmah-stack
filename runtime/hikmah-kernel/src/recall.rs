use crate::ledger::MemoryStore;
use crate::trace::{now_ms, Trace, TraceKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct RecallQuery {
    pub text: String,
    pub tags: Vec<String>,
    pub kinds: Vec<TraceKind>,
    pub limit: usize,
    pub now_ms: u64,
}

impl RecallQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tags: Vec::new(),
            kinds: Vec::new(),
            limit: 8,
            now_ms: now_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallChannels {
    pub lexical: f32,
    pub tag: f32,
    pub recency: f32,
    pub salience: f32,
    pub confidence: f32,
    pub provenance: f32,
    pub prospective: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallResult {
    pub trace: Trace,
    pub score: f32,
    pub channels: RecallChannels,
}

impl MemoryStore {
    pub fn recall(&self, query: &RecallQuery) -> Vec<RecallResult> {
        let query_terms = tokenize(&query.text);
        let query_tags: BTreeSet<String> = query
            .tags
            .iter()
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        let limit = query.limit.min(self.policy().recall_limit).max(1);
        let mut candidates: Vec<RecallResult> = self
            .active_traces()
            .filter(|trace| query.kinds.is_empty() || query.kinds.contains(&trace.kind))
            .map(|trace| score_trace(trace, &query_terms, &query_tags, query.now_ms))
            .filter(|result| result.score >= self.policy().minimum_recall_score)
            .collect();

        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.trace.created_at_ms.cmp(&a.trace.created_at_ms))
        });

        diversify(candidates, limit)
    }
}

fn score_trace(
    trace: &Trace,
    query_terms: &BTreeSet<String>,
    query_tags: &BTreeSet<String>,
    now_ms: u64,
) -> RecallResult {
    let trace_terms = tokenize(&trace.content);
    let lexical = jaccard(query_terms, &trace_terms);
    let trace_tags: BTreeSet<String> = trace
        .tags
        .iter()
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    let tag = jaccard(query_tags, &trace_tags);
    let age_days = now_ms.saturating_sub(trace.created_at_ms) as f64 / 86_400_000.0;
    let recency = (1.0 / (1.0 + age_days / 30.0)) as f32;
    let provenance = (trace.provenance.authority
        * if trace.provenance.verified { 1.0 } else { 0.65 })
    .clamp(0.0, 1.0);
    let prospective = match (trace.kind, trace.deadline_ms) {
        (TraceKind::Commitment, Some(deadline)) if deadline <= now_ms => 1.0,
        (TraceKind::Commitment, Some(deadline)) => {
            let days = deadline.saturating_sub(now_ms) as f64 / 86_400_000.0;
            (1.0 / (1.0 + days / 7.0)) as f32
        }
        (TraceKind::Commitment, None) => 0.35,
        _ => 0.0,
    };

    let channels = RecallChannels {
        lexical,
        tag,
        recency,
        salience: trace.salience,
        confidence: trace.confidence,
        provenance,
        prospective,
    };
    let score = (0.42 * lexical
        + 0.10 * tag
        + 0.10 * recency
        + 0.11 * trace.salience
        + 0.10 * trace.confidence
        + 0.10 * provenance
        + 0.07 * prospective)
        .clamp(0.0, 1.0);

    RecallResult {
        trace: trace.clone(),
        score,
        channels,
    }
}

fn diversify(candidates: Vec<RecallResult>, limit: usize) -> Vec<RecallResult> {
    let mut selected: Vec<RecallResult> = Vec::new();
    for mut candidate in candidates {
        let candidate_terms = tokenize(&candidate.trace.content);
        let redundancy = selected
            .iter()
            .map(|existing| jaccard(&candidate_terms, &tokenize(&existing.trace.content)))
            .fold(0.0_f32, f32::max);
        candidate.score *= 1.0 - 0.35 * redundancy;
        if candidate.score > 0.0 {
            selected.push(candidate);
            selected.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            if selected.len() > limit {
                selected.pop();
            }
        }
    }
    selected
}

fn tokenize(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|token| token.len() > 1)
        .map(ToOwned::to_owned)
        .collect()
}

fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jaccard_is_symmetric() {
        let a = tokenize("alpha beta gamma");
        let b = tokenize("beta gamma delta");
        assert_eq!(jaccard(&a, &b), jaccard(&b, &a));
    }
}
