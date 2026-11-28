"""Exercise shell startup and builtin contracts through the public executable."""
import argparse
import os
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("tonari", type=Path)
    parser.add_argument("bash", type=Path)
    parser.add_argument("--previous", type=Path)
    parser.add_argument("--runs", type=int, default=0)
    args = parser.parse_args()
    candidate, reference = args.tonari.resolve(), args.bash.resolve()
    with tempfile.TemporaryDirectory() as work:
        root = Path(work)
        env = dict(HOME=work, PATH="/usr/bin:/bin", LC_ALL="C", HISTFILE="/dev/null")

        def run(shell, *argv, extra=None, stdin=None):
            return subprocess.run([shell, *argv], cwd=root, env=env | (extra or {}),
                                  capture_output=True, input=stdin, timeout=25)

        def checked(shell, argv, expected, extra=None):
            result = run(shell, *argv, extra=extra)
            assert (result.returncode, result.stdout) == expected, (shell, argv, result)
            return result

        where = root / "scripts"
        where.mkdir()
        sample = where / "input.sh"
        sample.write_text('printf "%s\\n%s\\n" "$0" "${BASH_SOURCE[0]}"\n')
        setup = root / "startup"
        setup.write_text("loaded=yes\nreturn 7\n")
        cases = [
            (["-c", 'printf "%s/%s" "$?" "$loaded"'], (0, b"0/yes"), {"BASH_ENV": str(setup)}),
            (["-p", "-c", 'printf "%s" "${loaded-no}"; [[ $- == *p* ]]'], (0, b"no"), {"BASH_ENV": str(setup)}),
            (["input.sh"], (0, f"input.sh\n{sample}\n".encode()), {"PATH": str(where)}),
            (["--norc", "-i", "-o", "vi", "-o", "emacs", "-c", 'shopt -qo emacs && ! shopt -qo vi'], (0, b""), {}),
            (["-o", "history", "-c", 'shopt -qo history'], (0, b""), {}),
            (["-c", 'declare -n ref=value; export ref=ok; printenv value; export -n ref; printenv value'], (1, b"ok\n"), {}),
            (["-c", 'unset -v 1x; echo "$?"; test " 12 " -eq 12 && ! test ! ! -z ""'], (1, b"1\n"), {}),
            (["-c", r"""printf 'xx%n%s' n one n two; echo " $n" """], (0, b"xxonexxtwo 2\n"), {}),
            (["-c", r"""v='PATH=a:~'; printf -v q %q "$v"; eval "w=$q"; [[ $w == "$v" ]]"""], (0, b""), {}),
            (["-c", 'cd /; cd "$HOME"; pushd -; pwd'], (0, b"/\n/ ~\n/\n"), {}),
            (["-c", 'shopt -p restricted_shell'], (1, b"shopt -u restricted_shell\n"), {}),
        ]
        cases.extend([
            (["-c", "shopt -s mailwarn noexpand_translation; shopt -q mailwarn && shopt -q noexpand_translation"], (0, b""), {}),
            (["-c", "dirs wayward"], (2, b""), {}),
            (["-c", "v=x; IFS=: read -r a b <<< 'one:two'; printf '%s/%s' \"$a\" \"$b\""], (0, b"one/two"), {}),
            (["-c", "printf '%s' \"$(printf -v local_v %d 42)\"; printf '%s' \"${local_v-none}\""], (0, b"none"), {}),
            (["-c", "printf '%(%Y-%m-%d)T' 0"], (0, b"1970-01-01"), {"TZ": "UTC0"}),
        ])
        for argv, expected, extra in cases:
            for shell in (reference, candidate):
                checked(shell, argv, expected, extra)

        # Bash 5.3 reports a saturated conversion as an error. Its predecessor
        # returns success for the same bytes. Check the reference version's contract.
        version = run(reference, "-c", 'echo "${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}"')
        modern = tuple(map(int, version.stdout.strip().split(b"."))) >= (5, 3)
        for shell, status in ((candidate, 1), (reference, int(modern))):
            out = checked(shell, ["-c", r"printf '%d\n' 99999999999999999999999999999"],
                          (status, b"9223372036854775807\n"))
            assert out.stderr
        # Shell strings are UTF-8. A delimiter consumes the whole scalar and
        # leaves the shared descriptor positioned at the following character.
        data = "one雪two雪three".encode()
        (root / "words").write_bytes(data)
        for cmd in [
            'exec 3<words; IFS= read -ru3 -d雪 x; IFS= read -ru3 -d雪 y; printf "%s/%s" "$x" "$y"',
            'mapfile -t -d雪 a <words; printf "%s/%s" "${a[0]}" "${a[1]}"',
            'mapfile -n2 -t -d雪 a <words; printf "%s/%s" "${a[0]}" "${a[1]}"',
        ]:
            checked(candidate, ["-c", cmd], (0, b"one/two"))
        for argv in (["--noprofile", "--norc", "-ic", "printf ok"], ["--rcfile", str(setup), "-ic", 'printf %s "$loaded"']):
            checked(candidate, argv, (0, b"yes" if "--rcfile" in argv else b"ok"), {"HOME": ""})

        # A callback may read the same open file description. The following
        # mapfile record has to start after the line consumed by that callback.
        (root / "callback-lines").write_text("one\ntwo\nthree\n")
        callback = "cb() { local x; if [[ $1 == 0 ]]; then IFS= read -r -u3 x; fi; }; exec 3<callback-lines; mapfile -t -u3 -C cb -c1 a; printf '<%s>' \"${a[@]}\""
        for shell in (reference, candidate):
            checked(shell, ["-c", callback], (0, b"<one><three>"))
        for cmd, operand in [("read -u 99 x", b"99"), ("read -n -1", b"-1"),
                             ("read -t abc", b"abc"), ("mapfile -O bad", b"bad"),
                             ("printf '%y'", b"y"), ("umask 8", b"8"), ("dirs nope", b"nope")]:
            for shell in (reference, candidate):
                result = run(shell, "-c", cmd)
                assert result.returncode and operand in result.stderr, (cmd, result)
        checked(candidate, ["-c", "printf '%(%8s)T' 42"], (0, b"      42"), {"TZ": "UTC0"})
        checked(candidate, ["-c", "printf '%(%Es|%Os)T' 42"], (0, b"42|42"), {"TZ": "JST-9"})
        print(f"{len(cases) + 9} Bash comparisons and 7 Tonari contracts passed", flush=True)
        if not args.runs:
            return
        (root / "lines").write_text(("abcdefgh " * 100 + "\n") * 1200)
        (root / "child").mkdir()
        (root / "many.sh").write_text("batch() {\n" + "  x=hello; [ -n \"$x\" ];\n" * 1500 + "}\nbatch\n")
        benchmarks = {
            "physical cd between directories": 'for ((i=0;i<400;i++)); do cd -P child && cd -P .. || exit 1; done',
            "parse and run function from stdin": None,
            "growing shell strings": 'v=; for ((i=0;i<2000;i++)); do v+=abcdefghijklmnop; done; [[ ${#v} == 32000 ]]',
            "read 1 MiB lines": 'while IFS= read x; do :; done < lines',
            "mapfile 1200 lines with count": 'mapfile -t -n1200 a <lines; echo "${#a[@]}"',
            "format repeated integers": "for ((i=0;i<5000;i++)); do printf '%08d' 123; done >/dev/null",
            "read here inputs": "for ((i=0;i<1500;i++)); do IFS=: read -r a b <<< one:two; done",
            "mapfile callbacks": "cb() { :; }; mapfile -t -C cb -c1 a <lines",
            "integer comparisons": 'for ((i=0;i<5000;i++)); do test "$i" -ge 0; done',
            "startup via BASH_ENV": ":",
        }
        shells = ([args.previous.resolve()] if args.previous else []) + [candidate, reference]
        for label, script in benchmarks.items():
            samples = [[] for _ in shells]
            for trial in range(args.runs + 1):
                for i in range(len(shells)):
                    idx = (trial+i) % len(shells)
                    start = time.perf_counter_ns()
                    if script is None:
                        with (root / "many.sh").open("rb") as source:
                            out = subprocess.run([shells[idx], "--noprofile", "--norc"], cwd=root, env=env,
                                                 stdin=source, capture_output=True, timeout=15)
                    else:
                        out = run(shells[idx], "--noprofile", "--norc", "-c", script,
                                  extra={"BASH_ENV": str(setup)} if label.startswith("startup") else None)
                    elapsed = (time.perf_counter_ns() - start) / 1e6
                    assert out.returncode == 0 and not out.stderr, out
                    if label == "mapfile 1200 lines with count":
                        assert out.stdout == b"1200\n", out
                    if trial:
                        samples[idx].append(elapsed)
            print(label, *(f"{statistics.median(s):.3f}" for s in samples), sep="\t", flush=True)


if __name__ == "__main__":
    main()
