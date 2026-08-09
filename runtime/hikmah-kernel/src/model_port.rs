use crate::error::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalRequest {
    pub goal: String,
    pub context: Vec<String>,
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub text: String,
    pub assumptions: Vec<String>,
    pub confidence: f32,
}

/// Optional proposal engines sit outside the trusted kernel.
/// They may be transformers, state-space models, symbolic systems, remote APIs,
/// or future architectures. Their output is always treated as a proposal.
pub trait ProposalEngine {
    fn name(&self) -> &str;
    fn propose(&self, request: &ProposalRequest) -> Result<Vec<Proposal>>;
}

#[derive(Debug, Default)]
pub struct NoModel;

impl ProposalEngine for NoModel {
    fn name(&self) -> &str {
        "no-model"
    }

    fn propose(&self, _request: &ProposalRequest) -> Result<Vec<Proposal>> {
        Ok(Vec::new())
    }
}
