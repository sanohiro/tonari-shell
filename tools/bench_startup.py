"""Time exec through the first prompt with realistic history, inputrc and environment sizes."""

import argparse
import math
import shutil
import statistics
import tempfile
from pathlib import Path

from bench import PROMPT, first_prompt


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
        for scenario in ("clean", "forced interactive", "system inputrc", "user inputrc",
                         "large environment", "4 MiB journal", "8 MiB journal"):
            home = root / scenario
            home.mkdir()
            env = {f"WORK_VAR_{i}": str(i) for i in range(512) if scenario == "large environment"}
            env.update(PATH="/usr/bin:/bin", HOME=str(home), TERM="xterm-256color", LC_ALL="C",
                       PS1=PROMPT.decode(), INPUTRC="/dev/null", HISTFILE=str(home / ".bash_history"))
            if scenario in ("system inputrc", "user inputrc"):
                del env["INPUTRC"]
            if scenario == "user inputrc":
                (home / ".inputrc").write_text("set editing-mode emacs\nset enable-bracketed-paste on\n")
            participants = [*shells, args.bash.resolve()]
            if scenario.endswith("journal"):
                journal = home / ".local/state/tonari/history"
                journal.parent.mkdir(parents=True)
                size = int(scenario.split()[0]) * 1024 * 1024
                row = b"1\t1\t0\t/tmp\tprintf 'history row'\n"
                journal.write_bytes(row * (size // len(row)))
                participants = shells
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
            for scenario, shell, argv, env, _ in cases:
                path = args.traces / f"{shell.name}-{scenario.replace(' ', '-')}.txt"
                first_prompt([strace, "--kill-on-exit", "-o", str(path), "-e",
                              "trace=ioctl,rt_sigaction,open,openat,fstat,newfstatat,statx,write,writev,read,lseek,getppid",
                              *argv], env)
        for i in range(args.runs + 3):
            offset = i % len(cases)
            for _, _, argv, env, samples in cases[offset:] + cases[:offset]:
                _, elapsed = first_prompt(argv, env)
                if i >= 3:
                    samples.append(elapsed)
        print("scenario | shell | median / p95 exec to prompt (ms)")
        for scenario, shell, _, _, samples in cases:
            p95 = sorted(samples)[math.ceil(len(samples) * .95) - 1]
            print(f"{scenario} | {shell} | {statistics.median(samples):.3f} / {p95:.3f}")


if __name__ == "__main__":
    main()
