# Pure-kernel capability benchmark

This suite calls the real Rust kernel without an LLM, an API key, or the Jev network feature.
It measures synthetic scale and invariant workloads, plus rules-only Truth Gate performance on
an explicitly supplied labeled corpus. It does not turn the previous `NoEngine` abstention
control into a semantic classifier. No new runtime architecture or dependency is introduced.

## Run

From the repository root, with Rust/Cargo and Python 3 available:

```bash
bash bench/run_kernel.sh
```

Defaults: 1,000 / 10,000 / 100,000 records; 30 timed exact-cue queries at each size;
10,000 seeded decision cases; a 30-minute timeout per stage. The runner builds the release
example with `--no-default-features`, runs offline regression tests, package validation,
Python Truth Gate tests and the explicit release timing gate, then measures the workloads.
Dependency download during compilation may need network; measurements make no network calls.

For a small CI/smoke run:

```bash
bash bench/run_kernel.sh --sizes 1000 --queries 12 \
  --gate-corpus bench/fixtures/gate-corpus-example.json
python3 -m unittest discover -s bench -p test_kernel_bench.py -v
```

Output defaults to a new `.benchmark-results/kernel-<UTC timestamp>/` directory:

- `report.md`: stage status and scale measurements;
- `summary.json`: settings, source and executable hashes, commit/dirty flag, toolchain,
  platform, stage measurements, errors and interpretation limits;
- stage stdout/stderr logs and isolated synthetic ledger directories.

`--out PATH` must name a new directory. Existing memory is never reused or overwritten.
Stages checkpoint the report on completion; failed or timed-out stages make the final exit
nonzero. POSIX timeouts kill the whole stage process group, including recovery children.
There is no resume: re-run into a new directory. A timeout leaves that stage failed, not
silently excluded from the score. Do not treat a report whose status is `running`,
`interrupted` or `failed` as a completed evaluation.

## Measurements and scope

| Workload | Independent expectation / measurement | Boundary |
|---|---|---|
| Observation memory | Fixed timestamps, explicit IDs, unique lexical cue per record; exact cue must retrieve its record | Synthetic, no semantic interpretation or structured-claim workload |
| Append | Batch durations for batches up to 1,000; final 50 records timed individually | Batch duration is not per-record latency; seeding also includes trace creation |
| Recall | p50/p95/p99, sample count, exact-cue hit count | Warm process/cache; fixed query schedule, not production traffic |
| Replay/integrity | Reopen time, verify time, counts, head equality, source/verification metadata, same recalls after reopen | Same-process warm reopen, not a cold machine restart |
| RAM | Linux `/proc/self/status` current RSS and process high-water mark in KiB | Process-wide allocator/regex overhead included; null on unsupported systems |
| Process interruption | Child commits 50 records, acknowledges, then is forcibly terminated; reopen and verify every ID | Post-commit kill, not a crash in the middle of fsync/head replacement |
| Fault injection | Separate ledger copies receive an incomplete tail, content edit, or last-record truncation | Torn tail repair and edit/truncation detection; no automatic acceptance/reset of suspicious history |
| Decisions | Six families, known answers and metamorphic checks, seed + first 20 counterexamples | Observed invariant checks, not proof over all inputs or real-world decision quality |
| Truth Gate | Confusion matrix, false-block/false-pass rates, per-task results, Wilson intervals | Rules-only screen; corpus labels must come from external observations |

Every scale runs in a fresh subprocess, so a prior scale does not contaminate its process
high-water mark. The host is not reserved or load-controlled; record hardware and compare
repeated runs on the same idle host before making a performance claim. Verification,
replay and individual append have different costs; inspect them separately.

Six decision families cover: a high-scoring hard-blocked option; every option blocked;
complete equal-weight scores checked against direct arithmetic; unknown criteria with a
model estimate that must not count as evidence; reversible alternatives inside and outside
the preference band; out-of-range numeric input that must be rejected. All valid cases also
reverse option order and change irrelevant question wording. RNG is explicit SplitMix64.
These cases do not randomly sample every possible policy, weight, Unicode string or schema.

## The timing-test change

The original offline suite had 153 tests: 22 library and 131 integration tests. The reported
server timing failure was not reproduced in the local baseline: all 153 passed. Server load
is a possible explanation, not an established diagnosis.

`long_unpunctuated_text_has_expected_verdicts` now checks all eight long-text cases in the
ordinary debug suite without a host-speed assertion. The original
`long_unpunctuated_text_stays_fast` retains its two-second **per-input** ceiling and checks
the same verdicts, but is an explicit release-only performance test. It warms regex
compilation before timing and prints per-case milliseconds and byte counts. This separates
startup cost from steady-state text processing. Run it with:

```bash
cargo test --locked --release -p hikmah-kernel --no-default-features \
  --lib long_unpunctuated_text_stays_fast -- --ignored --test-threads=1 --nocapture
```

Both `run_kernel.sh` and `kernel-bench-checks.yml` execute it. It is not silently removed from
CI or weakened to a larger threshold. Ordinary test totals now include one additional
explicitly ignored timing test; the existing live Jev test remains separately ignored.

## Labeled real transcripts

No private incident history or agent traffic is fetched automatically. Supply a JSON corpus:

```json
{
  "kind": "real_labeled",
  "source": "Dataset identifier, version, split, and labeling procedure",
  "cases": [
    {
      "id": "message-001",
      "task_id": "task-001",
      "message": "Completed the change; all acceptance tests passed.",
      "false_completion": true,
      "label_source": "Independent task acceptance result: unresolved, artifact identifier"
    }
  ]
}
```

This is a schema illustration, not a real measured example. `false_completion=true` means a
completion claim conflicts with independently established task success. Restrict the corpus
to comparable completion messages; an honest statement of inability is not itself a false
completion. Sanitize content before ingestion and retain the label's evidence outside the
message. The evaluator checks types, IDs and required annotations; it cannot authenticate
the annotator or decide whether the labels are true. Only `id` and `message` go to the gate.

```bash
bash bench/run_kernel.sh --gate-corpus /path/to/labeled-completions.json
```

The bundled six-message fixture is **synthetic_regression**, deliberately includes both
false passes and a false block, and includes identical messages with opposite outcome labels.
Its score must never be presented as real-traffic performance. The runner reports evaluation
completion independently from classification accuracy; a low score is a valid measurement.

Report duplicates and group messages by task. Split future development/test data by task or
incident, not individual messages. Any future rule/threshold tuning must use development
data only; lock the rule version before final testing. Message-level Wilson intervals assume
independent observations and are not valid task-cluster confidence intervals. A stratified
or curated case mix does not estimate deployment prevalence.

## Remaining capability work

The existing regression tests still cover provenance validation, structured conflicts,
supersession, consolidation proposals, deadlines, typed admission, calibration, policy,
principals, planning, council and focus. In particular, `tests/focus.rs` already tests both
capacity bounds and pinned-item survival; it is not a challenge-lane test. The planner is
breadth-first bounded symbolic search, not branch-and-bound optimization.

This phase does **not** measure large labeled real-memory retrieval, consolidation accuracy,
commitment-history rates, independent real calibration outcomes, portable skills on/off
behaviour, or a connected agent/tool workflow. Those require their own labeled workloads
or a controlled host integration. Existing passing regression cases must not be substituted
for those missing measurements.
