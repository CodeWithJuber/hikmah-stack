
# Changelog

## 3.1.0 - 2026-09-21

### Typed decision port and Jev
- Added the **typed decision port** (`decision_port.rs`): `choice`, `score`, and `noul` questions, a `DecisionEngine` trait, `NoEngine`, `StaticEngine`, and kernel-side admission that rejects a whole response on any violation. Unanswered questions become explicit abstentions; engine failures become all-abstain decisions.
- Added an opt-in **TypeSafe Jev adapter** (`jev.rs`, default `jev` cargo feature) with retries inside a time budget, the platform certificate verifier, a redacted key, and malformed-answer rejection. Tests replay a captured `jev-1.13.0` response; a live test is available behind `--ignored`.
- Added **prediction and outcome traces** and `hikmah calibration` (Brier, base-rate Brier, ECE, observed rate per engine and family). Model-authored traces cannot be verified, supersede, or resolve predictions.
- Added CLI commands `ask`, `outcome`, `calibration`, `fulfill`, and `purge`, plus `remember --deadline/--authority/--locator`, `recall --kind`, `verify-ledger --expect-head`, and `decide --engine` (engine estimates count toward ranking, never toward evidence coverage).
- The Truth Gate can use a decision engine (`HIKMAH_HOOK_ENGINE=jev`) and falls back to the deterministic rules on any engine problem.

### Ledger (fixes)
- A `--supersedes` pointing at a missing trace no longer bricks the store: every event is validated before anything is written, and replay reports unapplicable legacy events as warnings instead of refusing to open.
- Concurrent writers no longer fork the chain: appends take an exclusive file lock and re-read records other processes appended.
- Each batch is written with a single `write_all`; a torn final line is ignored on read and truncated by the next writer; a record missing its final newline is kept.
- New records (format v2) hash the exact payload bytes on disk, so schema growth never breaks old hashes and injected payload fields are detected. v1 records still verify. Unknown top-level fields are rejected.
- A `<store>.head` file and `verify-ledger --expect-head` detect truncation and re-chaining when kept out of the writer's reach.
- Read-only commands no longer create a store at a mistyped path.
- Status transitions are constrained (only active commitments can be fulfilled; purge needs a reason and cannot repeat; no self-supersession). Superseding a verified trace requires a verified correction, and a correction no longer reports a conflict with the trace it replaces.

### Recall, claims, decisions, lanes (fixes)
- Recall is **relevance-gated**: a trace must match the query's terms or tags. Metadata scales relevance but can no longer surface an unrelated trace. Near-identical traces are folded with a duplicate count. Stopwords, light stemming, digits, CJK bigrams, and Unicode tag case are handled. Sensitive traces stay out of recall unless the policy allows them. Predictions are excluded by default.
- Claim keys compare case-insensitively after Unicode NFC normalization; values keep their case. Consolidation counts independent sources after normalization, requires a minimum blended confidence (0.6), ignores model-authored traces, and shows original values.
- Decisions treat a missing criterion as unknown rather than zero, validate weights (finite, non-negative, unique ids), report `recommended` / `no_admissible_option`, and prefer a reversible option within 0.02 when evidence is weak.
- The risk and human-impact lanes now veto on a single item. Lanes run sequentially (the previous threads added no independence).
- The planner has a state budget (`max_states`, default 100,000) and a depth cap of 64.

### Hooks and packaging (fixes)
- The Truth Gate matches whole words only (`incomplete`, `already`, `Mastodon` no longer trigger it), ignores negated words and code spans, and normalizes curly apostrophes. Rust and Python share golden cases in CI; the Python fallback no longer crashes on non-object payloads.
- `truth_gate.sh` never compiles code, checks each candidate's output, and always exits 0 with JSON. Hook commands quote the plugin root. The prompt hook allows when `stop_hook_active` is set or the request is unclear.
- `hikmah validate` now checks versions and names across all manifests, hook script paths, and skill frontmatter (`name` matches the directory, `description` present).
- The Claude orchestrator preloads `cognitive-kernel` as well. The Kimi adapter added earlier ships in this release.

## 3.0.0 - 2026-08-09

### Cognitive architecture
- Added **Hikmah Cognitive Kernel**, a deterministic Rust co-model runtime independent of transformer/neural hidden state.
- Added **TraceWeave** typed memory with hash-chained append-only provenance, dynamic multi-channel recall, redundancy suppression, structured conflicts, deadlines, and correction/supersession.
- Added parallel evidence, memory, risk, human-impact, and delivery deliberation lanes.
- Added deterministic multi-criteria decision evaluation with evidence-coverage penalties and hard blocks.
- Added model-agnostic `ProposalEngine` boundary for local, remote, symbolic, state-space, neural, or future proposal systems.

### Playbooks and lenses
- Added remember/recall/consolidate, parallel-deliberation, and error-to-learning playbooks.
- Added memory-integrity, model-independence, and human-memory-inspiration lenses.
- Integrated persistent memory and outcome learning directly into Operator Core, Agent Radar, Decision Forge, Ship Guard, and Hikmah Orchestrator.

### Research and assurance
- Added human-memory research notes, co-model architecture, memory architecture, and evaluation contract.
- Added an explicit no-perfect-engine rule: guarantees, heuristics, empirical evidence, and limitations must be separated.
- Rust Truth Gate is now primary; Python remains only a zero-install compatibility fallback.

## 2.0.0 - 2026-08-09

### Breaking
- Renamed the project from Wisdom Lens to **Hikmah Stack**.
- Renamed public skills: `wisdom-playbook` → `operator-core`, `new-lens` → `agent-radar`, `decision-engine` → `decision-forge`, `builder-protocol` → `ship-guard`.
- Renamed `wisdom-advisor` to `hikmah-orchestrator`.

### Added
- Native OpenAI `.codex-plugin/plugin.json` packaging for ChatGPT/Codex.
- Portable `hikmah-orchestrator` skill for hosts without Claude-style subagents.
- Codex command-based Truth Gate and host-specific hook separation.
- Repo-scoped OpenAI marketplace metadata.
- Compatibility, architecture, ethics, governance, CI, and validation documentation.

### Changed
- Corrected maintainer identity to Juber Shaikh.
- Reframed unsafe capacity language to acknowledge real human constraints.
- Kept empirical AI statistics in dated evidence notes rather than core doctrine.

## 1.1.0 - 2026-08-09
- Hardened the original package for open-source release, added evidence notes, licensing, contribution/security files, and a single completion quality gate.
