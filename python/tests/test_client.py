"""Round-trip tests for the hikmah Python bridge.

Runs against the locally built debug binary: set HIKMAH_BIN, or let the test
resolve <repo>/target/debug/hikmah. Runnable with plain python3:

    cd python && python3 -m unittest discover -s tests
    # or
    python3 python/tests/test_client.py
"""

import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from hikmah import Client, HikmahError  # noqa: E402

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(os.path.dirname(TESTS_DIR))


class ClientTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.binary = os.environ.get("HIKMAH_BIN") or os.path.abspath(
            os.path.join(REPO_ROOT, "target", "debug", "hikmah")
        )
        if not os.path.isfile(cls.binary):
            raise unittest.SkipTest(f"hikmah binary not built: {cls.binary}")

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.store = os.path.join(self.tmp.name, "memory.jsonl")
        self.client = Client(binary=self.binary, store=self.store)
        self.client.init()

    def test_remember_recall_roundtrip(self):
        result = self.client.remember(
            "The Q3 release shipped on Friday with zero regressions",
            kind="belief",
            source="human:tester",
            tags=("release", "q3"),
        )
        trace_id = result["trace"]["id"]
        self.assertTrue(trace_id.startswith("tr_"))
        self.assertEqual(result["trace"]["provenance"]["source"], "human:tester")

        hits = self.client.recall("Q3 release shipped")
        self.assertGreaterEqual(len(hits), 1)
        self.assertEqual(hits[0]["trace"]["content"],
                         "The Q3 release shipped on Friday with zero regressions")

    def test_recall_hit_carries_trace_id(self):
        trace_id = self.client.remember("Deploys freeze on Fridays")["trace"]["id"]
        hits = self.client.recall("deploys fridays")
        self.assertTrue(hits)
        for hit in hits:
            self.assertEqual(hit["trace_id"], hit["trace"]["id"])
        self.assertIn(trace_id, [h["trace_id"] for h in hits])

    def test_verify_ledger_green_after_writes(self):
        self.client.remember("First memory")
        self.client.remember("Second memory")
        report = self.client.verify_ledger()
        self.assertTrue(report["ok"])
        self.assertEqual(report["errors"], [])
        self.assertEqual(report["records"], 2)

    def test_get_returns_the_trace(self):
        trace_id = self.client.remember("A memorable fact")["trace"]["id"]
        trace = self.client.get(trace_id)
        self.assertEqual(trace["id"], trace_id)
        self.assertEqual(trace["content"], "A memorable fact")

    def test_get_missing_id_raises(self):
        with self.assertRaises(HikmahError) as ctx:
            self.client.get("tr_missing")
        self.assertIn("trace not found", str(ctx.exception))

    def test_conflicts_detected(self):
        self.assertEqual(self.client.conflicts(), [])
        self.client.remember("Primary region is us-east", claim_key="region",
                             claim_value="us-east")
        self.client.remember("Primary region is eu-west", claim_key="region",
                             claim_value="eu-west")
        conflicts = self.client.conflicts()
        self.assertEqual(len(conflicts), 1)

    def test_supersession_chain(self):
        old_id = self.client.remember("The old runbook is current")["trace"]["id"]
        new_id = self.client.remember("The runbook was replaced in March",
                                      supersedes=old_id)["trace"]["id"]
        # The replaced belief leaves default recall; the correction is found.
        hits = self.client.recall("runbook replaced", include_superseded=True)
        ids = [h["trace_id"] for h in hits]
        self.assertIn(new_id, ids)
        default_hits = self.client.recall("runbook current")
        self.assertNotIn(old_id, [h["trace_id"] for h in default_hits])

    def test_missing_binary_raises_clear_error(self):
        saved_bin = os.environ.pop("HIKMAH_BIN", None)
        saved_path = os.environ.get("PATH", "")
        os.environ["PATH"] = ""
        try:
            with self.assertRaises(HikmahError) as ctx:
                Client()
            self.assertIn("HIKMAH_BIN", str(ctx.exception))
        finally:
            os.environ["PATH"] = saved_path
            if saved_bin is not None:
                os.environ["HIKMAH_BIN"] = saved_bin

    def test_nonexistent_binary_path_raises(self):
        client = Client(binary="/nonexistent/hikmah", store=self.store)
        with self.assertRaises(HikmahError):
            client.recall("anything")

    def test_remember_batch_writes_all_records_in_one_call(self):
        result = self.client.remember_batch([
            {"kind": "observation", "content": "batch fact one", "tags": ["b1"]},
            {"kind": "belief", "content": "batch fact two",
             "claim_key": "py.batch", "claim_value": "yes"},
        ])
        written = result["written"]
        self.assertEqual(len(written), 2)
        self.assertEqual(written[0]["trace"]["content"], "batch fact one")
        self.assertEqual(written[1]["trace"]["claim_key"], "py.batch")

    def test_remember_batch_refuses_invalid_record_atomically(self):
        with self.assertRaises(HikmahError) as ctx:
            self.client.remember_batch([
                {"kind": "observation", "content": "fine"},
                {"kind": "not-a-kind", "content": "bad"},
            ])
        self.assertIn("batch line 2", str(ctx.exception))
        self.assertEqual(self.client.recall("fine"), [])


if __name__ == "__main__":
    unittest.main()
