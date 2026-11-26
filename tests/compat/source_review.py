"""Behavior and warm, interleaved release timings for the GNU Bash source review."""
import argparse
import json
import os
from pathlib import Path
import random
import socket
import threading
import statistics
import subprocess
import tempfile
import time
from terminal import Terminal

# Compare observable output and status. Error messages may identify the shell.
COMMON = {
    "wordbreak-variable": """printf '<%s>' "$COMP_WORDBREAKS"; COMP_WORDBREAKS=' :'; printf '<%s>' "$COMP_WORDBREAKS"; unset COMP_WORDBREAKS; printf '<%s>' "${COMP_WORDBREAKS-unset}" """,
    "toggle-case": """x='aB cD-eF'; printf '<%s>\\n' "${x~}" "${x~~}" "${x~[cB]}" "${x~~[cB]}"; a=('xyZ' '01' 'a雪B'); printf '<%s>\\n' "${a[@]~~}" """,
    "declared-case": """declare -c v='hELLO woRLD'; echo "$v"; v+=' ABC'; declare -p v; declare -ac arr=('eXAMPLE' '.NEXT'); declare -p arr; declare +c v; v='rAW'; echo "$v"; declare -cl z='mIx'; declare -p z""",
    "completion-sources": "complete -g -s -A signal pick; complete -p pick; compgen -A group $(id -gn) | LC_ALL=C sort -u; compgen -s http | LC_ALL=C sort -u; compgen -A signal SIGTER; compgen -A signal DEBUG",

    "posix-variable": 'f(){ shopt -qo posix; printf "%s/%s " "$?" "${POSIXLY_CORRECT-unset}"; }; f; POSIXLY_CORRECT=1 f; f; set -o posix; f; unset POSIXLY_CORRECT; f; g(){ local POSIXLY_CORRECT=inside; f; }; g; f',
    "physical": 'cd linkdir; pwd; set -P; pwd; cd ..; cd linkdir; echo "$PWD"; set +o physical; cd ..; cd -L linkdir; pwd',
    "execfail": 'shopt -s execfail; exec ./missing; echo "continued:$?"; (exec ./missing; echo wrong); echo "subshell:$?"',
    "shift-verbose": 'set --; shopt -s shift_verbose; shift 2 2>shift.err; test -s shift.err; echo $?; shopt -u shift_verbose; shift 2 2>shift.err; test -s shift.err; echo $?',
    "alias-table": """alias sample='echo one'; echo "${BASH_ALIASES[sample]}"; BASH_ALIASES[sample]='echo two'; alias sample; BASH_ALIASES=([other]=ok); echo "${#BASH_ALIASES[@]}:${BASH_ALIASES[other]}"; unset 'BASH_ALIASES[sample]'; echo "${BASH_ALIASES[sample]}"; unset BASH_ALIASES; alias sample; echo "${BASH_ALIASES[sample]-unset}" """,
    "command-table": 'hash -p /bin/sh sample; echo "${BASH_CMDS[sample]}"; BASH_CMDS[sample]=/bin/echo; hash -t sample; sample hello; unset "BASH_CMDS[sample]"; echo "${BASH_CMDS[sample]}"; hash -r; echo "${#BASH_CMDS[@]}"',
    "execignore": 'PATH=$PWD/a:$PWD/b; EXECIGNORE="$PWD/a/*"; choice; type -P choice; hash -r; set +h; choice; EXECIGNORE="$PWD/*"; choice; echo $?; ./a/choice',
    "groups": 'echo "$GROUPS:${GROUPS[*]}"; GROUPS[0]=999; echo "$GROUPS"; unset "GROUPS[0]"; echo "${GROUPS[0]-unset}/${#GROUPS[@]}"; unset GROUPS; GROUPS=(user); echo "$GROUPS"',
    "history-number": 'echo "$HISTCMD"; set -o history; history -s first; echo "$HISTCMD"; history -s second; echo "$HISTCMD"; set +o history; echo "$HISTCMD"; unset HISTCMD; echo "${HISTCMD-unset}"',
    "nested-source": 'set -- orig; source ./outer arg; echo "$1"; source ./inner; echo "$1"',
    "test-N": 'for f in newer older tied missing link; do [[ -N $f ]]; echo $?; test -N "$f"; echo $?; [ -N "$f" ]; echo $?; done',
    "execution-string": '[[ $BASH_EXECUTION_STRING == *marker* ]]; echo $?; x=$BASH_EXECUTION_STRING; unset BASH_EXECUTION_STRING; echo "${BASH_EXECUTION_STRING-unset}"; BASH_EXECUTION_STRING=value; echo "$BASH_EXECUTION_STRING"',
    "history-extension": """for x in dir/file.txt dir/a.b.c .dot dir.ext/file plain trailing.; do history -c; history -s "echo $x"; history -p '!!:$:r' '!!:$:e'; done""",
    "source-args": 'set -- old; source ./setargs a b; echo "$#:$1"; f(){ source ./setargs nested; echo "$#:$1"; }; f func; source ./shiftargs a b; echo "$#:$1"',
    "funcnest": 'FUNCNEST=2; f(){ echo f; g; echo wrong; }; g(){ h; }; h(){ echo h; }; (f); echo "$?"; unset FUNCNEST; f',
}
NEW = {
    "source-fullpath": 'shopt -s bash_source_fullpath; source ./linked; f; shopt -u bash_source_fullpath; source ./linked; f',
    "source-path": 'PATH=none; shopt -u sourcepath; source -p empty:src mod one; . -psrc mod two; source -p empty mod; echo $?; source -p "" mod; source -p empty ./mod',
    "compgen-array": """a=(old); declare -n ref=a; compgen -V ref -W 'be bee bee' be; printf '<%s>\\n' "${a[@]}"; compgen -V ref -W 'x' z; echo "$?:${#a[@]}"; compgen -V 'bad-name' -W x; echo $?; declare -A map=([key]=ok); compgen -V map -W x; echo "$?:${map[key]}"; readonly kept=old; compgen -V kept -W x; echo "$?:$kept" """,
    "trap-action": """trap 'printf "%s" word' USR1; trap '' USR2; trap -P TERM USR1 USR2; echo $?; x=$(trap -P USR1); echo "$x"; trap -P; echo $?; trap -pP TERM; echo $?""",
    "trap-signal": """BASH_TRAPSIG=outer; trap 'old=$BASH_TRAPSIG; kill -USR2 $$; echo "restored:$BASH_TRAPSIG"' USR1; trap 'echo "nested:$BASH_TRAPSIG"' USR2; kill -USR1 $$; echo "$old:$BASH_TRAPSIG"; trap 'echo "err:$BASH_TRAPSIG"' ERR; trap 'echo "exit:$BASH_TRAPSIG"' EXIT; false""",
    "monotonic": 'a=$BASH_MONOSECONDS; BASH_MONOSECONDS=2; b=$BASH_MONOSECONDS; [[ $a =~ ^[0-9]+$ && $b -ge $a && $b -ne 2 ]]; echo $?; unset BASH_MONOSECONDS; echo "${BASH_MONOSECONDS-unset}"; BASH_MONOSECONDS=manual; echo "$BASH_MONOSECONDS"',
    "glob-keys": 'cd names; for GLOBSORT in name -name + - size -size atime -atime mtime -mtime ctime -ctime blocks -blocks numeric -numeric invalid; do printf "%s:" "$GLOBSORT"; printf "<%s>" *; echo; done; GLOBSORT=nosort; a=(*); printf "%s\\n" "${a[@]}" | sort',
    "current-output": """v=old; x="${ v=new; local scratch=private; printf 'a\\nb\\n\\n'; return 9; echo wrong; }"; echo "$?:$v:${scratch-unset}:$x" """,
    "current-reply": """REPLY=outer; x=${| echo visible; echo "${REPLY-unset}"; REPLY=$'a\\nb\\n'; return 7; }; printf '<%s>:%s:<%s>\\n' "$x" "$?" "$REPLY" """,
    "current-grammar": """x=${ case x in x) { printf '%s' '}'; }; esac; y=${ printf inner; }; cat <<'END'
here } and ; }
END
printf '%s' "$y"; }suffix; echo "$x" """,
    "current-state": 'set -- one; f(){ local v=outer; x=${ local v=inner; printf "%s/%s/%s" "$v" "$FUNCNAME" "$1"; set -- new; }; echo "$v:$x:$1"; }; f abc',
    "current-nesting": 'FUNCNEST=1; f(){ echo yes; }; x=${ f; }; echo "$x"',
    "current-exit": 'x=${ exit 19; }; echo wrong',
    "current-loop": 'for v in 1 2; do x=${ break; }; echo wrong; done; echo done',
    "current-large": 'x=${ cat large; }; echo "${#x}"',
}

HOT = {
    "toggle-case": ('v=${phrase~~}', 2400),
    "declared-case": ('v="$phrase"', 2200),
    "completion-group": ('compgen -g "$group" >/dev/null', 24),
    "completion-service": ('compgen -s http >/dev/null', 40),
    "completion-signal": ('compgen -A signal SIGTER >/dev/null', 650),
    "posix-variable": ('POSIXLY_CORRECT=1; unset POSIXLY_CORRECT', 800),
    "physical": ('pwd >/dev/null', 900),
    "execfail": ('exec ./missing', 550),
    "shift-verbose": ('shift 2', 800),
    "alias-table": ('s=${BASH_ALIASES[sample]}', 2500),
    "command-table": ('s=${BASH_CMDS[sample]}', 2500),
    "execignore": ('type -P choice >/dev/null', 700),
    "groups": ('s="${GROUPS[*]}"', 1800),
    "history-number": ('s=$HISTCMD', 2400),
    "source-fullpath": ('source ./src/noop', 600),
    "test-N": ('[[ -N newer ]]', 2500),
    "execution-string": ('s=$BASH_EXECUTION_STRING', 2500),
    "history-extension": ("history -p '!!:$:e' >/dev/null", 800),
    "source-args": ('source ./setargs a b', 600),
    "funcnest": ('f', 2500),
    "source-path": ('source -p empty:src noop', 600),
    "compgen-array": ('compgen -V arr -W "$words" item', 80),
    "trap-action": ('a=$(trap -P USR1)', 800),
    "trap-signal": ('kill -USR1 $$', 1000),
    "monotonic": ('s=$BASH_MONOSECONDS', 2500),
    "glob-name": ('a=(items/*)', 100),
    "glob-size": ('a=(items/*)', 100),
    "glob-numeric": ('a=(*)', 180),
    "current-output": ('a=${ printf value; }', 700),
    "current-reply": ('a=${| REPLY=value; }', 1800),
}

def fixture(root):
    for name in ("src", "empty", "names", "items", "a", "b"):
        (root / name).mkdir()
    for directory in ("a", "b"):
        path = root / directory / "choice"
        path.write_text("#!/bin/sh\necho " + directory + "\n")
        path.chmod(0o755)
    for name in ("newer", "older", "tied"):
        path = root / name
        path.touch()
        base = 1700000000000000000
        os.utime(path, ns=(base, base + {"newer": 1, "older": -1, "tied": 0}[name]))
    (root / "link").symlink_to("newer")
    (root / "loc").write_text('echo "$BASH_SOURCE"; f(){ echo "$BASH_SOURCE"; }')
    (root / "linked").symlink_to("loc")
    (root / "linkdir").symlink_to("src")
    (root / "outer").write_text("set -- outer; source ./inner arg")
    (root / "inner").write_text("set -- inner")
    (root / "mod").write_text("echo cwd")
    (root / "src/mod").write_text('echo "src:$1"')
    (root / "src/noop").write_text(":")
    (root / "setargs").write_text('set -- changed last')
    (root / "shiftargs").write_text("shift")
    (root / "large").write_text("abcd" * 65536 + "\n")
    for i, name in enumerate(("10", "2", "30", "alpha", "beta", "snow雪")):
        path = root / "names" / name
        path.write_bytes(b"x" * (i % 3))
        os.utime(path, (100 - i, 100 + i))
    (root / "names/link").symlink_to("10")
    (root / "names/broken").symlink_to("missing")
    for i in range(256):
        (root / "items" / str(i * 17)).touch()

def completion_time(shell, root):
    # Start the clock after the prefix is visible. Verify the accepted argument
    # separately so echoed input or a suggestion cannot masquerade as completion.
    t = Terminal(shell, root, editing=True, env={"FIGNORE": ".tmp", "HISTFILE": "/dev/null",
                                               "XDG_STATE_HOME": str(root / "state")})
    try:
        assert b"READY>" in t.read(b"READY>")
        t.send(b'printf "%s" pick.')
        assert b"pick." in t.read(b"pick.")
        start = time.perf_counter_ns()
        t.send(b"\t")
        assert b"valid" in t.read(b"valid")
        elapsed = (time.perf_counter_ns() - start) / 1e6
        t.send(b" >accepted; printf '\\036'\n")
        assert b"\x1e" in t.read(b"\x1e")
        assert (root / "accepted").read_bytes() == b"pick.valid"
        return elapsed
    finally:
        t.close()


def ignored_completion(shell, root, prefix, force):
    t = Terminal(shell, root, editing=True, env={"FIGNORE": ".tmp", "HISTFILE": "/dev/null",
                                               "XDG_STATE_HOME": str(root / "state")})
    try:
        assert b"READY>" in t.read(b"READY>")
        t.send(b"shopt -" + (b"s" if force else b"u") + b" force_fignore\n")
        assert b"READY>" in t.read(b"READY>")
        t.send(b'printf "%s" ' + prefix.encode() + b'\t')
        t.send(b' >accepted; printf "\\036"\n')
        assert b"\x1e" in t.read(b"\x1e")
        return (root / "accepted").read_text()
    finally:
        t.close()


def callback_tab(shell, root, line, setup=""):
    t = Terminal(shell, root, editing=True, env={"HISTFILE": "/dev/null", "XDG_STATE_HOME": str(root / "state")})
    try:
        assert b"READY>" in t.read(b"READY>")
        declaration = r"""demo(){ printf '<%s>' "$@" >accepted; }; cb(){ printf '<%s>' "$2" "$3" "$COMP_CWORD" "${COMP_WORDS[@]}" >callback; COMPREPLY=(CHOSEN); }; complete -F cb demo; """
        t.send((declaration + setup + r"""printf '\036'""" + "\n").encode())
        assert b"\x1e" in t.read(b"\x1e")
        t.send(line.encode())
        assert line.split()[-1].encode() in t.read(line.split()[-1].encode())
        start = time.perf_counter_ns()
        t.send(b"\t")
        assert b"CHOSEN" in t.read(b"CHOSEN")
        ms = (time.perf_counter_ns()-start)/1e6
        t.send(b"; printf '\\036'\n")
        assert b"\x1e" in t.read(b"\x1e")
        return ((root / "callback").read_bytes(), (root / "accepted").read_bytes()), ms
    finally:
        t.close()


def socket_time(shell, run, kind, count):
    # The receiver validates every payload. Exercise a builtin, an external
    # command and a persistent named descriptor with the same loopback fixture.
    with socket.socket(socket.AF_INET, kind) as listener:
        listener.bind(("127.0.0.1", 0))
        stream = kind == socket.SOCK_STREAM
        if stream:
            listener.listen()
        listener.settimeout(3)
        received = []
        errors = []
        def receive():
            try:
                for _ in range(count):
                    if stream:
                        peer, _ = listener.accept()
                        with peer:
                            data = b""
                            while part := peer.recv(1024):
                                data += part
                    else:
                        data = listener.recv(1024)
                    received.append(data)
            except OSError as exc:
                errors.append(str(exc))
        reader = threading.Thread(target=receive)
        reader.start()
        path = f"/dev/{'tcp' if stream else 'udp'}/127.0.0.1/{listener.getsockname()[1]}"
        first = path.replace("127.0.0.1", "localhost") if stream else path
        code = (f'p={path}; /bin/echo external >"{first}"; exec {{sock}}<>"$p"; '
                'printf named >&"$sock"; exec {sock}>&-; '
                f'for ((i=2;i<{count};i++)); do printf payload >"$p" || exit; done')
        try:
            outcome, ms = run(shell, code)
        finally:
            reader.join()
        if errors or outcome != (0, b"", False) or received != [b"external\n", b"named"] + [b"payload"] * (count-2):
            raise ValueError((path, shell, outcome, errors, received[:4], len(received)))
        return ms


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bash", type=Path)
    parser.add_argument("tonari", type=Path)
    parser.add_argument("--first", type=Path)
    parser.add_argument("--runs", type=int, default=9)
    parser.add_argument("--case", action="append", choices=["programmable-boundaries", "filename-completion", "tcp-redirection", "udp-redirection", *HOT])
    args = parser.parse_args()
    selected = set(args.case or ())
    def requested(name):
        return not selected or name in selected
    shells = [p.resolve() for p in (args.bash, args.tonari, args.first) if p]
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        fixture(root)
        env = dict(PATH="/usr/bin:/bin", HOME=directory, LC_ALL="C", TMPDIR=directory,
                   INPUTRC="/dev/null", TERM="dumb")
        def run(shell, code):
            start = time.perf_counter_ns()
            p = subprocess.run([shell, "--noprofile", "--norc", "-c", code],
                               cwd=root, env=env, capture_output=True, timeout=20)
            return (p.returncode, p.stdout, bool(p.stderr)), (time.perf_counter_ns()-start)/1e6
        version = run(shells[0], 'printf "%s" "$BASH_VERSION"')[0][1].decode()
        modern = tuple(map(int, version.split(".")[:2])) >= (5, 3)
        cases = COMMON | (NEW if modern else {})
        failed = []
        for name, code in cases.items():
            expected = run(shells[0], code)[0]
            actual = run(shells[1], code)[0]
            if expected != actual:
                failed.append((name, repr(expected), repr(actual)))
        print(json.dumps(dict(reference=version, witnesses=len(cases), differences=failed)), flush=True)
        if failed:
            raise SystemExit(1)
        termroot = root / "completion"
        termroot.mkdir()
        (termroot / "pick.valid").touch()
        for i in range(1024):
            (termroot / f"pick.{i}.tmp").touch()
        (termroot / "unique.tmp").touch()
        (termroot / "shelldir").mkdir()
        (termroot / "shelldir/unique.tmp").touch()
        for shell in shells[:2]:
            for prefix, force, expected in [("./unique.", True, "./unique."),
                                            ("./unique.", False, "./unique.tmp"),
                                            ("she/unique.", True, "she/unique.")]:
                assert ignored_completion(shell, termroot, prefix, force) == expected
        rng = random.Random(1729)
        if requested("programmable-boundaries"):
            for line, setup in [("demo --key=va", ""), ("demo --key=", ""), ("demo x::y==v", ""),
                                ("demo first:val", "COMP_WORDBREAKS=' '; "), ("demo @user", ""),
                                ("demo --key=va", "unset COMP_WORDBREAKS; COMP_WORDBREAKS=' '; "),
                                ("demo 'a:b' tail", ""), (r"demo a\:b", "")]:
                results = [callback_tab(shell, termroot, line, setup)[0] for shell in shells[:2]]
                assert results[0] == results[1], (line, results)
            line = "demo " + "x:one=v " * 32 + "--key=fragTAIL"
            samples = [[] for _ in shells]
            for r in range(args.runs + 1):
                indices = list(range(len(shells)))
                rng.shuffle(indices)
                expected = None
                for i in indices:
                    outcome, ms = callback_tab(shells[i], termroot, line)
                    if expected is None: expected = outcome
                    assert expected == outcome, (shells[i], outcome, expected)
                    if r: samples[i].append(ms)
            if args.runs:
                print(json.dumps(dict(case="programmable-boundaries", repeats=1, median_ms=[round(statistics.median(g),3) for g in samples], samples_ms=samples)), flush=True)
        if requested("filename-completion"):
            samples = [[] for _ in shells]
            for r in range(args.runs + 1):
                indices = list(range(len(shells)))
                rng.shuffle(indices)
                for i in indices:
                    elapsed = completion_time(shells[i], termroot)
                    if r: samples[i].append(elapsed)
            if args.runs:
                print(json.dumps(dict(case="filename-completion", repeats=1,
                                      median_ms=[round(statistics.median(g), 3) for g in samples],
                                      samples_ms=samples)), flush=True)
        for label, kind in (("tcp-redirection", socket.SOCK_STREAM), ("udp-redirection", socket.SOCK_DGRAM)):
            if not requested(label): continue
            samples = [[] for _ in shells]
            count = 80 if args.runs else 3
            for r in range(args.runs + 1):
                indices = list(range(len(shells)))
                rng.shuffle(indices)
                for i in indices:
                    ms = socket_time(shells[i], run, kind, count)
                    if r: samples[i].append(ms)
            if args.runs:
                print(json.dumps(dict(case=label, repeats=count, median_ms=[round(statistics.median(g),3) for g in samples],
                                      samples_ms=samples)), flush=True)
        for name, (body, count) in HOT.items():
            if not requested(name) or not args.runs or (not modern and name not in COMMON):
                continue
            setup = {
                "toggle-case": "phrase='sOME mixed-CASE-123 example value for conversion'",
                "declared-case": "declare -c v; phrase='aBCDEf GHIJKl mixed case words'",
                "completion-group": "group=$(id -gn)",
                "physical": "set -P", "execfail": "shopt -s execfail; exec 2>/dev/null", "history-number": "set -o history; history -s seed", "shift-verbose": "shopt -s shift_verbose; exec 2>/dev/null",
                "alias-table": "alias sample=value", "command-table": "hash -p /bin/sh sample",
                "execignore": 'PATH=$PWD/a:$PWD/b; EXECIGNORE="$PWD/a/*"',
                "source-fullpath": "shopt -s bash_source_fullpath",
                "history-extension": "history -s 'echo dir/file.txt'",
                "funcnest": 'FUNCNEST=8; f(){ :; }',
                "compgen-array": 'words="' + " ".join(f"item{i}" for i in range(512)) + '"',
                "trap-action": "trap ':' USR1", "trap-signal": "trap 's=$BASH_TRAPSIG' USR1",
                "glob-size": "GLOBSORT=size", "glob-numeric": "cd items; GLOBSORT=numeric",
            }.get(name, ":")
            code = f"{setup}; for ((i=0;i<{count};i++)); do {body}; done; echo done"
            samples = [[] for _ in shells]
            for r in range(args.runs + 1):
                indices = list(range(len(shells)))
                rng.shuffle(indices)
                for i in indices:
                    outcome, ms = run(shells[i], code)
                    if outcome != (0, b"done\n", False):
                        raise ValueError((name, shells[i], outcome))
                    if r:
                        samples[i].append(ms)
            print(json.dumps(dict(case=name, repeats=count, median_ms=[round(statistics.median(g), 3) for g in samples],
                                  samples_ms=[[round(v, 3) for v in g] for g in samples])), flush=True)

if __name__ == "__main__":
    main()
