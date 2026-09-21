# Architecture

Hikmah Stack 3 uses a **portable skills + deterministic cognitive kernel + thin host adapters** architecture.

## Layer 1: portable cognitive doctrine

`skills/`, `playbooks/`, and `lenses/` describe the invariants and operating methods. They are the source of behavioral truth and should remain useful even when every vendor adapter is removed.

## Layer 2: Hikmah Cognitive Kernel

`runtime/hikmah-kernel/` is the non-neural co-model runtime. It stores inspectable state and performs operations that should not depend on one language model's hidden activations:

- hash-chained TraceWeave memory ledger;
- structured claim conflict detection;
- contextual multi-channel recall and redundancy suppression;
- commitment/deadline recall;
- deterministic decision scoring with evidence coverage;
- deterministic evidence/memory/risk/human-impact/delivery lanes (risk and human impact veto on one item);
- completion hygiene hook;
- repository validation;
- model-agnostic `ProposalEngine` (text) and `DecisionEngine` (typed) interfaces;
- outcome write-back and calibration of recorded predictions.

## Layer 3: proposal and decision engines

Engines are optional. A proposal engine returns text; a decision engine (see [Typed Decision Port](DECISION_PORT.md)) returns typed answers with probabilities. Either may be a frontier API model, a System One model such as TypeSafe's Jev, a small local model, a retrieval+rules system, a symbolic search engine, or a future architecture. Neither becomes durable truth by returning output: typed answers are admitted only after validation against the questions asked, and are stored as unverified `prediction` traces.

## Layer 4: host adapters

- `.codex-plugin/` packages skills/hooks for ChatGPT/Codex.
- `.claude-plugin/` exposes the same portable core to Claude Code.
- `agents/hikmah-orchestrator.md` is a Claude-specific convenience adapter.
- `.agents/plugins/marketplace.json` supports repo-scoped OpenAI development/testing.

## Rust-first Truth Gate

The primary completion gate is `hikmah hook`. `hooks/truth_gate.sh` runs the plugin's `bin/hikmah`, then a `hikmah` on PATH, then the Python fallback, and finally allows. It checks each candidate's output and always exits 0 with JSON, so a stale or broken binary can neither block a turn nor loop the host. It never compiles code at stop time, because cargo and rustup read configuration from the user's project directory. Rust and Python are tested against the same golden cases (`hooks/truth_gate_cases.json`). With `HIKMAH_HOOK_ENGINE=jev`, the Rust gate asks Jev one yes/no question and falls back to the deterministic rules on any engine problem.

## No forced graph/vector database

Explicit graphs and embeddings are optional **views/channels**, not the memory ontology. TraceWeave keeps source records independent and computes associations dynamically during recall. Add a graph only when explicit relationships are themselves required; add embeddings only when deterministic cues measurably miss relevant memories.

## Why no MCP server by default

The kernel is local and file-backed. An MCP server becomes useful when remote/multi-process tools need controlled access to memory/actions. Until then, adding an always-on server increases attack surface without improving the cognitive contract.
