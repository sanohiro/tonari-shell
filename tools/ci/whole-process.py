"""Interleave BEFORE, AFTER and Bash; report elapsed milliseconds at three sizes."""

import os
from pathlib import Path
import random
from statistics import median
import subprocess
import sys
import tempfile
import time


def shell_cases(n, size):
    def loop(body):
        return f"for ((i=0;i<{n};i++)); do {body}; done"

    yield "assignment pipeline", loop("x=ok | :") + "; echo $i", str(n), None
    yield "integer pipeline", ("declare -i x=5; count=0; " + loop("x=count++ | :")
                               + '; echo "$count:$x"'), "0:5", f"{n}:5"
    yield "background external, monitor", ("set -m; " + loop('/usr/bin/true & wait "$!"')
                                          + "; echo $i"), str(n), None
    yield "job insertion and wait", loop(": & :") + "; wait; echo $i", str(n), None
    yield "descriptor restore", "exec 7>output; " + loop("echo abc >&7") + "; wc -l <output", str(n), None
    yield "command substitution regular file", 's=$(<payload); echo "${#s}"', str(size), None
    yield "printf to regular file", ('s=$(<payload); printf "%s\\n" "$s" >output; wc -c <output'), str(size + 1), None
    yield "printf pipe", 's=$(<payload); printf "%s" "$s" | wc -c', str(size), None
    yield "early reader", ('s=$(<payload); printf "%s" "$s" | "$1" reader.py early; '
                           'echo "${PIPESTATUS[*]}"'), "x\n141 0", None
    yield "slow reader", ('s=$(<payload); printf "%s" "$s" | "$1" reader.py slow; '
                          'echo "${PIPESTATUS[*]}"'), f"{size}\n0 0", None
    yield "fixed byte count on a pipe", ('cat payload | { IFS= read -rN "$2" s; echo "${#s}"; }'), str(size), None
    yield "shared stream, delimited read", ('cat line | { IFS= read -r s; echo "${#s}"; cat; }'), f"{n * 512}\ntail", None


def report(name, size, programs, sample, runs, rng):
    measurements = {label: [] for label in programs}
    for index in range(runs + 1):
        order = list(programs)
        rng.shuffle(order)
        for label in order:
            start = time.perf_counter_ns()
            sample(label, programs[label])
            elapsed = (time.perf_counter_ns() - start) / 1e6
            if index:
                measurements[label].append(elapsed)
    for label, times in measurements.items():
        print(f"{name}\t{size}\t{label}\t{median(times):.3f}\t{min(times):.3f}\t{max(times):.3f}", flush=True)


def main():
    if len(sys.argv) != 4:
        raise SystemExit("usage: whole-process.py BEFORE AFTER BASH")
    shells = dict(zip(("before", "after", "bash"), (Path(s).resolve() for s in sys.argv[1:])))
    runs = int(os.environ.get("BENCH_RUNS", "7"))
    rng = random.Random(1754509)
    print("case\tN\tprogram\tmedian_ms\tmin_ms\tmax_ms", flush=True)
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        env = dict(PATH="/usr/bin:/bin", LC_ALL="C", HOME=tmp)
        consumer = """import os, sys, time
if sys.argv[1] == 'early':
    print(os.read(0, 1).decode())
else:
    count = 0
    while block := os.read(0, 16384):
        count += len(block)
        time.sleep(.00025)
    print(count)
"""
        (root / "reader.py").write_text(consumer)
        for n in (8, 16, 32):
            size = n * 32768
            payload = root / "payload"
            payload.write_bytes(b"x" * size)
            (root / "line").write_bytes(b"x" * (n * 512) + b"\ntail\n")
            for case, source, expected, earlier in shell_cases(n, size):
                def shell_sample(label, shell):
                    r = subprocess.run([shell, "--noprofile", "--norc", "-c", source,
                                        "benchmark", sys.executable, str(size)],
                                       cwd=tmp, env=env, capture_output=True, timeout=10)
                    valid = [expected]
                    if label == "before" and earlier is not None:
                        valid.append(earlier)
                    output = r.stdout.decode().strip()
                    if r.returncode or r.stderr or output not in valid:
                        raise RuntimeError(f"{case} N={n} {label}: {r.returncode, output, r.stderr} expected {valid}")

                report(case, n, shells, shell_sample, runs, rng)

            def spawn_control(_label, _):
                for _ in range(n):
                    subprocess.run(["/usr/bin/true"], cwd=root, env=env, check=True, timeout=5)

            report("spawn/wait OS control", n, {"native": None}, spawn_control, runs, rng)

            for mode in ("early", "slow"):
                def pipe_control(_label, _):
                    with subprocess.Popen(["/bin/cat", payload], cwd=root, env=env,
                                          stdout=subprocess.PIPE, stderr=subprocess.PIPE) as writer:
                        reader = subprocess.Popen([sys.executable, "reader.py", mode], cwd=root,
                                                  env=env, stdin=writer.stdout,
                                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                        writer.stdout.close()
                        with reader:
                            out, err = reader.communicate(timeout=8)
                        status = writer.wait(timeout=5)
                        if (reader.returncode or err or writer.stderr.read()
                            or out.strip() != (b"x" if mode == "early" else str(size).encode())
                            or status != (-13 if mode == "early" else 0)):
                            raise RuntimeError(f"native {mode}: {reader.returncode, status, out, err}")

                report(f"{mode} OS pipe control", n, {"native": None}, pipe_control, runs, rng)


if __name__ == "__main__":
    main()
