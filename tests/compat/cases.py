"""Behavior probes for the named Bash interfaces. Each source runs in a new shell."""
# Print observable values as well as using the process exit status. A command
# that silently accepts syntax must not pass a test of its actual effect.
BUILTINS = {
    ".": "printf 'v=sourced' > script; . ./script; echo \"$v\"",
    ":": ": ignored > empty; test -f empty && test ! -s empty",
    "[": "[ 'two words' = 'two words' ] && [ 3 -gt 2 ] && ! [ -e missing ]",
    "alias": "shopt -s expand_aliases\nalias a='printf yes'\na",
    "bg": None, "fg": None,  # job control is exercised with a controlling terminal
    "bind": r"""bind -m emacs-standard -u beginning-of-line; bind '"\C-a":beginning-of-line'; bind -q beginning-of-line""",
    "break": "for i in a b; do echo \"$i\"; for j in x; do break 2; done; done",
    "builtin": "echo() { printf overridden; }; builtin echo original",
    "caller": "f() { caller 0; }; f",
    "cd": "mkdir child; cd child && printf '%s' \"${PWD##*/}\"; cd - >/dev/null && test -d child",
    "command": "printf() { echo wrong; }; command printf ok; command -v printf",
    "compgen": "compgen -W 'red rose blue' ro",
    "complete": "complete -W 'red rose' app; complete -p app",
    "compopt": "complete -W one app; compopt -o nospace app; complete -p app",
    "continue": "for i in 1 2 3; do [ \"$i\" = 2 ] && continue; echo \"$i\"; done",
    "declare": "declare -i v=3; v+=4; declare -p v",
    "dirs": "mkdir d; pushd d >/dev/null; dirs -p | wc -l",
    "disown": "sleep .05 & disown %+; jobs -p",
    "echo": r"echo -ne 'one\ntwo'",
    "enable": "enable -n echo; type -t echo; enable echo; type -t echo",
    "eval": "v='two words'; eval 'printf \"<%s>\" \"$v\"'",
    "exec": "exec /bin/sh -c 'printf replaced; exit 17'; echo wrong",
    "exit": "exit 259; echo wrong",
    "export": "v='two words'; export v; /bin/sh -c 'printf %s \"$v\"'",
    "false": "false",
    "fc": "set -o history; history -s unique; fc -ln -1",
    "getopts": "set -- -a arg; getopts a: opt; printf '%s:%s:%s' \"$opt\" \"$OPTARG\" \"$OPTIND\"",
    "hash": "hash -p /bin/echo custom; hash -t custom; custom visible",
    "help": "help -s printf",
    "history": "history -c; history -s unique; history 1 | sed 's/^ *[0-9]* *//'",
    "jobs": "sleep .1 & p=$!; test \"$(jobs -p)\" = \"$p\" && echo running; wait \"$p\"",
    "kill": "sleep 1 & p=$!; kill -TERM %1; wait \"$p\" 2>/dev/null; echo \"$?\"",
    "let": "x=3; let 'x*=4'; echo \"$x\"",
    "local": "v=parent; f() { local v=child; echo \"$v\"; }; f; echo \"$v\"",
    "logout": None,  # login-shell invocation below
    "mapfile": "mapfile -t a <<< $'one\\ntwo'; printf '<%s>' \"${a[@]}\"",
    "popd": "mkdir d; pushd d >/dev/null; popd >/dev/null; test -d d",
    "printf": r"""printf -v v '%s:%04d' word 7; printf '%s\0' "$v" """,
    "pushd": "mkdir d; pushd d >/dev/null; echo \"${PWD##*/}\"",
    "pwd": "mkdir d; ln -s d link; cd link; pwd -L; pwd -P",
    "read": "IFS=: read -r a b <<< 'x:y:z'; printf '<%s>' \"$a\" \"$b\"",
    "readarray": "readarray -t a <<< $'one\\ntwo'; printf '<%s>' \"${a[@]}\"",
    "readonly": "readonly x=old; (x=new) 2>/dev/null; printf '%s/%s' \"$?\" \"$x\"",
    "return": "f() { return 39; echo wrong; }; f",
    "set": "set -- one 'two words'; set -o pipefail; false | true; printf '%s/%s/%s' \"$?\" \"$#\" \"$2\"",
    "shift": "set -- one two three; shift 2; printf '%s:%s' \"$#\" \"$1\"",
    "shopt": "shopt -s nullglob; set -- missing*; echo \"$#\"; shopt -u nullglob; set -- missing*; echo \"$1\"",
    "source": "printf 'v=$1' > script; source ./script passed; echo \"$v\"",
    "suspend": None,  # use a PTY or a supervising process
    "test": "test 'two words' = 'two words' && test 3 -gt 2 && ! test -e missing",
    "times": "t=$(times); [[ $t == *m*s* ]] && echo timings",
    "trap": "trap 'printf \"EXIT%s\" \"$?\"' EXIT; trap 'echo usr' USR1; kill -USR1 $$; exit 7",
    "true": "true",
    "type": "f() { :; }; type -t f; type -t printf; type -t /bin/sh",
    "typeset": "typeset -a v=(zero one); printf '%s' \"${v[1]}\"",
    "ulimit": "n=$(ulimit -n); [[ $n == +([0-9]) || $n == unlimited ]] && printf limit",
    "umask": "umask 077; : > f; ls -ld f | cut -c1-10",
    "unalias": "shopt -s expand_aliases\nalias a='printf ok'\na; unalias a; type -t a || echo gone",
    "unset": "x=1; f() { :; }; unset x; unset -f f; echo \"${x-unset}\"; type -t f || :",
    "wait": "(exit 23) & p=$!; wait \"$p\"; echo \"$?\"",
}

# Feature names here also identify exact executable witnesses in the report.
LANGUAGE = {
    "escape character": r"printf '<%s>' one\ two \* \$",
    "single quotes": """printf %s '$HOME *'""",
    "double quotes": r'''v='ab cd'; printf '<%s>' "$v" "\$v"''',
    "ANSI-C quoting": r"""printf '%s' $'a\tb\n\x41'""",
    "locale translation syntax": """printf %s $"hello" """,
    "comments": "printf before # ignore\nprintf after",
    "line continuation": "printf %s ab\\\ncd",
    "simple commands and assignments": "v=outer; v=inner /bin/sh -c 'echo \"$v\"'; echo \"$v\"",
    "lists and short-circuit": "false && echo wrong; false || printf yes; true && printf ok",
    "pipeline": "printf abc | tr a-z A-Z | wc -c",
    "merged stderr pipeline": "/bin/sh -c 'printf ok >&2' |& cat",
    "pipeline stage statuses": "false | true; printf '<%s>' \"$?\" \"${PIPESTATUS[@]}\"",
    "background and wait": BUILTINS["wait"],
    "if then elif else fi": "if false; then echo no; elif true; then echo yes; else echo no; fi",
    "case in esac": "case xyz in a) echo no;; x*) echo yes;; esac",
    "case ;& fallthrough": "case x in x) printf a ;& y) printf b;; esac",
    "case ;;& retest": "case x in x) printf a ;;& y) printf b;; x) printf c;; esac",
    "for do done": "for v in one 'two words'; do printf '<%s>' \"$v\"; done",
    "arithmetic for": "for ((i=0;i<3;++i)); do echo \"$i\"; done",
    "while": "i=0; while ((i<2)); do echo $((++i)); done",
    "until": "i=0; until ((i==2)); do echo $((++i)); done",
    "deferred bad substitution": "if false; then echo ${(M)${(k)parameters[@]}:#git_*}; fi; printf ok",
    "select": "select x in a b; do echo \"$x:$REPLY\"; break; done <<< 2 2>/dev/null",
    "function and return": "function f { printf '%s' \"$1\"; return 19; }; f 'a b'",
    "brace group": "v=outside; { v=inside; }; echo \"$v\"",
    "subshell scope": "v=outside; (v=inside; echo \"$v\"); echo \"$v\"",
    "negation": "! false; echo \"$?\"; ! true; echo \"$?\"",
    "conditional command": "[[ snow == s* && 2 -lt 3 ]] && printf yes",
    "time keyword": "TIMEFORMAT=TIMED; { time :; } 2>timing; cat timing",
    "coproc": """coproc WORK { read -r x; printf '%s' "$x"; }; echo ok >&"${WORK[1]}"; cat <&"${WORK[0]}" """,
    "indexed arrays": """a=(zero 'two words'); a[5]=five; printf '<%s>' "${!a[@]}" "${a[@]}" """,
    "associative arrays": """declare -A a; a[foo]=one; a[bar]=two; printf '%s:%s' "${a[foo]}" "${a[bar]}" """,
    "nameref": "x=one; declare -n ref=x; ref=two; echo \"$x\"",
    "brace alternation": "printf '<%s>' pre{one,two}post",
    "brace sequence": "printf '<%s>' {03..09..3}",
    "tilde expansion": "test ~ = \"$HOME\" && printf home",
    "command substitution": """v=$(printf 'a\\n\\n'); printf '<%s>' "$v" """,
    "backquote substitution": "printf '<%s>' \"`printf hi`\"",
    "current environment substitution (5.3)": """v=old; x=${ v=new; printf result; }; printf '%s/%s' "$v" "$x" """,
    "REPLY substitution (5.3)": """REPLY=old; x=${| REPLY=new; }; printf '%s/%s' "$x" "$REPLY" """,
    "arithmetic expansion": "a=3; printf '%s' \"$((a*4+2))\"",
    "process substitution input": "cat <(printf input)",
    "process substitution output": "printf value > >(cat >out); wait; cat out",
    "IFS word splitting": """IFS=:; v='a::b'; set -- $v; printf '<%s>' "$#" "$@" """,
    "filename wildcards": "touch 'a b.x' one.x; printf '<%s>' ?.x *.x",
    "bracket expressions and classes": """touch 'x*.txt' 'x?.txt' 'x2.txt'; printf '<%s>' x["*"]*.txt x[[:digit:]].txt""",
    "extended globs": "shopt -s extglob\ncase b in @(a|b)) echo yes;; esac",
    "quote removal": r"""printf '<%s>' 'a'"b"\ c """,
    "function export": """f() { printf exported; }; export -f f; "$SHELL" -c f""",
    "dynamic function scope": "f() { local v=local; g; }; g() { echo \"$v\"; }; v=outer; f; echo \"$v\"",
    "function definition redirections": "f() { echo hi; } >result; test ! -e result || exit 9; f; cat result",
    "DEBUG trap": """trap 'echo debug' DEBUG; :; trap - DEBUG""",
    "ERR trap": """trap 'echo err:$?' ERR; false; echo continued""",
    "RETURN trap": """f() { trap 'echo returned' RETURN; :; }; f""",
    "EXIT trap": "trap 'printf %s \"$?\"' EXIT; exit 23",
    "signal trap": "trap 'echo got' TERM; kill -TERM $$; printf end",
    "wait -n": "(sleep .02; exit 13) & wait -n; printf %s \"$?\"",
    "wait -p": "(sleep .02; exit 13) & p=$!; wait -n -p ended; [ \"$ended\" = \"$p\" ] && echo yes",
    "exit status 126": "printf x > f; chmod 600 f; ./f 2>/dev/null",
    "exit status 127": "no_such_tonari_audit_command 2>/dev/null",
    "signal exit status": "/bin/sh -c 'kill -TERM $$' 2>/dev/null; printf '%s' \"$?\"",
    "shell script source": BUILTINS["source"],
    "PATH command search": "mkdir bin; printf '#!/bin/sh\\nprintf found' >bin/cmd; chmod +x bin/cmd; PATH=$PWD/bin cmd",
    "alias parse time": "shopt -s expand_aliases\nalias a='echo yes'\nf(){ a; }; unalias a; f",
    "programmable completion callback": """f() { COMPREPLY=(one two); }; compgen -F f -- o 2>/dev/null""",
    "POSIX mode": "set -o posix; shopt -qo posix; printf '%s' \"$?\"",
    "history event expansion": """set -H; history -s 'echo remembered'; history -p '!!'""",
    "directory stack rotation": "mkdir a b; pushd a >/dev/null; pushd ../b >/dev/null; pushd +1 >/dev/null; basename \"$PWD\"",
    "printf %q round trip": """v='two words*'; printf -v encoded %q "$v"; eval "set -- $encoded"; printf '<%s>' "$#" "$1" """,
}

REDIRS = {
    "<": "printf data >f; cat <f", ">": "printf abc >f; printf x >f; cat f",
    ">>": "printf a >f; printf b >>f; cat f",
    ">|": "set -C; printf a >f; printf b >|f; cat f",
    "&>": "/bin/sh -c 'printf a; printf b >&2' &>f; cat f",
    "&>>": "printf x >f; /bin/sh -c 'printf y >&2' &>>f; cat f",
    "n>&m": "{ printf a; printf b >&2; } 2>&1 >f; cat f",
    "n<&m": "printf input >f; exec 3<f; cat <&3",
    "n>&-": "{ printf bad >&3; } 3>&- 2>/dev/null; printf %s \"$?\"",
    "n<&-": "read -r x 0<&- 2>/dev/null; printf %s \"$?\"",
    "n>&m-": "exec 3>f; printf hi 4>&3- >&4; cat f",
    "n<&m-": "printf data >f; exec 3<f; cat 4<&3- <&4",
    "<>": "printf data >f; exec 3<>f; printf X >&3; cat f",
    "here-document": "x=value; cat <<END\n$x\nEND",
    "quoted here-document": "x=value; cat <<'END'\n$x\nEND",
    "<<-": "cat <<-END\n\tword\n\tEND",
    "<<<": "cat <<< 'two words'",
    "{varname} descriptor": "exec {fd}>f; printf data >&$fd; exec {fd}>&-; cat f",
    "/dev/fd/N": "printf data >f; exec 3<f; cat /dev/fd/3",
}

SHOPT = {
    "dotglob": "touch .hidden shown; shopt -s dotglob; printf '<%s>' *",
    "nullglob": BUILTINS["shopt"],
    "failglob": "shopt -s failglob; (echo missing-*) 2>/dev/null; echo \"$?\"",
    "globstar": "mkdir -p a/b; touch a/b/file; shopt -s globstar; printf '<%s>' **/file",
    "nocaseglob": "touch UPPER; shopt -s nocaseglob; printf '<%s>' upp*",
    "nocasematch": r"""shopt -s nocasematch
v=AbABC
case $v in aB*) printf '%s ' "${v//ab/[&]}";; esac
[[ $v =~ ^(ab) ]] && printf '%s ' "${BASH_REMATCH[1]}"
[[ a != [[:upper:]] ]] && [[ A == @(a|z) ]] && printf pass
shopt -u nocasematch
[[ $v =~ ^(ab) ]] || printf ' strict'""",
    "globskipdots": "shopt -s globskipdots; touch .x; printf '<%s>' .*",
    "globasciiranges": "shopt -s globasciiranges && [[ b == [a-z] ]] && ! [[ B == [a-z] ]]",
    "expand_aliases": LANGUAGE["alias parse time"],
    "extglob": LANGUAGE["extended globs"],
    "lastpipe": "set +m; shopt -s lastpipe; printf 'ok\\n' | read v; echo \"$v\"",
    "inherit_errexit": "shopt -s inherit_errexit; set -e; x=$(false; echo wrong); echo \"$x\"",
    "patsub_replacement": "shopt -s patsub_replacement; v=apply; echo \"${v/pp/[&]}\"",
    "varredir_close": "shopt -s varredir_close; : {fd}>out; printf x >&$fd 2>/dev/null",
    "sourcepath": "mkdir d; echo 'printf found' >d/file; PATH=$PWD/d:$PATH; shopt -s sourcepath; . file",
    "xpg_echo": r"shopt -s xpg_echo; echo 'x\ty'",
    "localvar_inherit": "v=parent; shopt -s localvar_inherit; f(){ local v; echo \"$v\"; }; f",
    "checkhash": "mkdir a b; for d in a b; do echo '#!/bin/sh' >$d/c; echo echo $d >>$d/c; chmod +x $d/c; done; PATH=$PWD/a:$PWD/b:$PATH; c; rm a/c; shopt -s checkhash; c",
    "extdebug": "shopt -s extdebug 2>/dev/null; f() { :; }; declare -F f",
}
SETOPT = {
    "errexit": "set -e; false; echo wrong",
    "nounset": "set -u; printf '%s' \"$absent\"", "noglob": "touch a.x; set -f; printf %s *.x",
    "pipefail": LANGUAGE["pipeline stage statuses"].replace("false |", "set -o pipefail; false |"),
    "allexport": "set -a; x=value; /bin/sh -c 'echo \"$x\"'",
    "braceexpand": "set +B; echo {a,b}; set -B; echo {a,b}",
    "noclobber": "set -C; echo kept >f; (echo wrong >f) 2>/dev/null; cat f",
    "xtrace": "PS4='TRACE '; set -x; : argument; set +x",
    "posix": LANGUAGE["POSIX mode"], "physical": "mkdir d; ln -s d l; set -P; cd l; basename \"$PWD\"",
    "functrace": "set -T; trap 'echo returning' RETURN; f(){ :; }; f",
    "errtrace": "set -E; trap 'echo err' ERR; f(){ false; }; f",
    "noexec": "set -n; printf unreachable", "onecmd": "set -t; printf one\nprintf two",
    "notify": "set -b; [[ $- == *b* ]] && echo enabled",
    "monitor": """mkfifo gate; set -m; reader() { IFS= read -r line <gate; }; reader & p=$!
g=$(python3 -c 'import os,sys; print(os.getpgid(int(sys.argv[1])))' "$p"); test "$g" = "$p" && echo group
echo go >gate; wait "$p" """,
    "interactive-comments": "set -o interactive-comments; shopt -q interactive_comments; echo \"$?\"",
    "privileged": "set -p; [[ $- == *p* ]] && echo privileged",
    "verbose": "set -v\nprintf visible\nset +v",
}

VARIABLES = {
    "BASH": 'test -x "$BASH" && "$BASH" --noprofile --norc -c "printf child"',
    "SHELL": 'test -x "$SHELL" && "$SHELL" -c "printf child"',
    "BASH_VERSINFO": '(( BASH_VERSINFO[0] >= 5 && BASH_VERSINFO[1] >= 2 )) && printf version',
    "BASH_VERSION": '[[ $BASH_VERSION == 5.* ]] && printf version',
    "BASHPID": 'p=$BASHPID; ( [[ $BASHPID != "$p" && $$ == "$p" ]] && printf changed )',
    "BASH_SUBSHELL": 'printf %s "$BASH_SUBSHELL"; (printf %s "$BASH_SUBSHELL")',
    "BASH_ALIASES": "alias a='echo x'; printf %s \"${BASH_ALIASES[a]}\"",
    "BASH_CMDS": 'hash -p /bin/echo chosen; echo "${BASH_CMDS[chosen]}"',
    "BASH_COMMAND": """trap 'printf "<%s>" "$BASH_COMMAND"' DEBUG; : marker""",
    "BASH_EXECUTION_STRING": '[[ $BASH_EXECUTION_STRING == *needle* ]] && printf found',
    "BASH_ENV": """echo 'v=loaded' > envfile; BASH_ENV=$PWD/envfile "$SHELL" -c 'echo "$v"'""",
    "BASH_ARGV0": "BASH_ARGV0=new-name; printf %s \"$0\"",
    "BASH_REMATCH": """[[ abc123 =~ ^([a-z]+)([0-9]+)$ ]]; printf '<%s>' "${BASH_REMATCH[@]}" """,
    "BASH_SOURCE": """printf '%s\\n' 'echo "${BASH_SOURCE[0]##*/}"' >inc; . ./inc""",
    "BASH_LINENO": 'f(){ echo "${BASH_LINENO[0]}"; }\nf',
    "BASH_TRAPSIG": """trap 'echo "$BASH_TRAPSIG"' USR1; kill -USR1 $$""",
    "BASHOPTS": 'shopt -s nullglob; [[ :$BASHOPTS: == *:nullglob:* ]] && printf present',
    "SHELLOPTS": 'set -o pipefail; [[ :$SHELLOPTS: == *:pipefail:* ]] && printf present',
    "BASH_ARGV": "shopt -s extdebug 2>/dev/null; f(){ printf '<%s>' \"${BASH_ARGV[@]}\"; }; f A B",
    "BASH_ARGC": "shopt -s extdebug 2>/dev/null; f(){ echo \"${BASH_ARGC[0]}\"; }; f A B",
    "BASH_MONOSECONDS": '[[ $BASH_MONOSECONDS =~ ^[0-9]+$ ]] && printf number',
    "CDPATH": 'mkdir -p root/nested; CDPATH=$PWD/root; cd nested >/dev/null; basename "$PWD"',
    "DIRSTACK": 'mkdir d; pushd d >/dev/null; printf "%s" "${#DIRSTACK[@]}"',
    "EPOCHSECONDS": '[[ $EPOCHSECONDS =~ ^[0-9]{10,}$ ]] && printf epoch',
    "EPOCHREALTIME": '[[ $EPOCHREALTIME =~ ^[0-9]+[.][0-9]+$ ]] && printf epoch',
    "EUID": '[[ $EUID == "$(id -u)" ]] && printf uid', "UID": '[[ $UID == "$(id -ru)" ]] && printf uid',
    "FUNCNAME": 'f(){ g; }; g(){ printf "<%s>" "${FUNCNAME[@]}"; }; f',
    "FUNCNEST": "FUNCNEST=2; f(){ g; }; g(){ h; }; h(){ echo reached; }; (f) 2>/dev/null; echo $?",
    "GLOBIGNORE": "touch one.x two.x; GLOBIGNORE=one.x; printf '<%s>' *.x",
    "GLOBSORT": "touch a b; GLOBSORT=-name; printf '<%s>' *",
    "GROUPS": '[[ " ${GROUPS[*]} " == *" $(id -g) "* ]] && printf group',
    "HOME": 'mkdir alternate; HOME=$PWD/alternate; cd; basename "$PWD"',
    "IFS": LANGUAGE["IFS word splitting"], "LINENO": 'echo "$LINENO"\necho "$LINENO"',
    "MAPFILE": "mapfile -t <<< text; echo \"${MAPFILE[0]}\"",
    "OLDPWD": 'p=$PWD; mkdir d; cd d; test "$OLDPWD" = "$p" && echo old',
    "OPTARG": BUILTINS["getopts"], "OPTIND": BUILTINS["getopts"], "OPTERR": "OPTERR=0; set -- -z; getopts a opt; echo \"$opt\"",
    "PATH": LANGUAGE["PATH command search"], "PIPESTATUS": LANGUAGE["pipeline stage statuses"],
    "POSIXLY_CORRECT": "POSIXLY_CORRECT=1; shopt -qo posix; echo $?",
    "PPID": 'test "$PPID" -gt 1 && echo parent',
    "PWD": 'p=$PWD; mkdir d; cd d; test "$PWD" = "$p/d" && echo changed',
    "RANDOM": "RANDOM=42; a=$RANDOM; RANDOM=42; test \"$a\" = \"$RANDOM\" && [[ $a =~ ^[0-9]+$ ]] && echo seeded",
    "SRANDOM": '[[ $SRANDOM =~ ^[0-9]+$ ]] && printf number',
    "REPLY": 'read -r <<< "two words"; printf "<%s>" "$REPLY"',
    "SECONDS": 'SECONDS=120; (( SECONDS >= 120 && SECONDS < 123 )) && printf elapsed',
    "SHLVL": """p=$SHLVL; export p; "$SHELL" -c '((SHLVL == p+1)) && printf level'; :""",
    "TIMEFORMAT": LANGUAGE["time keyword"],
    "TMPDIR": None, "LANG": None, "LC_ALL": None,
}


KEYWORDS = dict.fromkeys("if then elif else fi".split(), "if then elif else fi")
KEYWORDS.update(dict.fromkeys("case esac".split(), "case in esac"))
KEYWORDS.update(dict.fromkeys("for in do done".split(), "for do done"))
KEYWORDS.update(dict.fromkeys(["{", "}"], "brace group"))
KEYWORDS.update(dict.fromkeys(["[[", "]]"], "conditional command"))
KEYWORDS.update({v: v for v in ("while", "until", "coproc", "select")})
KEYWORDS.update({"function": "function and return", "time": "time keyword", "!": "negation"})

PARAMETERS = {
    "$0": 'printf %s "$0"', "$#": 'set -- a b; echo "$#"',
    "$@": 'set -- a "b c" ""; printf "<%s>" "$@"', "$*": 'IFS=:; set -- a b; echo "$*"',
    "$?": 'false; echo "$?"', "$-": 'set -f; [[ $- == *f* ]] && echo set',
    "$$": '[[ $$ =~ ^[0-9]+$ ]] && ( [[ $$ != "$BASHPID" ]] && echo parent )',
    "$!": '(exit 13) & p=$!; wait "$p"; echo "$?"',
    "$_": 'echo ignored last >/dev/null; echo "$_"',
    "$1 … ${10}": 'set -- a b c d e f g h i j; echo "$1:${10}"',
}
for expr in (
    "${v}", "${v-a}", "${v:-a}", "${v=a}", "${v:=a}", "${v+a}", "${v:+a}",
    "${v?a}", "${v:?a}", "${#v}", "${v:1}", "${v: -2:1}",
    "${v#a*}", "${v##a*}", "${v%*a}", "${v%%*a}", "${v/a/X}", "${v//a/X}",
    "${v/#a/X}", "${v/%a/X}", "${v^}", "${v^^}", "${v,}", "${v,,}",
    *("${v@" + op + "}" for op in "UuLQEPAKka"),
):
    # An isolated subshell around each expansion keeps a fatal '?' or ':?' case
    # from hiding the empty and nonempty cases in the same probe.
    test = f'''(printf '<%s>' "{expr}"; printf '[%s]' "$v") 2>/dev/null; printf ':%s/' "$?"'''
    PARAMETERS[expr] = "; ".join(f"{setup}; {test}" for setup in ("unset v", "v=", "v=abCAa"))
PARAMETERS.update({
    "${!name}": 'v=one; name=v; echo "${!name}"',
    "${!prefix*}": 'probe_a=x; probe_b=y; printf "%s" "${!probe_*}"',
    "${!prefix@}": 'probe_a=x; probe_b=y; printf "<%s>" "${!probe_@}"',
    "${!a[@]}": 'a[4]=x; a[9]=y; printf "<%s>" "${!a[@]}"',
    "${a[@]}": 'a=("a b" ""); printf "<%s>" "${a[@]}"',
    "${a[*]}": 'a=("a b" ""); IFS=:; printf "<%s>" "${a[*]}"',
    "${#a[@]}": 'a[4]=one; a[6]=two; echo "${#a[@]}"',
})

ARITHMETIC = {}
for op in ("+", "-", "*", "/", "%", "**", "<<", ">>", "<", ">", "<=", ">=", "==", "!=",
           "&", "^", "|", "&&", "||", ",", "=", "+=", "-=", "*=", "/=", "%=", "<<=", ">>=", "&=", "^=", "|="):
    ARITHMETIC[op] = f'a=6; printf "%s:%s" "$((a {op} 2))" "$a"'
for expr in ("++a", "--a", "a++", "a--", "~a", "!a", "+a", "-a", "a?3:4", "(a+1)*2", "16#ff", "0xff", "017"):
    ARITHMETIC[expr] = f'a=6; printf "%s:%s" "$(({expr}))" "$a"'

CONDITIONS = {}
for op in ("-a", "-b", "-c", "-d", "-e", "-f", "-g", "-h", "-k", "-L", "-p", "-r", "-s", "-S",
           "-t", "-u", "-w", "-x", "-G", "-N", "-O"):
    fixtures = {"-c": "/dev/null", "-d": ".", "-p": "fifo", "-L": "link", "-h": "link", "-t": "0",
                "-s": "file", "-k": "sticky", "-g": "sg", "-u": "su", "-x": "exe", "-S": "socket",
                "-b": "/dev/loop0"}
    value = fixtures.get(op, "file")
    setup = {"fifo": "mkfifo fifo", "link": "ln -s file link", "sticky": "mkdir sticky; chmod +t sticky",
             "sg": "touch sg; chmod g+s sg", "su": "touch su; chmod u+s su",
             "exe": "touch exe; chmod +x exe"}.get(value, ":")
    CONDITIONS[op] = f"printf data >file; {setup}; [[ {op} {value} ]]; printf %s \"$?\"; [[ {op} absent ]]; printf %s \"$?\""
CONDITIONS.update({
    "-o": "set -f; [[ -o noglob ]]; echo $?; set +f; [[ -o noglob ]]; echo $?",
    "-v": "v=; [[ -v v ]]; echo $?; unset v; [[ -v v ]]; echo $?",
    "-R": "v=1; declare -n r=v; [[ -R r ]]; echo $?; [[ -R v ]]; echo $?",
    "-z": "for x in '' x; do [[ -z $x ]]; echo $?; done", "-n": "for x in '' x; do [[ -n $x ]]; echo $?; done",
    "-ef": "touch f; ln f link; [[ f -ef link ]] && ! [[ f -ef missing ]]",
    "-nt": "touch -t 200001010100 a; touch -t 200101010100 b; [[ b -nt a ]] && ! [[ a -nt b ]]",
    "-ot": "touch -t 200001010100 a; touch -t 200101010100 b; [[ a -ot b ]] && ! [[ b -ot a ]]",
    "=~": VARIABLES["BASH_REMATCH"],
})
for op in ("=", "==", "!=", "<", ">", "-eq", "-ne", "-lt", "-le", "-gt", "-ge"):
    CONDITIONS[op] = f'for v in 1 2 3; do [[ $v {op} 2 ]]; printf "%s" "$?"; done'

INVOCATION = {
    "-c": (["-c", 'printf "%s/%s/%s" "$0" "$#" "$1"', "named", "arg"], ""),
    "-s": (["-s", "--", "one"], 'printf "%s:%s" "$#" "$1"'),
    "-i": (["--norc", "-i", "-c", '[[ $- == *i* ]] && echo interactive'], ""),
    "-l / --login": (["--noprofile", "--login", "-c", "shopt -q login_shell && printf login"], ""),
    "-n": (["-n", "-c", "printf wrong"], ""),
    "-r / --restricted": (["--noprofile", "--norc", "-r", "-c", "(cd /) 2>/dev/null; echo $?"], ""),
    "--posix": (["--posix", "-c", "shopt -qo posix && printf posix"], ""),
    "--noprofile": (["--login", "--noprofile", "-c", "echo \"${profile-unread}\""], ""),
    "--norc": (["--norc", "-c", "printf entry"], ""),
    "--noediting": (["--noediting", "-c", "printf entry"], ""),
    "--init-file / --rcfile": (["--rcfile", "rc", "-i", "-c", 'echo "$rc_loaded"'], ""),
    "-O / +O": (["-O", "nullglob", "-c", 'set -- missing*; echo "$#"'], ""),
    "-o / +o": (["-o", "pipefail", "-c", "false | true"], ""),
    "--": (["--", "script", "one"], ""),
    "--verbose / -v": (["--verbose", "-c", "printf verbose"], ""),
    "-D / --dump-strings": (["--dump-strings", "-c", 'echo $"translated"'], ""),
    "--dump-po-strings": (["--dump-po-strings", "-c", 'echo $"translated"'], ""),
    "--pretty-print": (["--pretty-print", "-c", "if true;then echo x;fi"], ""),
    "--debugger": (["--debugger", "-c", "shopt -q extdebug && echo debug"], ""),
    "-t": (["-t"], "echo first\necho second\n"),
    "login logout": (["--noprofile", "-l", "-c", "logout 23; echo wrong"], ""),
}

LANGUAGE.update({
    "group redirections": "{ printf first; printf second; } >file; cat file",
    "script fallback without shebang": "printf 'printf fallback; exit 17' >s; chmod +x s; ./s",
    "case patterns from variables": "p='a*'; case abc in $p) echo yes;; esac",
    "arithmetic command status": "((0)); printf %s \"$?\"; ((1)); printf %s \"$?\"",
    "shell environment isolation": "v=outer; (v=inner; cd /; printf %s \"$v\"); printf %s \"$v\"",
    "terminal signal ignore and reset": "trap '' TERM; kill -TERM $$; trap - TERM; printf kept",
    "priorities of lookup": "f(){ echo function; }; type -t f; type -t true; type -t /bin/sh",
    "function argument restoration": "set -- outer; f(){ set -- inner; echo \"$1\"; }; f arg; echo \"$1\"",
    "array slice and negative index": 'a=(zero one two); printf "<%s>" "${a[-1]}" "${a[@]:1:2}"',
    "array append": 'a=(one); a+=(two "three words"); printf "<%s>" "${a[@]}"',
    "history word designator": """history -s 'echo one two'; history -p '!!:$'""",
    "history modifiers": """history -s 'echo dir/file.txt'; history -p '!!:$:t:r'""",
    "history search": """history -s 'printf unique'; history -p '!?unique?'""",
    "directory cd -": 'old=$PWD; mkdir child; cd child; cd - >/dev/null; test "$PWD" = "$old"',
    "coprocess input/output descriptors": LANGUAGE["coproc"],
    "completion filters": "compgen -W 'red rose blue' -X '*se' -P pre -S suf ro",
    "primary prompt parameter expansion": r"""PS1='$HOME'; printf %s "${PS1@P}" """,
    "arithmetic base notation": "echo \"$((2#1010 + 16#ff))\"",
    "time -p": "{ time -p :; } 2>f; grep -Eq '^real +[0-9]' f && echo timed",
    "Bash startup via BASH_ENV": VARIABLES["BASH_ENV"],
    "quoted and unquoted here-documents": REDIRS["quoted here-document"],
    "escaped backslash filenames": r"""touch 'x\a' 'x*a'; printf '<%s>' x\** x\\*""",
    "POSIX special builtin failure": """set -o posix; (readonly v=x; v=new :; echo wrong) 2>/dev/null; echo "$?" """,
    "local variable inheritance": SHOPT["localvar_inherit"],
})
for op in ("?", "*", "+", "@", "!"):
    LANGUAGE["extglob " + op + "(...)"] = "shopt -s extglob\nfor v in '' a ab aa z; do case $v in " + op + "(a|b)) printf '<%s>' \"$v\";; esac; done"
VARIABLES.update({
    "PPID": 'test "$PPID" -gt 0 && echo parent',
    "BASH_XTRACEFD": "exec 3>trace; BASH_XTRACEFD=3; PS4=X; set -x; : traced; set +x; cat trace",
    "BASH_COMPAT": "BASH_COMPAT=4.3; shopt -q compat43; echo $?",
    "COPROC": LANGUAGE["coproc"],
    "COMPREPLY": LANGUAGE["programmable completion callback"],
    "EXECIGNORE": "mkdir a b; for d in a b; do printf '#!/bin/sh\\necho %s\\n' $d >$d/thing; chmod +x $d/thing; done; PATH=$PWD/a:$PWD/b:$PATH; EXECIGNORE=$PWD/a/*; thing",
    "HISTCONTROL": """HISTFILE=$PWD/h; HISTCONTROL=ignoredups; set -o history
echo unique >/dev/null
echo unique >/dev/null
history -w "$HISTFILE"; grep -c '^echo unique' "$HISTFILE" """,
    "HISTIGNORE": """HISTFILE=$PWD/h; HISTIGNORE='echo hi*'; set -o history
echo hidden >/dev/null
echo kept >/dev/null
history -w "$HISTFILE"; grep '^echo' "$HISTFILE" """,
    "HISTFILE": "HISTFILE=$PWD/custom; history -s witness; history -w; grep witness custom",
    "HISTFILESIZE": "HISTFILE=$PWD/h; HISTFILESIZE=2; history -s one; history -s two; history -s three; history -w; wc -l <h",
    "HISTSIZE": "HISTSIZE=2; history -c; history -s one; history -s two; history -s three; history | sed 's/^ *[0-9]* *//'",
    "HISTTIMEFORMAT": "HISTTIMEFORMAT=tag; history -s item; history 1 | grep -o tagitem",
    "histchars": """histchars='^:#'; history -s 'echo prev'; history -p '^^'""",
    "FCEDIT": """printf '#!/bin/sh\\nprintf edited >called' >editor; chmod +x editor; FCEDIT=$PWD/editor; history -s ':'; fc -1 </dev/null 2>/dev/null; cat called""",
    "HOSTFILE": "printf '127.0.0.1 unique.host\\n' >hosts; HOSTFILE=$PWD/hosts; compgen -A hostname unique",
    "PS3": "PS3='CHOOSE '; select x in a b; do echo $x; break; done <<< 2",
    "PS4": "PS4='TRACE '; set -x; : witness; set +x",
    "LC_CTYPE": """export LC_CTYPE=C.UTF-8; unset LC_ALL; "$SHELL" -c 'x=雪; echo "${#x}"'""",
    "LC_ALL": """LC_ALL=C.UTF-8 "$SHELL" -c 'x=雪; echo "${#x}"'""",
    "LANG": """unset LC_ALL; LANG=C.UTF-8 "$SHELL" -c 'x=雪; echo "${#x}"'""",
    "LC_NUMERIC": """unset LC_ALL; LC_NUMERIC=de_DE.UTF-8; printf '%.1f' 2""",
    "LC_COLLATE": """touch z A é; unset LC_ALL; export LC_COLLATE=en_US.UTF-8; "$SHELL" -c 'printf "<%s>" *'""",
    "LC_TIME": """unset LC_ALL; LC_TIME=de_DE.UTF-8 TZ=UTC printf '%(%B)T' 0""",
    "ENV": """echo 'echo loaded' >rc; ENV=$PWD/rc "$SHELL" --posix -ic ':' 2>/dev/null""",
})

LANGUAGE.update({
    "tilde current and previous directories": 'mkdir child; p=$PWD; cd child; test ~+ = "$PWD" && test ~- = "$p" && echo yes',
    "tilde named user": "printf '<%s>' ~root",
    "pathname expansion of leading dots": "touch .hidden visible; printf '<%s>' * .*",
    "arithmetic integer division and remainder": "echo \"$(( -7/3 )):$(( -7%3 ))\"",
    "arithmetic short circuit": "x=0; : $((0 && ++x)); : $((1 || ++x)); echo \"$x\"",
    "subshell exit isolation": "(exit 17); printf %s \"$?\"",
    "grouped pipeline status": "set -o pipefail; { printf x; false; } | cat; printf '<%s>' \"$?\" \"${PIPESTATUS[@]}\"",
    "restricted shell": "shopt -q restricted_shell; echo $?",
    "printf reusable quoting": LANGUAGE["printf %q round trip"],
    "escape sequences in printf %b": r"""printf '%b' '\101\x42\n'""",
    "printf timestamp": """TZ=UTC printf '%(%Y-%m-%d)T' 0""",
})
# Format behavior matters to callers independently of the printf builtin's presence.
PRINTF = dict((fmt, f"printf '<%{fmt}>' " + val) for fmt, val in {
    "s":"'a b'", "b":r"'a\tb'", "c":"abc", "d":"-21", "i":"0xff", "u":"21", "o":"21", "x":"255",
    "X":"255", "e":"1.5", "E":"1.5", "f":"1.5", "g":"1.5", "G":"1.5", "a":"1.5", "A":"1.5",
    "q":"'two words'", "Q":"'two words'", "%":""}.items())
PRINTF.update({"n":"""printf 'abc%n' n; echo ":$n" """, "(fmt)T":LANGUAGE["printf timestamp"],
               "width / precision":"printf '<%*.*s>' 6 2 abc", "reuse format":"printf '<%s>' a b c"})

INVOCATION.update({"--help":(["--help"], ""), "--version":(["--version"], ""),
                   "read script file":(["script", "argument"], ""), "stdin script":([], "printf input"),
                   "-a (export assignments)":(["-a", "-c", """v=ok; /bin/sh -c 'echo "$v"'"""], ""),
                   "-e":(["-e", "-c", "false; echo wrong"], ""),
                   "-f":(["-f", "-c", "touch a.x; printf %s *.x"], ""),
                   "-u":(["-u", "-c", 'echo "$unset_var"'], ""),
                   "-x":(["-x", "-c", ": traced"], "")})

VARIABLES.update({
    "CHILD_MAX": """CHILD_MAX=16; (exit 23) & first=$!; for ((i=0;i<96;i++)); do : & jobs >/dev/null; done
sleep .03; jobs >/dev/null; wait "$first" 2>/dev/null; echo "$?" """,
    "LC_MESSAGES": """unset LC_ALL; LC_MESSAGES=de_DE.UTF-8; export LC_MESSAGES; "$SHELL" -c audit_missing_command 2>&1""",
})

# Exercise individual entry flags even where the set builtin also exposes the option.
for flag in "bhkmpBCEHPT":
    INVOCATION["-" + flag] = (["-" + flag, "-c", f'case $- in *{flag}*) printf enabled;; *) printf disabled;; esac'], "")
INVOCATION.update({
    "-O": (["-O", "nullglob", "-c", 'set -- absent*; echo "$#"'], ""),
    "+O": (["+O", "nullglob", "-c", 'set -- absent*; echo "$1"'], ""),
    "-o": (["-o", "pipefail", "-c", "false | true"], ""),
    "+o": (["+o", "pipefail", "-c", "false | true"], ""),
    "-l": (["--noprofile", "-l", "-c", "shopt -q login_shell && echo login"], ""),
    "--restricted": (["--noprofile", "--restricted", "-c", "(cd /) 2>/dev/null; echo $?"], ""),
    "--init-file": (["--init-file", "rc", "-i", "-c", 'echo "$rc_loaded"'], ""),
    "-v": (["-v", "-c", "printf verbose"], ""),
    "-D": (["-D", "-c", 'echo $"translated"'], ""),
})
for combined, tested in [("-O / +O", "-O"), ("-o / +o", "-o"),
                         ("-l / --login", "--login"), ("-r / --restricted", "-r"),
                         ("--init-file / --rcfile", "--rcfile"), ("--verbose / -v", "--verbose"),
                         ("-D / --dump-strings", "--dump-strings")]:
    INVOCATION[tested] = INVOCATION.pop(combined)
