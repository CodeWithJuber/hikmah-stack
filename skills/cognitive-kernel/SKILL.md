---
name: cognitive-kernel
description: >
  Use when designing or operating persistent AI memory, local co-models, cognitive runtimes,
  agent learning, parallel deliberation, model-independent state, provenance, contradiction
  handling, or human-inspired memory systems. Prefer first-principles cognitive invariants over
  defaulting to vector databases, knowledge graphs, transformers, or fashionable agent patterns.
metadata:
  version: "3.1.0"
  source: "Hikmah Cognitive Kernel"
---

# Cognitive Kernel — Persistent, Inspectable Intelligence

Use this skill when the system needs to remain coherent across turns, sessions, model swaps, or failures.

## Prime directive

**Do not ask a generative model to be the sole owner of memory, truth, policy, and self-evaluation.** Put durable cognitive responsibilities into inspectable mechanisms.

## Cognitive invariants

1. Every durable memory has identity and provenance.
2. Confidence and source authority are separate fields.
3. Corrections supersede; they do not erase history.
4. Contradictions remain visible until resolved.
5. Recall is contextual and bounded.
6. Commitments are prospective memories, not prose reminders.
7. Outcomes write back after action.
8. Sensitive memory obeys storage/retention policy before convenience.
9. Models propose; deterministic gates authorize durable state transitions.
10. Higher stakes activate independent evidence, risk, human-impact, memory, and delivery checks.

## TraceWeave memory cycle

### Encode
Classify the incoming item as observation, episode, belief, procedure, commitment, preference, constraint, outcome, or correction. Preserve source, time, confidence, and scope.

### Recall
Activate relevant traces by multiple cues. Do not rely on one embedding similarity score. Include correction/conflict/prospective urgency signals and suppress redundant results.

### Deliberate
Run relevant challenge lanes independently. Do not let the same generator both invent evidence and certify it.

### Decide
Use explicit criteria, hard constraints, reversibility, evidence coverage, and human accountability.

### Verify
Compare claimed outcome with observable outcome. Missing verification is not success.

### Consolidate
Replay episodes, outcomes, and corrections. Promote a durable belief/procedure only when repeated evidence justifies it. Consolidation is a proposal until accepted.

## Architecture selection rule

Do not begin with “graph/vector DB/RAG/transformer/RNN.” Begin with the invariant and then choose a mechanism:

- exact identity -> keyed trace;
- audit history -> append/hash-chained ledger;
- contextual association -> dynamic resonance;
- contradiction -> structured claim comparison;
- deadline -> prospective queue;
- bounded attention -> focus capsule;
- independent challenge -> parallel lanes;
- language generation -> optional proposal engine;
- explicit relations -> graph view only when relations are the actual problem;
- fuzzy semantic search -> optional embedding channel only when deterministic cues are insufficient.

## No-perfect-engine rule

Never claim general perfection. For each module define:

- what is guaranteed by construction;
- what is tested empirically;
- what remains heuristic;
- what requires a human or external authority;
- how a discovered failure becomes a regression case.

## Use the implementation

Reference runtime: `runtime/hikmah-kernel/`.

Playbooks:
- `playbooks/remember-recall-consolidate.md`
- `playbooks/parallel-deliberation.md`
- `playbooks/error-to-learning.md`

Lenses:
- `lenses/memory-integrity.md`
- `lenses/model-independence.md`
- `lenses/human-memory-inspiration.md`

## Execute with the kernel

This skill is doctrine; the `hikmah` binary is the mechanism. Requires the `hikmah` binary on PATH; the plugin does not install it. Until the next release reaches crates.io, install it from source: `cargo install --git https://github.com/CodeWithJuber/hikmah-stack hikmah-kernel --locked` (the crates.io 3.1.0 build lacks `remember-batch` and the `HIKMAH_AGENT_SESSION=0` opt-out). If `command -v hikmah` fails, say so and skip these commands instead of improvising.

- `hikmah init` — create the ledger-backed store (`.hikmah/memory.jsonl`). Add `.hikmah/` to the project's `.gitignore`: the store holds private content and agent-session locators.
- `hikmah remember --kind <observation|belief|commitment|...> --content ... --source ...` — encode a typed trace with provenance.
- `hikmah remember-batch --batch-file records.jsonl` — encode many traces in one process (one JSON object per line).
- `hikmah recall --query ...` — relevance-gated recall; every hit carries a `trace_id` and its relevance channels.
- `hikmah get <trace-id>` / `hikmah why <trace-id>` — fetch a trace / print its full provenance chain.
- `hikmah conflicts` — list open contradictions (detection only; the kernel never picks a winner).
- `hikmah verify-ledger` — prove the chain is intact.
