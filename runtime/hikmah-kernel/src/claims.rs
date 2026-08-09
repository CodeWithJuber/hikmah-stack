use crate::trace::Trace;
use serde::{Deserialize, Serialize};

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
    let normalized_value = normalize(value);
    existing
        .filter_map(|trace| {
            let (Some(existing_key), Some(existing_value)) = (&trace.claim_key, &trace.claim_value)
            else {
                return None;
            };
            if normalize(existing_key) == normalize(key)
                && normalize(existing_value) != normalized_value
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

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
