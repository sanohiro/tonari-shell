"""Measure keyboard-to-PTY latency for completion and warm history suggestions."""

import argparse
from collections import defaultdict
import math
import os
from pathlib import Path
import re
import statistics
import tempfile
import time

from bench_history import PROMPT, read_until, terminal


def timed_keys(fd, keys, marker):
    start = time.perf_counter_ns()
    os.write(fd, keys)
    read_until(fd, marker)
    return (time.perf_counter_ns() - start) / 1e6


def completion(shell, home, bindir, rc, line):
    with terminal(shell, home, bindir, rc, prompt=PROMPT) as fd:
        os.write(fd, line)
        read_until(fd, line)
        elapsed = timed_keys(fd, b"\t@", b"@")
        # Readline and Tonari use different incremental redraws. Force a full
        # redraw outside the timed interval to verify that Tab preceded '@'.
        os.write(fd, b"\x0c")
        read_until(fd, PROMPT + line + b"-0@")
        return elapsed


def listing(shell, home, bash, expected, prefix):
    rc = home / ("bashrc" if shell == bash else "rc")
    line = b": " + prefix
    with terminal(shell, home, home, rc, prompt=PROMPT) as fd:
        os.write(fd, line)
        read_until(fd, line)
        os.write(fd, b"\t")
        read_until(fd, b"\a" if shell == bash else PROMPT + line)
        start = time.perf_counter_ns()
        os.write(fd, b"\t@")
        output = read_until(fd, b"@")
        elapsed = (time.perf_counter_ns() - start) / 1e6
        if set(re.findall(rb"item-\d{5}\.dat", output)) != expected:
            raise ValueError(f"incomplete listing from {shell}")
        os.write(fd, b"\x0c")
        read_until(fd, PROMPT + line + b"@")
        return elapsed


def typing(shell, home, bindir, metrics):
    with terminal(shell, home, bindir) as fd:
        os.write(fd, b"p")
        read_until(fd, b"\x1b[2mrintf entry-099999")
        for case, word in [("recent hint", "printf entry-"), ("no hint", "zebra42雪abc")]:
            os.write(fd, b"\x15")
            read_until(fd, b"\r\x1b[J" + PROMPT)
            line = bytearray(PROMPT)
            for char in word:
                key = char.encode()
                line.extend(key)
                metrics[case].append(timed_keys(fd, key, line))
        os.write(fd, b"\x15")
        read_until(fd, b"\r\x1b[J" + PROMPT)
        start = time.perf_counter_ns()
        os.write(fd, b"q")
        output = read_until(fd, PROMPT + b"q")
        metrics["oldest hint: typed key"].append((time.perf_counter_ns() - start) / 1e6)
        hint = b"\x1b[2muartz oldest"
        if hint not in output:
            # The marker may straddle two PTY reads.
            read_until(fd, hint, initial=output)
        metrics["oldest hint: visible suggestion"].append((time.perf_counter_ns() - start) / 1e6)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shell", nargs="+", type=Path, help="Tonari executable(s)")
    parser.add_argument("--bash", type=Path, help="Bash reference for the Tab cases")
    parser.add_argument("--runs", type=int, default=30)
    parser.add_argument("--listing", action="store_true", help="measure the second Tab and full listing")
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("runs must be positive")
    shells = [p.resolve() for p in args.shell]
    bash = args.bash.resolve() if args.bash else None
    if bash:
        shells.append(bash)
    with tempfile.TemporaryDirectory() as scratch:
        home = Path(scratch)
        bindir = home / "files"
        bindir.mkdir()
        for i in range(10_000):
            (bindir / f"item-{i:05d}.dat").touch(mode=0o755)
        if args.listing:
            expected = {p.name.encode() for p in bindir.iterdir()}
            few = home / "few"
            few.mkdir()
            small = {name for name in expected if name.startswith(b"item-0000")}
            for name in sorted(small):
                (few / name.decode()).touch(mode=0o755)
            for folder in (few, bindir):
                prompt = f"PS1='{PROMPT.decode()}'\n"
                (folder / "rc").write_text(prompt)
                (folder / "bashrc").write_text(prompt + "bind 'set completion-query-items 0'\n"
                                               "bind 'set page-completions off'\n")
            cases = [(few, small, b"item-0000"), (bindir, expected, b"item-0")]
        else:
            typing_home = home / "typing"
            journal = typing_home / ".local/state/tonari/history"
            journal.parent.mkdir(parents=True)
            with journal.open("w") as f:
                f.write("1\t1\t0\t/tmp\tquartz oldest\n")
                for i in range(1, 100_000):
                    f.write(f"1\t1\t0\t/tmp\tprintf entry-{i:06d}\n")
            rc = home / "rc"
            rc.write_text(f"PS1='{PROMPT.decode()}'\ncomplete -f demo\n")
        results = {s: defaultdict(list) for s in shells}
        for i in range(3 + args.runs):
            shift = i % len(shells)
            for shell in shells[shift:] + shells[:shift]:
                metrics = results[shell] if i >= 3 else defaultdict(list)
                if args.listing:
                    for folder, names, prefix in cases:
                        elapsed = listing(shell, folder, bash, names, prefix)
                        metrics[f"list {len(names)}"].append(elapsed)
                    continue
                if shell != bash:
                    typing(shell, typing_home, bindir, metrics)
                for case, line in [("files", b": files/item"), ("complete -f", b"demo files/item"),
                                   ("PATH", b"item")]:
                    metrics[case].append(completion(shell, home, bindir, rc, line))
        print("shell | case | samples | median / p95 (ms)")
        for shell, metrics in results.items():
            for case, values in metrics.items():
                p95 = sorted(values)[math.ceil(.95 * len(values)) - 1]
                print(f"{shell} | {case} | {len(values)} | {statistics.median(values):.3f} / {p95:.3f}")


if __name__ == "__main__":
    main()
