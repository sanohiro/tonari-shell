"""Measure a real Tab picker from keystroke to visible result and verify the selected argument."""
import argparse
from collections import defaultdict
import math
import os
from pathlib import Path
import statistics
import tempfile
import time

from bench_history import PROMPT, read_until, terminal


def timed(fd, keys, marker):
    start = time.perf_counter_ns()
    os.write(fd, keys)
    read_until(fd, marker)
    return (time.perf_counter_ns() - start) / 1e6


def sample(shell, home, kind, filename, matched):
    result = home / "selection"
    result.unlink(missing_ok=True)
    timings = {}
    with terminal(shell, home, "/usr/bin:/bin", home / "rc", prompt=PROMPT) as fd:
        if kind == "files":
            line = b"save files/item-"
            os.write(fd, line)
            read_until(fd, PROMPT + line)
            timed(fd, b"\t", PROMPT + line)
            timings["Open file choices"] = timed(fd, b"\t", b"> files/item-00000")
            timings["Filter, many matches"] = timed(fd, b"item", b"> files/item-00000")
            timings["Filter, late match"] = timed(fd, b"-" + filename, b"> " + matched)
            expected = matched
        else:
            line = b"save --model "
            os.write(fd, line)
            read_until(fd, PROMPT + line)
            timings["History option, first Tab"] = timed(fd, b"\t", PROMPT + line)
            timings["History option, open choices"] = timed(fd, b"\t", b"  amber")
            expected = b"violet"
        os.write(fd, b"\r\r")
        read_until(fd, b"\x1b[?2004h")
        if result.read_bytes() != expected:
            raise ValueError(f"{shell}: incorrect {kind} selection")

    return timings


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shell", nargs="+", type=Path)
    parser.add_argument("--runs", type=int, default=31)
    parser.add_argument("--files", type=int, default=20000)
    args = parser.parse_args()
    if not 2 <= args.files <= 99999 or args.runs < 1:
        parser.error("use 2..99999 files and a positive number of trials")
    shells = [p.resolve() for p in args.shell]
    with tempfile.TemporaryDirectory() as temp:
        home = Path(temp)
        directory = home / "files"
        directory.mkdir()
        for i in range(args.files):
            (directory / f"item-{i:05}-report").touch()
        (home / "rc").write_text(f"PS1='{PROMPT.decode()}'\n"
                                 'save() { if [[ $1 == --model ]]; then shift; fi; printf %s "$1" > selection; }\n')
        journal = home / ".local/state/tonari/history"
        journal.parent.mkdir(parents=True)
        # Rebuild between trials because accepting the measured command appends a record.
        records = "".join(f"1\t1\t0\t{home}\tsave --model {value}\n" for value in
                          (["violet", "amber", "violet", "green"] * 500 + ["amber"]))
        filename = f"{args.files - 1:05}".encode()
        matched = b"files/item-" + filename + b"-report"
        for kind in ("files", "history"):
            times = defaultdict(lambda: defaultdict(list))
            for i in range(args.runs + 3):
                rotated = shells[i % len(shells):] + shells[:i % len(shells)]
                for shell in rotated:
                    journal.write_text(records if kind == "history" else "")
                    observed = sample(shell, home, kind, filename, matched)
                    if i >= 3:
                        for key, value in observed.items():
                            times[key][shell].append(value)
            for metric, by_shell in times.items():
                values = []
                for shell in shells:
                    points = sorted(by_shell[shell])
                    values.append(f"{statistics.median(points):.3f}/{points[math.ceil(.95*len(points))-1]:.3f}")
                print(metric, *values, sep=" | ", flush=True)


if __name__ == "__main__":
    main()
