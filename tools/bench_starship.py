"""Measure the upstream Starship Bash integration in interactive pseudo-terminals."""
import argparse
from collections import defaultdict
import fcntl
import math
import os
from pathlib import Path
import pty
import signal
import statistics
import struct
import subprocess
import tempfile
import termios
import time

from bench_history import read_until


PROMPT = "❯".encode() + b"\x1b[0m "
ACCEPTED = b"\x1b[?2004l"


def next_prompt(fd):
    accepted = read_until(fd, ACCEPTED)
    return read_until(fd, PROMPT, accepted.partition(ACCEPTED)[2])


def trial(shell, cwd, rc, env, verify):
    clock_in, clock_out = os.pipe()
    argv = [str(shell), "--noprofile", "--rcfile", str(rc), "-i"]
    pid, fd = pty.fork()
    if pid == 0:
        try:
            os.close(clock_in)
            os.chdir(cwd)
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
            os.write(clock_out, struct.pack("Q", time.monotonic_ns()))
            os.execve(shell, argv, env)
        finally:
            os._exit(127)
    os.close(clock_out)
    result = defaultdict(list)
    try:
        with os.fdopen(clock_in, "rb") as stamp:
            start, = struct.unpack("Q", stamp.read(8))
        first = read_until(fd, PROMPT)
        result["first prompt (ms)"].append((time.monotonic_ns() - start) / 1e6)
        if cwd.name == "project" and b"prompt-bench" not in first:
            raise ValueError(f"missing Git branch in prompt: {first!r}")
        for _ in range(3):
            start = time.monotonic_ns()
            os.write(fd, b":")
            echo = read_until(fd, b":")
            result["key echo (ms)"].append((time.monotonic_ns() - start) / 1e6)
            result["key echo (bytes)"].append(len(echo))
            start = time.monotonic_ns()
            os.write(fd, b"\r")
            next_prompt(fd)
            result["next prompt (ms)"].append((time.monotonic_ns() - start) / 1e6)
        if verify:
            os.write(fd, b"false\r")
            if b"\x1b[1;31m" + PROMPT not in next_prompt(fd):
                raise ValueError("missing failure prompt")
            os.write(fd, b"false | true\r")
            if b"\x1b[1;32m" + PROMPT not in next_prompt(fd):
                raise ValueError("missing success prompt")
            os.write(fd, b"""printf '<status=%s pipe=%s>\\n' "$STARSHIP_CMD_STATUS" "${STARSHIP_PIPE_STATUS[*]}"\r""")
            if b"<status=0 pipe=1 0>\r\n" not in next_prompt(fd):
                raise ValueError("Starship did not receive pipeline status")
        return result
    finally:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.close(fd)
        os.waitpid(pid, 0)


def compare(shells, runs, sample):
    values = {sh: defaultdict(list) for sh in shells}
    for i in range(runs + 2):
        start = i % len(shells)
        for sh in shells[start:] + shells[:start]:
            for key, samples in sample(sh, i == 0).items():
                if i >= 2:
                    values[sh][key].extend(samples)
    for key in values[shells[0]]:
        row = []
        for sh in shells:
            v = sorted(values[sh][key])
            row.append(f"{statistics.median(v):.3f}/{v[math.ceil(.95 * len(v)) - 1]:.3f}")
        print(key, *row, sep=" | ", flush=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("shells", type=Path, nargs="+")
    ap.add_argument("--starship", type=Path, required=True)
    ap.add_argument("--runs", type=int, default=21)
    args = ap.parse_args()
    shells = [sh.resolve() for sh in args.shells]
    starship = args.starship.resolve()
    print(subprocess.check_output([starship, "--version"], text=True).splitlines()[0])
    print("p50/p95", *shells, sep=" | ")
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        home, plain, project = (root / name for name in ("home", "plain", "project"))
        for d in (home, plain, project):
            d.mkdir()
        # Keep other Starship installations and user shell, Git and prompt settings out.
        (root / "starship").symlink_to(starship)
        env = dict(PATH=f"{root}:/usr/bin:/bin", HOME=str(home), INPUTRC="/dev/null",
                   TERM="xterm-256color", LANG="en_US.UTF-8", HISTFILE="/dev/null",
                   GIT_CONFIG_NOSYSTEM="1", GIT_CEILING_DIRECTORIES=str(root))
        subprocess.run(["git", "init", "-qb", "prompt-bench"], cwd=project, env=env, check=True)
        for i in range(200):
            (project / f"file-{i}").touch()
        subprocess.run(["git", "add", "."], cwd=project, env=env, check=True)
        subprocess.run(["git", "-c", "user.name=bench", "-c", "user.email=bench@localhost",
                        "commit", "-qm", "fixture"], cwd=project, env=env, check=True)
        rc = home / "rc"
        rc.write_text('eval "$(starship init bash)"\n')
        for cwd in (plain, project):
            print(cwd.name, flush=True)
            compare(shells, args.runs, lambda sh, verify: trial(sh, cwd, rc, env, verify))

        # Isolate shell work which is otherwise hidden by the renderer and Git.
        cases = [("200 empty jobs captures", 's=$(jobs -p)'),
                 ("200 external array captures", 's=$(/usr/bin/printf %s "${args[@]}")')]
        for label, body in cases:
            script = 'args=("a b" "" "*"); for ((i=0; i<200; i++)); do ' + body + '; done'
            def batch(sh, verify):
                start = time.monotonic_ns()
                subprocess.run([sh, "--noprofile", "--norc", "-c", script], env=env, cwd=plain, check=True)
                return {label + " (ms)": [(time.monotonic_ns() - start) / 1e6]}
            compare(shells, args.runs, batch)


if __name__ == "__main__":
    main()
