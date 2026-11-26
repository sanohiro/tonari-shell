"""Interleave Bash and Tonari runs for the external-suite fixes on a native host."""
import argparse
from datetime import datetime, timezone
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from bench_history import PROMPT, read_until, terminal

HISTORY_EPOCH = 1699999000
HISTORY_COUNT = 2000
HISTORY_OUTPUT = "".join(
    f"{i + 1:5}  {datetime.fromtimestamp(HISTORY_EPOCH + i, timezone.utc):%Y-%m-%d %H:%M:%S} cmd{i:04d}\n"
    for i in range(HISTORY_COUNT)
).encode()

CASES = {
    "history dates": ('export TZ=UTC0; HISTSIZE=3000; HISTTIMEFORMAT="%F %T "; '
                      'history -c; history -r dates; history', HISTORY_OUTPUT),
    "startup": (":", b""),
    "printf time": ('export TZ=America/New_York; for ((i=0;i<2000;i++)); do '
                    'printf -v stamp "%(%F %T %Z %z)T" 1593604800; done; echo "$stamp"',
                    b"2020-07-01 08:00:00 EDT -0400\n"),
    "regional time": ('export TZ=UTC0 LC_ALL=de_DE.UTF-8; for ((t=0;t<2000;t++)); do '
                      'printf -v stamp "%(%A %B %F %T)T" "$t"; done; echo "$stamp"',
                      b"Donnerstag Januar 1970-01-01 00:33:19\n"),
    "padded time": ('export TZ=UTC0 LC_ALL=C; for ((t=0;t<2000;t++)); do '
                    'printf -v stamp "%(%_d %0e %k %l)T" "$t"; done; echo "$stamp"', b" 1 01  0 12\n"),
    "changing timestamps": ('export TZ=America/New_York; for ((t=1583650200;t<=1583652600;t++)); do '
                            'printf -v stamp "%(%F %T %Z %z)T" "$t"; done; echo "$stamp"',
                            b"2020-03-08 03:30:00 EDT -0400\n"),
    "printf epoch": ('for ((i=0;i<4000;i++)); do printf -v ts "%(%s)T" 86400; done; echo "$ts"',
                     b"86400\n"),
    "default tilde": ('for ((i=0;i<4000;i++)); do p=${u-~:~}; done; echo "$p"',
                      b"/abc:/abc\n"),
    "brace tilde": ('for ((i=0;i<2000;i++)); do a=({~,~}/bin); done; echo "${a[*]}"',
                    b"/abc/bin /abc/bin\n"),
    "indexed argument": ('set -f; f() { x=$#; }; for ((i=0;i<3000;i++)); do f a[1 + 2]=x; done; echo "$x"',
                         b"3\n"),
    "bracket arithmetic": ('a[3]=7; for ((i=0;i<6000;i++)); do x=$[a[3]+1]; done; echo "$x"',
                           b"8\n"),
    "read readiness": ('for ((i=0;i<2000;i++)); do read -t 0 || exit 1; done </dev/null; echo ok', b"ok\n"),
    "test option": ('set -o pipefail; for ((i=0;i<5000;i++)); do test -o pipefail || exit 1; done; echo ok', b"ok\n"),
    "set listing": ('for ((i=0;i<300;i++)); do printf -v "v$i" "%s" "two words"; done; '
                    'for ((i=0;i<100;i++)); do set; done >vars; grep -m 1 "^v299=" vars',
                    b"v299='two words'\n"),
    "echo escapes": (r"shopt -s xpg_echo; for ((i=0;i<4000;i++)); do x=$(echo 'a\tb'); done; "
                     'printf "%s" "$x"', b"a\tb"),
    "large completion": ('for ((i=0;i<30;i++)); do compgen -o default files/item; done >matches; '
                         'n=$(wc -l <matches); echo $((n))', b"61440\n"),
    "completion fallback": ('mkdir -p work/prefix-dir; touch work/prefix-file; '
                            'for ((i=0;i<1000;i++)); do compgen -o default work/pre; done >matches; '
                            'tail -2 matches | sort', b"work/prefix-dir\nwork/prefix-file\n"),
    "array defaults": ('a=({1..512}); for ((i=0;i<500;i++)); do v="${a[@]:+present}"; done; echo "$v"',
                       b"present\n"),
    "array default fields": ('a=({1..512}); for ((i=0;i<200;i++)); do b=("${a[@]:-none}"); done; '
                             'echo "${#b[@]}:${b[511]}"', b"512:512\n"),
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
        if not args.case or "large completion" in args.case:
            files = home / "files"
            files.mkdir()
            for i in range(2048):
                (files / f"item{i:04}").touch()
        if not args.case or "history dates" in args.case:
            (home / "dates").write_text("".join(f"#{HISTORY_EPOCH + i}\ncmd{i:04d}\n"
                                                for i in range(HISTORY_COUNT)))
        rc = home / "rc"
        rc.write_text('PS1="tonari$ "\nset -o vi\ns(){ printf "edited:%s:%s\\n" "${#1}" "$2"; }\n')
        for trial in range(args.trials):
            for shell in (args.shells if trial % 2 == 0 else reversed(args.shells)):
                for name, (script, expected) in CASES.items():
                    if args.case and name not in args.case:
                        continue
                    start = time.perf_counter_ns()
                    run = subprocess.run([shell, "-c", script], cwd=home, capture_output=True,
                                         env={"PATH": "/usr/bin:/bin", **({"TZ": "UTC0"} if name == "history dates" else {}),
                                              "HOME": "/abc" if name in ("default tilde", "brace tilde") else
                                                      str(regex_home) if name == "regex home" else directory},
                                         timeout=15)
                    elapsed = time.perf_counter_ns() - start
                    if (run.returncode, run.stdout, run.stderr) != (0, expected, b""):
                        raise RuntimeError((shell, name, run.returncode, len(run.stdout), run.stdout[:512], run.stderr[:512]))
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
