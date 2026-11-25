"""Repeat shell-owned work and verify the visible result outside the timer."""
import argparse
import os
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile
import termios
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from bench_history import PROMPT, read_until, terminal


def script(shell, home, name):
    sources = {
        "dynamic-index": ("x=1; y=2; for ((i=0;i<6000;i++)); do a[$x$y]=ok; done; "
                          "printf '<%s>' \"${a[12]}\"", b"<ok>"),
        "indexed-append": ("a=(a); for ((i=0;i<6000;i++)); do a[0]+=x; done; "
                           "printf '<%s>' \"${#a[0]}\"", b"<6001>"),
        "empty-fc": ("for ((i=0;i<3000;i++)); do fc -ln || exit 1; done; printf ok", b"ok"),
    }
    source, expected = sources[name]
    start = time.perf_counter_ns()
    p = subprocess.run([shell, "-c", source], cwd=home, env={"HOME": str(home), "PATH": "/usr/bin:/bin"},
                       capture_output=True, timeout=6)
    elapsed = time.perf_counter_ns() - start
    if (p.returncode, p.stdout, p.stderr) != (0, expected, b""):
        return None
    return elapsed


def edit(shell, home, name):
    with terminal(shell, home, "/usr/bin:/bin", prompt=PROMPT) as fd:
        if name == "resize-accept":
            termios.tcsetwinsize(fd, (41, 103))
            packet = b"printf '<%sx%s>\\n' \"$LINES\" \"$COLUMNS\"\r"
            marker = b"<41x103>\r\n"
        else:
            os.write(fd, b"v=abcdef")
            read_until(fd, b"abcdef")
            moves = b"\x02\x06" if name == "control-motion" else b"\x1b[D\x1b[C"
            packet = moves * 160 + b"\x02Z\x05; printf '<%s>\\n' \"$v\"\r"
            marker = b"<abcdeZf>\r\n"
        start = time.perf_counter_ns()
        os.write(fd, packet)
        read_until(fd, marker)
        elapsed = time.perf_counter_ns() - start
        return elapsed


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("shells", nargs="+", type=lambda x: Path(x).resolve())
    p.add_argument("--runs", type=int, default=19)
    args = p.parse_args()
    with tempfile.TemporaryDirectory() as d:
        home = Path(d)
        for name in ("dynamic-index", "indexed-append", "empty-fc", "control-motion",
                     "arrow-motion", "resize-accept"):
            run = edit if name in ("control-motion", "arrow-motion", "resize-accept") else script
            samples = {s: [] for s in args.shells}
            excluded = set()
            for trial in range(args.runs + 1):
                offset = trial % len(args.shells)
                for sh in args.shells[offset:] + args.shells[:offset]:
                    if sh in excluded:
                        continue
                    try:
                        ns = run(sh, home, name)
                    except TimeoutError:
                        ns = None
                    if ns is None:
                        excluded.add(sh)
                    elif trial:
                        samples[sh].append(ns / 1e6)
            for sh in args.shells:
                v = sorted(samples[sh])
                print(name, sh, len(v), statistics.median(v) if sh not in excluded else "MISMATCH",
                      sep="\t", flush=True)


if __name__ == "__main__":
    main()
