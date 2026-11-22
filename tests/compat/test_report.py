"""The speed and coverage gates must fail when a previously verified result is lost."""
import unittest

from report import misses


class ReleaseGate(unittest.TestCase):
    key = ("Builtin", "printf")

    def result(self, elapsed=60, status="match"):
        return {self.key: {"status": status, "whole_n": 15, "whole_bash_p50_ns": 90,
                           "whole_tonari_p50_ns": elapsed, "whole_error": "",
                           "started_n": "", "started_error": ""}}

    def test_upper_target_uses_exact_fraction(self):
        self.assertEqual(misses(self.result(60), self.result(), True, True), [])
        self.assertTrue(misses(self.result(61), self.result(), False, False))

    def test_known_slow_path_still_fails_absolute_target(self):
        self.assertEqual(misses(self.result(70), self.result(70), False, False), [])
        self.assertTrue(misses(self.result(70), self.result(70), True, False))

    def test_disappearing_rows_behavior_and_measurements(self):
        self.assertTrue(misses({}, self.result(), False, False))
        self.assertTrue(misses(self.result(status="different"), self.result(), False, False))
        changed = self.result()
        changed[self.key].update(whole_n="", whole_error="changed reference observation")
        self.assertTrue(misses(changed, self.result(), False, False))

    def test_interface_only_does_not_count_as_complete(self):
        self.assertTrue(misses(self.result(status="interface only"), {}, False, True))


if __name__ == "__main__":
    unittest.main()
