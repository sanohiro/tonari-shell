"""Benchmark Tonari against GNU Bash with warm caches on a Unix host."""

import argparse
import math
import os
import pty
import select
import signal
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


PROMPT = b"tonari$ "
COMMANDS = {
    "-c :": ":",
    "external true": "/usr/bin/true",
    "two-stage pipeline": "/usr/bin/printf x | /usr/bin/wc -c",
}


def first_prompt(argv, env):
    start = time.perf_counter_ns()
    pid, fd = pty.fork()
    if pid == 0:
        try:
            os.execve(argv[0], argv, env)
        finally:
            os._exit(125)
    try:
        data = b""
        answered = 0
        deadline = time.monotonic() + 4
        while PROMPT not in data:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([fd], [], [], remaining)[0]:
                raise RuntimeError(f"prompt timed out for {argv[0]}: {data!r}")
            chunk = os.read(fd, 4096)
            if not chunk:
                raise RuntimeError(f"{argv[0]} exited before its prompt")
            data += chunk
            # Answer terminal cursor-position queries during prompt rendering.
            queries = data.count(b"\x1b[6n")
            for _ in range(queries - answered):
                os.write(fd, b"\x1b[1;1R")
            answered = queries
        return (time.perf_counter_ns() - start) / 1e6
    finally:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.close(fd)
        os.waitpid(pid, 0)


def round_trip(binary, flags, command, env):
    start = time.perf_counter_ns()
    subprocess.run(
        [binary, *flags, "-c", command], env=env, stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True,
    )
    return (time.perf_counter_ns() - start) / 1e6


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("tonari", type=Path)
    cli.add_argument("--bash", type=Path, default=Path("/bin/bash"))
    cli.add_argument("--runs", type=int, default=100)
    args = cli.parse_args()
    if args.runs < 1:
        cli.error("--runs must be positive")
    shells = [
        ("Tonari", args.tonari.resolve(), []),
        ("Bash", args.bash.resolve(), ["--noprofile", "--norc"]),
    ]

    with tempfile.TemporaryDirectory() as temp:
        env = {
            "PATH": "/usr/bin:/bin", "HOME": temp, "HISTFILE": "/dev/null",
            "TERM": "xterm-256color", "PS1": PROMPT.decode(), "LC_ALL": "C",
            "BASH_SILENCE_DEPRECATION_WARNING": "1",
        }
        for label in ["first prompt", *COMMANDS]:
            print(label, flush=True)
            samples = {name: [] for name, _, _ in shells}
            for i in range(5 + args.runs):
                offset = i % len(shells)
                for name, binary, flags in shells[offset:] + shells[:offset]:
                    if label == "first prompt":
                        value = first_prompt([binary, *flags], env)
                    else:
                        value = round_trip(binary, flags, COMMANDS[label], env)
                    if i >= 5:
                        samples[name].append(value)
            for name, times in samples.items():
                p95 = sorted(times)[math.ceil(0.95 * len(times)) - 1]
                print(f"  {name:7} median {statistics.median(times):7.3f} ms  p95 {p95:7.3f} ms")
    for name, binary, _ in shells:
        print(f"{name:7} {binary.stat().st_size:>9} bytes")


if __name__ == "__main__":
    main()
