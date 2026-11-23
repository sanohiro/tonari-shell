"""Measure external pipe endpoints, checking byte counts and early-close statuses."""
import argparse
import math
from pathlib import Path
import re
import statistics
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shells", nargs="+", type=Path)
    parser.add_argument("--runs", type=int, default=31)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("runs must be positive")
    shells = [p.resolve() for p in args.shells]
    cases = [
        ("32 short pipes", "for ((i=0;i<32;i++)); do /usr/bin/printf x | /bin/cat || exit; done", b"x"*32, None),
        ("early close", "/usr/bin/yes | /usr/bin/head -n 1; printf '%s:%s' \"$?\" \"${PIPESTATUS[*]}\"", b"y\n141:141 0", None),
    ]
    for size in (64, 262144, 4194304, 268435456):
        block = min(size, 65536)
        producer = f"/bin/dd if=/dev/zero bs={block} count={size//block} 2>/dev/null"
        cases.append((f"{size} bytes dd", f"{producer} | /bin/dd of=/dev/null bs={block}", b"", size))
        cases.append((f"{size} bytes wc", f"{producer} | /usr/bin/wc -c", str(size).encode(), None))
    with tempfile.TemporaryDirectory() as home:
        env = dict(PATH="/usr/bin:/bin", LC_ALL="C", HOME=home)
        for label, script, expected, count in cases:
            times = {s: [] for s in shells}
            for turn in range(args.runs + 3):
                offset = turn % len(shells)
                for shell in shells[offset:] + shells[:offset]:
                    start = time.monotonic_ns()
                    proc = subprocess.run([shell, "--noprofile", "--norc", "-o", "pipefail", "-c", script],
                                          cwd=home, env=env, stdin=subprocess.DEVNULL, capture_output=True,
                                          timeout=30, check=True)
                    elapsed = (time.monotonic_ns() - start) / 1e6
                    counted = re.findall(rb"^(\d+) bytes\b.*(?:copied|transferred)", proc.stderr, re.M)
                    stderr_ok = not proc.stderr if count is None else counted == [str(count).encode()]
                    if proc.stdout.strip() != expected or not stderr_ok:
                        raise ValueError((shell, label, proc.stdout, proc.stderr))
                    if turn >= 3:
                        times[shell].append(elapsed)
            for shell, values in times.items():
                print(label, shell.name, f"{statistics.median(values):.3f}", f"{sorted(values)[math.ceil(.95*len(values))-1]:.3f}",
                      sep=" | ", flush=True)


if __name__ == "__main__":
    main()
