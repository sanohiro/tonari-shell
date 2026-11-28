"""Behavioral witnesses for the enumerated GNU Bash POSIX-mode rules."""
import re
import tempfile
from pathlib import Path
from terminal import Terminal

# The stable keys follow the 5.3 Texinfo source, including its @ignore slot 9.
# GNU 5.2 uses a different order and has one distinct function-name rule.
ORDER_52 = (1,21,22,23,24,3,4,5,29,2,13,30,10,11,77,12,19,32,18,6,8,7,31,35,36,37,38,39,33,34,40,14,15,42,43,49,58,59,53,68,69,70,47,44,45,48,64,65,50,51,62,54,55,71,28,52,72,75,63,61,76)
SCRIPTS = {
    1: 'test -n "$POSIXLY_CORRECT"; echo $?',
    3: "alias demo='echo yes'\ndemo",
    4: "alias if='echo fail'\nif :; then echo yes; fi",
    5: 'alias p=\'echo first\'\nf(){ echo "$(p)"; }\nalias p=\'echo second\'\nf',
    6: 'TIMEFORMAT=measured; { time; } 2>elapsed; cat elapsed',
    7: "mkdir bin; printf '#!/bin/sh\\nprintf external' >bin/time; chmod +x bin/time; PATH=$PWD/bin:$PATH; time -p",
    8: 'unset v; printf \'%s\\n\' "${v-\'a}b\'}" ',
    10: 'touch aa; printf yes >a?; cat "a?"; test ! -s aa',
    11: 'v="two words"; printf yes >$v; cat "two words"',
    12: "eval 'export(){ echo wrong; }'; echo after",
    13: 'f(){ printf "<%s>" "$1"; }; x=~ f x=~; echo; export x=~; echo "$x"',
    14: 'set -- a b; v=\'#\'; printf \'<%s>\' "${!v}"; echo; echo "${!#}"',
    15: 'IFS=:; set -- a b; case "a b" in "$*") echo space;; *) echo other;; esac',
    16: 'cat <<END\n`printf \'%s\' \\"ok\\"`\nEND',
    17: 'false; echo "$(true)/$?"; x=$(exit 7); echo "$?"',
    18: "mkdir bin; printf '#!/bin/sh\\necho tilde\\n' >bin/hello; chmod +x bin/hello; PATH='~/bin'; command -v hello; echo $?",
    19: "set +o posix; eval 'export(){ echo function; }'; set -o posix; command -V export; type export",
    20: 'set +o posix; function ./hello { echo called; }; set -o posix; ./hello 2>/dev/null; echo $?',
    21: "mkdir a b; printf '#!/bin/sh\\necho one\\n' >a/tool; printf '#!/bin/sh\\necho two\\n' >b/tool; chmod +x a/tool b/tool; PATH=$PWD/a:$PWD/b:/bin; tool; rm a/tool; tool",
    22: 'touch probe; PATH=$PWD; probe 2>/dev/null; echo "$?"; hash -t probe 2>/dev/null; echo "$?"',
    23: '(exit 17) & p=$!; sleep .06; jobs | awk \'{print $2}\'; wait "$p"; echo $?',
    24: 'set -m; sleep 4 & p=$!; kill -s STOP "$p"; sleep .05; jobs -s | awk \'{print $2}\'; kill -s CONT "$p"; kill "$p"; wait "$p" 2>/dev/null; :',
    27: '(exit 7) & p=$!; wait "$p"; echo "$?"; jobs; wait "$p" 2>/dev/null; echo "$?"',
    31: 'set -H -o history\nhistory -s "echo prior"\nprintf "%s" "!!"',
    32: 'f(){ :; }; type f',
    33: 'echo "$((2+))"\necho after',
    34: 'unset x; : "${x?missing}"\necho after',
    35: 'readonly -Z x\necho after',
    36: 'readonly x=old\nx=new\necho after',
    37: 'readonly x=old\nx=new export y=1\necho after',
    38: 'readonly x=old\nfor x in new; do echo body; done\necho after',
    39: '. ./missing\necho after',
    40: 'eval "if then"\necho after',
    41: "export 'bad!'\necho after",
    42: 'x=before; f(){ x=inner :; }; x=temp f; declare -p x',
    43: 'v="two words"; command export test_value=$v; echo "$test_value"',
    44: 'set -e; x=$(false; echo after); echo "$x"',
    45: 'set -- only; shift 2; echo "$?:$#"',
    46: 'set +o posix; shopt -u interactive_comments; set -o posix; shopt -q interactive_comments; echo $?',
    47: "echo 'echo sourced' >setup; PATH=/bin; . setup\necho after",
    48: "alias demo='echo hi'; alias demo; alias -p demo",
    49: 'set -m; sleep 4 & p=$!; kill -s STOP "$p"; sleep .05; bg %+; kill "$p"; wait "$p" 2>/dev/null; :',
    50: 'mkdir -p top/leaf; cd top; mv ../top ../moved; cd leaf 2>/dev/null; echo $?; /bin/pwd',
    51: 'n=0; component=abcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcdeabcde; while test $n -lt 18; do mkdir "$component" && cd "$component" || exit 2; n=$((n+1)); done; test -d .; echo $n',
    52: "shopt -s xpg_echo; echo -n 'a\\tb'",
    53: "export audit='one two'; readonly fixed=abc; export -p | grep ' audit='; readonly -p | grep ' fixed='",
    54: "set -o history\nhistory -s 'echo first'\nfc -ln -1",
    55: 'unset FCEDIT EDITOR; printf \'#!/bin/sh\\necho "echo changed" >"$1"\\n\' >ed; chmod +x ed; PATH=$PWD:$PATH; set -o history\nhistory -s \'echo before\'\nfc -1',
    56: "set -o history\nhistory -s 'echo first'\nfc -ln 1 1 1; echo status:$?",
    57: "set -o history\nhistory -s 'echo first'\nfc -s 1 1 1; echo status:$?",
    58: "kill -l | awk '{n+=NF; if($0 ~ /SIG|[0-9]\\)/) other=1} END {print NR,(n>10),other+0}'",
    59: 'kill -s SIGINT 99999999 2>error; echo $?; grep -Eq "invalid signal|invalid signal specification" error; echo $?',
    60: 'kill -s 0 "$$" 99999999 2>/dev/null; echo $?',
    61: "printf '%.19g\\n' 1.1000000000000000888",
    62: 'mkdir d; cd d; mv ../d ../moved; pwd; /bin/pwd',
    63: 'mkfifo fifo; exec 8<>fifo; p=$$; seen=no; trap \'seen=yes\' USR1; (sleep .08; kill -s USR1 "$p") & read -t .8 -u 8 x; n=$?; test "$n" -gt 128 && test "$seen" = yes && printf interrupted; wait',
    64: "f(){ echo yes; }; set | grep '^f ()'; echo $?",
    65: "a=one; b='two words'; c='a#b'; set | grep -E '^[abc]='",
    66: "test a '<' b; echo $?; test z '>' a; echo $?",
    67: 'test -t; echo $?; test -t 1; echo $?',
    68: "trap 'echo one' USR1; trap | grep USR1",
    69: "trap 'echo old' USR1; trap USR1; echo $?; trap -p USR1",
    70: "trap 'echo old' USR1; trap - USR1; trap -p | grep -E '(^| )USR[12]$' | sort",
    71: 'PATH=$PWD; echo : >tool; type tool 2>/dev/null; echo $?; command -v tool; echo $?; tool 2>/dev/null; echo $?',
    72: 'set +o posix; ulimit -S -f 32; set -o posix; ulimit -S -f',
    73: 'readonly var=val; unset -v var\necho after',
    74: 'x=old; x=temp unset x; echo "${x-gone}"',
    75: 'n=0; trap \'n=$((n+1))\' CHLD; (exit 3) & p=$!; wait "$p"; echo "$?:$n"',
    76: '(exit 9) & p=$!; wait "$p"; echo "$?"; wait "$p" 2>/dev/null; echo "$?"',
    77: "eval 'function bad-name { echo wrong; }'; echo continued",
}



def terminal_rule(shell, root, key):
    if key == 28:
        from source_review import external_editor_pty
        external_editor_pty(shell, root, "posix-vi-editor")
        return True
    env = dict(POSIXLY_CORRECT="y", ENV=str(root / "env"))
    (root / "env").write_text("audit_startup=env\n")
    (root / ".bashrc").write_text("audit_startup=bashrc\n")
    t = Terminal(shell, root, editing=False, env=env)
    try:
        if b"READY>" not in t.read(b"READY>"):
            return False
        if key in (2, 30):
            src = b'printf "\\036%s\\037" "$audit_startup"\n' if key == 2 else b'printf "\\036%s\\037" "${HISTFILE##*/}"\n'
            t.send(src)
            out = t.read(b"\x1f")
            return (b"\x1eenv\x1f" if key == 2 else b"\x1e.sh_history\x1f") in out
        if key == 29:
            t.send(b"shopt -u promptvars; value=expanded; PS1='<!/!!/$value>'; PS2='<+!/!!/$value>'\n")
            first = t.read(b"/!/expanded>")
            t.send(b"if true; then\n")
            second = t.read(b"/!/expanded>")
            return bool(re.search(rb"<\d+/!/expanded>", first) and re.search(rb"<\+\d+/!/expanded>", second))
        command = b"(exit 17) & p=$!; sleep .06" if key == 25 else b"/bin/sh -c 'kill -TERM $$'"
        t.send(b"set +b; " + command + b"; printf '\\036'; :\n")
        data = t.read(b"READY>")
        before, marker, after = data.partition(b"\x1e")
        report = b"Done(17)" if key == 25 else b"Terminated"
        if not marker or report in before or report not in after:
            return False
        if key == 25:
            t.send(b'jobs; wait "$p"; printf "\\036%s\\037" "$?"\n')
            data = t.read(b"\x1f")
            return b"Done(17)" not in data and b"\x1e17\x1f" in data
        for settings in (b"set -b", b"set +o posix; set +b"):
            t.send(settings + b"; " + command + b"; printf '\\036'; :\n")
            before, marker, after = t.read(b"READY>").partition(b"\x1e")
            if not marker or report not in before or report in after:
                return False
        for listing in (b"jobs", b"jobs -n", b"jobs -p", b"jobs -r"):
            t.send(b"set -o posix; set +b; " + command + b"; printf '\\036'; " +
                   listing + b"; printf '\\037'; :\n")
            data = t.read(b"READY>").partition(b"\x1e")[2]
            listed, marker, after = data.partition(b"\x1f")
            if not marker:
                return False
            if listing == b"jobs -r":
                if report in listed or after.count(report) != 1:
                    return False
            elif listed.count(report) != 1 or report in after:
                return False
            elif listing == b"jobs -p":
                if not re.match(rb"\d+\r?\n", listed):
                    return False
            elif not re.match(rb"\[\d+\]\s+Terminated", listed):
                return False
        t.send(command + b"; printf '\\036'; wait %1; printf '%s\\037' \"$?\"; :\n")
        waited, marker, after = t.read(b"READY>").partition(b"\x1e")[2].partition(b"\x1f")
        if not marker or waited != b"143" or report in after:
            return False
        t.send(command + b"; " + command + b"; printf '\\036'; jobs -l; printf '\\037'; :\n")
        listed, marker, after = t.read(b"READY>").partition(b"\x1e")[2].partition(b"\x1f")
        if not marker or len(re.findall(rb"\[\d+\]\s+\d+\s+Terminated", listed)) != 2 or report in after:
            return False
        t.send(b"hook(){ unset PROMPT_COMMAND; printf '\\036'; " + command +
               b"; printf '\\037'; }; PROMPT_COMMAND=hook\n")
        during, marker, after = t.read(b"READY>").partition(b"\x1e")[2].partition(b"\x1f")
        return marker and during.count(report) == 1 and report not in after
    finally:
        t.close()


def audit_rules(a, section):
    listing = section.partition("@enumerate")[2].partition("@end enumerate")[0]
    listing = re.sub(r"(?ms)^@ignore\b.*?^@end ignore[^\n]*", "", listing)
    items = re.split(r"(?m)^@item\s*$", listing)[1:]
    keys = ORDER_52 if len(items) == len(ORDER_52) else tuple(n for n in range(1, 77) if n != 9)
    if len(keys) != len(items):
        raise ValueError(f"GNU Bash lists {len(items)} POSIX mode rules; assign each new rule a witness")
    for index, (key, item) in enumerate(zip(keys, items), 1):
        group, name = "POSIX mode item", f"{index:02d}"
        rule = " ".join(item.split())
        if key in (2, 25, 26, 28, 29, 30):
            results = []
            for sh in a.shells:
                with tempfile.TemporaryDirectory(dir=a.root) as tmp:
                    try:
                        results.append(terminal_rule(sh, Path(tmp), key))
                    except (OSError, AssertionError) as exc:
                        results.append(str(exc))
            status = "match" if results == [True, True] else "different" if results[1] is not True else "control inconclusive"
            a.add(group, name, status, "tests/compat/posix_rules.py terminal_rule " + str(key))
            a.rows[group][name]["terminal_result"] = results
        else:
            script = SCRIPTS[key]
            # Fatal errors in Bash -c and errors in an actual POSIX script can have
            # different status numbers. Test the documented script invocation.
            args = ["--noprofile", "--norc", "--posix"]
            a.compare(group, name, args=args, stdin=script + "\n")
            row = a.rows[group][name]
            if key in (19, 48, 51) and len(items) == len(ORDER_52) and "observed" in row:
                ref, candidate = row["observed"]
                known = {
                    19: ("export is a function", "export is a special shell builtin\nexport is a special shell builtin\n"),
                    48: ("demo='echo hi'\nalias demo='echo hi'\nalias demo='echo hi'\n", "demo='echo hi'\nalias demo='echo hi'\n"),
                    51: ("", "18\n"),
                }
                older, current = known[key]
                if (ref["out"].startswith(older) and (key != 51 or ref["code"] == 2 and "cd:" in ref["err"])
                        and candidate["code"] == 0 and candidate["out"] == current and not candidate["err"]):
                    row["status"] = "reference version difference"
                    row["decision"] = "matches the GNU 5.3 rule and control"
            if key in (56, 75) and "observed" in row:
                ref, candidate = row["observed"]
                required = "status:1\n" if key == 56 else "3:1\n"
                if candidate["out"] == required and candidate["code"] == 0 and not candidate["timeout"]:
                    if key == 56 and "status:0\n" in ref["out"] or key == 75 and re.fullmatch(r"1\d\d:1\n", ref["out"]):
                        row["status"] = "intentional difference"
                        row["decision"] = "enforce the documented excess-operand error" if key == 56 else "return the child's exit status and dispatch SIGCHLD once"
        a.rows[group][name]["manual_rule"] = rule
