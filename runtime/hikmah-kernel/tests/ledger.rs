mod common;

use common::{copy_fixture, line_count, temp_store};
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::recall::RecallQuery;
use hikmah_kernel::trace::{PrivacyClass, Trace, TraceKind, TraceStatus};
use hikmah_kernel::{KernelError, MemoryStore};
use std::fs;
use std::io::Write;

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
    assert!(report.warnings.iter().any(|w| w.contains("removed")));
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
    assert!(report.warnings.iter().any(|w| w.contains("unreadable")));
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
fn notes_that_describe_a_secret_are_stored_but_values_are_not() {
    // Configuration notes a HostLelo session tried to remember (review, 2026-09). Each says where
    // or how a secret is kept, which is what the refusal message asks people to record.
    let path = temp_store("secret-descriptions");
    let mut store = open(&path);
    for content in [
        "TYPESAFE_API_KEY=server-only, never shipped to the client",
        "Kubernetes secret: hostlelo-whmcs-creds is mounted into the pod",
        "password=hashed_with_argon2id before storage",
        "The WHMCS api_key: configured-in-env",
        "client_secret=rotated-2026-09 in the vault",
    ] {
        store
            .remember(note(content))
            .unwrap_or_else(|e| panic!("{content}: {e}"));
    }
    let before = line_count(&path);
    for content in [
        "client_secret=x9K2pQ7vR4mT8wZ1 in the vault",
        "TYPESAFE_API_KEY=server-only, DB_PASSWORD=hunter2hunter2",
        "wifi password=summer-2024",
    ] {
        assert!(
            matches!(store.remember(note(content)), Err(KernelError::Invalid(_))),
            "a value must still be refused: {content}"
        );
    }
    assert_eq!(line_count(&path), before, "nothing may be written");
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
