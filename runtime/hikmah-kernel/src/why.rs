//! Provenance chains: why a memory looks the way it does.
//!
//! A trace's provenance chain is the inspectable answer to "why does this memory
//! look like this": the trace itself with its provenance stamp, the supersession
//! lineage that corrected or replaced it, and the unresolved conflicts standing
//! beside it. No new plumbing: it reuses the ledger's supersession links and the
//! claims module's conflict detection, the same data `hikmah recall` already
//! carries per hit.

use crate::claims::conflicting_ids;
use crate::ledger::MemoryStore;
use crate::trace::{PrivacyClass, Trace, TraceStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// How far back a supersession chain is followed. A cycle or an absurdly long
/// chain is a data bug, not a reason to loop forever.
const MAX_CHAIN: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceLineage {
    pub trace: Trace,
    /// Predecessors via `supersedes`, newest first: the corrected beliefs this trace replaced.
    pub predecessors: Vec<Trace>,
    /// The trace that replaced this one, when a supersession applied.
    pub successor: Option<Trace>,
    /// Ids of active traces whose claims conflict with this one and are still unresolved.
    /// Empty for non-active traces, mirroring `hikmah recall`.
    pub conflicts: Vec<String>,
}

impl MemoryStore {
    /// The full provenance chain of one trace: itself, its supersession lineage,
    /// and its unresolved conflicts. `None` when no trace carries `id`.
    pub fn provenance_chain(&self, id: &str) -> Option<TraceLineage> {
        let entry = self.get(id)?;
        let trace = entry.trace.clone();
        let active = entry.status == TraceStatus::Active;

        let mut predecessors = Vec::new();
        let mut seen = HashSet::new();
        seen.insert(trace.id.clone());
        let mut cursor = trace.supersedes.clone();
        while let Some(old_id) = cursor {
            if predecessors.len() >= MAX_CHAIN || !seen.insert(old_id.clone()) {
                break;
            }
            let Some(old) = self.get(&old_id) else {
                break;
            };
            cursor = old.trace.supersedes.clone();
            predecessors.push(old.trace.clone());
        }

        // The successor is the trace whose supersession replaced this one. A purged
        // successor, or one hidden by the privacy filter, is not shown: its id would
        // point at something recall itself refuses to return.
        let allow_sensitive = self.policy().allow_sensitive_persistence;
        let successor = self
            .all()
            .filter(|e| e.status != TraceStatus::Purged)
            .filter(|e| allow_sensitive || e.trace.privacy != PrivacyClass::Sensitive)
            .filter(|e| e.trace.supersedes.as_deref() == Some(id))
            .map(|e| e.trace.clone())
            .next();

        let conflicts = if active {
            conflicting_ids(&trace, self.active_traces())
        } else {
            Vec::new()
        };

        Some(TraceLineage {
            trace,
            predecessors,
            successor,
            conflicts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::KernelPolicy;
    use crate::trace::{Trace, TraceKind};
    use std::ops::{Deref, DerefMut};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A scratch [`MemoryStore`] in a temp dir unique to this call: process id,
    /// wall clock, and counter. Parallel tests in one run and leftover state
    /// from previous runs can never share a store. The dir is removed when the
    /// test ends, even on panic.
    struct TestStore {
        dir: PathBuf,
        store: MemoryStore,
    }

    impl TestStore {
        fn open() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir().join(format!(
                "hikmah-why-test-{}-{nonce}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("mkdir");
            let store =
                MemoryStore::open(dir.join("memory.jsonl"), KernelPolicy::default()).expect("open");
            TestStore { dir, store }
        }
    }

    impl Drop for TestStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    impl Deref for TestStore {
        type Target = MemoryStore;
        fn deref(&self) -> &MemoryStore {
            &self.store
        }
    }

    impl DerefMut for TestStore {
        fn deref_mut(&mut self) -> &mut MemoryStore {
            &mut self.store
        }
    }

    fn open_store() -> TestStore {
        TestStore::open()
    }

    fn add(store: &mut MemoryStore, content: &str) -> Trace {
        store
            .remember(Trace::new(TraceKind::Belief, content, "human:a"))
            .expect("remember")
            .0
    }

    #[test]
    fn chain_follows_supersession_predecessors() {
        let mut store = open_store();
        let old = add(&mut store, "old belief");
        let mut corrected = Trace::new(TraceKind::Belief, "corrected belief", "human:a");
        corrected.supersedes = Some(old.id.clone());
        let new = store.remember(corrected).expect("remember").0;

        let chain = store.provenance_chain(&new.id).expect("chain");
        assert_eq!(chain.trace.id, new.id);
        assert_eq!(chain.predecessors.len(), 1);
        assert_eq!(chain.predecessors[0].id, old.id);
        assert!(chain.successor.is_none());
        assert!(chain.conflicts.is_empty());
    }

    #[test]
    fn chain_reports_the_successor() {
        let mut store = open_store();
        let old = add(&mut store, "old belief");
        let mut corrected = Trace::new(TraceKind::Belief, "corrected belief", "human:a");
        corrected.supersedes = Some(old.id.clone());
        store.remember(corrected).expect("remember");

        let chain = store.provenance_chain(&old.id).expect("chain");
        assert!(chain.predecessors.is_empty());
        let successor = chain.successor.expect("successor");
        assert_eq!(successor.supersedes.as_deref(), Some(old.id.as_str()));
    }

    #[test]
    fn chain_lists_unresolved_conflicts() {
        let mut store = open_store();
        let mut a = Trace::new(TraceKind::Belief, "the sky is green", "human:a");
        a.claim_key = Some("sky-color".into());
        a.claim_value = Some("green".into());
        let a = store.remember(a).expect("remember").0;
        let mut b = Trace::new(TraceKind::Belief, "the sky is blue", "human:b");
        b.claim_key = Some("sky-color".into());
        b.claim_value = Some("blue".into());
        store.remember(b).expect("remember");

        let chain = store.provenance_chain(&a.id).expect("chain");
        assert_eq!(chain.conflicts.len(), 1);
    }

    #[test]
    fn chain_is_none_for_unknown_id() {
        let store = open_store();
        assert!(store.provenance_chain("tr_missing").is_none());
    }
}
