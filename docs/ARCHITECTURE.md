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
- parallel evidence/memory/risk/human-impact/delivery lanes;
- completion hygiene hook;
- repository validation;
- model-agnostic `ProposalEngine` interface.

## Layer 3: proposal engines

A proposal engine is optional. It may be a frontier API model, small local model, retrieval+rules system, state-space model, symbolic search engine, or a future architecture. It cannot directly become durable truth merely by returning text.

## Layer 4: host adapters

- `.codex-plugin/` packages skills/hooks for ChatGPT/Codex.
- `.claude-plugin/` exposes the same portable core to Claude Code.
- `agents/hikmah-orchestrator.md` is a Claude-specific convenience adapter.
- `.agents/plugins/marketplace.json` supports repo-scoped OpenAI development/testing.

## Rust-first Truth Gate

The primary completion gate is `hikmah hook`. `hooks/truth_gate.sh` resolves an installed Hikmah binary first, then a local Rust toolchain. A small Python implementation remains only as a zero-install compatibility fallback so a source-installed plugin does not lose its completion hygiene on machines where the binary is not yet installed.

## No forced graph/vector database

Explicit graphs and embeddings are optional **views/channels**, not the memory ontology. TraceWeave keeps source records independent and computes associations dynamically during recall. Add a graph only when explicit relationships are themselves required; add embeddings only when deterministic cues measurably miss relevant memories.

## Why no MCP server by default

The kernel is local and file-backed. An MCP server becomes useful when remote/multi-process tools need controlled access to memory/actions. Until then, adding an always-on server increases attack surface without improving the cognitive contract.
