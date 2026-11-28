#!/usr/bin/env python3
"""Check language state and time public entry points with three-point size sweeps."""

import argparse
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile
import time


def launch(shell, args, env, root, data=None):
    return subprocess.run([shell, "--noprofile", "--norc", *args], input=data,
                          cwd=root, env=env, capture_output=True, timeout=30)


def check(label, result, output):
    if (result.returncode, result.stdout, result.stderr) != (0, output.encode(), b""):
        raise ValueError(f"{label}: {result.returncode}, {result.stdout!r}, {result.stderr!r}")


def state_checks(shells, env, root):
    checks = {
        "nested quote and live parameters": (
            'v=blue\ns="one\n$v $$\n$(printf "%s" "$v")\n${v:-none}\n$[1+2]\nend"\n'
            '[[ $s == *"$v $$"* && $s == *$\'blue\\nblue\\n3\\nend\' ]] && echo ok', "ok\n"),
        "character counts after rebinding": (
            'v=日本; w=abc; declare -n ref=v rr=ref; n=${#rr}; echo "$n"; '
            'v+=語; echo "${#v}/${#rr}"; '
            'f(){ local v=hello; echo "${#rr}"; v=; echo "${#rr}"; }; f; '
            'declare -n ref=w; echo "${#rr}"; w=(雪); echo "${#rr}/${#ref[@]}"',
            "2\n3/3\n5\n0\n3\n1/1\n"),
        "case pattern remains live": (
            'subject=word; p=word; case "$subject" in "$p") :;; *) exit 1;; esac; '
            'p="w*"; case $subject in "$p") exit 2;; $p) :;; *) exit 3;; esac; '
            'p=WORD; shopt -s nocasematch; case $subject in $p) :;; *) exit 4;; esac; '
            'p=none; case $subject in w*) p=word ;;& $p) echo live;; esac', "live\n"),
        "regex suffix, captures and errors": (
            r'''match(){ [[ $1 =~ $2 ]] 2>/dev/null; printf '%s/%s/<%s>/<%s>\n' "$?" "${#BASH_REMATCH[@]}" "${BASH_REMATCH[0]}" "${BASH_REMATCH[1]}"; }
match ffhit '^f+hit$'; BASH_REMATCH[7]=old; match ffhix '^f+hit$'
match ffxhit '^f+hit$'; match foo 'foo|barhit$'; match barhit '^(foo|bar)hit$'
match xxhit '^[[:alpha:]]+hit$'; match 'hit$' '^hit\$$'; match $'ffhit\n' '^f+hit$'
shopt -s nocasematch; match FFHIT '^f+hit$'; match x '['; shopt -u nocasematch
match ffhit '^f+(hit)$'; match 'f.hit' '^f\.hit$' ''',
            "0/1/<ffhit>/<>\n1/0/<>/<>\n1/0/<>/<>\n0/1/<foo>/<>\n0/2/<barhit>/<bar>\n"
            "0/1/<xxhit>/<>\n0/1/<hit$>/<>\n1/0/<>/<>\n0/1/<FFHIT>/<>\n"
            "2/1/<FFHIT>/<>\n0/2/<ffhit>/<hit>\n0/1/<f.hit>/<>\n"),
        "quoted comparison and side effects": (
            'set -- "a*b"; left=ab; right="a*b"; declare -n r=right; '
            '[[ $left == $r && $left != "$r" && $1 == "$r" ]] || exit 1; '
            'shopt -s nocasematch; left="A*B"; [[ $left == "$right" ]] || exit 2; '
            'unset x; [[ ${x:=word} == "$x" ]] && echo ok', "ok\n"),
        "arithmetic short circuit": (
            'i=0; echo "$((1 || i++))/$((0 && (2 / 0)))/$i"; '
            '(echo "$((0 ? 08 : 1))") 2>/dev/null; [[ $? != 0 ]] && echo rejected',
            "1/0/0\nrejected\n"),
        "eval cache and scope": (
            "value=global; f(){ local value=$1; eval 'printf \"%s\\n\" \"$value\"'; }; "
            "f one; f two; f three; echo \"$value\"", "one\ntwo\nthree\nglobal\n"),
        "dense, sparse and associative bindings": (
            'a=(one two); unset "a[0]"; a[8]=eight; declare -A m=([k]=old); '
            'f(){ local -n slot=m; slot[k]=new; printf "%s/" "${a[@]:2:1}"; }; '
            'f; echo "${m[k]}/${!a[@]}"', "eight/new/1 8\n"),
        "pathname prefix, options and fresh entries": (
            'cd prefix || exit; shopt -s nullglob; set -- item-*.txt; echo "$#/$1"; '
            'touch item-03.txt; set -- item-*.txt; echo "$#/${2}"; rm item-03.txt; '
            'GLOBIGNORE=item-04.txt; set -- item-*.txt; echo "$#/$1"',
            "2/item-02.txt\n3/item-03.txt\n1/item-02.txt\n"),
    }
    local_match = (
        '[[ outer =~ (out) ]]; g(){ local -a BASH_REMATCH=(g); [[ inner =~ (inn) ]]; '
        'echo "${BASH_REMATCH[*]}"; }; f(){ local -a BASH_REMATCH=(f); g; '
        'echo "${BASH_REMATCH[*]}"; }; f; echo "${BASH_REMATCH[*]}"'
    )
    feature_script = 'cd numbers; GLOBSORT=numeric; a=(*); [[ ${a[0]} == 2 ]]'
    have_numeric = [launch(shell, ["-c", feature_script], env, root).returncode == 0
                    for shell in shells]
    for label, (script, expected) in checks.items():
        for shell in shells:
            check(f"{label}/{shell}", launch(shell, ["-c", script], env, root), expected)
    pipe_script = (b'v=blue\ns="start\n$v\nend"\nread -r item\nrecord for read\n'
                   b'printf "<%s>\\n" "$item"\ncat\nrecord for child\n')
    for i, shell in enumerate(shells):
        check(f"shared command pipe/{shell}", launch(shell, [], env, root, pipe_script),
              "<record for read>\nrecord for child\n")
        script = 'w=(雪); declare -n elt="w[0]"; n=${#elt}; echo "$n"'
        result = launch(shell, ["-c", script], env, root)
        if i == 2 and not have_numeric[i] and result.stdout == b"0\n":
            check(f"legacy Bash element reference/{shell}", result, "0\n")
            print("# reference reports zero for the length of a nameref to an array element")
        else:
            check(f"element reference length/{shell}", result, "1\n")
        if have_numeric[i]:
            # This changed along with GLOBSORT in Bash 5.3.
            check(f"local BASH_REMATCH/{shell}", launch(shell, ["-c", local_match], env, root),
                  "inn inn\nf\nout out\n")

    script = ('cd huge; for GLOBSORT in numeric -numeric; do '
              'a=(*); printf "%s\\n" "${a[*]}"; done')
    names = sorted((p.name for p in (root / "huge").iterdir()),
                   key=lambda s: (0, int(s), s) if s.isascii() and s.isdigit() else (1, 0, s))
    expected = " ".join(names) + "\n" + " ".join(reversed(names)) + "\n"
    check("arbitrary-length numeric filename sort", launch(shells[1], ["-c", script], env, root),
          expected)
    for label, shell, supported in zip(("before", "after", "bash"), shells, have_numeric):
        if supported:
            result = launch(shell, ["-c", script], env, root)
            if result.returncode or result.stderr:
                raise ValueError(f"{label} numeric fixture: {result}")
            print(f"# numeric ordering with overflow and ties, {label}: "
                  f"{'matches' if result.stdout.decode() == expected else 'differs'}")
        else:
            print(f"# {label} does not implement GLOBSORT; omitting its numeric timings")
    return have_numeric


def workloads():
    for n in (192, 384, 768):
        for token in ("$v", "${v:-none}", "word"):
            script = 'v=word\ns="start\n' + (token + ' ordinary line\n') * n + 'end"\necho "${#s}"'
            expected = str(9 + n * len("word ordinary line\n"))
            for mode in (("file", "pipe", "eval") if token == "$v" else ("file",)):
                yield f"double quote {token}/{mode}", n, script, expected, mode
    for n in (1024, 2048, 4096):
        for counter in ("n=${#s}", ":"):
            script = ("s=雪; for ((i=0;i<" + str(n) + ";i++)); do s+=日本語; "
                      + counter + '; done; echo "${#s}"')
            yield "grow plus " + counter, n, script, str(1 + n * 3), "file"
    for n in (2048, 4096, 8192):
        for name in ("s", "ref"):
            script = ("s='" + "界" * n + "'; declare -n ref=s; for ((i=0;i<3000;i++)); "
                      "do n=${#" + name + '}; done; echo "$n"')
            yield "length of " + name, n, script, str(n), "file"
    for n in (16384, 32768, 65536):
        for rhs in ('"$b"', "z"):
            script = (f"printf -v s '%0{n}d' 0; b=z; for ((i=0;i<3000;i++)); "
                      f'do [[ $s != {rhs} ]] || exit 1; done; echo "$i"')
            yield "equality " + rhs, n, script, "3000", "file"
    for n in (128, 256, 512):
        setup = f"cd numeric-{n} || exit; GLOBSORT=numeric; "
        body = 'for ((i=0;i<60;i++)); do a=(*); done; echo "${#a[@]}/${a[0]}/${a[-1]}"'
        yield "numeric pathname sort", n, setup + body, f"{n}/0/{n-1}", "numeric"
    for n in (32, 64, 128):
        variables = "".join(f"p{i}=key{i}\n" for i in range(n))
        for kind in ("static", "variable"):
            patterns = [f"key{i}" if kind == "static" else f"$p{i}" for i in range(n)]
            branches = "".join(f"{p}) chosen={i};;\n" for i, p in enumerate(patterns))
            for position, target in (("first", 0), ("last", n - 1), ("miss", n)):
                script = (variables + f"key=key{target}\nfor ((i=0;i<100;i++)); do\n"
                          + "case $key in\n" + branches + "*) chosen=miss;; esac\ndone\necho \"$chosen\"")
                yield f"case {kind} {position}", n, script, (str(target) if target < n else "miss"), "file"
    for n in (512, 1024, 2048):
        start = "s=" + "f" * n + "hix; wanted=" + "f" * n + "hit; "
        for kind, expr in (("literal", '[[ $s == "$wanted" ]]'),
                           ("glob", "[[ $s == f*hit ]]"),
                           ("regex", "[[ $s =~ ^f+hit$ ]]")):
            script = start + f"for ((i=0;i<150;i++)); do {expr} && exit 1; done; echo miss"
            yield f"near-tail miss {kind}", n, script, "miss", "file"
        start = "s=" + "f" * n
        yield "regex present suffix/internal miss", n, (
            start + 'xhit; for ((i=0;i<150;i++)); do [[ $s =~ ^f+hit$ ]] && exit 1; done; '
            'echo "${#BASH_REMATCH[@]}"'), "0", "file"
        yield "regex capture hit", n, (
            start + 'hit; for ((i=0;i<150;i++)); do [[ $s =~ ^f+(hit)$ ]] || exit 1; done; '
            'echo "${#BASH_REMATCH[0]}/${BASH_REMATCH[1]}"'), f"{n + 3}/hit", "file"
    for axis, sizes in (("matches", (128, 256, 512)), ("replacement bytes", (8, 16, 32))):
        for n in sizes:
            matches = n if axis == "matches" else 128
            replacement = "!" + "X" * (1 if axis == "matches" else n - 1)
            text = "AB" * matches + "a" * (4096 - 2 * matches)
            output = text.replace("AB", replacement)
            script = (f"s={text}; repl='{replacement}'; "
                      'for ((i=0;i<16;i++)); do out=${s//AB/$repl}; done; '
                      'marks=${out//[^!]/}; echo "${#marks}/${#out}/${out:0:8}/${out: -8}"')
            yield f"replace {axis}", n, script, f"{matches}/{len(output)}/{output[:8]}/{output[-8:]}", "file"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shells", metavar="SHELL", type=Path, nargs=3, help="before, after, Bash")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--case", help="time only workloads whose label includes this text")
    parser.add_argument("--checks-only", action="store_true")
    args = parser.parse_args()
    shells = [p.resolve() for p in args.shells]
    with tempfile.TemporaryDirectory() as dirname:
        root = Path(dirname)
        env = {"HOME": dirname, "LC_ALL": "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8",
               "PATH": "/usr/bin:/bin"}
        groups = {"prefix": ["item-02.txt", "item-04.txt", "other-02.txt", "item-05.md"],
                  "numbers": ["2", "10"],
                  "huge": ["0", "00", "02", "2", "08", "alpha", "+4", "90000000000000000000",
                           "100000000000000000000", "090000000000000000000",
                           "100000000000000000000a"]}
        for n in (128, 256, 512):
            groups[f"numeric-{n}"] = [str(i) for i in range(n)]
        for folder, names in groups.items():
            (root / folder).mkdir()
            for name in names:
                (root / folder / name).touch()
        supported = state_checks(shells, env, root)
        if args.checks_only:
            return
        print("case\tsize\tbefore_ms\tafter_ms\tbash_ms")
        for label, n, script, expected, mode in workloads():
            if args.case and args.case not in label:
                continue
            script += "\n"
            file = root / "script.sh"
            file.write_text(script, encoding="utf-8")
            command = (["-c", 'eval "$1"', "eval-probe", script] if mode == "eval"
                       else [] if mode == "pipe" else [str(file)])
            data = script.encode() if mode == "pipe" else None
            timings = [[] for _ in shells]
            for trial in range(args.runs + 1):
                for step in range(len(shells)):
                    k = (trial + step) % len(shells)
                    if mode == "numeric" and not supported[k]:
                        continue
                    start = time.perf_counter_ns()
                    result = launch(shells[k], command, env, root, data)
                    ms = (time.perf_counter_ns() - start) / 1e6
                    check(f"{label}/{shells[k]}", result, expected + "\n")
                    if trial:
                        timings[k].append(ms)
            print(label, n, *(f"{statistics.median(v):.3f}" if v else "unsupported"
                              for v in timings), sep="\t", flush=True)


if __name__ == "__main__":
    main()
