use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{KernelError, Result};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TraceKind {
    Observation,
    Episode,
    Belief,
    Procedure,
    Commitment,
    Preference,
    Constraint,
    Outcome,
    Correction,
    /// A typed answer admitted from a decision engine. Never verified, never evidence by itself.
    Prediction,
}

impl Display for TraceKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Observation => "observation",
            Self::Episode => "episode",
            Self::Belief => "belief",
            Self::Procedure => "procedure",
            Self::Commitment => "commitment",
            Self::Preference => "preference",
            Self::Constraint => "constraint",
            Self::Outcome => "outcome",
            Self::Correction => "correction",
            Self::Prediction => "prediction",
        };
        f.write_str(value)
    }
}

impl FromStr for TraceKind {
    type Err = KernelError;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "observation" => Ok(Self::Observation),
            "episode" => Ok(Self::Episode),
            "belief" => Ok(Self::Belief),
            "procedure" => Ok(Self::Procedure),
            "commitment" => Ok(Self::Commitment),
            "preference" => Ok(Self::Preference),
            "constraint" => Ok(Self::Constraint),
            "outcome" => Ok(Self::Outcome),
            "correction" => Ok(Self::Correction),
            "prediction" => Ok(Self::Prediction),
            other => Err(KernelError::Invalid(format!("unknown trace kind: {other}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyClass {
    Public,
    #[default]
    Private,
    Sensitive,
}

impl FromStr for PrivacyClass {
    type Err = KernelError;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "public" => Ok(Self::Public),
            "private" => Ok(Self::Private),
            "sensitive" => Ok(Self::Sensitive),
            other => Err(KernelError::Invalid(format!(
                "unknown privacy class: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceStatus {
    Active,
    Superseded,
    Fulfilled,
    Purged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Provenance {
    pub source: String,
    pub locator: Option<String>,
    pub observed_at_ms: u64,
    pub authority: f32,
    pub verified: bool,
}

impl Provenance {
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            locator: None,
            observed_at_ms: now_ms(),
            authority: 0.5,
            verified: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Trace {
    pub id: String,
    pub kind: TraceKind,
    pub content: String,
    pub tags: Vec<String>,
    pub created_at_ms: u64,
    pub deadline_ms: Option<u64>,
    pub salience: f32,
    pub confidence: f32,
    pub privacy: PrivacyClass,
    pub provenance: Provenance,
    pub claim_key: Option<String>,
    pub claim_value: Option<String>,
    pub supersedes: Option<String>,
    /// Structured payload of a `Prediction` trace (typed decision port).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prediction: Option<PredictionRecord>,
    /// Structured payload of an `Outcome` trace that resolves a prediction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<OutcomeRecord>,
}

/// What a decision engine answered, as admitted by the kernel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PredictionRecord {
    pub request_id: String,
    pub question_id: String,
    /// Calibration bucket; defaults to the question id.
    pub family: String,
    /// `noul`, `choice`, or `score`.
    pub answer_kind: String,
    /// Engine identity, e.g. `jev@jev-1.13.0`.
    pub engine: String,
    /// Noul: probability of `true`. Choice/score: probability of the reported value.
    pub p: f64,
    /// Choice option id or score level index (as text); `true`/`false` for noul.
    pub value: String,
    #[serde(default)]
    pub probabilities: std::collections::BTreeMap<String, f64>,
    /// Always false until calibration for this family is measured from outcomes.
    pub calibrated: bool,
}

/// An observed outcome for a prediction, written by a non-model principal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutcomeRecord {
    pub prediction_id: String,
    /// Same encoding as `PredictionRecord::value`.
    pub observed: String,
}

/// Sources written by decision engines carry this prefix and can never be verified.
pub const MODEL_SOURCE_PREFIX: &str = "model:";

impl Trace {
    pub fn new(kind: TraceKind, content: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            id: String::new(),
            kind,
            content: content.into(),
            tags: Vec::new(),
            created_at_ms: now_ms(),
            deadline_ms: None,
            salience: 0.5,
            confidence: 0.5,
            privacy: PrivacyClass::Private,
            provenance: Provenance::new(source),
            claim_key: None,
            claim_value: None,
            supersedes: None,
            prediction: None,
            outcome: None,
        }
    }

    /// True when a decision engine, not a person or tool, wrote this trace.
    pub fn is_model_authored(&self) -> bool {
        self.provenance
            .source
            .trim()
            .to_ascii_lowercase()
            .starts_with(MODEL_SOURCE_PREFIX)
    }

    pub fn validate(&self) -> Result<()> {
        if self.content.trim().is_empty() {
            return Err(KernelError::Invalid("trace content cannot be empty".into()));
        }
        if !(0.0..=1.0).contains(&self.salience) {
            return Err(KernelError::Invalid(
                "salience must be between 0 and 1".into(),
            ));
        }
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(KernelError::Invalid(
                "confidence must be between 0 and 1".into(),
            ));
        }
        if !(0.0..=1.0).contains(&self.provenance.authority) {
            return Err(KernelError::Invalid(
                "provenance authority must be between 0 and 1".into(),
            ));
        }
        if self.claim_key.is_some() != self.claim_value.is_some() {
            return Err(KernelError::Invalid(
                "claim_key and claim_value must be supplied together".into(),
            ));
        }
        if self.provenance.source.trim().is_empty() {
            return Err(KernelError::Invalid(
                "provenance source cannot be empty".into(),
            ));
        }
        if self.is_model_authored() && self.provenance.verified {
            return Err(KernelError::Invalid(
                "model-authored traces cannot be marked verified; record an outcome from a non-model principal instead"
                    .into(),
            ));
        }
        if self.supersedes.as_deref() == Some(self.id.as_str()) && !self.id.is_empty() {
            return Err(KernelError::Invalid(
                "a trace cannot supersede itself".into(),
            ));
        }
        match self.kind {
            TraceKind::Prediction => {
                if self.prediction.is_none() {
                    return Err(KernelError::Invalid(
                        "prediction traces need a prediction record".into(),
                    ));
                }
                if !self.is_model_authored() {
                    return Err(KernelError::Invalid(format!(
                        "prediction traces must use a `{MODEL_SOURCE_PREFIX}` source"
                    )));
                }
                if self.supersedes.is_some() {
                    return Err(KernelError::Invalid(
                        "prediction traces cannot supersede other traces".into(),
                    ));
                }
            }
            _ if self.prediction.is_some() => {
                return Err(KernelError::Invalid(
                    "only prediction traces may carry a prediction record".into(),
                ));
            }
            _ => {}
        }
        if self.outcome.is_some() {
            if self.kind != TraceKind::Outcome {
                return Err(KernelError::Invalid(
                    "only outcome traces may carry an outcome record".into(),
                ));
            }
            if self.is_model_authored() {
                return Err(KernelError::Invalid(
                    "outcomes must come from a non-model principal".into(),
                ));
            }
        }
        Ok(())
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
