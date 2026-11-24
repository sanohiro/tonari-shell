"""Time interactive argument recall and editing on short and long lines."""
import argparse
import math
import os
from pathlib import Path
import statistics
import tempfile
import time

from bench_history import PROMPT, read_until, terminal


def send(fd, keys, visible):
    start = time.perf_counter_ns()
    os.write(fd, keys)
    read_until(fd, visible)
    return (time.perf_counter_ns() - start) / 1e6


def sample(shell, home, case):
    with terminal(shell, home, "/usr/bin:/bin", home / "rc", PROMPT) as fd:
        if case.startswith("Alt-."):
            os.write(fd, b'v=')
            read_until(fd, b'v=')
            ms = send(fd, b"\x1b.", b"ENDarg")
            os.write(fd, b'\rprintf "<result:%s>\\n" "$v"\r')
            read_until(fd, b"<result:ENDarg>\r\n")
            return (ms,)
        size = (home / "seed").stat().st_size - len("v=TAGend\n")
        os.write(fd, b"\x1b[A")
        read_until(fd, b"d")
        # A paste is one standalone edit regardless of packet boundaries.
        append = send(fd, b"\x1b[200~" + b"Z" * 32 + b"!\x1b[201~", b"!")
        undo = send(fd, b"\x1f" + b"Q", b"Q")
        os.write(fd, b'\rprintf "<result:%s:%s>\\n" "${#v}" "${v: -7}"\r')
        read_until(fd, f"<result:{size + 7}:TAGendQ>\r\n".encode())
        return append, undo


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shells", type=Path, nargs="+")
    parser.add_argument("--runs", type=int, default=21)
    args = parser.parse_args()
    shells = [s.resolve() for s in args.shells]
    with tempfile.TemporaryDirectory() as d:
        home = Path(d)
        (home / "rc").write_text("PS1='tonari$ '\nunset HISTFILE\nhistory -r seed\n")
        for case, seed in [("Alt-. short", ': a ENDarg'), ("Alt-. many", ': ' + 'item ' * 4096 + "'ENDarg'"),
                           ("short line", "v=123TAGend"), ("1 MiB line", "v=" + "a" * 1048576 + "TAGend")]:
            (home / "seed").write_text(seed + "\n")
            by_shell = {sh: [] for sh in shells}
            for trial in range(args.runs + 2):
                pos = trial % len(shells)
                for sh in shells[pos:] + shells[:pos]:
                    values = sample(sh, home, case)
                    if trial >= 2:
                        by_shell[sh].append(values)
            labels = [case] if case.startswith("Alt-.") else ["Paste into " + case, "Undo + type into " + case]
            for i, label in enumerate(labels):
                row = []
                for sh in shells:
                    v = sorted(t[i] for t in by_shell[sh])
                    row.append(f"{statistics.median(v):.3f}/{v[math.ceil(.95 * len(v)) - 1]:.3f}")
                print(label, *row, sep=" | ", flush=True)


if __name__ == "__main__":
    main()
