"""Measure history and directory navigation through the visible response in a PTY."""
import argparse
from collections import defaultdict
import math
import os
from pathlib import Path
import statistics
import tempfile
import time

from bench_history import PROMPT, read_until, terminal

UP, DOWN = b"\x1b[A", b"\x1b[B"
LEFT, RIGHT = b"\x1b[1;3D", b"\x1b[1;3C"


def key(fd, packet, marker):
    start = time.perf_counter_ns()
    os.write(fd, packet)
    read_until(fd, marker)
    return (time.perf_counter_ns() - start) / 1e6


def history(shell, home, bash):
    with terminal(shell, home, "/usr/bin:/bin", home / ("bashrc" if bash else "rc"), PROMPT) as fd:
        os.write(fd, b"v=token!\x02")
        read_until(fd, b"!")
        times = [key(fd, UP, b"LAST"), key(fd, UP, b"FIR"),
                 key(fd, DOWN, b"LA"), key(fd, UP, b"FIR")]
        os.write(fd, b'\rprintf "<result:%s>\\n" "$v"\r')
        read_until(fd, b"<result:tokenFIRST>\r\n")
        return dict(zip(("Recent match", "Distant match", "Forward revisit", "Backward revisit"), times))


def directory(shell, home):
    with terminal(shell, home, "/usr/bin:/bin", home / "dirrc", b"visit-B$ ") as fd:
        return {"Back to directory": key(fd, LEFT, b"visit-A$ "),
                "Forward to directory": key(fd, RIGHT, b"visit-B$ ")}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shells", nargs="+", type=Path)
    parser.add_argument("--bash", type=Path)
    parser.add_argument("--entries", type=int, default=100000)
    parser.add_argument("--runs", type=int, default=21)
    args = parser.parse_args()
    if args.entries < 3 or args.runs < 1:
        parser.error("need at least three entries and one trial")
    shells = [p.resolve() for p in args.shells]
    bash = args.bash.resolve() if args.bash else None
    with tempfile.TemporaryDirectory() as tmp:
        home = Path(tmp)
        with (home / "seed").open("w") as f:
            f.write("v=tokenFIRST\n")
            for i in range(args.entries - 2):
                f.write(f"v=other-{i}\n")
            f.write("v=tokenLAST\n")
        rc = f"PS1='tonari$ '\nunset HISTFILE\nHISTSIZE={args.entries + 10}\nhistory -r seed\n"
        (home / "rc").write_text(rc)
        (home / "bashrc").write_text(rc + """bind '"\\e[A":history-search-backward'
bind '"\\e[B":history-search-forward'
bind 'set enable-bracketed-paste on'
""")
        root = home.joinpath(*("parent" for _ in range(12)))
        for name in ["visit-A", "visit-B"]:
            (root / name).mkdir(parents=True)
        (home / "dirrc").write_text(f"PS1='\\W$ '\nunset HISTFILE\ncd '{root}/visit-A'\ncd ../visit-B\n")
        for bench in (history, directory):
            candidates = shells + ([bash] if bash and bench == history else [])
            values = defaultdict(lambda: defaultdict(list))
            for trial in range(args.runs + 2):
                pos = trial % len(candidates)
                for sh in candidates[pos:] + candidates[:pos]:
                    result = history(sh, home, sh == bash) if bench == history else directory(sh, home)
                    if trial >= 2:
                        for label, value in result.items():
                            values[label][sh].append(value)
            for label, by_shell in values.items():
                row = []
                for sh in candidates:
                    v = sorted(by_shell[sh])
                    row.append(f"{statistics.median(v):.3f}/{v[math.ceil(.95 * len(v)) - 1]:.3f}")
                print(label, *row, sep=" | ", flush=True)


if __name__ == "__main__":
    main()
