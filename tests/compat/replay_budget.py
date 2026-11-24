"""Replay a frozen set of above-budget Bash observations against every release candidate."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

from audit import Audit

MANIFEST = Path(__file__).with_name("budget-2026-11-24.jsonl")
END = b"__TONARI_BENCH_COMPLETE__"


def floor_args(source, count):
    if source.startswith("unset v; (") and source.count("(printf ") == 3:
        if source.count(") 2>/dev/null") == 3:
            return ["groups", str(count), "redirect"]
        if ">/dev/null" not in source:
            return ["groups", str(count)]
    cond = re.fullmatch(r'printf data >file; :; \[\[ (-[a-zA-Z]) ([^ ;]+) \]\]; printf %s "\$\?"; \[\[ \1 absent \]\]; printf %s "\$\?"', source)
    if cond:
        return ["file", str(count), *cond.groups()]


def sample_floor(binary, args, cwd):
    with subprocess.Popen([binary, *args], cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE) as proc:
        if proc.stdout.readline() != b"__READY__\n":
            raise ValueError("control failed to start")
        start = time.perf_counter_ns()
        out = proc.communicate(b"\n")[0]
        elapsed = time.perf_counter_ns() - start
        if proc.returncode or out:
            raise ValueError("control failed")
        return elapsed


def compare(audits, row, count, runs, floor):
    argv = row["argv"]
    def run(shell, source):
        return audits[shell].call(1, [*argv[:-1], source], fixture=row["fixture"], timing="started")
    original = run("bash", argv[-1])
    if original.timeout or row["stdin"]:
        raise ValueError("missing reference")
    status = f"__TONARI_BENCH_STATUS_{original.code}__".encode()
    head = f"for __tonari_bench_i in {{1..{count}}}; do\n:\n{argv[-1]}\n"
    tail = "\ndone\nbuiltin printf __TONARI_BENCH_COMPLETE__"
    checked = head + """builtin printf '__TONARI_BENCH_STATUS_%d__' "$?" >&2""" + tail
    expected = original.out * count + END
    def verify(out, marker):
        return (not out.timeout and out.code == 0 and out.out == expected
                and out.err == (original.err + marker) * count)
    names = list(audits)
    for name in names:
        o = run(name, checked)
        if not verify(o, status):
            raise ValueError(f"{name} iteration check {o.brief()}")
    native = floor_args(argv[-1], count) if floor else None
    if native:
        names.append("direct")
    samples = {name: [] for name in names}
    for i in range(runs + 1):
        for name in names[i % len(names):] + names[:i % len(names)]:
            if name == "direct":
                ns = sample_floor(floor, native, audits["bash"].root / "work")
            else:
                o = run(name, head + tail)
                if not verify(o, b""):
                    raise ValueError(f"{name} timed output {o.brief()}")
                ns = o.ns
            if i:
                samples[name].append(ns)
    return {"count": count, "samples_ns": samples, "control": native}


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--lane", required=True)
    cli.add_argument("--bash", type=Path, required=True)
    cli.add_argument("--bin", type=Path, required=True)
    cli.add_argument("--runs", type=int, default=9)
    cli.add_argument("--only", nargs="*")
    args = cli.parse_args()
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        audits = {}
        hashes = {}
        for name in ("bash", "baseline", "current", "opt-3", "opt-s"):
            original = args.bash if name == "bash" else args.bin / name
            staged = root / name / "bash"
            staged.parent.mkdir()
            shutil.copy(original, staged)
            hashes[name] = hashlib.sha256(staged.read_bytes()).hexdigest()
            audits[name] = Audit(staged, staged, root)
        floor = root / "control"
        subprocess.run(["cc", "-O2", str(Path(__file__).with_name("budget_floor.c")), "-o", str(floor)], check=True)
        print(json.dumps({"lane": args.lane, "sha256": hashes}), flush=True)
        for line in MANIFEST.read_text().splitlines():
            row = json.loads(line)
            config = row["lanes"].get(args.lane)
            if not config or (args.only is not None and row["id"] not in args.only):
                continue
            n = config["iterations"]
            result = {"id": row["id"]}
            for name, count, runs in [("fixed", n, args.runs), ("extended", 1024, 5)]:
                if name == "extended" and (n != 128 or max(config["old_ns"]) > 4_000_000):
                    continue
                try:
                    result[name] = compare(audits, row, count, runs, floor)
                except ValueError as err:
                    result[name] = {"error": str(err)}
            print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
