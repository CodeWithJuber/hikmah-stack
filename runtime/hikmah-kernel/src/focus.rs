//! Focus Capsule: the bounded working set an agent deliberates over.
//!
//! Recall answers one query; a capsule accumulates the results of several recalls and keeps at
//! most `capacity` traces (the policy's `working_set_limit`). Pinned traces are never evicted;
//! otherwise the lowest-activation trace leaves first. The capsule is in-memory only: it is a
//! view over the ledger, never a durable state transition.
use crate::ledger::MemoryStore;
use crate::recall::{RecallQuery, RecallResult};
use crate::trace::Trace;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusItem {
    pub trace: Trace,
    pub activation: f32,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusCapsule {
    pub capacity: usize,
    pub items: Vec<FocusItem>,
}

impl FocusCapsule {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            items: Vec::new(),
        }
    }

    pub fn from_recall(results: Vec<RecallResult>, capacity: usize) -> Self {
        let mut capsule = Self::new(capacity);
        capsule.absorb(results);
        capsule
    }

    /// Merge recall results. A trace already in focus keeps its pin and takes the higher
    /// activation. When over capacity, unpinned items with the lowest activation are evicted.
    pub fn absorb(&mut self, results: Vec<RecallResult>) {
        for result in results {
            if let Some(item) = self
                .items
                .iter_mut()
                .find(|item| item.trace.id == result.trace.id)
            {
                item.activation = item.activation.max(result.score);
                continue;
            }
            self.items.push(FocusItem {
                trace: result.trace,
                activation: result.score,
                pinned: false,
            });
        }
        self.items.sort_by(|a, b| {
            b.pinned.cmp(&a.pinned).then_with(|| {
                b.activation
                    .partial_cmp(&a.activation)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        let pinned = self.items.iter().filter(|item| item.pinned).count();
        self.items.truncate(self.capacity.max(pinned));
    }

    pub fn pin(&mut self, trace_id: &str) -> bool {
        if let Some(item) = self.items.iter_mut().find(|item| item.trace.id == trace_id) {
            item.pinned = true;
            true
        } else {
            false
        }
    }

    pub fn unpin(&mut self, trace_id: &str) -> bool {
        if let Some(item) = self.items.iter_mut().find(|item| item.trace.id == trace_id) {
            item.pinned = false;
            true
        } else {
            false
        }
    }
}

impl MemoryStore {
    /// Start a working set from one recall, bounded by the policy's `working_set_limit`.
    pub fn focus(&self, query: &RecallQuery) -> FocusCapsule {
        FocusCapsule::from_recall(self.recall(query), self.policy().working_set_limit)
    }
}
