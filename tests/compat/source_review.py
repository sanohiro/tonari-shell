"""Behavior and warm, interleaved release timings for the GNU Bash source review."""
import argparse
import json
import os
from pathlib import Path
import random
import statistics
import subprocess
import tempfile
import time

# Compare observable output and status. Error messages may identify the shell.
COMMON = {
    "test-N": 'for f in newer older tied missing link; do [[ -N $f ]]; echo $?; test -N "$f"; echo $?; [ -N "$f" ]; echo $?; done',
    "execution-string": '[[ $BASH_EXECUTION_STRING == *marker* ]]; echo $?; x=$BASH_EXECUTION_STRING; unset BASH_EXECUTION_STRING; echo "${BASH_EXECUTION_STRING-unset}"; BASH_EXECUTION_STRING=value; echo "$BASH_EXECUTION_STRING"',
    "history-extension": """for x in dir/file.txt dir/a.b.c .dot dir.ext/file plain trailing.; do history -c; history -s "echo $x"; history -p '!!:$:r' '!!:$:e'; done""",
    "source-args": 'set -- old; source ./setargs a b; echo "$#:$1"; f(){ source ./setargs nested; echo "$#:$1"; }; f func; source ./shiftargs a b; echo "$#:$1"',
    "funcnest": 'FUNCNEST=2; f(){ echo f; g; echo wrong; }; g(){ h; }; h(){ echo h; }; (f); echo "$?"; unset FUNCNEST; f',
}
NEW = {
    "source-path": 'PATH=none; shopt -u sourcepath; source -p empty:src mod one; . -psrc mod two; source -p empty mod; echo $?; source -p "" mod; source -p empty ./mod',
    "compgen-array": """a=(old); declare -n ref=a; compgen -V ref -W 'be bee bee' be; printf '<%s>\\n' "${a[@]}"; compgen -V ref -W 'x' z; echo "$?:${#a[@]}"; compgen -V 'bad-name' -W x; echo $?; declare -A map=([key]=ok); compgen -V map -W x; echo "$?:${map[key]}"; readonly kept=old; compgen -V kept -W x; echo "$?:$kept" """,
    "trap-action": """trap 'printf "%s" word' USR1; trap '' USR2; trap -P TERM USR1 USR2; echo $?; x=$(trap -P USR1); echo "$x"; trap -P; echo $?; trap -pP TERM; echo $?""",
    "trap-signal": """BASH_TRAPSIG=outer; trap 'old=$BASH_TRAPSIG; kill -USR2 $$; echo "restored:$BASH_TRAPSIG"' USR1; trap 'echo "nested:$BASH_TRAPSIG"' USR2; kill -USR1 $$; echo "$old:$BASH_TRAPSIG"; trap 'echo "err:$BASH_TRAPSIG"' ERR; trap 'echo "exit:$BASH_TRAPSIG"' EXIT; false""",
    "monotonic": 'a=$BASH_MONOSECONDS; BASH_MONOSECONDS=2; b=$BASH_MONOSECONDS; [[ $a =~ ^[0-9]+$ && $b -ge $a && $b -ne 2 ]]; echo $?; unset BASH_MONOSECONDS; echo "${BASH_MONOSECONDS-unset}"; BASH_MONOSECONDS=manual; echo "$BASH_MONOSECONDS"',
    "glob-keys": 'cd names; for GLOBSORT in name -name + - size -size atime -atime mtime -mtime ctime -ctime blocks -blocks numeric -numeric invalid; do printf "%s:" "$GLOBSORT"; printf "<%s>" *; echo; done; GLOBSORT=nosort; a=(*); printf "%s\\n" "${a[@]}" | sort',
    "current-output": """v=old; x="${ v=new; local scratch=private; printf 'a\\nb\\n\\n'; return 9; echo wrong; }"; echo "$?:$v:${scratch-unset}:$x" """,
    "current-reply": """REPLY=outer; x=${| echo visible; echo "${REPLY-unset}"; REPLY=$'a\\nb\\n'; return 7; }; printf '<%s>:%s:<%s>\\n' "$x" "$?" "$REPLY" """,
    "current-grammar": """x=${ case x in x) { printf '%s' '}'; }; esac; y=${ printf inner; }; cat <<'END'
here } and ; }
END
printf '%s' "$y"; }suffix; echo "$x" """,
    "current-state": 'set -- one; f(){ local v=outer; x=${ local v=inner; printf "%s/%s/%s" "$v" "$FUNCNAME" "$1"; set -- new; }; echo "$v:$x:$1"; }; f abc',
    "current-exit": 'x=${ exit 19; }; echo wrong',
    "current-loop": 'for v in 1 2; do x=${ break; }; echo wrong; done; echo done',
    "current-large": 'x=${ cat large; }; echo "${#x}"',
}

HOT = {
    "test-N": ('[[ -N newer ]]', 2500),
    "execution-string": ('s=$BASH_EXECUTION_STRING', 2500),
    "history-extension": ("history -p '!!:$:e' >/dev/null", 800),
    "source-args": ('source ./setargs a b', 600),
    "funcnest": ('f', 2500),
    "source-path": ('source -p empty:src noop', 600),
    "compgen-array": ('compgen -V arr -W "$words" item', 80),
    "trap-action": ('a=$(trap -P USR1)', 800),
    "trap-signal": ('kill -USR1 $$', 1000),
    "monotonic": ('s=$BASH_MONOSECONDS', 2500),
    "glob-name": ('a=(items/*)', 100),
    "glob-size": ('a=(items/*)', 100),
    "glob-numeric": ('a=(*)', 180),
    "current-output": ('a=${ printf value; }', 700),
    "current-reply": ('a=${| REPLY=value; }', 1800),
}

def fixture(root):
    for name in ("src", "empty", "names", "items"):
        (root / name).mkdir()
    for name in ("newer", "older", "tied"):
        path = root / name
        path.touch()
        base = 1700000000000000000
        os.utime(path, ns=(base, base + {"newer": 1, "older": -1, "tied": 0}[name]))
    (root / "link").symlink_to("newer")
    (root / "mod").write_text("echo cwd")
    (root / "src/mod").write_text('echo "src:$1"')
    (root / "src/noop").write_text(":")
    (root / "setargs").write_text('set -- changed last')
    (root / "shiftargs").write_text("shift")
    (root / "large").write_text("abcd" * 65536 + "\n")
    for i, name in enumerate(("10", "2", "30", "alpha", "beta", "snow雪")):
        path = root / "names" / name
        path.write_bytes(b"x" * (i % 3))
        os.utime(path, (100 - i, 100 + i))
    (root / "names/link").symlink_to("10")
    (root / "names/broken").symlink_to("missing")
    for i in range(256):
        (root / "items" / str(i * 17)).touch()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bash", type=Path)
    parser.add_argument("tonari", type=Path)
    parser.add_argument("--first", type=Path)
    parser.add_argument("--runs", type=int, default=9)
    args = parser.parse_args()
    shells = [p.resolve() for p in (args.bash, args.tonari, args.first) if p]
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        fixture(root)
        env = dict(PATH="/usr/bin:/bin", HOME=directory, LC_ALL="C", TMPDIR=directory,
                   INPUTRC="/dev/null", TERM="dumb")
        def run(shell, code):
            start = time.perf_counter_ns()
            p = subprocess.run([shell, "--noprofile", "--norc", "-c", code],
                               cwd=root, env=env, capture_output=True, timeout=20)
            return (p.returncode, p.stdout, bool(p.stderr)), (time.perf_counter_ns()-start)/1e6
        version = run(shells[0], 'printf "%s" "$BASH_VERSION"')[0][1].decode()
        modern = tuple(map(int, version.split(".")[:2])) >= (5, 3)
        cases = COMMON | (NEW if modern else {})
        failed = []
        for name, code in cases.items():
            expected = run(shells[0], code)[0]
            actual = run(shells[1], code)[0]
            if expected != actual:
                failed.append((name, repr(expected), repr(actual)))
        print(json.dumps(dict(reference=version, witnesses=len(cases), differences=failed)), flush=True)
        if failed:
            raise SystemExit(1)
        rng = random.Random(1729)
        for name, (body, count) in HOT.items():
            if not args.runs or (not modern and name not in COMMON):
                continue
            setup = {
                "history-extension": "history -s 'echo dir/file.txt'",
                "funcnest": 'FUNCNEST=8; f(){ :; }',
                "compgen-array": 'words="' + " ".join(f"item{i}" for i in range(512)) + '"',
                "trap-action": "trap ':' USR1", "trap-signal": "trap 's=$BASH_TRAPSIG' USR1",
                "glob-size": "GLOBSORT=size", "glob-numeric": "cd items; GLOBSORT=numeric",
            }.get(name, ":")
            code = f"{setup}; for ((i=0;i<{count};i++)); do {body}; done; echo done"
            samples = [[] for _ in shells]
            for r in range(args.runs + 1):
                indices = list(range(len(shells)))
                rng.shuffle(indices)
                for i in indices:
                    outcome, ms = run(shells[i], code)
                    if outcome != (0, b"done\n", False):
                        raise ValueError((name, shells[i], outcome))
                    if r:
                        samples[i].append(ms)
            print(json.dumps(dict(case=name, repeats=count, median_ms=[round(statistics.median(g), 3) for g in samples],
                                  samples_ms=[[round(v, 3) for v in g] for g in samples])), flush=True)

if __name__ == "__main__":
    main()
