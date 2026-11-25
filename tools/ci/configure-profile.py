"""Localize configure latency by timing each completed progress line.

On Linux a separate diagnostic records process lifetimes. It runs after the
uninstrumented pair because tracing changes the timing of short commands.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time


def profile(script, shell, out, label, tracer):
    with tempfile.TemporaryDirectory() as work:
        root = Path(work)
        env = dict(HOME=work, TMPDIR=work, PATH=os.environ["PATH"], LC_ALL="C",
                   SHELL=shell, CONFIG_SHELL=shell, CC="/usr/bin/cc")
        argv = [shell, script, "--disable-nls", "--cache-file=/dev/null"]
        if tracer:
            argv = [tracer, "-f", "--seccomp-bpf", "-qq", "-ttt", "-s", "160",
                    "-e", "trace=execve,execveat,exit,exit_group", "-o",
                    out / f"{label}.trace", *argv]
        start = last = time.monotonic_ns()
        phases = []
        with subprocess.Popen(argv, cwd=root, env=env, stdout=subprocess.PIPE) as proc:
            for line in proc.stdout:
                now = time.monotonic_ns()
                phases.append(f"{(now-last)/1e6:.3f}\t{line.decode().rstrip()}\n")
                last = now
            if proc.wait() != 0:
                raise RuntimeError(f"{shell} failed")
        elapsed = (time.monotonic_ns() - start) / 1e6
        (out / f"{label}.tsv").write_text("".join(phases))
        if (root / "config.status").read_bytes().splitlines()[0].removeprefix(b"#!").strip() != os.fsencode(shell):
            raise RuntimeError("config.status interpreter changed")
        print(f"{label}: {elapsed:.3f} ms", flush=True)
        return (root / "config.h").read_bytes()


if __name__ == "__main__":
    script, tonari, bash, output = sys.argv[1:]
    out = Path(output)
    header = None
    trace = shutil.which("strace") if sys.platform == "linux" else None
    for tag, tracer in [("warmup", None), ("phases", None)] + ([("traced", trace)] if trace else []):
        pairs = [("tonari", tonari), ("bash", bash)]
        for name, shell in (reversed(pairs) if tag == "phases" else pairs):
            value = profile(script, shell, out, f"{name}-{tag}", tracer)
            if header is not None and value != header:
                raise ValueError("config.h changed")
            header = value
