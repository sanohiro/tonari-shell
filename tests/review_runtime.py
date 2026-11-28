"""Differential witnesses for the execution, scope and jobs review."""
import argparse
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
        "export after unsetting a prefix local": """f(){
  local v=x; unset v; declare -p v; v=again; printenv v || echo hidden
  unset v; v=again; printenv v || echo hidden
}; g(){ local v=x; declare +x v; unset v; v=again; printenv v || echo hidden; }
v=outer; v=temp f; v=temp g; f; echo "$v" """,
        "pipeline index isolation": """a=(one two); i=0; echo "${a[i++]}" | cat; echo "$i"; shopt -s lastpipe; echo | echo "${a[i++]}"; echo "$i";""",
        "named descriptor close": """exec {v}>descriptor; : {v}>&-; echo $?;""",
        "EXIT redirection": """trap 'echo end' EXIT; { eval 'exit 7 >/dev/null'; } >/dev/null;""",
        "EXIT state and scope": """trap 'echo "$localname/$1"; trap -p EXIT' EXIT; f(){ local localname=child; exit 6 >/dev/null; }; f arg;""",
        "EXIT listing": """trap 'echo "$localname/$1"; trap -p EXIT' EXIT; f(){ local localname=child; exit 6; }; f arg;""",
        "time trailing percent": """TIMEFORMAT='percent%'; { time :; } 2>timing; cat timing;""",
        "wait stopped job without monitor": """set +m; "$BASH" -c 'kill -STOP "$$"; exit 7' & pid=$!; (for ((i=0; i<100; i++)); do
  case $(ps -o stat= -p "$pid") in *T*) break;; esac
  sleep .01
done; kill -CONT "$pid") & wait "$pid"; echo "$?"; wait;""".replace('"$BASH"', '/bin/sh'),
    }
    scripts.update({
        "circular nameref read": """declare -n a=b b=a; echo "[$a]"; echo "status=$?" """,
        "circular nameref assignment": """declare -n a=b b=a; a=1; echo after""",
        "empty associative key": """declare -A A; A[""]=x; echo after""",
        "local +n and typed invalid reference": """f(){ a=1; declare -n r=a; declare +n r=b; declare -p r a; declare -in n=x; echo "$?"; declare -p n; }; f""",
        "outer local unset": """shopt -s localvar_unset; x=g; f(){ local x=l; g; }; g(){ unset x; declare -p x; }; f; echo "$x" """,
        "wait -n saved process": """(sleep .05; exit 7) & p=$!; wait -n -p got "$p"; echo "$?:$((got==p))"; wait -n "$p"; echo "$?"; wait "$p"; echo "$?" """,
        "wait -n process substitution": """exec 8< <(sleep .15; exit 7); p=$!; wait -n -p got "$p"; echo "$?:$((got==p))"; wait "$p"; echo "$?" """,
        "here-doc parameters": """x='日本語'; cat <<EOF
$x/${x}/$missing
EOF
""",
        "compound redirection": """f(){ { if [[ $1 == hi ]]; then echo "$2"; fi; } >result; }; f hi ok; cat result; f bye no; cat result;""",
        "foreground job shorthand": """set -m; exec 3>&1 >/dev/null
/bin/sh -c 'kill -STOP "$$"; exit 9' & p=$!; wait "$p"; j=%1; command "$j" >ignored
echo "$?:$_:$((p==$!))" >&3; test ! -e ignored
""",
        "background job shorthand": """set -m; exec 3>&1 >/dev/null
/bin/sh -c 'kill -STOP "$$"; exit 13' & p=$!
wait "$p"; %1 &
echo "$?:$_:$((p==$!))" >&3; wait "$p"; echo $? >&3
""",
        "quoted asynchronous job name": """set -m; sleep .05 & p=$!; '%1' 2>/dev/null &
child=$!; echo $((child!=p)); wait "$child"; echo "$?"; wait "$p"
""",
        "empty job operands": """sleep .15 & p=$!
for op in 'jobs' 'disown' 'wait' 'wait -n'; do $op "" >/dev/null 2>&1; echo $?; done
wait "$p"; echo $?
""",
        "completed last job after bare wait": """(exit 3) & old=$!; (exit 5) & last=$!
sleep .15; wait; wait "$old" 2>/dev/null; echo $?; wait "$last"; echo $?
""",
        "stopped job TERM resumes its handler": """set -m
/bin/sh -c 'trap "exit 7" TERM; kill -STOP "$$"; sleep .1' & child=$!
wait "$child"; kill -TERM %1; sleep .15; jobs -s >stopped
test -s stopped; echo $?; kill -CONT "$child" 2>/dev/null; wait "$child"; echo $?
""",
        "attached signal options": """for opt in -s9 -nTERM; do
kill "$opt" invalid 2>err; s=$?; case $(<err) in *'invalid signal specification'*) echo invalid:$s;; esac
done""",
        "extdebug skip status": """shopt -s extdebug
trap 'case $BASH_COMMAND in "echo skip") false;; esac' DEBUG
false; echo skip; echo "status=$?"
""",
        "pipeline context": """a=(one); i=0; f(){ echo "$1/$2/$BASH_SUBSHELL"; }; f "$BASH_SUBSHELL" "${a[i++]}" | cat; echo "$i" """,
    })
    # Bash 5.3 retains background statuses for repeated wait -n and accepts a
    # process substitution PID. Bash 5.2 differs in these two cases.
    wait_results = {
        "wait -n saved process": (b"7:1\n7\n7\n", b"7:1\n127\n7\n"),
        "wait -n process substitution": (b"7:1\n7\n", b"127:0\n7\n"),
    }
    for label, script in scripts.items():
        with tempfile.TemporaryDirectory() as tmp:
            reference = Path(tmp) / "reference"
            candidate = Path(tmp) / "candidate"
            reference.mkdir()
            candidate.mkdir()
            ref = check(args.bash.resolve(), script, reference)
            got = check(args.tonari.resolve(), script, candidate)
            if label in wait_results:
                latest, previous = wait_results[label]
                assert got == (0, latest), (label, got)
                assert ref in ((0, latest), (0, previous)), (label, ref)
            elif got != ref:
                raise AssertionError(f"{label}: expected {ref!r}, got {got!r}")
    # Bash 5.2 allowed an invalid descriptor name here. Bash 5.3 diagnoses it.
    with tempfile.TemporaryDirectory() as tmp:
        assert check(args.tonari.resolve(), "v=abc; : {v}>&-; echo $?", Path(tmp)) == (0, b"1\n")
    print(f"{len(scripts)} runtime witnesses passed; invalid named descriptor rejected")


if __name__ == "__main__":
    main()
