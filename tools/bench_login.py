"""Measure process launch through exit with the system and distribution login profiles."""

import argparse
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

from bench import COMMANDS, first_prompt, report


def measure_prompt(shells, flags, env, runs):
    samples = {name: [] for name, _ in shells}
    for run in range(5 + runs):
        for offset in range(len(shells)):
            name, path = shells[(run + offset) % len(shells)]
            _, elapsed = first_prompt([str(path), *flags], env, b"tonari-login-ready$ ")
            if run >= 5:
                samples[name].append(elapsed)
    report(samples)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tonari", type=Path)
    parser.add_argument("--bash", type=Path, default=Path("/bin/bash"))
    parser.add_argument("--previous", type=Path, help="previous Tonari release for comparison")
    parser.add_argument("--runs", type=int, default=101)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--prompt", action="store_true", help="also time exec to the first login PTY prompt")
    mode.add_argument("--rcfile", type=Path, help="only time interactive startup with this .bashrc")
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")

    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        home = root / "home"
        home.mkdir()
        for name in (".profile", ".bashrc"):
            skeleton = Path("/etc/skel") / name
            if skeleton.is_file():
                shutil.copyfile(skeleton, home / name)
        env = {"HOME": str(home), "TERM": "dumb", "PATH": "/usr/bin:/bin", "LC_ALL": "C"}
        shells = []
        for name, source in (("Bash", args.bash), ("Previous", args.previous), ("Tonari", args.tonari)):
            if source is not None:
                path = root / str(len(shells)) / "bash"
                path.parent.mkdir()
                shutil.copy(source, path)
                shells.append((name, path))

        tty_env = dict(env, TERM="xterm-256color")
        marker = "\nPS1='tonari-login-ready$ '\n"
        if args.rcfile:
            shutil.copyfile(args.rcfile, home / ".bashrc")
            with (home / ".bashrc").open("a") as rc:
                rc.write(marker)
            print("exec through .bashrc to first prompt")
            measure_prompt(shells, ["--noprofile", "-i"], tty_env, args.runs)
            return

        for label, flags in (("plain", ["-c", ":"]), ("login", ["--login", "-c", ":"]),
                             ("interactive login", ["--login", "-ic", ":"]),
                             ("spawn series", ["-c", COMMANDS["external-series"][1]])):
            samples = {name: [] for name, _ in shells}
            for run in range(5 + args.runs):
                for offset in range(len(shells)):
                    name, path = shells[(run + offset) % len(shells)]
                    start = time.perf_counter_ns()
                    process = subprocess.run([path, *flags], env=env, cwd=home, stdin=subprocess.DEVNULL,
                                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    end = time.perf_counter_ns()
                    process.check_returncode()
                    if run >= 5:
                        samples[name].append((end - start) / 1_000_000)
            print(label)
            report(samples)

        if args.prompt:
            # The marker is installed after the distro and user profile have run.
            with (home / ".profile").open("a") as profile:
                profile.write(marker)
            print("exec to first login prompt")
            measure_prompt(shells, ["--login", "-i"], tty_env, args.runs)


if __name__ == "__main__":
    main()
