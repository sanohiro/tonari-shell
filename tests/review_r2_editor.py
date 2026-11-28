"""Time ordinary incremental history imports and interactive search after input is ready."""
import argparse
import os
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

from review_interactive import PROMPT, read_until, terminal


def sample(shell, home, rc, case, expected):
    if case != "history":
        with terminal(shell, home, str(home), rc, prompt=PROMPT) as fd:
            request = b"\x12navy\x05\x14" if case == "search" else b": $(zzprobe\t\x14"
            start = time.perf_counter_ns()
            os.write(fd, request)
            output = read_until(fd, b"<finished>")
            elapsed = (time.perf_counter_ns() - start) / 1e6
            if case == "search":
                assert b"<draft>: navy blue navy|16</draft>" in output, output
            elif expected:
                assert b"<draft>: $(zzprobecmd |15</draft>" in output, output
            return elapsed

    history = home / "plain"
    history.write_bytes(b"")
    script = """HISTFILE=$PWD/plain
printf 'ready\n'; read -r gate
for ((i=1; i<=64; ++i)); do history -s "entry-$i"; history -a; history -n; done
history"""
    with subprocess.Popen([shell, "--noprofile", "--norc", "-c", script], cwd=home,
                          env={"HOME": str(home), "PATH": "/usr/bin:/bin", "LC_ALL": "C"},
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE) as proc:
        assert proc.stdout.readline() == b"ready\n"
        start = time.perf_counter_ns()
        output, errors = proc.communicate(b"go\n", timeout=8)
        elapsed = (time.perf_counter_ns() - start) / 1e6
        assert proc.returncode == 0 and not errors, errors
    on_disk = history.read_text().splitlines()
    assert on_disk == [f"entry-{i}" for i in range(1, 65)], on_disk
    if expected:
        assert [line.split()[-1].decode() for line in output.splitlines()] == on_disk, output
    return elapsed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shells", nargs="+", type=Path)
    parser.add_argument("--samples", type=int, default=15)
    parser.add_argument("--previous", type=Path,
                        help="Previous snapshot with the known duplicate-import result")
    args = parser.parse_args()
    shells = [shell.resolve() for shell in args.shells]
    with tempfile.TemporaryDirectory() as tmp:
        home = Path(tmp)
        (home / "zzprobecmd").touch(mode=0o755)
        rc = home / "rc"
        rc.write_text(r"""PS1='tonari$ '
unset HISTFILE
history -s ': navy blue navy'
snapshot() { printf '\n<draft>%s|%s</draft><finished>\n' "$READLINE_LINE" "$READLINE_POINT"; }
bind -x '"\C-t":snapshot'
""")
        for case in ("history", "search", "completion"):
            timings = {shell: [] for shell in shells}
            for i in range(args.samples + 3):
                for shell in shells[i % len(shells):] + shells[:i % len(shells)]:
                    ms = sample(shell, home, rc, case, args.previous is None or shell != args.previous.resolve())
                    if i >= 3:
                        timings[shell].append(ms)
            for shell, values in timings.items():
                ordered = sorted(values)
                print(f"{case} {shell} median {statistics.median(values):.3f} ms "
                      f"p95 {ordered[(95 * len(ordered) - 1) // 100]:.3f} ms")


if __name__ == "__main__":
    main()
