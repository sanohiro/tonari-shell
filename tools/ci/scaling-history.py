"""Measure Enter and Up after filling an erasedups history on a PTY."""
import os
from pathlib import Path
import statistics
import sys
import tempfile
import time

from bench_history import PROMPT, read_until, terminal


def sample(shell, root, rc):
    with terminal(shell, root, "/usr/bin:/bin", rc, PROMPT) as fd:
        # This command is already present near the middle of the history.
        line = b": entry-5000"
        os.write(fd, line)
        read_until(fd, line)
        start = time.perf_counter_ns()
        os.write(fd, b"\r")
        read_until(fd, PROMPT)
        entered = time.perf_counter_ns()
        os.write(fd, b"\x1b[A")
        read_until(fd, line)
        return [(entered - start) / 1e6, (time.perf_counter_ns() - entered) / 1e6]


def main():
    shells = [Path(p).resolve() for p in sys.argv[1:4]]
    runs = int(os.environ.get("BENCH_RUNS", "15"))
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "seed").write_text("".join(f": entry-{i}\n" for i in range(10000)))
        rc = root / "rc"
        rc.write_text("HISTSIZE=20000; HISTFILE=/dev/null; HISTCONTROL=erasedups; history -c; history -r seed\n"
                      "PS1='tonari$ '\n")
        results = [[] for _ in shells]
        for trial in range(3 + runs):
            for j in range(3):
                k = (trial+j) % 3
                v = sample(shells[k], root, rc)
                if trial >= 3:
                    results[k].append(v)
        for i, name in enumerate(("Enter to prompt", "Up to recalled command")):
            print(name, *[f"{statistics.median(v[i] for v in rows):.3f}" for rows in results], sep=" | ")


if __name__ == "__main__":
    main()
