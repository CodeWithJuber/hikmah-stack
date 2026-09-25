import contextlib
import io
import json
import os
import sqlite3
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import real_datasets as bench

BINARY = bench.ROOT / "target/release/examples/real_bench_port"


class MetricsTests(unittest.TestCase):
    def test_gold_label_does_not_change_request(self):
        row = {"dataset": "sms_spam", "state": "Hello friend", "gold": "ham"}
        first = bench.make_request(row, ["ham", "spam"])
        row["gold"] = "PRIVATE_GOLD_SENTINEL"
        second = bench.make_request(row, ["ham", "spam"])
        self.assertEqual(first, second)
        self.assertNotIn("PRIVATE_GOLD_SENTINEL", bench.canonical(second))

    def test_abstention_reduces_coverage_and_full_accuracy(self):
        rows = [{"gold": "ham", "x": {"prediction": "ham", "ms": 2}},
                {"gold": "spam", "x": {"prediction": "ham", "ms": 4}},
                {"gold": "spam", "x": {"prediction": None, "ms": 6}}]
        result = bench.metrics(rows, "x", ["ham", "spam"])
        self.assertAlmostEqual(result["accuracy_all"], 1/3)
        self.assertEqual(result["accuracy_answered"], .5)
        self.assertAlmostEqual(result["coverage"], 2/3)
        self.assertEqual(result["per_class"]["spam"]["recall"], 0)

    def test_empty_baseline_has_no_conditional_accuracy(self):
        row = {"gold": "ham", "x": {"prediction": None}}
        result = bench.metrics([row], "x", ["ham", "spam"])
        self.assertIsNone(result["accuracy_answered"])
        self.assertEqual(result["accuracy_all"], 0)

    def test_vendor_confidence_is_not_substituted_for_probability(self):
        result = bench.decoded({"type": "choice", "choice": "ham", "confidence": .99}, ["ham", "spam"])
        self.assertIsNone(result["top_label_probability"])

    def test_noul_and_malformed_probability(self):
        self.assertEqual(bench.decoded({"type": "noul", "noul": .2}, ["true", "false"])["prediction"], "false")
        self.assertIsNone(bench.decoded({"type": "noul", "noul": 9}, ["true", "false"])["prediction"])
        self.assertIsNone(bench.decoded(None, ["a", "b"])["prediction"])

    def test_wilson_bounds_and_tail_latency(self):
        lo, hi = bench.wilson(4, 4)
        self.assertLess(lo, .6)
        self.assertAlmostEqual(hi, 1)
        self.assertEqual(bench.quantile([1, 1, 1, 1, 600, 700], .5), 1)
        self.assertGreater(bench.quantile([1, 1, 1, 1, 600, 700], .95), 600)

    def test_persistent_api_cap_stops_before_network(self):
        with sqlite3.connect(":memory:") as db:
            db.execute("CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT)")
            db.execute("INSERT INTO meta VALUES('attempts','20')")
            with patch.object(bench.urllib.request, "urlopen") as network:
                with self.assertRaises(bench.StopRun):
                    bench.call_jev(db, {}, "unused-placeholder", SimpleNamespace(max_calls=20))
                network.assert_not_called()


@unittest.skipUnless(BINARY.exists(), "compile real_bench_port before integration tests")
class KernelTests(unittest.TestCase):
    def setUp(self):
        self.kernel = bench.Kernel(BINARY)
        self.request = {"state": "A sample utterance", "questions": [{"id": "label", "instructions": "Classify", "type": "choice", "options": {"a": "A", "b": "B"}}]}

    def tearDown(self):
        self.kernel.close()

    def response(self, answer):
        return {"model": "jev-1.13.0", "answers": {"label": answer}}

    def test_baseline_abstains(self):
        result = self.kernel.ask(self.request)
        self.assertEqual(result["baseline"]["answers"]["label"]["type"], "abstain")

    def test_valid_response_is_preserved_but_not_claimed_calibrated(self):
        answer = {"type": "choice", "choice": "b", "probabilities": {"a": .1, "b": .9}}
        result = self.kernel.ask(self.request, self.response(answer))["combined"]
        self.assertEqual(result["answers"]["label"]["choice"], "b")
        self.assertFalse(result["calibrated"])
        self.assertIsNone(result["rejected"])

    def test_fault_injection_is_rejected(self):
        faults = [
            {"type": "choice", "choice": "other"},
            {"type": "choice", "choice": "a", "probabilities": {"a": 2, "b": -1}},
            {"type": "choice", "choice": "a", "probabilities": {"a": .1, "b": .9}},
            {"type": "noul", "noul": .7},
        ]
        for answer in faults:
            with self.subTest(answer=answer):
                result = self.kernel.ask(self.request, self.response(answer))["combined"]
                self.assertIsNotNone(result["rejected"])
                self.assertEqual(result["answers"]["label"]["type"], "abstain")

    def test_resume_does_not_repeat_a_paid_response(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data_dir = root / "data"
            data_dir.mkdir()
            row = {"id": "sms_spam/corpus/000000", "dataset": "sms_spam", "gold": "ham", "state": "Hello friend"}
            data = (bench.canonical(row)+"\n").encode()
            (data_dir / "cases.jsonl").write_bytes(data)
            bench.write_json(data_dir / "manifest.json", {"labels": {"sms_spam": ["ham", "spam"]}, "cases_sha256": bench.sha(data)})
            args = SimpleNamespace(data=data_dir, out=root / "result", limit=0, seed=1, live=True,
                                   model="jev-1.13.0", binary=BINARY, rps=10, timeout=2, max_calls=5)
            calls = []

            def fake_call(db, body, key, params):
                calls.append(body)
                db.execute("UPDATE meta SET value='1' WHERE key='attempts'")
                db.commit()
                return {"response": {"model": "jev-1.13.0", "answers": {"label": {"type": "choice", "choice": "ham", "probabilities": {"ham": .9, "spam": .1}}}}, "ms": 10, "attempts": 1}

            with patch.dict(os.environ, {"TYPESAFE_API_KEY": "test-placeholder"}), patch.object(bench, "call_jev", fake_call), contextlib.redirect_stdout(io.StringIO()):
                bench.run(args)
                bench.run(args)
            self.assertEqual(len(calls), 1)
            summary = json.loads((args.out / "summary.json").read_text())
            self.assertEqual(summary["api_attempts"], 1)
            self.assertEqual(summary["datasets"]["sms_spam"]["jev"]["accuracy_all"], 1)
            self.assertNotIn("test-placeholder", (args.out / "results.jsonl").read_text())


if __name__ == "__main__":
    unittest.main()
