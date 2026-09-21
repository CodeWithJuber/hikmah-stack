//! Typed decision port: unstructured state in, typed decisions out.
//!
//! This is the boundary for "System One" style engines (for example TypeSafe's Jev) that answer
//! bounded questions with probabilities instead of prose. The doctrine is the same as the text
//! [`crate::model_port::ProposalEngine`]: engines propose, the kernel admits.
//!
//! - Questions are declared up front ([`QuestionKind::Choice`], [`QuestionKind::Score`],
//!   [`QuestionKind::Noul`]).
//! - [`admit`] checks every answer against the question that was asked. Any violation rejects the
//!   *whole* response and every answer becomes an explicit abstention; nothing is repaired.
//! - Only the kernel crate can construct an [`AdmittedDecision`] (it is `#[non_exhaustive]`).
//! - Engine probabilities are passed through as reported and flagged `calibrated: false`.
//!   Calibration is earned from recorded outcomes (see [`crate::calibration`]).
//! - Admitted answers can be recorded as `Prediction` traces with a `model:` source. They are
//!   never verified, never supersede anything, and never count as consolidation evidence.
use crate::error::{KernelError, Result};
use crate::secrets::contains_secret;
use crate::trace::{PredictionRecord, Trace, TraceKind, MODEL_SOURCE_PREFIX};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

pub const MAX_QUESTIONS: usize = 32;
pub const MAX_STATE_CHARS: usize = 32_000;
const PROBABILITY_SUM_TOLERANCE: f64 = 0.02;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QuestionKind {
    /// Pick exactly one option. Map of option id to its meaning (2..=255 options).
    Choice { options: BTreeMap<String, String> },
    /// Rate along ordered levels, lowest first (2..=10 levels).
    Score { levels: Vec<String> },
    /// Yes/no, answered as the probability of `true`.
    Noul {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        if_true: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        if_false: Option<String>,
    },
}

impl QuestionKind {
    fn name(&self) -> &'static str {
        match self {
            Self::Choice { .. } => "choice",
            Self::Score { .. } => "score",
            Self::Noul { .. } => "noul",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Question {
    pub id: String,
    pub instructions: String,
    #[serde(flatten)]
    pub kind: QuestionKind,
    /// Calibration bucket. Defaults to the question id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
}

impl Question {
    pub fn noul(id: impl Into<String>, instructions: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            instructions: instructions.into(),
            kind: QuestionKind::Noul {
                if_true: None,
                if_false: None,
            },
            family: None,
        }
    }

    pub fn family(&self) -> &str {
        self.family.as_deref().unwrap_or(&self.id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionRequest {
    pub state: String,
    pub questions: Vec<Question>,
}

impl DecisionRequest {
    pub fn new(state: impl Into<String>, questions: Vec<Question>) -> Result<Self> {
        let request = Self {
            state: state.into(),
            questions,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<()> {
        if self.state.trim().is_empty() {
            return Err(KernelError::Invalid(
                "decision state cannot be empty".into(),
            ));
        }
        if self.state.chars().count() > MAX_STATE_CHARS {
            return Err(KernelError::Invalid(format!(
                "decision state exceeds {MAX_STATE_CHARS} characters"
            )));
        }
        if self.questions.is_empty() || self.questions.len() > MAX_QUESTIONS {
            return Err(KernelError::Invalid(format!(
                "a decision request needs 1..={MAX_QUESTIONS} questions"
            )));
        }
        let mut ids = BTreeSet::new();
        for question in &self.questions {
            let valid_id = !question.id.is_empty()
                && question.id.len() <= 64
                && question
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !valid_id {
                return Err(KernelError::Invalid(format!(
                    "question id `{}` must be 1-64 chars of [A-Za-z0-9_-]",
                    question.id
                )));
            }
            if !ids.insert(question.id.as_str()) {
                return Err(KernelError::Invalid(format!(
                    "duplicate question id: {}",
                    question.id
                )));
            }
            if question.instructions.trim().is_empty() {
                return Err(KernelError::Invalid(format!(
                    "question {} needs instructions",
                    question.id
                )));
            }
            match &question.kind {
                QuestionKind::Choice { options } => {
                    if !(2..=255).contains(&options.len()) {
                        return Err(KernelError::Invalid(format!(
                            "choice question {} needs 2..=255 options",
                            question.id
                        )));
                    }
                    if options.keys().any(|k| k.trim().is_empty()) {
                        return Err(KernelError::Invalid(format!(
                            "choice question {} has an empty option id",
                            question.id
                        )));
                    }
                }
                QuestionKind::Score { levels } => {
                    if !(2..=10).contains(&levels.len()) {
                        return Err(KernelError::Invalid(format!(
                            "score question {} needs 2..=10 levels",
                            question.id
                        )));
                    }
                }
                QuestionKind::Noul { if_true, if_false } => {
                    if if_true.is_some() != if_false.is_some() {
                        return Err(KernelError::Invalid(format!(
                            "noul question {} needs both if_true and if_false, or neither",
                            question.id
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    /// Stable id derived from the canonical request (state + questions).
    pub fn request_id(&self) -> String {
        let canonical = serde_json::to_vec(self).unwrap_or_default();
        let hash = blake3::hash(&canonical).to_hex().to_string();
        format!("req_{}", &hash[..16])
    }

    pub fn question(&self, id: &str) -> Option<&Question> {
        self.questions.iter().find(|q| q.id == id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EngineDescriptor {
    pub name: String,
    pub version: String,
}

impl EngineDescriptor {
    pub fn source(&self) -> String {
        format!("{MODEL_SOURCE_PREFIX}{}@{}", self.name, self.version)
    }
}

/// An answer exactly as an engine reported it. The field names match TypeSafe's System One
/// response shape, so Jev answers deserialize directly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RawAnswer {
    Choice {
        choice: String,
        #[serde(default)]
        probabilities: Option<BTreeMap<String, f64>>,
        #[serde(default)]
        confidence: Option<f64>,
    },
    Score {
        score: f64,
        #[serde(default)]
        probabilities: Option<BTreeMap<String, f64>>,
        #[serde(default)]
        confidence: Option<f64>,
    },
    Noul {
        noul: f64,
    },
    Abstain {
        reason: String,
    },
    /// Output an adapter could not parse. Admission rejects the whole response.
    Malformed {
        detail: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawDecision {
    pub request_id: String,
    pub engine: EngineDescriptor,
    pub answers: BTreeMap<String, RawAnswer>,
    pub latency_ms: u64,
}

/// A typed decision engine. Implementations live outside the trusted kernel.
pub trait DecisionEngine {
    fn descriptor(&self) -> EngineDescriptor;
    fn decide(&self, request: &DecisionRequest) -> Result<RawDecision>;
}

/// The default engine: abstains on everything, so callers must handle "unknown".
#[derive(Debug, Default, Clone, Copy)]
pub struct NoEngine;

impl DecisionEngine for NoEngine {
    fn descriptor(&self) -> EngineDescriptor {
        EngineDescriptor {
            name: "none".into(),
            version: "0".into(),
        }
    }

    fn decide(&self, request: &DecisionRequest) -> Result<RawDecision> {
        Ok(RawDecision {
            request_id: request.request_id(),
            engine: self.descriptor(),
            answers: request
                .questions
                .iter()
                .map(|q| {
                    (
                        q.id.clone(),
                        RawAnswer::Abstain {
                            reason: "no decision engine attached".into(),
                        },
                    )
                })
                .collect(),
            latency_ms: 0,
        })
    }
}

/// Replays fixed answers. Useful for tests, fixtures, and replaying a recorded engine response.
#[derive(Debug, Clone)]
pub struct StaticEngine {
    pub descriptor: EngineDescriptor,
    pub answers: BTreeMap<String, RawAnswer>,
}

impl DecisionEngine for StaticEngine {
    fn descriptor(&self) -> EngineDescriptor {
        self.descriptor.clone()
    }

    fn decide(&self, request: &DecisionRequest) -> Result<RawDecision> {
        Ok(RawDecision {
            request_id: request.request_id(),
            engine: self.descriptor.clone(),
            answers: self.answers.clone(),
            latency_ms: 0,
        })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdmittedAnswer {
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f64>,
        confidence: Option<f64>,
    },
    Score {
        /// Engine's expected level index in `[0, levels-1]`.
        score: f64,
        /// `score / (levels - 1)`, in `[0, 1]`.
        normalized: f64,
        /// Nearest level index.
        level: usize,
        probabilities: BTreeMap<String, f64>,
        confidence: Option<f64>,
    },
    Noul {
        p_true: f64,
    },
    Abstain {
        reason: String,
    },
}

impl AdmittedAnswer {
    pub fn is_abstain(&self) -> bool {
        matches!(self, Self::Abstain { .. })
    }

    pub fn p_true(&self) -> Option<f64> {
        match self {
            Self::Noul { p_true } => Some(*p_true),
            _ => None,
        }
    }

    pub fn normalized_score(&self) -> Option<f64> {
        match self {
            Self::Score { normalized, .. } => Some(*normalized),
            _ => None,
        }
    }
}

/// Only the kernel crate constructs this (through [`admit`] / [`ask`]).
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct AdmittedDecision {
    pub request_id: String,
    pub engine: EngineDescriptor,
    pub answers: BTreeMap<String, AdmittedAnswer>,
    /// Why the engine's whole response was rejected (or why the engine failed), if it was.
    pub rejected: Option<String>,
    pub latency_ms: u64,
    /// Engine probabilities are reported as-is and are not calibrated by the kernel.
    pub calibrated: bool,
}

impl AdmittedDecision {
    fn abstain_all(
        request: &DecisionRequest,
        engine: EngineDescriptor,
        reason: String,
        latency_ms: u64,
    ) -> Self {
        Self {
            request_id: request.request_id(),
            engine,
            answers: request
                .questions
                .iter()
                .map(|q| {
                    (
                        q.id.clone(),
                        AdmittedAnswer::Abstain {
                            reason: reason.clone(),
                        },
                    )
                })
                .collect(),
            rejected: Some(reason),
            latency_ms,
            calibrated: false,
        }
    }

    pub fn answer(&self, question_id: &str) -> Option<&AdmittedAnswer> {
        self.answers.get(question_id)
    }

    /// Build `Prediction` traces for every non-abstaining answer. They carry a `model:` source,
    /// are unverified, and are excluded from default recall and from consolidation.
    pub fn prediction_traces(&self, request: &DecisionRequest) -> Vec<Trace> {
        let mut traces = Vec::new();
        for question in &request.questions {
            let Some(answer) = self.answers.get(&question.id) else {
                continue;
            };
            let (answer_kind, value, p, probabilities) = match answer {
                AdmittedAnswer::Abstain { .. } => continue,
                AdmittedAnswer::Noul { p_true } => (
                    "noul",
                    if *p_true >= 0.5 { "true" } else { "false" }.to_string(),
                    *p_true,
                    BTreeMap::from([
                        ("true".to_string(), *p_true),
                        ("false".to_string(), 1.0 - p_true),
                    ]),
                ),
                AdmittedAnswer::Choice {
                    choice,
                    probabilities,
                    confidence,
                } => (
                    "choice",
                    choice.clone(),
                    probabilities
                        .get(choice)
                        .copied()
                        .or(*confidence)
                        .unwrap_or(0.5),
                    probabilities.clone(),
                ),
                AdmittedAnswer::Score {
                    level,
                    probabilities,
                    confidence,
                    ..
                } => (
                    "score",
                    level.to_string(),
                    probabilities
                        .get(&level.to_string())
                        .copied()
                        .or(*confidence)
                        .unwrap_or(0.5),
                    probabilities.clone(),
                ),
            };
            let mut trace = Trace::new(
                TraceKind::Prediction,
                format!(
                    "{} → {value} (p={p:.2}, {})",
                    question.instructions.trim(),
                    self.engine.source()
                ),
                self.engine.source(),
            );
            trace.tags = vec!["prediction".into(), question.family().to_string()];
            trace.salience = 0.3;
            trace.confidence = 0.5;
            trace.prediction = Some(PredictionRecord {
                request_id: self.request_id.clone(),
                question_id: question.id.clone(),
                family: question.family().to_string(),
                answer_kind: answer_kind.into(),
                engine: format!("{}@{}", self.engine.name, self.engine.version),
                p,
                value,
                probabilities,
                calibrated: false,
            });
            traces.push(trace);
        }
        traces
    }
}

/// Ask an engine. Invalid requests and credential-bearing state are refused before any call;
/// engine failures come back as an all-abstain decision with `rejected` set.
pub fn ask(engine: &dyn DecisionEngine, request: &DecisionRequest) -> Result<AdmittedDecision> {
    request.validate()?;
    if contains_secret(&request.state) {
        return Err(KernelError::Invalid(
            "decision state appears to contain a credential; refusing to send it to a decision engine"
                .into(),
        ));
    }
    let started = Instant::now();
    match engine.decide(request) {
        Ok(raw) => Ok(admit(request, raw)),
        Err(error) => Ok(AdmittedDecision::abstain_all(
            request,
            engine.descriptor(),
            format!("engine error: {error}"),
            started.elapsed().as_millis() as u64,
        )),
    }
}

/// Validate a raw engine response against the request. All-or-nothing.
pub fn admit(request: &DecisionRequest, raw: RawDecision) -> AdmittedDecision {
    match admit_inner(request, &raw) {
        Ok(answers) => AdmittedDecision {
            request_id: raw.request_id,
            engine: raw.engine,
            answers,
            rejected: None,
            latency_ms: raw.latency_ms,
            calibrated: false,
        },
        Err(detail) => AdmittedDecision::abstain_all(
            request,
            raw.engine,
            format!("response rejected: {detail}"),
            raw.latency_ms,
        ),
    }
}

fn admit_inner(
    request: &DecisionRequest,
    raw: &RawDecision,
) -> std::result::Result<BTreeMap<String, AdmittedAnswer>, String> {
    if raw.request_id != request.request_id() {
        return Err(format!(
            "request id {} does not match {}",
            raw.request_id,
            request.request_id()
        ));
    }
    for id in raw.answers.keys() {
        if request.question(id).is_none() {
            return Err(format!("answered a question that was not asked: {id}"));
        }
    }
    let mut admitted = BTreeMap::new();
    for question in &request.questions {
        let answer = match raw.answers.get(&question.id) {
            None => AdmittedAnswer::Abstain {
                reason: "engine did not answer".into(),
            },
            Some(answer) => admit_answer(question, answer)?,
        };
        admitted.insert(question.id.clone(), answer);
    }
    Ok(admitted)
}

fn unit(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn check_confidence(
    question: &Question,
    confidence: Option<f64>,
) -> std::result::Result<(), String> {
    match confidence {
        Some(c) if !unit(c) => Err(format!("{}: confidence {c} is not in [0,1]", question.id)),
        _ => Ok(()),
    }
}

fn check_distribution(
    question: &Question,
    probabilities: &BTreeMap<String, f64>,
    allowed: impl Fn(&str) -> bool,
) -> std::result::Result<(), String> {
    if probabilities.is_empty() {
        return Ok(());
    }
    let mut sum = 0.0;
    for (key, p) in probabilities {
        if !allowed(key) {
            return Err(format!(
                "{}: probability for unknown option `{key}`",
                question.id
            ));
        }
        if !unit(*p) {
            return Err(format!(
                "{}: probability {p} for `{key}` is not in [0,1]",
                question.id
            ));
        }
        sum += p;
    }
    if (sum - 1.0).abs() > PROBABILITY_SUM_TOLERANCE {
        return Err(format!(
            "{}: probabilities sum to {sum:.3}, not 1",
            question.id
        ));
    }
    Ok(())
}

fn admit_answer(
    question: &Question,
    answer: &RawAnswer,
) -> std::result::Result<AdmittedAnswer, String> {
    match (answer, &question.kind) {
        (RawAnswer::Malformed { detail }, _) => {
            Err(format!("{}: malformed answer: {detail}", question.id))
        }
        (RawAnswer::Abstain { reason }, _) => Ok(AdmittedAnswer::Abstain {
            reason: reason.clone(),
        }),
        (RawAnswer::Noul { noul }, QuestionKind::Noul { .. }) => {
            if unit(*noul) {
                Ok(AdmittedAnswer::Noul { p_true: *noul })
            } else {
                Err(format!("{}: noul {noul} is not in [0,1]", question.id))
            }
        }
        (
            RawAnswer::Choice {
                choice,
                probabilities,
                confidence,
            },
            QuestionKind::Choice { options },
        ) => {
            if !options.contains_key(choice) {
                return Err(format!(
                    "{}: `{choice}` is not one of the offered options",
                    question.id
                ));
            }
            check_confidence(question, *confidence)?;
            let probabilities = probabilities.clone().unwrap_or_default();
            check_distribution(question, &probabilities, |k| options.contains_key(k))?;
            Ok(AdmittedAnswer::Choice {
                choice: choice.clone(),
                probabilities,
                confidence: *confidence,
            })
        }
        (
            RawAnswer::Score {
                score,
                probabilities,
                confidence,
            },
            QuestionKind::Score { levels },
        ) => {
            let top = (levels.len() - 1) as f64;
            if !score.is_finite() || *score < 0.0 || *score > top {
                return Err(format!(
                    "{}: score {score} is outside [0, {top}]",
                    question.id
                ));
            }
            check_confidence(question, *confidence)?;
            let probabilities = probabilities.clone().unwrap_or_default();
            check_distribution(question, &probabilities, |k| {
                k.parse::<usize>()
                    .map(|i| i < levels.len())
                    .unwrap_or(false)
            })?;
            Ok(AdmittedAnswer::Score {
                score: *score,
                normalized: score / top,
                level: score.round() as usize,
                probabilities,
                confidence: *confidence,
            })
        }
        (answer, kind) => Err(format!(
            "{}: answer type does not match a {} question: {answer:?}",
            question.id,
            kind.name()
        )),
    }
}
