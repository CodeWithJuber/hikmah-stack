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

Each transition produces inspectable artifacts. The kernel does not need a model to maintain this cycle. A model can be attached through `ProposalEngine` (text) or `DecisionEngine` (typed; see [DECISION_PORT.md](DECISION_PORT.md)), but its output enters as untrusted proposals: typed answers are validated all-or-nothing against the questions asked and stored only as unverified predictions until an outcome from a non-model principal resolves them.

## The six subsystems

### 1. TraceWeave memory

Durable memory is a collection of independent traces. Associations are **computed at recall time** from multiple channels instead of stored as permanent edges. See [MEMORY.md](MEMORY.md).

### 2. Amanah Ledger

Memory mutations are append-only, sequence-numbered, and hash-chained. A correction creates a new trace and can supersede an older trace. This makes history inspectable and tamper-evident.

### 3. CounterTrace

Structured claims may carry a `claim_key` and `claim_value`. When a new active trace asserts a different value for the same key, the kernel emits a conflict rather than selecting whichever sentence arrived last. Conflicts are derived from current state rather than stored: every recall result lists the ids of other active traces whose claim disagrees with it (`conflicts`), a correction shows the trace it replaced (`supersedes`), and `hikmah conflicts` lists every open disagreement. A supersession or purge resolves a conflict; there is no separate resolve event. Redundancy folding never folds a claim into the claim it contradicts.

### 4. Deliberation Lanes

Evidence, memory integrity, irreversible risk, human impact, and delivery completeness are independent lanes: each reads only its own input, and none sees another lane's verdict. The current Rust implementation (`council.rs`) evaluates them one after another, sequentially and deterministically, over counts the caller (or an engine, through the typed port) supplies. It returns an explicit arbitration signal: `can_proceed`, the `blocking_lanes`, and one signal per lane. The risk and human-impact lanes veto on a single item. The lanes are rules over counts, not agents or threads. 3.1.0 removed the earlier threads because they added no independence.

### 5. Branch Loom planner

A bounded symbolic planner explores explicit world states and actions without a neural network. It is intentionally simple and auditable: preconditions, additions, removals, goal facts, and a maximum depth. This gives the co-model a non-generative planning baseline against which future learned planners can be measured.

### 6. Decision Forge runtime

Decision frames use explicit criteria, weights, evidence coverage, hard blocks, and reversibility. Missing evidence is neither treated as a zero nor filled with a guess, such as the average of the known criteria. An unscored criterion could be anywhere on the score scale.

- **Score interval.** Each option gets `score_interval = [lo, hi]`. `lo` is the weighted score with every unscored criterion at the scale minimum, and `hi` is the same with each at the maximum. This is interval arithmetic with no invented prior.
- **Ranking.** Admissible options rank by `lo`, then `hi`, then reversibility, then name. An option with one excellent score and several unknowns cannot outrank a fully evidenced option whose guaranteed score is higher.
- **Decisiveness.** `decisive` uses `evidence_interval`, the same interval with model-estimated criteria treated as unknown. It is true only when the recommended option's evidence `lo` is strictly greater than every other admissible option's evidence `hi`. Then neither measuring the unknowns nor an engine estimate proving wrong could change the winner. Exact ties are not decisive.
- **Hard blocks and reversibility.** Hard blocks still rank last. The reversibility preference applies to `lo`.

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
