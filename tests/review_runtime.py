"""Differential witnesses for the execution, scope and jobs review."""
import argparse
import os
import subprocess
import tempfile
from pathlib import Path


def check(shell, script, work):
    result = subprocess.run(
        [shell, "--noprofile", "--norc", "-c", script, "test-shell"],
        env={"HOME": str(work), "PATH": "/usr/bin:/bin", "LC_ALL": "C"},
        cwd=work, capture_output=True, timeout=8,
    )
    return result.returncode, result.stdout


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("tonari", type=Path)
    parser.add_argument("bash", type=Path)
    args = parser.parse_args()
    scripts = {
        "ordered assignments": """unset a; a=first b=$(printenv a) c=$a; printf '[%s][%s]\n' "$b" "$c"; export a; a=second b=$(printenv a); echo "$b";""",
        "local initializer": """x=20; f(){ local -i x=x+1; echo "$x"; }; f; echo "$x";""",
        "integer attributes": """OPTIND=2+3; echo $OPTIND; x=4; OPTIND=x; echo $OPTIND; declare OPTIND=1+1; echo $OPTIND; a=(2+3); declare -i a; a+=([0]+=4); echo "${a[0]}";""",
        "prefix attributes": """declare -i n=4; n=2+3 printenv n; set -o posix; n=2+3 :; echo "$n"; declare -u upper; upper=mixed :; echo "$upper";""",
        "case converted settings": """touch a.txt; declare -l GLOBIGNORE; GLOBIGNORE='*.TXT'; echo *;""",
        "assigned shell name": """declare BASH_ARGV0=changed; echo "$0"; f(){ local BASH_ARGV0=local; echo "$0/$BASH_ARGV0"; }; f; echo "$0";""",
        "pipeline index isolation": """a=(one two); i=0; echo "${a[i++]}" | cat; echo "$i"; shopt -s lastpipe; echo | echo "${a[i++]}"; echo "$i";""",
        "named descriptor close": """v=abc; : {v}>&-; echo $?;""",
        "EXIT redirection": """trap 'echo end' EXIT; { eval 'exit 7 >/dev/null'; } >/dev/null;""",
        "EXIT state and scope": """trap 'echo "$localname/$1"; trap -p EXIT' EXIT; f(){ local localname=child; exit 6 >/dev/null; }; f arg;""",
        "EXIT listing": """trap 'echo "$localname/$1"; trap -p EXIT' EXIT; f(){ local localname=child; exit 6; }; f arg;""",
        "time trailing percent": """TIMEFORMAT='percent%'; { time :; } 2>timing; cat timing;""",
        "wait stopped job without monitor": """set +m; "$BASH" -c 'kill -STOP "$$"; exit 7' & pid=$!; (sleep .1; kill -CONT "$pid") & wait "$pid"; echo "$?"; wait;""".replace('"$BASH"', '/bin/sh'),
    }
    for label, script in scripts.items():
        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp)
            ref = check(args.bash.resolve(), script, work)
            got = check(args.tonari.resolve(), script, work)
            if got != ref:
                raise AssertionError(f"{label}: expected {ref!r}, got {got!r}")
    print(f"{len(scripts)} runtime witnesses match Bash")


if __name__ == "__main__":
    main()
