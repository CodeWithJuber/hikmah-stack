//! TypeSafe Jev (System One) adapter for the typed decision port.
//!
//! API contract: `POST {base}/v1/systemone` with bearer auth and
//! `{ model, state, questions: { id: { type, instructions, criteria } } }`; answers come back as
//! `{ model, answers: { id: { type, ... } }, usage }` (see <https://docs.typesafe.ai/api.md>).
//!
//! - Opt-in: nothing here runs unless an engine is selected and `TYPESAFE_API_KEY` is set.
//! - The key is never logged; `Debug` redacts it and errors never include request headers.
//! - The kernel's [`crate::decision_port::ask`] refuses credential-bearing state before this
//!   adapter is called, and [`crate::decision_port::admit`] validates every answer afterwards.
//! - 429/529/5xx are retried with backoff inside the time budget; other failures return an error,
//!   which `ask` turns into an all-abstain decision so callers fall back to deterministic rules.
use crate::decision_port::{
    DecisionEngine, DecisionRequest, EngineDescriptor, QuestionKind, RawAnswer, RawDecision,
};
use crate::error::{KernelError, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::time::{Duration, Instant};

pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";
pub const DEFAULT_MODEL: &str = "jev-latest";
pub const DEFAULT_TIMEOUT_MS: u64 = 5_000;

/// HTTP transport, injectable for tests. Returns `(status, json_body)`.
pub trait Transport: Send + Sync {
    fn post_json(
        &self,
        url: &str,
        api_key: &str,
        body: &Value,
        timeout: Duration,
    ) -> std::result::Result<(u16, Value), String>;
}

/// Production transport: `ureq` with the platform certificate verifier (honours system roots,
/// `SSL_CERT_FILE`, and corporate proxies) and proxy settings from the environment.
#[derive(Debug, Default, Clone, Copy)]
pub struct UreqTransport;

impl Transport for UreqTransport {
    fn post_json(
        &self,
        url: &str,
        api_key: &str,
        body: &Value,
        timeout: Duration,
    ) -> std::result::Result<(u16, Value), String> {
        use ureq::tls::{RootCerts, TlsConfig};
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into();
        let mut response = agent
            .post(url)
            .header("authorization", &format!("Bearer {api_key}"))
            .header("content-type", "application/json")
            .send_json(body)
            .map_err(|error| format!("request failed: {error}"))?;
        let status = response.status().as_u16();
        let value = response
            .body_mut()
            .read_json::<Value>()
            .unwrap_or(Value::Null);
        Ok((status, value))
    }
}

pub struct JevEngine {
    api_key: String,
    base_url: String,
    model: String,
    timeout: Duration,
    max_retries: u32,
    transport: Box<dyn Transport>,
}

impl fmt::Debug for JevEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JevEngine")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl JevEngine {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.into(),
            model: DEFAULT_MODEL.into(),
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            max_retries: 2,
            transport: Box::new(UreqTransport),
        }
    }

    /// Build from `TYPESAFE_API_KEY` (required), `TYPESAFE_BASE_URL`, `HIKMAH_JEV_MODEL`, and
    /// `HIKMAH_JEV_TIMEOUT_MS`. Returns `None` when no key is configured.
    pub fn from_env() -> Option<Self> {
        let key = std::env::var("TYPESAFE_API_KEY").ok()?;
        if key.trim().is_empty() {
            return None;
        }
        let mut engine = Self::new(key.trim());
        if let Ok(url) = std::env::var("TYPESAFE_BASE_URL") {
            if !url.trim().is_empty() {
                engine.base_url = url.trim().trim_end_matches('/').to_string();
            }
        }
        if let Ok(model) = std::env::var("HIKMAH_JEV_MODEL") {
            if !model.trim().is_empty() {
                engine.model = model.trim().to_string();
            }
        }
        if let Some(ms) = std::env::var("HIKMAH_JEV_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
        {
            engine.timeout = Duration::from_millis(ms.clamp(100, 60_000));
        }
        Some(engine)
    }

    pub fn with_transport(mut self, transport: Box<dyn Transport>) -> Self {
        self.transport = transport;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The System One request body for a kernel request.
    pub fn payload(&self, request: &DecisionRequest) -> Value {
        let mut questions = Map::new();
        for question in &request.questions {
            let mut spec = Map::new();
            spec.insert("instructions".into(), json!(question.instructions));
            match &question.kind {
                QuestionKind::Choice { options } => {
                    spec.insert("type".into(), json!("choice"));
                    spec.insert("criteria".into(), json!(options));
                }
                QuestionKind::Score { levels } => {
                    spec.insert("type".into(), json!("score"));
                    spec.insert("criteria".into(), json!(levels));
                }
                QuestionKind::Noul { if_true, if_false } => {
                    spec.insert("type".into(), json!("noul"));
                    if let (Some(t), Some(f)) = (if_true, if_false) {
                        spec.insert("criteria".into(), json!({"true": t, "false": f}));
                    }
                }
            }
            questions.insert(question.id.clone(), Value::Object(spec));
        }
        json!({
            "model": self.model,
            "state": request.state,
            "questions": Value::Object(questions),
        })
    }

    fn post_with_retries(&self, body: &Value) -> Result<Value> {
        let url = format!("{}/v1/systemone", self.base_url);
        let deadline = Instant::now() + self.timeout;
        let mut attempt = 0_u32;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(KernelError::Engine("jev: time budget exhausted".into()));
            }
            let (status, value) = self
                .transport
                .post_json(&url, &self.api_key, body, remaining)
                .map_err(|error| KernelError::Engine(format!("jev: {error}")))?;
            if (200..300).contains(&status) {
                return Ok(value);
            }
            let retryable = status == 429 || status == 529 || status >= 500;
            if retryable && attempt < self.max_retries {
                let backoff = Duration::from_millis(250 * 2_u64.pow(attempt));
                if Instant::now() + backoff < deadline {
                    std::thread::sleep(backoff);
                    attempt += 1;
                    continue;
                }
            }
            let kind = value
                .pointer("/detail/error_type")
                .and_then(Value::as_str)
                .unwrap_or("error");
            return Err(KernelError::Engine(format!("jev: http {status} ({kind})")));
        }
    }
}

impl DecisionEngine for JevEngine {
    fn descriptor(&self) -> EngineDescriptor {
        EngineDescriptor {
            name: "jev".into(),
            version: self.model.clone(),
        }
    }

    fn decide(&self, request: &DecisionRequest) -> Result<RawDecision> {
        let started = Instant::now();
        let body = self.payload(request);
        let response = self.post_with_retries(&body)?;
        let answers_value = response
            .get("answers")
            .and_then(Value::as_object)
            .ok_or_else(|| KernelError::Engine("jev: response has no answers object".into()))?;
        let mut answers = BTreeMap::new();
        for (id, value) in answers_value {
            let answer =
                serde_json::from_value::<RawAnswer>(value.clone()).unwrap_or_else(|error| {
                    RawAnswer::Malformed {
                        detail: format!("{error}"),
                    }
                });
            answers.insert(id.clone(), answer);
        }
        let version = response
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(&self.model)
            .to_string();
        Ok(RawDecision {
            request_id: request.request_id(),
            engine: EngineDescriptor {
                name: "jev".into(),
                version,
            },
            answers,
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }
}
