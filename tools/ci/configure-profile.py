"""Localize configure latency by timing each completed progress line.

On Linux a separate diagnostic records process lifetimes. It runs after the
uninstrumented pair because tracing changes the timing of short commands.
"""
import os
from pathlib import Path
import shutil
import statistics
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


def report_clock(path):
    intervals = []
    for line in path.read_text().splitlines():
        name, start, end = line.split()
        intervals.append((int(start), int(end), name))
    if not intervals:
        raise ValueError(f"no compiler or test intervals in {path}")
    total = previous_end = 0
    cost = {}
    for start, end, name in sorted(intervals):
        total += max(0, end - max(previous_end, start))
        previous_end = max(previous_end, end)
        count, ns = cost.get(name, (0, 0))
        cost[name] = count + 1, ns + end - start
    for name, (count, ns) in cost.items():
        print(f"{path.stem} {name}: {count} calls, {ns / 1e9:.3f} s")
    print(f"{path.stem} combined elapsed: {total / 1e9:.3f} s", flush=True)


def components(script, shells):
    aux = Path(script).parent / "build-aux"
    true = shutil.which("true", path="/usr/bin:/bin")
    commands = [
        ("config.guess", [str(aux / "config.guess")]),
        ("config.sub", [str(aux / "config.sub"), "x86_64-linux-gnu"]),
        ("250 stderr duplicates", ["-c", 'exec 5>/dev/null; for ((i=0;i<250;i++)); do "$1" 2>&5; done', "sh", true]),
    ]
    with tempfile.TemporaryDirectory() as work:
        env = dict(PATH=os.environ["PATH"], HOME=work, LC_ALL="C")
        for label, command in commands:
            times = {name: [] for name, _ in shells}
            expected = None
            for trial in range(8):
                for name, shell in (reversed(shells) if trial % 2 else shells):
                    start = time.monotonic_ns()
                    p = subprocess.run([shell, *command], env=env, cwd=work, check=True, capture_output=True)
                    elapsed = (time.monotonic_ns() - start) / 1e6
                    if p.stderr or (expected is not None and p.stdout != expected):
                        raise ValueError(f"{name} changed {label} output")
                    expected = p.stdout
                    if trial:
                        times[name].append(elapsed)
            print(label, {name: round(statistics.median(ms), 3) for name, ms in times.items()}, flush=True)


if __name__ == "__main__":
    script, tonari, bash, output, *previous = sys.argv[1:]
    out = Path(output).resolve()
    out.mkdir(parents=True, exist_ok=True)
    header = None
    shells = [("tonari", tonari), ("bash", bash)]
    if previous:
        shells.append(("previous", previous[0]))
    components(script, shells)
    trace = shutil.which("strace") if sys.platform == "linux" else None
    for tag, tracer in [("warmup", None), ("phases", None)] + ([("traced", trace)] if trace else []):
        pairs = shells[:2] if tracer else shells
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
            report_clock(out / f"{name}-clock.clock")
