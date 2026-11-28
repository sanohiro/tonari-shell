#!/usr/bin/env python3
"""Run language-review regression witnesses against Tonari and a reference Bash."""
import argparse
import os
import random
import statistics
import sys
import time
from pathlib import Path
import subprocess
import tempfile


def benchmarks(programs, env, scratch):
    case = "x=$(case item399 in\n" + "".join(f" item{i}) :;;\n" for i in range(400)) + " esac)"
    for name in ("escaped", "escaped" + chr(92)):
        directory = Path(scratch, name)
        directory.mkdir()
        for i in range(96):
            (directory / f"f{i}").touch()
    workloads = {
        "ordinary-path-glob": 'for ((i=0;i<200;i++)); do set -- escaped/f*; done; [[ $# == 96 ]]',
        "escaped-path-glob": r"""bs='\'; for ((i=0;i<200;i++)); do set -- escaped${bs}/f*; done; [[ $# == 96 ]]""",
        "case-400": case,
        "arith-quoted": 'k=one; declare -A a; a[one]=0; for ((i=0;i<2000;i++)); do ((a["$k"]++)); done',
        "array-snapshot": 'a=({1..100}); for ((i=0;i<500;i++)); do : "prefix${a[@]}end"; done',
        "quoted-equality": 'v="a*b"; w=$v; for ((i=0;i<10000;i++)); do [[ $v == "$w" ]]; done',
        "dynamic-extglob": 'v=apple; w="@(apple|pear)"; for ((i=0;i<10000;i++)); do [[ $v == $w ]]; done',
        "prefix-capture": 'for ((i=0;i<2000;i++)); do declare "x$i=entry"; done; for ((i=0;i<100;i++)); do a=1 b=$(echo "$a") :; done',
        "shell-locale": 'LC_ALL=C; v=one; for ((i=0;i<5000;i++)); do [[ $v == [[:alpha:]]* ]]; done',
        "posix-literal-aliases": 'set -o posix\n' + ''.join(f'alias a{i}=word\n' for i in range(800)),
        "compiled-arithmetic": 'for ((i=0;i<10000;i++)); do ((x=(i*5+3)^(i>>1))); done; :',
        "substitution-body": 'for ((i=0;i<3000;i++)); do v=$(printf "%s" "$i"); done',
        "static-trim": 'shopt -s extglob; x=wordending; for ((i=0;i<4000;i++)); do v=${x%%@(ending|ed)}; done',
        "repeated-bracket": 'v=ABCABCABC; p="+([[:upper:]])"; for ((i=0;i<4000;i++)); do [[ $v == $p ]]; done',
        "utf8-boundaries": 'v=$(printf "あ%.0s" {1..1200}); for ((i=0;i<1000;i++)); do x=${v:800:100}; done; echo "${#x}"',
        "brace-literal": ": '" + "z" * 100 + "' " + "a{" * 1000,
    }
    rng = random.Random(42)
    print("name," + ",".join(programs))
    for name, script in workloads.items():
        case_env = env if name != "utf8-boundaries" else dict(env, LC_ALL="en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8")
        samples = {k: [] for k in programs}
        for run in range(8):
            order = list(programs)
            rng.shuffle(order)
            for k in order:
                start = time.perf_counter_ns()
                sub = subprocess.run([programs[k], "--noprofile", "--norc", "-c", script],
                                     env=case_env, cwd=scratch, capture_output=True, timeout=30)
                elapsed = (time.perf_counter_ns() - start) / 1e6
                if name == "utf8-boundaries" and sub.stdout != b"100\n":
                    raise RuntimeError(f"{k}: unexpected UTF-8 slice length {sub.stdout!r}")
                if sub.returncode:
                    raise RuntimeError(f"{name}/{k}: {sub.stderr!r}")
                if run: samples[k].append(elapsed)
        print(name + "," + ",".join(f"{statistics.median(samples[k]):.3f}" for k in programs),flush=True)


def pathname_separators(sh, bash, root, env):
    for directory, filename in (("tmp", "normal"), ("tmp\\", "backslash")):
        (root / directory).mkdir()
        (root / directory / filename).touch()
    script = r"""bs='\'
printf '<%s>\n' tmp${bs}/* tm[p]${bs}/* tmp${bs}${bs}/* tm[p]\/n* tmp${bs}/normal
"""
    expected = b"<tmp/normal>\n<tmp\\/backslash>\n<tmp\\/backslash>\n<tmp/normal>\n<tmp\\/normal>\n"
    for shell in (sh, bash):
        result = subprocess.run([shell, "--noprofile", "--norc", "-c", script], cwd=root,
                                env=env, capture_output=True, check=True)
        assert result.stdout == expected, (shell, result)
    print("ok pathname separators from expansion")

    # A pathname separator ends a bracket expression even when shell-quoted.
    # nullglob discards an actual unmatched pattern and preserves plain text.
    script = r"""shopt -s nullglob
set -- [absent\/word] [absent/word] [absent\/] [absent\/word]*; printf '<%s>\n' "$@"
"""
    result = subprocess.run([sh, "--noprofile", "--norc", "-c", script], cwd=root,
                            env=env, capture_output=True, check=True)
    assert result.stdout == b"<[absent/word]>\n<[absent/word]>\n<[absent/]>\n", result
    print("ok pathname brackets separated by a slash")


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
        "quoted array separator order": 'a=(a "b c" 雪); IFS=; printf "<%s>" "${a[*]}${IFS:=:}${a[*]}"',
        "posix literal aliases and deferred subscripts": 'set -o posix\nalias index=\'printf 1\'\nalias other=index\na=(x y); printf \'%s\' "${a[$(other)]}"',
        "flat associative pairs": r"""k='two words'; i=0
declare -A a=(first [bracket]=word 'quoted' '[bracket]=word' dynamic ["$k"]+="$k" side [$((++i))]=$((++i)) [x]=named last odd)
printf '<%s>' "${a[first]}" "${a[quoted]}" "${a[dynamic]}" "${a[side]}" "$i" "${a['[x]=named']}" "${a[odd]+set}:${a[odd]}"
a+=(append [k]=~ tilde name=~)
printf '<%s>' "${a[append]}" "${a[tilde]}"
declare -A 'b=(key [abc]=word)'; printf '<%s>' "${b[key]}" """,
        "array initializer syntax at each binding": r"""f() { a=(first [2]=two); }
declare -a a; f; printf '<%s>' "${a[@]}"; unset a
declare -A a; f; printf '<%s>' "${a[first]}"
a=([first]=one [second]=two); a+=([first]+=two); printf '<%s>' "${a[first]}" "${a[second]}" """,
        "flat pair expansion side effects": r"""i=0
declare -A a=('' $((++i)) k [$((++i))]=$((++i))) 2>/dev/null
printf '<%s>' "$i" "${a[k]}"
a=('' $((++i)) k [$((++i))]=$((++i))) 2>/dev/null
printf '<%s>' "$i" "${a[k]}" """,
        "flat pair process substitution": r"""declare -A a=(k [<(printf x)]=<(printf y))
printf '<%s>' "${a[k]}"
a=(k [<(printf x)]=<(printf y) [<(printf z)]=word done)
case ${a[k]} in ('['/dev/fd/*']=/dev/fd/'*) printf '<direct>';; (*) printf '<unexpected>';; esac
printf '<%s>' "${a['[<(printf z)]=word']}" """,
        "nameref array counts": r"""a=([2]=s [55]=last); declare -A m=([a]=first [b]=next)
declare -n ref=a rr=ref mref=m none=absent elem='a[2]'
printf '<%s>' "${#ref[@]}" "${#ref[*]}" "${#rr[@]}" "${#mref[*]}" "${#none[@]}" "${#elem[@]}" """,
        "whole-array target length": """a=([2]=s [55]=last); declare -n all='a[@]'; printf '%s' "${#all}" """,
        "count materializes dynamic group IDs": r"""declare -n ref=GROUPS; : "${#ref[@]}"
set | grep -E '^GROUPS=\(\[0\]' >/dev/null; printf '%s' "$?" """,
        "first expansion materializes group IDs": r"""seen() { set | grep -E '^GROUPS=\(\[0\]' >/dev/null; printf '%s' "$?"; }
seen
( : "${GROUPS[@]}"; seen )
( : ${GROUPS[*]}; seen )
( : "before${GROUPS[@]}after"; seen )
( : "${!GROUPS[@]}"; seen )
( : "${GROUPS[@]@Q}"; seen )
( declare -n group_ref=GROUPS; : "${group_ref[@]}"; seen )
( a=("${GROUPS[@]}"); seen )
seen""",
        "flat pair tilde contexts": r"""declare -A a=(key path=bin:~/tools start ~:~/b)
printf '<%s>' "${a[key]}" "${a[start]}"
a=(key path=bin:~/tools equals path=~/bin:~/tools slash bin:~/tools '~:~' 'literal:~' x:~/key data start ~:~/b esc x\:~/a q x:"~"/b)
printf '<%s>' "${a[key]}" "${a[equals]}" "${a[slash]}" "${a['~:~']}" "${a['x:~/key']}" "${a[start]}" "${a[esc]}" "${a[q]}"
declare -A 'b=(key path=bin:~/tools)'; printf '<%s>' "${b[key]}" """,
        "dynamic associative field snapshots": r"""alias aa='OLD alias'
printf '<%s>' "${BASH_ALIASES[@]}${BASH_ALIASES[bb]:=NEW}"
unalias bb; printf '<%s>' "${!BASH_ALIASES[@]}${BASH_ALIASES[bb]:=NEW}"
hash -r; hash -p /old/path aa
printf '<%s>' "${BASH_CMDS[@]}${BASH_CMDS[bb]:=/new/path}"
hash -d bb; printf '<%s>' "${!BASH_CMDS[@]}${BASH_CMDS[bb]:=/new/path}" """,
        "expanded array element append": r"""v=' : '; a=([4]=first); i=4; a[i++]+=$v
declare -A m=([a]=x); key=a; m[$key]+=${v}; declare -n ref='a[4]'; ref+=Q
(a[4]+=$v; printf '<%s>' "${a[4]}")
printf '<%s>' "$i" "${a[4]}" "${m[a]}"
declare -ai n=(6); n[0]+=2; declare -au u=(a); u[0]+=$key; printf '<%s>' "${n[0]}" "${u[0]}" """,
        "bulk indexed append": r"""src=('a b' '' tail); a=([9]=old); a+=("${src[@]}"); printf '<%s>' "${!a[@]}" "${a[@]}"
a=(); a+=("${src[@]}"); export a+=(end); printf '<%s>' "${a[@]}"
mapfile -t r <<< data; r+=("${src[@]}"); printf '<%s>' "${r[@]}" """,
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
        pathname_separators(sh, bash, Path(scratch), env)
        # A replacement's += must see the preceding write to the same key.
        # Bash 5.3.20 appends to the old table while building its replacement.
        script = 'declare -A a=([k]=old); a=([k]=one [k]+=two); printf "%s" "${a[k]}"'
        result = subprocess.run([sh, "--noprofile", "--norc", "-c", script], cwd=scratch,
                                env=env, capture_output=True, check=True)
        assert result.stdout == b"onetwo", result
        print("ok associative replacement appends to the preceding write")
        version = subprocess.check_output([bash, "-c", 'printf "%s.%s" "${BASH_VERSINFO[0]}" "${BASH_VERSINFO[1]}"'], env=env, text=True)
        for name, script in cases.items():
            options = env
            if name == "alias print with operands":
                options = dict(env, BASH_COMPAT=version)

            if name in ("brace groups in current substitution", "whole-array target length") and tuple(map(int,version.split("."))) < (5,3):
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
