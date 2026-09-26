#!/usr/bin/env python3
"""Pinned public retrieval datasets, actual Hikmah MemoryStore, and an explicit BM25 baseline."""
import argparse
import csv
import datetime as dt
import hashlib
import io
import json
import math
import os
from pathlib import Path
import platform
import random
import signal
import statistics
import subprocess
import sys
import threading
import time
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/release/examples/retrieval_bench"
DATASETS = {
    "scifact": {
        "sha256": "536e14446a0ba56ed1398ab1055f39fe852686ecad24a6306c80c490fa8e0165",
        "md5": "5f7d1de60b170fc8027bb7898e2efca1", "documents": 5183, "queries": 300, "qrels": 339,
        "labels": "Expert-annotated scientific claim evidence; BEIR test split.",
        "terms": "Claims/annotations CC BY 4.0; abstracts ODC-By 1.0.",
        "source": "https://github.com/allenai/scifact",
    },
    "nfcorpus": {
        "sha256": "efe5be03f8c5b86a5870102d0599d227c8c6e2484328e68c6522560385671b0b",
        "md5": "a89dba18a62ef92f7d323ec890a0d38d", "documents": 3633, "queries": 323, "qrels": 12334,
        "labels": "Automatically derived direct/indirect link and tag relevance, grades 1/2; BEIR subset.",
        "terms": "Primary source grants free academic use; consult source for other uses.",
        "source": "https://www.cl.uni-heidelberg.de/statnlpgroup/nfcorpus/",
    },
    "fiqa": {
        "sha256": "32c7df99ed21252fdfb2cf3f5673502a8d245ee0c44c4a133570d92ce2b3ad02",
        "md5": "17918ed23cd04fb15047f73e6c3bd9d9", "documents": 57638, "queries": 648, "qrels": 1706,
        "labels": "Published financial question-answer relevance judgments; BEIR test split.",
        "terms": "BEIR dataset card specifies CC BY-SA 4.0.",
        "source": "https://huggingface.co/datasets/BeIR/fiqa",
    },
}
LIMITS = [
    "Public document retrieval with original relevance labels; not a real episodic-memory or agent-task benchmark.",
    "NFCorpus labels derive from links/tags; these datasets do not share one human-labeling protocol.",
    "Unjudged documents count as nonrelevant under the published qrels; incomplete judgments can penalize useful results.",
    "Hikmah metadata are held constant and unverified; recall_limit is 10 rather than the default 8. Other policy defaults are retained.",
    "BM25 uses its own documented tokenizer and an index; query latency excludes indexing and ingestion for both systems.",
    "No test-label tuning, LLM, embeddings, API calls, custom tags, query rewriting, or relevance labels enter retrieval.",
    "Corrected/stale-memory checks are separately generated structured histories, not real-world correction accuracy.",
    "Source/locator retention checks preserve caller assertions; they do not authenticate the original claims.",
    "One sequential local run per dataset; warm OS caches, no latency confidence interval or cross-machine speed claim.",
    "Metrics macro-average queries within each dataset; datasets are not pooled into one accuracy score.",
]


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False) + "\n", encoding="utf-8")


def write_jsonl(path, rows):
    with path.open("w", encoding="utf-8") as stream:
        for row in rows:
            stream.write(json.dumps(row, ensure_ascii=False, allow_nan=False) + "\n")


def download(name, cache):
    cache.mkdir(parents=True, exist_ok=True)
    path = cache / f"{name}.zip"
    url = f"https://public.ukp.informatik.tu-darmstadt.de/thakur/BEIR/datasets/{name}.zip"
    if not path.exists():
        temporary = path.with_suffix(".zip.part")
        try:
            with urllib.request.urlopen(url, timeout=60) as response, temporary.open("wb") as out:
                while chunk := response.read(1024 * 1024):
                    out.write(chunk)
            if digest(temporary) != DATASETS[name]["sha256"]:
                raise ValueError(f"archive SHA256 mismatch: {name}")
            temporary.replace(path)
        finally:
            temporary.unlink(missing_ok=True)
    if digest(path) != DATASETS[name]["sha256"]:
        raise ValueError(f"archive SHA256 mismatch (refusing cached file): {path}")
    return path, url


def indexed_jsonl(payload):
    result = {}
    for line in payload.decode("utf-8").splitlines():
        row = json.loads(line)
        identifier = row["_id"]
        if not isinstance(identifier, str) or not identifier.strip() or identifier in result:
            raise ValueError("empty/non-string/duplicate source id")
        if not isinstance(row["text"], str) or not isinstance(row.get("title", ""), str):
            raise ValueError("source text/title must be strings")
        result[identifier] = row
    return result


def parse_qrels(payload, documents, queries):
    qrels, pairs = {}, set()
    reader = csv.DictReader(io.StringIO(payload.decode("utf-8")), delimiter="\t")
    if reader.fieldnames != ["query-id", "corpus-id", "score"]:
        raise ValueError("unexpected qrels schema")
    for row in reader:
        qid, docid = row["query-id"], row["corpus-id"]
        score = int(row["score"])
        if qid not in queries or docid not in documents or (qid, docid) in pairs or score < 0:
            raise ValueError("invalid, duplicate, or dangling qrel")
        pairs.add((qid, docid))
        qrels.setdefault(qid, {})[docid] = score
    if not qrels or any(not any(score > 0 for score in labels.values()) for labels in qrels.values()):
        raise ValueError("each evaluated query needs a positive judgment")
    return qrels, len(pairs)


def prepare(name, archive, out, max_queries):
    with zipfile.ZipFile(archive) as z:
        members = {part: z.read(f"{name}/{part}") for part in
                   ("corpus.jsonl", "queries.jsonl", "qrels/test.tsv")}
    documents = indexed_jsonl(members["corpus.jsonl"])
    queries = indexed_jsonl(members["queries.jsonl"])
    qrels, rows = parse_qrels(members["qrels/test.tsv"], documents, queries)
    expected = DATASETS[name]
    if (len(documents), len(qrels), rows) != (expected["documents"], expected["queries"], expected["qrels"]):
        raise ValueError(f"source cardinality mismatch: {name}")
    # Stable, label-independent subset selection when explicitly requested. Full split is the default.
    qids = sorted(qrels, key=lambda x: (hashlib.sha256(("20260926:" + x).encode()).hexdigest(), x))
    qids = sorted(qids[:max_queries] if max_queries else qids)
    text_hashes = set()

    def corpus_rows():
        for identifier, row in sorted(documents.items()):
            content = row.get("title", "") + "\n" + row["text"]
            text_hashes.add(hashlib.sha256(content.encode()).hexdigest())
            yield {"id": identifier, "content": content}

    write_jsonl(out / "corpus.jsonl", corpus_rows())
    write_jsonl(out / "queries.jsonl", ({"id": qid, "text": queries[qid]["text"]} for qid in qids))
    selected = {qid: qrels[qid] for qid in qids}
    write_json(out / "qrels.json", selected)  # Evaluator only. Never a child-process argument.
    manifest = {**expected, "name": name, "selected_queries": len(qids), "full_test_split": len(qids) == len(qrels),
                "exact_duplicate_content_rows": len(documents) - len(text_hashes),
                "member_sha256": {part: hashlib.sha256(value).hexdigest() for part, value in members.items()},
                "prepared_sha256": {part: digest(out / part) for part in ("corpus.jsonl", "queries.jsonl", "qrels.json")}}
    return selected, set(documents), manifest


def metrics(ranked, labels, k=10):
    if len(ranked) != len(set(ranked)) or len(ranked) > k:
        raise ValueError("duplicate results or more results than cutoff")
    positive = {doc for doc, grade in labels.items() if grade > 0}
    if not positive:
        raise ValueError("no positive relevance judgments")
    result = {}
    for cutoff in (1, 5, 10):
        hits = sum(doc in positive for doc in ranked[:cutoff])
        result[f"precision@{cutoff}"] = hits / cutoff
        result[f"recall@{cutoff}"] = hits / len(positive)
        result[f"hit@{cutoff}"] = float(hits > 0)
    result["mrr@10"] = next((1 / rank for rank, doc in enumerate(ranked, 1) if doc in positive), 0.0)
    # Linear graded gain matches NIST trec_eval ndcg_cut's default gain map.
    dcg = sum(labels.get(doc, 0) / math.log2(rank + 1) for rank, doc in enumerate(ranked, 1))
    ideal = sum(grade / math.log2(rank + 1) for rank, grade in
                enumerate(sorted(labels.values(), reverse=True)[:k], 1))
    result["ndcg@10"] = dcg / ideal
    return result


def percentile(values, q):
    values = sorted(values)
    at = (len(values) - 1) * q
    low = int(at)
    return values[low] + (values[min(low + 1, len(values) - 1)] - values[low]) * (at - low)


def paired_interval(differences):
    rng = random.Random(20260926)
    means = [statistics.fmean(rng.choices(differences, k=len(differences))) for _ in range(2000)]
    return {"mean": statistics.fmean(differences), "query_bootstrap_95": [percentile(means, .025), percentile(means, .975)],
            "replicates": 2000, "seed": 20260926}


def stream_child(command, out, timeout):
    # Stream stdout to an auditable log; a watchdog covers ingestion as well as querying.
    with (out / "stderr.log").open("w") as err, (out / "raw.jsonl").open("w") as raw:
        process = subprocess.Popen(command, cwd=ROOT, text=True, encoding="utf-8", stdout=subprocess.PIPE,
                                   stderr=err, start_new_session=(os.name == "posix"))

        def stop():
            if process.poll() is None:
                try:
                    os.killpg(process.pid, signal.SIGKILL) if os.name == "posix" else process.kill()
                except ProcessLookupError:
                    pass

        watchdog = threading.Timer(timeout, stop)
        watchdog.start()
        try:
            for line in process.stdout:
                raw.write(line)
                raw.flush()
                yield json.loads(line)
            code = process.wait()
            if code:
                raise RuntimeError(f"retriever failed (exit {code}); see {out / 'stderr.log'}; timeout={timeout}s")
        finally:
            watchdog.cancel()
            stop()
            process.wait()
            process.stdout.close()


def evaluate(name, labels, document_ids, out, timeout):
    command = [str(BINARY), "retrieve", "--dataset", name, "--corpus", str(out / "corpus.jsonl"),
               "--queries", str(out / "queries.jsonl"), "--store-dir", str(out / "store"), "--k", "10"]
    loaded, finished, rows = None, None, {}
    for row in stream_child(command, out, timeout):
        if row["kind"] == "loaded" and loaded is None and not rows:
            loaded = row
            rejects = [r["id"] for r in row["rejected"]]
            if len(rejects) != len(set(rejects)) or not set(rejects) <= document_ids:
                raise ValueError("invalid rejection list")
            if row["corpus_total"] != len(document_ids) or row["admitted"] + len(rejects) != len(document_ids):
                raise ValueError("retrieval corpus mismatch")
            admitted = document_ids - set(rejects)
            print(f"{name}: loaded {len(admitted):,} documents; {len(rejects)} rejected", flush=True)
        elif row["kind"] == "query" and loaded is not None and finished is None:
            qid = row["id"]
            if qid not in labels or qid in rows:
                raise ValueError("unexpected/duplicate query output")
            row["metrics"] = {}
            for engine in ("hikmah", "bm25"):
                ranked = [hit["id"] for hit in row[engine]]
                if not set(ranked) <= admitted:
                    raise ValueError("retrieved non-admitted document")
                if any(not math.isfinite(hit["score"]) for hit in row[engine]):
                    raise ValueError("nonfinite score")
                duration = row[f"{engine}_ms"]
                if not math.isfinite(duration) or duration < 0:
                    raise ValueError("invalid latency")
                row["metrics"][engine] = metrics(ranked, labels[qid])
            if any(hit["verified"] or hit["provenance_source"] != f"beir:{name}" for hit in row["hikmah"]):
                raise ValueError("retrieved provenance invariant failed")
            rows[qid] = row
            if len(rows) % 25 == 0 or len(rows) == len(labels):
                print(f"{name}: {len(rows)}/{len(labels)} queries", flush=True)
        elif row["kind"] == "finished" and loaded is not None and finished is None:
            finished = row
        else:
            raise ValueError("unexpected retriever event")
    if finished is None or finished["queries"] != len(labels) or set(rows) != set(labels):
        raise ValueError("missing queries: refusing a partial-success report")
    if loaded["provenance_retained"] != loaded["admitted"]:
        raise ValueError("provenance retention mismatch")
    write_jsonl(out / "per-query.jsonl", (rows[qid] for qid in sorted(rows)))
    engines = {}
    for engine in ("hikmah", "bm25"):
        samples = [row["metrics"][engine] for row in rows.values()]
        durations = [row[f"{engine}_ms"] for row in rows.values()]
        engines[engine] = {"metrics": {key: statistics.fmean(s[key] for s in samples) for key in samples[0]},
                           "empty_results": sum(not row[engine] for row in rows.values()),
                           "latency_ms": {"p50": percentile(durations, .5), "p95": percentile(durations, .95)}}
    paired = {key: paired_interval([row["metrics"]["hikmah"][key] - row["metrics"]["bm25"][key]
                                    for row in rows.values()]) for key in ("ndcg@10", "recall@10")}
    return {"queries": len(rows), "ingestion": loaded, "engines": engines, "hikmah_minus_bm25": paired,
            "raw_sha256": digest(out / "raw.jsonl"), "per_query_sha256": digest(out / "per-query.jsonl")}


def report(summary):
    lines = ["# Labeled retrieval benchmark", "", f"Status: **{summary['status']}**. Source: `{summary['source']['commit']}`.",
             "", "All retrieval metrics are fractions, macro-averaged over the indicated test queries. Higher is better.", "",
             "| Dataset | Corpus | Queries | Engine | NDCG@10 | Recall@10 | P@10 | Hit@10 | MRR@10 | p50 ms | p95 ms |",
             "| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for name, data in summary["datasets"].items():
        for engine, measured in data["engines"].items():
            m, t = measured["metrics"], measured["latency_ms"]
            lines.append(f"| {name} | {data['ingestion']['admitted']} | {data['queries']} | {engine} | "
                         f"{m['ndcg@10']:.4f} | {m['recall@10']:.4f} | {m['precision@10']:.4f} | "
                         f"{m['hit@10']:.4f} | {m['mrr@10']:.4f} | {t['p50']:.2f} | {t['p95']:.2f} |")
    if "corrections" in summary:
        lines.extend(["", "## Separate synthetic correction checks", "", "```json", json.dumps(summary["corrections"], indent=2), "```"])
    lines.extend(["", "## Scope and limits", ""] + [f"- {line}" for line in LIMITS])
    lines.extend(["", "Archive hashes, source/binary hashes, grades, query counts, full metrics, ingestion and paired bootstrap intervals are in `summary.json`.",
                  "Query bootstrap intervals describe this query sample, not certainty about other domains. Per-query IDs, ranks and metrics are saved without corpus text in each dataset's `per-query.jsonl`."])
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--datasets", nargs="+", choices=DATASETS, default=list(DATASETS))
    parser.add_argument("--max-queries", type=int, default=0, help="0: full test split; otherwise deterministic subset per dataset")
    parser.add_argument("--query-limits", nargs="*", default=[], metavar="DATASET=N", help="override max-queries for named datasets; 0 means full")
    parser.add_argument("--cache", type=Path, default=ROOT / ".benchmark-data/retrieval")
    parser.add_argument("--out", type=Path)
    parser.add_argument("--timeout", type=int, default=7200, help="maximum seconds per dataset, including ingestion")
    args = parser.parse_args()
    if args.max_queries < 0 or args.timeout <= 0 or len(args.datasets) != len(set(args.datasets)):
        parser.error("require nonnegative max-queries, positive timeout, and unique datasets")
    query_limits = {}
    for item in args.query_limits:
        try:
            name, number = item.split("=", 1)
            limit = int(number)
            if name not in args.datasets or name in query_limits or limit < 0:
                raise ValueError()
            query_limits[name] = limit
        except ValueError:
            parser.error("query-limits requires unique selected DATASET=nonnegative_integer entries")
    started = dt.datetime.now(dt.timezone.utc)
    start = time.monotonic()
    out = (args.out or ROOT / ".benchmark-results" / started.strftime("retrieval-%Y%m%dT%H%M%S.%fZ")).resolve()
    out.mkdir(parents=True, exist_ok=False)

    def git(*arguments):
        return subprocess.check_output(["git", *arguments], cwd=ROOT, text=True).strip()

    sources = ["bench/retrieval_bench.py", "runtime/hikmah-kernel/examples/retrieval_bench.rs", "Cargo.lock"]
    summary = {"status": "running", "started_utc": started.isoformat(), "kind": "public_labeled_document_retrieval",
               "platform": {"system": platform.platform(), "cpu_count": os.cpu_count(), "python": platform.python_version()},
               "source": {"commit": git("rev-parse", "HEAD"), "tree": git("rev-parse", "HEAD^{tree}"),
                          "working_tree_status": git("status", "--porcelain"),
                          "files_sha256": {path: digest(ROOT / path) for path in sources}},
               "arguments": {"datasets": args.datasets, "max_queries": args.max_queries, "query_limits": query_limits, "timeout": args.timeout},
               "datasets": {}, "limits": LIMITS}

    def save():
        summary["elapsed_seconds"] = time.monotonic() - start
        write_json(out / "summary.json", summary)
        (out / "report.md").write_text(report(summary), encoding="utf-8")

    save()
    try:
        with (out / "build.log").open("w") as log:
            subprocess.run(["cargo", "build", "--locked", "--release", "--no-default-features", "-p", "hikmah-kernel",
                            "--example", "retrieval_bench"], cwd=ROOT, stdout=log, stderr=log, check=True, timeout=args.timeout)
        summary["source"]["binary_sha256"] = digest(BINARY)
        summary["platform"]["rustc"] = subprocess.check_output(["rustc", "--version"], text=True).strip()
        for name in args.datasets:
            stage = out / name
            stage.mkdir()
            archive, url = download(name, args.cache.resolve())
            labels, ids, manifest = prepare(name, archive, stage, query_limits.get(name, args.max_queries))
            manifest["download_url"] = url
            write_json(stage / "manifest.json", manifest)
            data = evaluate(name, labels, ids, stage, args.timeout)
            data["manifest"] = manifest
            summary["datasets"][name] = data
            save()
        stage = out / "corrections"
        stage.mkdir()
        events = list(stream_child([str(BINARY), "corrections", "--store-dir", str(stage / "store"), "--cases", "100"], stage, args.timeout))
        if len(events) != 1 or events[0].get("kind") != "synthetic_corrections" or events[0].get("ok") is not True:
            raise ValueError("correction invariant checks failed")
        summary["corrections"] = events[0]
        summary["status"] = "completed"
    except BaseException as error:
        summary["status"] = "failed"
        summary["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        save()
        print(f"Report: {out / 'report.md'}", flush=True)


if __name__ == "__main__":
    main()
