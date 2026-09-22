# TraceWeave Memory

TraceWeave is Hikmah Stack's human-inspired, machine-auditable memory architecture. It borrows principles from biological memory research without claiming to reproduce a brain.

## Human inspiration, carefully stated

Recent research supports several useful design ideas: memory representations can change during consolidation; replay can reactivate past experience; human hippocampal-entorhinal neurons encode temporal structure; and recall/consolidation are selective rather than a perfect recording. These findings motivate **dynamic association, replay, selectivity, and reconstruction**, not a literal neuron simulator.

Research notes and limitations are recorded in [RESEARCH.md](RESEARCH.md).

## Memory types

Hikmah stores different cognitive responsibilities as different trace kinds:

| Trace | Human analogue | Purpose |
|---|---|---|
| `observation` | perceptual/working input | a directly observed piece of current evidence |
| `episode` | episodic memory | what happened in a bounded event or session |
| `belief` | semantic memory | a proposition believed with explicit provenance/confidence |
| `procedure` | procedural memory | a reusable method or playbook |
| `commitment` | prospective memory | something that must happen later, optionally with a deadline |
| `preference` | personal/contextual preference | a stable preference, scoped and revisable |
| `constraint` | task/environment boundary | a condition that must remain true |
| `outcome` | feedback memory | what actually happened after an action |
| `correction` | reconsolidation input | evidence that updates or supersedes a prior trace |

## A memory trace

Every trace can carry:

- stable ID;
- kind;
- content;
- tags;
- creation time and optional deadline;
- salience;
- confidence;
- privacy class;
- provenance source, locator, authority, and verification flag;
- optional structured `claim_key` / `claim_value`;
- optional `supersedes` link for correction.

The point is not metadata maximalism. The point is to retain the minimum information required to answer: **what do we think we know, why, from where, when, how strongly, and what changed it?**

## Resonance Recall

TraceWeave does not store permanent semantic edges. A query produces a temporary activation path. Relevance gates the result, and metadata only scales it:

```text
lexical = max(0.7 × query-term coverage + 0.3 × Jaccard overlap, 0.15)   if any query term matches, else 0
cue     = 0.8 × lexical + 0.2 × tag coverage        (just one of them when the query has only terms or only tags)
meta    = 0.25 × recency + 0.20 × salience + 0.20 × confidence + 0.25 × provenance + 0.10 × commitment urgency
score   = cue × (0.55 + 0.45 × meta)                a trace with cue < minimum_recall_score (0.12) is not recalled
```

Recency is `1 / (1 + age_days / 30)`. Provenance is `authority × (1 if verified, else 0.65)`. Every number above is a default field of the kernel policy (`KernelPolicy.recall` in `runtime/hikmah-kernel/src/policy.rs`). They are explicit design choices, not calibrated values. A policy file can change them (`hikmah --policy <file>` or `HIKMAH_POLICY`); `hikmah policy --print-defaults` lists them all. The current implementation uses deterministic token overlap, not embeddings. An embedding/local-model channel may be added later behind an adapter, but it cannot replace provenance or contradiction controls.

After scoring, **suppression** reduces redundant near-duplicate recalls. The result is a small, diverse working set rather than a dump of everything vaguely related. A claim is never folded into, or penalized against, a claim it contradicts.

Each recall result also carries what challenges it:

- `conflicts`: ids of other active traces with the same normalized claim key and a different normalized value (unresolved);
- `supersedes`: the trace a correction replaced;
- `superseded_by`: the replacement. Superseded traces are recalled only with `--include-superseded` (history), so this is set only then.

`hikmah conflicts` lists every open conflict, grouped by normalized key and value. Conflicts are recomputed from current state, so a supersession or purge resolves them.

## Quiet Replay and consolidation

Consolidation is not “summarize chat and save it.” Replay should inspect repeated episodes/observations, independent sources, outcomes, corrections, and contradictions. The reference kernel now groups compatible structured claims, counts independent sources, measures verification/confidence, and emits `ConsolidationProposal` records. Conflicting values prevent automatic eligibility. Durable promotion remains explicit.

Important design rule: **replay produces a proposal; it does not silently manufacture truth.**

## Reconsolidation

A correction should not mutate yesterday's record in place. The system writes a new trace, records what it supersedes, and keeps the old trace marked superseded. This preserves both the current state and the history of how it changed.

## Forgetting

Human forgetting is not equivalent to deleting a row. Hikmah separates:

1. **accessibility decay:** older, low-salience traces receive less recall weight among relevant results (metadata scales relevance; it never makes an irrelevant trace recallable);
2. **supersession:** old beliefs stop being active when replaced;
3. **retention deletion:** privacy/legal deletion is a storage operation, not a cognitive heuristic.

The current append-only reference ledger deliberately refuses `sensitive` persistence unless policy explicitly enables it. A production deployment that stores sensitive payloads should use an encrypted vault with key destruction or another deletion-capable storage layer. Tamper evidence and right-to-delete must be designed together rather than hand-waved.

## Working memory: Focus Capsule

`recall_limit` bounds one recall. `MemoryStore::focus` starts a Focus Capsule, a working set that can absorb several recalls and holds at most `working_set_limit` traces. Pinned traces are never evicted; otherwise the lowest-activation trace leaves first. The capsule is an in-memory library view, not a CLI command or a durable state transition. The agent should deliberate over the smallest set of traces that changes the decision. More context is not automatically more cognition.

## Prospective memory: Promise Queue

Commitments are first-class traces. Deadline proximity contributes to recall, so “remember to do X” can become an inspectable pending obligation rather than a sentence that vanishes after context compaction.

## Memory poisoning controls

Before durable memory writes:

- distinguish user statement from verified external fact;
- keep source/authority separate from confidence;
- do not auto-promote model output into belief;
- quarantine contradictory or suspicious claims rather than overwriting;
- never persist secrets merely because they appeared in conversation (`remember` and every other write refuse a trace whose content, tags, claim key or value, source, or locator matches the credential detector in `secrets.rs`; it recognizes well-known credential shapes such as tokens, keys, `KEY=value` assignments, and passwords in URLs, and is not a DLP system);
- scope preferences to the person/project/context that supplied them;
- attach outcomes to prior actions so failed plans do not become success-pattern memories (`hikmah outcome` links an observed outcome to a recorded prediction);
- never let model output verify itself: `model:` sources cannot be verified, cannot supersede, and cannot resolve predictions.
