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
}

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
        }
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
        Ok(())
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
