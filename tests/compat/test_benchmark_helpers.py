"""Exercise native audit helpers through the same executables used by CI."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


class BenchmarkHelpers(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        temporary = tempfile.TemporaryDirectory()
        cls.addClassCleanup(temporary.cleanup)
        cls.work = Path(temporary.name)
        for name in ("configure-clock", "oils-argv", "oils-printenv"):
            subprocess.run(["cc", "-O2", ROOT / "tools" / "ci" / (name + ".c"),
                            "-o", cls.work / name], check=True)

    def test_clock_validates_the_log_before_running_the_child(self):
        clock = self.work / "configure-clock"
        log = self.work / "measurements"
        marker = self.work / "child-ran"
        command = [clock, "/bin/sh", "-c", 'echo ok > "$1"; exit 7', "sh", marker]
        for value in (None, "", str(log)):
            env = {} if value is None else {"CONFIGURE_CLOCK_LOG": value}
            self.assertEqual(subprocess.run(command, env=env).returncode, 125)
            self.assertFalse(marker.exists())
        log.touch()
        self.assertEqual(subprocess.run(command, env={"CONFIGURE_CLOCK_LOG": str(log)}).returncode, 7)
        self.assertEqual(marker.read_bytes(), b"ok\n")
        name, start, end, user, system = log.read_text().split()
        self.assertEqual(name, "/bin/sh")
        self.assertLessEqual(int(start), int(end))
        self.assertGreaterEqual(min(int(user), int(system)), 0)

    def test_clock_roundtrip_requires_only_its_output_descriptor(self):
        clock = self.work / "configure-clock"
        command = ["/bin/sh", "-c", 'exec 7>&1; exec "$1" --roundtrip', "sh", clock]
        result = subprocess.run(command, env={}, capture_output=True, check=True)
        entered, leaving = map(int, result.stdout.split())
        self.assertLessEqual(entered, leaving)
        self.assertEqual(result.stderr, b"")

    def test_oils_helpers_report_buffered_output_errors(self):
        with open(os.devnull, "rb") as unwritable:
            for name, args, expected in [
                ("oils-argv", ["a b", "c"], b"['a b', 'c']\n"),
                ("oils-printenv", ["VAR", "UNSET"], b"abc\nNone\n"),
            ]:
                command = [self.work / name, *args]
                env = {"VAR": "abc"}
                self.assertEqual(subprocess.check_output(command, env=env), expected)
                self.assertNotEqual(subprocess.run(command, stdout=unwritable, env=env).returncode, 0)
