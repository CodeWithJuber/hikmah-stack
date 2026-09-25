# Public-data comparison: Jev, Hikmah, Jev + Hikmah

This suite evaluates labeled text decisions on four public datasets. It makes
one remote Jev call per admitted example, then replays the **identical response**
through the actual Rust kernel. It does not train or tune anything on these labels.

## Dataset coverage

| Dataset | Task | Selected data | License |
|---|---|---|---|
| BANKING77 | Fine-grained banking intent, 77 classes | Official test: 3,080 | CC BY 4.0 |
| CLINC150 | 150 intents and out-of-scope detection | Official test + oos_test: 5,500 | CC BY 3.0 |
| UCI SMS Spam | Spam / legitimate SMS | Full unsplit public corpus: 5,574 | CC BY 4.0 |
| BoolQ | Yes/no reading comprehension | Labeled development split: 3,270; no tuning on it | CC BY-SA 3.0 |

Sources and citations:

- [BANKING77](https://github.com/PolyAI-LDN/task-specific-datasets): Casanueva et al., *Efficient Intent Detection with Dual Sentence Encoders* (2020). Source commit `57ec275d8078af65b7731c2a98be812d844a6d6b`.
- [CLINC150](https://github.com/clinc/oos-eval): Larson et al., *An Evaluation Dataset for Intent Classification and Out-of-Scope Prediction* (2019). Source commit `828f8093932c8fe6ca7936c3d2e52903b1c523de`.
- [SMS Spam](https://archive.ics.uci.edu/dataset/228/sms+spam+collection): Almeida and Hidalgo (2011), DOI `10.24432/C5CC84`.
- [BoolQ](https://github.com/google-research-datasets/boolean-questions): Clark et al., *BoolQ: Exploring the Surprising Difficulty of Natural Yes/No Questions* (2019), downloaded through the [SuperGLUE v2 distribution](https://dl.fbaipublicfiles.com/glue/superglue/data/v2/BoolQ.zip). The original Google bucket returned HTTP 403 during validation.

Total: **17,424 labeled examples**. [The full offline run](https://github.com/CodeWithJuber/hikmah-stack/actions/runs/36192415007) completed on 2026-09-25 UTC with no remote API calls. NoEngine answered none, as expected; this is a control result, not semantic accuracy evidence for Jev.

Downloads and derived evaluation records retain source attribution in
`manifest.json`. Upstream data is not committed to this repository. BANKING and
CLINC use immutable Git revisions. Every downloaded file and prepared corpus gets
a SHA-256 checksum. The five exact source hashes measured in the full offline run
are pinned in the runner, including SMS and BoolQ; changed downloads are refused.
Retain the manifest when sharing a result. Duplicate texts are counted and retained,
not quietly removed.

## What each column means

- **Jev:** direct typed classification, model pinned to `jev-1.13.0`.
- **Hikmah:** the existing kernel's `NoEngine`. It honestly abstains on these
  semantic tasks. It is not a trained semantic classifier. Answer coverage and
  conditional accuracy make that limitation visible.
- **Jev + Hikmah:** paired replay through `ask(StaticEngine, request)`. This uses
  the real kernel's credential checks and admission rules, not a Python imitation.
  It isolates validation effects from stochastic model variation. Valid wrong
  labels can remain wrong; type safety is not factual correctness.

The initial no-engine check is a shared outbound policy: examples that the kernel
rejects as credential-like are not sent to Jev by either variant. They remain in
the denominator as `input_rejected`. Both variants get the same label taxonomy and
text. Correct labels are used only for evaluation and never added to the prompt.
The CLINC training file is read only for the set of category names.

Combined latency is **Jev HTTP latency plus measured Rust admission time**. It is
an estimate for the paired setup, not a second independently timed API pipeline.
NoEngine latency measures the in-process Rust check. Do not call this a model
speedup or compare it with LLM generation benchmarks.

## Server execution

From the existing isolated checkout:

```bash
git switch benchmark/jev-hikmah
git pull --ff-only origin benchmark/jev-hikmah
bash bench/run_real.sh
```

The script builds the Rust bridge, runs tests, prepares all data, then runs the
full suite. Your existing vault wrapper may populate `TYPESAFE_API_KEY` in the
environment before calling it. Otherwise it asks through a hidden terminal prompt.
No API key is accepted as a CLI argument, saved in output, or given to the Rust
replay process. The shell script does not save or delete vault entries.

Use a persistent terminal session such as `tmux` for the full run. At roughly
0.6 seconds per call, 17,000 calls take approximately 3 hours before preparation,
retry, rate-limit and other overhead. Actual runtime depends on the service.

### Resume and smaller preflight

Rerun the same command after interruption. `checkpoint.sqlite` saves raw API
responses before admission and saves completed rows; completed examples are not
called again. A crash between the remote response and its local commit can still
cause that one request to repeat. All API attempts, including retries, count toward
the cumulative 20,000 cap. Authentication failure or five consecutive service errors
stop the run. Error rows remain in the results and are not silently retried on resume.

For 20 examples per dataset in a separate output directory:

```bash
bash bench/run_real.sh --limit 20 --out .benchmark-results/real-smoke-v1
```

Sampling uses a fixed seed and IDs, independently of labels. Changing the model,
binary, script, sample size or dataset refuses resume into the same directory.
Use a new directory for changed experiments. The pinned model is not automatically
replaced if the provider retires it. Concurrent writers to one output are refused.

## Offline validation

```bash
cargo build --locked --release -p hikmah-kernel --example real_bench_port
python3 -m unittest discover -s bench -p test_real_datasets.py
python3 bench/real_datasets.py prepare
python3 bench/real_datasets.py run --out .benchmark-results/real-offline-v1
```

Unit and integration tests cover label leakage, abstention denominators, probability
handling, fault injection through the real Rust kernel, and resuming a cached model
response without another paid call. Injected unknown labels, invalid probabilities,
contradictory choices and wrong types are deliberately synthetic protocol tests;
they are not mixed into the public-data accuracy score.

## Outputs and interpretation

The default full live output is `.benchmark-results/real-live-v1/`:

- `report.md`: compact dataset-by-dataset comparison.
- `summary.json`: accuracy, Wilson intervals, macro F1, coverage, per-class metrics
  including OOS precision/recall, errors/rejections, p50/p95/p99, top-label Brier and ECE.
- `results.jsonl`: each gold/prediction pair, raw chosen answer/probabilities,
  rejection reason, model version, tokens and measured timings.
- `checkpoint.sqlite`: cached full API responses and resume state.
- `run-config.json`: data versions, hashes, binary/script fingerprints and configuration.

Send `report.md` and `summary.json` for interpretation; `results.jsonl` supports error
analysis. Do not paste vault files or credentials. No dollar cost is invented:
the report provides actual reported token usage and API attempts.

These are established public benchmarks; their presence in Jev's training data is
unknown. SMS has no official held-out split; here the whole corpus is a zero-shot
evaluation with no fitting. BoolQ uses labeled development data. Confidence
intervals assume independent examples and are optimistic when examples repeat.
Neither the raw nor admitted probabilities are declared calibrated by the kernel.
This suite assesses text decisions and the admission boundary; it does not measure
Hikmah's memory, planning, broad agent performance, or autonomous execution.
