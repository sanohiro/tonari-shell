"""Time exec through the first prompt with realistic history, inputrc and environment sizes."""

import argparse
import math
import os
import shutil
import statistics
import tempfile
import time
from pathlib import Path

from bench import PROMPT, first_prompt
from bench_history import read_until


def first_echo(fd):
    start = time.perf_counter_ns()
    os.write(fd, b"Z")
    read_until(fd, b"Z")
    elapsed = (time.perf_counter_ns() - start) / 1e6
    os.write(fd, b"=1; printf 'ok%s%s\\n' \"$Z\" x\r")
    read_until(fd, b"ok1x\r\n")
    return elapsed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shells", nargs="+", type=Path)
    parser.add_argument("--bash", type=Path, default=Path("/bin/bash"))
    parser.add_argument("--runs", type=int, default=61)
    parser.add_argument("--traces", type=Path, help="save separate Linux syscall traces outside timing samples")
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        shells = [path.resolve() for path in args.shells]
        cases = []
        journals = {}
        for scenario in ("clean", "forced interactive", "system inputrc", "user inputrc",
                         "large environment", "4 MiB journal", "8 MiB journal"):
            home = root / scenario
            home.mkdir()
            env = {f"WORK_VAR_{i}": str(i) for i in range(512)} if scenario == "large environment" else {}
            env.update(PATH="/usr/bin:/bin", HOME=str(home), TERM="xterm-256color", LC_ALL="C",
                       PS1=PROMPT.decode(), INPUTRC="/dev/null", HISTFILE=str(home / ".bash_history"),
                       HISTIGNORE="*")
            if scenario in ("system inputrc", "user inputrc"):
                del env["INPUTRC"]
            if scenario == "user inputrc":
                (home / ".inputrc").write_text("set editing-mode emacs\nset enable-bracketed-paste on\n")
            participants = [*shells, args.bash.resolve()]
            journal = home / ".local/state/tonari/history"
            if scenario.endswith("journal"):
                journal.parent.mkdir(parents=True)
                size = int(scenario.split()[0]) * 1024 * 1024
                row = b"1\t1\t0\t/tmp\tprintf 'history row'\n"
                journal.write_bytes(row * (size // len(row)))
                participants = shells
            journals[journal] = journal.stat().st_size if journal.exists() else 0
            for shell in participants:
                argv = [str(shell), "--noprofile", "--norc"]
                if scenario == "forced interactive":
                    argv.append("-i")
                cases.append((scenario, shell, argv, env, []))
        if args.traces:
            strace = shutil.which("strace")
            if not strace:
                parser.error("--traces requires strace")
            args.traces.mkdir(parents=True, exist_ok=True)
            for index, (scenario, shell, argv, env, _) in enumerate(cases):
                path = args.traces / f"{index}-{shell.name}-{scenario.replace(' ', '-')}.txt"
                first_prompt([strace, "--kill-on-exit", "-o", str(path), "-e",
                              "trace=ioctl,rt_sigaction,open,openat,fstat,newfstatat,statx,write,writev,read,lseek,getppid",
                              *argv], env)
        for i in range(args.runs + 3):
            offset = i % len(cases)
            for _, _, argv, env, samples in cases[offset:] + cases[:offset]:
                _, elapsed, echo = first_prompt(argv, env, after_prompt=first_echo)
                if i >= 3:
                    samples.append((elapsed, echo))
        for path, size in journals.items():
            if (path.stat().st_size if path.exists() else 0) != size:
                raise RuntimeError(f"startup probe modified its own history fixture: {path}")
        print("scenario | shell | exec to prompt | first echo (median / p95 ms)")
        for scenario, shell, _, _, samples in cases:
            metrics = []
            for column in zip(*samples):
                p95 = sorted(column)[math.ceil(len(column) * .95) - 1]
                metrics.append(f"{statistics.median(column):.3f} / {p95:.3f}")
            print(scenario, shell, *metrics, sep=" | ")


if __name__ == "__main__":
    main()
