"""Measure process launch through exit with the system and distribution login profiles."""

import argparse
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

from bench import COMMANDS, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tonari", type=Path)
    parser.add_argument("--bash", type=Path, default=Path("/bin/bash"))
    parser.add_argument("--previous", type=Path, help="previous Tonari release for comparison")
    parser.add_argument("--runs", type=int, default=101)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")

    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        home = root / "home"
        home.mkdir()
        for name in (".profile", ".bashrc"):
            skeleton = Path("/etc/skel") / name
            if skeleton.is_file():
                shutil.copyfile(skeleton, home / name)
        env = {"HOME": str(home), "TERM": "dumb", "PATH": "/usr/bin:/bin", "LC_ALL": "C"}
        shells = []
        for name, source in (("Bash", args.bash), ("Previous", args.previous), ("Tonari", args.tonari)):
            if source is not None:
                path = root / name / "bash"
                path.parent.mkdir()
                shutil.copy(source, path)
                shells.append((name, path))

        for label, flags in (("plain", ["-c", ":"]), ("login", ["--login", "-c", ":"]),
                             ("interactive login", ["--login", "-ic", ":"]),
                             ("spawn series", ["-c", COMMANDS["external-series"][1]])):
            samples = {name: [] for name, _ in shells}
            for run in range(5 + args.runs):
                for offset in range(len(shells)):
                    name, path = shells[(run + offset) % len(shells)]
                    start = time.perf_counter_ns()
                    process = subprocess.run([path, *flags], env=env, cwd=home, stdin=subprocess.DEVNULL,
                                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    end = time.perf_counter_ns()
                    process.check_returncode()
                    if run >= 5:
                        samples[name].append((end - start) / 1_000_000)
            print(label)
            report(samples)


if __name__ == "__main__":
    main()
