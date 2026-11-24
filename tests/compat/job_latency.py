"""Compare job operations on blocked children with an explicit release instead of a timer."""
import argparse
import math
import statistics
import tempfile
from pathlib import Path

from audit import Audit

SETUP = "mkfifo release finished && exec 8<>release 9<>finished || exit 1"
CASES = {
    "list-running-and-wait": """
        (IFS= read -r token <&8; exit 13) & child=$!
        jobs -l %+ >/dev/null && jobs -r %+ >/dev/null && jobs -n >/dev/null || exit 1
        test "$(jobs -p %+)" = "$child" || exit 2
        printf 'go\\n' >&8
        wait "$child"; test "$?" = 13 || exit 3
    """,
    "disown-running": """
        (IFS= read -r token <&8; printf 'done\\n' >&9) & child=$!
        disown -r && test -z "$(jobs -p)" || exit 1
        printf 'go\\n' >&8
        IFS= read -r reply <&9 && test "$reply" = done || exit 2
    """,
}

def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("shell", type=Path, nargs="+")
    cli.add_argument("--runs", type=int, default=31)
    args = cli.parse_args()
    with tempfile.TemporaryDirectory() as directory:
        work = Path(directory)
        for name, body in CASES.items():
            shells = [shell.resolve() for shell in args.shell]
            timings = {shell: [] for shell in shells}
            code = "for i in {1..32}; do\n" + body + "\ndone; printf ok"
            for turn in range(args.runs + 3):
                shift = turn % len(shells)
                for shell in shells[shift:] + shells[:shift]:
                    outcome = Audit(shell, shell, work).call(
                        0, ["--noprofile", "--norc", "-c", code], timing="started", before_ready=SETUP)
                    if outcome.timeout or (outcome.code, outcome.out, outcome.err) != (0, b"ok", b""):
                        raise ValueError((name, shell, outcome.brief()))
                    if turn >= 3:
                        timings[shell].append(outcome.ns / 1e6)
            print(name, *(f"{statistics.median(v):.3f}/{sorted(v)[math.ceil(.95*len(v))-1]:.3f}"
                          for v in timings.values()), sep=" | ", flush=True)

if __name__ == "__main__":
    main()
