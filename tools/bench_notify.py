"""Measure a key and background-job notification while an edited command is pending."""
import argparse
import errno
import json
import math
import os
from pathlib import Path
from statistics import median
import tempfile
import time

from bench_history import PROMPT, read_until, terminal


def sample(shell, home):
    fifo = home / "release-job"
    with terminal(shell, home, "/usr/bin:/bin", home / "rc", PROMPT) as fd:
        # Synchronize with the child before measuring, regardless of startup time.
        os.write(fd, b"cat < release-job >/dev/null &\r")
        read_until(fd, b"[1]")
        deadline = time.monotonic() + 5
        while True:
            try:
                writer = os.open(fifo, os.O_WRONLY | os.O_NONBLOCK)
                break
            except OSError as e:
                if e.errno != errno.ENXIO or time.monotonic() >= deadline:
                    raise
                time.sleep(.001)
        try:
            os.write(fd, b"printf 'kept%s\\n' ")
            read_until(fd, b"kept")
            start = time.monotonic_ns()
            os.write(fd, b"Q")
            read_until(fd, b"Q")
            key = time.monotonic_ns() - start
        finally:
            os.close(writer)
        released = time.monotonic_ns()
        read_until(fd, b"Done")
        notice = time.monotonic_ns() - released
        os.write(fd, b"\r")
        read_until(fd, b"keptQ\r\n")
        return [key / 1e6, notice / 1e6]


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("shells", type=Path, nargs="+")
    cli.add_argument("--runs", type=int, default=21)
    args = cli.parse_args()
    shells = [sh.resolve() for sh in args.shells]
    values = {sh: [] for sh in shells}
    with tempfile.TemporaryDirectory() as name:
        home = Path(name)
        (home / "rc").write_text("set -b\nunset HISTFILE\nPS1='tonari$ '\n")
        os.mkfifo(home / "release-job")
        for trial in range(args.runs + 1):
            offset = trial % len(shells)
            for sh in shells[offset:] + shells[:offset]:
                row = sample(sh, home)
                if trial:
                    values[sh].append(row)
    for sh in shells:
        result = {"shell": str(sh), "samples_ms": values[sh]}
        for name, column in zip(("key", "notice"), zip(*values[sh])):
            ordered = sorted(column)
            result[name + "_median_ms"] = median(ordered)
            result[name + "_p95_ms"] = ordered[math.ceil(.95 * len(ordered)) - 1]
        print(json.dumps(result))


if __name__ == "__main__":
    main()
