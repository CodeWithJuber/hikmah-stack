use crate::recall::RecallResult;
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
    pub fn from_recall(results: Vec<RecallResult>, capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let items = results
            .into_iter()
            .take(capacity)
            .map(|result| FocusItem {
                trace: result.trace,
                activation: result.score,
                pinned: false,
            })
            .collect();
        Self { capacity, items }
    }

    pub fn pin(&mut self, trace_id: &str) -> bool {
        if let Some(item) = self.items.iter_mut().find(|item| item.trace.id == trace_id) {
            item.pinned = true;
            true
        } else {
            false
        }
    }
}
