"""Time sixteen foreground executables in a ready, job-controlling shell."""
import argparse
import os
from pathlib import Path
import statistics
import tempfile
import time

from bench_history import PROMPT, read_until, terminal


def sample(shell):
    with tempfile.TemporaryDirectory() as home, terminal(shell, home, "/usr/bin:/bin", prompt=PROMPT) as fd:
        start = time.perf_counter_ns()
        os.write(fd, b'for ((i=0;i<16;i++)); do /usr/bin/true || { echo fail=$?; break; }; done; echo count=$i\n')
        try:
            read_until(fd, b'count=16\r\n')
        except TimeoutError as error:
            raise RuntimeError(f"{shell}: {error}") from error
        return (time.perf_counter_ns() - start) / 1e6


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shell", nargs="+", type=Path)
    parser.add_argument("--runs", type=int, default=31)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("runs must be positive")
    shells = [p.resolve() for p in args.shell]
    samples = {p: [] for p in shells}
    for i in range(args.runs + 3):
        offset = i % len(shells)
        for shell in shells[offset:] + shells[:offset]:
            elapsed = sample(shell)
            if i >= 3:
                samples[shell].append(elapsed)
    for shell, times in samples.items():
        print(f"{shell} median {statistics.median(times):.3f} ms")


if __name__ == "__main__":
    main()
