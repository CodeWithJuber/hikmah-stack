# Public labeled document retrieval

This evaluates the actual `MemoryStore::recall` implementation against published
relevance judgments. It measures retrieval quality on real public documents and
natural queries. It does **not** measure all Hikmah capabilities or convert a
document dataset into a real episodic-memory corpus.

## Run without an API key

Requires Rust/Cargo, Python 3.9+ and network access for the initial dataset downloads.
The release example is built with `--no-default-features --locked`; no model calls
or API credentials are used. Run from the repository root:

```bash
# All 1,271 test queries against the complete corpus of each dataset.
bash bench/run_retrieval.sh

# Short smoke run: five selected queries, still the full SciFact corpus.
bash bench/run_retrieval.sh --datasets scifact --max-queries 5

# One full dataset, or an explicit subset of its test queries.
bash bench/run_retrieval.sh --datasets fiqa --timeout 14400
bash bench/run_retrieval.sh --datasets fiqa --max-queries 100

# Full SciFact/NFCorpus, 100 fixed FiQA queries, full corpora throughout.
bash bench/run_retrieval.sh --query-limits fiqa=100
```

Full runs can take tens of minutes: Hikmah scans and tokenizes the full corpus for
each query. There is a two-hour default timeout **per dataset**, including ingestion.
Queries are sequential, without concurrency. `--out PATH` must name a new directory;
the default is `.benchmark-results/retrieval-<UTC timestamp>/`. Dataset archives are
cached in `.benchmark-data/retrieval/`. Both directories are gitignored. Do not use an
existing user memory ledger for evaluation.

Outputs: `summary.json`, `report.md`, build log, and per-dataset manifests, raw
retrieval JSONL, `per-query.jsonl` with scores/metrics, prepared corpus/query files,
and isolated ledgers. Only compact measurements and result IDs should be published;
raw source text must follow the source's terms. A failed/missing query makes the
run fail rather than becoming a silently excluded observation. Completed datasets
are checkpointed even if a later dataset fails.

## Fixed datasets and provenance

Downloads use the [official BEIR dataset distribution](https://github.com/beir-cellar/beir/wiki/Datasets).
The runner pins SHA256 hashes and expected cardinalities, and records member hashes.
The original published MD5 hashes were cross-checked when selecting these archives.

| Dataset | Corpus | Test queries | Qrel rows | Label provenance and source terms |
| --- | ---: | ---: | ---: | --- |
| SciFact | 5,183 | 300 | 339 | Expert-annotated scientific claim evidence. [Primary repository](https://github.com/allenai/scifact); [license](https://github.com/allenai/scifact/blob/master/LICENSE.md): claims/annotations CC BY 4.0; abstracts ODC-By 1.0. |
| NFCorpus | 3,633 | 323 | 12,334 | Relevance grades 1/2 automatically derived from links and tags. This is the BEIR subset, not the larger original corpus. [Primary source](https://www.cl.uni-heidelberg.de/statnlpgroup/nfcorpus/) grants free academic use; consult its terms for other uses. |
| FiQA | 57,638 | 648 | 1,706 | Published question-answer relevance judgments. [BEIR dataset card](https://huggingface.co/datasets/BeIR/fiqa) specifies CC BY-SA 4.0. |

These are different kinds of relevance labels. They are not all independently
human-annotated task outcomes. No private transcripts or new human annotation is
claimed. Queries are original claims/questions, not generated exact document cues;
however, we do not claim they are all paraphrases or low-overlap semantic queries.

## Isolation and ranking protocol

- Use only `qrels/test.tsv`, with its original positive relevance grades. Dev/train
  labels are not used. Defaults are fixed before running; no tuning on test labels.
- Corpus input is exactly `{id, content}` where content is `title + "\n" + text`.
  Query input is exactly `{id, text}`. The Rust adapter rejects unknown fields.
  Relevance labels remain in the Python evaluator and never enter retrieval inputs.
- Every document is an observation with equal fixed timestamps and unchanged
  default salience/confidence/authority. Provenance is `beir:<dataset>` plus an ID
  locator, explicitly unverified. No qrels, answer IDs or domain tags are injected.
- Hikmah uses its existing lexical scoring, stemming, relevance gate and duplicate
  folding. The only policy override is `recall_limit=10` (default 8), also requested
  in each query. Query time is fixed to the document timestamp. Full policy is logged.
- The BM25 reference baseline uses lowercase Unicode alphanumeric tokens, no
  stemming or stopword removal, unique query terms, `k1=1.2`, `b=0.75`, and positive
  Robertson IDF `ln(1 + (N-df+0.5)/(df+0.5))`. Ties sort by document ID. This is a
  transparent baseline, not a claim to reproduce a published BEIR leaderboard run.
- Both engines receive the **same admitted corpus**. Rejected documents and reasons
  are reported; original qrel denominators retain rejected relevant documents.
  Empty output scores zero. Returned duplicates/unknown IDs and missing queries fail.
- Exact duplicate document rows are counted but retained as source IDs. Hikmah's
  own duplicate folding remains enabled and is exposed per result. Arbitrarily
  matching query and corpus IDs are not excluded. Corpus admission order is ID-sorted.
- Default evaluation uses every test query. `--max-queries` selects a deterministic
  hash-ordered subset using `SHA256("20260926:" + query_id)` before restoring ID order;
  it does not select by labels or model results. The full corpus is always retained.

## Metrics and timing

Report macro means per dataset for P@1/5/10, Recall@1/5/10, Hit@1/5/10, MRR@10,
and graded NDCG@10. Precision divides by the cutoff even for a short list; recall
divides by all original positive judgments for that query. Hit@10 means at least
one judged relevant result, not answer accuracy. Unjudged results are treated as
nonrelevant, so incomplete qrels can penalize useful results.

NDCG uses **linear graded gains**, matching the default gain mapping of
[NIST trec_eval](https://github.com/usnistgov/trec_eval/blob/main/m_ndcg_cut.c).
Paired Hikmah-minus-BM25 differences in NDCG@10 and Recall@10 include 2,000 seeded
query-bootstrap percentile intervals. These intervals assume exchangeable queries;
related claims/questions and incomplete labels limit generalization. No single
pooled "accuracy" is computed across datasets.

Per-query latency is wall time inside the Rust process, excluding IPC, ingestion,
replay and BM25 indexing. Ingestion/index/replay timings are recorded separately.
Hikmah executes first, then BM25, for each query. This is one run with potentially
warm filesystem caches, not a controlled hardware competition or CPU/memory profile.
BM25 is indexed and Hikmah currently scans/tokenizes the corpus; the comparison
includes that architectural difference. P50/P95 use linear quantile interpolation.
Source commit/tree, working-tree status, source/binary/archive hashes, platform,
policy and elapsed time make the measured artifact auditable.

## Separate synthetic correction checks

After document retrieval, generate 100 structured subjects with three successive
claim versions each. Check newest-version retrieval, zero stale-version activation,
visible supersession links, rejection of model-authored overwrites, replay and
provenance retention. A query matching only retired terms should abstain; semantic
redirection to the new version is not implemented or claimed. These histories are
generated from one template; 100 variants are not 100 independent real incidents.

This does not measure natural-language conflict extraction, authenticated
provenance, consolidation quality, deadline behavior, Truth Gate field rates,
planning, portable-skill effects or integrated agent performance.

## Evaluator validation

```bash
cargo build --locked --release --no-default-features -p hikmah-kernel --example retrieval_bench
python3 -m unittest discover -s bench -p test_retrieval_bench.py -v
```

Tests cover independently computed graded NDCG/BM25 scores, fixed denominators,
empty rankings, missing queries, invalid qrels, archive corruption, rejection of
label fields, existing-store refusal, real kernel retrieval, and correction/restart.
These are regression tests, reported separately from real-data retrieval metrics.
