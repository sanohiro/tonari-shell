#!/usr/bin/env python3
"""Run language-review regression witnesses against Tonari and a reference Bash."""
import argparse
import os
import random
import statistics
import time
from pathlib import Path
import subprocess
import tempfile


def benchmarks(programs, env, scratch):
    case = "x=$(case item399 in\n" + "".join(f" item{i}) :;;\n" for i in range(400)) + " esac)"
    workloads = {
        "case-400": case,
        "arith-quoted": 'k=one; declare -A a; a[one]=0; for ((i=0;i<2000;i++)); do ((a["$k"]++)); done',
        "array-snapshot": 'a=({1..100}); for ((i=0;i<500;i++)); do : "prefix${a[@]}end"; done',
        "quoted-equality": 'v="a*b"; w=$v; for ((i=0;i<10000;i++)); do [[ $v == "$w" ]]; done',
        "dynamic-extglob": 'v=apple; w="@(apple|pear)"; for ((i=0;i<10000;i++)); do [[ $v == $w ]]; done',
        "prefix-capture": 'for ((i=0;i<2000;i++)); do declare "x$i=entry"; done; for ((i=0;i<100;i++)); do a=1 b=$(echo "$a") :; done',
        "shell-locale": 'LC_ALL=C; v=one; for ((i=0;i<5000;i++)); do [[ $v == [[:alpha:]]* ]]; done',
        "brace-literal": ": '" + "z" * 100 + "' " + "a{" * 1000,
    }
    rng = random.Random(42)
    print("name," + ",".join(programs))
    for name, script in workloads.items():
        samples = {k: [] for k in programs}
        for run in range(8):
            order = list(programs)
            rng.shuffle(order)
            for k in order:
                start = time.perf_counter_ns()
                sub = subprocess.run([programs[k], "--noprofile", "--norc", "-c", script],
                                     env=env, cwd=scratch, capture_output=True, timeout=30)
                elapsed = (time.perf_counter_ns() - start) / 1e6
                if sub.returncode:
                    raise RuntimeError(f"{name}/{k}: {sub.stderr!r}")
                if run: samples[k].append(elapsed)
        print(name + "," + ",".join(f"{statistics.median(samples[k]):.3f}" for k in programs),flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tonari", type=Path)
    parser.add_argument("bash", type=Path)
    parser.add_argument("--benchmark", action="store_true")
    parser.add_argument("--before", type=Path)
    args = parser.parse_args()
    cases = {
        "numeric value and offset": 'v=abcdef; n=2; w="n+1"; declare -i x; x=$w; printf "%s/%s" "$x" "${v:$w:2}"',
        "arithmetic keys": 'declare -A m; k=one; m[one]=7; printf "%s" "$((m[$k]+3))"',
        "arithmetic parentheses": "x=9; let '(x)=7' 2>/dev/null; echo \"$?/$x\"",
        "empty hexadecimal": 'echo "$((0x))"',
        "nounset command": 'set -u; (( absent )); echo unexpected',
        "nounset let": "set -u; let 'absent+1'; echo unexpected",
        "nounset loop": 'set -u; for ((i=0; i<absent; ++i)); do :; done; echo unexpected',
        "nounset parameter in command": '(( ${absent?required} )); echo unexpected',
        "array expansion order": 'a=(1 2); printf "<%s>" "${a[@]}$((a[2]=9))"',
        "slice expansion order": 's=abcdef; echo "${s:s=2}"',
        "literal arithmetic": "n='$k'; k=42; let 'v=n' 2>/dev/null; echo $?/$v",
        "malformed extglob suffix": "p='*@(abc'; v='aa@(abc'; [[ $v == $p ]]; echo $?",
        "mixed alias output": "alias a=yes; alias a missing a 2>&1 | sed -n '/^alias /p'",
        "closed redirection token": 'echo a 3>&-b; echo c 3>& -d',
        "case alternation": 'x=$(case yes in a|b) :;; c | no) :;; yes) echo match;; esac); echo "$x"',
        "quoted comparison": 'x="a*b"; y="a*b"; [[ $x == "$y" ]] && echo same; x=ab; [[ $x == "$y" ]]; echo $?',
        "continued array name": 'a\\\nb=(q)x; printf "%s" "$ab"',
        "alias print with operands": 'alias existing=value; alias -p next=item >/dev/null; alias next',
        "here document function": 'f() { cat <<END; }\nshell data\nEND\ndefinition=$(declare -f f); unset -f f; eval "$definition"; f',
        "two substitution documents": 'printf "<%s>" "$(cat <<A)" "$(cat <<B)"\n# first\nA\n# second\nB\n',
        "suffix outside document": 'printf "<%s>" "$(cat <<B)" <<A\n# inner\nB\n# outer\nA\n',
        "arithmetic sequence subscript": 'a=(5); echo "$((a[@]+1))"; declare -A m=([@]=7); echo "$((m[@]))"',
        "brace tilde after alternatives": 'printf "%s\n" ~+{,/x} ~ro{ot,ot}',
        "brace groups in current substitution": 'printf "%s" "${ { echo one; }; { echo two; }; }"',
        "wildcard literal escape": r"""p='a\';v='a\'; [[ $v == $p ]]; printf "%s/%s/%s" "$?" "${v/$p/X}" "${v%%$p}" """,
        "nested extended groups": "p='@" + "(@" * 70 + "(a|b)" + ")" * 71 + "'; [[ a == $p ]]; echo $?",
        "two here document functions": 'f() { cat <<A; }; g() { cat <<B; }\none\nA\ntwo\nB\nfirst=$(declare -f f); second=$(declare -f g); unset -f f g; eval "$first"; eval "$second"; f; g',
    }
    sh, bash = args.tonari.resolve(), args.bash.resolve()
    with tempfile.TemporaryDirectory() as scratch:
        env = {"PATH": os.defpath, "HOME": scratch, "LC_ALL": "C"}
        if args.benchmark:
            programs = {"tonari": sh, "bash": bash}
            if args.before: programs["before"] = args.before.resolve()
            benchmarks(programs, env, scratch)
            return
        version = subprocess.check_output([bash, "-c", 'printf "%s.%s" "${BASH_VERSINFO[0]}" "${BASH_VERSINFO[1]}"'], env=env, text=True)
        for name, script in cases.items():
            options = env
            if name == "alias print with operands":
                options = dict(env, BASH_COMPAT=version)

            if name == "brace groups in current substitution" and tuple(map(int,version.split("."))) < (5,3):
                continue
            observed = [
                subprocess.run([exe, "--noprofile", "--norc", "-c", script],
                               env=options, cwd=scratch, capture_output=True, timeout=10)
                for exe in (sh, bash)
            ]
            results = [(run.returncode, run.stdout) for run in observed]
            if results[0] != results[1]:
                raise AssertionError(f"{name}: {results!r}; {observed[0].stderr!r}")
            print(f"ok {name}")


if __name__ == "__main__":
    main()
