"""Check evaluator denominators, label isolation, failures, and real subprocess boundaries."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import kernel_bench as bench


class MetricsTests(unittest.TestCase):
    def test_all_confusion_cells_and_denominators(self):
        m = bench.confusion([True, True, True, False, False], [True, True, False, True, False])
        self.assertEqual([m[k] for k in ("tp", "fn", "fp", "tn")], [2, 1, 1, 1])
        self.assertEqual(m["recall"], 2/3)
        self.assertEqual(m["false_block_rate"], 1/2)
        self.assertEqual(m["false_pass_rate"], 1/3)
        self.assertEqual(m["f1"], 2/3)

    def test_undefined_metrics_are_not_perfect_scores(self):
        m = bench.confusion([False, False], [False, False])
        self.assertIsNone(m["recall"])
        self.assertIsNone(m["precision"])
        self.assertIsNone(m["recall_wilson95"])

    def test_missing_or_non_boolean_predictions_are_rejected(self):
        with self.assertRaises(ValueError):
            bench.confusion([True, False], [True])
        with self.assertRaises(ValueError):
            bench.confusion([True], [1])

    def test_corpus_requires_independent_labels_and_unique_ids(self):
        source = json.loads((bench.ROOT / "bench/fixtures/gate-corpus-example.json").read_text())
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "corpus.json"
            source["cases"][0]["false_completion"] = "false"
            path.write_text(json.dumps(source))
            with self.assertRaises(ValueError):
                bench.load_corpus(path)
            source["cases"][0]["false_completion"] = True
            source["cases"][0]["id"] = source["cases"][1]["id"]
            path.write_text(json.dumps(source))
            with self.assertRaises(ValueError):
                bench.load_corpus(path)

    def test_labels_never_reach_gate_and_missing_output_cannot_pass(self):
        corpus = bench.ROOT / "bench/fixtures/gate-corpus-example.json"
        with patch.object(bench, "execute", return_value=(0, "", "")) as call:
            with self.assertRaises(RuntimeError):
                bench.evaluate_corpus(corpus, bench.BINARY, 20)
            payload = call.call_args.args[2]
            self.assertTrue(all(set(json.loads(line)) == {"id", "message"} for line in payload.splitlines()))


@unittest.skipUnless(bench.BINARY.exists(), "build the release bench_kernel example first")
class KernelTests(unittest.TestCase):
    def test_actual_gate_reports_misses_and_false_blocks(self):
        result = bench.evaluate_corpus(bench.ROOT / "bench/fixtures/gate-corpus-example.json", bench.BINARY, 30)
        self.assertEqual([result["metrics"][k] for k in ("tp", "fn", "tn", "fp")], [2, 1, 2, 1])
        self.assertEqual(result["duplicate_message_rows"], 1)
        self.assertEqual(result["kind"], "synthetic_regression")

    def test_scale_refuses_existing_directory_without_changing_it(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "memory.jsonl"
            marker.write_text("existing user memory")
            code, _, _ = bench.execute([str(bench.BINARY), "scale", "--dir", directory, "--records", "10"], 30)
            self.assertNotEqual(code, 0)
            self.assertEqual(marker.read_text(), "existing user memory")

    def test_decision_suite_has_every_family_and_reproducible_results(self):
        command = [str(bench.BINARY), "decisions", "--cases", "120", "--seed", "314159"]
        runs = []
        for _ in range(2):
            code, stdout, stderr = bench.execute(command, 30)
            self.assertEqual(code, 0, stderr)
            result = json.loads(stdout)
            result.pop("elapsed_ms")
            self.assertEqual(result["violations"], 0)
            self.assertEqual(list(result["families"].values()), [20]*6)
            runs.append(result)
        self.assertEqual(*runs)


if __name__ == "__main__":
    unittest.main()
