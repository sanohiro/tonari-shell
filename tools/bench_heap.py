"""Sample Linux shell RSS while holding and after dropping large values."""

import argparse
from pathlib import Path
import select
import statistics
import subprocess
import tempfile


SCRIPT = """echo ready; read -r _
set -- files/*; echo "$#"; read -r _
set --; echo cleared; read -r _
printf -v large '%*s' 16777216 x; echo "${#large}"; read -r _
unset large; echo unset; read -r _
"""
STAGES = ("ready", "10000", "cleared", "16777216", "unset")


def sample(shell, home):
    env = {"HOME": str(home), "PATH": "/usr/bin:/bin", "LC_ALL": "C"}
    with subprocess.Popen([shell, "--norc", "--noprofile", "-c", SCRIPT], cwd=home, env=env,
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE) as child:
        sizes = []
        for expected in STAGES:
            if not select.select([child.stdout], [], [], 10)[0]:
                child.kill()
                raise TimeoutError(f"{shell} timed out before {expected!r}")
            if child.stdout.readline().strip() != expected.encode():
                child.kill()
                raise RuntimeError(f"{shell} failed the {expected!r} checkpoint")
            fields = Path(f"/proc/{child.pid}/smaps_rollup").read_text().splitlines()
            sizes.append(int(next(row.split()[1] for row in fields if row.startswith("Rss:"))))
            child.stdin.write(b"\n")
            child.stdin.flush()
        if child.wait(timeout=5):
            raise RuntimeError(f"{shell} exited with {child.returncode}")
        return sizes


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("shell", nargs="+", type=Path)
    cli.add_argument("--runs", type=int, default=5)
    args = cli.parse_args()
    if args.runs < 1 or not Path("/proc/self/smaps_rollup").exists():
        cli.error("requires Linux smaps_rollup and a positive run count")
    shells = [s.resolve() for s in args.shell]
    with tempfile.TemporaryDirectory() as temp:
        home = Path(temp)
        files = home / "files"
        files.mkdir()
        for i in range(10000):
            (files / f"item-{i:06}.txt").touch()
        samples = {shell: [] for shell in shells}
        for i in range(args.runs):
            start = i % len(shells)
            for shell in shells[start:] + shells[:start]:
                samples[shell].append(sample(shell, home))
        print("shell | ready | 10k matches | cleared | 16 MiB variable | unset (median KiB)")
        for shell, runs in samples.items():
            print(shell, *(f"{statistics.median(stage):g}" for stage in zip(*runs)), sep=" | ")


if __name__ == "__main__":
    main()
