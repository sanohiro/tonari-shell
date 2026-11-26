"""Interleave Bash and Tonari runs for the external-suite fixes on a native host."""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from bench_history import PROMPT, read_until, terminal

CASES = {
    "attributes": ('a=({1..1500}); for ((i=0;i<100;i++)); do b=("${a[@]@a}"); done; '
                   'echo "${#b[@]}:${b[0]}"', b"1500:a\n"),
    "last argument": (': old; for ((i=0;i<10000;i++)); do : some-argument; done; echo "$_"',
                      b"some-argument\n"),
    "array index": ('for ((i=0;i<1800;i++)); do a=([0]=3 [a[0]]=5 [a[3]]=7); done; '
                    'echo "${a[5]}"', b"7\n"),
    "associative arithmetic": ('declare -A a; key=counter; for ((i=0;i<8000;i++)); '
                               'do ((a["$key"]++)); done; echo "${a[$key]}"', b"8000\n"),
    "signal names": ('for ((i=0;i<3000;i++)); do kill -l HUP BAD INT 9999 129; done '
                     '>list 2>/dev/null; echo "$?"; tail -3 list', b"1\n1\n2\nHUP\n"),
    "regex home": ('for ((i=0;i<3000;i++)); do [[ "$HOME/a" =~ ~/[ab] ]] || exit 1; '
                   'done; echo ok', b"ok\n"),
    "lastpipe": ('shopt -s lastpipe; for ((i=0;i<120;i++)); do printf "yes\\n" | read -r v; done; '
                 'echo "$v"', b"yes\n"),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shells", nargs="+", type=lambda p: Path(p).resolve())
    parser.add_argument("--trials", type=int, default=13)
    parser.add_argument("--case", choices=[*CASES, "vi"], action="append")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as directory:
        home = Path(directory)
        regex_home = home / "^a$"
        regex_home.mkdir()
        rc = home / "rc"
        rc.write_text('PS1="tonari$ "\nset -o vi\ns(){ printf "edited:%s:%s\\n" "${#1}" "$2"; }\n')
        for trial in range(args.trials):
            for shell in (args.shells if trial % 2 == 0 else reversed(args.shells)):
                for name, (script, expected) in CASES.items():
                    if args.case and name not in args.case:
                        continue
                    start = time.perf_counter_ns()
                    run = subprocess.run([shell, "-c", script], cwd=home, capture_output=True,
                                         env={"PATH": "/usr/bin:/bin",
                                              "HOME": str(regex_home) if name == "regex home" else directory},
                                         timeout=15)
                    elapsed = time.perf_counter_ns() - start
                    if (run.returncode, run.stdout, run.stderr) != (0, expected, b""):
                        raise RuntimeError((shell, name, run.returncode, run.stdout, run.stderr))
                    print(name, shell, trial, elapsed / 1e6, sep="\t", flush=True)
                if args.case and "vi" not in args.case:
                    continue
                with terminal(shell, home, "/usr/bin:/bin", rc, PROMPT) as fd:
                    os.write(fd, b"s " + b"x" * 400 + b" tail")
                    read_until(fd, b"tail")
                    os.write(fd, b"\x1b")
                    time.sleep(0.62)
                    start = time.perf_counter_ns()
                    os.write(fd, b"0w" + b"wb" * 30 + b"cwgood\r")
                    read_until(fd, b"edited:4:tail\r\n")
                    print("vi", shell, trial, (time.perf_counter_ns() - start) / 1e6,
                          sep="\t", flush=True)


if __name__ == "__main__":
    main()
