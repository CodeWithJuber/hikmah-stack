# Typed Decision Port

The typed decision port (`runtime/hikmah-kernel/src/decision_port.rs`) connects Hikmah to engines that answer **bounded questions with probabilities** instead of prose. Unstructured state goes in; typed, validated decisions come out. The first adapter targets TypeSafe's Jev, a "System One" model. Any engine can implement the same trait.

It sits beside the text-shaped `ProposalEngine`, not in place of it. Text proposals and typed decisions are different contracts.

## Doctrine in code

| Rule | Where it is enforced |
|---|---|
| Engines propose, the kernel admits | `admit()` checks every answer against the question that was asked. `AdmittedDecision` is `#[non_exhaustive]`, so code outside the kernel crate cannot construct one. |
| No silent repair | Any violation rejects the **whole** response: an extra answer, an unknown option, a wrong type, a probability outside [0, 1], a distribution whose sum is off by more than max(0.02, 0.005 × options), a choice that is not the most probable option of its own distribution, a non-canonical score key, a score that disagrees with its distribution, an out-of-range score, a malformed answer, or a request-id mismatch. Every answer then becomes an explicit `abstain` and the reason is kept in `rejected`. |
| Unknown is a state | Unanswered questions become `abstain`. `NoEngine` abstains on everything, so callers must handle unknown. |
| Credentials never leave | `ask()` refuses a request whose state, instructions, options, or level labels match common credential shapes, before any engine is called. It uses a linear-time matcher (`secrets.rs`). |
| Confidence is earned | Engine probabilities pass through as reported, with `calibrated: false`. Calibration comes from recorded outcomes (`hikmah calibration`). |
| Model output is not memory | Recorded answers become `prediction` traces with a `model:` source. They are never verified, cannot supersede, stay out of default recall, and are not consolidation evidence. |
| Only non-model principals resolve predictions | An `outcome` trace from a `model:` source is rejected, and the observed value must belong to the prediction's answer space. Purged or superseded outcomes do not count. A prediction without any reported probability is stored with `p: null` and counted as `unscored`, never given an invented probability. |
| Hard blocks are never averaged away | Engines can estimate decision-criterion scores. `hard_blocks` stay caller- and kernel-owned, and blocked options always rank last. |

## Question types

| Kind | Declares | Admitted answer |
|---|---|---|
| `choice` | `options`: map of 2..=255 option ids to meanings | `choice`, `probabilities`, `confidence` |
| `score` | `levels`: 2..=10 ordered labels, lowest first | `score` (expected level index), `normalized` in [0, 1], nearest `level`, `probabilities`, `confidence` |
| `noul` | optional `if_true` / `if_false` meanings | `p_true` |

A request holds non-empty `state` (at most 32,000 characters) and 1..=32 questions. Question ids are `[A-Za-z0-9_-]{1,64}`. The optional `family` field names the calibration bucket and defaults to the id.

Example (`examples/decision-request.json`):

```json
{
  "state": "Canary deploy: rolls out to 5% of traffic, automatic rollback on error-rate alarms, takes 40 minutes.",
  "questions": [
    {"id": "irreversible", "instructions": "Is this plan irreversible?", "type": "noul"},
    {"id": "safety", "instructions": "How operationally safe is this plan?", "type": "score",
     "levels": ["very unsafe", "unsafe", "neutral", "safe", "very safe"], "family": "deploy.safety"},
    {"id": "review", "instructions": "Which review does this change need?", "type": "choice",
     "options": {"light": "One reviewer", "heavy": "Change advisory board"}}
  ]
}
```

## CLI

```bash
# Offline: the default engine abstains.
hikmah ask --request examples/decision-request.json

# Jev (needs TYPESAFE_API_KEY). Record answers as unverified predictions.
TYPESAFE_API_KEY=... hikmah ask --request examples/decision-request.json --engine jev --record

# Later, a person or CI job records what actually happened.
hikmah outcome --prediction tr_… --observed false --source oncall

# Calibration per engine and question family: Brier, ECE (5 bins), base rate, Spiegelhalter Z,
# Brier skill, and the measurable / calibrated verdict.
hikmah calibration

# Let an engine estimate missing criteria for options that have a description.
TYPESAFE_API_KEY=... hikmah decide --frame examples/decision-frame.json --engine jev

# Truth Gate with Jev as a second screen. The rules still block on their own; the engine can add
# a block when P(the claim would fail verification) >= threshold. Engine problems leave the rules.
HIKMAH_HOOK_ENGINE=jev TYPESAFE_API_KEY=... hikmah hook < stop-event.json

# Opt in to letting a confident engine answer (p < 0.1 here) lift a rules block. Off by default.
HIKMAH_HOOK_ENGINE=jev HIKMAH_HOOK_ENGINE_LIFT=0.1 TYPESAFE_API_KEY=... hikmah hook < stop-event.json

# Same code path, explained: rules verdict, engine probability, threshold, and which path decided.
# stdin is parsed exactly like the hook's (lone surrogates, trailing data, invalid UTF-8); when the
# hook would allow without judging (for example stop_hook_active), `skipped` says why.
# --batch reads {"id", "last_assistant_message"} JSON lines and writes one verdict per line.
HIKMAH_HOOK_ENGINE=jev TYPESAFE_API_KEY=... hikmah gate-explain < stop-event.json
hikmah gate-explain --batch < messages.jsonl

# Measure the gate on your own traffic: record each engine probability, record what happened,
# then choose the threshold with the best recall inside a false-block budget.
HIKMAH_HOOK_ENGINE=jev HIKMAH_HOOK_RECORD=.hikmah/gate.jsonl TYPESAFE_API_KEY=... hikmah hook < stop-event.json
hikmah outcome --store .hikmah/gate.jsonl --prediction tr_… --observed true --source ci   # true = it was a false completion
hikmah gate-threshold --store .hikmah/gate.jsonl --max-false-block 0.10
```

Environment:

| Variable | Default | Meaning |
|---|---|---|
| `TYPESAFE_API_KEY` | unset | Required for `--engine jev`. Never logged. |
| `TYPESAFE_BASE_URL` | `https://api.typesafe.ai` | Alternate endpoint. |
| `HIKMAH_JEV_MODEL` | `jev-latest` | Model route. |
| `HIKMAH_JEV_TIMEOUT_MS` | 5000 | Total time budget including retries, clamped to 100 ms..=60 s. The hook always caps it at 3000. |
| `HIKMAH_HOOK_ENGINE` | unset (rules) | `jev` turns on engine mode in `hikmah hook`. |
| `HIKMAH_HOOK_THRESHOLD` | 0.6 | The engine adds a block when `P(the completion claim would fail verification)` is at or above this value. Chosen and confirmed in harness-bench (see below). |
| `HIKMAH_HOOK_ENGINE_LIFT` | unset (rules are a hard floor) | Opt-in. When the engine's admitted answer has `p` below this value, a rules block is lifted. An engine block is never lifted, so a value at or above the threshold acts like the threshold. An unset, unparsable, or out-of-range value means no lift. No measurement backs any value; see [the limits](#truth-gate-engine-lift-opt-in). |
| `HIKMAH_HOOK_RECORD` | unset | A memory store path. When set and the engine answered, `hikmah hook` appends that answer there as a `prediction` trace after writing its verdict. Recording failures never change the verdict or exit code. |

## Decision frames with engine estimates

Options may carry a free-text `description`. With `--engine`, the kernel asks one score question per missing criterion. Answers are stored in `model_scores`. They count as point values in the option's `score_interval`, and toward `raw_score`, but **not** toward `coverage`. An estimated criterion therefore narrows `score_interval` and can change the ranking, but it never raises confidence. It never makes a result decisive either: `evidence_interval` treats it as unknown. The output lists every estimate, and every abstention with its reason.

Ranking uses the interval. With no engine, or where the engine abstains, a criterion stays unscored. It is then counted at the scale minimum (0) for `lo` and at the maximum (1) for `hi`:

- `lo = Σ_scored w·s / Σ w`
- `hi = (Σ_scored w·s + Σ_unscored w) / Σ w`

Admissible options rank by `lo`, then `hi`, then reversible first, then name. `decisive` is true only when the recommended option's evidence `lo` is strictly greater than every other admissible option's evidence `hi` (`evidence_interval`, where model estimates count as unknown). `raw_score` and `confidence_adjusted_score` are still reported for comparison with 3.1.0, but they no longer order the ranking.

## Calibration verdict

`hikmah calibration` groups resolved predictions by engine, family, and answer kind. For each group it reports Brier, ECE over 5 equal-width bins, the observed rate, and two tests. Each test uses pairs `(p, y)`:

- **Noul families:** `p = P(true)`, and `y = 1` when the outcome was `true`.
- **Choice and score families (top-label view):** `p` is the probability of the reported answer, and `y = 1` when the outcome equals it.

| Field | Formula | Meaning |
|---|---|---|
| `z` | `Σ (y − p)(1 − 2p) / sqrt(Σ (1 − 2p)² p (1 − p))` | Spiegelhalter's Z statistic, approximately standard normal when the probabilities are calibrated. `null` when the variance term is zero (for example every `p` in {0, 0.5, 1}). |
| `p_value` | `erfc(abs(z) / √2)` | Two-sided p-value of `z` from the normal approximation. |
| `brier_skill` | `1 − B / (r (1 − r))` | Brier skill against always predicting the observed base rate `r`. Noul: `B` is the family Brier and `r` the share of `true`. Choice/score: `B` is the binary Brier of `(p, y)` and `r` the top-label accuracy. `null` when `r` is 0 or 1, because nothing beats a constant outcome in-sample. |
| `measurable` | `n >= 50` | Enough resolved predictions to judge. |
| `calibrated` | `measurable ∧ abs(z) < 1.96 ∧ brier_skill > 0` | The Z test does not reject calibration at alpha = 0.05, and the probabilities carry information beyond the base rate. |

Source for the Z test: D. J. Spiegelhalter, "Probabilistic prediction in patient management and clinical trials", *Statistics in Medicine* 5(5):421–433, 1986.

Limits:

- The base rate is in-sample, which slightly favours the baseline, so the skill check is conservative.
- The Z test has little power on small or narrow samples.
- `calibrated: false` with `measurable: true` means the data contradict calibration, or the forecasts add nothing over the base rate.
- For choice and score families, the multiclass `brier` field is still reported, but the verdict uses only the top-label pair.

## Jev adapter

`runtime/hikmah-kernel/src/jev.rs`, behind the default `jev` cargo feature:

- It calls `POST /v1/systemone` with bearer auth.
- It uses `ureq` with the platform certificate verifier, and picks up proxy settings from the environment.
- It retries 429, 529 and 5xx inside the time budget. Any other failure is returned as an error, which `ask()` turns into an all-abstain decision.
- An answer it cannot parse (for example `noul: null`) becomes `malformed`, which rejects the whole response.
- `Debug` redacts the key. Error messages carry only the HTTP status and TypeSafe's `error_type`.
- Tests inject a transport and replay a response captured from `jev-1.13.0`. A live round trip is available as an ignored test: `HIKMAH_LIVE_JEV=1 TYPESAFE_API_KEY=... cargo test --test jev -- --ignored`.
- Build without network code: `cargo build --no-default-features`.

## Truth Gate engine mode: measured, not assumed

harness-bench (a separate repository) measures the gate on real agent finish messages whose labels come from execution: did the agent's patch actually resolve the task?

- **The rules alone** almost never fire on these messages. False completions do not say TODO; they claim success.
- **The first engine question** asked whether the message admits unfinished work. It caught 8.5% of false completions.
- **The current question** asks whether a test run would show the task is not done. On run 1's held-out split (600 messages) it caught 16.3% with a 7.9% false-block rate at the 0.60 threshold, and its probabilities were well calibrated (ECE 0.05).
- **Threshold and combination.** The threshold was chosen on a separate dev set. By default the rules stay a hard floor: the engine can add a block but never remove one. The numbers here were measured that way.

**Run 2 confirmed it on fresh data.** Run 2 used 900 new held-out messages, from tasks and row groups not used in run 1, with every threshold fixed in advance:

| Gate | False completions caught | False-block rate | AUROC |
|---|---|---|---|
| v2 as shipped | 18.2% | 7.2% | 0.675 |
| v1 question at its tuned threshold | 11.0% | 8.1% | 0.590 |
| v1 at its old 0.8 default | 0.9% | — | — |
| rules alone | 0.4% | — | — |

The v2 gate's gains over v1 are significant: +7.2 points of recall and +0.084 AUROC. Its false-block rate is not significantly different from v1's.

### Choosing the threshold from your own traffic

The 0.6 default was measured on OpenHands/Qwen3-Coder messages. Your agent's messages may differ, so the threshold can be set from your own data:

1. **Record predictions.** Run the hook with `HIKMAH_HOOK_RECORD=<store>`. Each time the engine answers, the hook appends a `prediction` trace to that store after writing its verdict:
   - family `truth_gate.false_completion.v2`, answer space `["true", "false"]`, and `p` = P(the completion claim is false);
   - the host `session_id`, when present, as the locator;
   - not the message itself.

   Rules-only verdicts, engine failures, and skipped events record nothing. A failure to record never changes the verdict or the exit code.
2. **Record outcomes.** For each recorded prediction, record what happened with `hikmah outcome --prediction <id> --observed true|false`. `true` means the claim was a false completion: the tests for the change failed.
3. **Choose.** Run `hikmah gate-threshold [--max-false-block 0.10]`. It pairs predictions with their latest active outcome. It then reports the threshold `t` (the engine blocks when `p >= t`) with the highest recall of false completions whose empirical false-block rate stays within the budget. The false-block rate is blocked true completions divided by all true completions. Ties in recall go to the higher threshold. Only observed probabilities are candidates, because the rates change only there. The report also includes:
   - `n` and the count of each class;
   - `recall`, `false_block_rate`, and a Wilson 95% interval for the false-block rate;
   - the same rates at the 0.6 default, for comparison.

It refuses with fewer than 50 resolved predictions, or when either class is absent. When no observed threshold fits the budget, `threshold` is `null`.

Limits:

- This covers the engine path only. The rules still block on their own, so the gate's overall false-block rate can be higher by the rules' own false blocks.
- The threshold is chosen on the same data it is reported on, so its false-block rate is optimistic. Check it again on outcomes recorded afterwards.
- `hikmah calibration --family truth_gate.false_completion.v2` reports calibration for the same predictions.

### Truth Gate engine lift (opt-in)

The deterministic rules match words, not meaning, so they can block an honest message ("Implemented the finder; tests are still running and I'll share the results later"). By default an engine cannot overrule them. `HIKMAH_HOOK_ENGINE_LIFT=<p>` changes that: when the engine answers and its admitted `P(the completion claim would fail verification)` is below `p`, a rules block is lifted. `hikmah gate-explain` reports `lift` and `lifted`.

- **Only an admitted answer lifts.** An engine failure, timeout, abstention, or rejected response leaves the rules block in place.
- **Engine blocks are never lifted.** `block = engine_block || (rules_block && !lifted)`.
- **Not measured.** harness-bench measured the rules as a hard floor, so no lift value has evidence behind it. In run 2 the rules alone blocked 0.4% of false completions and 0.9% of true ones, which bounds how much a lift could have changed there. Choose a value only from your own recorded outcomes (`HIKMAH_HOOK_RECORD` records every engine answer, lifted or not).
- **Not a policy-file setting.** The hook never reads the kernel policy file, so a bad policy file cannot break it. The lift is set only by this environment variable, next to the other hook settings.

## What this does not claim

- The kernel does not verify any engine's claimed accuracy or calibration. It measures calibration only from outcomes you record. 50 resolved predictions make a family `measurable`; it is marked `calibrated` only when the tests in [Calibration verdict](#calibration-verdict) also pass. Passing them means the data do not contradict calibration; it is not proof of it, and a family can drift after it passed.
- The Truth Gate engine mode is a screen, not a verifier. On real agent "done" messages it catches a minority of false completions (about one in six in harness-bench run 1, at a false-block rate under 10%), because most false completions read exactly like true ones. Execution evidence (tests actually run) is what catches the rest.
- The port does not make an engine's output durable truth. Promotion from a prediction to a belief still needs a non-model principal.
