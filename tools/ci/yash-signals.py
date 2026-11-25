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


def sample(argv, signame, directory):
    before = time.monotonic_ns()
    with subprocess.Popen(argv, cwd=directory,
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
        raise ValueError(f"{argv[0]}/{signame}: expected marker and signal, got {marker!r}, {code}")
    return [(after_marker - before)/1e6, (end-after_marker)/1e6]


def alarm_timeout(signum, frame):
    raise TimeoutError("signal delivery timed out")


def run(directory, binaries, control=None, runs=11):
    signal.signal(signal.SIGALRM, alarm_timeout)
    shells = {}
    for label, binary in binaries.items():
        path = directory / label / "sh"
        path.parent.mkdir()
        path.symlink_to(binary.resolve())
        shells[label] = path
    resource.setrlimit(resource.RLIMIT_CORE, (0, resource.getrlimit(resource.RLIMIT_CORE)[1]))
    pattern = Path("/proc/sys/kernel/core_pattern")
    for name in ("TERM", "FPE", "SEGV", "ABRT", "ILL", "BUS", "QUIT"):
        # Bash ignores QUIT in a simple foreground invocation. Measure the
        # kernel control for this signal alongside the upstream async-job case.
        commands = {k: [sh, "-c", f"printf .; kill -s {name} $$"]
                    for k, sh in shells.items() if name != "QUIT"}
        if control:
            commands["direct"] = [control, "--signal-exit", str(int(getattr(signal, "SIG" + name)))]
        if not commands:
            continue
        times = {k: [] for k in commands}
        order = list(commands)
        for n in range(runs + 1):
            for k in order:
                value = sample(commands[k], name, directory)
                if n:
                    times[k].append(value)
            order = order[1:] + order[:1]
        yield {"signal": name, "core_pattern": pattern.read_text().strip() if pattern.exists() else None,
               "core_soft_limit": 0, "median_ms_start_to_marker_and_marker_to_wait": {
                   k: [round(median(a), 3) for a in zip(*v)] for k,v in times.items()}, "samples_ms": times}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for label in ("tonari", "bash"):
        parser.add_argument("--" + label, type=Path, required=True)
    parser.add_argument("--control", type=Path)
    parser.add_argument("--runs", type=int, default=11)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("at least one measured trial is required")
    with tempfile.TemporaryDirectory() as directory:
        for record in run(Path(directory), {"tonari": args.tonari, "bash": args.bash},
                          args.control.resolve() if args.control else None, args.runs):
            print(json.dumps(record), flush=True)
