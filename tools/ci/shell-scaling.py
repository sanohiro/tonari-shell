"""Compare median times at two sizes for the shell-scaling regressions."""

import os
import signal
from pathlib import Path
from statistics import median
import sys
import tempfile

from bench_scripts import sample


def workloads(n):
    yield "array literal", "a=(\n" + "entry\n" * n + ')\necho ${#a[@]}', str(n)
    yield "subshell", "(\n" + ":\n" * n + "echo ok\n)", "ok"
    yield "substitution", "x=$(\n" + ":\n" * n + 'echo ok\n); echo "$x"', "ok"
    yield "continued word", "x=" + "a\\\n" * n + 'b; echo ${#x}', str(n + 1)
    yield "double quote", 'x="' + "a\n" * n + '"; echo ${#x}', str(2 * n)
    yield "arithmetic", "((x=\n" + "1+\n" * n + "0)); echo $x", str(n)
    yield "braces", "x=" + "{a," * n + "; echo ${#x}", str(3 * n)
    yield "braces with a close", "x=" + "{a" * n + "}; echo ${#x}", str(2 * n + 1)
    for label, expr, seed, expected, setup in [
        ("repeated extglob miss", "${s//+(a)z/X}", "a" * n, n, ""),
        ("extglob alternatives hit", "${s//+(a|b)z/X}", "ab" * (n//2) + "z", 1, ""),
        ("extglob suffix", "${s/%+(a)b/X}", "a" * n, n, ""),
        ("extglob trim", "${s%%+(a)b}", "a" * n, n, ""),
        ("star replace", "${s//*z/X}", "a" * n, n, ""),
        ("folded star", "${s//*z/X}", "a" * n, n, "shopt -s nocasematch"),
        ("class star", "${s//[a-z]*z/X}", "a" * n, n, ""),
        ("class prefix trim", "${s##*[0-9]}", "a" * n, n, ""),
        ("class suffix trim", "${s%%*[0-9]}", "a" * n, n, ""),
        ("class suffix replace", "${s/%*[0-9]/X}", "a" * n, n, ""),
    ]:
        yield label, f'shopt -s extglob\n{setup}\ns={seed}\nfor i in 1 2 3; do x={expr}; done; echo ${{#x}}', str(expected)
    yield "negated extglob", "s=" + "a"*n + '\n[[ $s == !(z)*!(z)*y ]]; echo $?', "1"
    yield "negation with match", "s=" + "a"*n + 'y\n[[ $s == !(z)*!(z)*y ]]; echo $?', "0"
    yield "indexed declaration", f'a=({{1..{n}}}); for ((i=0;i<200;i++)); do declare a[0]=x; done; echo "${{a[0]}} ${{#a[@]}}"', f"x {n}"
    yield "assoc declaration", f'declare -A m; for ((i=0;i<{n};i++)); do m[k$i]=$i; done; for ((i=0;i<200;i++)); do declare -g m[k0]=x; done; echo "${{m[k0]}} ${{#m[@]}}"', f"x {n}"
    yield "mapfile origin", f'exec 3<records; for ((i=0;i<{n};i++)); do mapfile -u3 -O "$i" -n1 -t a; done; echo "${{#a[@]}} ${{a[i-1]}}"', f"{n} record"
    yield "history erasedups", f'HISTCONTROL=erasedups; HISTSIZE=20000; for ((i=0;i<{n};i++)); do history -s "command $i"; done; history 1 | wc -l | tr -d " "', "1"
    for label, init, key in (("indexed self append", "a=(x)", "a[0]"), ("assoc self append", "declare -A a=([k]=x)", "a[k]")):
        yield label, f'{init}; for ((i=0;i<{n*4};i++)); do {key}="${{{key}}}x"; done; echo ${{#{key}}}', str(n*4+1)


def main():
    def deadline(*_):
        raise TimeoutError

    signal.signal(signal.SIGALRM, deadline)
    shells = [Path(p).resolve() for p in sys.argv[1:4]]
    runs = int(os.environ.get("BENCH_RUNS", "7"))
    sizes = [int(s) for s in os.environ.get("SCALING_SIZES", "1000 2000").split()]
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "records").write_text("record\n" * max(sizes))
        env = dict(HOME=tmp, LC_ALL="C", PATH="/usr/bin:/bin")
        print("case\tsize\tprevious_ms\ttonari_ms\tbash_ms", flush=True)
        for n in sizes:
            for label, source, expected in workloads(n):
                script = root / "source.sh"
                script.write_text(source + "\n")
                values = [[] for _ in shells]
                slow = [False] * len(shells)
                for trial in range(runs + 2):
                    for j in range(len(shells)):
                        k = (trial+j) % len(shells)
                        if slow[k]:
                            continue
                        answer = "0" if label == "negated extglob" and k != 1 else expected
                        signal.alarm(12)
                        try:
                            ms = sample(shells[k], script, (answer + "\n").encode(), env, True)
                        except TimeoutError:
                            slow[k] = True
                            continue
                        finally:
                            signal.alarm(0)
                        if trial >= 2 or ms >= 1000:
                            values[k].append(ms)
                        if ms >= 1000:
                            slow[k] = True
                print(label, n, *[f"{median(v):.3f}" if v else ">12000" for v in values], sep="\t", flush=True)



if __name__ == "__main__":
    main()

