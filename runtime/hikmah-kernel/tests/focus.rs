mod common;

use common::temp_store;
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::recall::RecallQuery;
use hikmah_kernel::trace::{Trace, TraceKind};
use hikmah_kernel::MemoryStore;

fn store_with_limit(name: &str, working_set_limit: usize) -> MemoryStore {
    let policy = KernelPolicy {
        working_set_limit,
        ..KernelPolicy::default()
    };
    MemoryStore::open(temp_store(name), policy).unwrap()
}

fn add(store: &mut MemoryStore, content: &str) -> String {
    store
        .remember(Trace::new(TraceKind::Observation, content, "test"))
        .unwrap()
        .0
        .id
}

#[test]
fn working_set_limit_bounds_the_capsule_across_recalls() {
    let mut s = store_with_limit("focus-limit", 3);
    add(&mut s, "Deployment failed on the migration lock");
    add(&mut s, "Deployment rollback took twenty minutes");
    add(&mut s, "Billing invoices are generated nightly");
    add(&mut s, "Billing retries use exponential backoff");

    let mut capsule = s.focus(&RecallQuery::new("deployment"));
    assert_eq!(capsule.capacity, 3);
    assert_eq!(capsule.items.len(), 2);

    capsule.absorb(s.recall(&RecallQuery::new("billing")));
    assert_eq!(capsule.items.len(), 3, "capacity is enforced after merging");

    // Absorbing the same results again does not duplicate traces.
    capsule.absorb(s.recall(&RecallQuery::new("billing")));
    let mut ids: Vec<_> = capsule.items.iter().map(|i| i.trace.id.clone()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), capsule.items.len());
}

#[test]
fn pinned_traces_survive_eviction() {
    let mut s = store_with_limit("focus-pin", 1);
    let kept = add(&mut s, "Deployment failed on the migration lock");
    add(&mut s, "Billing invoices are generated nightly");

    let mut capsule = s.focus(&RecallQuery::new("deployment"));
    assert!(capsule.pin(&kept));
    capsule.absorb(s.recall(&RecallQuery::new("billing invoices nightly")));
    assert_eq!(capsule.items.len(), 1);
    assert_eq!(capsule.items[0].trace.id, kept);
    assert!(capsule.items[0].pinned);
}
