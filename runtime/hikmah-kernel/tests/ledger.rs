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
