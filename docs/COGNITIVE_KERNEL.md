# Hikmah Cognitive Kernel

Hikmah Stack 3 introduces a **deterministic co-model runtime** beside the language model. The kernel is intentionally not a transformer, recurrent neural network, state-space network, or embedding model. It is an inspectable state-and-control system written in Rust.

The language model becomes a **proposal surface**. It may suggest interpretations, plans, summaries, or candidate claims. The kernel owns memory integrity, provenance, contradictions, commitments, deterministic scoring, policy gates, verification state, and durable learning records.

## First-principles invariants

We did not begin with “use a graph/vector database/agent framework.” We began with properties a trustworthy cognitive system must preserve:

1. **Identity:** know which memory, claim, commitment, or action is being referenced.
2. **Provenance:** know where consequential information came from and whether it was verified.
3. **Non-destructive correction:** new evidence can supersede an old belief without erasing history.
4. **Contradiction visibility:** incompatible claims coexist as an unresolved conflict until evidence resolves them.
5. **Contextual recall:** retrieval changes with the current goal, cues, time, and consequences.
6. **Selective forgetting:** access can decay without silently destroying the audit trail; true deletion needs a privacy-aware storage policy.
7. **Prospective memory:** future commitments are first-class memories, not prose buried in chat.
8. **Bounded attention:** only a small working set should dominate deliberation at once.
9. **Parallel challenge:** evidence, memory, risk, human impact, and delivery can be evaluated independently before arbitration.
10. **No hidden authority:** a generative model cannot directly rewrite durable truth or authorize irreversible action merely because it produced fluent text.
11. **Outcome learning:** completed actions must write back what actually happened, not only what was planned.
12. **Calibrated uncertainty:** “unknown” is a valid state and must survive serialization.

## Cognitive cycle

```text
SENSE -> ENCODE -> RECALL -> DELIBERATE -> DECIDE -> ACT -> VERIFY -> CONSOLIDATE
            ^          |                                  |            |
            |          +------- contradiction ------------+            |
            +-------------------- outcome / correction ----------------+
```

Each transition produces inspectable artifacts. The kernel does not need a model to maintain this cycle. A model can be attached through `ProposalEngine`, but its output enters as untrusted proposals.

## The six subsystems

### 1. TraceWeave memory

Durable memory is a collection of independent traces. Associations are **computed at recall time** from multiple channels instead of stored as permanent edges. See [MEMORY.md](MEMORY.md).

### 2. Amanah Ledger

Memory mutations are append-only, sequence-numbered, and hash-chained. A correction creates a new trace and can supersede an older trace. This makes history inspectable and tamper-evident.

### 3. CounterTrace

Structured claims may carry a `claim_key` and `claim_value`. When a new active trace asserts a different value for the same key, the kernel emits a conflict rather than selecting whichever sentence arrived last.

### 4. Deliberation Lanes

Evidence, memory integrity, irreversible risk, human impact, and delivery completeness run as independent lanes. The current Rust implementation executes these lanes concurrently and returns an explicit arbitration signal.

### 5. Branch Loom planner

A bounded symbolic planner explores explicit world states and actions without a neural network. It is intentionally simple and auditable: preconditions, additions, removals, goal facts, and a maximum depth. This gives the co-model a non-generative planning baseline against which future learned planners can be measured.

### 6. Decision Forge runtime

Decision frames use explicit criteria, weights, evidence coverage, hard blocks, and reversibility. Missing evidence reduces confidence instead of being silently treated as a zero or a guess.

### 7. Model Port

`ProposalEngine` is a trait, not an assumption about architecture. A future local co-model may be symbolic, search-based, state-space, neural, hybrid, or something not yet invented. The kernel contract stays stable.

## Why not a knowledge graph?

A graph is useful when explicit relations are themselves the product. It is not the default memory substrate here. Human recall does not appear to be a static traversal of a database graph; memories are distributed, linked, reconstructed, replayed, and selectively consolidated. Hikmah therefore stores **traces** and lets a contextual resonance function create a temporary activation path for each recall.

If a domain genuinely needs explicit relations, a graph can be added as one **index or view** over traces. It must not become the ontology that every memory is forced to obey.

## Why not “our own neural network” immediately?

Novelty does not excuse skipping falsifiability. Training a new foundation architecture before we have a measurable cognitive contract would make it impossible to know whether improvement came from architecture, data, memory, prompting, or evaluation leakage.

Hikmah inverts the order:

1. define cognitive invariants;
2. implement deterministic state and evaluation;
3. measure failure modes;
4. attach multiple proposal engines behind the same interface;
5. only then invent/train new learned components for the bottlenecks the measurements reveal.

This keeps architecture research honest and lets a non-neural co-model already provide value today.
