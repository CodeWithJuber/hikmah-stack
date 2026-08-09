# Hikmah Stack

**Judgment + cognitive infrastructure for agentic AI.**

Hikmah Stack is an open-source, model-independent capability system for AI agents. Version **3.0.0** adds the **Hikmah Cognitive Kernel**, a deterministic Rust co-model runtime that owns persistent memory integrity, provenance, contradictions, commitments, decision scoring, parallel challenge lanes, and verification state outside a language model's hidden activations.

> MIT · Maintained by [Juber Shaikh](https://github.com/CodeWithJuber)

## The idea

Do not build an “agent brain” by stacking a bigger prompt, vector DB, graph, and another transformer. Start with cognitive invariants and choose a mechanism only when it earns its complexity.

```text
                    optional proposal engines
          GPT / Claude / local LM / rules / search / future model
                               |
                               v
+------------------------------------------------------------------+
|                    HIKMAH COGNITIVE KERNEL                        |
| TraceWeave | CounterTrace | Decision Forge | Deliberation | Gate |
+-------------------------------+----------------------------------+
                                |
              append-only provenance + outcome memory
                                |
                  portable Hikmah Stack skills
```

The kernel itself is **not a neural network and not a transformer**. Learned models are optional proposal engines behind a stable interface. That means a model can be swapped without deleting the agent's commitments, provenance, policy, or correction history.

## Capability stack

| Capability | Job |
|---|---|
| **Operator Core** | Human judgment, leadership, pressure, ethics, communication, recovery, stewardship |
| **Agent Radar** | Hallucination, loops, context loss, sycophancy, slop, opacity, cost, memory failures |
| **Decision Forge** | Options, uncertainty, evidence coverage, hard constraints, reversibility, action |
| **Ship Guard** | Acceptance criteria, verification, rollback, handoffs, completion discipline, outcome write-back |
| **Hikmah Orchestrator** | Cross-skill routing and synthesis |
| **Cognitive Kernel** | Persistent memory, co-model runtime, contradiction handling, model-independent learning |
| **Truth Gate** | Narrow completion hygiene; Rust-first runtime with zero-install compatibility fallback |

## TraceWeave: memory without forcing a graph

A memory is an immutable **trace**: kind, content, provenance, confidence, salience, privacy, time, optional deadline, optional structured claim, and correction history.

At recall time, a **resonance wave** scores traces through lexical cues, tags, recency, salience, confidence, provenance, and prospective urgency. Redundant traces are suppressed. The recall “path” emerges for the current goal rather than being stored forever as graph edges.

Read [Memory](docs/MEMORY.md) and [Cognitive Kernel](docs/COGNITIVE_KERNEL.md).

## Human-inspired, not brain cosplay

Hikmah borrows testable principles from memory research: selective consolidation, replay, temporal structure, reconsolidation-like correction, bounded attention, and prospective memory. Biology inspires mechanisms; it does not prove them. Sources and limitations live in [Research Notes](docs/RESEARCH.md).

## Rust kernel

```bash
cargo run -p hikmah-kernel -- validate --root .
cargo run -p hikmah-kernel -- init
cargo run -p hikmah-kernel -- remember \
  --kind observation \
  --content "Migration failed because the lock timed out" \
  --tag database --tag deployment --verified
cargo run -p hikmah-kernel -- recall --query "why did the deployment fail"
cargo run -p hikmah-kernel -- consolidate
cargo run -p hikmah-kernel -- commitments --within-hours 168
```

A non-neural symbolic plan and a structured decision frame can be evaluated with:

```bash
cargo run -p hikmah-kernel -- plan --problem examples/plan-problem.json
```

Then:

```bash
cargo run -p hikmah-kernel -- decide --frame examples/decision-frame.json
```

The kernel also exposes a narrow `ProposalEngine` trait so neural, symbolic, search-based, state-space, or future architectures can be benchmarked behind the same cognitive contract.

## Playbooks

- [Remember → Recall → Consolidate](playbooks/remember-recall-consolidate.md)
- [Parallel Deliberation](playbooks/parallel-deliberation.md)
- [Error → Durable Learning](playbooks/error-to-learning.md)

## Lenses

- [Memory Integrity](lenses/memory-integrity.md)
- [Model Independence](lenses/model-independence.md)
- [Human Memory Inspiration](lenses/human-memory-inspiration.md)

## Design doctrine

1. Evidence before confidence.
2. Memory is typed, scoped, provenance-bearing state, not hidden chain-of-thought.
3. Corrections supersede; contradictions remain visible.
4. Models propose; durable state transitions are gated.
5. Human-impact, privacy, and hard safety constraints cannot be averaged away by a high score.
6. Parallel challenge beats one model grading itself.
7. Outcomes teach more than plans.
8. Unknown is a valid serialized state.
9. Novel architecture must beat a measurable baseline, not merely sound futuristic.
10. Never claim a perfect general engine; build a perfectibility loop where failures become tests and controls.

## Install as skills/plugin

### ChatGPT + Codex

```bash
codex plugin marketplace add CodeWithJuber/hikmah-stack
```

### Claude Code

```text
/plugin marketplace add CodeWithJuber/hikmah-stack
/plugin install hikmah-stack@hikmah-stack
/reload-plugins
```

### Kimi

The repository root carries `kimi.plugin.json`, so the repo itself is a valid Kimi plugin bundle: all six skills load under the `hikmah-stack:*` namespace with routing guidance from the manifest's `skillInstructions`. Install by adding this repository as a Kimi plugin, or pack the directory into `bundle.zip` for the catalog flow.

### Other skill-aware agents

Use the folders under `skills/` as the portable layer. Host-specific adapters remain thin by design.

## Repository map

```text
skills/                     portable judgment/cognition skills
runtime/hikmah-kernel/      deterministic Rust co-model runtime
playbooks/                  operational cognitive loops
lenses/                     reusable diagnostic perspectives
docs/                       architecture, memory, co-model, research, ethics, evaluation
hooks/                      host completion hooks; Rust-first Truth Gate
.codex-plugin/              OpenAI adapter
.claude-plugin/             Claude Code adapter
kimi.plugin.json            Kimi adapter (repo root doubles as plugin bundle)
.github/                    CI and open-source workflows
```

## Safety / privacy

Hikmah is decision-support infrastructure, not a substitute for current qualified medical, legal, financial, security, religious, or other professional judgment. The reference ledger refuses `sensitive` persistence by default. Production deployments needing sensitive durable memory should add an encrypted, deletion-capable vault rather than pretending an append-only log satisfies every privacy requirement.

## License

MIT. See [LICENSE](LICENSE).
