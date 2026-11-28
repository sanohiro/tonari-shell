#!/usr/bin/env python3
"""Check stored array subscripts, nameref counters, slice operands and UTF-8 quoting."""

import argparse
import os
from pathlib import Path
import random
import statistics
import subprocess
import sys
import time


CASES = {
    "stored parameter transformations": (
        """color=blue; args=(a b); a=(five seven nine); declare -A m=([BLUE]=31 [blu]=41 [blew]=51)
declare -n upper='m[${color^^}]' trim='m[${color%e}]' replace='m[${color/ue/ew}]' n='a[${#args[@]}]'
echo "$upper/$((upper))/$trim/$replace/$n"
shopt -u assoc_expand_once; unset 'm[${color^^}]' 'a[${#args[@]}]'; echo "${#m[@]}/${a[*]}" """,
        "31/31/41/51/nine\n2/five seven\n"),
    "active arithmetic nameref index": (
        """a=(5 7); i=0; declare -n ref='a[i++]'; echo "$((ref))/$i"
i=0; ((n=ref)); echo "$n/$i"; i=0; ((ref+=3)); echo "${a[*]}/$i" """,
        "5/1\n5/1\n5 8/2\n"),
    "quoted initializer index": (
        """j=1; a=(['1'+'2']=x ['$j']=y [$'4']=z); echo "${!a[@]}/${a[*]}" """, "1 3 4/y x z\n"),
    "missing array element lengths": (
        """set -u; a=(); declare -A m=([x]=1); echo "${#a[4]}/${#m[other]}" """, "0/0\n"),
    "invalid arithmetic target preserves RHS": (
        """n=0; ((1=n++)) 2>/dev/null; ((2+=n++)) 2>/dev/null; echo "$n" """, "0\n"),
    "unset invalid transform": (
        """unset x; set -- "${x@Z}"; echo "$#/$1" """, "1/\n"),
    "nameref loop counter": (
        """a=11 b=22 c=33; declare -n r=a
for r in b c; do printf '%s/' "$r"; done; echo "$a/$b/$c/${!r}"
declare -n empty; for empty in c b a; do printf '%s/' "$empty"; done; echo "$a/$b/$c" """,
        "22/33/11/22/33/c\n33/22/11/11/22/33\n"),
    "associative lookup in an indexed target": (
        """declare -A t=([slot]=1); a=(10 20); ref='a[t[slot]]'; declare -n r="$ref"
echo "${!ref}/$r/$((r))"; printf -v "$ref" %s 30; echo "${a[*]}"
IFS= read -r "$ref" <<<40; echo "${a[*]}"; r=50; echo "${a[*]}" """,
        "20/20/20\n10 30\n10 40\n10 50\n"),
    "arithmetic and ordinary references resolve the same key": (
        """declare -A m=([apple]=4); key=apple; declare -n r='m[$key]'; ref='m[$key]'
printf '%s/' "$r" "$((r))" "${#r}" "${!ref}"; ((r*=2)); echo "${m[apple]}"
r=13; ((r+=3)); echo "${!ref}" """, "4/4/1/4/8\n16\n"),
    "stored indexed and associative expressions": (
        """a=(5 7 9); j=1; declare -A m=([slot]=2)
for v in 'a[$j]' 'a[${j}]' 'a[${z:-1}]' 'a[$((1+1))]' 'a[m[slot]]' 'm[${z:-slot}]'; do
    printf '%s/' "$((v))"
done; echo""", "7/7/7/9/9/2/\n"),
    "quoted slice offsets and lengths": (
        """x=abcdef; n=1; a=(one two three)
printf '%s/' "${x:"1":"2"}" "${x:"$n"}" "${x: "1"}" "${a[@]:"1"}"; echo""",
        "bc/bcdef/bcdef/two/three/\n"),
    "nameref defaults and indirect keys": (
        """key='two words'; declare -A m; declare -n r='m[$key]'; ref='m[$key]'
echo "${r:=word}/$r/${#r}/${!ref}"; r+=s; echo "${!ref}"
a=(one two); i=0; declare -n e='a[i++]'; echo "$e/$i"; unset 'a[$i]'; echo "${a[*]}" """,
        "word/word/4/word\nwords\none/1\none\n"),
}

WORKLOADS = {
    "stored array length subscript (12000)": (
        """args=(a b); a=(5 7 9); declare -n ref='a[${#args[@]}]'
for ((i=0;i<12000;i++)); do n=$((ref)); done; echo "$n" """, "9\n"),
    "stored associative arithmetic (12000)": (
        """declare -A m=([apple]=4); key=apple; declare -n ref='m[$key]';
for ((i=0;i<12000;i++)); do n=$((ref)); done; echo "$n" """, "4\n"),
    "stored indexed arithmetic (12000)": (
        """a=(4 8); key=1; ref='a[$key]';
for ((i=0;i<12000;i++)); do n=$((ref)); done; echo "$n" """, "8\n"),
    "quoted slice (12000)": (
        """x=abcdef; n=1; for ((i=0;i<12000;i++)); do v=${x:"$n":"2"}; done; echo "$v" """,
        "bc\n"),
}


def run(shell, source, expected, env):
    result = subprocess.run([shell, "--noprofile", "--norc", "-c", source],
                            env=env, capture_output=True, timeout=30)
    if (result.returncode, result.stdout, result.stderr) != (0, expected.encode(), b""):
        raise ValueError(f"{shell}: {result.returncode}, {result.stdout!r}, {result.stderr!r}")


def verify(programs, env, reference_version):
    for name, (source, wanted) in CASES.items():
        for label, shell in programs.items():
            expected = wanted
            if label == "bash" and reference_version.startswith("5.2."):
                if name == "nameref defaults and indirect keys":
                    continue  # Debian Bash terminates on this valid expression.
                if name == "arithmetic and ordinary references resolve the same key":
                    expected = "4/4/0/4/8\n16\n"
            run(shell, source, expected, env)

    source = """declare -A m; m["$key"]=5; a=(0 1 2 3 4 9)
r='m[$key]'; declare -n n='m[$key]'; index='a[m[$key]]'
echo "${!r}/$n/$((n))/$((index))"; ((n+=2)); echo "${!r}" """
    for key in ("a]b", "a'b", 'a"b', "a[b", "$price", "two words", "foo\\bar", "@", "*"):
        for shell in programs.values():
            run(shell, source, "5/5/5/9\n7\n", dict(env, key=key))

    for shell in programs.values():
        result = subprocess.run([shell, "--noprofile", "--norc", "-c",
                                 """a=(10); declare -n r='a[1/0]'; echo "$((r+1))" """],
                                env=env, capture_output=True, timeout=30)
        assert result.returncode != 0 and not result.stdout and result.stderr, (shell, result)

    failure_cases = [
        ("""set -u; a=(1); i='a[$missing]'; echo "$((i))" """, 127, "unbound variable"),
        ('x=word; echo "${x@Z}"\necho continued', 127, "bad substitution"),
        ('declare -n r; echo "${!r}"', 1, "invalid indirect expansion"),
        ('set -u; declare -A m; echo "${#m[key]}"', 127, "unbound variable"),
        ('set -u; echo "${#missing}"', 127, "missing: unbound variable"),
    ]
    for label, shell in programs.items():
        for source, status, message in failure_cases:
            if label == "bash" and reference_version.startswith("5.2.") and "declare -A m;" in source:
                status = 1
            result = subprocess.run([shell, "--noprofile", "--norc", "-c", source], env=env,
                                    capture_output=True, timeout=30)
            assert (result.returncode, result.stdout) == (status, b""), (shell, source, result)
            assert message.encode() in result.stderr, (shell, source, result)
    for text in ("e\u0301", "❤️", "A\u00a0B", "\ue000"):
        env["v"] = text
        source = """x=$v; declare -p x; printf '%q\\n' "$x"; echo "${x@Q}"; set | while IFS= read -r line; do case $line in x=*) echo "$line";; esac; done"""
        wanted = f"""declare -- x="{text}"\n{text}\n'{text}'\nx={text}\n"""
        for shell in programs.values():
            run(shell, source, wanted, env)
    for shell in programs.values():
        run(shell, 'x=$v; printf "%q" "$x"', "$'\\302\\205'", dict(env, v="\u0085"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tonari", type=Path)
    parser.add_argument("bash", type=Path)
    parser.add_argument("--first", type=Path, help="also time an earlier correct implementation")
    parser.add_argument("--benchmark", action="store_true")
    args = parser.parse_args()
    env = dict(os.environ, LC_ALL="en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8")
    env.pop("BASH_ENV", None)
    programs = {"tonari": args.tonari.resolve(), "bash": args.bash.resolve()}
    version = subprocess.check_output([programs["bash"], "-c",
                                       'printf "%s.%s.%s" "${BASH_VERSINFO[@]:0:3}"'],
                                      text=True, env=env)
    if int(version.split(".")[0]) < 5:
        del programs["bash"]
    verify(programs, env, version)
    print("ok stored subscripts, nameref loops, quoting and error handling")

    if args.benchmark:
        if args.first:
            programs = {"first": args.first.resolve(), **programs}
        rng = random.Random(9)
        print("workload," + ",".join(programs))
        for name, (source, expected) in WORKLOADS.items():
            samples = {k: [] for k in programs}
            for trial in range(16):
                order = list(programs)
                rng.shuffle(order)
                for k in order:
                    start = time.perf_counter_ns()
                    run(programs[k], source, expected, env)
                    elapsed = (time.perf_counter_ns() - start) / 1e6
                    if trial:
                        samples[k].append(elapsed)
            print(name + "," + ",".join(f"{statistics.median(samples[k]):.3f}"
                                       for k in programs), flush=True)


if __name__ == "__main__":
    main()
