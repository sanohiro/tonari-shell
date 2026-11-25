"""Measure repeated sh-mode operations with a Bash witness on each sample."""
import argparse
import json
from pathlib import Path
import random
from statistics import median
import subprocess
from tempfile import TemporaryDirectory
import time


WORKLOADS = {
    "startup": ("command", ":"),
    "prefix": ("command", "i=0; while [ $i -lt 10000 ]; do s=$i :; i=$((i+1)); done; echo $s"),
    "echo": ("command", "{ i=0; while [ $i -lt 12000 ]; do echo hello; i=$((i+1)); done; } >/dev/null; echo $i"),
    "spawn": ("command", "i=0; while [ $i -lt 100 ]; do /usr/bin/true; i=$((i+1)); done; echo $i"),
    "alias": ("file", "alias e='echo ok'\n{\n" + "e\n"*500 + "} >/dev/null\ne\n"),
    "alias-3000": ("file", "alias e='echo ok'\n{\n" + "e\n"*3000 + "} >/dev/null\ne\n"),
    "stdin": ("stdin", "i=1\nx=$i\n"*750 + "echo $x\n"),
}


def measure(base, shells):
    subjects = {}
    for label, binary in shells.items():
        directory = base / label
        directory.mkdir()
        subjects[label] = directory / "sh"
        subjects[label].symlink_to(binary.resolve())
    env = {"PATH": "/usr/bin:/bin", "LC_ALL": "C", "HOME": str(base)}
    for name, (mode, source) in WORKLOADS.items():
        script = base / "input"
        script.write_text(source)
        args = {"command": ["-c", source], "file": [script], "stdin": []}[mode]

        def sample(label):
            with script.open("rb") as stdin:
                start = time.monotonic_ns()
                result = subprocess.run([subjects[label], *args], env=env, cwd=base, stdin=stdin,
                                        capture_output=True, timeout=8)
                elapsed = (time.monotonic_ns() - start) / 1e6
            return (result.returncode, result.stdout, result.stderr), elapsed

        witness, _ = sample("bash")
        if witness[0] != 0:
            raise RuntimeError(f"{name}: invalid Bash witness {witness!r}")
        timings = {label: [] for label in shells}
        rng = random.Random(41)
        for trial in range(1 + (61 if name == "startup" else 17)):
            order = list(shells)
            rng.shuffle(order)
            for label in order:
                result, elapsed = sample(label)
                if result != witness:
                    raise RuntimeError(f"{name}/{label}: {result!r} != {witness!r}")
                if trial:
                    timings[label].append(elapsed)
        yield {"case": name, "median_ms": {k: round(median(v), 3) for k, v in timings.items()},
               "samples_ms": timings}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for label in ("first", "tonari", "bash"):
        parser.add_argument("--" + label, type=Path, required=True)
    with TemporaryDirectory() as directory:
        for record in measure(Path(directory), vars(parser.parse_args())):
            print(json.dumps(record), flush=True)
