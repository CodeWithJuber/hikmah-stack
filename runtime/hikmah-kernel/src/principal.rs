//! Who is writing: detect an AI agent session from the environment its host sets.
//!
//! `source` and `verified` on a trace are claims made by whoever runs the command; the kernel does
//! not authenticate them. An agent could otherwise write `--source human:x --verified` and turn
//! its own output into "verified" memory. When a coding-agent host's variables are present
//! (`CLAUDECODE`, `CLAUDE_CODE_*`, `CODEX_*`, `CURSOR_*`, `GEMINI_CLI`, `AI_AGENT`), the CLI
//! [`stamp`](AgentSession::stamp)s the trace: its locator records `agent-session:<host>:<id>` and
//! it cannot be marked verified. [`Trace::validate`](crate::trace::Trace::validate) enforces the
//! second half for every write path. The CLI also refuses, in such a session, the commands that
//! accept ledger records no hikmah write acknowledged (`verify-ledger --accept-tail` and
//! `--reset-head`; see [`AgentSession::refuse_person_only`]): those records may be a forged append,
//! and an agent that follows a refusal message must not be the one to approve them.
//!
//! This is a guard against self-verification by default, not authentication. A process that clears
//! these variables is not detected. A variable a person's own shell or IDE terminal sets counts as
//! a marker (except the documented configuration settings in [`USER_CONFIGURATION`]); the refusal
//! names it so that person can run the command from a shell without it. Authenticated attestation,
//! meaning a signature with a key the agent cannot read, is not implemented.
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

/// Configuration a person commonly exports in their own shell profile (documented settings of the
/// host tools). These configure the tool; they do not show that an agent is running this process,
/// so they are not markers. Claude Code always sets `CLAUDECODE` in the shells it runs, so leaving
/// its settings out does not hide a Claude Code session.
pub const USER_CONFIGURATION: &[&str] = &[
    "CODEX_HOME",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
    "CLAUDE_CODE_SKIP_VERTEX_AUTH",
    "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
    "CLAUDE_CODE_ENABLE_TELEMETRY",
    "CLAUDE_CODE_ENHANCED_TELEMETRY_BETA",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_CLIENT_CERT",
    "CLAUDE_CODE_CLIENT_KEY",
    "CLAUDE_CODE_CLIENT_KEY_PASSPHRASE",
    "CLAUDE_CODE_GIT_BASH_PATH",
];

/// Where a person whose own shell is mistaken for an agent session finds how to proceed.
const FALSE_POSITIVE_HELP: &str = "\"Agent sessions\" in docs/MEMORY.md";
/// A refusal names at most this many marker variables (an agent host can set dozens).
const LISTED_MARKERS: usize = 5;

const MAX_SESSION_ID: usize = 64;

/// An AI agent session detected from environment variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSession {
    /// `claude-code`, `codex`, `cursor`, `gemini-cli`, or `ai-agent`.
    pub host: &'static str,
    /// The environment variable that identified the session.
    pub marker: String,
    /// Every non-empty marker variable present, for any host, sorted by name.
    pub markers: Vec<String>,
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
                "--verified is refused inside an AI agent session ({}). `verified` means a person checked the claim, and an agent cannot attest its own memory; `--source` does not change that. Record it without --verified (it is stored with locator {}). To verify it, a person runs `hikmah remember --verified --supersedes <trace id> ...` from their own terminal, outside the agent session. {}",
                self.detected(),
                self.locator(),
                self.false_positive_note()
            )));
        }
        trace.provenance.locator = Some(match trace.provenance.locator.take() {
            Some(given) if !given.trim().is_empty() => format!("{}; {given}", self.locator()),
            _ => self.locator(),
        });
        Ok(())
    }

    /// The refusal for a command only a person may run inside this session, such as accepting
    /// ledger records no hikmah write acknowledged (`verify-ledger --accept-tail` or
    /// `--reset-head`). Those records may be a forged append (the chain is unkeyed), and an agent
    /// that follows a refusal message must not be the one to approve them. The message tells the
    /// agent to stop and ask a person; it deliberately gives no command that gets around the check.
    pub fn refuse_person_only(&self, command: &str, why: &str) -> KernelError {
        KernelError::Invalid(format!(
            "`{command}` is refused inside an AI agent session ({}): {why}. An agent must stop here and ask a person. The person inspects what `hikmah verify-ledger` lists and runs `{command}` from their own terminal, outside the agent session. {}",
            self.detected(),
            self.false_positive_note()
        ))
    }

    fn detected(&self) -> String {
        format!("{} detected through {}", self.host, self.marker)
    }

    fn false_positive_note(&self) -> String {
        let mut listed: Vec<&str> = self
            .markers
            .iter()
            .take(LISTED_MARKERS)
            .map(String::as_str)
            .collect();
        let more;
        if self.markers.len() > LISTED_MARKERS {
            more = format!("and {} more", self.markers.len() - LISTED_MARKERS);
            listed.push(&more);
        }
        format!(
            "Agent variables set: {}. If you are a person and your own shell profile or IDE terminal sets them, see {FALSE_POSITIVE_HELP}",
            listed.join(", ")
        )
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
    let markers: Vec<String> = vars
        .keys()
        .filter(|name| is_agent_marker(name))
        .cloned()
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
            markers: markers.clone(),
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
