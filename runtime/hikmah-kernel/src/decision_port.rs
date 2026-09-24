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
//! - A person or agent records a [`Forecast`] in the same shape, under its own source, so its
//!   calibration can be compared with an engine's on the same family.
use crate::error::{KernelError, Result};
use crate::secrets::contains_secret;
use crate::trace::{PredictionRecord, Trace, TraceKind, MODEL_SOURCE_PREFIX};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

pub const MAX_QUESTIONS: usize = 32;
pub const MAX_STATE_CHARS: usize = 32_000;
/// Engines round probabilities (Jev reports two decimals), so the allowed deviation of the sum
/// from 1 grows with the number of keys.
fn sum_tolerance(keys: usize) -> f64 {
    (0.005 * keys as f64).max(0.02) + 1e-9
}

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

    /// Every piece of text that would be sent to an engine.
    pub fn outbound_texts(&self) -> Vec<&str> {
        let mut texts = vec![self.state.as_str()];
        for question in &self.questions {
            texts.push(&question.instructions);
            if let Some(family) = &question.family {
                texts.push(family);
            }
            match &question.kind {
                QuestionKind::Choice { options } => {
                    for (id, meaning) in options {
                        texts.push(id);
                        texts.push(meaning);
                    }
                }
                QuestionKind::Score { levels } => texts.extend(levels.iter().map(String::as_str)),
                QuestionKind::Noul { if_true, if_false } => {
                    texts.extend(if_true.as_deref());
                    texts.extend(if_false.as_deref());
                }
            }
        }
        texts
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
    /// `name@version`, trimmed: the forecaster a prediction record names. An engine reports its
    /// own version (Jev's response `model` field), so surrounding whitespace is dropped here,
    /// once, and the trace source and the record always name the same engine.
    pub fn identity(&self) -> String {
        format!("{}@{}", self.name.trim(), self.version.trim())
    }

    /// `model:<identity>`: the provenance source of this engine's answers.
    pub fn source(&self) -> String {
        format!("{MODEL_SOURCE_PREFIX}{}", self.identity())
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
                    Some(*p_true),
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
                    probabilities.get(choice).copied().or(*confidence),
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
                        .or(*confidence),
                    probabilities.clone(),
                ),
            };
            let answer_space: Vec<String> = match &question.kind {
                QuestionKind::Noul { .. } => vec!["true".into(), "false".into()],
                QuestionKind::Choice { options } => options.keys().cloned().collect(),
                QuestionKind::Score { levels } => {
                    (0..levels.len()).map(|i| i.to_string()).collect()
                }
            };
            let shown_p = p
                .map(|p| format!("{p:.2}"))
                .unwrap_or_else(|| "unknown".into());
            let mut trace = Trace::new(
                TraceKind::Prediction,
                format!(
                    "{} → {value} (p={shown_p}, {})",
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
                engine: self.engine.identity(),
                p,
                value,
                probabilities,
                answer_space,
                calibrated: false,
            });
            traces.push(trace);
        }
        traces
    }
}

/// A forecast made by a person or an agent (`hikmah predict`), recorded in the same
/// [`PredictionRecord`] shape as an engine answer so `hikmah calibration` can score both on the
/// same family. The record's `engine` is the source principal (for example `human:alex`), and
/// calibration keys rows by source class as well as name, so the two always get separate rows.
/// A forecast is never verified; an outcome resolves it.
#[derive(Debug, Clone, PartialEq)]
pub struct Forecast {
    /// Calibration bucket, shared with any engine that forecasts the same thing.
    pub family: String,
    /// What is being forecast, in words.
    pub question: String,
    /// `noul`, `choice`, or `score`.
    pub kind: String,
    /// Noul: probability of `true`. Choice and score: probability of `value`.
    pub p: f64,
    /// Choice and score: the forecast answer, one of `answer_space`. Noul: none (it follows
    /// from `p`).
    pub value: Option<String>,
    /// Choice: the option ids. Score: the levels, lowest first. Noul: empty (`true`/`false`).
    pub answer_space: Vec<String>,
    /// Who forecast, as `<kind>:<name>`: for example `human:alex` or `agent:planner`. Never a
    /// `model:` source.
    pub source: String,
    /// Where the forecast was made, for example a decision record.
    pub locator: Option<String>,
}

impl Forecast {
    /// Validate the forecast and build its `prediction` trace. Engine answers (`model:`
    /// sources) are refused here: they are recorded from an admitted decision.
    pub fn into_trace(self) -> Result<Trace> {
        let invalid = |message: String| Err(KernelError::Invalid(message));
        let source = self.source.trim().to_string();
        let family = self.family.trim().to_string();
        let question = self.question.trim().to_string();
        if source.is_empty() || source.eq_ignore_ascii_case("unknown") {
            return invalid("a forecast needs a named source, for example `human:<name>`".into());
        }
        if source.to_ascii_lowercase().starts_with(MODEL_SOURCE_PREFIX) {
            return invalid(format!(
                "`{MODEL_SOURCE_PREFIX}` sources are engine answers; record them with `hikmah ask --record` or `hikmah decide --record`"
            ));
        }
        // A principal names its kind (`human:alex`, `agent:planner`), so a forecaster's row never
        // reads like an engine identity such as `jev@jev-1.13.0`. Calibration also keeps the two
        // classes apart by source (`forecaster_kind`), whatever a principal calls itself.
        let named_principal = source.split_once(':').is_some_and(|(kind, name)| {
            !kind.is_empty()
                && kind
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                && !name.trim().is_empty()
        });
        if !named_principal {
            return invalid(format!(
                "forecast source `{source}` must be `<kind>:<name>`, for example `human:alex` or `agent:planner`"
            ));
        }
        if family.is_empty() || question.is_empty() {
            return invalid("a forecast needs a family and a question".into());
        }
        if !unit(self.p) {
            return invalid(format!("--p {} is not a probability in [0, 1]", self.p));
        }
        let kind = self.kind.trim().to_ascii_lowercase();
        let (value, answer_space, probabilities) = match kind.as_str() {
            "noul" => {
                if self.value.is_some() || !self.answer_space.is_empty() {
                    return invalid(
                        "a noul forecast is P(true) in --p; it takes no --value or --answer-space"
                            .into(),
                    );
                }
                (
                    if self.p >= 0.5 { "true" } else { "false" }.to_string(),
                    vec!["true".to_string(), "false".to_string()],
                    BTreeMap::from([
                        ("true".to_string(), self.p),
                        ("false".to_string(), 1.0 - self.p),
                    ]),
                )
            }
            "choice" | "score" => {
                let space: Vec<String> = self
                    .answer_space
                    .iter()
                    .map(|v| v.trim().to_string())
                    .collect();
                let bounds = if kind == "choice" { 2..=255 } else { 2..=10 };
                let distinct: BTreeSet<&str> = space.iter().map(String::as_str).collect();
                if !bounds.contains(&space.len())
                    || distinct.len() != space.len()
                    || distinct.contains("")
                {
                    return invalid(format!(
                        "a {kind} forecast needs --answer-space with {}..={} distinct, non-empty values",
                        bounds.start(),
                        bounds.end()
                    ));
                }
                let Some(value) = self.value.as_deref().map(str::trim) else {
                    return invalid(format!("a {kind} forecast needs --value"));
                };
                if !distinct.contains(value) {
                    return invalid(format!("--value `{value}` is not one of {space:?}"));
                }
                // Only P(value) is known. The rest of the distribution is not invented, so
                // calibration scores this forecast on its top label.
                (value.to_string(), space, BTreeMap::new())
            }
            other => {
                return invalid(format!(
                    "unknown forecast type `{other}` (expected noul, choice, or score)"
                ))
            }
        };
        let created_at_ms = crate::trace::now_ms();
        let seed = format!("{source}\n{family}\n{question}\n{created_at_ms}");
        let request_id = format!("fc_{}", &blake3::hash(seed.as_bytes()).to_hex()[..16]);
        let mut trace = Trace::new(
            TraceKind::Prediction,
            format!("{question} → {value} (p={:.2}, {source})", self.p),
            source.clone(),
        );
        trace.created_at_ms = created_at_ms;
        trace.tags = vec!["prediction".into(), family.clone()];
        trace.salience = 0.3;
        trace.confidence = 0.5;
        trace.provenance.locator = self.locator;
        trace.prediction = Some(PredictionRecord {
            request_id,
            question_id: family.clone(),
            family,
            answer_kind: kind,
            engine: source,
            p: Some(self.p),
            value,
            probabilities,
            answer_space,
            calibrated: false,
        });
        trace.validate()?;
        Ok(trace)
    }
}

/// Ask an engine. Invalid requests and credential-bearing state are refused before any call;
/// engine failures come back as an all-abstain decision with `rejected` set.
pub fn ask(engine: &dyn DecisionEngine, request: &DecisionRequest) -> Result<AdmittedDecision> {
    request.validate()?;
    if request.outbound_texts().into_iter().any(contains_secret) {
        return Err(KernelError::Invalid(
            "the request appears to contain a credential; refusing to send it to a decision engine"
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
    if (sum - 1.0).abs() > sum_tolerance(probabilities.len()) {
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
            if !probabilities.is_empty() {
                let chosen = probabilities.get(choice).copied().unwrap_or(0.0);
                let best = probabilities.values().copied().fold(0.0_f64, f64::max);
                if chosen <= 0.0 || chosen + 1e-9 < best {
                    return Err(format!(
                        "{}: `{choice}` is not the most probable option in its own distribution",
                        question.id
                    ));
                }
            }
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
            // Level keys must be canonical indices ("0", "1", ...), never aliases like "01".
            check_distribution(question, &probabilities, |k| {
                k.parse::<usize>()
                    .map(|i| i < levels.len() && k == i.to_string())
                    .unwrap_or(false)
            })?;
            if !probabilities.is_empty() {
                let expected: f64 = probabilities
                    .iter()
                    .map(|(k, p)| k.parse::<f64>().unwrap_or(0.0) * p)
                    .sum();
                let tolerance = 0.02 * top + 0.02;
                if (expected - score).abs() > tolerance {
                    return Err(format!(
                        "{}: score {score} disagrees with its distribution (expected {expected:.3})",
                        question.id
                    ));
                }
            }
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
