# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`AGENTS.md` holds the binding architectural rules for this repo (doctrine vs. kernel vs. adapters, no hidden chain-of-thought persistence, no fashionable infrastructure without a named failure and metric). Read it; this file does not restate it.

## Commands

The Cargo workspace has one crate, `runtime/hikmah-kernel` (library `hikmah_kernel`, binary `hikmah`). CI (`.github/workflows/validate.yml`) runs these steps, and all must pass before a change is complete:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings                        # CI denies warnings
cargo clippy --workspace --all-targets --no-default-features -- -D warnings  # build without the Jev network adapter
cargo test --workspace
cargo run -p hikmah-kernel -- validate --root .
python3 hooks/test_truth_gate.py          # Python fallback against the shared golden cases
```

CI also runs the kernel tests, the validator, and the hook launcher on `windows-latest`.

Run a single test with `cargo test -p hikmah-kernel --test ledger <test_name>`. Each file in `tests/` is its own test binary, and shared helpers such as `temp_store` and the fixture helpers live in `tests/common/mod.rs`. The live Jev test is `#[ignore]` and needs `HIKMAH_LIVE_JEV=1 TYPESAFE_API_KEY=... cargo test -p hikmah-kernel --test jev -- --ignored`.

For CLI smoke tests, run `cargo run -p hikmah-kernel -- <subcommand>`. Memory commands use `KernelPolicy::default()` unless a policy JSON is given with the global `--policy <file>` or `HIKMAH_POLICY` (missing fields default, unknown fields are errors; `hikmah policy --print-defaults` prints the defaults). Policy data can tune weights and thresholds but can never set `allow_sensitive_persistence`; `KernelPolicy::from_json` refuses it. Recall weights live in `KernelPolicy.recall` (`RecallWeights`), not in constants. The `--help` output lists every subcommand, including `remember`, `recall`, `conflicts`, `ask`, `predict`, `outcome`, `calibration`, `decide`, `hook`, and `validate`. Memory commands default to `.hikmah/memory.jsonl`. Pass `--store <tmp path>` so your experiments stay out of the repo. Read-only commands refuse a store that doesn't exist, and `init` or a write creates it.

On Windows, the default `jev` feature compiles C code (the `ring` crate), so a MinGW or MSVC C toolchain is needed. `--no-default-features` removes all network code.

## Architecture

There are three layers, and each change should stay within its layer:

1. **Portable doctrine** is Markdown only: `skills/*/SKILL.md`, `playbooks/`, and `lenses/`. Skills are installed one directory at a time, so a skill must not link outside its own folder. `validate` enforces this for relative Markdown links.
2. **Deterministic kernel** is `runtime/hikmah-kernel/src/`. It holds all executable behavior. Model or engine output reaches it only through typed ports.
3. **Thin host adapters** are `.claude-plugin/`, `.codex-plugin/`, `.agents/plugins/`, `kimi.plugin.json`, `agents/hikmah-orchestrator.md`, and `hooks/`.

### Memory: ledger → recall

- `trace.rs` defines `Trace`, which is the unit of memory. A trace is immutable. It carries a kind, provenance (source, authority, locator, and `verified`), confidence, salience, `PrivacyClass`, an optional deadline, an optional claim, and an optional `supersedes` link. Sources starting with `model:` are model-authored. Such traces can never be verified, cannot supersede, stay out of default recall, and are not consolidation evidence. A `prediction` trace is either an engine answer (`model:` source) or a forecast a person or agent recorded with `hikmah predict` (any other source). No prediction can be verified, whatever its source, and its record's `engine` must equal its source without the `model:` prefix.
- `ledger.rs` defines `MemoryStore`, which is an append-only JSONL file with a hash chain.
  - **Validate first:** `validate_batch` checks every event before anything touches disk. A bad event must never reach the file, because replay would then fail. `remember_many` writes several traces in one such batch (all or none); each is checked against the stored state only, not against the others in the batch.
  - **Record formats:** v1 records (≤3.0.0) hash a re-serialized payload. v2 records hash the exact payload bytes stored on disk. Both must keep verifying, and `tests/fixtures/ledger_v1*.jsonl` guards this. The fixtures must stay byte-exact, and `.gitattributes` marks them `-text` for that reason.
  - **Writers:** writers take an exclusive lock on the `<store>.lock` sidecar, never on the ledger itself, because Windows locks are mandatory and would block readers. Under the lock, a writer re-reads records other processes appended, repairs a torn tail with `set_len`, and checks the `<store>.head` file so a write can't paper over a truncation. The ledger is opened read+write rather than append-only, because a Windows append handle can't truncate. Every write seeks to the end explicitly.
  - **Status changes:** `fulfill` and `purge` append status events. Purge is a tombstone, and the content stays on disk.
- `recall.rs` is relevance-gated. A trace must share a query term or tag, and only then do metadata signals (salience, confidence, recency, provenance, deadline) scale its score. `Sensitive` traces and predictions are excluded unless asked for. `focus.rs` has `MemoryStore::focus`, a `FocusCapsule` bounded by `working_set_limit` that can `absorb` several recalls.
- `consolidation.rs` only proposes promotions, with source-independence and confidence thresholds from `policy.rs`. `claims.rs` detects conflicts but does not resolve them. Recall annotates each result with `conflicts` (other active traces with the same normalized key and a different value), `supersedes`, and `superseded_by` (superseded traces are recalled only with `include_superseded`), and `hikmah conflicts` lists open conflicts. All of it is derived from current state, never stored.

### Decisions and engines

- `decision_port.rs` is the typed decision port. It supports `choice`, `score`, and `noul` questions and defines a `DecisionEngine` trait with `NoEngine` and `StaticEngine`. The engine proposes and `admit()` decides, all or nothing: any violation turns *every* answer into an explicit abstain. `AdmittedDecision` is `#[non_exhaustive]` so nothing outside the crate can forge one. `EngineDescriptor::identity()` (trimmed `name@version`) is the single source of both a recorded answer's `model:` source and its record's `engine`. `Forecast::into_trace` builds a person's or agent's prediction; it requires a `<kind>:<name>` source and refuses `model:` ones. `secrets.rs` scans every outbound string before any engine sees it, and `Trace::validate` runs the same scan over a trace's content, tags, claim, source, and locator, so memory refuses credentials.
- `jev.rs` is behind the default `jev` cargo feature. It is the TypeSafe Jev HTTP adapter, configured with `TYPESAFE_API_KEY`, `TYPESAFE_BASE_URL`, and `HIKMAH_JEV_*`. Its tests replay captured responses.
- `calibration.rs` computes Brier score and ECE from the `prediction` traces and the linked `outcome` traces. Rows are keyed by family, answer kind, `forecaster_kind` (`engine` for a `model:` source, `principal` otherwise, taken from the trace's source), and the record's `engine` name, so a person or agent never shares an engine's row even under its name. A row is `measurable` at `KernelPolicy.calibration_min_outcomes` outcomes (default 50) and labelled `evidence: anecdotal` below it. It is `calibrated` only with at least `MIN_OUTCOMES` (50) outcomes whatever the policy, when Spiegelhalter's Z test does not reject (`|z| < 1.96`), and when Brier skill over the base rate is positive (top-label view for choice/score). `calibration_report(FamilyFilter::Prefix(..))` adds one pooled top-label row per forecaster.
- `decision.rs` ranks multi-criteria options. A missing criterion counts as unknown: each option gets `score_interval = [lo, hi]`, with unscored criteria at 0 and at 1. Admissible options rank by `lo`, then `hi`, and `decisive` means the winner's evidence `lo` is strictly above every rival's evidence `hi` (`evidence_interval` treats model estimates as unknown). Engine estimates are point values in `score_interval`, so they can reorder options, but they never raise evidence coverage or make a result decisive. Hard blocks always win. `estimate_missing_criteria` asks the engine for missing criteria in as few requests as `MAX_QUESTIONS` and `MAX_STATE_CHARS` allow, never for a hard-blocked option; admission is per request, so one bad answer leaves that whole request unscored. `council.rs` runs lanes sequentially, and a single risk or human-impact item can veto. `planner.rs` is bounded by `max_states` and a depth cap.
- `model_port.rs` (`ProposalEngine` / `NoModel`) is the older free-text proposal boundary.

### Truth Gate

`hook.rs` is the Stop hook. It is a narrow completion-claim check with whole-word matching, negation handling, and code spans skipped. `HIKMAH_HOOK_ENGINE=jev` adds an engine screen:

- **Question and threshold:** the engine estimates whether the completion claim would fail a test run of the change, and it blocks at `p >= 0.6` by default. Both the question wording and the threshold were measured in harness-bench (see `docs/EVIDENCE.md`). Changing either invalidates that evidence.
- **The rules are a hard floor.** A rules block always stands; the engine can only add blocks.
- **Limits:** the engine has a hard 3 s cap. Any engine problem leaves the rules' verdict.
- **Measuring on your own traffic:** `HIKMAH_HOOK_RECORD=<store>` makes the hook append each engine answer as a `prediction` trace after the verdict is written. Recording is best effort and must never change the verdict or exit code. `hikmah gate-threshold` pairs those predictions with `hikmah outcome` records and reports the highest-recall threshold within a false-block budget, with a Wilson interval. It counts only engine (`model:`) predictions and refuses below 50 resolved ones (a fixed `MIN_OUTCOMES`, not the policy field).
- **Explaining a decision:** `hikmah gate-explain [--batch]` prints the rules verdict, the engine p, and which path decided, through the same `evaluate_message()` the hook uses; without `--batch` it also reads stdin through the hook's own lenient payload parser (`explain_stop_event`). `hooks/truth_gate.py` is the zero-install fallback. **It must stay behaviorally identical to `hook.rs`**, including the three-stage payload parse (strict, surrogate-sanitized, field extraction). Both are tested against `hooks/truth_gate_cases.json` (`cases` for the rules, `payload_cases` for malformed stdin), so any rule or parsing change goes into both files and the shared cases. `hooks/truth_gate.sh` never compiles anything. It tries `bin/hikmah[.exe]`, then `hikmah` on PATH, then `python3`/`python`. It checks each candidate's output, and it always exits 0 with JSON.

### Validator and versions

`validate.rs` backs `hikmah validate`. It checks that required files exist, all manifest JSON parses, the version matches across `Cargo.toml`, the three plugin manifests, and the Claude marketplace, and the plugin name matches everywhere. It also checks that hook script paths exist, that golden cases are present, and that each skill has frontmatter whose `name` equals its directory plus a `description`, and self-contained links. The `metadata.version` in each `skills/*/SKILL.md` and the `CHANGELOG.md` heading are **not** checked, so bump those by hand.

## Repo conventions

- The README and `docs/` use deliberately scoped claims, with "Implemented" / "not claimed" tables. Do not add capability claims the code does not implement. Test counts and benchmark numbers in docs should match what you actually ran.
- Kernel changes include a test for the invariant they alter (`CONTRIBUTING.md`). Empirical claims in docs need a primary source recorded with its date and limitation (`docs/RESEARCH.md`, `docs/EVIDENCE.md`).
- `docs/DECISION_PORT.md` is the design reference for the decision port, the Jev adapter, and calibration.
