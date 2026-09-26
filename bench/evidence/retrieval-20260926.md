# Labeled retrieval benchmark

Status: **completed**. Published source: [`d21f5f0`](https://github.com/CodeWithJuber/hikmah-stack/commit/d21f5f0b420da0691f96f82abf5cd13055df60cc).

Measured in the **local Codex workspace**, not on `172.104.164.165`.
Executed clean source tree: `d940c27d0654a208797bdef4104f949b09e476a2`. Local measured commit
`172fb21d756605bc96488e73d7a1a58f497b6762` and the published source commit have identical trees.
Command: `bash bench/run_retrieval.sh --query-limits fiqa=100`.
Elapsed: **669.9 seconds**, including build check, preparation, retrieval and correction checks.

SciFact and NFCorpus use their complete test splits. FiQA uses **100 of 648** fixed,
hash-selected queries against its full source corpus. Total: **723 queries** and
**66,454 source documents**, of which **66,415** were admitted. This is not a full
FiQA test-split evaluation. Source archives contain real public documents; the
correction histories remain synthetic. No Jev/API/model requests were made.

All retrieval metrics are fractions, macro-averaged over the indicated test queries. Higher is better.

| Dataset | Corpus | Queries | Engine | NDCG@10 | Recall@10 | P@10 | Hit@10 | MRR@10 | p50 ms | p95 ms |
| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| scifact | 5183 | 300 | hikmah | 0.5358 | 0.6494 | 0.0713 | 0.6733 | 0.5093 | 555.13 | 867.38 |
| scifact | 5183 | 300 | bm25 | 0.6617 | 0.7909 | 0.0873 | 0.8100 | 0.6276 | 0.72 | 0.92 |
| nfcorpus | 3633 | 323 | hikmah | 0.2380 | 0.1205 | 0.1666 | 0.6161 | 0.4193 | 165.76 | 421.07 |
| nfcorpus | 3633 | 323 | bm25 | 0.3069 | 0.1491 | 0.2167 | 0.6873 | 0.5151 | 0.13 | 0.67 |
| fiqa | 57599 | 100 | hikmah | 0.1084 | 0.1567 | 0.0330 | 0.2700 | 0.1314 | 3579.11 | 8159.13 |
| fiqa | 57599 | 100 | bm25 | 0.2271 | 0.2920 | 0.0650 | 0.4700 | 0.2705 | 10.60 | 12.80 |

## Separate synthetic correction checks

```json
{
  "cases": 100,
  "kind": "synthetic_corrections",
  "latest_hits": 100,
  "latest_hits_after_restart": 100,
  "limits": [
    "Generated structured histories; not extracted real incident corrections.",
    "Subject variants of one generated template, not independent real incidents or a proof across all histories.",
    "A query matching only retired terms abstains; no semantic alias expansion is implied."
  ],
  "model_overwrite_rejections": 100,
  "ok": true,
  "old_only_query_results": 0,
  "provenance_retained": 300,
  "stale_hits": 0,
  "supersession_link_hits": 100
}
```

## Scope and limits

- Public document retrieval with original relevance labels; not a real episodic-memory or agent-task benchmark.
- NFCorpus labels derive from links/tags; these datasets do not share one human-labeling protocol.
- Unjudged documents count as nonrelevant under the published qrels; incomplete judgments can penalize useful results.
- Hikmah metadata are held constant and unverified; recall_limit is 10 rather than the default 8. Other policy defaults are retained.
- BM25 uses its own documented tokenizer and an index; query latency excludes indexing and ingestion for both systems.
- No test-label tuning, LLM, embeddings, API calls, custom tags, query rewriting, or relevance labels enter retrieval.
- Corrected/stale-memory checks are separately generated structured histories, not real-world correction accuracy.
- Source/locator retention checks preserve caller assertions; they do not authenticate the original claims.
- One sequential local run per dataset; warm OS caches, no latency confidence interval or cross-machine speed claim.
- Metrics macro-average queries within each dataset; datasets are not pooled into one accuracy score.

Archive hashes, source/binary hashes, grades, query counts, full metrics, ingestion and paired bootstrap intervals are in `retrieval-20260926.json`.
Query bootstrap intervals describe this query sample, not certainty about other domains. Per-query IDs, ranks and metrics are saved without corpus text in each dataset's `per-query.jsonl`.

## Interpretation

Hikmah ranks below this BM25 baseline on all three measured datasets/samples. This identifies a document-retrieval ranking gap; it does not invalidate or establish decision safety, planning or agent-task success. No kernel ranking behavior was changed or tuned using these test labels.

| Dataset | NDCG difference, Hikmah − BM25 | Query-bootstrap 95% interval | Hikmah wins / BM25 wins / ties |
| --- | ---: | --- | --- |
| scifact | -0.1259 | [-0.1625, -0.0905] | 35 / 102 / 163 |
| nfcorpus | -0.0689 | [-0.0864, -0.0519] | 49 / 140 / 134 |
| fiqa | -0.1187 | [-0.1737, -0.0668] | 10 / 40 / 50 |

The 39 FiQA rejections comprise 38 empty documents and one credential-pattern refusal. Both engines use the same admitted documents, while the original qrel denominators are retained. These fields are retained after replay for every admitted document: source, locator, observation timestamp and unverified status. This is metadata preservation, not source authentication.

## Verification and reproducibility

160 Rust tests pass (2 explicitly ignored); 18 Python evaluator tests pass, including 10 new retrieval/bridge checks. Formatting, Clippy in both feature configurations, package validation and Python Truth Gate checks pass. See JSON for validation provenance and log hashes. The release-only long-text timing gate was not run as part of this retrieval evaluation.

The compact ranked IDs below reproduce every quality metric using the pinned source qrels; no original corpus/query text is republished.

- [scifact ranked IDs](retrieval-20260926-scifact.tsv) (300 queries)
- [nfcorpus ranked IDs](retrieval-20260926-nfcorpus.tsv) (323 queries)
- [fiqa ranked IDs](retrieval-20260926-fiqa.tsv) (100 queries)
- [Full measurement JSON](retrieval-20260926.json)
- [Protocol and full server command](../LABELED_RETRIEVAL.md)
