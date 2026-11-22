"""Time successful Bash feature witnesses and validate the observation on every sample."""
import argparse
import hashlib
import shutil
import collections
import importlib.util
import json
import math
import os
from pathlib import Path
import platform
import re
import socket
import statistics
import sys
import tempfile
import time

from audit import Audit
from terminal import prompt, dialogue
from upstream import run as upstream_run


def terminal_checks(binary):
    spec = importlib.util.spec_from_file_location("shell_tests", Path(__file__).resolve().parents[1] / "shell.py")
    module = importlib.util.module_from_spec(spec)
    sys.argv.insert(1, str(binary))
    spec.loader.exec_module(module)
    return module.ShellTests


def as_workload(rows, group, name):
    row = rows[group][name]
    if "workload" in row:
        return row["workload"]
    if row["status"] == "golden match":
        return {"driver": name, "diffs": row["driver"]["bash"]["diffs"]}
    if group == "Terminal" and row["status"] == "match":
        return {"pty_test": name}
    if row["status"] == "match" and " / " in row["witness"]:
        other, item = row["witness"].split(" / ", 1)
        if item in rows.get(other, {}):
            return as_workload(rows, other, item)
    return None


def stats(samples):
    def summary(values):
        return [int(statistics.median(values)), sorted(values)[math.ceil(.95 * len(values)) - 1]]
    return dict(n=len(samples[0]), bash=summary(samples[0]), tonari=summary(samples[1]))


def sample(a, work, index, mode, cli, test_class):
    if "argv" in work:
        o = a.call(index, work["argv"], work["stdin"], work["fixture"], timing=mode)
        return o.digest() == work["expected"], o.ns
    shell = a.shells[index]
    with tempfile.TemporaryDirectory(dir=a.root) as temp:
        cwd = Path(temp)
        if "driver" in work:
            result = upstream_run(work["driver"], shell, cwd, cli.source, cli.helpers, cli.timeout)
            ok = not (result["exit"] or result["timeout"] or result["skip"]) and result["diffs"] == work["diffs"]
            return ok, result["ns"]
        start = time.perf_counter_ns()
        if "prompt" in work:
            value = prompt(shell, cwd, work["prompt"])
            if work["prompt"] in (r"\h", r"\H"):
                host = socket.gethostname()
                valid = value == (host.split(".")[0] if work["prompt"] == r"\h" else host).encode()
            elif work["prompt"] in (r"\w", r"\W"):
                # macOS may report /private/var while HOME was passed as /var.
                if bytes.fromhex(work["expected"]) == b"~":
                    valid = value == b"~"
                else:
                    directory = cwd.resolve()
                    valid = value == os.fsencode(directory if work["prompt"] == r"\w" else directory.name)
            elif work["shape"]:
                valid = value is not None and bool(re.fullmatch(work["shape"].encode(), value, re.I))
            else:
                valid = value is not None and value.hex() == work["expected"]
        elif "dialogue" in work:
            name = work["dialogue"]
            value = dialogue(shell, cwd, name)
            if name in ("completion", "READLINE", "COLUMNS"):
                fields = value.split(b"," if name == "COLUMNS" else b"\0")
                idx = work["field"]
                value = b"\0".join(fields[idx:]) if work["item"] == "COMP_WORDS" else (fields[idx] if idx < len(fields) else b"no field")
            valid = value.hex() == work["expected"]
        else:
            test = test_class()
            test.setUp()
            try:
                if work["pty_test"] == "foreground pipeline":
                    test.check_foreground_pipeline(test.terminal_session(shell=shell))
                else:
                    test.check_interactive_alias_and_interrupt_trap(shell)
                valid = True
            except (AssertionError, OSError):
                valid = False
            finally:
                test.doCleanups()
        return valid, time.perf_counter_ns() - start


def benchmark(a, work, mode, cli, test_class):
    if "driver" in work and (not cli.source or not cli.helpers):
        return {"error": "GNU fixture unavailable"}
    if "driver" in work and (os.getuid() == 0 or not Path("/.dockerenv").exists()):
        raise ValueError("Run GNU drivers in a disposable non-root container")
    samples = [[], []]
    count = cli.suite_runs if "driver" in work else cli.runs
    for i in range(cli.warmups + count):
        for index in (i % 2, 1 - i % 2):
            ok, ns = sample(a, work, index, mode, cli, test_class)
            if not ok:
                return {"error": "changed reference observation" if index == 0 else "changed Tonari observation"}
            if i >= cli.warmups:
                samples[index].append(ns)
    return stats(samples)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ("audit", "bash", "tonari"):
        parser.add_argument("--" + flag, type=Path, required=True)
    for flag in ("source", "helpers"):
        parser.add_argument("--" + flag, type=Path)
    parser.add_argument("--runs", type=int, default=15)
    parser.add_argument("--suite-runs", type=int, default=7)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--timeout", type=int, default=110)
    args = parser.parse_args()
    if min(args.suite_runs, args.runs) < 1 or args.warmups < 0:
        parser.error("samples must be positive and warmups nonnegative")
    data = json.loads(args.audit.read_text())
    rows = data["rows"]
    grouped = collections.defaultdict(list)
    for group, entries in rows.items():
        for name, row in entries.items():
            if row["status"] in ("match", "golden match", "interface only"):
                work = as_workload(rows, group, name)
                if work is not None:
                    grouped[json.dumps(work, sort_keys=True)].append((group, name))
    test_class = terminal_checks(args.tonari)
    with tempfile.TemporaryDirectory() as temp:
        # A host-shared mount under a Linux VM can cost more per exec than the shell.
        # Place both installed binaries on the work filesystem before any timing.
        shells = []
        hashes = []
        for label, binary in (("ref", args.bash), ("new", args.tonari)):
            directory = Path(temp) / label
            directory.mkdir()
            path = directory / "bash"
            shutil.copy(binary.resolve(), path)
            shells.append(path)
            hashes.append(hashlib.sha256(path.read_bytes()).hexdigest())
        audited = data.get("executable_sha256", data.get("benchmark", {}).get("executable_sha256"))
        if audited is not None and hashes != audited:
            parser.error("executable changed since the compatibility check; regenerate the audit before timing")
        a = Audit(*shells, Path(temp))
        for n, (key, items) in enumerate(grouped.items(), 1):
            work = json.loads(key)
            modes = ["whole"]
            if work.get("argv", [])[:3] == ["--noprofile", "--norc", "-c"] and len(work["argv"]) == 4:
                modes.append("started")
            measurements = {mode: benchmark(a, work, mode, args, test_class) for mode in modes}
            for g, name in items:
                rows[g][name]["timing"] = measurements
            if n % 100 == 0:
                print(f"{n}/{len(grouped)} witnesses", file=sys.stderr, flush=True)
    data["benchmark"] = dict(host=platform.platform(), runs=args.runs, suite_runs=args.suite_runs,
                             warmups=args.warmups, distinct_workloads=len(grouped), unit="ns", executable_sha256=hashes)
    print(json.dumps(data, ensure_ascii=True, separators=(",", ":")))


if __name__ == "__main__":
    main()
