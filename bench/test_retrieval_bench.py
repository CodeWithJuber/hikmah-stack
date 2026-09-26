"""Independent metric oracles, label isolation, and actual retrieval subprocess checks."""
import json
import math
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import retrieval_bench as bench


class EvaluatorTests(unittest.TestCase):
    def test_short_lists_keep_precision_and_recall_denominators(self):
        measured = bench.metrics(["a"], {"a": 1, "b": 1})
        self.assertEqual(measured["precision@10"], .1)
        self.assertEqual(measured["recall@10"], .5)
        self.assertEqual(measured["hit@10"], 1)
        self.assertEqual(measured["ndcg@10"], 1 / (1 + 1 / math.log2(3)))

    def test_graded_ndcg_uses_linear_trec_gain_and_rank(self):
        measured = bench.metrics(["unjudged", "low", "high"], {"high": 2, "low": 1, "absent": 0})
        expected = (1 / math.log2(3) + 2 / 2) / (2 + 1 / math.log2(3))
        self.assertAlmostEqual(measured["ndcg@10"], expected)
        self.assertEqual(measured["mrr@10"], .5)
        self.assertEqual(measured["recall@1"], 0)
        self.assertEqual(measured["recall@5"], 1)

    def test_empty_rankings_score_zero_instead_of_being_excluded(self):
        self.assertTrue(all(value == 0 for value in bench.metrics([], {"a": 1}).values()))

    def test_invalid_rankings_and_judgments_are_rejected(self):
        for ranked, labels in [(["a", "a"], {"a": 1}), ([str(i) for i in range(11)], {"a": 1}), ([], {"a": 0})]:
            with self.assertRaises(ValueError):
                bench.metrics(ranked, labels)
        for rows in ["q\ta\t1\nq\ta\t1\n", "q\tmissing\t1\n", "q\ta\t-1\n"]:
            with self.assertRaises(ValueError):
                bench.parse_qrels(("query-id\tcorpus-id\tscore\n" + rows).encode(), {"a"}, {"q"})

    def test_missing_query_output_cannot_be_success(self):
        loaded = {"kind": "loaded", "corpus_total": 1, "admitted": 1, "rejected": [], "provenance_retained": 1}
        with tempfile.TemporaryDirectory() as directory, patch.object(bench, "stream_child", return_value=iter([
            loaded, {"kind": "finished", "queries": 0},
        ])) as call:
            with self.assertRaisesRegex(ValueError, "missing queries"):
                bench.evaluate("fixture", {"q": {"a": 1}}, {"a"}, Path(directory), 20)
            command = call.call_args.args[0]
            self.assertFalse(any("qrels" in part for part in command))

    def test_archive_cache_corruption_is_not_silently_used(self):
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "scifact.zip").write_bytes(b"bad archive")
            with self.assertRaisesRegex(ValueError, "SHA256 mismatch"):
                bench.download("scifact", Path(directory))

    def test_rejected_relevant_document_remains_in_gold_denominator(self):
        self.assertEqual(bench.metrics(["admitted"], {"admitted": 1, "rejected": 1})["recall@10"], .5)


@unittest.skipUnless(bench.BINARY.exists(), "build release retrieval_bench first")
class KernelBridgeTests(unittest.TestCase):
    def test_actual_retrieval_and_independent_bm25_score(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            corpus = [{"id": "a", "content": "apple apple"}, {"id": "b", "content": "pear pear"},
                      {"id": "c", "content": "apple pear"}]
            bench.write_jsonl(out / "corpus.jsonl", corpus)
            bench.write_jsonl(out / "queries.jsonl", [{"id": "q", "text": "apple"}, {"id": "none", "text": "unmatchedzz"}])
            result = bench.evaluate("fixture", {"q": {"a": 2, "c": 1}, "none": {"b": 1}}, {"a", "b", "c"}, out, 30)
            rows = [json.loads(line) for line in (out / "per-query.jsonl").read_text().splitlines()]
            row = next(r for r in rows if r["id"] == "q")
            self.assertEqual([r["id"] for r in row["bm25"]], ["a", "c"])
            # All docs length 2: normalization=1.2, IDF=ln(1+1.5/2.5).
            self.assertAlmostEqual(row["bm25"][0]["score"], math.log(1.6) * (2 * 2.2) / 3.2)
            self.assertAlmostEqual(row["bm25"][1]["score"], math.log(1.6))
            self.assertEqual(result["ingestion"]["provenance_retained"], 3)
            self.assertEqual(result["engines"]["bm25"]["metrics"]["ndcg@10"], .5)
            self.assertEqual(result["engines"]["hikmah"]["empty_results"], 1)
            self.assertFalse(any(hit["verified"] for r in rows for hit in r["hikmah"]))

    def test_actual_adapter_rejects_label_fields_and_existing_store(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            bench.write_jsonl(out / "corpus.jsonl", [{"id": "a", "content": "apple", "relevance": 1}])
            bench.write_jsonl(out / "queries.jsonl", [{"id": "q", "text": "apple"}])
            command = [str(bench.BINARY), "retrieve", "--dataset", "fixture", "--corpus", str(out / "corpus.jsonl"),
                       "--queries", str(out / "queries.jsonl"), "--store-dir", str(out / "store")]
            child = subprocess.run(command, capture_output=True, text=True, timeout=30)
            self.assertNotEqual(child.returncode, 0)
            self.assertIn("unknown field", child.stderr)
            marker = out / "store" / "sentinel"
            marker.write_text("existing memory")
            child = subprocess.run(command, capture_output=True, text=True, timeout=30)
            self.assertNotEqual(child.returncode, 0)
            self.assertEqual(marker.read_text(), "existing memory")

    def test_actual_corrections_and_restart(self):
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run([str(bench.BINARY), "corrections", "--store-dir", str(Path(directory) / "store"),
                                     "--cases", "5"], capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stderr)
            data = json.loads(result.stdout)
            self.assertTrue(data["ok"])
            self.assertEqual(data["stale_hits"], 0)
            self.assertEqual(data["latest_hits_after_restart"], 5)
            self.assertEqual(data["model_overwrite_rejections"], 5)


if __name__ == "__main__":
    unittest.main()
