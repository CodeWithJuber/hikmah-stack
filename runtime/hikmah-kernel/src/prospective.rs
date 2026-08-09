use crate::ledger::MemoryStore;
use crate::trace::{Trace, TraceKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitmentDue {
    pub trace: Trace,
    pub overdue: bool,
    pub due_in_ms: i128,
}

impl MemoryStore {
    pub fn commitments_due(&self, now_ms: u64, within_ms: u64) -> Vec<CommitmentDue> {
        let horizon = now_ms.saturating_add(within_ms);
        let mut due = self
            .active_traces()
            .filter(|trace| trace.kind == TraceKind::Commitment)
            .filter_map(|trace| {
                let deadline = trace.deadline_ms?;
                if deadline > horizon {
                    return None;
                }
                Some(CommitmentDue {
                    trace: trace.clone(),
                    overdue: deadline <= now_ms,
                    due_in_ms: deadline as i128 - now_ms as i128,
                })
            })
            .collect::<Vec<_>>();
        due.sort_by_key(|item| item.trace.deadline_ms.unwrap_or(u64::MAX));
        due
    }
}
