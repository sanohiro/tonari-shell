"""Check shared seekable stdin at several line lengths and buffer capacities."""
import argparse
import json
from pathlib import Path
import random
from statistics import median
import subprocess
import tempfile
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ("bash", "current", "small", "large"):
        p.add_argument("--" + name, type=Path, required=True)
    shells = vars(p.parse_args())
    workloads = {
        "short": "i=1\nx=$i\n" * 750 + "echo $x\n",
        "medium": ("v=" + "x" * 600 + "\n") * 1000 + "echo ${#v}\n",
        "long": ("v=" + "x" * 16000 + "\n") * 100 + "echo ${#v}\n",
        "shared read": "i=1\nread -r x\ndata\nprintf '%s\\n' \"$x\"\n" * 300,
    }
    with tempfile.TemporaryDirectory() as tmp:
        directory = Path(tmp)
        env = {"PATH": "/usr/bin:/bin", "HOME": tmp, "LC_ALL": "C"}
        for label, binary in shells.items():
            path = directory / label
            path.mkdir()
            (path / "sh").symlink_to(binary.resolve())
        for name, source in workloads.items():
            path = directory / "input"
            path.write_text(source)
            samples = {k: [] for k in shells}
            rng = random.Random(41)
            expected = None
            for iteration in range(18):
                order = list(shells)
                rng.shuffle(order)
                for label in order:
                    with path.open("rb") as stdin:
                        start = time.monotonic_ns()
                        proc = subprocess.run([directory / label / "sh"], stdin=stdin, env=env, cwd=directory,
                                              capture_output=True, timeout=10)
                        ms = (time.monotonic_ns()-start)/1e6
                    if proc.returncode or proc.stderr or (expected is not None and expected != proc.stdout):
                        raise RuntimeError(f"{name}/{label}: unexpected result")
                    expected = proc.stdout
                    if iteration:
                        samples[label].append(ms)
            print(json.dumps({"case": name, "medians_ms": {k: round(median(v), 3) for k,v in samples.items()},
                              "samples_ms": samples}), flush=True)


if __name__ == "__main__":
    main()
