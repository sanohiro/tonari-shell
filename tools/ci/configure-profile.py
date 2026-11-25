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


def profile(script, shell, out, label, tracer, compiler=None):
    with tempfile.TemporaryDirectory() as work:
        root = Path(work)
        env = dict(HOME=work, TMPDIR=work, PATH=os.environ["PATH"], LC_ALL="C",
                   SHELL=shell, CONFIG_SHELL=shell, CC="/usr/bin/cc")
        if compiler:
            env.update(CC=str(compiler), CONFIGURE_CLOCK_LOG=str(out / f"{label}.clock"))
            Path(env["CONFIGURE_CLOCK_LOG"]).write_text("")
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

    # Include OS execution and waiting for each generated test. Only this
    # diagnostic copy has a launcher inserted at the shell's conftest calls.
    # The ordinary phase traces and published medians use the original script.
    with tempfile.TemporaryDirectory() as work:
        root = Path(work)
        clock = root / "clock"
        subprocess.run(["/usr/bin/cc", "-O2", Path(__file__).with_name("configure-clock.c"),
                        "-o", clock], check=True)
        compiler = root / "cc"
        compiler.symlink_to(clock)
        copy = root / "source"
        shutil.copytree(Path(script).parent, copy, symlinks=True)
        probe = copy / "configure"
        source = probe.read_text()
        for prefix in ("ac_try='", "LC_CTYPE= ", "&& "):
            source = source.replace(prefix + "./conftest", f"{prefix}{clock} ./conftest")
        probe.write_text(source)
        for name, shell in [("bash", bash), ("tonari", tonari)]:
            if profile(probe, shell, out, f"{name}-clock", None, compiler) != header:
                raise ValueError("timed compiler or probe changed config.h")
