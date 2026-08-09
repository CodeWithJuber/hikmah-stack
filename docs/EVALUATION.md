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
