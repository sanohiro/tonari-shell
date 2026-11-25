"""Compare clean configure runs and account for child intervals in paired runs."""
from collections import Counter
from decimal import Decimal
from importlib import import_module
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

clocked_source = import_module("configure-profile").clocked_source


def measure(script, shell, directory, compiler, log):
    directory.mkdir()
    env = dict(PATH=os.environ["PATH"], HOME=str(directory), TMPDIR=str(directory),
               LC_ALL="C", CC=str(compiler), SHELL=str(shell), CONFIG_SHELL=str(shell))
    if log is not None:
        env["CONFIGURE_CLOCK_LOG"] = str(log)
    try:
        start = time.clock_gettime_ns(time.CLOCK_MONOTONIC)
        result = subprocess.run([shell, script, "--disable-nls", "--cache-file=/dev/null"],
                                stdin=subprocess.DEVNULL, capture_output=True, env=env,
                                cwd=directory, timeout=180, check=True)
        elapsed = (time.clock_gettime_ns(time.CLOCK_MONOTONIC) - start) / 1e9
        interpreter = (directory / "config.status").read_bytes().splitlines()[0]
        if interpreter.removeprefix(b"#!").strip() != bytes(shell):
            raise ValueError(f"different config.status interpreter: {interpreter!r}")
        checks = [v.replace(bytes(compiler), b"/usr/bin/cc") for v in result.stdout.splitlines()
                  if v.startswith(b"checking ")]
        if len(checks) < 200 or result.stderr:
            raise ValueError(f"incomplete configure: {len(checks)} checks, {result.stderr!r}")
        witness = (checks, (directory / "config.h").read_bytes())
    finally:
        shutil.rmtree(directory)
    record = {"elapsed": elapsed}
    if log is not None:
        intervals = []
        for line in log.read_text().splitlines():
            name, left, right = line.split()
            intervals.append((int(left), int(right), name))
        totals = Counter()
        counts = Counter()
        covered_until = start
        for left, right, name in sorted(intervals):
            if not start <= left <= right <= start + round(elapsed * 1e9):
                raise ValueError("child clock outside configure interval")
            totals[name] += max(0, right - max(left, covered_until)) / 1e9
            covered_until = max(right, covered_until)
            counts[name] += 1
        if set(totals) != {"/usr/bin/cc", "./conftest"}:
            raise ValueError(f"unexpected clocked programs: {counts}")
        record.update(compiler=totals["/usr/bin/cc"], probes=totals["./conftest"],
                      outside=elapsed-sum(totals.values()), counts=dict(counts))
    return record, witness


def roundtrips(shells, clock, root, output):
    # Keep the interpreter alive. Match timestamps across the child and parent
    # for each invocation; shell timestamps are quantized to microseconds.
    rows = []
    for name, command in (("foreground", '"$1" --roundtrip'),
                          ("dup", '"$1" --roundtrip >&5 2>&1'),
                          ("open", '"$1" --roundtrip >/dev/null 2>&1'),
                          ("substitution", 'value=$("$1" --roundtrip)'),
                          ("background-wait", '"$1" --roundtrip & wait "$!"')):
        body = ('exec 5>/dev/null 7>child 8>parent\n'
                'for ((i=0; i<64; i++)); do\nleft=$EPOCHREALTIME\n' + command +
                '\nrc=$? right=$EPOCHREALTIME\n((rc == 0)) || exit 1\n'
                'printf "%s %s\\n" "$left" "$right" >&8\ndone\n')
        for trial in range(7):
            for label in (list(shells) if trial % 2 == 0 else reversed(shells)):
                result = subprocess.run([shells[label], "-c", body, "sh", clock], cwd=root,
                                        env=dict(PATH="/usr/bin:/bin", HOME=str(root), LC_ALL="C"),
                                        capture_output=True, check=True, timeout=30)
                if result.stdout or result.stderr:
                    raise ValueError(f"unexpected {label} output on {name}")
                parent = (root / "parent").read_text().splitlines()
                children = (root / "child").read_text().splitlines()
                if len(parent) != len(children) or len(parent) != 64:
                    raise ValueError("missing launch timestamps")
                for index, (p, c) in enumerate(zip(parent, children)):
                    left, right = [int(Decimal(v)*1_000_000_000) for v in p.split()]
                    entry, leaving = map(int, c.split())
                    if not left <= entry <= leaving <= right + 1000:
                        raise ValueError(f"clock moved backwards on {name}")
                    if trial:
                        rows.append((name, label, trial, index, (entry-left)/1e6,
                                     (right-leaving)/1e6, (right-left)/1e6))
    (output / "roundtrips.json").write_text(json.dumps(rows) + "\n")
    for name in dict.fromkeys(row[0] for row in rows):
        for label in shells:
            data = [row for row in rows if row[:2] == (name, label)]
            print(name, label, "entry/return/total median ms",
                  *(round(statistics.median(row[index] for row in data), 4)
                    for index in (4, 5, 6)), flush=True)


def main():
    source, tonari, bash, destination = map(Path, sys.argv[1:])
    output = destination.resolve()
    output.mkdir(parents=True, exist_ok=True)
    shells = {"tonari": tonari.absolute(), "bash": bash.absolute()}
    runs = []
    witness = None
    with tempfile.TemporaryDirectory() as work:
        root = Path(work).resolve()
        probe, compiler = clocked_source(source, root)
        roundtrips(shells, root / "clock", root, output)
        # The same interpreter path, argv, working directory and HOME in every
        # pair. Each run starts with an empty build tree. Alternate AB/BA in
        # each mode and alternate the clean and clocked pairs between rounds.
        link = root / "bash"
        for trial in range(7):
            for mode in (("clean", "clock") if trial % 2 == 0 else ("clock", "clean")):
                for label in (list(shells) if (trial + (mode == "clock")) % 2 == 0 else reversed(shells)):
                    link.unlink(missing_ok=True)
                    link.symlink_to(shells[label])
                    log = output / f"{trial}-{mode}-{label}.clock" if mode == "clock" else None
                    if log:
                        log.write_text("")
                    row, result = measure(probe if log else source.resolve(), link,
                                          root / "build", compiler if log else "/usr/bin/cc", log)
                    if witness is not None and result != witness:
                        raise ValueError(f"{label} changed configure checks or config.h in {mode}")
                    witness = result
                    row.update(trial=trial, shell=label, mode=mode, checks=len(result[0]))
                    runs.append(row)
                    print(json.dumps(row), flush=True)
                    (output / "paired.json").write_text(json.dumps(runs, indent=2) + "\n")


if __name__ == "__main__":
    main()
