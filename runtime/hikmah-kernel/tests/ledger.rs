mod common;

use common::{copy_fixture, line_count, temp_store};
use hikmah_kernel::ledger::LedgerPayload;
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::recall::RecallQuery;
use hikmah_kernel::trace::{PrivacyClass, Trace, TraceKind, TraceStatus};
use hikmah_kernel::{KernelError, MemoryStore};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

fn open(path: &std::path::Path) -> MemoryStore {
    MemoryStore::open(path, KernelPolicy::default()).unwrap()
}

fn note(content: &str) -> Trace {
    Trace::new(TraceKind::Observation, content, "test")
}

#[test]
fn legacy_v1_ledger_still_verifies_and_accepts_v2_appends() {
    let path = copy_fixture("ledger_v1.jsonl");
    let mut store = open(&path);
    assert_eq!(store.record_count(), 4);
    assert!(store.records().iter().all(|r| r.version == 1));
    store.verify().unwrap();
    store
        .remember(note("First record written by 3.1.0"))
        .unwrap();
    drop(store);

    let reopened = open(&path);
    assert_eq!(reopened.record_count(), 5);
    assert_eq!(reopened.records().last().unwrap().version, 2);
    let report = reopened.verify_report(None).unwrap();
    assert!(report.ok, "{report:?}");
    assert!(report.head_file_checked);
}

#[test]
fn ledger_bricked_by_an_old_binary_opens_with_a_warning() {
    let path = copy_fixture("ledger_v1_bricked.jsonl");
    let mut store = open(&path);
    let report = store.verify_report(None).unwrap();
    assert!(report.ok);
    assert!(report
        .warnings
        .iter()
        .any(|w| w.contains("tr_doesnotexist")));
    store.remember(note("still writable")).unwrap();
    open(&path).verify().unwrap();
}

#[test]
fn invalid_supersede_is_rejected_before_anything_is_written() {
    let path = temp_store("supersede");
    let mut store = open(&path);
    store.remember(note("baseline")).unwrap();
    let before = line_count(&path);
    let mut bad = Trace::new(TraceKind::Correction, "fix a typo", "test");
    bad.supersedes = Some("tr_doesnotexist".into());
    assert!(matches!(store.remember(bad), Err(KernelError::NotFound(_))));
    assert_eq!(line_count(&path), before, "nothing may be written");
    let reopened = open(&path);
    reopened.verify().unwrap();
    assert_eq!(reopened.record_count(), 1);
}

#[test]
fn verified_traces_need_a_verified_correction_and_corrections_do_not_self_conflict() {
    let path = temp_store("authority");
    let mut store = open(&path);
    let mut rule = Trace::new(
        TraceKind::Constraint,
        "Service region is us-east-1",
        "runbook",
    );
    rule.claim_key = Some("service.region".into());
    rule.claim_value = Some("us-east-1".into());
    rule.provenance.verified = true;
    let (rule, _) = store.remember(rule).unwrap();

    let mut weak = Trace::new(TraceKind::Correction, "Region is eu-west-1", "agent");
    weak.supersedes = Some(rule.id.clone());
    assert!(store.remember(weak).is_err());

    let mut model = Trace::new(TraceKind::Correction, "Region is eu-west-1", "model:jev@1");
    model.supersedes = Some(rule.id.clone());
    assert!(store.remember(model).is_err());

    let mut fix = Trace::new(TraceKind::Correction, "Region is eu-west-1", "runbook");
    fix.claim_key = Some("service.region".into());
    fix.claim_value = Some("eu-west-1".into());
    fix.supersedes = Some(rule.id.clone());
    fix.provenance.verified = true;
    let (_, conflicts) = store.remember(fix).unwrap();
    assert!(conflicts.is_empty(), "{conflicts:?}");
    assert_eq!(store.get(&rule.id).unwrap().status, TraceStatus::Superseded);
}

#[test]
fn model_authored_traces_cannot_be_verified_or_supersede() {
    let path = temp_store("model");
    let mut store = open(&path);
    let mut claim = Trace::new(TraceKind::Belief, "The build is green", "model:jev@jev-1");
    claim.provenance.verified = true;
    assert!(store.remember(claim).is_err());

    let (note, _) = store.remember(note("unverified observation")).unwrap();
    let mut correction = Trace::new(TraceKind::Correction, "model rewrite", "model:jev@jev-1");
    correction.supersedes = Some(note.id.clone());
    assert!(store.remember(correction).is_err());
    assert_eq!(store.get(&note.id).unwrap().status, TraceStatus::Active);
}

#[test]
fn concurrent_writers_keep_one_valid_chain() {
    let path = temp_store("concurrent");
    open(&path);
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let path = path.clone();
            std::thread::spawn(move || {
                let mut store = MemoryStore::open(&path, KernelPolicy::default()).unwrap();
                for i in 0..5 {
                    store
                        .remember(Trace::new(
                            TraceKind::Observation,
                            format!("writer {t} event {i}"),
                            format!("writer-{t}"),
                        ))
                        .unwrap();
                }
            })
        })
        .collect();
    for handle in threads {
        handle.join().unwrap();
    }
    let store = open(&path);
    assert_eq!(store.record_count(), 40);
    let report = store.verify_report(None).unwrap();
    assert!(report.ok, "{report:?}");
}

#[test]
fn torn_tail_is_ignored_then_repaired() {
    let path = temp_store("torn");
    let mut store = open(&path);
    store.remember(note("complete record")).unwrap();
    drop(store);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(br#"{"seq":2,"prev_hash":"abc","v":2,"payl"#)
        .unwrap();
    drop(file);

    let mut store = open(&path);
    assert_eq!(store.record_count(), 1);
    assert!(store
        .verify_report(None)
        .unwrap()
        .warnings
        .iter()
        .any(|w| w.contains("torn")));
    store.remember(note("after the crash")).unwrap();
    let reopened = open(&path);
    assert_eq!(reopened.record_count(), 2);
    assert!(reopened.verify_report(None).unwrap().ok);
}

#[test]
fn record_without_final_newline_is_kept_and_newline_restored() {
    let path = copy_fixture("ledger_v1.jsonl");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.trim_end_matches('\n')).unwrap();
    let mut store = open(&path);
    assert_eq!(store.record_count(), 4);
    store
        .remember(note("appended after a missing newline"))
        .unwrap();
    let reopened = open(&path);
    assert_eq!(reopened.record_count(), 5);
    reopened.verify().unwrap();
}

#[test]
fn truncation_is_detected_through_the_head_file() {
    let path = temp_store("truncate");
    let mut store = open(&path);
    for i in 0..3 {
        store.remember(note(&format!("event {i}"))).unwrap();
    }
    drop(store);
    let text = fs::read_to_string(&path).unwrap();
    let kept: Vec<&str> = text.lines().take(2).collect();
    fs::write(&path, format!("{}\n", kept.join("\n"))).unwrap();
    let report = open(&path).verify_report(None).unwrap();
    assert!(!report.ok);
    assert!(report.errors.iter().any(|w| w.contains("removed")));
}

#[test]
fn pinned_head_detects_a_rewritten_ledger() {
    let path = temp_store("pinned");
    let mut store = open(&path);
    store.remember(note("event")).unwrap();
    let head = store.head().unwrap().hash;
    assert!(store.verify_report(Some(&head)).unwrap().ok);
    assert!(!store.verify_report(Some("not-the-head")).unwrap().ok);
}

#[test]
fn edited_payload_bytes_and_injected_fields_are_rejected() {
    let path = temp_store("tamper");
    let mut store = open(&path);
    store.remember(note("Never deploy on Fridays")).unwrap();
    drop(store);
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("Never deploy", "Always deploy")).unwrap();
    assert!(matches!(
        MemoryStore::open(&path, KernelPolicy::default()),
        Err(KernelError::Integrity { .. })
    ));
    fs::write(
        &path,
        text.replacen("{\"seq\"", "{\"injected\":1,\"seq\"", 1),
    )
    .unwrap();
    assert!(MemoryStore::open(&path, KernelPolicy::default()).is_err());
}

#[test]
fn read_only_open_does_not_create_missing_stores() {
    let path = temp_store("missing").with_file_name("typo.jsonl");
    assert!(MemoryStore::open_existing(&path, KernelPolicy::default()).is_err());
    assert!(!path.exists());
}

#[test]
fn status_transitions_are_constrained() {
    let path = temp_store("status");
    let mut store = open(&path);
    let (belief, _) = store.remember(note("a belief")).unwrap();
    assert!(store.fulfill(&belief.id).is_err(), "only commitments");
    store.purge(&belief.id, "wrong").unwrap();
    assert!(store.purge(&belief.id, "again").is_err());
    let mut commitment = Trace::new(TraceKind::Commitment, "send the report", "test");
    commitment.deadline_ms = Some(1);
    let (commitment, _) = store.remember(commitment).unwrap();
    store.fulfill(&commitment.id).unwrap();
    assert!(store.fulfill(&commitment.id).is_err());
    open(&path).verify().unwrap();
}

#[test]
fn sensitive_traces_stay_out_of_recall_under_the_default_policy() {
    let path = temp_store("sensitive");
    let permissive = KernelPolicy {
        allow_sensitive_persistence: true,
        ..KernelPolicy::default()
    };
    let mut store = MemoryStore::open(&path, permissive).unwrap();
    let mut secret = Trace::new(TraceKind::Observation, "patient allergy record", "clinic");
    secret.privacy = PrivacyClass::Sensitive;
    store.remember(secret).unwrap();
    drop(store);
    let strict = open(&path);
    assert!(strict
        .recall(&RecallQuery::new("patient allergy"))
        .is_empty());
}

#[test]
fn concurrent_creation_of_a_new_store_loses_nothing() {
    for round in 0..20 {
        let path = temp_store(&format!("create-{round}"));
        let threads: Vec<_> = (0..6)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let mut store = MemoryStore::open(&path, KernelPolicy::default()).unwrap();
                    store
                        .remember(Trace::new(
                            TraceKind::Observation,
                            format!("creator {t}"),
                            format!("creator-{t}"),
                        ))
                        .unwrap();
                })
            })
            .collect();
        for handle in threads {
            handle.join().unwrap();
        }
        let store = open(&path);
        assert_eq!(store.record_count(), 6, "round {round}");
        assert!(store.verify_report(None).unwrap().ok);
    }
}

#[test]
fn a_reader_opened_before_a_write_does_not_raise_a_false_alarm() {
    let path = temp_store("stale-reader");
    let mut writer = open(&path);
    writer.remember(note("first")).unwrap();
    let reader = open(&path);
    writer.remember(note("second")).unwrap();
    let report = reader.verify_report(None).unwrap();
    assert!(report.ok, "{report:?}");
}

#[test]
fn writes_are_refused_after_truncation_until_the_head_is_reset() {
    let path = temp_store("refuse");
    let mut store = open(&path);
    for i in 0..3 {
        store.remember(note(&format!("event {i}"))).unwrap();
    }
    drop(store);
    let text = fs::read_to_string(&path).unwrap();
    let first: Vec<&str> = text.lines().take(1).collect();
    fs::write(&path, format!("{}\n", first[0])).unwrap();

    let mut store = open(&path);
    assert!(matches!(
        store.remember(note("papering over")),
        Err(KernelError::Integrity { .. })
    ));
    assert!(!open(&path).verify_report(None).unwrap().ok);

    store.reset_head().unwrap();
    store.remember(note("after an explicit reset")).unwrap();
    assert!(open(&path).verify_report(None).unwrap().ok);
}

#[test]
fn torn_tail_inside_a_multibyte_character_is_repaired() {
    let path = temp_store("utf8-torn");
    let mut store = open(&path);
    store.remember(note("بسم الله الرحمن الرحيم")).unwrap();
    drop(store);
    let text = fs::read(&path).unwrap();
    let line = text.clone();
    let cut = line
        .iter()
        .position(|&b| b >= 0xd8)
        .expect("arabic bytes present")
        + 1;
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(&line[..cut]).unwrap();
    drop(file);

    let mut store = open(&path);
    assert_eq!(store.record_count(), 1);
    store.remember(note("after the torn write")).unwrap();
    assert!(open(&path).verify_report(None).unwrap().ok);
}

#[test]
fn non_json_whitespace_tail_is_treated_as_torn() {
    let path = temp_store("nbsp");
    let mut store = open(&path);
    store.remember(note("first")).unwrap();
    drop(store);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all("\u{a0}".as_bytes()).unwrap();
    drop(file);
    let mut store = open(&path);
    store.remember(note("second")).unwrap();
    let reopened = open(&path);
    assert_eq!(reopened.record_count(), 2);
    assert!(reopened.verify_report(None).unwrap().ok);
}

#[test]
fn an_unreadable_head_file_is_reported_not_fatal() {
    let path = temp_store("bad-head");
    let mut store = open(&path);
    store.remember(note("first")).unwrap();
    let head_path = store.head_path();
    drop(store);
    fs::write(&head_path, b"").unwrap();
    let report = open(&path).verify_report(None).unwrap();
    assert!(!report.ok);
    assert!(report.errors.iter().any(|w| w.contains("unreadable")));
}

#[test]
fn credentials_are_refused_in_every_field_before_anything_is_written() {
    let path = temp_store("secrets");
    let mut store = open(&path);
    store.remember(note("baseline")).unwrap();
    let before = line_count(&path);
    let token = "ghp_a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8";
    let mut cases: Vec<(&str, Trace)> = Vec::new();
    cases.push(("content", note("DB_PASSWORD=hunter2hunter2")));
    let mut tagged = note("deploy notes");
    tagged.tags = vec!["ops".into(), token.into()];
    cases.push(("tag", tagged));
    let mut claim_key = note("api settings");
    claim_key.claim_key = Some(format!("token {token}"));
    claim_key.claim_value = Some("set".into());
    cases.push(("claim_key", claim_key));
    let mut claim_value = note("database url");
    claim_value.claim_key = Some("db.url".into());
    claim_value.claim_value = Some("postgres://app:Pr0dPassw0rd@db.internal/app".into());
    cases.push(("claim_value", claim_value));
    let mut locator = note("pulled from the shared doc");
    locator.provenance.locator = Some(format!("https://docs.example.com/?access_token={token}"));
    cases.push(("locator", locator));
    let mut source = note("imported");
    source.provenance.source = "postgres://svc:SuperSecret1@db.internal/app".into();
    cases.push(("source", source));

    for (field, trace) in cases {
        match store.remember(trace) {
            Err(KernelError::Invalid(message)) => {
                assert!(message.contains(field), "{field}: {message}");
                assert!(message.contains("secret"), "{message}");
                assert!(!message.contains(token) && !message.contains("hunter2"));
            }
            other => panic!("{field}: expected Invalid, got {other:?}"),
        }
    }
    assert_eq!(line_count(&path), before, "nothing may be written");

    // Talking about passwords is not a password.
    store
        .remember(note("the password field should be hashed with argon2"))
        .unwrap();
    open(&path).verify().unwrap();
}

#[test]
fn a_purge_reason_cannot_re_leak_the_credential() {
    let path = temp_store("purge-secret");
    let mut store = open(&path);
    let id = store.remember(note("deploy notes")).unwrap().0.id;
    let before = line_count(&path);
    let token = "ghp_a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8";
    match store.purge(&id, format!("leaked {token}")) {
        Err(KernelError::Invalid(message)) => assert!(!message.contains(token), "{message}"),
        other => panic!("expected Invalid, got {other:?}"),
    }
    assert_eq!(line_count(&path), before, "nothing may be written");
    store.purge(&id, "contained a leaked GitHub token").unwrap();
}

#[test]
fn a_file_that_is_not_a_ledger_is_never_truncated() {
    let path = temp_store("not-a-ledger");
    let text = "important notes without trailing newline";
    fs::write(&path, text).unwrap();
    let mut store = open(&path);
    assert!(matches!(
        store.remember(note("oops")),
        Err(KernelError::Integrity { .. })
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), text);

    // A crash during the very first write still leaves a repairable store.
    let torn = temp_store("torn-first-record");
    fs::write(&torn, br#"{"se"#).unwrap();
    open(&torn).remember(note("after the crash")).unwrap();
    assert_eq!(open(&torn).record_count(), 1);
}

/// Append a chain-valid v2 record the way someone with write access to the file (but not using
/// hikmah) could: the chain is unkeyed, so the hash is computable by anyone.
fn forge_append(path: &Path, trace: Trace) {
    let text = fs::read_to_string(path).unwrap();
    let last: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    let seq = last["seq"].as_u64().unwrap() + 1;
    let prev = last["hash"].as_str().unwrap().to_string();
    let payload = serde_json::to_string(&LedgerPayload::Remember {
        trace: Box::new(trace),
    })
    .unwrap();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"hikmah-ledger-v2\n");
    hasher.update(seq.to_string().as_bytes());
    hasher.update(b"\n");
    hasher.update(prev.as_bytes());
    hasher.update(b"\n");
    hasher.update(payload.as_bytes());
    let hash = hasher.finalize().to_hex().to_string();
    let line = format!(
        "{{\"seq\":{seq},\"prev_hash\":\"{prev}\",\"v\":2,\"payload\":{payload},\"hash\":\"{hash}\"}}\n"
    );
    fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(line.as_bytes())
        .unwrap();
}

/// The audit's forged record: a verified claim from a source that never wrote it.
fn forged_claim() -> Trace {
    let mut trace = Trace::new(
        TraceKind::Observation,
        "WHMCS currency id for INR is 9",
        "whmcs-snapshot",
    );
    trace.id = "tr_forged000000001".into();
    trace.claim_key = Some("whmcs.currency.inr".into());
    trace.claim_value = Some("9".into());
    trace.provenance.verified = true;
    trace
}

#[test]
fn forged_appends_are_refused_until_explicitly_accepted() {
    let path = temp_store("forged-append");
    let mut store = open(&path);
    store.remember(note("first")).unwrap();
    store.remember(note("second")).unwrap();
    drop(store);
    forge_append(&path, forged_claim());

    // The chain itself is valid, so the store opens; verification fails with an error, not a
    // warning, and lists the record for inspection.
    let mut store = open(&path);
    assert_eq!(store.record_count(), 3);
    let report = store.verify_report(None).unwrap();
    assert!(!report.ok, "{report:?}");
    assert!(report.errors.iter().any(|e| e.contains("--accept-tail")));
    assert_eq!(report.unacknowledged.len(), 1);
    let forged = &report.unacknowledged[0];
    assert_eq!(
        (forged.seq, forged.event.as_str(), forged.trace_id.as_str()),
        (3, "remember", "tr_forged000000001")
    );
    assert_eq!(forged.verified, Some(true));
    assert_eq!(forged.claim.as_deref(), Some("whmcs.currency.inr=9"));
    assert!(store.verify().is_err());

    // The next ordinary write must not approve it: refused, nothing written, record listed.
    let before = line_count(&path);
    match store.remember(note("ordinary write")) {
        Err(KernelError::Integrity { seq, message }) => {
            assert_eq!(seq, 3);
            assert!(message.contains("tr_forged000000001"), "{message}");
            assert!(message.contains("whmcs.currency.inr=9"), "{message}");
            assert!(message.contains("--accept-tail"), "{message}");
        }
        other => panic!("expected an integrity refusal, got {other:?}"),
    }
    assert_eq!(line_count(&path), before, "nothing may be written");

    // Accepting is explicit and reports exactly what was accepted.
    let acceptance = store.accept_tail().unwrap();
    assert_eq!(acceptance.accepted, report.unacknowledged);
    assert_eq!(acceptance.head.unwrap().seq, 3);
    store.remember(note("after acceptance")).unwrap();
    let reopened = open(&path);
    let report = reopened.verify_report(None).unwrap();
    assert!(report.ok && report.unacknowledged.is_empty(), "{report:?}");
    // Nothing left to accept.
    assert!(open(&path).accept_tail().unwrap().accepted.is_empty());
}

#[test]
fn accepting_the_tail_never_accepts_a_rewrite_or_truncation() {
    let path = temp_store("accept-tail-truncated");
    let mut store = open(&path);
    for i in 0..3 {
        store.remember(note(&format!("event {i}"))).unwrap();
    }
    drop(store);
    let text = fs::read_to_string(&path).unwrap();
    let kept: Vec<&str> = text.lines().take(2).collect();
    fs::write(&path, format!("{}\n", kept.join("\n"))).unwrap();
    assert!(matches!(
        open(&path).accept_tail(),
        Err(KernelError::Integrity { .. })
    ));
    assert!(!open(&path).verify_report(None).unwrap().ok);

    // Without a head file there is nothing to extend; the deliberate path is --reset-head.
    let legacy = copy_fixture("ledger_v1.jsonl");
    assert!(matches!(
        open(&legacy).accept_tail(),
        Err(KernelError::Invalid(_))
    ));
    let report = open(&legacy).verify_report(None).unwrap();
    assert!(report.ok);
    assert!(report.warnings.iter().any(|w| w.contains("no head file")));
}

#[test]
fn a_reader_that_saw_a_write_in_flight_does_not_raise_a_false_alarm() {
    // A writer syncs its records before it updates the head. A lock-free reader that reads the
    // head before a write and the ledger after it sees records past its head snapshot; they are
    // not forgeries if the head acknowledges them by the time the reader verifies.
    let path = temp_store("in-flight");
    let mut writer = open(&path);
    writer.remember(note("first")).unwrap();
    let head_path = writer.head_path();
    let head_before = fs::read(&head_path).unwrap();
    writer.remember(note("second")).unwrap();
    let head_after = fs::read(&head_path).unwrap();

    // Freeze the moment between the record sync and the head update.
    fs::write(&head_path, &head_before).unwrap();
    let reader = open(&path);
    assert_eq!(reader.record_count(), 2);
    let in_flight = reader.verify_report(None).unwrap();
    assert!(
        !in_flight.ok,
        "still unacknowledged (a crash here needs --accept-tail)"
    );

    // The write completes: the reader's snapshot is now acknowledged.
    fs::write(&head_path, &head_after).unwrap();
    let report = reader.verify_report(None).unwrap();
    assert!(report.ok, "{report:?}");

    // The head has since moved past the reader's snapshot: still no false alarm.
    fs::write(&head_path, &head_before).unwrap();
    let stale = open(&path);
    fs::write(&head_path, &head_after).unwrap();
    writer.remember(note("third")).unwrap();
    let report = stale.verify_report(None).unwrap();
    assert!(report.ok, "{report:?}");
}

#[test]
fn unacknowledged_record_listings_never_echo_a_credential() {
    let path = temp_store("forged-secret");
    let mut store = open(&path);
    store.remember(note("first")).unwrap();
    drop(store);
    let token = "ghp_a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8";
    let mut leaked = note("placeholder");
    leaked.id = "tr_forged000000002".into();
    leaked.content = format!("deploy with {token}");
    forge_append(&path, leaked);

    let mut store = open(&path);
    let report = store.verify_report(None).unwrap();
    let listed = serde_json::to_string(&report).unwrap();
    assert!(listed.contains("tr_forged000000002") && !listed.contains(token));
    match store.remember(note("ordinary write")) {
        Err(KernelError::Integrity { message, .. }) => {
            assert!(
                message.contains("withheld") && !message.contains(token),
                "{message}"
            )
        }
        other => panic!("expected an integrity refusal, got {other:?}"),
    }
}

fn hikmah(args: &[&str]) -> std::process::Output {
    common::without_agent_session(&mut Command::new(env!("CARGO_BIN_EXE_hikmah")))
        .args(args)
        .env_remove("HIKMAH_POLICY")
        .output()
        .unwrap()
}

#[test]
fn the_cli_refuses_writes_after_a_forged_append_until_accept_tail() {
    let path = temp_store("forged-cli");
    let store = path.to_str().unwrap();
    let remember = |content: &str| {
        hikmah(&[
            "remember",
            "--store",
            store,
            "--kind",
            "belief",
            "--content",
            content,
        ])
    };
    assert!(remember("first").status.success());
    forge_append(&path, forged_claim());

    let refused = remember("second");
    assert!(!refused.status.success());
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("tr_forged000000001") && stderr.contains("--accept-tail"));

    let verify = hikmah(&["verify-ledger", "--store", store]);
    assert_eq!(verify.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&verify.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert_eq!(
        report["unacknowledged"][0]["trace_id"],
        "tr_forged000000001"
    );

    let accepted = hikmah(&["verify-ledger", "--store", store, "--accept-tail"]);
    assert!(accepted.status.success());
    let accepted: Value = serde_json::from_slice(&accepted.stdout).unwrap();
    assert_eq!(accepted["accepted"][0]["seq"], 2);

    assert!(remember("second").status.success());
    assert!(hikmah(&["verify-ledger", "--store", store])
        .status
        .success());
}

#[test]
fn verifying_during_concurrent_writes_never_raises_a_false_alarm() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let path = temp_store("verify-while-writing");
    open(&path).remember(note("seed")).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..2)
        .map(|_| {
            let (path, done) = (path.clone(), done.clone());
            std::thread::spawn(move || {
                let mut checks = 0;
                while !done.load(Ordering::SeqCst) || checks == 0 {
                    let report = open(&path).verify_report(None).unwrap();
                    assert!(report.ok, "false alarm: {report:?}");
                    checks += 1;
                }
                checks
            })
        })
        .collect();
    let writers: Vec<_> = (0..4)
        .map(|t| {
            let path = path.clone();
            std::thread::spawn(move || {
                let mut store = open(&path);
                for i in 0..10 {
                    store
                        .remember(note(&format!("writer {t} event {i}")))
                        .unwrap();
                }
            })
        })
        .collect();
    for handle in writers {
        handle.join().unwrap();
    }
    done.store(true, Ordering::SeqCst);
    for handle in readers {
        assert!(handle.join().unwrap() > 0);
    }
    assert_eq!(open(&path).record_count(), 41);
}

/// Run the CLI as an agent host would: its session variables set.
fn hikmah_in_agent_session(args: &[&str]) -> std::process::Output {
    common::without_agent_session(&mut Command::new(env!("CARGO_BIN_EXE_hikmah")))
        .args(args)
        .env_remove("HIKMAH_POLICY")
        .env("CLAUDECODE", "1")
        .env("CLAUDE_CODE_SESSION_ID", "accept-test")
        .output()
        .unwrap()
}

#[test]
fn an_agent_session_cannot_accept_the_records_it_is_refused_over() {
    // Reviewer repro: the write refusal pointed to `--accept-tail`, and running it from the same
    // agent session approved a forged verified claim. Accepting is now a person's decision.
    let path = temp_store("forged-agent-accept");
    let store = path.to_str().unwrap();
    assert!(hikmah(&[
        "remember",
        "--store",
        store,
        "--kind",
        "belief",
        "--content",
        "first"
    ])
    .status
    .success());
    forge_append(&path, forged_claim());
    let head_path = open(&path).head_path();
    let head = fs::read(&head_path).unwrap();

    for flag in ["--accept-tail", "--reset-head"] {
        let refused = hikmah_in_agent_session(&["verify-ledger", "--store", store, flag]);
        assert!(!refused.status.success(), "{flag}");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert!(
            stderr.contains(&format!(
                "verify-ledger {flag}` is refused inside an AI agent session"
            )) && stderr.contains("CLAUDECODE")
                && stderr.contains("ask a person"),
            "{stderr}"
        );
        assert_eq!(
            fs::read(&head_path).unwrap(),
            head,
            "{flag} changed the head"
        );
    }
    let report = open(&path).verify_report(None).unwrap();
    assert!(!report.ok && report.unacknowledged.len() == 1, "{report:?}");
    // The agent's own writes stay refused until a person accepts.
    let write = hikmah_in_agent_session(&[
        "remember",
        "--store",
        store,
        "--kind",
        "belief",
        "--content",
        "second",
    ]);
    assert!(!write.status.success());

    // A person, outside the agent session, can accept after inspecting.
    let accepted = hikmah(&["verify-ledger", "--store", store, "--accept-tail"]);
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    assert!(hikmah(&["verify-ledger", "--store", store])
        .status
        .success());
}
