mod common;

use common::temp_store;
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::recall::{RecallQuery, RecallResult};
use hikmah_kernel::trace::{PrivacyClass, Trace, TraceKind};
use hikmah_kernel::MemoryStore;

fn store(name: &str) -> MemoryStore {
    MemoryStore::open(temp_store(name), KernelPolicy::default()).unwrap()
}

fn region(store: &mut MemoryStore, key: &str, value: &str, source: &str) -> String {
    claim(
        store,
        key,
        value,
        source,
        &format!("The deploy region is {value}"),
    )
}

fn claim(store: &mut MemoryStore, key: &str, value: &str, source: &str, content: &str) -> String {
    let mut trace = Trace::new(TraceKind::Belief, content, source);
    trace.claim_key = Some(key.into());
    trace.claim_value = Some(value.into());
    store.remember(trace).unwrap().0.id
}

fn result<'a>(results: &'a [RecallResult], id: &str) -> &'a RecallResult {
    results
        .iter()
        .find(|r| r.trace.id == id)
        .unwrap_or_else(|| panic!("{id} not recalled: {results:#?}"))
}

#[test]
fn recall_shows_conflicting_claims_beside_each_other() {
    let mut s = store("recall-conflicts");
    let east = region(&mut s, "deploy.region", "us-east-1", "runbook");
    let west = region(&mut s, "Deploy.Region", "eu-west-1", "oncall");
    // Same key and value after normalization: agreement, not a conflict.
    let east_again = claim(
        &mut s,
        "deploy.region",
        "us-east-1 ",
        "ci",
        "CI pipeline config pins the deploy region",
    );
    let mut note = Trace::new(TraceKind::Observation, "deploy region dashboards", "test");
    note.tags = vec!["ops".into()];
    let note = s.remember(note).unwrap().0.id;

    let results = s.recall(&RecallQuery::new("deploy region"));
    assert_eq!(result(&results, &east).conflicts, vec![west.clone()]);
    assert_eq!(result(&results, &east_again).conflicts, vec![west.clone()]);
    let mut expected = vec![east.clone(), east_again.clone()];
    expected.sort();
    assert_eq!(result(&results, &west).conflicts, expected);
    assert!(result(&results, &note).conflicts.is_empty());

    let conflicts = s.active_conflicts();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].claim_key, "deploy.region");
    assert_eq!(conflicts[0].values.len(), 2);
    let east_group = conflicts[0]
        .values
        .iter()
        .find(|g| g.value == "us-east-1")
        .unwrap();
    assert_eq!(east_group.trace_ids.len(), 2);
}

#[test]
fn a_correction_shows_what_it_replaced_and_resolves_the_conflict() {
    let mut s = store("recall-supersession");
    let old = region(&mut s, "deploy.region", "us-east-1", "runbook");
    let rival = region(&mut s, "deploy.region", "eu-west-1", "oncall");
    assert_eq!(s.active_conflicts().len(), 1);

    let mut fix = Trace::new(
        TraceKind::Correction,
        "Correction after the March outage: deploy region changed to eu-west-1",
        "runbook",
    );
    fix.claim_key = Some("deploy.region".into());
    fix.claim_value = Some("eu-west-1".into());
    fix.supersedes = Some(old.clone());
    let fix = s.remember(fix).unwrap().0.id;

    // The supersession resolved the only disagreement.
    assert!(s.active_conflicts().is_empty());
    let results = s.recall(&RecallQuery::new("deploy region"));
    let correction = result(&results, &fix);
    assert_eq!(correction.supersedes.as_deref(), Some(old.as_str()));
    assert!(correction.superseded_by.is_none());
    assert!(correction.conflicts.is_empty());
    assert!(result(&results, &rival).conflicts.is_empty());

    // History on request: the replaced trace comes back pointing at its replacement.
    let mut history = RecallQuery::new("deploy region");
    history.include_superseded = true;
    let results = s.recall(&history);
    let replaced = result(&results, &old);
    assert_eq!(replaced.superseded_by.as_deref(), Some(fix.as_str()));
    assert!(
        replaced.conflicts.is_empty(),
        "a superseded claim is not an open conflict"
    );
}

#[test]
fn sensitive_traces_are_not_listed_as_conflicts_under_the_default_policy() {
    let path = temp_store("sensitive-conflicts");
    let permissive = KernelPolicy {
        allow_sensitive_persistence: true,
        ..KernelPolicy::default()
    };
    let mut s = MemoryStore::open(&path, permissive).unwrap();
    let public = region(&mut s, "deploy.region", "us-east-1", "runbook");
    let mut private = Trace::new(TraceKind::Belief, "deploy region is secret-site", "ops");
    private.claim_key = Some("deploy.region".into());
    private.claim_value = Some("secret-site".into());
    private.privacy = PrivacyClass::Sensitive;
    s.remember(private).unwrap();
    drop(s);

    let strict = MemoryStore::open(&path, KernelPolicy::default()).unwrap();
    assert!(strict.active_conflicts().is_empty());
    let results = strict.recall(&RecallQuery::new("deploy region"));
    assert!(result(&results, &public).conflicts.is_empty());
}

#[test]
fn near_identical_wording_does_not_fold_a_competing_claim_away() {
    let mut s = store("fold-conflicts");
    let text = |v: &str| format!("The production deploy region for the billing service is {v}");
    let east = claim(
        &mut s,
        "billing.region",
        "us-east-1",
        "runbook",
        &text("us-east-1"),
    );
    let west = claim(
        &mut s,
        "billing.region",
        "us-east-2",
        "oncall",
        &text("us-east-2"),
    );
    // An agreeing near-duplicate is still folded.
    claim(
        &mut s,
        "billing.region",
        "us-east-1",
        "ci",
        &text("us-east-1"),
    );

    let results = s.recall(&RecallQuery::new("billing deploy region"));
    assert_eq!(results.len(), 2, "{results:#?}");
    let east_result = results
        .iter()
        .find(|r| r.trace.claim_value.as_deref() == Some("us-east-1"))
        .unwrap();
    assert_eq!(east_result.duplicates, 1, "the agreeing twin folds");
    assert!(east_result.conflicts.contains(&west));
    let west_result = result(&results, &west);
    assert_eq!(west_result.conflicts.len(), 2);
    assert!(west_result.conflicts.contains(&east));
}
