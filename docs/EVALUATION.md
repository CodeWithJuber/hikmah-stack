# Evaluation Contract

There is no “perfect engine” claim in Hikmah Stack. There is a **perfectibility loop**: every observed failure should become a reproducible case, an invariant, a test, a policy, or an explicitly accepted limitation.

## Memory metrics

- Recall precision@k for known relevant traces.
- Contradiction recall rate for structured conflicting claims.
- Provenance retention rate after consolidation/replay.
- Commitment recall before/at deadline.
- Stale-belief activation rate after supersession.
- Memory-poisoning rate from unverified model-generated claims.
- Sensitive-persistence violations.

## Decision metrics

- Missing-evidence visibility.
- Hard-block compliance.
- Ranking stability under irrelevant context.
- Reversibility preference when evidence is weak and expected value is otherwise close.
- Human-impact question surfacing for consequential actions.

## Delivery metrics

- False completion rate.
- Claimed-vs-observed test pass mismatch.
- Rollback information completeness.
- Outcome write-back coverage.

## Runtime metrics

- p50/p95 recall latency by trace count.
- Memory footprint.
- Ledger replay time.
- Hash-chain verification time.
- Determinism across identical inputs.

## Architecture bake-off

Any future local model must be tested behind the same `ProposalEngine` boundary. Compare candidate architectures on task quality **and** downstream repair, verification, memory pollution, latency, energy, and operator trust. A model that writes beautiful prose but causes more incorrect durable memories loses.

## Measurable now (3.1.0)

These run without new infrastructure:

- **Truth Gate:** false-block and false-pass rate on `hooks/truth_gate_cases.json` (rules), and on your own labeled transcripts with `HIKMAH_HOOK_ENGINE=jev` at a chosen threshold. The shipped golden cases are author-written regression tests, not evidence of field accuracy.
- **Engine calibration:** `hikmah ask --record` stores predictions; `hikmah outcome` stores what happened; `hikmah calibration` reports Brier score, the Brier of a base-rate predictor, ECE over 5 equal-width bins, and the observed rate per engine and question family. A family is `measurable` after 50 resolved predictions. It is reported `calibrated` only if, in addition, Spiegelhalter's Z test does not reject calibration at alpha = 0.05 (`|Z| < 1.96`, `Z = Σ (y − p)(1 − 2p) / sqrt(Σ (1 − 2p)² p (1 − p))`) and the Brier skill against the base-rate predictor, `1 − Brier / (r (1 − r))`, is positive. Choice and score families use the top-label probability and top-label correctness for both checks. `z`, `p_value`, and `brier_skill` are reported per family (see `docs/DECISION_PORT.md`).
- **Memory:** stale-belief activation after supersession (`tests/recall.rs`: a superseded belief is not recalled by default, even for a query that matches only its own words, and its correction is), duplicate folding, relevance gating, conflicts shown beside recalled claims (`tests/conflicts.rs`), and sensitive-persistence and credential refusal (`tests/ledger.rs`) are covered by integration tests (`runtime/hikmah-kernel/tests/`). These are pass/fail regression tests, not measured rates on real data.
- **Ledger:** tamper, truncation (head file), forged chain-valid appends past the head (refused until a person runs `verify-ledger --accept-tail`, which the CLI refuses inside an agent session), torn tail, legacy format, concurrent writers, and verification during concurrent writes (no false alarm) are covered by tests; replay time can be measured with `hikmah verify-ledger` on a large store. The chain is unkeyed, so an append by someone who can also rewrite or delete the head file is not detectable without a pinned head.

Still not measurable without new data: recall precision@k on a labeled corpus, and provenance retention across consolidation.
