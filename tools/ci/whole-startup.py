"""Check and time real shell entry points, imported functions and shared regular stdin."""
import argparse
import json
import math
import os
from pathlib import Path
import random
import statistics
import subprocess
import tempfile
import time

from bench import first_prompt
from bench_history import read_until


def timed(shell, args, root, env, expected, stdin=None):
    with (stdin or Path(os.devnull)).open("rb") as stream:
        start = time.perf_counter_ns()
        proc = subprocess.run([shell, *args], cwd=root, env=env, stdin=stream,
                              capture_output=True, timeout=12)
        elapsed = (time.perf_counter_ns() - start) / 1e6
    if (proc.returncode, proc.stdout, proc.stderr) != (0, expected, b""):
        raise AssertionError((args, shell, proc))
    return {"wall": elapsed}


def ready(shell, args, env, prompt):
    def command(fd):
        start = time.perf_counter_ns()
        os.write(fd, b"printf 'first-%s\\n' ok\r")
        read_until(fd, b"first-ok\r\n")
        return (time.perf_counter_ns() - start) / 1e6
    _, display, first = first_prompt([str(shell), *args], env, prompt, after_prompt=command)
    return {"exec_to_prompt": display, "first_command": first}


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("shells", type=Path, nargs=3, metavar="BINARY")
    cli.add_argument("--runs", type=int, default=15)
    cli.add_argument("--only", action="append", help="workload prefix")
    args = cli.parse_args()
    rng = random.Random(1754509)
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        env = dict(HOME=tmp, PWD=tmp, PATH=os.defpath, LC_ALL="C", TERM="xterm",
                   INPUTRC="/dev/null", HISTFILE="/dev/null", PS1="plain~ ")
        def measure(name, n, sample):
            if args.only and not any(name.startswith(p) for p in args.only):
                return
            bins = dict(zip(("before", "after", "bash"), (p.resolve() for p in args.shells)))
            samples = {k: {} for k in bins}
            for i in range(args.runs + 2):
                keys = list(bins)
                rng.shuffle(keys)
                for k in keys:
                    for metric, value in sample(bins[k]).items():
                        if i >= 2:
                            samples[k].setdefault(metric, []).append(value)
            for k, metrics in samples.items():
                for metric, values in metrics.items():
                    print(json.dumps(dict(case=name, n=n, shell=k, metric=metric,
                                          median=statistics.median(values),
                                          p95=sorted(values)[math.ceil(.95*len(values))-1],
                                          runs=len(values))), flush=True)
        measure("agent-c", 1, lambda s: timed(s, ["-c", ":"], root, env, b""))
        measure("clean-interactive", 1, lambda s: ready(s, ["--noprofile","--norc","-i"], env, b"plain~ "))
        measure("login-without-profiles", 1, lambda s: ready(s, ["--noprofile","--norc","--login","-i"], env, b"plain~ "))
        rc = root / ".bashrc"
        rc.write_text("shopt -s checkwinsize\nalias ll='ls -l'\n"
                      + "".join(f'cmd_{i}() {{ printf "%s" "$1"; }}\n' for i in range(80))
                      + "complete -W 'start stop status' tool\nPS1='config~ '\n")
        (root / ".bash_profile").write_text('. "$HOME/.bashrc"\nPS1="login~ "\n')
        measure("configured-interactive", 80, lambda s: ready(s, ["--noprofile","--rcfile",str(rc),"-i"], env, b"config~ "))
        measure("login-interactive", 80, lambda s: ready(s, ["--login","-i"], env, b"login~ "))

        for n in (128, 256, 512):
            exported = env | {f"BASH_FUNC_f_{i}%%": f"() {{ printf '%s' {i}; }}" for i in range(n)}
            measure("syntax-with-exports", n, lambda s: timed(s, ["-n","-c","echo unused"], root, exported, b""))
            measure("execute-with-exports", n, lambda s: timed(s, ["-c",f"f_{n-1}"], root, exported, str(n-1).encode()))
        for n in (256, 512, 1024):
            block = "if true; then\n" + "".join(f"if [ {i} -ge 0 ]; then v={i}; fi\n" for i in range(n))
            script = root / "shared"
            # The first read inside the compound must see data after its closing fi.
            script.write_text(block + "IFS= read -r row\nfi\npayload\\x\nprintf '%s/%s' \"$v\" \"$row\"\n")
            measure("shared-input", n, lambda s: timed(s, [], root, env, f"{n-1}/payload\\x".encode(), script))
        # Measure first and repeated expansion in each directory through argv delivery.
        for n in (512, 1024, 2048):
            d = root / str(n)
            d.mkdir()
            for i in range(n):
                (d / f"file-{i:05}.txt").touch()
            for repeat in (1, 4):
                command = f"for ((i=0;i<{repeat};i++)); do set -- file-[0-9]*.txt; done; printf '%s|%s|%s' \"$#\" \"$1\" \"${{!#}}\""
                expected = f"{n}|file-00000.txt|file-{n-1:05}.txt".encode()
                measure("glob-"+str(repeat), n, lambda s: timed(s, ["-c",command], d, env | {"PWD":str(d)}, expected))


if __name__ == "__main__":
    main()
