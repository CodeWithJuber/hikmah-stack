
# Changelog

## 3.1.0 - 2026-09-21

### Typed decision port and Jev
- Added the **typed decision port** (`decision_port.rs`): `choice`, `score`, and `noul` questions, a `DecisionEngine` trait, `NoEngine`, `StaticEngine`, and kernel-side admission that rejects a whole response on any violation, including a choice that is not the most probable option of its own distribution, non-canonical score keys, and a score that disagrees with its distribution. The probability-sum tolerance scales with the number of options to allow two-decimal rounding. Unanswered questions become explicit abstentions; engine failures become all-abstain decisions. Every outbound string (state, instructions, options, levels) is checked for credentials.
- Added an opt-in **TypeSafe Jev adapter** (`jev.rs`, default `jev` cargo feature) with retries inside a time budget, the platform certificate verifier, a redacted key, and malformed-answer rejection. Tests replay a captured `jev-1.13.0` response; a live test is available behind `--ignored`.
- Added **prediction and outcome traces** and `hikmah calibration` (Brier, base-rate Brier, ECE, observed rate per engine and family). Model-authored traces cannot be verified, supersede, or resolve predictions. Outcomes must be a value from the prediction's answer space; purged or superseded outcomes do not count; predictions without any reported probability are recorded as unknown and counted separately, never given an invented value.
- Added CLI commands `ask`, `outcome`, `calibration`, `fulfill`, and `purge`, plus `remember --deadline/--authority/--locator`, `recall --kind`, `verify-ledger --expect-head`, and `decide --engine` (engine estimates count toward ranking, never toward evidence coverage).
- The Truth Gate can use a decision engine (`HIKMAH_HOOK_ENGINE=jev`) and falls back to the deterministic rules on any engine problem.

### Ledger (fixes)
- A `--supersedes` pointing at a missing trace no longer bricks the store: every event is validated before anything is written, and replay reports unapplicable legacy events as warnings instead of refusing to open.
- Concurrent writers no longer fork the chain: appends take an exclusive file lock and re-read records other processes appended.
- Each batch is written with a single `write_all`; a torn final line is ignored on read and truncated by the next writer; a record missing its final newline is kept.
- New records (format v2) hash the exact payload bytes on disk, so schema growth never breaks old hashes and injected payload fields are detected. v1 records still verify. Unknown top-level fields are rejected.
- A `<store>.head` file and `verify-ledger --expect-head` detect truncation and re-chaining. The head is read before the ledger, so a concurrent writer cannot cause a false alarm, and writes are refused while the ledger and its head disagree, so an ordinary write cannot paper over a truncation. `verify-ledger --reset-head` accepts the current ledger after a deliberate repair.
- New stores are created without truncation, so concurrent first writes no longer erase each other. Torn tails that end inside a multi-byte character, or in non-JSON whitespace, are repaired instead of making the store unreadable.
- **Downgrade is one-way:** once 3.1.0 appends a v2 record, a 3.0.0 binary cannot read the ledger.
- Read-only commands no longer create a store at a mistyped path.
- Status transitions are constrained (only active commitments can be fulfilled; purge needs a reason and cannot repeat; no self-supersession). Superseding a verified trace requires a verified correction, and a correction no longer reports a conflict with the trace it replaces.

### Recall, claims, decisions, lanes (fixes)
- Recall is **relevance-gated**: a trace must match the query's terms or tags. The minimum score applies to relevance, and any shared content term counts at least 0.15, so a long question that shares one key word still recalls the memory. Metadata scales relevance but can no longer surface an unrelated trace. A query made only of stopwords matches nothing. Near-identical traces are folded with a duplicate count. Stopwords, light stemming, digits, CJK bigrams, and Unicode tag case are handled. Sensitive traces stay out of recall unless the policy allows them. Predictions are excluded by default.
- Claim keys compare case-insensitively after Unicode NFC normalization; values keep their case. Consolidation counts independent sources after normalization, requires a minimum blended confidence (0.6), ignores model-authored traces, and shows original values.
- Decisions treat a missing criterion as unknown rather than zero, validate weights (finite, non-negative, unique ids), report `recommended` / `no_admissible_option`, and prefer a reversible option within 0.02 when evidence is weak.
- The risk and human-impact lanes now veto on a single item. Lanes run sequentially (the previous threads added no independence).
- The planner has a state budget (`max_states`, default 100,000) and a depth cap of 64.

### Hooks and packaging (fixes)
- The Truth Gate matches whole words only (`incomplete`, `already`, `Mastodon` no longer trigger it), ignores negated words and code spans, and normalizes text identically in Rust and Python (NFC, apostrophes, zero-width characters, whitespace, ASCII word boundaries). The two agreed on all 4,026 generated messages in a parity fuzz, and share golden cases in CI. Negation look-back is bounded, so long text stays linear. Payloads with lone surrogate escapes or out-of-range numbers no longer disable the Rust gate. The hook caps the Jev budget at 3 s whatever `HIKMAH_JEV_TIMEOUT_MS` says.
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
