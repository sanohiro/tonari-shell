"""The speed and coverage gates must fail when a previously verified result is lost."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from audit import Audit, Outcome
from report import misses
from replay_report import classify
from unittest.mock import Mock


class ReleaseGate(unittest.TestCase):
    key = ("Builtin", "printf")

    def result(self, elapsed=60, status="match"):
        return {self.key: {"status": status, "whole_n": 15, "whole_bash_p50_ns": 90,
                           "whole_tonari_p50_ns": elapsed, "whole_error": "",
                           "started_n": "", "started_error": ""}}

    def test_replay_exceptions_require_measured_parity_or_a_matching_control(self):
        def batch(current, bash, count=128, direct=None):
            samples = {"current": [current], "bash": [bash]}
            if direct is not None:
                samples["direct"] = [direct]
            return {"count": count, "samples_ns": samples}

        over = batch(81, 120)
        self.assertEqual(classify(batch(80, 120), {}), "fixed batch within target")
        self.assertEqual(classify(over, batch(1024, 1536, 1024)), "long batch within target")
        self.assertEqual(classify(over, batch(5000 * 1024, 5000 * 1024, 1024)), "short at parity")
        self.assertEqual(classify(over, batch(5001 * 1024, 5000 * 1024, 1024)), "tried, above target")
        self.assertEqual(classify(batch(100, 120, direct=97), {}), "matched OS control")
        self.assertEqual(classify(batch(100, 120, direct=75), {}), "tried, above target")
        self.assertEqual(classify(batch(100, 120, direct=135), {}), "tried, above target")
        self.assertEqual(classify(batch(130, 100, direct=129), {}), "tried, above target")

    def test_upper_target_uses_exact_fraction(self):
        self.assertEqual(misses(self.result(60), self.result(), True, True), [])
        self.assertTrue(misses(self.result(61), self.result(), False, False))

    def test_batch_crossing_after_loading_a_tsv(self):
        before, after = self.result(), self.result()
        for rows, value in ((before, "60"), (after, "61")):
            rows[self.key].update(batch_n="9", batch_iterations="128",
                                  batch_tonari_p50_ns=value, batch_bash_p50_ns="90")
        self.assertIn((*self.key, "repeated command crossed 2/3"), misses(after, before, False, False))
        after[self.key]["batch_iterations"] = "16"
        self.assertEqual(misses(after, before, False, False), [])

    def test_known_slow_path_still_fails_absolute_target(self):
        self.assertEqual(misses(self.result(70), self.result(70), False, False), [])
        self.assertTrue(misses(self.result(70), self.result(70), True, False))

    def test_disappearing_rows_behavior_and_measurements(self):
        self.assertTrue(misses({}, self.result(), False, False))
        self.assertTrue(misses(self.result(status="different"), self.result(), False, False))
        changed = self.result()
        changed[self.key].update(whole_n="", whole_error="changed reference observation")
        self.assertTrue(misses(changed, self.result(), False, False))

    def test_verified_tonari_contract_and_reference_changes(self):
        audit = Audit(None, None, None)
        for reference, candidate, status in [
            (b"old", b"ok", "intentional difference"), (b"ok", b"ok", "match"),
            (b"old", b"old", "contract violation"), (b"new", b"ok", "control inconclusive"),
        ]:
            with self.subTest(reference=reference, candidate=candidate):
                audit.script = Mock(side_effect=[Outcome(0, reference, b""), Outcome(0, candidate, b"")])
                audit.compare("UTF-8 text", "example", "example", text_contract=(b"ok", b"old"))
                self.assertEqual(audit.rows["UTF-8 text"]["example"]["status"], status)

        verified = self.result(status="intentional difference")
        self.assertEqual(misses(verified, {}, True, True), [])
        self.assertEqual(misses(self.result(), verified, False, False), [])
        self.assertTrue(misses(self.result(status="contract violation"), verified, False, False))
        self.assertTrue(misses(self.result(status="contract violation"), {}, False, True))

    def test_benchmark_rejects_a_different_executable(self):
        with tempfile.TemporaryDirectory() as work:
            audit = Path(work) / "checked.json"
            audit.write_text(json.dumps({"rows": {}, "executable_sha256": ["older", "older"]}))
            result = subprocess.run([sys.executable, str(Path(__file__).with_name("bench.py")),
                                     "--audit", audit, "--bash", sys.executable, "--tonari", sys.executable],
                                    capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 2)
        self.assertIn(b"executable changed", result.stderr)
        self.assertEqual(result.stdout, b"")

    def test_interface_only_does_not_count_as_complete(self):
        self.assertTrue(misses(self.result(status="interface only"), {}, False, True))


if __name__ == "__main__":
    unittest.main()
