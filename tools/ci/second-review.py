"""Time the complete file and stdin paths on native hosts, checking every result."""
import argparse
import os
from pathlib import Path
import random
import statistics
import subprocess
import tempfile
import time


def workloads(n):
    inner = "".join(f"  if [ {i} -ge 0 ]; then v={i}; fi\n" for i in range(n))
    compound = f"if true; then\n{inner}fi; printf '%s' \"$v\"\n"
    yield "nested-if-file", compound, str(n - 1), False
    yield "nested-if-stdin", compound, str(n - 1), True
    cases = " ".join(f"case x in x) v={i};; esac;" for i in range(n))
    yield "case-substitution", f"x=$({cases} echo \"$v\"); printf '%s' \"$x\"\n", str(n - 1), False
    value = "é" * n
    yield "utf8-substitution", f"s='{value}'; s=${{s//é?/}}; printf '<%s>' \"$s\"\n", "<>", False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("before", "after", "bash"):
        parser.add_argument(name, type=Path)
    parser.add_argument("--runs", type=int, default=7)
    parser.add_argument("--sizes", type=int, nargs="+", default=(256, 512, 1024))
    args = parser.parse_args()
    programs = {k: getattr(args, k).resolve() for k in ("before", "after", "bash")}
    order = random.Random(20261128)
    print("workload N " + " ".join(programs) + " (median wall ms)")
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        env = {"HOME": tmp, "LC_ALL": "en_US.UTF-8" if os.uname().sysname == "Darwin" else "C.UTF-8",
               "PATH": os.defpath}
        path = work / "input.sh"
        for size in args.sizes:
            for name, source, expected, stdin in workloads(size):
                path.write_text(source, encoding="utf-8")
                samples = {k: [] for k in programs}
                for i in range(args.runs + 1):
                    keys = list(programs)
                    order.shuffle(keys)
                    for k in keys:
                        with path.open("rb") as f:
                            start = time.perf_counter_ns()
                            r = subprocess.run([programs[k], *([] if stdin else [path])],
                                               stdin=f if stdin else subprocess.DEVNULL, cwd=work,
                                               env=env, capture_output=True, timeout=30)
                            elapsed = (time.perf_counter_ns() - start) / 1e6
                        if (r.returncode, r.stdout, r.stderr) != (0, expected.encode(), b""):
                            raise ValueError((k, name, size, r.returncode, r.stdout, r.stderr))
                        if i:
                            samples[k].append(elapsed)
                print(name, size, *(f"{statistics.median(v):.3f}" for v in samples.values()),
                      flush=True)


if __name__ == "__main__":
    main()
