
# Changelog

## Unreleased

Fixes for gaps found by the research-to-implementation audit.

### Memory
- **Memory refuses credentials.** Trace validation runs the credential detector already used for outbound engine requests over content, tags, claim key and value, source, and locator. `remember`, `outcome`, `ask --record`, and every other append path reject a match with an `invalid input` error that names the field without echoing the value. Stores that already contain such text still open; only new writes are checked.
- **Recall shows conflicts and corrections beside the claim.** Each recall result now has:
  - `conflicts`: ids of other active traces whose claim has the same normalized key and a different normalized value;
  - `supersedes`: the trace a correction replaced;
  - `superseded_by`: the replacement, set when `recall --include-superseded` (new) returns history.
- New `hikmah conflicts` lists every open conflict, grouped by normalized key and value. Conflicts are derived from current state, so a supersession or purge resolves them. Sensitive traces are left out under the default policy.
- Added the missing regression test for stale-belief suppression: after a supersession, and after reopening the store, the replaced belief is not recalled by default and the latest correction is. `docs/EVALUATION.md` claimed this coverage before the test existed.
- Redundancy folding no longer folds or penalizes a claim against a claim it contradicts. Before, two near-identical sentences with different claim values could collapse into one result and hide the disagreement.

### Policy
- **The kernel policy is configurable from the CLI.** Before, every CLI command used `KernelPolicy::default()`.
  - **Loading a policy.** A new global `--policy <file.json>` (or `HIKMAH_POLICY`) loads a policy JSON. Missing fields keep their defaults. Unknown fields and out-of-range values are rejected, so a typo cannot silently do nothing.
  - **Printing it.** New `hikmah policy` prints the effective policy, and `hikmah policy --print-defaults` prints the defaults.
  - **Hook isolation.** The Truth Gate hook never reads the policy, so a bad policy file cannot break it.
- **Recall weights are policy data.** The constants that were hard-coded in `recall.rs` are now fields of `KernelPolicy.recall` with the same default values, so default behaviour is unchanged. They cover:
  - the term blend (0.7 / 0.3) and the tag blend (0.8 / 0.2);
  - the five metadata weights;
  - the 0.55 / 0.45 relevance/metadata split;
  - the 0.15 match floor;
  - the 0.8 fold threshold and the 0.35 redundancy penalty;
  - the 30-day recency scale and the 0.65 unverified factor;
  - the 7-day commitment scale, the 0.35 undated urgency, the 0.15 overdue floor, and the 0.5 listing scale.
- `docs/MEMORY.md` now shows the recall formula the code actually uses (it still showed the 3.0.0 additive sum).

### Decisions
- **Missing evidence is no longer imputed as the average of the known criteria.** Each option now reports `score_interval: [lo, hi]`, with every unscored criterion at the scale minimum for `lo` and at the maximum for `hi`. This is interval arithmetic with no invented prior.
  - **Ranking.** Admissible options rank by `lo`, then `hi`, then the existing tie-breaks.
  - **New `decisive` field.** It is true only when the recommended option's `lo` is at least every other admissible option's `hi`.
  - **Unchanged behaviour.** Hard blocks still rank last. The reversibility preference now compares lower bounds.
  - **Reference fields.** `raw_score` and `confidence_adjusted_score` are still reported but no longer order the ranking.
  - **Effect.** An option with one criterion scored 1.0 and three unknown (interval [0.25, 1.0]) used to outrank an option scored 0.6 on all four. It no longer does, and the result is marked not decisive.

### Docs
- `COGNITIVE_KERNEL.md` said the deliberation lanes run concurrently. They run sequentially and deterministically over caller-supplied counts, as the 3.1.0 notes already said. It and `CO_MODEL.md` now say "independent, not concurrent".

### Calibration
- **`calibrated` now needs statistical support, not just 50 outcomes.** 50 resolved predictions make a family `measurable` (new field). It is `calibrated` only if Spiegelhalter's Z test does not reject calibration at alpha = 0.05 and the Brier skill over the base-rate predictor is positive. Choice and score families use the top-label probability and correctness for both checks. New per-family fields: `z`, `p_value` (normal approximation), `brier_skill`. Previously 50 predictions at p = 0.95 that were all wrong were reported `calibrated: true`; they are now `measurable: true, calibrated: false`. Formulas are in `docs/DECISION_PORT.md`.

### Truth Gate
- **`gate-explain` parses stdin like the hook.** Without `--batch` it used a strict JSON parse, so a payload with a lone surrogate escape was blocked by `hook` but reported `block: false` by `gate-explain`. Both now share `explain_stop_event`. When the hook would allow without judging (`stop_hook_active`, no message, not an object), `gate-explain` reports that in a new `skipped` field instead of judging anyway.
- **Python fallback parity for malformed payloads.** `truth_gate.py` now mirrors the Rust three-stage payload parse (strict, with serde_json's strictness on NaN, out-of-range numbers, and lone surrogates; then surrogate-sanitized; then field extraction). A payload with trailing data after the object was blocked by Rust and allowed by Python; both now block. Ten malformed payloads are shared golden cases (`payload_cases` in `hooks/truth_gate_cases.json`).
- The field-extraction fallback now honours `"stop_hook_active": "true"` and `"1"` (the old pattern required a word character after the closing quote, so with trailing data the loop guard was lost and the hook could block again). Surrogate sanitizing keeps valid escaped surrogate pairs and replaces only unpaired escapes (previously the high half of every pair was replaced).

## 3.1.0 - 2026-09-21

### Typed decision port and Jev
- Added the **typed decision port** (`decision_port.rs`): `choice`, `score`, and `noul` questions, a `DecisionEngine` trait, `NoEngine`, `StaticEngine`, and kernel-side admission that rejects a whole response on any violation, including a choice that is not the most probable option of its own distribution, non-canonical score keys, and a score that disagrees with its distribution. The probability-sum tolerance scales with the number of options to allow two-decimal rounding. Unanswered questions become explicit abstentions; engine failures become all-abstain decisions. Every outbound string (state, instructions, options, levels) is checked for credentials.
- Added an opt-in **TypeSafe Jev adapter** (`jev.rs`, default `jev` cargo feature) with retries inside a time budget, the platform certificate verifier, a redacted key, and malformed-answer rejection. Tests replay a captured `jev-1.13.0` response; a live test is available behind `--ignored`.
- Added **prediction and outcome traces** and `hikmah calibration` (Brier, base-rate Brier, ECE, observed rate per engine and family). Model-authored traces cannot be verified, supersede, or resolve predictions. Outcomes must be a value from the prediction's answer space; purged or superseded outcomes do not count; predictions without any reported probability are recorded as unknown and counted separately, never given an invented value.
- Added CLI commands `ask`, `outcome`, `calibration`, `fulfill`, and `purge`, plus `remember --deadline/--authority/--locator`, `recall --kind`, `verify-ledger --expect-head`, and `decide --engine` (engine estimates count toward ranking, never toward evidence coverage).
- The Truth Gate can use a decision engine (`HIKMAH_HOOK_ENGINE=jev`) and falls back to the deterministic rules on any engine problem. `hikmah gate-explain [--batch]` prints the rules verdict, engine probability, threshold, and deciding path through the same code path as the hook, so the gate can be benchmarked and its threshold tuned.
- **Truth Gate engine mode v2** (measured in harness-bench):
  - **New question.** The engine now estimates whether the completion claim would fail a test run of the change, instead of whether the message admits unfinished work.
  - **Rules as a hard floor.** The rules block on their own, and the engine can only add blocks.
  - **New default threshold: 0.6** (was 0.8), chosen on a dev set.
  - **Evidence.** On 600 held-out real agent messages, the old question caught 8.5% of false completions and the new one 16.3%, at a false-block rate under 10%. At the old 0.8 default, the gate caught 1 of 295.
  - **Calibration.** The calibration family is now `truth_gate.false_completion.v2`.
  - **Confirmed on fresh data** (harness-bench run 2: 900 new messages, pre-registered, thresholds fixed). v2 caught 18.2% of false completions against 11.0% for v1, at a 7.2% false-block rate.

### Ledger (fixes)
- A `--supersedes` pointing at a missing trace no longer bricks the store: every event is validated before anything is written, and replay reports unapplicable legacy events as warnings instead of refusing to open.
- Concurrent writers no longer fork the chain: appends take an exclusive lock on a `<store>.lock` sidecar and re-read records other processes appended. The lock is never taken on the ledger itself, because Windows locks are mandatory and would block lock-free readers. The ledger is opened read+write, not append-only, so torn-tail repair can truncate on Windows too.
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
- `MemoryStore::focus` returns a Focus Capsule bounded by the policy's `working_set_limit` (previously unused). `absorb` merges further recall results without duplicates, and pinned traces are never evicted.
- The planner has a state budget (`max_states`, default 100,000) and a depth cap of 64.

### Hooks and packaging (fixes)
- The Truth Gate matches whole words only (`incomplete`, `already`, `Mastodon` no longer trigger it), ignores negated words and code spans, and normalizes text identically in Rust and Python (NFC, apostrophes, zero-width characters, whitespace, ASCII word boundaries). The two agreed on all 4,026 generated messages in a parity fuzz, and share golden cases in CI. Negation look-back is bounded, so long text stays linear. Payloads with lone surrogate escapes or out-of-range numbers no longer disable the Rust gate. The hook caps the Jev budget at 3 s whatever `HIKMAH_JEV_TIMEOUT_MS` says.
- `truth_gate.sh` never compiles code, checks each candidate's output, and always exits 0 with JSON. Hook commands quote the plugin root. The prompt hook allows when `stop_hook_active` is set or the request is unclear.
- `hikmah validate` now checks versions and names across all manifests, hook script paths, and skill frontmatter (`name` matches the directory, `description` present).
- `truth_gate.sh` also tries `bin/hikmah.exe` and `python` (Windows, where `python3` is often a Store stub). CI runs the kernel tests, validator, and hook launcher on Windows too, and its actions are pinned to commit SHAs.
- `hikmah validate` rejects relative Markdown links in a skill that do not resolve inside that skill's directory, because skills are installed one directory at a time. `operator-core` and an `agent-radar` reference no longer depend on repository-only docs.
- A `.gitattributes` keeps LF endings for shell hooks and byte-exact ledger fixtures on Windows checkouts.
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
