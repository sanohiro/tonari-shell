"""Exercise shell startup and builtin contracts through the public executable."""
import argparse
import os
import pwd
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
            merged = {k: v for k, v in (env | (extra or {})).items() if v is not None}
            return subprocess.run([shell, *argv], cwd=root, env=merged,
                                  capture_output=True, input=stdin, timeout=25)

        def checked(shell, argv, expected, extra=None):
            result = run(shell, *argv, extra=extra)
            assert (result.returncode, result.stdout) == expected, (shell, argv, result)
            return result

        (root / "tonari-review-completion").mkdir()
        for prefix in ("~/", "~//"):
            for shell in (reference, candidate):
                checked(shell, ["-c", "compgen -d \"$1\"", "sh", prefix + "tonari-review-comp"],
                        (0, (prefix + "tonari-review-completion\n").encode()))
        for shell in (reference, candidate):
            checked(shell, ["-c", "compgen -d '~/us'"], (0, b"~/usr\n"), {"HOME": ""})

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

        # A bulk declaration observes dynamic variables. Looking one up by name
        # evaluates it. Compare subsequent state in the same shell process.
        listings = r'''RANDOM=23; first=$RANDOM; second=$RANDOM
RANDOM=23
alias witness_alias=true
readonly marker=ok
declare -px > exports && grep -qx 'declare -x OLDPWD' exports || exit 9
declare -pr > readonlys && grep -qx 'declare -r marker="ok"' readonlys || exit 1
declare -pi > integers && declare -p > all || exit 2
grep -qx 'declare -i RANDOM="23"' all || exit 3
grep -qx 'declare -- SECONDS' all || exit 7
: "$SECONDS"; declare -pi > integers && grep -q '^declare -i SECONDS=' integers || exit 8
[[ $RANDOM == "$first" ]] || exit 4
RANDOM=23
declare -p RANDOM > named && [[ $RANDOM == "$second" ]] || exit 5
grep -qx "declare -i RANDOM=\"$first\"" named || exit 6
unset RANDOM; RANDOM=literal
declare -p > all && grep -qx 'declare -- RANDOM="literal"' all'''
        options = r'''export SHELLOPTS BASHOPTS
for setting in on off; do
  if [[ $setting == on ]]; then set -o pipefail; shopt -s nullglob
  else set +o pipefail; shopt -u nullglob; fi
  set > values
  grep -Fqx "SHELLOPTS=$SHELLOPTS" values && grep -Fqx "BASHOPTS=$BASHOPTS" values || exit 1
  declare -p SHELLOPTS BASHOPTS > named || exit 2
  grep -Fq "SHELLOPTS=\"$SHELLOPTS\"" named && grep -Fq "BASHOPTS=\"$BASHOPTS\"" named || exit 3
done'''
        scope = r'''x=outer
f() {
  local x=inner local_only=v
  declare -gp x local_only || return
  declare -gp > all && grep -qx 'declare -- x="inner"' all || return 1
  grep -qx 'declare -- local_only="v"' all || return 1
  declare -g x=updated
}
f && echo "$x"'''
        names = r'''declare zzA; declare -a zzB; declare -A zzC; readonly zzD; export zzE
zzF=""; zzG=(); declare -A zzH=(); declare -n zzI=zzA
printf '<%s>' "${!zz@}"; echo; compgen -v zz
declare -p zzA; zzA=now; compgen -v zzA'''
        frames = """shopt -s extdebug
outer() { inner x 'y z'; }
inner() { declare -p BASH_ARGV BASH_ARGC; }
outer a b"""
        for shell in (reference, candidate):
            checked(shell, ["-c", frames], (0, b'declare -a BASH_ARGV=([0]="y z" [1]="x" [2]="b" [3]="a")\n'
                                               b'declare -a BASH_ARGC=([0]="2" [1]="2" [2]="0")\n'))
            checked(shell, ["-c", scope], (0, b'declare -- x="inner"\ndeclare -- local_only="v"\nupdated\n'))
            checked(shell, ["-c", names], (0, b'<zzF><zzG><zzH><zzI>\nzzF\nzzG\nzzH\nzzI\ndeclare -- zzA\nzzA\n'))
            checked(shell, ["-c", listings], (0, b""))
            checked(shell, ["-c", options], (0, b""))

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
        # Exercise delimiter candidates with matching last bytes on either
        # side of both the scalar reader's and bulk reader's I/O boundaries.
        sep = "雪".encode()
        collision = "ª".encode()  # Same final byte as 雪.
        for size in (4094, 4095, 65534, 65535):
            first = b"x" * (size - len(collision)) + collision
            (root / "words").write_bytes(first + sep + b"two" + sep + b"tail")
            for flags, end in (("-t", b""), ("", sep)):
                cmd = f'exec 3<words; mapfile {flags} -u3 -d雪 a; printf "%s|" "${{a[@]}}"; cat <&3'
                checked(candidate, ["-c", cmd], (0, first + end + b"|two" + end + b"|tail|"))
            for source in (["words", "<(cat words)"] if size == 4095 else ["words"]):
                cmd = f'exec 3< {source}; IFS= read -ru3 -d雪 a; s=$?; mapfile -n1 -tu3 -d雪 b; printf "%s/%s|%s|%s|" "$s" "$?" "$a" "$b"; cat <&3'
                checked(candidate, ["-c", cmd], (0, b"0/0|" + first + b"|two|tail"))

        (root / "words").write_bytes(b"one\\two\\three")
        for shell in (candidate, reference):
            for raw, expected in [("", b"1:onetwothree/"), ("-r", b"0:one/two\\three")]:
                cmd = f"""exec 3<words; IFS= read {raw} -u3 -d '\\' value; printf '%s:%s/' "$?" "$value"; cat <&3"""
                checked(shell, ["-c", cmd], (0, expected))

        # Invalid UTF-8 fails one record while preserving completed array
        # entries and the following shared regular-file position.
        for delim in (b"\n", sep):
            (root / "words").write_bytes(delim.join((b"good", b"\xff", b"next")) + delim)
            sep_arg = "-d雪" if delim == sep else ""
            checked(candidate, ["-c", f'exec 3<words; mapfile -tu3 {sep_arg} a; printf "%s/%s/" "$?" "$a"; cat <&3'],
                    (0, b"1/good/next" + delim))

        # Isolate startup from the user's rc files and journal. HOME unset
        # uses the actual account directory for the default path; HOME="" has
        # no personal configuration path. Explicit BASH_ENV/--rcfile still work.
        account = Path(pwd.getpwuid(os.getuid()).pw_dir)
        for supplied, expected in [(None, os.fsencode(account / ".bash_history")), ("", b"unset")]:
            checked(candidate, ["--noprofile", "--norc", "-ic", 'printf %s "${HISTFILE-unset}"'],
                    (0, expected), {"HOME": supplied, "HISTFILE": None, "XDG_STATE_HOME": work})
        (root / "+0").mkdir()
        for shell in (candidate, reference):
            checked(shell, ["-c", 'HOME=; OLDPWD=sentinel; cd && [[ $OLDPWD == "$PWD" ]]'], (0, b""))
            checked(shell, ["-c", 'unset HOME; cd; printf %s "$?"'], (0,b"1"))
            checked(shell, ["-c", 'OLDPWD=; cd -; printf %s "$?"'], (0,b"\n0"))
            checked(shell, ["-c", 'cd scripts; OLDPWD=..; cd -; printf %s "$?"'], (0,b"..\n0"))
            checked(shell, ["-c", 'pushd -n scripts >/dev/null; dirs -c extra; s=$?; printf "%s/%s" "$s" "${#DIRSTACK[@]}"'], (0,b"2/2"))
            checked(shell, ["-c", 'pushd -- +0 >/dev/null && [[ $PWD == */+0 ]]'], (0, b""))
            checked(shell, ["-c", 'pushd / scripts'], (1, b""))
            checked(shell, ["-c", 'pushd +not-a-number'], (2, b""))
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
        for command in ("test '(' '(' ')'", "test = -o -1 -o -z", "test z -a -t -o x"):
            checked(candidate, ["-c", command], (0, b""))
        for command, word in [("test invalid -eq 1", b"invalid"), ("test 1 -eq invalid", b"invalid"),
                              ("test -t badfd", b"badfd"), ("[[ -t badfd ]]", b"badfd")]:
            failure = checked(candidate, ["-c", command], (2, b""))
            assert word in failure.stderr, failure
        # GNU Bash 5.3 tests argv[1] even inside a later one-argument group.
        # An empty string has false truth value wherever the group appears.
        checked(candidate, ["-c", "test '(' '' ')' -a '(' x ')'"], (1, b""))

        checked(candidate, ["-c", "printf '%(%8s)T' 42"], (0, b"      42"), {"TZ": "UTC0"})
        checked(candidate, ["-c", "printf '%(%Es|%Os)T' 42"], (0, b"42|42"), {"TZ": "JST-9"})
        print("Startup, directory, shared-descriptor and builtin contracts passed", flush=True)
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
