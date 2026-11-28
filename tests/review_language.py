#!/usr/bin/env python3
"""Run language-review regression witnesses against Tonari and a reference Bash."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tonari", type=Path)
    parser.add_argument("bash", type=Path)
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
        "case alternation": 'x=$(case yes in a|b) :;; c | no) :;; yes) echo match;; esac); echo "$x"',
        "quoted comparison": 'x="a*b"; y="a*b"; [[ $x == "$y" ]] && echo same; x=ab; [[ $x == "$y" ]]; echo $?',
        "continued array name": 'a\\\nb=(q)x; printf "%s" "$ab"',
        "alias print with operands": 'alias existing=value; alias -p next=item >/dev/null; alias next',
        "here document function": 'f() { cat <<END; }\nshell data\nEND\ndefinition=$(declare -f f); unset -f f; eval "$definition"; f',
        "two here document functions": 'f() { cat <<A; }; g() { cat <<B; }\none\nA\ntwo\nB\nfirst=$(declare -f f); second=$(declare -f g); unset -f f g; eval "$first"; eval "$second"; f; g',
    }
    sh, bash = args.tonari.resolve(), args.bash.resolve()
    with tempfile.TemporaryDirectory() as scratch:
        env = {"PATH": os.defpath, "HOME": scratch, "LC_ALL": "C"}
        for name, script in cases.items():
            observed = [
                subprocess.run([exe, "--noprofile", "--norc", "-c", script],
                               env=env, cwd=scratch, capture_output=True, timeout=10)
                for exe in (sh, bash)
            ]
            results = [(run.returncode, run.stdout) for run in observed]
            if results[0] != results[1]:
                raise AssertionError(f"{name}: {results!r}; {observed[0].stderr!r}")
            print(f"ok {name}")


if __name__ == "__main__":
    main()
