use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelPolicy {
    pub working_set_limit: usize,
    pub recall_limit: usize,
    pub minimum_recall_score: f32,
    pub allow_sensitive_persistence: bool,
    pub consolidation_min_support: usize,
    pub consolidation_min_independent_sources: usize,
    /// Consolidation proposals below this blended confidence are never eligible for promotion.
    #[serde(default = "default_min_confidence")]
    pub consolidation_min_confidence: f32,
}

fn default_min_confidence() -> f32 {
    0.6
}

impl Default for KernelPolicy {
    fn default() -> Self {
        Self {
            working_set_limit: 12,
            recall_limit: 8,
            minimum_recall_score: 0.12,
            allow_sensitive_persistence: false,
            consolidation_min_support: 2,
            consolidation_min_independent_sources: 2,
            consolidation_min_confidence: default_min_confidence(),
        }
    }
}
