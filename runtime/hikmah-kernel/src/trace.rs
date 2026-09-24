use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{KernelError, Result};
use crate::principal::AGENT_LOCATOR_PREFIX;
use crate::secrets::contains_secret;

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
    /// A forecast: a typed answer admitted from a decision engine (`model:` source), or a
    /// forecast a person or agent recorded with `hikmah predict`. Never verified, never
    /// evidence by itself; an `Outcome` resolves it.
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

/// A forecast: what a decision engine answered (as admitted by the kernel), or what a person or
/// agent forecast.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PredictionRecord {
    pub request_id: String,
    pub question_id: String,
    /// Calibration bucket; defaults to the question id.
    pub family: String,
    /// `noul`, `choice`, or `score`.
    pub answer_kind: String,
    /// Who forecast: the engine identity (`jev@jev-1.13.0`) for a `model:` source, otherwise
    /// the source principal itself (`human:alex`). Calibration groups by it.
    pub engine: String,
    /// Noul: probability of `true`. Choice/score: probability of the reported value.
    /// `None` when the engine reported neither a distribution nor a confidence.
    pub p: Option<f64>,
    /// Choice option id or score level index (as text); `true`/`false` for noul.
    pub value: String,
    #[serde(default)]
    pub probabilities: std::collections::BTreeMap<String, f64>,
    /// Every value an outcome may take (`true`/`false`, option ids, or level indices).
    #[serde(default)]
    pub answer_space: Vec<String>,
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

    /// True when the trace was written from a detected AI agent session (see
    /// [`crate::principal`]). `source` stays whatever the caller claimed.
    pub fn is_from_agent_session(&self) -> bool {
        self.provenance
            .locator
            .as_deref()
            .is_some_and(|locator| locator.trim_start().starts_with(AGENT_LOCATOR_PREFIX))
    }

    pub fn validate(&self) -> Result<()> {
        if self.content.trim().is_empty() {
            return Err(KernelError::Invalid("trace content cannot be empty".into()));
        }
        self.refuse_secrets()?;
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
        if self.provenance.verified && self.is_from_agent_session() {
            return Err(KernelError::Invalid(
                "a trace written from an AI agent session cannot be marked verified; a person must attest it from outside the agent session"
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
                let Some(record) = &self.prediction else {
                    return Err(KernelError::Invalid(
                        "prediction traces need a prediction record".into(),
                    ));
                };
                // A forecast is never a verified fact, whoever made it; an outcome resolves it.
                if self.provenance.verified {
                    return Err(KernelError::Invalid(
                        "prediction traces cannot be marked verified; record an outcome from a non-model principal instead"
                            .into(),
                    ));
                }
                // The record's `engine` is who forecast. A `model:` source is an engine answer
                // and names that engine; any other source is a human or agent forecast and names
                // that principal. The name alone does not say which class wrote it (a principal
                // could call itself `jev@…`), so calibration also keys every row by
                // `is_model_authored()`, and neither class can land in the other's row.
                let principal = self.provenance.source.trim();
                let expected = if self.is_model_authored() {
                    &principal[MODEL_SOURCE_PREFIX.len()..]
                } else {
                    principal
                };
                if record.engine != expected {
                    return Err(KernelError::Invalid(format!(
                        "prediction record engine `{}` must match its source `{principal}`",
                        record.engine
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

    /// Memory never stores a credential merely because it appeared. Every free-text field is
    /// checked with the detector that guards outbound engine requests ([`contains_secret`]); the
    /// error names the field but never echoes the matched text.
    fn refuse_secrets(&self) -> Result<()> {
        let mut fields: Vec<(&str, &str)> = vec![
            ("content", self.content.as_str()),
            ("source", self.provenance.source.as_str()),
        ];
        fields.extend(self.tags.iter().map(|tag| ("tag", tag.as_str())));
        fields.extend(self.claim_key.as_deref().map(|key| ("claim_key", key)));
        fields.extend(
            self.claim_value
                .as_deref()
                .map(|value| ("claim_value", value)),
        );
        fields.extend(
            self.provenance
                .locator
                .as_deref()
                .map(|locator| ("locator", locator)),
        );
        if let Some(record) = &self.prediction {
            fields.push(("prediction family", record.family.as_str()));
            fields.push(("prediction value", record.value.as_str()));
            fields.extend(
                record
                    .answer_space
                    .iter()
                    .map(|value| ("prediction answer space", value.as_str())),
            );
        }
        match fields.into_iter().find(|(_, text)| contains_secret(text)) {
            Some((field, _)) => Err(KernelError::Invalid(format!(
                "trace {field} appears to contain a credential; memory never stores secrets. Record where the secret is kept (for example a vault path), not its value"
            ))),
            None => Ok(()),
        }
    }
}

/// Parse a deadline: epoch milliseconds, `+<n>h`, `+<n>d`, or `YYYY-MM-DD[THH:MM[:SS]][Z]` in UTC.
/// Impossible dates (2026-02-31), negative parts, and years outside 1970..=9999 are rejected.
pub fn parse_deadline(value: &str, now_ms: u64) -> Result<u64> {
    let v = value.trim();
    let invalid = || KernelError::Invalid(format!("unrecognized deadline: {value}"));
    if let Some(rest) = v.strip_prefix('+') {
        let (number, unit_ms) = if let Some(n) = rest.strip_suffix('h') {
            (n, 3_600_000_u64)
        } else if let Some(n) = rest.strip_suffix('d') {
            (n, 86_400_000_u64)
        } else {
            return Err(invalid());
        };
        let n: u64 = number.parse().map_err(|_| invalid())?;
        return Ok(now_ms.saturating_add(n.saturating_mul(unit_ms)));
    }
    if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) {
        return v.parse().map_err(|_| invalid());
    }
    let v = v.strip_suffix('Z').unwrap_or(v);
    let (date, time) = v.split_once('T').unwrap_or((v, "00:00:00"));
    let number = |part: &str| -> Result<u64> {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return Err(invalid());
        }
        part.parse::<u64>().map_err(|_| invalid())
    };
    let d: Vec<u64> = date.split('-').map(number).collect::<Result<_>>()?;
    let t: Vec<u64> = time.split(':').map(number).collect::<Result<_>>()?;
    if d.len() != 3 || !(1..=3).contains(&t.len()) {
        return Err(invalid());
    }
    let (y, m, day) = (d[0], d[1], d[2]);
    let (hh, mm, ss) = (t[0], *t.get(1).unwrap_or(&0), *t.get(2).unwrap_or(&0));
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days_in_month = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return Err(invalid()),
    };
    if !(1970..=9999).contains(&y)
        || day == 0
        || day > days_in_month
        || hh > 23
        || mm > 59
        || ss > 59
    {
        return Err(invalid());
    }
    // Days from civil (Howard Hinnant's algorithm), UTC. All values are bounded above.
    let (y, m, day) = (y as i64, m as i64, day as i64);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + (hh * 3_600 + mm * 60 + ss) as i64;
    Ok(secs as u64 * 1_000)
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
