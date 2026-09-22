//! Deterministic contextual recall.
//!
//! Scoring (3.1.0): a trace must match the query's cues (terms or tags) to be recalled at all.
//! Relevance then sets the score and metadata can only scale it:
//!
//! `score = cue × (relevance_base + metadata_share × meta)` (defaults 0.55 and 0.45)
//!
//! where `cue ∈ [0,1]` comes from query-term coverage and Jaccard overlap (plus tag coverage), and
//! `meta ∈ [0,1]` blends recency, salience, confidence, provenance, and commitment urgency.
//! `minimum_recall_score` is applied to `cue` (relevance), not to the final score, and any shared
//! content term lifts `cue` to at least `match_floor`, so a long natural-language question that
//! shares one key word with a memory still recalls it. With the default weights, self-reported
//! salience/confidence can reorder relevant memories by less than 2× (at most
//! `1 + metadata_share / relevance_base`); with any weights they can never make an irrelevant
//! memory appear. Overdue commitments surface without a cue. A query made only of stopwords
//! matches nothing. Every weight is a field of [`RecallWeights`] in the kernel policy: explicit
//! design choices, not calibrated values.
//!
//! Each result carries its claim's unresolved `conflicts` (other active traces with the same
//! normalized claim key and a different value) and its supersession links, so a correction or a
//! competing claim is visible beside the claim it challenges.
use crate::claims::{claims_conflict, conflicting_ids};
use crate::ledger::MemoryStore;
use crate::policy::RecallWeights;
use crate::trace::{now_ms, PrivacyClass, Trace, TraceKind, TraceStatus};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct RecallQuery {
    pub text: String,
    pub tags: Vec<String>,
    pub kinds: Vec<TraceKind>,
    pub limit: usize,
    pub now_ms: u64,
    /// Also recall superseded traces (history), each with `superseded_by` set. Off by default:
    /// a replaced belief must not be activated as if it were current.
    pub include_superseded: bool,
}

impl RecallQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tags: Vec::new(),
            kinds: Vec::new(),
            limit: 8,
            now_ms: now_ms(),
            include_superseded: false,
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
    /// Near-identical traces folded into this result by redundancy suppression.
    #[serde(default)]
    pub duplicates: usize,
    /// Other active traces whose claim has the same normalized key and a different normalized
    /// value: an unresolved conflict. Empty for superseded results and for traces without a claim.
    #[serde(default)]
    pub conflicts: Vec<String>,
    /// The trace this one replaced (a correction's predecessor), when that supersession applied.
    #[serde(default)]
    pub supersedes: Option<String>,
    /// The trace that replaced this one. Only superseded traces have one, so it is set only when
    /// the query asked for `include_superseded`.
    #[serde(default)]
    pub superseded_by: Option<String>,
}

impl MemoryStore {
    pub fn recall(&self, query: &RecallQuery) -> Vec<RecallResult> {
        let query_terms = tokenize(&query.text);
        let query_tags: BTreeSet<String> = query
            .tags
            .iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        let has_cue = !query_terms.is_empty() || !query_tags.is_empty();
        // Text was given but every word was a stopword: there is nothing to match on.
        if !has_cue && query.text.chars().any(char::is_alphanumeric) {
            return Vec::new();
        }
        let minimum = self.policy().minimum_recall_score;
        let limit = query.limit.min(self.policy().recall_limit).max(1);
        let allow_sensitive = self.policy().allow_sensitive_persistence;
        let weights = &self.policy().recall;
        let mut candidates: Vec<RecallResult> = self
            .all()
            .filter(|entry| {
                entry.status == TraceStatus::Active
                    || (query.include_superseded && entry.status == TraceStatus::Superseded)
            })
            .map(|entry| &entry.trace)
            .filter(|trace| {
                if query.kinds.is_empty() {
                    // Model predictions are not memories of the world; ask for them explicitly.
                    trace.kind != TraceKind::Prediction
                } else {
                    query.kinds.contains(&trace.kind)
                }
            })
            .filter(|trace| allow_sensitive || trace.privacy != PrivacyClass::Sensitive)
            .filter_map(|trace| {
                score_trace(
                    trace,
                    &query_terms,
                    &query_tags,
                    has_cue,
                    query.now_ms,
                    minimum,
                    weights,
                )
            })
            .collect();

        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.trace.created_at_ms.cmp(&a.trace.created_at_ms))
        });

        let mut results = diversify(candidates, limit, weights);
        self.annotate_links(&mut results, allow_sensitive);
        results
    }

    /// Attach unresolved claim conflicts and supersession links to recall results.
    fn annotate_links(&self, results: &mut [RecallResult], allow_sensitive: bool) {
        // Successor of every trace whose supersession applied (old id -> new id). A purged
        // successor, or one hidden by the privacy filter, is not shown: its id would point at
        // something recall itself refuses to return.
        let successors: BTreeMap<&str, &str> = self
            .all()
            .filter(|entry| entry.status != TraceStatus::Purged)
            .filter(|entry| allow_sensitive || entry.trace.privacy != PrivacyClass::Sensitive)
            .filter_map(|entry| {
                let old = entry.trace.supersedes.as_deref()?;
                let replaced = self.get(old)?.status == TraceStatus::Superseded;
                replaced.then_some((old, entry.trace.id.as_str()))
            })
            .collect();
        for result in results {
            let id = result.trace.id.as_str();
            let active = self
                .get(id)
                .is_some_and(|entry| entry.status == TraceStatus::Active);
            if active {
                result.conflicts = conflicting_ids(
                    &result.trace,
                    self.active_traces()
                        .filter(|t| allow_sensitive || t.privacy != PrivacyClass::Sensitive),
                );
            }
            result.supersedes = result
                .trace
                .supersedes
                .clone()
                .filter(|old| successors.get(old.as_str()) == Some(&id));
            result.superseded_by = successors.get(id).map(|new| new.to_string());
        }
    }
}

fn score_trace(
    trace: &Trace,
    query_terms: &BTreeSet<String>,
    query_tags: &BTreeSet<String>,
    has_cue: bool,
    now_ms: u64,
    minimum: f32,
    w: &RecallWeights,
) -> Option<RecallResult> {
    let trace_terms = tokenize(&trace.content);
    let lexical = if query_terms.is_empty() {
        0.0
    } else {
        let blended = w.lexical_coverage * coverage(query_terms, &trace_terms)
            + w.lexical_jaccard * jaccard(query_terms, &trace_terms);
        if blended > 0.0 {
            blended.max(w.match_floor)
        } else {
            0.0
        }
    };
    let trace_tags: BTreeSet<String> = trace
        .tags
        .iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    let tag = coverage(query_tags, &trace_tags);
    let age_days = now_ms.saturating_sub(trace.created_at_ms) as f64 / 86_400_000.0;
    let recency = (1.0 / (1.0 + age_days / w.recency_scale_days)) as f32;
    let provenance = (trace.provenance.authority
        * if trace.provenance.verified {
            1.0
        } else {
            w.unverified_provenance_factor
        })
    .clamp(0.0, 1.0);
    let prospective = match (trace.kind, trace.deadline_ms) {
        (TraceKind::Commitment, Some(deadline)) if deadline <= now_ms => 1.0,
        (TraceKind::Commitment, Some(deadline)) => {
            let days = deadline.saturating_sub(now_ms) as f64 / 86_400_000.0;
            (1.0 / (1.0 + days / w.prospective_scale_days)) as f32
        }
        (TraceKind::Commitment, None) => w.undated_commitment_urgency,
        _ => 0.0,
    };

    let cue = match (query_terms.is_empty(), query_tags.is_empty()) {
        (false, false) => w.cue_lexical * lexical + w.cue_tag * tag,
        (false, true) => lexical,
        (true, false) => tag,
        (true, true) => 0.0,
    };
    let meta = (w.meta_recency * recency
        + w.meta_salience * trace.salience
        + w.meta_confidence * trace.confidence
        + w.meta_provenance * provenance
        + w.meta_prospective * prospective)
        .clamp(0.0, 1.0);
    let overdue = prospective >= 1.0;
    let score = if has_cue {
        if cue < minimum && !overdue {
            return None;
        }
        (cue * (w.relevance_base + w.metadata_share * meta)).max(if overdue {
            w.overdue_floor
        } else {
            0.0
        })
    } else {
        // Listing mode (no query cues): order by metadata only.
        w.listing_scale * meta
    }
    .clamp(0.0, 1.0);

    Some(RecallResult {
        trace: trace.clone(),
        score,
        channels: RecallChannels {
            lexical,
            tag,
            recency,
            salience: trace.salience,
            confidence: trace.confidence,
            provenance,
            prospective,
        },
        duplicates: 0,
        conflicts: Vec::new(),
        supersedes: None,
        superseded_by: None,
    })
}

fn diversify(candidates: Vec<RecallResult>, limit: usize, w: &RecallWeights) -> Vec<RecallResult> {
    let mut selected: Vec<(RecallResult, BTreeSet<String>)> = Vec::new();
    for mut candidate in candidates {
        let terms = tokenize(&candidate.trace.content);
        let normalized = normalize_content(&candidate.trace.content);
        let mut redundancy = 0.0_f32;
        let mut twin: Option<usize> = None;
        for (index, (existing, existing_terms)) in selected.iter().enumerate() {
            // A competing claim is not redundant, however similar its wording: never fold or
            // penalize it against the claim it contradicts.
            if claims_conflict(&candidate.trace, &existing.trace) {
                continue;
            }
            let overlap = if normalize_content(&existing.trace.content) == normalized {
                1.0
            } else {
                jaccard(&terms, existing_terms)
            };
            if overlap > redundancy {
                redundancy = overlap;
                twin = Some(index);
            }
        }
        if redundancy >= w.redundant_at {
            if let Some(index) = twin {
                selected[index].0.duplicates += 1;
            }
            continue;
        }
        candidate.score *= 1.0 - w.redundancy_penalty * redundancy;
        selected.push((candidate, terms));
        selected.sort_by(|a, b| {
            b.0.score
                .partial_cmp(&a.0.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if selected.len() > limit {
            selected.pop();
        }
    }
    selected.into_iter().map(|(result, _)| result).collect()
}

fn normalize_content(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "been", "but", "by", "can", "did", "do", "does",
    "for", "from", "had", "has", "have", "how", "i", "if", "in", "into", "is", "it", "its", "me",
    "my", "no", "not", "of", "on", "or", "our", "so", "than", "that", "the", "their", "them",
    "then", "there", "these", "they", "this", "to", "was", "we", "were", "what", "when", "where",
    "which", "who", "why", "will", "with", "would", "you", "your",
];

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF     // Hangul Jamo
        | 0x3040..=0x30FF   // Hiragana, Katakana
        | 0x3130..=0x318F   // Hangul compatibility Jamo
        | 0x3400..=0x4DBF   // CJK Extension A
        | 0x4E00..=0x9FFF   // CJK Unified Ideographs
        | 0xAC00..=0xD7AF   // Hangul syllables
        | 0xF900..=0xFAFF   // CJK compatibility ideographs
        | 0xFF66..=0xFF9F   // Half-width katakana
        | 0x20000..=0x2FA1F) // CJK Extensions B-F and compatibility supplement
}

/// Light English suffix folding applied identically to queries and traces: strip a plural `s`,
/// then one of `ing`/`ed`, then a trailing `e`, so setting/settings, service/services, and
/// release/released agree.
fn stem(token: &str) -> String {
    if !token.is_ascii() || token.len() <= 4 {
        return token.to_string();
    }
    let mut word = token;
    if let Some(base) = word.strip_suffix('s') {
        if base.len() >= 4 && !base.ends_with('s') {
            word = base;
        }
    }
    for suffix in ["ing", "ed"] {
        if let Some(base) = word.strip_suffix(suffix) {
            if base.len() >= 3 {
                word = base;
                break;
            }
        }
    }
    if word.len() > 4 {
        if let Some(base) = word.strip_suffix('e') {
            word = base;
        }
    }
    word.to_string()
}

fn push_word(out: &mut BTreeSet<String>, word: &str) {
    let is_number = word.chars().all(|c| c.is_ascii_digit());
    if word.is_empty() || (!is_number && word.chars().count() < 2) || STOPWORDS.contains(&word) {
        return;
    }
    out.insert(stem(word));
}

fn push_cjk_run(out: &mut BTreeSet<String>, run: &[char]) {
    // Scripts written without spaces: index character bigrams (and single characters for
    // one-character runs) so a phrase can match inside a longer clause.
    if run.len() == 1 {
        out.insert(run[0].to_string());
    }
    for pair in run.windows(2) {
        out.insert(pair.iter().collect());
    }
}

pub(crate) fn tokenize(text: &str) -> BTreeSet<String> {
    let lower = text.to_lowercase();
    let mut out = BTreeSet::new();
    for raw in lower.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if raw.is_empty() {
            continue;
        }
        if !raw.chars().any(is_cjk) {
            push_word(&mut out, raw);
            continue;
        }
        // Split mixed tokens such as "修复了api的bug" into same-script runs.
        let chars: Vec<char> = raw.chars().collect();
        let mut start = 0;
        while start < chars.len() {
            let cjk = is_cjk(chars[start]);
            let mut end = start + 1;
            while end < chars.len() && is_cjk(chars[end]) == cjk {
                end += 1;
            }
            if cjk {
                push_cjk_run(&mut out, &chars[start..end]);
            } else {
                let word: String = chars[start..end].iter().collect();
                push_word(&mut out, &word);
            }
            start = end;
        }
    }
    out
}

fn coverage(query: &BTreeSet<String>, target: &BTreeSet<String>) -> f32 {
    if query.is_empty() || target.is_empty() {
        return 0.0;
    }
    query.intersection(target).count() as f32 / query.len() as f32
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

    #[test]
    fn stopwords_and_suffixes_fold() {
        let q = tokenize("why did the migrations fail");
        assert!(q.contains("migration"));
        assert!(!q.contains("the"));
        assert!(!q.contains("why"));
        assert!(tokenize("python 3").contains("3"));
    }

    #[test]
    fn plural_and_ing_forms_agree() {
        assert_eq!(stem("settings"), stem("setting"));
        assert_eq!(stem("services"), stem("service"));
        assert_eq!(stem("released"), stem("release"));
        assert_eq!(stem("warnings"), stem("warning"));
    }

    #[test]
    fn mixed_script_tokens_keep_latin_words() {
        let t = tokenize("修复了API的bug");
        assert!(t.contains("api") && t.contains("bug"));
        assert!(t.contains("修复"));
    }

    #[test]
    fn cjk_phrases_match_inside_clauses() {
        let q = tokenize("数据库迁移");
        let t = tokenize("数据库迁移失败，因为锁超时");
        assert!(coverage(&q, &t) > 0.99);
    }
}
