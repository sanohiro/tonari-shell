"""Time complete redirects, pipes, capture and here-documents. Check every output byte."""

import argparse
from pathlib import Path
import statistics
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shell", nargs="+", type=Path)
    parser.add_argument("--case", action="append", choices=["redirect", "pipe", "capture", "external capture", "quoted heredoc", "read -N"])
    parser.add_argument("--runs", type=int, default=15)
    parser.add_argument("--sizes", type=int, nargs="+", default=[0, 1, 4096, 262144, 4194304])
    args = parser.parse_args()
    if args.runs < 1 or min(args.sizes) < 0:
        parser.error("expected positive runs and nonnegative sizes")
    shells = [p.resolve() for p in args.shell]
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        env = {"HOME": directory, "PATH": "/usr/bin:/bin", "LC_ALL": "C"}
        for size in args.sizes:
            raw = (bytes(range(256)) * (size // 256 + 1))[:size]
            text = (b"abc12345" * (size // 8 + 1))[:size]
            (root / "input").write_bytes(raw)
            (root / "text").write_bytes(text + b"\n\n")
            document = text + b"\n" if size else b""
            for name, script, expected in (
                ("redirect", b"/bin/cat <input >out\n", raw),
                ("pipe", b"/bin/cat <input | /bin/cat | /bin/cat >out\n", raw),
                ("capture", b'v=$(<text); printf %s "$v" >out\n', text),
                ("external capture", b'v=$(/bin/cat text); printf %s "$v" >out\n', text),
                ("read -N", f'/bin/cat text | {{ IFS= read -r -N {size} v; printf %s "$v"; /bin/cat; }} >out\n'.encode(), text + b"\n\n"),
                ("quoted heredoc", b"/bin/cat >out <<'END'\n" + document + b"END\n", document),
            ):
                if args.case and name not in args.case:
                    continue
                (root / "script").write_bytes(script)
                times = {s: [] for s in shells}
                for i in range(args.runs + 2):
                    shift = i % len(shells)
                    for shell in shells[shift:] + shells[:shift]:
                        (root / "out").unlink(missing_ok=True)
                        start = time.perf_counter_ns()
                        subprocess.run([shell, "--noprofile", "--norc", "script"], cwd=root,
                                       stdin=subprocess.DEVNULL, env=env, check=True)
                        elapsed = (time.perf_counter_ns() - start) / 1e6
                        if (root / "out").read_bytes() != expected:
                            raise ValueError(f"{shell} {name} {size}: incorrect output")
                        if i >= 2:
                            times[shell].append(elapsed)
                print(size, name, *(f"{statistics.median(v):.3f}" for v in times.values()),
                      sep=" | ", flush=True)


if __name__ == "__main__":
    main()
