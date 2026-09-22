mod common;

use common::temp_store;
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::recall::RecallQuery;
use hikmah_kernel::trace::{PredictionRecord, Trace, TraceKind};
use hikmah_kernel::MemoryStore;
use std::collections::BTreeMap;

fn store(name: &str) -> MemoryStore {
    MemoryStore::open(temp_store(name), KernelPolicy::default()).unwrap()
}

fn add(store: &mut MemoryStore, content: &str) -> String {
    store
        .remember(Trace::new(TraceKind::Observation, content, "test"))
        .unwrap()
        .0
        .id
}

#[test]
fn unrelated_queries_return_nothing() {
    let mut s = store("irrelevant");
    add(&mut s, "Office city is Dubai");
    add(
        &mut s,
        "The deployment failed because the migration lock timed out",
    );
    assert!(s
        .recall(&RecallQuery::new("xylophone orchestra tuning"))
        .is_empty());
}

#[test]
fn self_reported_metadata_cannot_surface_irrelevant_memories() {
    let mut s = store("metadata");
    let mut loud = Trace::new(TraceKind::Belief, "Use Postgres for billing", "agent");
    loud.salience = 1.0;
    loud.confidence = 1.0;
    loud.provenance.verified = true;
    s.remember(loud).unwrap();
    let quiet = add(&mut s, "Migrations need approval from the DBA team");
    let results = s.recall(&RecallQuery::new("migration approval"));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].trace.id, quiet);
}

#[test]
fn duplicates_are_folded_instead_of_filling_the_working_set() {
    let mut s = store("dupes");
    for _ in 0..10 {
        add(&mut s, "Use Postgres for the billing service");
    }
    let contract = add(
        &mut s,
        "Billing service must use the managed MySQL cluster per contract",
    );
    let results = s.recall(&RecallQuery::new("billing service database"));
    assert_eq!(results.len(), 2, "{results:#?}");
    assert!(results.iter().any(|r| r.trace.id == contract));
    assert!(results.iter().any(|r| r.duplicates == 9));
}

#[test]
fn query_coverage_beats_short_partial_matches() {
    let mut s = store("coverage");
    add(&mut s, "Deploy failed");
    let postmortem = add(
        &mut s,
        "Postmortem: the deploy stalled because the database lock hit its timeout after the schema change ran long",
    );
    let results = s.recall(&RecallQuery::new("deploy lock timeout"));
    assert_eq!(results[0].trace.id, postmortem);
}

#[test]
fn predictions_are_only_recalled_when_asked_for() {
    let mut s = store("predictions");
    let mut prediction = Trace::new(
        TraceKind::Prediction,
        "Is the billing migration risky? → true",
        "model:jev@jev-1",
    );
    prediction.prediction = Some(PredictionRecord {
        request_id: "req_x".into(),
        question_id: "risky".into(),
        family: "risky".into(),
        answer_kind: "noul".into(),
        engine: "jev@jev-1".into(),
        p: Some(0.8),
        value: "true".into(),
        probabilities: BTreeMap::new(),
        answer_space: vec!["true".into(), "false".into()],
        calibrated: false,
    });
    s.remember(prediction).unwrap();
    assert!(s
        .recall(&RecallQuery::new("billing migration risky"))
        .is_empty());
    let mut query = RecallQuery::new("billing migration risky");
    query.kinds = vec![TraceKind::Prediction];
    assert_eq!(s.recall(&query).len(), 1);
}

#[test]
fn unicode_tags_match_case_insensitively() {
    let mut s = store("tags");
    let mut trace = Trace::new(TraceKind::Observation, "Katalog güncellendi", "test");
    trace.tags = vec!["Ürün".into()];
    s.remember(trace).unwrap();
    let mut query = RecallQuery::new("");
    query.tags = vec!["ürün".into()];
    assert_eq!(s.recall(&query).len(), 1);
}

#[test]
fn overdue_commitments_surface_without_a_lexical_match() {
    let mut s = store("commitment");
    let mut commitment = Trace::new(TraceKind::Commitment, "Send the invoice to Acme", "test");
    commitment.deadline_ms = Some(1);
    s.remember(commitment).unwrap();
    let results = s.recall(&RecallQuery::new("quarterly roadmap"));
    assert_eq!(results.len(), 1);
}

#[test]
fn a_long_question_still_recalls_a_one_keyword_match() {
    let mut s = store("long-question");
    let id = add(
        &mut s,
        "Postgres migration was rolled back after lock contention",
    );
    let results = s.recall(&RecallQuery::new(
        "why did the database migration fail last night in production",
    ));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].trace.id, id);
}

#[test]
fn stopword_only_queries_match_nothing() {
    let mut s = store("stopwords");
    add(&mut s, "Office city is Dubai");
    assert!(s.recall(&RecallQuery::new("will the")).is_empty());
}

#[test]
fn inflected_and_mixed_script_words_match() {
    let mut s = store("inflection");
    let settings = add(&mut s, "The settings page loads slowly");
    let bug = add(&mut s, "修复了API的bug");
    assert_eq!(s.recall(&RecallQuery::new("setting"))[0].trace.id, settings);
    assert_eq!(s.recall(&RecallQuery::new("api bug"))[0].trace.id, bug);
}
