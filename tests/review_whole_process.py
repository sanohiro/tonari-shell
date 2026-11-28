"""Public shell checks for process ownership, descriptor restoration and I/O."""

from pathlib import Path
import re
import subprocess
import sys
import tempfile


def cases():
    yield "integer assignment isolation", """
declare -i n=41; side=0; n=side++ | :; echo "$side,$n,${PIPESTATUS[*]}"
n=side++ & p=$!; wait "$p"; echo "$side,$n,$?"
shopt -s lastpipe; : | n=side++; echo "$side,$n,${PIPESTATUS[*]}"
""", "0,41,0 0\n0,41,0\n1,0,0 0"
    yield "nameref integer assignment", """
declare -i n=8; declare -n ref=n; i=0; ref=i++ | :; echo "$i:$n"
set -o posix; ref=i++ : | :; echo "$i:$n"
""", "0:8\n0:8"
    yield "asynchronous argument and redirect levels", """
f(){ echo "$1:$BASH_SUBSHELL"; }; f "$BASH_SUBSHELL" & wait
echo "$BASH_SUBSHELL:$((0))" | cat & wait
: | echo "$BASH_SUBSHELL" & wait
echo "$BASH_SUBSHELL" | cat
echo ok >"depth$BASH_SUBSHELL" & wait; cat depth1
""", "1:1\n1:0\n1\n0\nok"
    yield "background expansion failure remains waitable", """
exec 2>errors
readonly v=keep; v=changed & p=$!
[[ $p =~ ^[0-9]+$ ]]; echo "pid=$?"; wait "$p"; echo "$?:$v"
shopt -s failglob; : ./missing-file-*.none & p=$!
[[ $p =~ ^[0-9]+$ ]]; echo "pid=$?"; wait "$p"; echo "$?"
grep -q 'readonly variable' errors && grep -q 'no match' errors && echo reported
""", "pid=0\n1:keep\npid=0\n1\nreported"
    yield "asynchronous external environment", """
echo "$SHLVL"; /usr/bin/printenv SHLVL & wait
SHLVL=9 /usr/bin/printenv SHLVL & wait
/usr/bin/printenv SHLVL </dev/null & wait
/usr/bin/printenv SHLVL | cat & wait
: | /usr/bin/printenv SHLVL & wait
set -m; /usr/bin/printenv SHLVL & wait; echo "$SHLVL"
""", "1\n0\n8\n0\n1\n1\n0\n1"
    yield "direct subshell redirection and list controls", """
( /usr/bin/printenv SHLVL </dev/null )
( SHLVL=8 /usr/bin/printenv SHLVL </dev/null )
( :; /usr/bin/printenv SHLVL </dev/null )
( /usr/bin/printenv SHLVL </dev/null ) | cat
TIMEFORMAT=; time ( /usr/bin/printenv SHLVL </dev/null )
f(){ /usr/bin/printenv SHLVL </dev/null; }; ( f )
""", "0\n7\n1\n0\n1\n1"
    yield "background function on final command string line", (
        'f(){ /usr/bin/printenv SHLVL; }; f & wait'), "0"
    yield "background function with more command string input", (
        'f(){ /usr/bin/printenv SHLVL; }; f &\nwait'), "1"
    yield "nested and parenthesized function contexts", """
g(){ /usr/bin/printenv SHLVL; }; f(){ g; }
( g ); ( echo "$(g)" ); f & wait
""", "1\n0\n1"
    yield "temporary redirects and shared reader", """
exec 7<input; exec 8<&7; { IFS= read -ru8 a; : <&9; } 2>errors
printf '%s/%s\n' "$?" "$a"; cat <&7; echo 'outside'
""", "1/first\nsecond\noutside"
    yield "duplicate output, failed writes and parent stdout", """
exec 7>out 8<input
echo one >&7; printf '%s\\n' two >&7; echo parent
{ echo denied >&8; } 2>err; echo "read-only:$?"; grep -q 'echo: write error: Bad file descriptor' err && echo reported
{ printf '' >&9; } 2>err; echo "missing:$?"; grep -q '9: Bad file descriptor' err && echo reported
printf '' >&8; echo "empty:$?"; cat out
trap '' PIPE; exec 7> >(true); wait "$!"
{ echo closed >&7; } 2>err; echo "pipe:$?"; grep -q 'echo: write error: Broken pipe' err && echo reported
""", "parent\nread-only:1\nreported\nmissing:1\nreported\nempty:0\none\ntwo\npipe:1\nreported"
    yield "self duplicate and move", """
exec 7>out; exec 7>&7-; echo kept >&7; cat out
exec 7>&-; : 7>&7; echo "$?"; /usr/bin/true 7>&7-; echo "$?"
exec 8>&1 1>&-; printf '' >&1; r=$?; echo "empty:$r" >&8
{ echo denied >&1; } 2>err; r=$?; echo "closed:$r" >&8
grep -q 'echo: write error: Bad file descriptor' err && echo reported >&8
""", "kept\n0\n0\nempty:0\nclosed:1\nreported"
    yield "duplicate output at the descriptor limit", """
( exec 7>out 3>/dev/null; ulimit -n 4; echo dropped >&7; echo "full:$?" ) 2>err
grep -Eq '(Too many open files|No file descriptors available)( [(]os error 24[)])?$' err && echo reported
[ ! -s out ] && echo empty
( exec 7>out 3>/dev/null 0<&-; ulimit -n 4; echo kept >&7; echo "spare:$?" ); cat out
( exec 7>out 3>/dev/null 1>&-; ulimit -n 4; echo kept >&7 ); echo "closed:$?"; cat out
""", "full:1\nreported\nempty\nspare:0\nkept\nclosed:0\nkept"
    yield "printf escape diagnostic follows completed lines", r"""
{ printf 'first\n\x' >&2; } 2>err
sed 's/^.*printf: /printf: /' err; echo
{ printf 'second\n%b' 'arg\n\u' >&2; } 2>err
sed 's/^.*printf: /printf: /' err; echo
""", "first\nprintf: missing hex digit for \\x\n\\x\nsecond\nprintf: missing unicode digit for \\u\narg\n\\u"
    yield "pipeline early reader", """
set -o pipefail; a=$(<large); printf '%s' "$a" | "$1" -c 'import os; print(os.read(0, 1).decode())'
echo "${PIPESTATUS[*]}"
""", "x\n141 0"
    yield "pipe shared with an external reader", """
{ printf 'aa\nbb\n'; } | { IFS= read -r a; cat; echo "stored:$a"; }
""", "bb\nstored:aa"
    yield "disowned coprocess stays usable", """
coproc HELPER { read -r x; echo "ack:$x"; }; r=${HELPER[0]}; w=${HELPER[1]}; p=$HELPER_PID
disown "$p"; echo "${HELPER[0]+set}:${HELPER_PID+set}"
echo hi >&"$w"; IFS= read -ru "$r" line; echo "$?:$line"
""", "set:set\n0:ack:hi"
    yield "direct coprocess redirection", """
mkfifo gate
coproc /usr/bin/printenv SHLVL <gate
exec 8<&"${COPROC[0]}"; p=$COPROC_PID; : >gate
IFS= read -ru8 level; wait "$p"; echo "$?:$level"
TIMEFORMAT=; time coproc /usr/bin/printenv SHLVL <gate
exec 8<&"${COPROC[0]}"; p=$COPROC_PID; : >gate
IFS= read -ru8 level; wait "$p"; echo "$?:$level"
""", "0:0\n0:1"
    yield "signal while waiting", """
trap 'echo handled' USR1
( sleep .05; kill -USR1 "$$"; sleep .05; exit 7 ) & p=$!
wait "$p"; rc=$?; (( rc > 128 )); echo "interrupted=$?"
wait "$p"; echo "final=$?"
""", "handled\ninterrupted=0\nfinal=7"


def file_cases():
    yield "script-file function eligibility", """
f(){ /usr/bin/printenv SHLVL; }; g(){ :; /usr/bin/printenv SHLVL; }
f & wait; g & wait
""", "1\n0"


def main():
    for arg in sys.argv[1:]:
        shell = Path(arg).resolve()
        bash52 = subprocess.check_output([shell, "--version"]).startswith(b"GNU bash, version 5.2.")
        inputs = [(case, False) for case in cases()] + [(case, True) for case in file_cases()]
        for (label, script, expected), file_input in inputs:
            # GNU 5.3 added async function-tail optimization. Older Bash
            # keeps the intermediate shell for both forms.
            if bash52 and label == "background function on final command string line":
                expected = "1"
            if bash52 and label == "script-file function eligibility":
                expected = "1\n1"
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                (root / "input").write_bytes(b"first\nsecond\n")
                (root / "large").write_bytes(b"x" * 1024 * 1024)
                command = ["-c", script, "check"]
                if file_input:
                    (root / "script").write_text(script)
                    command = ["script"]
                p = subprocess.run([shell, "--noprofile", "--norc", *command, sys.executable], cwd=root,
                                   env=dict(PATH="/usr/bin:/bin", LC_ALL="C", HOME=tmp),
                                   capture_output=True, timeout=8)
                want = (0, (expected + "\n").encode(), b"")
                err = p.stderr
                if label == "asynchronous external environment":
                    # GNU can announce a completed monitored job before wait.
                    err = re.sub(rb"(?m)^\[\d+\]\+ +Done +/usr/bin/printenv SHLVL\n", b"", err)
                if (p.returncode, p.stdout, err) != want:
                    raise AssertionError(f"{shell} {label}: {p.returncode, p.stdout, p.stderr}; expected {want}")
                print(f"ok {shell.name} {label}")


if __name__ == "__main__":
    main()
