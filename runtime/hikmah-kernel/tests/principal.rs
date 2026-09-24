mod common;

use common::{temp_store, without_agent_session};
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::principal::{detect, AgentSession};
use hikmah_kernel::trace::{Trace, TraceKind, TraceStatus};
use hikmah_kernel::{KernelError, MemoryStore};
use serde_json::Value;
use std::process::{Command, Output};

fn env(pairs: &[(&str, &str)]) -> Option<AgentSession> {
    detect(pairs.iter().copied())
}

#[test]
fn agent_sessions_are_detected_from_host_markers() {
    let claude = env(&[
        ("PATH", "/usr/bin"),
        ("CLAUDECODE", "1"),
        ("CLAUDE_CODE_SESSION_ID", "30a3c8d2-fdc9-5a96"),
        // A configuration setting is still not listed as a marker inside a session.
        ("CLAUDE_CODE_ENABLE_TELEMETRY", "1"),
    ])
    .unwrap();
    assert_eq!(claude.host, "claude-code");
    assert_eq!(claude.marker, "CLAUDECODE");
    assert_eq!(claude.markers, ["CLAUDECODE", "CLAUDE_CODE_SESSION_ID"]);
    assert_eq!(
        claude.locator(),
        "agent-session:claude-code:30a3c8d2-fdc9-5a96"
    );

    // Any CLAUDE_CODE_* runtime variable is enough, even without CLAUDECODE.
    let entry = env(&[("CLAUDE_CODE_ENTRYPOINT", "cli")]).unwrap();
    assert_eq!(entry.locator(), "agent-session:claude-code:unknown");

    let codex = env(&[("CODEX_SANDBOX", "seatbelt"), ("CODEX_THREAD_ID", "t-9")]).unwrap();
    assert_eq!(codex.locator(), "agent-session:codex:t-9");
    assert_eq!(env(&[("CURSOR_AGENT", "1")]).unwrap().host, "cursor");
    assert_eq!(env(&[("GEMINI_CLI", "1")]).unwrap().host, "gemini-cli");
    assert_eq!(env(&[("AI_AGENT", "some-agent")]).unwrap().host, "ai-agent");

    // The host order is fixed, whatever order the variables arrive in.
    let both = env(&[
        ("AI_AGENT", "x"),
        ("CODEX_SANDBOX", "1"),
        ("CLAUDECODE", "1"),
    ])
    .unwrap();
    assert_eq!(both.host, "claude-code");
    // Every marker is listed, whichever host identified the session.
    assert_eq!(both.markers, ["AI_AGENT", "CLAUDECODE", "CODEX_SANDBOX"]);
}

#[test]
fn a_person_s_shell_is_not_an_agent_session() {
    assert_eq!(env(&[]), None);
    assert_eq!(
        env(&[
            ("PATH", "/usr/bin"),
            ("HOME", "/home/me"),
            ("TERM_PROGRAM", "vscode"),
            // Configuration people export in their own shell profile.
            ("CODEX_HOME", "/home/me/.codex"),
            ("CLAUDE_CODE_USE_BEDROCK", "1"),
            ("CLAUDE_CODE_MAX_OUTPUT_TOKENS", "8192"),
            ("CLAUDE_CODE_ENABLE_TELEMETRY", "1"),
            ("CLAUDE_CODE_USE_FOUNDRY", "1"),
            ("CLAUDE_CODE_OAUTH_TOKEN", "placeholder"),
            (
                "CLAUDE_CODE_GIT_BASH_PATH",
                "C:/Program Files/Git/bin/bash.exe"
            ),
            // Set but empty.
            ("CLAUDECODE", ""),
            ("AI_AGENT", "  "),
            // Similar names that are not markers.
            ("MY_CLAUDECODE", "1"),
            ("CLAUDE", "1"),
        ]),
        None
    );
}

#[test]
fn session_ids_are_sanitized_and_bounded() {
    let long = "a".repeat(200);
    let session = env(&[("CLAUDECODE", "1"), ("CLAUDE_CODE_SESSION_ID", &long)]).unwrap();
    assert_eq!(session.session_id.as_deref().map(str::len), Some(64));

    let odd = env(&[
        ("CLAUDECODE", "1"),
        ("CLAUDE_CODE_SESSION_ID", "id; rm -rf / \n$(x)"),
    ])
    .unwrap();
    assert_eq!(odd.session_id.as_deref(), Some("idrm-rfx"));

    // Nothing usable left: falls back to the next variable, then to "unknown".
    let fallback = env(&[
        ("CLAUDECODE", "1"),
        ("CLAUDE_CODE_SESSION_ID", "!!!"),
        ("CLAUDE_CODE_REMOTE_SESSION_ID", "remote-1"),
    ])
    .unwrap();
    assert_eq!(fallback.session_id.as_deref(), Some("remote-1"));
    let none = env(&[("CLAUDECODE", "1"), ("CLAUDE_CODE_SESSION_ID", "!!!")]).unwrap();
    assert_eq!(none.locator(), "agent-session:claude-code:unknown");
}

fn session() -> AgentSession {
    env(&[("CLAUDECODE", "1"), ("CLAUDE_CODE_SESSION_ID", "s-1")]).unwrap()
}

#[test]
fn an_agent_session_cannot_verify_its_own_memory() {
    // The CLI rule: stamping refuses `verified`, whatever source the caller claims.
    let mut claim = Trace::new(TraceKind::Belief, "INR currency id is 1", "human:juber");
    claim.provenance.verified = true;
    match session().stamp(&mut claim) {
        Err(KernelError::Invalid(message)) => {
            assert!(message.contains("CLAUDECODE"), "{message}");
            assert!(message.contains("own terminal"), "{message}");
            // A person mistaken for an agent learns which variables did it and where to look.
            assert!(
                message.contains("Agent variables set: CLAUDECODE, CLAUDE_CODE_SESSION_ID")
                    && message.contains("docs/MEMORY.md"),
                "{message}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }

    // Unverified writes are stamped; a caller locator is kept after the session.
    let mut note = Trace::new(
        TraceKind::Observation,
        "WHMCS returned 3 plans",
        "human:juber",
    );
    session().stamp(&mut note).unwrap();
    assert_eq!(
        note.provenance.locator.as_deref(),
        Some("agent-session:claude-code:s-1")
    );
    let mut cited = Trace::new(TraceKind::Observation, "Pricing page copy", "agent");
    cited.provenance.locator = Some("https://example.com/pricing".into());
    session().stamp(&mut cited).unwrap();
    assert_eq!(
        cited.provenance.locator.as_deref(),
        Some("agent-session:claude-code:s-1; https://example.com/pricing")
    );

    // The kernel invariant holds for every write path, not only the CLI.
    let path = temp_store("agent-verified");
    let mut store = MemoryStore::open(&path, KernelPolicy::default()).unwrap();
    let mut forged = note.clone();
    forged.provenance.verified = true;
    assert!(matches!(
        store.remember(forged),
        Err(KernelError::Invalid(message)) if message.contains("agent session")
    ));
    assert_eq!(store.record_count(), 0, "nothing may be written");
    let (note, _) = store.remember(note).unwrap();
    assert!(note.is_from_agent_session());

    // A person attests from outside the session with a verified correction.
    let mut attested = Trace::new(
        TraceKind::Correction,
        "WHMCS returned 3 plans",
        "human:juber",
    );
    attested.provenance.verified = true;
    attested.supersedes = Some(note.id.clone());
    store.remember(attested).unwrap();
    assert_eq!(store.get(&note.id).unwrap().status, TraceStatus::Superseded);
}

fn hikmah(args: &[&str], agent: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hikmah"));
    without_agent_session(&mut command)
        .args(args)
        .env_remove("HIKMAH_POLICY");
    for (name, value) in agent {
        command.env(name, value);
    }
    command.output().unwrap()
}

#[test]
fn the_cli_stamps_agent_writes_and_refuses_their_verified_flag() {
    let path = temp_store("agent-cli");
    let store = path.to_str().unwrap();
    let agent = [("CLAUDECODE", "1"), ("CLAUDE_CODE_SESSION_ID", "cli-test")];
    let remember = |extra: &[&str], agent: &[(&str, &str)]| {
        let mut args = vec![
            "remember",
            "--store",
            store,
            "--kind",
            "belief",
            "--content",
            "Billing runs on WHMCS",
            "--source",
            "human:juber",
        ];
        args.extend_from_slice(extra);
        hikmah(&args, agent)
    };

    let refused = remember(&["--verified"], &agent);
    assert!(!refused.status.success());
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("--verified is refused") && stderr.contains("CLAUDECODE"),
        "{stderr}"
    );
    assert!(!path.exists(), "a refused write creates no store");

    let stamped = remember(&[], &agent);
    assert!(stamped.status.success());
    let stamped: Value = serde_json::from_slice(&stamped.stdout).unwrap();
    let provenance = &stamped["trace"]["provenance"];
    assert_eq!(provenance["locator"], "agent-session:claude-code:cli-test");
    assert_eq!(provenance["verified"], false);
    assert_eq!(provenance["source"], "human:juber", "the claim is kept");

    // Outside an agent session a person can still verify.
    let human = remember(&["--verified"], &[]);
    assert!(human.status.success());
    let human: Value = serde_json::from_slice(&human.stdout).unwrap();
    assert_eq!(human["trace"]["provenance"]["verified"], true);
    assert_eq!(human["trace"]["provenance"]["locator"], Value::Null);
}

#[test]
fn the_cli_stamps_outcomes_recorded_from_an_agent_session() {
    use hikmah_kernel::decision_port::{
        ask, DecisionRequest, EngineDescriptor, Question, RawAnswer, StaticEngine,
    };
    let path = temp_store("agent-outcome");
    let request = DecisionRequest::new(
        "Change set for review.",
        vec![Question::noul(
            "breaks",
            "Will this change break the build?",
        )],
    )
    .unwrap();
    let engine = StaticEngine {
        descriptor: EngineDescriptor {
            name: "fixture".into(),
            version: "1".into(),
        },
        answers: [("breaks".to_string(), RawAnswer::Noul { noul: 0.3 })].into(),
    };
    let prediction = ask(&engine, &request)
        .unwrap()
        .prediction_traces(&request)
        .remove(0);
    let prediction = MemoryStore::open(&path, KernelPolicy::default())
        .unwrap()
        .remember(prediction)
        .unwrap()
        .0
        .id;

    let output = hikmah(
        &[
            "outcome",
            "--store",
            path.to_str().unwrap(),
            "--prediction",
            &prediction,
            "--observed",
            "false",
            "--source",
            "ci",
        ],
        &[("CODEX_SANDBOX", "seatbelt"), ("CODEX_THREAD_ID", "run-7")],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let outcome: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        outcome["provenance"]["locator"],
        "agent-session:codex:run-7"
    );
}

#[test]
fn the_cli_stamps_forecasts_recorded_from_an_agent_session() {
    let path = temp_store("agent-predict");
    let output = hikmah(
        &[
            "predict",
            "--store",
            path.to_str().unwrap(),
            "--family",
            "site.hero.ctr",
            "--question",
            "Will the new hero raise click-through?",
            "--type",
            "noul",
            "--p",
            "0.7",
            "--source",
            "agent:planner",
            "--locator",
            "DECISIONS.md#hero",
        ],
        &[("CODEX_SANDBOX", "seatbelt"), ("CODEX_THREAD_ID", "run-8")],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let forecast: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        forecast["provenance"]["locator"],
        "agent-session:codex:run-8; DECISIONS.md#hero"
    );
    assert_eq!(forecast["provenance"]["verified"], false);
    assert_eq!(forecast["prediction"]["engine"], "agent:planner");
}

#[test]
fn accepting_unacknowledged_records_is_left_to_a_person() {
    let message = session()
        .refuse_person_only(
            "hikmah verify-ledger --accept-tail",
            "it approves records no hikmah write acknowledged",
        )
        .to_string();
    for expected in [
        "`hikmah verify-ledger --accept-tail` is refused inside an AI agent session",
        "claude-code detected through CLAUDECODE",
        "An agent must stop here and ask a person",
        "from their own terminal",
        "Agent variables set: CLAUDECODE, CLAUDE_CODE_SESSION_ID",
    ] {
        assert!(
            message.contains(expected),
            "{expected:?} missing: {message}"
        );
    }
    // The refusal names no way around the check (such as clearing the variables).
    assert!(
        !message.contains("unset") && !message.contains("env -u"),
        "{message}"
    );

    // A host that sets dozens of variables gets a bounded list.
    let names: Vec<String> = (0..40)
        .map(|i| format!("CLAUDE_CODE_RUNTIME_{i:02}"))
        .collect();
    let crowded = detect(names.iter().map(|name| (name.as_str(), "1"))).unwrap();
    assert_eq!(crowded.markers.len(), 40);
    let message = crowded
        .refuse_person_only("hikmah verify-ledger --reset-head", "why")
        .to_string();
    assert!(
        message.contains("CLAUDE_CODE_RUNTIME_04, and 35 more.")
            && !message.contains("CLAUDE_CODE_RUNTIME_05"),
        "{message}"
    );
}
