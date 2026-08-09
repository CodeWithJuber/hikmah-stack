use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::recall::RecallQuery;
use hikmah_kernel::trace::{Trace, TraceKind};
use hikmah_kernel::MemoryStore;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_store(name: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("hikmah-{name}-{nonce}.jsonl"))
}

#[test]
fn ledger_reopens_and_recall_finds_relevant_trace() {
    let path = temp_store("recall");
    let mut store = MemoryStore::open(&path, KernelPolicy::default()).unwrap();
    let mut trace = Trace::new(
        TraceKind::Observation,
        "The deployment failed because the migration lock timed out",
        "test",
    );
    trace.tags = vec!["deployment".into(), "database".into()];
    trace.salience = 0.9;
    trace.confidence = 0.9;
    trace.provenance.verified = true;
    store.remember(trace).unwrap();
    drop(store);

    let reopened = MemoryStore::open(&path, KernelPolicy::default()).unwrap();
    reopened.verify().unwrap();
    let results = reopened.recall(&RecallQuery::new("why did deployment migration fail"));
    assert_eq!(results.len(), 1);
    assert!(results[0].trace.content.contains("migration lock"));
    let _ = fs::remove_file(path);
}

#[test]
fn contradictory_structured_claims_are_not_silently_overwritten() {
    let path = temp_store("conflict");
    let mut store = MemoryStore::open(&path, KernelPolicy::default()).unwrap();
    let mut first = Trace::new(TraceKind::Belief, "Service region is us-east-1", "config-a");
    first.claim_key = Some("service.region".into());
    first.claim_value = Some("us-east-1".into());
    first.provenance.verified = true;
    store.remember(first).unwrap();

    let mut second = Trace::new(TraceKind::Belief, "Service region is eu-west-1", "config-b");
    second.claim_key = Some("service.region".into());
    second.claim_value = Some("eu-west-1".into());
    let (_, conflicts) = store.remember(second).unwrap();
    assert_eq!(conflicts.len(), 1);
    let _ = fs::remove_file(path);
}
