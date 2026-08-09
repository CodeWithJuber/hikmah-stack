# Agent instructions for this repository

Preserve Hikmah Stack's **first-principles cognitive architecture**.

1. `skills/`, `playbooks/`, and `lenses/` define portable doctrine.
2. `runtime/hikmah-kernel/` owns deterministic cognitive state and controls.
3. Host/model adapters must remain replaceable.
4. Do not add a graph, vector database, embedding model, MCP server, agent framework, or neural architecture merely because it is fashionable. Name the invariant/failure it solves and the metric that will prove the improvement.
5. Never persist hidden chain-of-thought. Persist inspectable facts, provenance, constraints, commitments, outcomes, corrections, preferences, and concise decision records.
6. Never allow model output to become durable verified truth without provenance and an explicit state transition.
7. Do not average away privacy, consent, security, or other hard blocks.
8. Keep sensitive-memory policy explicit; an append-only audit ledger is not automatically deletion-compliant.

Before declaring a change complete:
- run `cargo fmt --check`;
- run `cargo clippy --workspace --all-targets`;
- run `cargo test --workspace`;
- run `cargo run -p hikmah-kernel -- validate --root .`;
- inspect changed files and factual compatibility claims;
- turn discovered failures into regression cases when feasible.
