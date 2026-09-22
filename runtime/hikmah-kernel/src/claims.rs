use crate::ledger::MemoryStore;
use crate::trace::{PrivacyClass, Trace};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimConflict {
    pub claim_key: String,
    pub existing_trace_id: String,
    pub existing_value: String,
    pub incoming_value: String,
}

pub fn detect_conflicts<'a>(
    incoming: &Trace,
    existing: impl Iterator<Item = &'a Trace>,
) -> Vec<ClaimConflict> {
    let (Some(key), Some(value)) = (&incoming.claim_key, &incoming.claim_value) else {
        return Vec::new();
    };
    let normalized_key = normalize_key(key);
    let normalized_value = normalize_value(value);
    existing
        .filter_map(|trace| {
            let (Some(existing_key), Some(existing_value)) = (&trace.claim_key, &trace.claim_value)
            else {
                return None;
            };
            if normalize_key(existing_key) == normalized_key
                && normalize_value(existing_value) != normalized_value
            {
                Some(ClaimConflict {
                    claim_key: key.clone(),
                    existing_trace_id: trace.id.clone(),
                    existing_value: existing_value.clone(),
                    incoming_value: value.clone(),
                })
            } else {
                None
            }
        })
        .collect()
}

/// Ids of the traces in `others` (never `trace` itself) whose claim has the same normalized key as
/// `trace` and a different normalized value. Sorted and de-duplicated.
pub fn conflicting_ids<'a>(trace: &Trace, others: impl Iterator<Item = &'a Trace>) -> Vec<String> {
    let mut ids: Vec<String> = detect_conflicts(trace, others.filter(|other| other.id != trace.id))
        .into_iter()
        .map(|conflict| conflict.existing_trace_id)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// True when both traces carry a claim with the same normalized key and different values.
pub fn claims_conflict(a: &Trace, b: &Trace) -> bool {
    match (&a.claim_key, &a.claim_value, &b.claim_key, &b.claim_value) {
        (Some(ak), Some(av), Some(bk), Some(bv)) => {
            normalize_key(ak) == normalize_key(bk) && normalize_value(av) != normalize_value(bv)
        }
        _ => false,
    }
}

/// One value of a contested claim and the active traces asserting it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimValueGroup {
    /// The normalized value (Unicode NFC, whitespace collapsed, case kept).
    pub value: String,
    /// Sorted by id.
    pub trace_ids: Vec<String>,
}

/// An unresolved conflict: active traces that share a claim key but disagree on its value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveConflict {
    /// The normalized key (Unicode NFC, whitespace collapsed, lowercase).
    pub claim_key: String,
    pub values: Vec<ClaimValueGroup>,
}

impl MemoryStore {
    /// Every claim key whose active traces carry more than one normalized value. Conflicts are
    /// derived from current state on every call, so a supersession or purge resolves them without
    /// any extra event. Sensitive traces are left out unless the policy allows them.
    pub fn active_conflicts(&self) -> Vec<ActiveConflict> {
        let allow_sensitive = self.policy().allow_sensitive_persistence;
        let mut keys: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
        for trace in self.active_traces() {
            if !allow_sensitive && trace.privacy == PrivacyClass::Sensitive {
                continue;
            }
            let (Some(key), Some(value)) = (&trace.claim_key, &trace.claim_value) else {
                continue;
            };
            keys.entry(normalize_key(key))
                .or_default()
                .entry(normalize_value(value))
                .or_default()
                .push(trace.id.clone());
        }
        keys.into_iter()
            .filter(|(_, values)| values.len() > 1)
            .map(|(claim_key, values)| ActiveConflict {
                claim_key,
                values: values
                    .into_iter()
                    .map(|(value, trace_ids)| ClaimValueGroup { value, trace_ids })
                    .collect(),
            })
            .collect()
    }
}

/// Claim keys are identifiers: Unicode NFC, whitespace-collapsed, case-insensitive.
pub fn normalize_key(value: &str) -> String {
    collapse(&value.nfc().collect::<String>()).to_lowercase()
}

/// Claim values keep their case (`/srv/App` and `/srv/app` are different paths) but are
/// NFC-normalized and whitespace-collapsed, so canonically equal text never "conflicts".
pub fn normalize_value(value: &str) -> String {
    collapse(&value.nfc().collect::<String>())
}

fn collapse(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
