# Comparison: hikmah-stack vs agent-memory frameworks

Honest, measured comparison against the frameworks people actually evaluate
alongside this one. Every number below comes from our own runs or our own
source reading — nothing is quoted from a vendor's marketing page.

**Claim tags:** ✅ observed in our 2026-10-09 head-to-head run (500 facts,
20 known-answer queries, hit@5) · 📋 from the repo's own committed evidence
(`bench/evidence/`, not re-run that day) · 🔍 from reading the competitor's
source, not a press page.

**The one-paragraph version:** hikmah-stack and the embedding frameworks are
barely the same product. mem0 / langmem answer *"what did the user probably
mean?"* with hundreds of megabytes of machinery and an LLM bill. hikmah
answers *"what exactly was recorded, who said it, and has it been altered?"*
in ~12 MB with zero trust in any model. Pick the question first; the table
below follows from it.

## What was actually measured

| | Store 500 facts | Recall p50 | hit@5 lexical (10) | hit@5 paraphrase (10) | Peak RSS |
|---|---|---|---|---|---|
| **hikmah-stack** 3.1.0 (CLI) | 160 s (spawn-dominated) | 209 ms ✅ | 9/10 ✅ | 7/10 ✅ | **11.7 MB** ✅ |
| **langmem** 0.0.30 (store only) | 17.6 s | 76 ms ✅ | 10/10 ✅ | 10/10 ✅ | 324 MB ✅ |
| **mem0** 2.2.1 (full pipeline, stub LLM) | 183 s | 83 ms ✅ | 10/10 ✅ | 10/10 ✅ | 403 MB ✅ |
| **cognee** 1.6.3 | — | — | — | — | — |

cognee installed (653 s of dependencies) but cannot run without a real LLM —
its knowledge-graph extraction is LLM-or-nothing by construction, so a stub
run would have measured our stub, not cognee. zep / memobase were not
attempted (client→server architectures; the sandbox has no Docker). mem0's
LLM was a deterministic rule-based stub, so its *extraction quality* was not
measured — only pipeline machinery (dedup, embed, store, search).

hikmah's CLI numbers above are one process spawn per operation. The engine
underneath is far faster: in-process recall p50 **0.49 ms @ 1K records**
(📋 `bench/evidence/`). New in this release: `remember-batch` writes N
records in one process (~3.9 ms/fact for 100 records vs ~90 ms/fact
spawned one by one — measured).

hikmah's own published retrieval eval (📋 `bench/evidence/retrieval-20260926.md`,
BEIR, 723 queries / 66K docs): NDCG@10 **below a BM25 baseline on all three
sets** — SciFact 0.5358 vs 0.6617, NFCorpus 0.2380 vs 0.3069, FiQA 0.1084 vs
0.2271. We publish this because the honesty is the point; see
[Evaluation](EVALUATION.md).

## Where hikmah-stack wins (measured or source-verified)

1. **Auditability is demonstrable, not a tagline.** The only framework here
   where tamper-evidence could be *shown*: one record's bytes flipped in the
   JSONL → `integrity error at ledger sequence 251: record hash does not
   match payload` ✅. mem0 has a `hash` field (a dedup key, not a chain) and
   no integrity verification; langmem's store is in-memory only
   (*"Data is lost when the process exits"* — its own docstring 🔍).
2. **Contradictions stay visible; nothing is silently deleted.** hikmah
   detects conflicting claim values deterministically and keeps both sides
   listed (`hikmah conflicts`, and beside every recall hit) — it never picks
   a winner 🔍. mem0's policy is LLM-judged **deletion** of the contradicted
   fact 🔍; graphiti invalidates via LLM judgment 🔍; cognee resolves by
   recency 🔍; langmem leaves it to the model 🔍.
3. **An agent cannot verify its own memory.** Principal-bound verification:
   writes from a detected agent session are stamped `agent-session:<host>:<id>`
   and can never be marked verified — enforced in `Trace::validate` on every
   write path 🔍. No competitor distinguishes *who* attests a fact.
4. **Resource usage isn't close.** ~12 MB vs 300–400 MB peak RSS; no model
   downloads, no background services, no API keys, no network at runtime ✅.
5. **Determinism.** Same query → same ranking, byte-identical ledger, no
   model nondeterminism anywhere ✅.

## Where hikmah-stack loses (measured or source-verified)

1. **Semantic recall.** 7/10 on paraphrases vs 10/10, and our own BEIR eval
   trails BM25 on every dataset (above). If the agent must recall "the thing
   the user meant but didn't say," the lexical engine misses and embeddings
   don't. This is architectural, not a tuning gap.
2. **Automatic extraction.** Every competitor uses an LLM to extract memories
   from conversation; hikmah ships only `NoModel` — the caller supplies
   structured claim keys, and nothing unstructured becomes a claim on its
   own 🔍.
3. **Serving story.** Competitors offer servers/APIs/MCP/multi-user; hikmah
   is a local single-user CLI + library **by design** (explicitly disclaimed
   in the [Maturity boundary](../README.md#maturity-boundary)) — fine as a
   boundary, but zero drop-in adoption for agent frameworks.
4. **Ecosystem.** mem0 pip-installs in 2 minutes into any Python agent stack;
   hikmah needs a Rust toolchain (or the prebuilt release binaries) and
   speaks CLI/JSON. The [Python bridge](../python/README.md) narrows this,
   but the friction is real.

## Notes on specific competitors

- **mem0** phones home to PostHog by default (first stderr line of every
  run) ✅; `search()` rejects a top-level `user_id` (must use
  `filters={'user_id': ...}`) ✅; exact-duplicate dedup works (500 adds →
  428 vectors) ✅.
- **Motorhead** is the Rust precedent in this space and appears in the
  awesome lists where hikmah-stack is absent — discovery, not mechanism, is
  the gap there. Not measured here.
- **Zep/Graphiti** temporal edges are the closest prior art to hikmah's
  supersession links — but the *invalidation judgment* is LLM-made, and
  hikmah refuses to resolve at all.

## Bottom line

If you want drop-in semantic memory with automatic conversation extraction
("remember what the user said last week"), mem0, Zep/graphiti, or langmem
serve that far better — they actually do embeddings and LLM extraction. If
you want an audit trail with teeth — contradictions that can't be silently
deleted, verification that can't be self-attested, a ledger that proves it
wasn't tampered with — that is what this kernel is for. It is a complement
to those systems, not a replacement.
