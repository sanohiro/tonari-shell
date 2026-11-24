"""Time sixteen foreground executables in a ready, job-controlling shell."""
import argparse
import os
import re
from pathlib import Path
import statistics
import tempfile
import time

from bench_history import PROMPT, read_until, terminal


def sample(shell):
    with tempfile.TemporaryDirectory() as home, terminal(shell, home, "/usr/bin:/bin", prompt=PROMPT) as fd:
        start = time.perf_counter_ns()
        os.write(fd, b'rc=0; for ((i=0;i<16;i++)); do /usr/bin/true || { rc=$?; break; }; done; '
                     b'printf "\\nresult=%d/%d %s%s\\n" "$i" "$rc" _E ND_\n')
        output = read_until(fd, b' _END_\r\n')
        elapsed = (time.perf_counter_ns() - start) / 1e6
        result = re.search(rb'\r\nresult=(\d+)/(\d+) _END_\r\n', output)
        if result is None:
            raise RuntimeError(f"{shell}: malformed foreground result {output[-100:]!r}")
        return elapsed, result.groups()



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shell", nargs="+", type=Path)
    parser.add_argument("--previous", type=Path, help="report and retry invalid exit statuses in the historical control")
    parser.add_argument("--runs", type=int, default=31)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("runs must be positive")
    shells = [p.resolve() for p in args.shell]
    previous = args.previous.resolve() if args.previous else None
    if previous:
        shells.insert(1, previous)
    samples = {p: [] for p in shells}
    for i in range(args.runs + 3):
        offset = i % len(shells)
        for shell in shells[offset:] + shells[:offset]:
            for attempt in range(3):
                elapsed, result = sample(shell)
                if result == (b"16", b"0"):
                    break
                detail = f"{shell} stopped after {result[0].decode()} launches with status {result[1].decode()}"
                if shell != previous or attempt == 2:
                    raise RuntimeError(detail)
                print(f"Invalid previous-build sample: {detail}", flush=True)
            if i >= 3:
                samples[shell].append(elapsed)
    for shell, times in samples.items():
        print(f"{shell} median {statistics.median(times):.3f} ms")


if __name__ == "__main__":
    main()
