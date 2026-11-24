"""Compare complete script launches and sourcing the same files after shell startup."""

import argparse
from pathlib import Path
import select
import sys
import subprocess
import tempfile
import time

from bench import report


def split_case(separator):
    row = separator.join(["x" * 128] * 32)
    source = (f"IFS='{separator}'; row='{row}'\n"
              'for ((i=0; i<400; i++)); do set -- $row; done\necho "$# ${#1} ${#32}"')
    return source, "32 128 128"


# The last command prints a result that the driver checks on every run.
CASES = {
    "arithmetic": ("""\
sum=0
for ((i=1; i<=10000; i++)); do ((sum += i)); done
echo "$sum"
""", "50005000"),
    "functions": ("""\
add() { local n=$1; ((sum += n)); }
sum=0
for ((i=1; i<=4000; i++)); do add "$i"; done
echo "$sum"
""", "8002000"),
    "function-definitions": (
        "".join(f"f{i}() {{ echo {i}; }}\n" for i in range(2000)) + "f1999", "1999",
    ),
    "function-redefinitions": (
        "".join(f"f() {{ echo {i}; }}\n" for i in range(2000)) + "f", "1999",
    ),
    "function-wrapper-redefinitions": (
        "".join(f"""f() {{ printf '%s:%s\\n' '{i}' "$@"; }}\n""" for i in range(2000))
        + "f '雪 x'", "1999:雪 x",
    ),
    "function-compound-redefinitions": (
        "".join(f'f() {{\n  local n=$1\n  if ((n)); then echo {i}; else echo other; fi\n}}\n'
                for i in range(2000)) + "f 1", "1999",
    ),
    "function-loop-redefinitions": (
        'for ((i=0; i<2000; i++)); do\n'
        '  f() { local n=$1; if ((n)); then echo "$i:$n"; else echo other; fi; }\n'
        'done\nf 7', "2000:7",
    ),
    "strings": ("""\
for ((i=0; i<2000; i++)); do
    path="/var/log/worker-${i}.log"
    base=${path##*/}
    stem=${base%.log}
done
echo "$stem"
""", "worker-1999"),
    "regex": ("""\
re='^([^/]+)/([[:digit:]]+)[.]txt$'
for ((i=0; i<1000; i++)); do
    [[ dir/$i.txt =~ $re ]] || exit 1
done
echo "${BASH_REMATCH[1]}/${BASH_REMATCH[2]}"
""", "dir/999"),
    "case-edges": ("""\
for ((i=0; i<3000; i++)); do
    case src/unit-$i.rs in
        docs/*.md|tests/*.py) exit 1 ;;
        src/*.rs) last=$i ;;
        *) exit 1 ;;
    esac
done
echo "$last"
""", "2999"),
    "nocasematch": ("""
shopt -s nocasematch
for ((i=0; i<1500; i++)); do
    name=Src/Node-$i.RS
    case $name in src/*.rs) last=${name/#src/lib};; *) exit 1;; esac
    [[ $last == LIB/* ]] || exit 1
done
echo "$last"
""", "lib/Node-1499.RS"),
    "case-classes": ("""\
for ((i=0; i<3000; i++)); do
    case src/unit-$i.rs in
        src/unit-[0-9]*.rs) last=$i ;;
        *) exit 1 ;;
    esac
done
echo "$last"
""", "2999"),
    "glob": ("""\
for ((i=0; i<30; i++)); do set -- fixtures/*.txt; done
echo "$# $1"
""", "512 fixtures/item-000.txt"),
    "glob-sparse": ("""\
for ((i=0; i<30; i++)); do set -- fixtures/item-000*.txt; done
echo "$# $1"
""", "1 fixtures/item-000.txt"),
    "glob-all": ("""\
for ((i=0; i<30; i++)); do set -- fixtures/*; done
echo "$# $1"
""", "512 fixtures/item-000.txt"),
    "glob-multi-star": ("""\
for ((i=0; i<30; i++)); do set -- fixtures/*em*07.txt; done
echo "$# $1"
""", "6 fixtures/item-007.txt"),
    "glob-subdirs": ("""\
for ((i=0; i<30; i++)); do set -- mixed/*/*.rs; done
echo "$# $1"
""", "8 mixed/pkg-0/lib.rs"),
    "glob-utf8-sort": ("""\
for ((i=0; i<30; i++)); do set -- localized/*.txt; done
echo "$#"
""", "512"),
    "glob-utf8-once": ('set -- localized/*.txt; echo "$#"', "512"),
    "glob-utf8-large-repeat": ('for ((i=0; i<8; i++)); do set -- large/*.txt; done; echo "$#"', "4096"),
    "glob-utf8-large": ('set -- large/*.txt; echo "$#"', "4096"),
    "glob-ignore-large": ('GLOBIGNORE="large/B-*.txt"; for ((i=0; i<8; i++)); do set -- large/*.txt; done; echo "$#"', "3584"),
    "glob-nocase-large": ('shopt -s nocaseglob; for ((i=0; i<8; i++)); do set -- large/*.TXT; done; echo "$#"', "4096"),
    "glob-recursive": ("""\
shopt -s globstar
for ((i=0; i<10; i++)); do set -- tree/**/*.rs; done
echo "$# $1"
""", "80 tree/d00/s0/lib.rs"),
    "read-lines": ("""\
count=0
while IFS= read -r row; do
    ((count++))
    last=$row
done < rows
echo "$count $last"
""", "2000 record 1999"),
    "substitutions": ("""\
for ((i=0; i<64; i++)); do value=$(printf 'v%s' "$i"); done
echo "$value"
""", "v63"),
    "echo-substitutions": ("""
for ((i=0; i<64; i++)); do value=$(echo "v$i"); done
echo "$value"
""", "v63"),
    "mapfile-lines": ("""
for ((i=0; i<16; i++)); do mapfile -t table <rows; done
echo "${#table[@]}:${table[1999]}"
""", "2000:record 1999"),
    "case-fallthrough": ("""
for ((i=0; i<1000; i++)); do
    case x in x) v=1 ;& y) v=2 ;;& y) v=bad ;; x) v+=3 ;; esac
done
echo "$v"
""", "23"),
    "extended-patterns": ("""
shopt -s extglob
for ((i=0; i<500; i++)); do case pkg-${i}.rs in pkg-+([0-9]).@(c|rs)) last=$i;; esac; done
echo "$last"
""", "499"),
    "resource-limits": ("""
for ((i=0; i<256; i++)); do ulimit -Sn 64; ulimit -Sn > /dev/null; done
ulimit -Sn
""", "64"),
    "directory-stack": ("""
for ((i=0; i<200; i++)); do pushd -n dir >/dev/null; popd -n >/dev/null; done
echo "${#DIRSTACK[@]}"
""", "1"),
    "descriptor-moves": ("""
for ((i=0; i<128; i++)); do exec 7<>rows; exec 6<&7-; exec 6<&-; done
test -s rows && echo moved
""", "moved"),
    "associative-lookup": ("""
declare -A values=([left]=1 ["right hand"]=2)
for ((i=0; i<1200; i++)); do k="right hand"; values[left]=${values[$k]}; done
echo "${values[left]}:${#values[@]}"
""", "2:2"),
    "readonly-locals": ("""
check(){ local -r item="$1"; result=$item; }
for ((i=0; i<1000; i++)); do check "$i"; done
echo "$result"
""", "999"),
    "error-trap": ("""
n=0; trap '((n+=1))' ERR
for ((i=0; i<600; i++)); do false; done
echo "$n"
""", "600"),
    "case-conversion": ("""
v='two WORDS plus 42'; for ((i=0; i<1200; i++)); do upper=${v^^}; lower=${upper,,}; done
echo "$lower"
""", "two words plus 42"),
    "integer-assignments": ("""
declare -i n=1
for ((i=0; i<2000; i++)); do n+=3; done
echo "$n"
""", "6001"),
    "brace-range": ("""
for ((i=0; i<16; i++)); do set -- {0001..1000}; done
echo "$# $1 ${1000}"
""", "1000 0001 1000"),
    "getopts-clusters": ("""
for ((i=0; i<128; i++)); do
    OPTIND=1
    while getopts 'ab:c' opt -abvalue -c; do last=$opt; done
done
echo "$OPTIND:$last"
""", "3:c"),
    "compound-substitutions": ("""\
for ((i=0; i<16; i++)); do value=$(printf v; printf '%s' "$i"); done
echo "$value"
""", "v15"),
    "external-substitutions": ("""\
for ((i=0; i<16; i++)); do value=$(/usr/bin/printf 'v%s' "$i"); done
echo "$value"
""", "v15"),
    "parenthesized-pipeline": ("""\
for ((i=0; i<32; i++)); do
    printf '%s\\n' "$i" | ( read -r value && [[ $value == "$i" ]] ) || exit 1
done
echo "$i"
""", "32"),
    "pipeline": (
        "for ((i=0; i<16; i++)); do /usr/bin/printf '%s\\n' \"$i\" | /bin/cat || exit 1; done",
        "\n".join(map(str, range(16))),
    ),
    "merged-pipeline": (
        "for ((i=0; i<16; i++)); do /usr/bin/printf '%s\\n' \"$i\" |& /bin/cat || exit 1; done",
        "\n".join(map(str, range(16))),
    ),
    "redirect-stderr": (
        "for ((i=0; i<16; i++)); do /usr/bin/printf '%s\\n' \"$i\" 2>&1 || exit 1; done",
        "\n".join(map(str, range(16))),
    ),
    "merged-pipeline-redirect": (
        "for ((i=0; i<16; i++)); do /usr/bin/printf '%s\\n' \"$i\" |& /bin/cat 2>&1 || exit 1; done",
        "\n".join(map(str, range(16))),
    ),
    "builtin-merged-pipeline": (
        "for ((i=0; i<16; i++)); do printf '%s\\n' \"$i\" |& /bin/cat || exit 1; done",
        "\n".join(map(str, range(16))),
    ),
    "builtin-output": (
        "for ((i=0; i<16; i++)); do printf '%s\\n' \"$i\" || exit 1; done",
        "\n".join(map(str, range(16))),
    ),
    "field-split": split_case(":"),
    "field-split-unicode": split_case("😀"),
    "long-trim": (
        "path='dir/" + "x" * 2048 + "'\n"
        'for ((i=0; i<400; i++)); do base=${path##*/}; done\necho "${#base}"',
        "2048",
    ),
    "external-env": (
        "export " + " ".join(f"BENCH_KEY_{i}=value{i}" for i in range(32)) + "\n"
        "for ((i=0; i<16; i++)); do /usr/bin/printenv BENCH_KEY_31 || exit 1; done\n",
        "\n".join(["value31"] * 16),
    ),
    "external": ("""\
for ((i=0; i<16; i++)); do /usr/bin/true || exit 1; done
echo "$i"
""", "16"),
}


def sample(binary, script, expected, env, started, setup=""):
    argv = [binary, "--noprofile", "--norc"]
    if started:
        argv.extend(["-c", setup + 'printf "ready\\n"; IFS= read -r gate && . "$1"', "bench", script])
    elif setup:
        argv.extend(["-c", setup + '. "$1"', "bench", script])
    else:
        argv.append(script)
    start = time.perf_counter_ns()
    with subprocess.Popen(argv, env=env, cwd=script.parent, stdin=subprocess.PIPE,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE) as child:
        try:
            if started:
                if not select.select([child.stdout], [], [], 10)[0] or child.stdout.readline() != b"ready\n":
                    raise RuntimeError(f"{binary} did not reach the startup marker")
                start = time.perf_counter_ns()
            out, err = child.communicate(b"\n" if started else None)
            elapsed = (time.perf_counter_ns() - start) / 1e6
        finally:
            if child.poll() is None:
                child.kill()
    if (child.returncode, out, err) != (0, expected, b""):
        raise RuntimeError(f"{binary} {script.name}: status={child.returncode}, stdout={out!r}, stderr={err!r}")
    return elapsed


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("tonari", type=Path)
    cli.add_argument("--bash", type=Path, default=Path("/bin/bash"))
    cli.add_argument("--previous", type=Path, help="include a previous Tonari build")
    cli.add_argument("--spawn-control", type=Path, help="compiled tools/bench_spawn.c for the external case")
    cli.add_argument("--timing", choices=["whole", "started", "both"], default="both")
    cli.add_argument("--utf8-locale", default="en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8",
                     help="LC_ALL for Unicode field-splitting and pathname workloads")
    cli.add_argument("--runs", type=int, default=30)
    cli.add_argument("--case", choices=CASES, action="append")
    cli.add_argument("--state-mib", type=int, default=0,
                     help="fill an unexported shell variable before the ready marker")
    cli.add_argument("--require-target", action="store_true", help="exit nonzero if any Tonari median exceeds two thirds of Bash")
    args = cli.parse_args()
    within_target = True
    if args.state_mib < 0 or args.state_mib and args.spawn_control:
        cli.error("--state-mib requires a nonnegative size and a shell-only comparison")
    size = args.state_mib * 1024 * 1024
    setup = f"printf -v bench_resident '%*s' {size} x || exit 1; " if size else ""
    if args.runs < 1:
        cli.error("runs must be positive")
    shells = [("Tonari", args.tonari.resolve()), ("Bash", args.bash.resolve())]
    if args.previous:
        shells.append(("Previous", args.previous.resolve()))
    with tempfile.TemporaryDirectory() as temp:
        home = Path(temp)
        env = {"HOME": temp, "PATH": "/usr/bin:/bin", "LC_ALL": "C"}
        (home / "rows").write_text("".join(f"record {i}\n" for i in range(2000)))
        if not args.case or any(case in ("glob", "glob-all", "glob-sparse", "glob-multi-star") for case in args.case):
            fixtures = home / "fixtures"
            fixtures.mkdir()
            for i in range(512):
                (fixtures / f"item-{i:03}.txt").touch()
        if not args.case or "glob-subdirs" in args.case:
            mixed = home / "mixed"
            mixed.mkdir()
            for i in range(512):
                (mixed / f"item-{i:03}.txt").touch()
            for i in range(8):
                package = mixed / f"pkg-{i}"
                package.mkdir()
                (package / "lib.rs").touch()
        for directory, count, cases in [("localized", 512, ("glob-utf8-sort", "glob-utf8-once")),
                                        ("large", 4096, ("glob-utf8-large", "glob-utf8-large-repeat",
                                                         "glob-ignore-large", "glob-nocase-large"))]:
            if args.case and not any(case in args.case for case in cases):
                continue
            localized = home / directory
            localized.mkdir()
            prefixes = ["a", "B", "é", "雪", "🌱", "Z", "Å", "c"]
            for i in range(count):
                (localized / f"{prefixes[i % 8]}-{i:03}.txt").touch()
        if not args.case or "glob-recursive" in args.case:
            for i in range(16):
                for j in range(5):
                    leaf = home / "tree" / f"d{i:02}" / f"s{j}"
                    leaf.mkdir(parents=True)
                    (leaf / "lib.rs").touch()
                    for k in range(15):
                        (leaf / f"item-{k:02}.txt").touch()
        for case in args.case or CASES:
            case_env = env
            if case in ("field-split-unicode", "glob-utf8-sort", "glob-utf8-once", "glob-utf8-large", "glob-utf8-large-repeat",
                        "glob-ignore-large", "glob-nocase-large"):
                case_env = dict(env, LC_ALL=args.utf8_locale)
            source, expected = CASES[case]
            script = home / f"{case}.sh"
            script.write_text(source)
            participants = shells
            if case == "external" and args.spawn_control:
                participants = [*shells, ("Direct C", args.spawn_control.resolve())]
            modes = (False, True) if args.timing == "both" else (args.timing == "started",)
            for started in modes:
                samples = {name: [] for name, _ in participants}
                for i in range(5 + args.runs):
                    offset = i % len(participants)
                    for name, binary in participants[offset:] + participants[:offset]:
                        elapsed = sample(binary, script, (expected + "\n").encode(), case_env, started, setup)
                        if i >= 5:
                            samples[name].append(elapsed)
                print(f"{case} ({'already started' if started else 'whole script'})", flush=True)
                within_target = report(samples) and within_target

    if args.require_target and not within_target:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
