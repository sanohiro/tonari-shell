"""Repeat bensch command lines until both their output and status arrive on the PTY."""
import argparse
import os
from pathlib import Path
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from bench_history import PROMPT, read_until, terminal

# The first seven are single-line cases in the pinned interactive YAMLs. The
# last case has two successive lines; preserve the immediate read of $?.
CASES = {
    "posix_core/94": ("echo /nonexistent/*.xyz", "/nonexistent/*.xyz"),
    "posix_untested_auto/6": ("exec 5>&1; echo stdout >&5; exec 5>&-", "stdout"),
    "posix_gaps_auto/92": ("P='a*'; x=abc; case $x in $P) echo var_pattern;; esac", "var_pattern"),
    "posix_basic_auto/16": ('VAR=/usr/local/bin; echo "${VAR#/*/}"', "local/bin"),
    "posix_core/112": ("CMD='echo hello'; eval $CMD", "hello"),
    "posix_core/1": ("echo hello", "hello"),
    "posix_basic_auto/75": ('echo "test$var"', "test"),
    "posix_extended_auto/24": ('test 42 -ne 13; echo "EXIT=$?"', "EXIT=0"),
}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("bash", type=Path)
    p.add_argument("tonari", type=Path)
    p.add_argument("--runs", type=int, default=41)
    args = p.parse_args()
    shells = [("bash", args.bash.resolve()), ("tonari", args.tonari.resolve())]
    with tempfile.TemporaryDirectory() as tmp:
        print("case", "trial", "shell", "ms", sep="\t")
        for key, (command, expected) in CASES.items():
            packet = command.encode() + b"""; printf '\\036%s\\037' "$?"\r"""
            marker = b"\x1e0\x1f"
            for i in range(args.runs + 1):
                for name, binary in shells[::1 if i % 2 else -1]:
                    with tempfile.TemporaryDirectory(dir=tmp) as home:
                        with terminal(binary, Path(home), "/usr/bin:/bin", prompt=PROMPT) as fd:
                            start = time.perf_counter_ns()
                            os.write(fd, packet)
                            response = read_until(fd, marker)
                            ms = (time.perf_counter_ns() - start) / 1e6
                            if expected.encode() + b"\r\n" + marker not in response:
                                raise ValueError(f"{key} {name}: {response[-200:]!r}")
                            if i:
                                print(key, i, name, f"{ms:.6f}", sep="\t", flush=True)


if __name__ == "__main__":
    main()
