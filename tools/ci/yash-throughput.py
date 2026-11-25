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


# Separate the builtin from the mkdir, ls, cut and grep processes used by
# the upstream assertions. These are diagnostic workloads selected explicitly.
DIAGNOSTICS = {
    "plain-script": ("stdin", "./prog foo bar baz\n"),
    "plain-script-30": ("stdin", "i=0; while [ $i -lt 30 ]; do ./prog foo bar baz || exit; i=$((i+1)); done\n"),
    "symbolic-umask": ("command", "umask 027; i=0; while [ $i -lt 5000 ]; do "
                       "umask u=xwr || exit 1; i=$((i+1)); done; umask"),
    "set-flags": ("command", "i=0; while [ $i -lt 5000 ]; do set -e +a -o noclobber -u; "
                  "i=$((i+1)); done; case $- in *C*) echo $i;; *) exit 1;; esac"),
}


def measure(base, shells, spawn_control=None, cases=None, runs=None):
    subjects = {}
    for label, binary in shells.items():
        directory = base / label
        directory.mkdir()
        subjects[label] = directory / "sh"
        subjects[label].symlink_to(binary.resolve())
    env = {"PATH": "/usr/bin:/bin", "LC_ALL": "C", "HOME": str(base)}
    for name in cases or WORKLOADS:
        mode, source = (DIAGNOSTICS if name in DIAGNOSTICS else WORKLOADS)[name]
        if name in {"plain-script", "plain-script-30"}:
            program = base / "prog"
            program.write_text("echo external\necho command\nprintf '[%s]\\n' \"$@\"\n")
            program.chmod(0o755)
        script = base / "input"
        script.write_text(source)
        args = {"command": ["-c", source], "file": [script], "stdin": []}[mode]

        def sample(label):
            with script.open("rb") as stdin:
                start = time.monotonic_ns()
                argv = [spawn_control, "--spawn-100"] if label == "direct" else [subjects[label], *args]
                result = subprocess.run(argv, env=env, cwd=base, stdin=stdin,
                                        capture_output=True, timeout=8)
                elapsed = (time.monotonic_ns() - start) / 1e6
            return (result.returncode, result.stdout, result.stderr), elapsed

        witness, _ = sample("bash")
        if witness[0] != 0:
            raise RuntimeError(f"{name}: invalid Bash witness {witness!r}")
        labels = [*shells, "direct"] if name == "spawn" and spawn_control else list(shells)
        timings = {label: [] for label in labels}
        rng = random.Random(41)
        order = labels.copy()
        for trial in range(1 + (runs or (61 if name == "startup" else 17))):
            if trial <= 1 or (trial - 1) % len(order) == 0:
                rng.shuffle(order)
            else:
                order = order[1:] + order[:1]
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
    parser.add_argument("--spawn-control", type=Path)
    parser.add_argument("--case", choices=[*WORKLOADS, *DIAGNOSTICS], action="append", dest="cases")
    parser.add_argument("--runs", type=int, help="measured trials per workload, excluding warmup")
    args = vars(parser.parse_args())
    cases, runs = args.pop("cases"), args.pop("runs")
    if runs is not None and runs < 1:
        parser.error("--runs must be positive")
    control = args.pop("spawn_control")
    with TemporaryDirectory() as directory:
        for record in measure(Path(directory), args, control, cases, runs):
            print(json.dumps(record), flush=True)
