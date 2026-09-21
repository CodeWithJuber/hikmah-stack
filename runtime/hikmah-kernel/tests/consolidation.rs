mod common;

use common::temp_store;
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::trace::{Trace, TraceKind};
use hikmah_kernel::MemoryStore;

fn claim(source: &str, value: &str, confidence: f32) -> Trace {
    let mut trace = Trace::new(TraceKind::Belief, format!("path is {value}"), source);
    trace.claim_key = Some("deploy.path".into());
    trace.claim_value = Some(value.into());
    trace.confidence = confidence;
    trace
}

#[test]
fn source_names_are_normalized_before_counting_independence() {
    let mut s = MemoryStore::open(temp_store("sources"), KernelPolicy::default()).unwrap();
    s.remember(claim("config-a", "/srv/app", 0.9)).unwrap();
    s.remember(claim("Config-A", "/srv/app", 0.9)).unwrap();
    let proposals = s.consolidation_proposals();
    assert_eq!(proposals[0].independent_sources, vec!["config-a"]);
    assert!(!proposals[0].eligible_for_promotion);
}

#[test]
fn low_confidence_support_is_not_eligible() {
    let mut s = MemoryStore::open(temp_store("confidence"), KernelPolicy::default()).unwrap();
    s.remember(claim("ci", "/srv/app", 0.3)).unwrap();
    s.remember(claim("runbook", "/srv/app", 0.3)).unwrap();
    assert!(!s.consolidation_proposals()[0].eligible_for_promotion);
}

#[test]
fn values_keep_case_but_canonical_unicode_agrees() {
    let mut s = MemoryStore::open(temp_store("values"), KernelPolicy::default()).unwrap();
    s.remember(claim("ci", "/srv/App", 0.9)).unwrap();
    let (_, conflicts) = s.remember(claim("runbook", "/srv/app", 0.9)).unwrap();
    assert_eq!(conflicts.len(), 1, "different paths must conflict");

    let mut s = MemoryStore::open(temp_store("nfc"), KernelPolicy::default()).unwrap();
    s.remember(claim("ci", "Montr\u{e9}al", 0.9)).unwrap();
    let (_, conflicts) = s
        .remember(claim("runbook", "Montre\u{301}al", 0.9))
        .unwrap();
    assert!(
        conflicts.is_empty(),
        "NFC and NFD spellings are the same text"
    );
    let proposals = s.consolidation_proposals();
    assert_eq!(proposals.len(), 1);
    assert!(proposals[0].eligible_for_promotion);
}
