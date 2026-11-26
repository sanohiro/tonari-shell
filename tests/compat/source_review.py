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
    "stopped-job-selection": "set -m; sleep 3 & a=$!; sleep 3 & b=$!; sleep 3 & c=$!; trap 'kill -CONT \"$a\" \"$b\" \"$c\" 2>/dev/null; kill \"$a\" \"$b\" \"$c\" 2>/dev/null; wait 2>/dev/null' EXIT; mark(){ x=$(jobs -p %+); y=$(jobs -p %-); for p in \"$x\" \"$y\"; do case $p in \"$a\") printf a;; \"$b\") printf b;; \"$c\") printf c;; *) printf '?';; esac; done; echo; }; mark; kill -STOP \"$b\"; wait \"$b\" 2>/dev/null; mark; kill -CONT \"$b\"; for ((k=0;k<100;k++)); do [[ -z $(jobs -ps) ]] && break; sleep .01; done; mark; kill -STOP \"$a\"; wait \"$a\" 2>/dev/null; mark; bg %1 >/dev/null; mark",
    "source-trap-frame": "printf ': one\\n: two\\n' > sourced; set -T\ntrap 'echo \"${FUNCNAME[0]}:$LINENO:$1\"' RETURN\nf(){\n source ./sourced inner\n}\nf outer",
    "substitution-source-lines": "f(){\n :\n echo \"$(echo $LINENO)\" `echo $LINENO`\n echo \"$(echo \"$LINENO\"; echo \"$(echo $LINENO)\")\"\n}; f",
    "scalar-array-declaration": "declare -lr var; printf '<%s>\\n' \"${var@A}\" \"${var[@]@A}\"; declare -l setvar=''; printf '<%s>\\n' \"${setvar@A}\" \"${setvar[@]@A}\"; declare -a arr; printf '<%s>\\n' \"${arr[@]@A}\"",
    "temporary-posix-settings": "shopt -s expand_aliases; shopt -u sourcepath inherit_errexit shift_verbose; f(){ shopt -p expand_aliases sourcepath inherit_errexit shift_verbose; }; POSIXLY_CORRECT=y f; f; shopt -qo posix; echo $?",
    "history-script-comments": "set -o history\n# first\n   \n   # second\nhistory",
    "fc-range-substitutions": "history -c; history -s 'echo one'; history -s 'echo axb'; history -s 'echo three'; fc -ln 999; fc -ln -0; fc -ln 0; fc -ln 3; fc -ln 1 999; fc -s a=y b=z 2 2>/dev/null; fc -s -0 2>/dev/null; echo $?",
    "caller-input": "f(){ caller foo 2>err; echo \"$?\"; test -s err; echo $?; caller -1 2>err; echo \"$?\"; caller 23; echo $?; }; f",
    "debug-function-entry-return": "set -T\ntrap 'echo D.$FUNCNAME.$LINENO' DEBUG\ntrap 'echo R.$FUNCNAME.$LINENO' RETURN\nf()\n{\n : one\n : two\n}\nf\nf(){ return 7; }; f; trap - DEBUG RETURN; shopt -s extdebug; declare -F f | awk '{print $1,$2}'",
    "bind-human-list": "{ bind -u beginning-of-line; bind '\"zzq\": beginning-of-line'; bind -q beginning-of-line; bind -P | grep '^beginning-of-line '; } 2>/dev/null",
    "alias-option-processing": "alias -- -x=ok; alias -pp -- -x; alias -Z 2>/dev/null; echo $?; unalias -aZ 2>/dev/null; echo $?; alias -- -x; unalias -aa; alias -p",
    "arithmetic-assignment-index": "n=0; ((a[n]=++n)); n=0; ((b[n]+=++n)); n=0; ((c[n++]=n++)); declare -p a b c; echo \"$n\"; n=0; a='(a[n]=++n)<7&&a[0]'; ((a[0])); echo \"${a[@]:1}\"",
    "ascii-range-option": "LC_ALL=C; for opt in -s -u -s; do shopt \"$opt\" globasciiranges; shopt -q globasciiranges; printf '%s:' \"$?\"; for c in b B z Z; do [[ $c == [a-z] ]]; printf '%s' \"$?\"; done; echo; done",
    "gnu-diagnostic-location": "shopt -s gnu_errfmt; printf '%s\\n' 'shopt -s gnu_errfmt' 'no_such_audit_tool' >location.sh; . ./location.sh 2>error; grep -Eq 'location.sh:2: .*command not found' error; echo $?; shopt -u gnu_errfmt; shopt -q gnu_errfmt; echo $?",
    "xtrace-for-case-header": "PS4='+ '; exec 9>trace; BASH_XTRACEFD=9; set -x; set -- a 'b c'; for x; do :; done; case \"$1\" in a) :;; esac; set +x; cat trace",
    "global-nameref-shadow": "declare -n ref=foo; f(){ local -n ref=var; declare -g ref=(X); declare -p ref foo; }; f; declare -p ref foo; declare -p var 2>/dev/null; echo $?",
    "parenthesized-scalar-suffix": "a=(\"one two\" three)tail; declare -p a; declare -i x; x=(4*3)/2; declare -p x; let x=(5+3)/2; echo \"$x\"",
    "nested-default-apostrophe": "unset u; printf '<%s>\\n' \"${u-'\"'hey}\" \"${u-'\"hey'}\" \"${u-'one\"two\"three'}\"",
    "posix-time-utility": "set -o posix\nalias time='printf \"<%s>\\\\n\"'\ntime -p\ntime --help",
    "history-ansi-quote": "set -o history -H\nprintf '%s\\n' $'hello\\' once!'\nprintf '%s\\n' $'two\n\\' lines!'\necho last",
    "posix-resume-job": "set -o posix; set -m; sleep 4 & p=$!; kill -s STOP \"$p\"; sleep .05; jobs -s | awk '{print $2}'; bg %+; kill \"$p\"; wait \"$p\" 2>/dev/null; :",
    "posix-job-state": "set -o posix; (exit 17) & p=$!; sleep .04; jobs | awk '{print $2}'; wait \"$p\"; echo \"$?\"",
    "debug-return-scope": "set -T; helper(){ :; }; f(){ :; }; trap 'echo \"ret:${FUNCNAME[0]}\"' RETURN; trap 'helper' DEBUG; f",
    "debug-handler-function-line": "set -T\nhelper(){\n printf '%s %s %s\\n' \"$1\" \"$LINENO\" \"$BASH_COMMAND\"\n}\ntrap 'helper \"$LINENO\"' DEBUG\n:\n:",
    "ere-portable-punctuation": "for re in '[a[.-.]z]' '[[.].]]' '[[.^.]]' '[[.[.]]'; do for s in '-' m ']' '^' '['; do [[ $s =~ $re ]]; printf '%s' \"$?\"; done; echo; done",
    "tilde-associative-key": "declare -A a; a[~/key]=absolute; a['~/quoted']=literal; k=\"$HOME/key\"; printf '%s/%s' \"${a[$k]}\" \"${a['~/quoted']}\"",
    "ansi-associative-key": "declare -A a; a[$'two\\x20words']=green; k='two words'; printf '%s' \"${a[$k]}\"",
    "quoted-numeric-subscript": "a=(zero); let 'a[\"\"]=26'; declare 'a[\" \"]=14'; let 'a[\"2+1\"]=5'; declare -p a",
    "posix-prompt-expansion": "set -o posix; shopt -u promptvars; v=visible; p='!!/$v'; printf '%s\\n' \"${p@P}\"",
    "posix-ulimit-blocks": "ulimit -S -f 32; set -o posix; ulimit -S -f; ulimit -S -f 40; set +o posix; ulimit -S -f",
    "posix-signal-operands": "set -o posix; kill -l | awk '{n+=NF} END {print NR,(n>10)}'; kill -s SIGTERM 99999999 2>&1 | grep -Eq 'invalid signal'; echo $?",
    "posix-fc-format": "set -o posix; set -o history\nhistory -s 'echo first'\nfc -ln -1",
    "posix-logical-directory": "set -o posix; mkdir -p movedir/leaf; cd movedir; mv ../movedir ../gone; cd leaf 2>/dev/null; echo $?; echo \"$(pwd | sed 's|.*/||')\"; cd \"$HOME\"; rmdir gone/leaf gone",
    "posix-comments-reset": "shopt -u interactive_comments; set -o posix; shopt -q interactive_comments; echo $?",
    "posix-consume-wait": "set -o posix; (exit 9) & p=$!; wait \"$p\"; echo \"$?\"; wait \"$p\" 2>/dev/null; echo \"$?\"",
    "posix-special-in-function": "set -o posix; a=outer; f(){ a=3 readonly a; }; a=7 f; declare -p a; g(){ local q=local; q=inside :; echo \"$q\"; }; q=outside; q=temp g; echo \"$q\"",
    "trace-arithmetic-loop": "PS4='+'; { set -x; for (( j=$((++v)); j<2; j++ )); do :; done; for ((;;)); do break; done; set +x; } 2>trace; cat trace; echo \"$v:$j\"",
    "trace-arithmetic-expansion": "PS4='+'; { set -x; (( x=$((++v)) )); set +x; } 2>trace; cat trace; echo \"$x:$v\"",
    "function-render-attributes": "f(){ :; }; export -f f; type f; declare -f f; declare -pf f; export -pf; set | sed -n '/^f ()/,$p'",
    "conditional-trace-operator": "PS4='+ '; exec 2>trace; set -x; [[ a > b ]]; set +x; cat trace",
    "failed-function-trap-text": "trap 'echo \"ERR:$LINENO:$BASH_COMMAND\"' ERR\nf(){\n [[ -z nonempty ]]\n}\nf",
    "extended-operand-quotes": "hi=ok; unset u; shopt -u extquote\nprintf '<%s>\\n' \"${u-$'a\\tb'}\" \"${u-$\"hi\"}\"; shopt -s extquote\nprintf '<%s>\\n' \"${u-$'a\\tb'}\" \"${u-$\"hi\"}\"",
    "ere-bracket-quotes": "[[ '\\' =~ [\".\"] ]]; echo $?; [[ ']' =~ [']'] ]]; echo $?; [[ dog =~ ([[.d.][.D.]])o(.) ]]; printf '%s/%s/%s\\n' \"$?\" \"${BASH_REMATCH[1]}\" \"${BASH_REMATCH[2]}\"; [[ dog =~ ['[=d=]'].. ]]; echo \"$?\"",
    "remove-nameref-attributes": "declare -n r=dest; declare +n -i r=7+4; declare -p r dest; declare -nr locked=dest; declare +n locked; echo \"$?\"; declare -p locked",
    "coproc-nameref": "arr=(previous); declare -n r=arr; coproc r { read -r line; test \"$line\" = sent; }; pid=$!; test \"$pid\" = \"$r_PID\"; echo \"$?\"; echo sent >&\"${r[1]}\"; wait \"$pid\"; echo \"$?\"",
    "read-array-nameref": "declare -n r=arr; read -a r <<<'a b'; declare -p r arr; declare -n unbound; read -a unbound <<<'c d'; declare -p unbound",
    "mapfile-nameref": "declare -n r=arr; mapfile -t r <<<'hello'; declare -p r arr; mapfile -O 2 -t r <<<'end'; declare -p arr; declare -n element='arr[2]'; mapfile -t element <<<'ignore'; echo status:$?; declare -p arr",
    "encoded-parameter-name": "x1=not; x=notOK; echo \"${x#${$'x1'%$'t'}}\"; echo \"${x$'\\x31'}\"; cat <<EOF\n${x#${$'x1'%'t'}}\nEOF",
    "operand-process-substitution": "unset v; cat ${v:-<(printf ok)}; printf '[%s]' \"${v:-<(echo a)}\"; v=yes; : ${v:-<(touch wrong)}; test ! -e wrong",
    "nameref-target-syntax": "for n in 'x[a][b]' 'x[foo]bar]' 'x[]' 'x[ ]' 'x[[1]]' 'x[2+1]'; do unset -n ref; declare -n ref=\"$n\"; echo \"$?\"; done",
    "array-named-descriptor": "declare -A m; key=out; exec {m[$key]}>result; echo 'ok' >&\"${m[out]}\"; exec {m[$key]}>&-; cat result; exec {a[3]}>&1; test \"${a[3]}\" -gt 2; echo $?; exec {a[3]}>&-",
    "conditional-arithmetic-error": "[[ 1 -eq 4+ ]]; echo \"$?\"; [[ 1 -eq 4+ || 1 -eq 1 ]]; echo \"$?\"",
    "replacement-backslash": "v=letssee; pattern=ss; for r in 'x&y' 'x\\&y' '\\\\&'; do printf '<%s>\\n' \"${v//$pattern/$r}\" \"${v//$pattern/\"$r\"}\"; done",
    "quote-set-values": "set -o posix; ZZ1=\"'\"; ZZ2=\"'abc\"; ZZ3=\"abc'\"; ZZ4='ab#cd'; ZZ5='a~b'; ZZ6='#abcd'; set | sed -n '/^ZZ[0-9]/p'",
    "posix-input-capture": "printf literal >'glob-*'; printf another >glob-a; set -o posix; p='glob-*'; result=$(< $p); echo \"$? <$result>\"; printf spaced >'with space'; p='with space'; result=$(< $p); echo \"$? <$result>\"",
    "exit-trap-frame": "( f(){ local item=inner; trap 'printf \"<%s/%s/%s>\\n\" \"$FUNCNAME\" \"$item\" \"$1\"' EXIT; exit 7 >result; }; f arg ); echo \"$?\"; cat result",
    "posix-background-parameter": "set -o posix\n: &\nj=$!; wait \"$j\"; test \"${!?}\" = \"$j\"; echo \"$?\"",
    "descriptor-overflow": "printf '<%s>\\n' 99999999999999999999999</dev/null; : 33</dev/null; echo \"$?\"",
    "quoted-operand-brace": "v=set; x=value; printf '<%s>\\n' \"${v-'}'}\" \"${w-'}'}\" \"${w-'$x}'}\"",
    "eval-array-operand": "a=([3]=one [5]=two); f(){ local -a s; eval s=(\\\"\\${!${1}[@]}\\\"); declare -p s; }; f a",
    "prefix-declaration": "x=outer; f(){ x=inner local x; printf '<%s>' \"$x\"; }; f; echo \"$x\"; y=old; y=temp readonly y; echo \"$y\"",
    "array-declaration-print": "declare -r 'c[100]'; declare -p c; declare -a 'e[10]=(x y)'; declare -p e; export e; set -o posix; readonly -a | grep ' c$'; export -p | grep ' e='",
    "slice-error-recovery": "set -- a b; a=hello; b=(one two)\nprintf ignored \"${@:0:-2}\"; echo skipped\necho \"$?\"\nprintf ignored \"${b[@]:0:-2}\"; echo skipped\necho \"$?\"\nset -o posix\nprintf ignored \"${a:2:-7}\"; echo skipped\necho \"continued:$?\"",
    "arithmetic-adjacent-signs": "v=4; echo $((++7)) $((--7)) $((5--2)) $((3+++2)) $((--(v))) $v; echo $((++v)) $((v++)) $v",
    "portable-bracket-members": "s=hello; echo \"${s//[[:alpha]/}\"; for p in '[[.space.]]' '[[.hyphen.]-[.nine.]]' '[[.tab.]]' '[[:ascii:]]' '[[.unknown.]a]'; do for v in ' ' $'\\t' '-' 5 a 雪; do [[ $v == $p ]]; printf '%s' \"$?\"; done; echo; done",
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
    "current-arithmetic-alias": "shopt -s expand_aliases\nalias number='echo 123'\necho $(( ${ number; } ))\n(( ${ number; } )); echo $?\nset -o posix\necho $(( ${ number; } ))\n(( ${ number; } )); echo $?",
    "long-working-directory": "set -o posix; cd deep-root; p=deep-part-abcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcd; for ((j=0;j<20;j++)); do cd \"$p\" || exit 9; done; logical=$(pwd -L); phys=$(pwd -P); [[ $logical == */\"$p\" && $phys == */\"$p\" && ${#logical} -gt 4096 && ${#phys} -gt 4096 ]]; echo $?; cd ..; test -d \"$p\"; echo $?",
    "printf-wide-characters": "case $OSTYPE in darwin*) LC_ALL=en_US.UTF-8;; *) LC_ALL=C.UTF-8;; esac; printf '[%4.2ls][%-4C][%S][%.0lc]' '雪🌸a' '雪a' '四' q; printf '%lc' ''",
    "printf-alternate-quote": "printf '[%#q][%#.3Q]\\n' 'a b' 'a b c' '' 'ok' $'x\\ny' \"ab'\"; printf '%#q\\n' 'no-quotes-needed'",
    "posix-fc-operands": "set -o posix; set -o history\nhistory -s 'echo first'\nfc -s 1 1 1; echo \"$?\"",
    "posix-function-path": "function ./absent { echo wrong; }; set -o posix; ./absent 2>/dev/null; echo $?",
    "posix-trap-display": "set -o posix; trap ':' USR1; trap -p USR1; trap USR1; echo after",
    "alias-display-selected": "set -o posix; alias demo='echo hi'; alias demo; alias -p demo",
    "posix-substitution-status": "set -o posix; false; echo \"$(true)/$?\"; x=$(exit 7); echo \"$?\"",
    "nameref-element-length": "a=(zero abc); declare -n r='a[$(printf 1)]'; declare -n all='a[@]'; printf '%s/%s/%s\\n' \"${#r}\" \"$r\" \"${#all}\"",
    "array-single-evaluation-option": "shopt -s assoc_expand_once; shopt -q array_expand_once; echo $?; a=(one two); i=0; echo \"${a[i++]}:$i\"; shopt -u array_expand_once; shopt -q assoc_expand_once; echo $?",
    "lazy-malformed-operand": "a=4\necho \"${a-'$('\\'}\"\necho \"${a+'$('\\'}\"\necho survived",
    "posix-command-precedence": "break(){ echo fn; }; set -o posix; type -t break; type -at break; command -V break",
    "invalid-ere": "re='a('; [[ x =~ $re ]]; echo \"$?\"; re='(a)(b)?'; [[ a =~ $re ]]; declare -p BASH_REMATCH",
    "indexed-init-recovery": "a=(old)\na=([0]=start [1]=yes [-3]=bad [2]=later); echo skipped\necho \"$?\"; declare -p a\na=([]=bad [3]=end); echo skipped\necho \"$?\"; declare -p a",
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
    "stopped-job-selection": ("jobs -p %+ %- >/dev/null", 1200),
    "source-trap-frame": ("f >/dev/null", 600),
    "substitution-source-lines": ("f >/dev/null", 250),
    "current-arithmetic-alias": ("eval 'value=$(( ${ number; } ))'", 750),
    "scalar-array-declaration": (": \"${var[@]@A}\" \"${setvar[@]@A}\"", 1600),
    "temporary-posix-settings": ("POSIXLY_CORRECT=x true; :", 1800),
    "history-script-comments": ("history -s '# history comment'; history -d -1", 950),
    "fc-range-substitutions": ("history -c; history -s ': axb'; history -s ': two'; fc -ln 100 -1 >/dev/null; fc -s a=y b=z 1 >/dev/null 2>&1", 600),
    "caller-input": ("f", 2000),
    "debug-function-entry-return": ("f; :", 650),
    "bind-human-list": ("bind -q beginning-of-line >/dev/null 2>&1", 900),
    "long-working-directory": ("pwd -P >/dev/null; pwd -L >/dev/null", 18),
    "alias-option-processing": ("alias -- -sample=yes; alias -pp -- -sample >/dev/null; unalias -- -sample", 1100),
    "arithmetic-assignment-index": ("n=0; ((a[n]=++n)); ((a[n]+=n++))", 1600),
    "ascii-range-option": ("[[ b == [a-z] ]]; [[ Z == [a-z] ]] || :", 2000),
    "gnu-diagnostic-location": ("no_such_audit_tool 2>/dev/null; :", 65),
    "xtrace-for-case-header": ("for x in a b c; do :; done; case \"$x\" in c) :;; esac", 800),
    "global-nameref-shadow": ("f", 1600),
    "parenthesized-scalar-suffix": ("x=(4*3)/2; let x=(5+3)/2", 1400),
    "nested-default-apostrophe": (": \"${u-'one\"two\"three'}\"", 2600),
    "posix-time-utility": ("eval 'time -p' >/dev/null", 1100),
    "history-ansi-quote": ("history -p \"echo \\$'hello\\\\' once!'\" >/dev/null", 1000),
    "printf-wide-characters": ("printf -v v '%6.3ls:%C' '雪🌸abc' '日'", 2400),
    "printf-alternate-quote": ("printf -v v '%#q:%#.3Q' 'a b c' 'a;b'", 2300),
    "posix-resume-job": ("/bin/sh -c 'kill -STOP $$; exit 7' & p=$!; wait \"$p\"; bg %+ >/dev/null; wait \"$p\"; [[ $? == 7 ]] || exit 9", 35),
    "posix-job-state": ("(exit 17) & p=$!; wait \"$p\"; :", 80),
    "debug-return-scope": ("f", 900),
    "debug-handler-function-line": ("trap 'helper \"$LINENO\"' DEBUG; :; :; trap - DEBUG", 700),
    "ere-portable-punctuation": ("for re in '[a[.-.]z]' '[[.].]]' '[[.^.]]' '[[.[.]]'; do [[ z =~ $re ]]; done", 800),
    "tilde-associative-key": ("a[~/key]=green; : \"${a[~/key]}\"", 1800),
    "ansi-associative-key": ("a[$'two\\x20words']=green; : \"${a[$'two\\x20words']}\"", 1600),
    "quoted-numeric-subscript": ("let 'a[\"k+1\"]=5'; declare 'a[\" \"]=14'", 1700),
    "posix-prompt-expansion": ("s=${p@P}", 2700),
    "posix-ulimit-blocks": ("ulimit -S -f >/dev/null", 2500),
    "posix-trap-display": ("trap -p USR1 >/dev/null", 1900),
    "posix-signal-operands": ("kill -l >/dev/null; kill -s SIGTERM 99999999 2>/dev/null", 1200),
    "posix-fc-operands": ("fc -s 1 1 1 2>/dev/null", 1400),
    "posix-fc-format": ("fc -ln -1 >/dev/null", 1400),
    "posix-logical-directory": ("pwd >/dev/null; cd .", 900),
    "alias-display-selected": ("alias -p demo >/dev/null", 1400),
    "posix-comments-reset": ("set +o posix; shopt -u interactive_comments; set -o posix; shopt -q interactive_comments", 1600),
    "posix-consume-wait": ("(exit 9) & p=$!; wait \"$p\"; wait \"$p\" 2>/dev/null", 85),
    "posix-function-path": ("./absent", 70),
    "posix-substitution-status": ("false; v=\"$(true)$?\"", 350),
    "posix-special-in-function": ("q=temp f", 2000),
    "trace-arithmetic-loop": ("for ((j=0;j<3;j++)); do :; done", 700),
    "trace-arithmetic-expansion": ("(( x=$v + 1 ))", 2500),
    "nameref-element-length": (": \"${#r}\" \"${#all}\"", 2600),
    "function-render-attributes": ("type f >/dev/null; declare -f f >/dev/null", 1200),
    "conditional-trace-operator": ("[[ a > b ]]", 2600),
    "failed-function-trap-text": ("f", 1700),
    "array-single-evaluation-option": ("shopt -s assoc_expand_once; idx=0; : \"${a[idx++]}\"; shopt -u array_expand_once", 2200),
    "extended-operand-quotes": (": \"${u-$'a\\tb'}\" \"${u-$\"hi\"}\"", 2200),
    "ere-bracket-quotes": ("[[ dog =~ (['d'][[.o.]])([g]) ]]", 3000),
    "remove-nameref-attributes": ("declare -n r=dest; declare +n -i r=7+4", 3000),
    "coproc-nameref": ("coproc r { read -r line; }; pid=$!; echo sent >&\"${r[1]}\"; wait \"$pid\"", 60),
    "read-array-nameref": ("read -a r <<<'one two three'", 1000),
    "mapfile-nameref": ("mapfile -t r <<<\"one\"", 900),
    "lazy-malformed-operand": (": \"${a-'$('\\'}\"", 2000),
    "encoded-parameter-name": (": \"${x#${$'x1'%$'t'}}\"", 2000),
    "operand-process-substitution": ("read -r s < ${v:-<(echo hi)}", 90),
    "nameref-target-syntax": ("declare -n ref='arr[12]'; unset -n ref", 3000),
    "array-named-descriptor": ("exec {m[$key]}>&1; exec {m[$key]}>&-", 1300),
    "posix-command-precedence": ("type -t break >/dev/null", 2500),
    "invalid-ere": ("[[ x =~ $re ]]", 2200),
    "conditional-arithmetic-error": ("[[ 1 -eq 4+ ]]", 4000),
    "replacement-backslash": (": \"${v//$pattern/$r}\" \"${v//$pattern/\"$r\"}\"", 3500),
    "quote-set-values": ("set >/dev/null", 500),
    "posix-input-capture": ("v=$(< $p)", 110),
    "exit-trap-frame": ("( f arg )", 90),
    "posix-background-parameter": (": \"${!?}\"", 7000),
    "descriptor-overflow": (": 99999999999999999999999</dev/null", 2500),
    "quoted-operand-brace": (": \"${w-'$x}'}\"", 7000),
    "eval-array-operand": ("eval s=(\\\"\\${!${name}[@]}\\\")", 3500),
    "prefix-declaration": ("f", 5000),
    "indexed-init-recovery": ("a=([0]=start [1]=yes [-3]=bad)", 800),
    "array-declaration-print": ("declare -a 'e[10]=(x y)'; declare -p e >/dev/null", 500),
    "slice-error-recovery": (":", 800),
    "arithmetic-adjacent-signs": ("((v=7--2+ ++4)); ((++v))", 1700),
    "portable-bracket-members": ("[[ $text == $pat ]]; v=${text//[[:alpha]/}", 1000),
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


EXTRA_CASES = ("pretty-print-file", "foreground-signal-notice")
EDITOR_PTY = ("vi-external-editor", "emacs-external-editor", "posix-vi-editor")
DIRECTORY_PTY = ("cd-spelling", "directory-spelling-completion", "expanded-directory-completion", "quoted-directory-completion", "check-exit-jobs")

def external_editor_pty(shell, root, name):
    # Both executables verify the original buffer and replace it with an observable command.
    for editor in ("chosen", "vi"):
        exe = root / editor
        exe.write_text("#!/bin/sh\ngrep -Fx 'echo original' \"$1\" >/dev/null || exit 9\n"
                       f"echo 'printf {editor} >editor-result; printf \"\\\\036\"' >\"$1\"\n")
        exe.chmod(0o755)
    result = root / "editor-result"
    result.unlink(missing_ok=True)
    env = dict(PATH=str(root) + ":/usr/bin:/bin", VISUAL="chosen", EDITOR="false", HISTFILE="/dev/null")
    term = Terminal(shell, root, editing=True, env=env)
    try:
        assert b"READY>" in term.read(b"READY>")
        vi = name != "emacs-external-editor"
        if vi:
            term.send(b"set -o vi" + (b" -o posix" if name == "posix-vi-editor" else b"") + b"\n")
            assert b"READY>" in term.read(b"READY>")
        term.send(b"echo original")
        term.read(b"original")
        start = time.perf_counter_ns()
        term.send(b"\x1bv" if vi else b"\x18\x05")
        output = term.read(b"\x1e", timeout=3)
        elapsed = (time.perf_counter_ns() - start) / 1e6
        assert b"\x1e" in output and result.read_text() == ("vi" if name == "posix-vi-editor" else "chosen"), (shell, name, output)
        return elapsed
    finally:
        term.close()


def directory_pty(shell, root, name):
    term = Terminal(shell, root, editing=True, env={"HISTFILE": "/dev/null"})
    result = root / "accepted-path"
    result.unlink(missing_ok=True)
    pidfile = root / "job-pid"
    pidfile.unlink(missing_ok=True)
    try:
        assert b"READY>" in term.read(b"READY>")
        if name == "check-exit-jobs":
            term.send(b"shopt -s checkjobs; sleep 10 & echo $! >job-pid; printf '\\036'\n")
            assert b"\x1e" in term.read(b"\x1e")
            start = time.perf_counter_ns()
            term.send(b"exit 23\n")
            observed = term.read(b"There are running jobs.")
            elapsed = (time.perf_counter_ns() - start) / 1e6
            assert b"There are running jobs." in observed, (shell, observed)
            after = observed.split(b"There are running jobs.", 1)[1]
            if b"READY>" not in after: after += term.read(b"READY>")
            assert b"sleep 10" in after and b"READY>" in after, (shell, after)
            term.send(b"exit 23\n")
            # Drain the terminal while the foreground process restores its tty.
            deadline = time.monotonic() + 4
            while term.proc.poll() is None and time.monotonic() < deadline:
                term.read(b"\x00", timeout=.02)
            assert term.proc.poll() == 23, (shell, term.proc.poll())
            return elapsed
        flag, text, expected = {
            "cd-spelling": ("cdspell", 'cd samle; printf "%s" "$PWD"', str(root.resolve() / "sample")),
            "directory-spelling-completion": ("direxpand dirspell", 'printf "%s" sapmle/uni\t', str(root.resolve() / "sample/unique")),
            "expanded-directory-completion": ("direxpand", 'printf "%s" $q/uni\t', str(root.resolve() / "sample/unique")),
            "quoted-directory-completion": ("complete_fullquote", 'printf "%s" $q/uni\t', "sample/unique"),
        }[name]
        term.send(f"q=sample; shopt -s {flag}; printf '\\036'\n".encode())
        assert b"\x1e" in term.read(b"\x1e")
        start = time.perf_counter_ns()
        term.send((text + ' >"$HOME/accepted-path"; printf "\\035"\n').encode())
        observed = term.read(b"\x1d")
        elapsed = (time.perf_counter_ns() - start) / 1e6
        assert b"\x1d" in observed and result.read_text() == expected, (name, shell, observed, result.read_text())
        return elapsed
    finally:
        if pidfile.exists():
            try: os.kill(int(pidfile.read_text()), 15)
            except ProcessLookupError: pass
        term.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bash", type=Path)
    parser.add_argument("tonari", type=Path)
    parser.add_argument("--first", type=Path)
    parser.add_argument("--runs", type=int, default=9)
    parser.add_argument("--case", action="append", choices=["history-reedit", "stdin-replacement", "inputrc-word-edit", "inputrc-file-completion", "array-recovery", "interactive-comments", "programmable-boundaries", "filename-completion", "tcp-redirection", "udp-redirection", *DIRECTORY_PTY, *EDITOR_PTY, *EXTRA_CASES, *HOT])
    args = parser.parse_args()
    selected = set(args.case or ())
    def requested(name):
        return not selected or name in selected
    shells = [p.resolve() for p in (args.bash, args.tonari, args.first) if p]
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        fixture(root)
        # Create each component relative to an open parent, beyond PATH_MAX on musl.
        deep = root / "deep-root"
        deep.mkdir()
        parent = os.open(deep, os.O_RDONLY)
        try:
            for _ in range(20):
                component = "deep-part-" + "abcd" * 53
                os.mkdir(component, dir_fd=parent)
                child = os.open(component, os.O_RDONLY, dir_fd=parent)
                os.close(parent)
                parent = child
        finally:
            os.close(parent)
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
        if os.uname().sysname == "Darwin":
            locale_range = """LC_ALL=en_US.UTF-8; for opt in -s -u -s; do shopt "$opt" globasciiranges; for ch in b B é; do [[ $ch == [a-z] ]]; echo "$?"; done; done; LC_ALL=C; [[ B == [a-z] ]]; echo "$?" """
            cases["native-locale-range"] = locale_range
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
        # POSIX requires a full delimiter line and the subshell's final status.
        # The reference Bash releases differ from those two guarantees.
        for code, output in [
            ('! false & launched=$?; wait "$!"; echo "$launched:$?"', b"0:0\n"),
            ('IFS=:; set -- "a b" c; unset v; printf "<%s>" ${v-$@}', b"<a b><c>"),
            ('x=$(cat <<EOF\none\nEOF) middle\ntwo\nEOF\n)\nprintf "%s\\n" "$x"', b"one\nEOF) middle\ntwo\n"),
        ]:
            assert run(shells[1], code)[0] == (0, output, False), code
        # Preserve valid data and bindings when an operation cannot succeed.
        for code, output in [
            ('readonly fd=42; printf keep >protected; { : {fd}>protected; } 2>/dev/null; printf "%s/" "$?"; cat protected', b"1/keep"),
            ('export local_env_probe=outer; f(){ local local_env_probe=inner; declare +x local_env_probe; printenv local_env_probe || echo hidden; }; f', b"hidden\n"),
            ('declare -n ref; declare ref=123 2>/dev/null; echo "$?"; declare -p ref', b"1\ndeclare -n ref\n"),
            ('declare -n job=ends; coproc job { :; }; echo "${job_PID:+named}/${ends_PID:+resolved}"; wait', b"named/\n"),
            ("declare -n job='ends[0]'; { coproc job { echo started >unexpected-job; }; } 2>/dev/null; echo \"$?\"; wait; test ! -e unexpected-job", b"1\n"),
            ('set --; x=; f(){ echo "$#"; }; f "$x$@"; f "$x""$@"; f "$@"', b"1\n1\n0\n"),
            ("HISTIGNORE='*reject'; history -c; history -s ': source'; history -s ': keep'; fc -s source=reject 1 2>/dev/null; fc -ln 1 -1", b"\t : source\n\t : keep\n"),
            ("function if { printf ok; }; text=$(declare -f if); unset -f if; eval \"$text\"; command_name=if; \"$command_name\"", b"ok"),
        ]:
            assert run(shells[1], code)[0] == (0, output, False), code
        failed = []
        for name, code in cases.items():
            expected = run(shells[0], code)[0]
            actual = run(shells[1], code)[0]
            if (name == "local-set-options" and version.startswith("5.2.21(")
                    and expected == (0, b"glob\n10\nlocal -\nleaked\nunset\n", False)):
                # This Bash patch level overwrites the first snapshot on a second
                # local -. GNU 5.3 CHANGES ccc documents the correction.
                expected = (0, b"glob\n10\nlocal -\nunset\n", False)
            if name == "tilde-associative-key" and version.startswith("5.2.21(") and expected == (0, b"/literal", False):
                # Debian 5.2.37 and both 5.3 controls expand only the unquoted tilde.
                expected = (0, b"absolute/literal", False)
            if expected != actual:
                failed.append((name, repr(expected), repr(actual)))
        print(json.dumps(dict(reference=version, witnesses=len(cases), differences=failed)), flush=True)
        if failed:
            raise SystemExit(1)
        # Formatting a command string must never execute it or source BASH_ENV.
        print_env = root / "print-env"
        print_env.write_text("touch pretty-side-effect\n")
        formatted = subprocess.run([shells[1], "--pretty-print", "-c", "if :;then touch pretty-side-effect;fi"], cwd=root, env=env | {"BASH_ENV":str(print_env)}, capture_output=True)
        assert formatted.returncode == 0 and b"touch pretty-side-effect" in formatted.stdout and not (root/"pretty-side-effect").exists()
        template = "v='a; b'\ncat <<'end'\n if data; then # literal\nend\nprintf '%s' \"$v\"\n"
        formatted = subprocess.run([shells[1], "--pretty-print"], input=template.encode(), env=env, cwd=root, capture_output=True)
        assert formatted.returncode == 0 and formatted.stdout == template.encode()
        invalid = subprocess.run([shells[1], "--pretty-print", "-c", "if :; then"], env=env, cwd=root, capture_output=True)
        assert invalid.returncode != 0 and invalid.stderr and not invalid.stdout
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
        (termroot / "sample").mkdir()
        (termroot / "sample/unique").touch()
        for name in EXTRA_CASES:
            if not requested(name): continue
            if name == "foreground-signal-notice" and not modern:
                # The POSIX foreground-notification ordering rule is new in 5.3.
                from posix_rules import terminal_rule
                assert terminal_rule(shells[1], termroot, 26), (name, shells[1])
                continue
            samples = [[] for _ in shells]
            for r in range(args.runs + 1):
                order = list(range(len(shells)))
                rng.shuffle(order)
                for i in order:
                    if name == "pretty-print-file":
                        script = termroot / "layout.sh"
                        marker = termroot / "formatted-side-effect"
                        script.write_text(("if true;then printf 'hello;%s\\n' 雪;fi\n" * 250)
                                          + "printf done >formatted-side-effect\n")
                        marker.unlink(missing_ok=True)
                        start = time.perf_counter_ns()
                        formatted = subprocess.run([shells[i], "--pretty-print", script], cwd=termroot, capture_output=True, timeout=4, env=env)
                        elapsed = (time.perf_counter_ns() - start) / 1e6
                        assert formatted.returncode == 0 and not marker.exists() and b"\n    printf" in formatted.stdout, (shells[i], formatted.stderr)
                        if r == 0:
                            rerun = subprocess.run([shells[0]], input=formatted.stdout, cwd=termroot, env=env, capture_output=True, timeout=4)
                            assert rerun.stdout == "hello;雪\n".encode() * 250 and marker.read_text() == "done", rerun
                    else:
                        from posix_rules import terminal_rule
                        start = time.perf_counter_ns()
                        assert terminal_rule(shells[i], termroot, 26), (name, shells[i])
                        elapsed = (time.perf_counter_ns() - start) / 1e6
                    if r: samples[i].append(round(elapsed, 3))
            if args.runs:
                print(json.dumps(dict(case=name, repeats=250 if name == "pretty-print-file" else 1, median_ms=[round(statistics.median(g),3) for g in samples], samples_ms=samples)), flush=True)
        for name in (*DIRECTORY_PTY, *EDITOR_PTY):
            if not requested(name): continue
            samples = [[] for _ in shells]
            for r in range(args.runs + 1):
                order = list(range(len(shells)))
                rng.shuffle(order)
                for i in order:
                    elapsed = (external_editor_pty if name in EDITOR_PTY else directory_pty)(shells[i], termroot, name)
                    if r: samples[i].append(round(elapsed, 3))
            if args.runs:
                print(json.dumps(dict(case=name, repeats=1, median_ms=[round(statistics.median(g), 3) for g in samples], samples_ms=samples)), flush=True)
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
                "stopped-job-selection": "set -m; sleep 15 & a=$!; sleep 15 & b=$!; trap 'kill -CONT \"$a\" \"$b\" 2>/dev/null; kill \"$a\" \"$b\" 2>/dev/null; wait 2>/dev/null' EXIT; kill -STOP \"$a\"; wait \"$a\" 2>/dev/null",
                "source-trap-frame": "printf ': one\\n: two\\n' > source-test; set -T; trap ': \"$LINENO\" \"$1\"' RETURN; f(){ source ./source-test inner; }",
                "substitution-source-lines": "f(){\n :\n echo \"$(echo $LINENO)\" `echo $LINENO`;\n}",
                "current-arithmetic-alias": "shopt -s expand_aliases; alias number='printf 23'",
                "scalar-array-declaration": "declare -lr var; declare -l setvar=''",
                "temporary-posix-settings": "shopt -s expand_aliases",
                "caller-input": "f(){ caller bad 2>/dev/null; :; }",
                "debug-function-entry-return": "set -T; f(){ :; :; }; trap ': \"$FUNCNAME:$LINENO\"' DEBUG RETURN",
                "long-working-directory": "set -o posix; cd deep-root; p=deep-part-abcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcd; for ((j=0;j<20;j++)); do cd \"$p\" || exit 9; done",
                "ascii-range-option": "LC_ALL=C; shopt -u globasciiranges",
                "gnu-diagnostic-location": "shopt -s gnu_errfmt",
                "xtrace-for-case-header": "exec 9>/dev/null; BASH_XTRACEFD=9; set -x",
                "global-nameref-shadow": "declare -n ref=foo; f(){ local -n ref=var; declare -g ref=(X); }",
                "parenthesized-scalar-suffix": "declare -i x",
                "posix-time-utility": "set -o posix; alias time='printf \"%s\"'",
                "history-ansi-quote": "set -o history",
                "printf-wide-characters": "case $OSTYPE in darwin*) LC_ALL=en_US.UTF-8;; *) LC_ALL=C.UTF-8;; esac",
                "posix-resume-job": "set -o posix; set -m",
                "posix-job-state": "set -o posix",
                "debug-return-scope": "set -T; helper(){ :; }; f(){ :; }; trap ':' RETURN; trap 'helper' DEBUG",
                "debug-handler-function-line": "set -T; helper(){ local x=$LINENO; : \"$1:$x:$BASH_COMMAND\"; }",
                "tilde-associative-key": "declare -A a",
                "ansi-associative-key": "declare -A a",
                "quoted-numeric-subscript": "a=(zero); k=1",
                "posix-prompt-expansion": "set -o posix; shopt -u promptvars; v=visible; p='!!/$v'",
                "posix-ulimit-blocks": "set -o posix",
                "posix-trap-display": "set -o posix; trap ':' USR1",
                "posix-signal-operands": "set -o posix",
                "posix-fc-operands": "set -o posix; set -o history; history -s 'echo first'",
                "posix-fc-format": "set -o posix; set -o history; history -s 'echo first'",
                "posix-logical-directory": "set -o posix",
                "alias-display-selected": "alias demo='echo hi'; set -o posix",
                "posix-consume-wait": "set -o posix",
                "posix-function-path": "function ./absent { :; }; set -o posix; exec 2>/dev/null",
                "posix-substitution-status": "set -o posix",
                "posix-special-in-function": "set -o posix; f(){ q=inner :; }",
                "trace-arithmetic-loop": "exec 2>/dev/null; set -x",
                "trace-arithmetic-expansion": "exec 2>/dev/null; v=2; set -x",
                "nameref-element-length": "a=(zero abc); declare -n r='a[k]'; k=1; declare -n all='a[@]'",
                "function-render-attributes": "f(){ :; }; export -f f",
                "conditional-trace-operator": "PS4='+ '; exec 2>/dev/null; set -x",
                "failed-function-trap-text": "trap ': \"$BASH_COMMAND\"' ERR; f(){ [[ -z nonempty ]]; }",
                "array-single-evaluation-option": "a=(one two)",
                "coproc-nameref": "declare -n r=arr; exec 2>/dev/null",
                "read-array-nameref": "declare -n r=arr",
                "mapfile-nameref": "declare -n r=arr",
                "lazy-malformed-operand": "a=4",
                "encoded-parameter-name": "x1=not; x=notOK",
                "array-named-descriptor": "declare -A m; key=out",
                "posix-command-precedence": "break(){ :; }; set -o posix",
                "invalid-ere": "re='a('; exec 2>/dev/null",
                "conditional-arithmetic-error": "exec 2>/dev/null",
                "replacement-backslash": "v=letssee; pattern=ss; r='x\\&y'",
                "quote-set-values": "set -o posix; ZZ1=\"'\"; ZZ2=\"'abc\"; ZZ3='a~b'; ZZ4='ab#cd'",
                "posix-input-capture": "printf literal >'glob-*'; set -o posix; p='glob-*'",
                "exit-trap-frame": "f(){ local item=inner; trap ': \"$FUNCNAME\" \"$item\" \"$1\"' EXIT; exit; }",
                "posix-background-parameter": "set -o posix; : & j=$!; wait \"$j\"",
                "quoted-operand-brace": "x=value",
                "eval-array-operand": "a=([3]=one [5]=two); name=a",
                "prefix-declaration": "x=outer; f(){ x=inner local x; : \"$x\"; }",
                "array-declaration-print": "e=(init)",
                "arithmetic-adjacent-signs": "v=0",
                "portable-bracket-members": "pat='[[.space.][.hyphen.][.tab.]]'; text=hello",
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
            if name == "slice-error-recovery":
                code = ("exec 2>/dev/null\nset -- one two\n"
                        + ': "${@:1:-1}"\n' * count + 'echo done')
            elif name == "indexed-init-recovery":
                code = ("exec 2>/dev/null\n" + (body + "\n") * count
                        + '[[ ${a[1]} == yes && ${#a[@]} == 2 ]] && echo done')
            elif name == "history-physical-lines":
                code = ("exec 2>/dev/null\nset -o history -H\n: seed\n"
                        + "if :; then\n: !!:$\nfi\n" * count + "echo done")
            elif name == "partial-assoc-init":
                code = ("exec 2>/dev/null\ndeclare -A a\n"
                        + "a=([one]=1 [two]=2 bad [extra]=3)\n" * count
                        + '[[ ${a[one]} == 1 && ${a[two]} == 2 && -z ${a[extra]+set} ]] && echo done')
            elif name == "history-ansi-quote":
                code = ("set -o history -H\n" + "v=$'hello\\' once!'\n" * count
                        + "test \"$v\" = \"hello' once\"'!' && echo done")
            elif name == "multiline-history":
                # Input history is recorded for top-level commands, even with
                # no terminal. Executing a sourced loop would bypass this path.
                code = "set -o history\n" + "for v in one two\ndo\n  : item\ndone\n" * count + "echo done"
            elif name in ("extended-operand-quotes", "encoded-parameter-name", "lazy-malformed-operand", "quoted-operand-brace", "descriptor-overflow", "eval-array-operand", "posix-background-parameter"):
                code = setup + "\n" + (body + "\n") * min(count, 2500) + "echo done"
                count = min(count, 2500)
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
