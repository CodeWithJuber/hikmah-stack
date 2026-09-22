use crate::trace::Trace;
use serde::{Deserialize, Serialize};
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
