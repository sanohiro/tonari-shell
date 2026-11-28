"""Separate process startup from loading the installed bash-completion rc file."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import random
import shutil
import statistics
import subprocess
import tempfile
import time

from bench import first_prompt

PACKAGE_211 = "9c66501f9aec033f4cb9d79058f47889d128c20bbea04b15035ab4c6847515be"
PROMPT = b"tn-completion-ready~ "
CHECK = "declare -F _init_completion >/dev/null || declare -F _comp_initialize >/dev/null || exit 1"


def clock_us(value):
    second, microsecond = value.split(".")
    return int(second) * 1_000_000 + int(microsecond)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    parser.add_argument("shells", nargs="+", metavar="LABEL=PATH")
    parser.add_argument("--runs", type=int, default=15)
    args = parser.parse_args()
    package = args.package.resolve()
    digest = hashlib.sha256(package.read_bytes()).hexdigest()
    print(json.dumps(dict(package=str(package), sha256=digest,
                          compat_dir=sorted(p.name for p in Path("/etc/bash_completion.d").glob("*")))))
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        home = root / "home"
        home.mkdir()
        compat = root / "empty"
        compat.mkdir()
        rc = home / ".bashrc"
        result = home / "rc-times"
        env = dict(HOME=str(home), TERM="xterm-256color", PATH="/usr/bin:/bin", LC_ALL="C",
                   TN_RC_TIMES=str(result), TN_PACKAGE=str(package))
        shells = {}
        for spec in args.shells:
            label, path = spec.split("=", 1)
            shell = root / str(len(shells)) / "bash"
            shell.parent.mkdir()
            shutil.copy(path, shell)
            shells[label] = shell
        rng = random.Random(211)
        def measure(case, n, sample):
            values = {}
            for i in range(2 + args.runs):
                order = list(shells)
                rng.shuffle(order)
                for label in order:
                    for metric, value in sample(shells[label]).items():
                        if i >= 2:
                            values.setdefault((label, metric), []).append(value)
            for (label, metric), series in values.items():
                print(json.dumps(dict(case=case, n=n, shell=label, metric=metric, runs=len(series),
                                      median=statistics.median(series),
                                      p95=sorted(series)[math.ceil(.95 * len(series)) - 1])), flush=True)

        def prompt_case(name, n, body, case_env, marks):
            # Record expansion of EPOCHREALTIME in-shell around each section.
            # A missing completion function or nonzero source status prevents readiness.
            rc.write_text("tn_start=$EPOCHREALTIME\n" + body + "\ntn_end=$EPOCHREALTIME\n"
                          + (CHECK + "\n" if n else "")
                          + "printf '%s\\n' " + " ".join(f'"$tn_{key}"' for key in marks)
                          + ' > "$TN_RC_TIMES"\nPS1=' + repr(PROMPT.decode()) + "\n")
            def sample(shell):
                _, display = first_prompt([str(shell), "--noprofile", "-i"], case_env, PROMPT)
                stamps = [clock_us(v) for v in result.read_text().splitlines()]
                durations = {stage: (end - start) / 1000 for stage, start, end in
                             zip(marks[1:], stamps, stamps[1:])}
                return dict(exec_to_prompt=display, **durations)
            measure(name, n, sample)

        prompt_case("empty-rc", 0, ":", env, ["start", "end"])
        load = '. "$TN_PACKAGE" || exit $?\n'
        prompt_case("source-ambient", 1, load, env, ["start", "end"])
        core_env = env | {"BASH_COMPLETION_COMPAT_DIR": str(compat)}
        for n in (1, 2, 4):
            prompt_case("source-empty-compat", n, load * n, core_env, ["start", "end"])

        # Insert only timestamp assignments between complete top-level commands.
        # Check the exact distro file before relying on its physical line numbers.
        if digest == PACKAGE_211:
            phases = {86: "initial", 2127: "definitions", 2186: "xspec", 2268: "loaders"}
            lines = package.read_text().splitlines(keepends=True)
            parts = []
            for i, line in enumerate(lines, 1):
                if i in phases:
                    parts.append(f"tn_{phases[i]}=$EPOCHREALTIME\n")
                parts.append(line)
            phased = root / "bash_completion"
            phased.write_text("".join(parts))
            for label, case_env in (("ambient", env), ("empty-compat", core_env)):
                prompt_case("sections-" + label, 1, load, case_env | {"TN_PACKAGE": str(phased)},
                            ["start", *phases.values(), "end"])

        blank = root / "empty.sh"
        blank.touch()
        for label, path in (("empty", blank), ("package", package)):
            def syntax(shell):
                start = time.perf_counter_ns()
                proc = subprocess.run([shell, "-O", "extglob", "-n", path], env=env, cwd=home,
                                      stdin=subprocess.DEVNULL, capture_output=True, timeout=10)
                elapsed = (time.perf_counter_ns() - start) / 1e6
                if (proc.returncode, proc.stdout, proc.stderr) != (0, b"", b""):
                    raise AssertionError((shell, path, proc))
                return {"wall": elapsed}
            measure("syntax-" + label, 1, syntax)


if __name__ == "__main__":
    main()
