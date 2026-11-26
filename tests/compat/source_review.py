"""Behavior and warm, interleaved release timings for the GNU Bash source review."""
import argparse
import json
import os
from pathlib import Path
import random
import socket
import shlex
import sys
import threading
import statistics
import subprocess
import tempfile
import time
from terminal import Terminal

# Compare observable output and status. Error messages may identify the shell.
COMMON = {
    "history-physical-lines": "set -o history -H\necho first\nif :; then\necho !absent\necho done\nfi\ncat <<'END'\n!!\nEND\necho 'line\n!!\nend'\necho \"$(echo '!')\"",
    "ignoreeof-variable": "IGNOREEOF=4; shopt -qo ignoreeof; echo $?; f(){ local IGNOREEOF=2; unset IGNOREEOF; shopt -qo ignoreeof; echo $?; }; f; echo \"$IGNOREEOF\"; shopt -qo ignoreeof; echo $?; set +o ignoreeof; echo \"${IGNOREEOF-unset}\"; ignoreeof=2; shopt -qo ignoreeof; echo $?; unset ignoreeof; shopt -qo ignoreeof; echo $?",
    "nameref-type-change": "declare -i x=1; y=2; declare -n x=y; declare -p x; echo \"$x\"; declare +n x; declare -p x; readonly ro=y; declare -n ro; echo $?; declare -p ro\ndeclare -n r\nr='invalid name'; echo skipped\necho \"$?\"; declare -p r; read r <<< /; echo \"$?\"; printf -v r /; echo \"$?\"; declare -p r",
    "typed-export": "a=(old); readonly 'a=(3)'; f(){ export -a 'r=(4 5)'; export -A 'm=([one]=1)'; }; f; declare -p a r m; export -an 'r=(7)'; declare -p r; export -a empty; declare -p empty",
    "globstar-overlap": "mkdir -p tree/a/a/a tree/a/b; cd tree; shopt -s globstar; printf '<%s>' **/a/**; echo; printf '<%s>' **/**/a/**",
    "local-set-options": "set +f +u; inner(){ local -; set +f; set -u; }; f(){ local -; set -f; local -; inner; case $- in *f*) echo glob;; esac; set -o ignoreeof; echo \"$IGNOREEOF\"; local -p; }; f; case $- in *f*|*u*) echo leaked;; esac; echo \"${IGNOREEOF-unset}\"",
    "getopts-readonly-end": "readonly v; OPTIND=1; getopts a v -a; echo \"$?:$OPTIND\"; getopts a v -a; echo \"$?:$OPTIND\"",
    "history-event-words": "history -s 'cmd a.old a.html a.h  x|y 2>/dev/null'; history -p '!?a.h?:%' '!cmd-2' '!cmd*' '!cmd$' 'echo [!A-Z] ${!name}'",
    "keyword-arguments": r"""inspect(){ printf '<%s>' "$#" "$X" "$Y" "$@"; echo; }; run(){ inspect a X=one 'Q'=b Y="$X/t" E\=c; }; run; set -k; run; inspect a X[2]=ignored b; set -- keep A=value; echo "$#:$1"; x=$(inspect aa X=tt); echo "$x"; inspect aa X=pipe | cat; set +o keyword; run; echo "${X-unset}" """,
    "array-recovery": "a=(orig)\na=(bad > untouched)\necho \"$?:${a[*]}\"\ntest ! -e untouched",
    "compat-random": r"""for level in 3.1 44 5.0 51 5.2; do BASH_COMPAT=$level; RANDOM=42; echo "$RANDOM/$RANDOM/$RANDOM"; hash -r; hash -l; done; shopt -s compat43; echo "$BASH_COMPAT"; shopt -q compat42; echo $?; f(){ local BASH_COMPAT=3.1; shopt -q compat31; echo $?; }; f; shopt -q compat43; echo $?; unset BASH_COMPAT; shopt -q compat43; echo $?""",
    "compat-regex": r"""for level in 31 32; do BASH_COMPAT=$level; re='^a.*z$'; [[ abcz =~ "$re" ]]; echo $?; [[ abcz =~ $re ]]; echo $?; done""",
    "compat-loop": r"""f(){ break; echo function; }; for n in 43 44 52; do BASH_COMPAT=$n; for i in 1; do f; echo caller; done; if (( n <= 44 )); then for i in 1; do (break; echo child); echo "end:$?"; done; fi; done""",
    "compat-array": r"""for n in 51 52; do BASH_COMPAT=$n; a=(one two); declare -A m=([k]=v); test -v 'a[@]'; echo $?; [[ -v 'm[*]' ]]; echo $?; unset 'a[@]' 'm[*]'; declare -p a m; unset a m; done""",
    "assignment-tilde": "base=/bin\np=$base:~/bin:~/lib\necho \"$p\"\nexport p=$base:~/export\necho \"$p\"\nset -o posix\necho plain=~:~\ndeclare p=~:~\necho \"$p\"",
    "assignment-attributes": r"""declare -i n; declare -u u; declare -ia ix; declare -Au names; printf '<%s>' "${n:=4+3}" "${u:=foo}" "${ix[2]:=3+6}" "${names[id]:=foo}"; declare -p n u ix names; declare -n ref=u; u=; echo "${ref:=snow}" "$u" """,
    "coprocess-moved-ends": r"""coproc P { IFS= read -r v; echo "$v"; IFS= read -r v; }; pid=$P_PID; exec 4<&${P[0]}-; exec 5>&${P[1]}-; printf '<%s>' "${P[@]}"; echo hello >&5; IFS= read -r text <&4; echo "$text"; exec 5>&-; wait "$pid"; echo $?; exec 4<&-""",
    "nameref-loop": r"""readonly a=one b=two; xs=(zero first); declare -n inner=a outer=inner; for outer in a b 'xs[1]'; do printf '<%s/%s>' "${!inner}" "$outer"; done; declare -p outer inner; for outer in b 'bad-name' a; do echo "$outer"; done; echo "$?"; declare -nr locked=a; for locked in b; do echo wrong; done; echo "$?"; declare -n fresh; for fresh in a b; do echo "$fresh"; done; declare -p fresh""",
    "nested-operand-quotes": r"""v=set; printf '<%s>' "${v:+"a\p\'雪"}" "${v:+a\p\'雪}" ${v:+"a\p"} "${missing:-"one\q two"}" "${made:="new\q value"}" "$made"; set -o posix; printf '<%s>' "${v:+"a\p"}" "${v:+a\p}" """,
    "nameref-clear": """v=before; declare -n ref=v; declare +n ref=after; declare -p ref v; declare -n ref=v; declare +n ref+=plus; declare -p ref v; readonly lock=x; declare -n ref=lock; declare +n ref=other; echo "$?"; declare -p ref lock""",
    "scalar-array-slice": """v=abcd; declare -n ref=v; printf '<%s>\\n' "${v[@]:0:2}" "${v[*]:1:2}" "${ref[@]:1:2}"; a=(abcd def); printf '<%s>\\n' "${a[@]:0:1}" """,
    "indirect-positionals": """set --; printf '<%s>' A "${!@}" B "${!*:-missing}" "${!9-default}"; set -- who; who=value; echo "${!@}" """,
    "multiline-history": "set -o history\nfor v in a b\ndo\n  : item\ndone\nshopt -s lithist\nf() {\n  : 'two\\nlines'\n}\nshopt -u cmdhist\nfor v in a\ndo\n  :\ndone\nhistory",
    "read-one-field": r"""read value <<< '  x  y  '; printf '<%s>' "$value"; IFS=$'\t' read value <<< $'\t a \t'; printf '<%s>' "$value"; read value <<< ' x\ '; printf '<%s>' "$value" """,
    "command-missing-hook": """command_not_found_handle(){ printf '<%s>' "$@" "$BASH_SUBSHELL" "${tag-unset}"; touched=yes; return 23; }; tag=local no_such_tnr 'two words' '' >hooked; echo "$?:${touched-none}"; cat hooked; echo; command no_such_tnr | cat; echo "${PIPESTATUS[*]}"; v=$(no_such_tnr x); echo "$?:$v"; ./no_such_tnr 2>/dev/null; echo $?; no_such_tnr bg >hooked & wait $!; echo $?; cat hooked""",
    "binary-source": "source /bin/sh 2>/dev/null; echo $?; source / 2>/dev/null; echo $?",
    "history-file-enable": "printf 'stored command\\n' > imported-history\nHISTFILE=imported-history\nset -o history\nhistory -p '!!' ",
    "function-stack": """FUNCNAME=ignored; echo "<$FUNCNAME>"; f(){ FUNCNAME=no; echo "<$FUNCNAME>"; }; f; unset FUNCNAME; FUNCNAME=ordinary; echo "$FUNCNAME" """,
    "nolog": """set -o nolog; shopt -qo nolog; echo $?; set +o nolog; shopt -qo nolog; echo $?""",
    "debug-arguments": r"""set -- a b; echo "${BASH_ARGC[*]}|${BASH_ARGV[*]}"; set -- changed; f(){ echo "${BASH_ARGC[*]}|${BASH_ARGV[*]}"; }; f x y; shopt -s extdebug; f x y; echo "${BASH_ARGC[*]}|${BASH_ARGV[*]}" """,
    "positional-assignment-error": 'echo ${6=x}; echo wrong\necho "next:$?"\nset -e\necho ${@:=x}; echo wrong\necho wrong',
    "quoted-star-order": 'set -- a "b c" 雪; a=("$@"); unset IFS; printf "<%s>" "$*${IFS:=-}$*" ; IFS=; printf "<%s>" "${a[*]}${IFS:=:}${a[*]}"; set -u --; printf "<%s>" "$*"',
    "ansi-braced-byte": r"""printf '<%s>' $'ab\x{41}cd' $'\x{0000000000000000000041}' $'\x{c3}\x{a9}' $'x\x{}ignored' $'\x{4}x' $'\x{41X' """,
    "invalid-length": "echo ${#/}; echo wrong\necho ${#%}; echo wrong\necho ${#=}; echo wrong\necho ${#+}; echo wrong\nset -- one two\nprintf '<%s>' \"${#}\" \"${##}\" \"${#=x}\" \"${#+x}\"",
    "bad-expansion": "echo first\necho ${#:}; echo wrong\necho \"next:$?\"",
    "stack-tilde": 'pushd src >/dev/null; printf "<%s>\\n" ~0 ~1 ~+1 ~-0 ~-1 ~99; popd >/dev/null',
    "quick-history": "HISTIGNORE='history*:set -H'; set -o history\nhistory -s 'echo line 2 for history'\nset -H\n^2^8\necho after",
    "history-search-reuse": """history -s 'echo abc def'; history -p '!??' '!?ef?' '!!:%' '!!:s//X/' '!??'""",
    "history-repeat": r"""history -s 'echo foo foo bar'; history -p '!!:s/foo/new/' '!!:&' '!!:g&' '!!:s//\&/' '!?ar?:%' '!!:%' '^foo^X^' '!!:&'; history -p '!absent' '!!'; echo $? """,
    "custom-history-event": """history -s 'echo first last'; histchars='^:#'; history -p '^^'; histchars='$:#'; history -p '$$'; histchars='@:#'; history -p '@@'""",
    "alias-delimiters": "shopt -s expand_aliases\nalias head='echo $(echo value'\nhead) after\nalias calc='echo $(( 4+3 )'\ncalc)\nset -o posix\nalias nest='('\necho $(nest echo in ) )",
    "backquote-escape": "p='\\\\/some\\\\/path'\nprintf '<%s>' ${p//\"`printf '%s' \\\\`\"/}",
    "alias-comment": "shopt -s expand_aliases\nalias quiet='# note'\nquiet echo unexpected; echo wrong\necho done\nalias out='echo value > '\nalias destination='out-file ; echo stored'\nout destination\ncat out-file",
    "open-quote-alias": "shopt -s expand_aliases\nalias message=\"printf '<%s>' 'Error:\"\nmessage name'\necho done",
    "posix-case-alias": "shopt -s expand_aliases\nset -o posix\nalias switch=case\nx=$(switch x in x) echo pass;; esac)\necho \"$x\"",
    "child-trap-wait": """n=0; trap '((++n))' CHLD; /bin/sleep .24 & /bin/sleep .14 & /bin/sleep .04 & wait; echo "$?:$n"; trap - CHLD""",
    "signal-table": r"""kill -l >kill-list; trap -l >trap-list; cmp -s kill-list trap-list; echo $?; awk 'NR==1 {print $1,$2,$3,$4}' kill-list; kill -l 1 129; kill -l HUP""",
    "legacy-zone": r"""export TZ=EST5EDT; printf '%(%F %T %z %Z)T\n' 1275250155 1262304000; export TZ=EST5EDT,M3.2.0/2,M11.1.0/2; printf '%(%F %T %z %Z)T\n' 1275250155""",
    "bind-names": r"""BASH_COMPAT=5.2; bind -p forward-word >bindings 2>/dev/null; echo $?; grep -q backward-word bindings; echo $?""",
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
    "empty-async-negation": "!& echo $?; wait; !; echo $?",
    "partial-assoc-init": "declare -A a=([one]=1 bad [two]=2); echo skipped\necho \"$?:${a[one]}:${a[two]}\"; a=(old surplus)\na=([hi]=OK ['']=no [missing]=no)\necho \"$?:${a[hi]}:${a[old]}:${a[missing]}\"; a=(good yes '' ignored last ok); echo \"$?:${a[last]}\"",
    "terminal-test-operand": "for fd in bad '' ' +3 ' -1 4294967296 9999999999999999999999; do test -t \"$fd\"; echo $?; [[ -t \"$fd\" ]]; echo $?; done",
    "trap-return-context": """invoke(){ kill -USR1 $$; return 99; }; helper(){ false; return; }; trap 'helper; s=$?; false; return --' USR1; invoke; echo "$?:$s"; trap 'false; exit' USR1; kill -USR1 $$; echo wrong""",
    "compat-test": """for n in 52 53; do BASH_COMPAT=$n; test '(' '!' = '!' ')' -a yes; echo $?; test '(' '-n' ')' -a yes; echo $?; done""",
    "bind-filter": r"""bind -p forward-word beginning-of-line unknown-name 2>/dev/null | awk -F': ' '!seen[$NF]++ {print $NF}' """,
    "source-fullpath": 'shopt -s bash_source_fullpath; source ./linked; f; shopt -u bash_source_fullpath; source ./linked; f',
    "source-path": 'PATH=none; shopt -u sourcepath; source -p empty:src mod one; . -psrc mod two; source -p empty mod; echo $?; source -p "" mod; source -p empty ./mod',
    "compgen-array": """a=(old); declare -n ref=a; compgen -V ref -W 'be bee bee' be; printf '<%s>\\n' "${a[@]}"; compgen -V ref -W 'x' z; echo "$?:${#a[@]}"; compgen -V 'bad-name' -W x; echo $?; declare -A map=([key]=ok); compgen -V map -W x; echo "$?:${map[key]}"; readonly kept=old; compgen -V kept -W x; echo "$?:$kept" """,
    "read-fields": r"""IFS=$'\t\r\f\v'; read a b <<< $'  line\tb \t\r\f\v'; printf '<%s><%s>\n' "$a" "$b"; readonly locked=x; read first locked last <<< $'one\ttwo\tthree'; echo "$?:$first:$locked:$last" """,
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
    "empty-async-negation": ('!& wait "$!"', 80),
    "history-physical-lines": (":", 300),
    "ignoreeof-variable": ("IGNOREEOF=2; unset IGNOREEOF", 900),
    "nameref-type-change": ("declare -i x=2; declare -n x=y; declare +n x", 700),
    "partial-assoc-init": ("a=([one]=1 [two]=2 [three]=3)", 700),
    "typed-export": ("export -a 'r=(ab cd)'", 650),
    "globstar-overlap": (": tree/**/a/**", 300),
    "local-set-options": ("f", 800),
    "terminal-test-operand": ("test -t 99999; [[ -t 99999 ]]", 1800),
    "getopts-readonly-end": ("OPTIND=1; getopts a v -a; getopts a v -a", 800),
    "history-event-words": ("history -p '!?a.h?:%' '!cmd-2' '!cmd*' >/dev/null", 600),
    "keyword-arguments": ('inspect keep X=one Y=$X', 900),
    "compat-random": ('v=$RANDOM', 2200),
    "compat-regex": ('[[ abcz =~ "$re" ]]', 1100),
    "compat-loop": ('for x in 1; do f; done', 800),
    "compat-array": ("[[ -v 'm[@]' ]]", 2200),
    "compat-test": ("test '(' '!' = '!' ')' -a yes", 900),
    "assignment-tilde": ('p=$base:~/bin:~/lib', 1400),
    "debug-arguments": ('v=${BASH_ARGV[1]}', 1500),
    "multiline-history": (":", 300),
    "scalar-array-slice": ('v=${text[@]:1:4}', 1600),
    "indirect-positionals": ('v=${!@:-missing}', 1600),
    "ansi-braced-byte": (r"""v=$'prefix-\x{41}-\x{c3}\x{a9}-end'""", 1200),
    "invalid-length": ("echo ${#/}", 300),
    "positional-assignment-error": ("echo ${6=x}", 300),
    "quoted-star-order": ('IFS=; v="$*${IFS:=-}$*"', 1200),
    "function-stack": ('f', 1200),
    "read-one-field": ("read value <fields", 750),
    "read-fields": ("read a b <fields", 750),
    "function-origin": ('f(){ :; }; declare -F f >/dev/null', 700),
    "nolog": ('set -o nolog; set +o nolog', 800),
    "signal-lookup": ('kill -l TERM >/dev/null', 800),
    "numeric-locale": ("""printf -v v "%'+012.2f" 12345,5""", 1100),
    "binary-source": ("source /bin/sh 2>/dev/null", 90),
    "command-missing-hook": ("no_such_tnr", 90),
    "history-repeat": ("history -p '!!:g&' >/dev/null", 750),
    "history-search-reuse": ("history -p '!?word?' '!!:%' >/dev/null", 500),
    "quick-history": ("history -p '^old^new' >/dev/null", 750),
    "custom-history-event": ("history -p '^^' >/dev/null", 750),
    "backquote-escape": (r"""v="`printf '%s' \\`" """, 400),
    "open-quote-alias": ('source ./open-alias', 450),
    "posix-case-alias": ('source ./case-alias', 450),
    "child-trap-wait": ("/usr/bin/true & wait $!", 90),
    "untrapped-wait": ("/usr/bin/true & wait $!", 90),
    "signal-table": ('kill -l >/dev/null', 500),
    "legacy-zone": ("printf -v v '%(%T %z)T' 1275250155", 1100),
    "bind-filter": ('bind -p forward-word >/dev/null', 400),
    "assignment-attributes": ('unset n u; declare -i n; declare -u u; : "${n:=4+3}" "${u:=hello}"', 700),
    "coprocess-moved-ends": ('coproc P { IFS= read -r v; }; pid=$P_PID; exec 5>&${P[1]}-; echo ok >&5; exec 5>&-; wait "$pid"', 64),
    "nameref-loop": ("for outer in a b c; do :; done", 900),
    "nested-operand-quotes": (r'v="${missing:-"AlphaBravoCharlie0123456789\p"}"', 750),
    "nameref-clear": ("declare -n ref=v; declare +n ref=after", 900),
    "stack-tilde": ('p=~+1', 1100),
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
    "trap-return-context": ("invoke", 500),
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
    (root / "open-alias").write_text("message tail'\n")
    (root / "case-alias").write_text("x=$(switch x in x) :;; esac)\n")
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


def comment_dialogue(shell, root):
    (root / "comment-source").write_text("record source #hidden\n")
    # Check the accepted arguments after each setting change in a real session.
    t = Terminal(shell, root, editing=True, env={"HISTFILE": "/dev/null"})
    try:
        assert b"READY>" in t.read(b"READY>")
        for line in (
            "record() { printf '<%s>' \"$@\" >>comment-result; }",
            "again() { eval 'record cached #tag'; }",
            "again; again; again",
            "record before #hidden",
            "shopt -u interactive_comments",
            "record bare #visible",
            "again",
            "source ./comment-source",
            "eval 'record eval #visible'",
            "record 'quoted #text' mid#word",
            "set -o interactive-comments",
            "again",
            "record after #hidden",
        ):
            t.send(line.encode() + b"\nprintf '\\036'\n")
            response = t.read(b"\x1e")
            assert b"\x1e" in response, (shell, line, response)
        observed = (root / "comment-result").read_text()
        expected = ("<cached><cached><cached><before><bare><#visible><cached><#tag>"
                    "<source><eval><#visible><quoted #text><mid#word><cached><after>")
        assert observed == expected, (shell, observed)
    finally:
        t.close()
        (root / "comment-result").unlink(missing_ok=True)



def history_reedit(shell, root):
    result = root / "accepted-history"
    result.unlink(missing_ok=True)
    term = Terminal(shell, root, editing=True, env={"HISTFILE": "/dev/null"})
    try:
        assert b"READY>" in term.read(b"READY>")
        term.send(b"""shopt -s histverify histreedit; record(){ printf '<%s>' "$@" >>accepted-history; printf '\\036'; }; record one\n""")
        assert b"\x1e" in term.read(b"\x1e")
        # Use a distinct history entry so the visible expansion is unambiguous.
        term.send(b"record seed\n")
        assert b"\x1e" in term.read(b"\x1e")
        term.send(b"!!")
        assert b"!!" in term.read(b"!!")
        start = time.perf_counter_ns()
        term.send(b"\n")
        visible = term.read(b"READY>record seed")
        assert b"READY>record seed" in visible, (shell, visible)
        elapsed = (time.perf_counter_ns() - start) / 1e6
        assert result.read_bytes() == b"<one><seed>"
        term.send(b" edited\n")
        assert b"\x1e" in term.read(b"\x1e")
        term.send(b"!absent_event\n")
        assert b"not found" in term.read(b"not found")
        term.send(b"\x01record '\x05'\n")
        assert b"\x1e" in term.read(b"\x1e")
        assert result.read_bytes() == b"<one><seed><seed><edited><!absent_event>"
        return elapsed
    finally:
        term.close()


def readline_input(shell, root, name):
    rc = root / "inputrc"
    rc.write_text('$if mode=emacs\n"\\C-xu": upcase-word\n$endif\n'
                  '"\\C-xk": kill-whole-line\n"\\C-x/": unix-filename-rubout\n'
                  '"\\C-xt": tab-insert\n"\\C-xm": magic-space\n'
                  'set completion-ignore-case on\nset completion-map-case on\n'
                  'set match-hidden-files off\nset mark-directories off\n')
    term = Terminal(shell, root, editing=True, env={"INPUTRC": str(rc), "HISTFILE": "/dev/null"})
    accepted = root / "accepted"
    try:
        assert b"READY>" in term.read(b"READY>")
        term.send(b"record(){ printf '<%s>' \"$@\" >accepted; }; printf '\\036'\n")
        assert b"\x1e" in term.read(b"\x1e")
        if name == "inputrc-word-edit":
            typed = b'record One tWo Three'
            # Start inside the line. Uppercase, capitalize and lowercase three
            # words, transpose the last two, then check what was executed.
            keys = b'\x01\x1bf\x18u\x1bc\x1bl\x1bt'
            expected = b'<ONE><three><Two>'
            visible = b'three Two'
        elif name == "inputrc-file-completion":
            typed = b'record mix_case-uni'
            keys = b'\t'
            expected = b'<Mix-Case-Unique>'
            visible = b'Unique'
        else:
            raise ValueError(name)
        term.send(typed)
        assert typed.split()[-1] in term.read(typed.split()[-1])
        start = time.perf_counter_ns()
        term.send(keys)
        response = term.read(visible)
        assert visible in response, (shell, name, response)
        ms = (time.perf_counter_ns() - start) / 1e6
        term.send(b"; printf '\\036'\n")
        assert b"\x1e" in term.read(b"\x1e")
        assert accepted.read_bytes() == expected, (shell, name, accepted.read_bytes())
        if name == "inputrc-word-edit":
            for line, expected in [
                (b'record ignored\x18krecord kept', b'<kept>'),
                (b'record dir/name\x18/new', b'<dir/new>'),
                (b"record 'a\x18tb'", b'<a\tb>'),
                (b'record word\nrecord !!:$\x18mend', b'<word><end>'),
            ]:
                term.send(line + b"; printf '\\036'\n")
                assert b"\x1e" in term.read(b"\x1e")
                assert accepted.read_bytes() == expected, (shell, line, accepted.read_bytes())
        return ms
    finally:
        term.close()
        accepted.unlink(missing_ok=True)


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
    parser.add_argument("--case", action="append", choices=["history-reedit", "stdin-replacement", "inputrc-word-edit", "inputrc-file-completion", "array-recovery", "interactive-comments", "programmable-boundaries", "filename-completion", "tcp-redirection", "udp-redirection", *HOT])
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
        if os.uname().sysname == "Linux":
            cases["signal-range"] = """for sig in PWR STKFLT RTMIN RTMIN+1 RTMAX-1 RTMAX; do n=$(kill -l "$sig"); echo "$(kill -l "$n")"; trap 'echo handled' "$sig"; kill -s "$sig" "$$"; trap - "$sig"; done"""
        elif os.uname().sysname == "Darwin":
            cases["signal-range"] = """for sig in INFO EMT; do n=$(kill -l "$sig"); echo "$(kill -l "$n")"; trap 'echo handled' "$sig"; kill -s "$sig" "$$"; trap - "$sig"; done"""
        for shell in shells[:2]:
            verbose = subprocess.run([shell, "--verbose", "-c", "echo verbose"], cwd=root, env=env,
                                     capture_output=True, timeout=5)
            assert (verbose.returncode, verbose.stdout, verbose.stderr) == (0, b"verbose\n", b"echo verbose\n"), (shell, verbose)
        # The static shell carries its own UTF-8 data. Runner images need not
        # have the corresponding system locale generated for the control.
        localized = """unset LC_ALL; LC_NUMERIC=de_DE.UTF-8; printf "<%'+012.2f><%'.8d><%-8.1f>\\n" 12345,5 12345 1,5; LC_ALL=C; printf '<%.1f>\\n' 1.5"""
        expected_localized = (0, b"<+0012.345,50><0012.345><1,5     >\n<1.5>\n", False)
        assert run(shells[1], localized)[0] == expected_localized, run(shells[1], localized)[0]
        if run(shells[0], localized)[0] == expected_localized:
            cases["numeric-locale"] = localized
        # Explicit monitor mode can still create job process groups without a
        # controlling terminal. Inspect the actual child rather than just $-.
        pgid = shlex.join([sys.executable, "-c", "import os; print(os.getpid() == os.getpgrp())"])
        monitor = subprocess.run([shells[1], "-m", "-c", pgid + ' & wait "$!"'], start_new_session=True,
                                 cwd=root, env=env, capture_output=True, timeout=5)
        assert (monitor.returncode, monitor.stdout) == (0, b"True\n"), monitor
        if modern:
            for shell in shells[:2]:
                origin = subprocess.run([shell, "-c", "shopt -s extdebug; f(){ :; }; declare -F f", "caller-name"],
                                        cwd=root, env=env, capture_output=True, timeout=5)
                assert (origin.returncode, origin.stdout, origin.stderr) == (0, b"f 1 caller-name\n", b""), (shell, origin)
        for shell in shells[:2]:
            for script in (root, Path("/bin/sh")):
                binary = subprocess.run([shell, script], cwd=root, env=env, capture_output=True, timeout=5)
                assert (binary.returncode, binary.stdout, bool(binary.stderr)) == (126, b"", True), (shell, script, binary)
        failed = []
        for name, code in cases.items():
            expected = run(shells[0], code)[0]
            actual = run(shells[1], code)[0]
            if expected != actual:
                failed.append((name, repr(expected), repr(actual)))
        print(json.dumps(dict(reference=version, witnesses=len(cases), differences=failed)), flush=True)
        if failed:
            raise SystemExit(1)
        # A locally linked Bash readline/tilde library may consult the process
        # environment. A shell assignment to HOME must take effect immediately.
        assert run(shells[1], 'HOME=/example; x=~/src; echo "$x"')[0] == (0, b"/example/src\n", False)
        assert run(shells[1], 'v=雪abc; printf "%s" "${v[@]:0:2}"')[0] == (0, "雪a".encode(), False)
        unicode_ifs = 'set -- a b; a=("$@"); IFS=雪; printf "%s:%s" "$*" "${a[*]}"'
        assert run(shells[1], unicode_ifs)[0] == (0, "a雪b:a雪b".encode(), False)
        # SIGCHLD can be coalesced by the kernel while many children exit.
        # Count wait statuses, and exclude children reaped by the action itself.
        no_child = "n=0; trap '((++n))' CHLD; kill -CHLD $$; echo $n"
        assert run(shells[1], no_child)[0] == (0, b"0\n", False)
        burst = "n=0; trap '((++n))' CHLD; for ((i=0;i<24;i++)); do /usr/bin/true & done; wait; echo $n"
        assert run(shells[1], burst)[0] == (0, b"24\n", False)
        nested_child = "n=0; trap '/usr/bin/true; ((++n))' CHLD; /usr/bin/true; /usr/bin/true; echo $n"
        assert run(shells[1], nested_child)[0] == (0, b"2\n", False)
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
        (termroot / "Mix-Case-Unique").touch()
        for i in range(512):
            (termroot / f"Mix-Case-other{i}").touch()
        if requested("history-reedit"):
            samples = [[] for _ in shells]
            for r in range(args.runs + 1):
                indices = list(range(len(shells)))
                rng.shuffle(indices)
                for i in indices:
                    ms = history_reedit(shells[i], termroot)
                    if r: samples[i].append(round(ms, 3))
            if args.runs:
                print(json.dumps(dict(case="history-reedit", repeats=1, median_ms=[round(statistics.median(g), 3) for g in samples], samples_ms=samples)), flush=True)
        for name in ("inputrc-word-edit", "inputrc-file-completion"):
            if not requested(name): continue
            samples = [[] for _ in shells]
            for r in range(args.runs + 1):
                indices = list(range(len(shells)))
                rng.shuffle(indices)
                for i in indices:
                    ms = readline_input(shells[i], termroot, name)
                    if r: samples[i].append(round(ms, 3))
            if args.runs:
                print(json.dumps(dict(case=name, repeats=1, median_ms=[round(statistics.median(g), 3) for g in samples], samples_ms=samples)), flush=True)
        if requested("stdin-replacement"):
            # Start with both kinds of input and redirect from a function. A
            # named script must keep reading its source after the same exec.
            (root / "redirected").write_text('echo switched\n')
            source = 'f(){ exec <redirected; }; f\necho original\n'
            (root / "input-script").write_text(source)
            for shell in shells[:2]:
                with (root / "input-script").open("rb") as f:
                    result = subprocess.run([shell], stdin=f, capture_output=True, cwd=root, env=env)
                piped = subprocess.run([shell], input=source.encode(), capture_output=True, cwd=root, env=env)
                named = subprocess.run([shell, "input-script"], stdin=subprocess.DEVNULL, capture_output=True, cwd=root, env=env)
                for p, want in [(result,b"switched\n"), (piped,b"switched\n"), (named,b"original\n")]:
                    assert (p.returncode,p.stdout,p.stderr) == (0,want,b""), (shell,p)
            count = 180
            (root / "input-script").write_text('exec 0<&0\n' * count + 'echo done\n')
            series = [[] for _ in shells]
            for r in range(args.runs + 1):
                order = list(range(len(shells))); rng.shuffle(order)
                for i in order:
                    with (root / "input-script").open("rb") as f:
                        start = time.perf_counter_ns()
                        p = subprocess.run([shells[i]], stdin=f, cwd=root, env=env, capture_output=True, timeout=20)
                        ms = (time.perf_counter_ns()-start)/1e6
                    assert (p.returncode,p.stdout,p.stderr) == (0,b"done\n",b""), (shells[i],p)
                    if r: series[i].append(round(ms,3))
            if args.runs:
                print(json.dumps(dict(case="stdin-replacement", repeats=count, median_ms=[round(statistics.median(g),3) for g in series], samples_ms=series)), flush=True)

        if requested("array-recovery"):
            script = "a=(orig)\na=(bad > untouched)\necho \"$?:${a[*]}\"\n"
            broken = "if false; then\n a=(bad > untouched)\n echo unexpected > ran\nfi\n"
            for options, expected in [((), (0, b"1:orig\n")), (("-e",), (2,b"")), (("-n",), (1,b"")), (("--posix",), (1,b""))]:
                for shell in shells[:2]:
                    for pipe in (True, False):
                        p = subprocess.run([shell, "--noprofile", "--norc", *options, *(("-s",) if pipe else ("-c", script))], input=script.encode() if pipe else None, cwd=root, env=env, capture_output=True)
                        # Both GNU references return 127 only for --posix -c.
                        # This syntax error never attempted a command lookup.
                        wanted = (127, b"") if options == ("--posix",) and not pipe and shell == shells[0] else expected
                        assert (p.returncode, p.stdout) == wanted and p.stderr and not (root / "untouched").exists(), (shell, options, pipe, p)
            for pipe in (True, False):
                p = subprocess.run([shells[1], *(("-s",) if pipe else ("-c", broken))], input=broken.encode() if pipe else None, cwd=root, env=env, capture_output=True)
                assert p.returncode == 2 and p.stderr and not (root / "ran").exists()
            multi = "a=(orig)\na=(bad > untouched\n echo wrong > ran\n)\necho \"$?:${a[*]}\"\n"
            for pipe in (True, False):
                p = subprocess.run([shells[1], *(("-s",) if pipe else ("-c", multi))], input=multi.encode() if pipe else None, cwd=root, env=env, capture_output=True)
                assert (p.returncode, p.stdout) == (0,b"1:orig\n") and not (root / "ran").exists(), p
            count = 300
            script = "a=(orig)\n" + "a=(bad > untouched)\n" * count + "echo \"$?:${a[*]}\""
            samples = [[] for _ in shells]
            for r in range(args.runs + 1):
                order = list(range(len(shells))); rng.shuffle(order)
                for i in order:
                    outcome, ms = run(shells[i], script)
                    assert outcome == (0, b"1:orig\n", True) and not (root / "untouched").exists()
                    if r: samples[i].append(round(ms,3))
            if args.runs:
                print(json.dumps(dict(case="array-recovery", repeats=count, median_ms=[round(statistics.median(g),3) for g in samples], samples_ms=samples)), flush=True)
        if requested("interactive-comments"):
            for shell in shells[:2]:
                comment_dialogue(shell, termroot)
                result = run(shell, "shopt -u interactive_comments; set -- kept #ignored\necho \"$#\"")
                assert result[0] == (0, b"1\n", False)
            count = 700
            # Every parsed input line must contribute its literal hash argument.
            code = "total=0\n" + "".join(
                f"set -- #release-component-{i}-ready-for-validation\n((total+=$#))\n" for i in range(count)
            ) + 'echo "$total"'
            samples = [[] for _ in shells]
            for r in range(args.runs + 1):
                indices = list(range(len(shells)))
                rng.shuffle(indices)
                for i in indices:
                    start = time.perf_counter_ns()
                    proc = subprocess.run([shells[i], "--noprofile", "--norc", "+o",
                                           "interactive-comments", "-ic", code],
                                          cwd=root, env=env, capture_output=True, timeout=20)
                    elapsed = (time.perf_counter_ns() - start) / 1e6
                    assert proc.returncode == 0 and proc.stdout == f"{count}\n".encode(), (shells[i], proc.stdout)
                    if r: samples[i].append(round(elapsed, 3))
            if args.runs:
                print(json.dumps(dict(case="interactive-comments", repeats=count, median_ms=[
                    round(statistics.median(g), 3) for g in samples], samples_ms=samples)), flush=True)
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
                "nameref-type-change": "y=3",
                "partial-assoc-init": "declare -A a",
                "typed-export": "r=(one)",
                "globstar-overlap": "shopt -s globstar; mkdir -p tree/a/a/a/a tree/a/b/a/b tree/b/a/a/b",
                "local-set-options": "f(){ local -; set -f; :; }",
                "getopts-readonly-end": "readonly v; exec 2>/dev/null",
                "history-event-words": "history -s 'cmd a.old a.html a.h   x|y 2>/dev/null'",
                "numeric-locale": "unset LC_ALL; LC_NUMERIC=de_DE.UTF-8",
                "multiline-history": "set -o history",
                "scalar-array-slice": "text=雪abcdef",
                "quoted-star-order": 'set -- one "two words" 雪',
                "function-stack": "f(){ FUNCNAME=ignored; :; }",
                "read-one-field": "printf '  one  two  \\n' >fields",
                "read-fields": r"IFS=$'\t\r\f\v'; printf '  left\tright \t\r\f\v\n' >fields",
                "function-origin": "shopt -s extdebug",
                "command-missing-hook": "command_not_found_handle(){ :; }",
                "history-repeat": "history -s 'echo old old'; history -p '!!:s/old/new/' >/dev/null",
                "history-search-reuse": "history -s 'echo matched-word last'",
                "quick-history": "history -s 'echo old old'",
                "custom-history-event": "histchars='^:#'; history -s 'echo first last'",
                "open-quote-alias": "shopt -s expand_aliases; alias message=\": 'head\"",
                "posix-case-alias": "set -o posix; shopt -s expand_aliases; alias switch=case",
                "debug-arguments": "set -- one two three",
                "bind-filter": "exec 2>/dev/null",
                "legacy-zone": "export TZ=EST5EDT",
                "keyword-arguments": "set -k; inspect(){ :; }",
                "compat-random": "BASH_COMPAT=50; RANDOM=42",
                "compat-regex": "BASH_COMPAT=31; re='^a.*z$'",
                "compat-loop": "BASH_COMPAT=43; f(){ break; }",
                "nameref-loop": "readonly a=one b=two c=three; declare -n inner=a outer=inner",
                "compat-array": "BASH_COMPAT=51; declare -A m=([k]=v)",
                "assignment-tilde": "base=/bin",
                "stack-tilde": "pushd src >/dev/null",
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
                "child-trap-wait": "n=0; trap '((++n))' CHLD",
                "trap-return-context": "invoke(){ kill -USR1 $$; }; trap ':; return' USR1",
                "trap-action": "trap ':' USR1", "trap-signal": "trap 's=$BASH_TRAPSIG' USR1",
                "glob-size": "GLOBSORT=size", "glob-numeric": "cd items; GLOBSORT=numeric",
            }.get(name, ":")
            code = f"{setup}; for ((i=0;i<{count};i++)); do {body}; done; echo done"
            if name == "history-physical-lines":
                code = ("exec 2>/dev/null\nset -o history -H\n: seed\n"
                        + "if :; then\n: !!:$\nfi\n" * count + "echo done")
            elif name == "partial-assoc-init":
                code = ("exec 2>/dev/null\ndeclare -A a\n"
                        + "a=([one]=1 [two]=2 bad [extra]=3)\n" * count
                        + '[[ ${a[one]} == 1 && ${a[two]} == 2 && -z ${a[extra]+set} ]] && echo done')
            elif name == "multiline-history":
                # Input history is recorded for top-level commands, even with
                # no terminal. Executing a sourced loop would bypass this path.
                code = "set -o history\n" + "for v in one two\ndo\n  : item\ndone\n" * count + "echo done"
            elif name in ("positional-assignment-error", "invalid-length"):
                code = "exec 2>/dev/null\n" + (body + "\n") * count + "echo done"
            elif name in ("backquote-escape", "ansi-braced-byte", "nested-operand-quotes"):
                # Parse a fresh top-level assignment for each source spelling.
                code = "".join(f"v={i}{body[2:]}\n" for i in range(count)) + "echo done"
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
