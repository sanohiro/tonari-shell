"""Time literal file captures in an already running shell and check the complete value."""

import argparse
import math
from pathlib import Path
import select
import statistics
import subprocess
import tempfile
import time


CASES = [(0, 24), (5, 256), (4096, 128), (262144, 16), (4194304, 4), (16777216, 1)]


def wait_marker(pipe, marker):
    if not select.select([pipe], [], [], 90)[0] or pipe.readline() != marker:
        raise RuntimeError(f"shell did not reach {marker!r}")


def sample(binary, work, expected):
    output = work / "out"
    output.unlink(missing_ok=True)
    env = {"HOME": str(work), "LC_ALL": "C", "PATH": "/usr/bin:/bin"}
    argv = [binary, "--noprofile", "--norc", "-c",
            'printf "ready\\n"; IFS= read -r gate && . ./work.sh']
    with subprocess.Popen(argv, env=env, cwd=work, stdin=subprocess.PIPE,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE) as child:
        try:
            wait_marker(child.stdout, b"ready\n")
            start = time.perf_counter_ns()
            child.stdin.write(b"\n")
            child.stdin.flush()
            wait_marker(child.stdout, b"done\n")
            elapsed = (time.perf_counter_ns() - start) / 1e6
            out, err = child.communicate(timeout=90)
            if (child.returncode, out, err) != (0, b"", b"") or output.read_bytes() != expected:
                raise ValueError(f"{binary}: incorrect file capture (status {child.returncode}, {err!r})")
        finally:
            if child.poll() is None:
                child.kill()
    return elapsed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shell", nargs="+", type=Path)
    parser.add_argument("--runs", type=int, default=17)
    parser.add_argument("--case", action="append", type=int, nargs=2, metavar=("BYTES", "READS"))
    args = parser.parse_args()
    cases = args.case or CASES
    if args.runs < 1 or any(size < 0 or count < 1 for size, count in cases):
        parser.error("expected nonnegative sizes, positive reads and positive runs")
    bins = [p.resolve() for p in args.shell]
    print("bytes | reads |", " | ".join(f"{p} median/p95 ms" for p in bins), flush=True)
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        for size, count in cases:
            unit = b"Tonari123ABCxyz!"
            data = (unit * (size // len(unit) + 1))[:size]
            if size >= 2:
                data = data[:-2] + b"\n\n"
            (work / "data").write_bytes(data)
            expected = data.rstrip(b"\n")
            (work / "work.sh").write_text(f'for ((i=0;i<{count};i++)); do v=$(<data); done\n'
                                          'printf "done\\n"\nprintf %s "$v" >out\n')
            times = {binary: [] for binary in bins}
            for i in range(args.runs + 3):
                offset = i % len(bins)
                for binary in bins[offset:] + bins[:offset]:
                    elapsed = sample(binary, work, expected)
                    if i >= 3:
                        times[binary].append(elapsed)
            p95 = math.ceil(0.95 * args.runs) - 1
            print(size, count, *(f"{statistics.median(v):.3f}/{sorted(v)[p95]:.3f}"
                                 for v in times.values()), sep=" | ", flush=True)


if __name__ == "__main__":
    main()
