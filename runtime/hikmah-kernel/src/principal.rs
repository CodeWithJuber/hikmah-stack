//! Who is writing: detect an AI agent session from the environment its host sets.
//!
//! `source` and `verified` on a trace are claims made by whoever runs the command; the kernel does
//! not authenticate them. An agent could otherwise write `--source human:x --verified` and turn
//! its own output into "verified" memory. When a coding-agent host's variables are present
//! (`CLAUDECODE`, `CLAUDE_CODE_*`, `CODEX_*`, `CURSOR_*`, `GEMINI_CLI`, `AI_AGENT`), the CLI
//! [`stamp`](AgentSession::stamp)s the trace: its locator records `agent-session:<host>:<id>` and
//! it cannot be marked verified. [`Trace::validate`](crate::trace::Trace::validate) enforces the
//! second half for every write path.
//!
//! This is a guard against self-verification by default, not authentication. A process that clears
//! these variables is not detected, and a variable a person exports in their own shell profile
//! counts as a marker (except the common configuration settings in [`USER_CONFIGURATION`]).
//! Authenticated attestation, meaning a signature with a key the agent cannot read, is not
//! implemented.
use crate::error::{KernelError, Result};
use crate::trace::Trace;
use std::collections::BTreeMap;

/// Locator prefix of a trace written from a detected agent session.
pub const AGENT_LOCATOR_PREFIX: &str = "agent-session:";

struct Host {
    name: &'static str,
    exact: &'static [&'static str],
    prefixes: &'static [&'static str],
    /// Variables that carry the host's session id, in order of preference.
    session_vars: &'static [&'static str],
}

const HOSTS: &[Host] = &[
    Host {
        name: "claude-code",
        exact: &["CLAUDECODE"],
        prefixes: &["CLAUDE_CODE_"],
        session_vars: &["CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_REMOTE_SESSION_ID"],
    },
    Host {
        name: "codex",
        exact: &[],
        prefixes: &["CODEX_"],
        session_vars: &["CODEX_SESSION_ID", "CODEX_THREAD_ID"],
    },
    Host {
        name: "cursor",
        exact: &[],
        prefixes: &["CURSOR_"],
        session_vars: &["CURSOR_SESSION_ID", "CURSOR_TRACE_ID"],
    },
    Host {
        name: "gemini-cli",
        exact: &["GEMINI_CLI"],
        prefixes: &[],
        session_vars: &[],
    },
    Host {
        name: "ai-agent",
        exact: &["AI_AGENT"],
        prefixes: &[],
        session_vars: &[],
    },
];

/// Configuration a person commonly exports in their own shell profile. These configure the tool;
/// they do not show that an agent is running this process, so they are not markers.
pub const USER_CONFIGURATION: &[&str] = &[
    "CODEX_HOME",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
];

const MAX_SESSION_ID: usize = 64;

/// An AI agent session detected from environment variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSession {
    /// `claude-code`, `codex`, `cursor`, `gemini-cli`, or `ai-agent`.
    pub host: &'static str,
    /// The environment variable that identified the session.
    pub marker: String,
    /// The host's session id (ASCII letters, digits, `-`, `_`, `.`; at most 64), when it sets one.
    pub session_id: Option<String>,
}

impl AgentSession {
    /// `agent-session:<host>:<session id or "unknown">`.
    pub fn locator(&self) -> String {
        format!(
            "{AGENT_LOCATOR_PREFIX}{}:{}",
            self.host,
            self.session_id.as_deref().unwrap_or("unknown")
        )
    }

    /// Apply the agent-session rule to a trace this session is about to write: refuse `verified`
    /// (an agent cannot attest its own memory) and record the session as the locator, keeping any
    /// caller locator after it.
    pub fn stamp(&self, trace: &mut Trace) -> Result<()> {
        if trace.provenance.verified {
            return Err(KernelError::Invalid(format!(
                "--verified is refused inside an AI agent session ({} detected through {}). `verified` means a person checked the claim, and an agent cannot attest its own memory; `--source` does not change that. Record it without --verified (it is stored with locator {}). To verify it, a person runs `hikmah remember --verified --supersedes <trace id> ...` from their own terminal, outside the agent session",
                self.host,
                self.marker,
                self.locator()
            )));
        }
        trace.provenance.locator = Some(match trace.provenance.locator.take() {
            Some(given) if !given.trim().is_empty() => format!("{}; {given}", self.locator()),
            _ => self.locator(),
        });
        Ok(())
    }
}

/// True when `name` alone marks an agent session (an empty value does not; see [`detect`]).
pub fn is_agent_marker(name: &str) -> bool {
    host_of(name).is_some()
}

fn host_of(name: &str) -> Option<&'static Host> {
    if USER_CONFIGURATION.contains(&name) {
        return None;
    }
    HOSTS.iter().find(|host| {
        host.exact.contains(&name) || host.prefixes.iter().any(|p| name.starts_with(p))
    })
}

/// Detect an agent session from `(name, value)` pairs. Variables with an empty value are ignored.
/// Deterministic: hosts are checked in a fixed order and variable names in sorted order.
pub fn detect<I, K, V>(vars: I) -> Option<AgentSession>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    let vars: BTreeMap<String, String> = vars
        .into_iter()
        .filter(|(_, value)| !value.as_ref().trim().is_empty())
        .map(|(name, value)| (name.as_ref().to_string(), value.as_ref().to_string()))
        .collect();
    HOSTS.iter().find_map(|host| {
        let marker = vars
            .keys()
            .find(|name| host_of(name).is_some_and(|h| h.name == host.name))?;
        let session_id = host
            .session_vars
            .iter()
            .filter_map(|var| vars.get(*var))
            .find_map(|value| sanitize_id(value));
        Some(AgentSession {
            host: host.name,
            marker: marker.clone(),
            session_id,
        })
    })
}

/// [`detect`] over this process's environment (variables that are not valid Unicode are skipped).
pub fn detect_from_env() -> Option<AgentSession> {
    detect(std::env::vars_os().filter_map(|(name, value)| {
        Some((
            name.into_string().ok()?,
            value.to_string_lossy().into_owned(),
        ))
    }))
}

fn sanitize_id(value: &str) -> Option<String> {
    let id: String = value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(MAX_SESSION_ID)
        .collect();
    (!id.is_empty()).then_some(id)
}
