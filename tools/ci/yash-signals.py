"""Split shell startup from signal delivery and process reaping on a native host."""
import argparse
import json
import os
from pathlib import Path
import resource
import signal
from statistics import median
import subprocess
import tempfile
import time


def sample(sh, signame, directory):
    before = time.monotonic_ns()
    with subprocess.Popen([sh, "-c", "printf .; kill -s " + signame + " $$"], cwd=directory,
                          env={"LC_ALL": "C", "HOME": str(directory), "PATH": "/usr/bin:/bin"},
                          stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                          stderr=subprocess.DEVNULL) as child:
        signal.alarm(12)
        try:
            marker = os.read(child.stdout.fileno(), 1)
            after_marker = time.monotonic_ns()
            # A timeout-based wait can add the interval between polls.
            code = child.wait()
            end = time.monotonic_ns()
        except TimeoutError:
            child.kill()
            raise
        finally:
            signal.alarm(0)
    if marker != b"." or code != -getattr(signal, "SIG" + signame):
        raise ValueError(f"{sh}/{signame}: expected marker and signal, got {marker!r}, {code}")
    return [(after_marker - before)/1e6, (end-after_marker)/1e6]


def alarm_timeout(signum, frame):
    raise TimeoutError("signal delivery timed out")


def run(directory, binaries):
    signal.signal(signal.SIGALRM, alarm_timeout)
    shells = {}
    for label, binary in binaries.items():
        path = directory / label / "sh"
        path.parent.mkdir()
        path.symlink_to(binary.resolve())
        shells[label] = path
    resource.setrlimit(resource.RLIMIT_CORE, (0, resource.getrlimit(resource.RLIMIT_CORE)[1]))
    pattern = Path("/proc/sys/kernel/core_pattern")
    for name in ("TERM", "FPE", "SEGV", "ABRT"):
        times = {k: [] for k in shells}
        for n in range(12):
            for k in (shells if n % 2 == 0 else reversed(shells)):
                value = sample(shells[k], name, directory)
                if n:
                    times[k].append(value)
        yield {"signal": name, "core_pattern": pattern.read_text().strip() if pattern.exists() else None,
               "core_soft_limit": 0, "median_ms_start_to_marker_and_marker_to_wait": {
                   k: [round(median(a), 3) for a in zip(*v)] for k,v in times.items()}, "samples_ms": times}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for label in ("tonari", "bash"):
        parser.add_argument("--" + label, type=Path, required=True)
    with tempfile.TemporaryDirectory() as directory:
        for record in run(Path(directory), vars(parser.parse_args())):
            print(json.dumps(record), flush=True)
