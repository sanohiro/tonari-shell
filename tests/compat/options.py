"""Separate probes for builtin flags; a successful parent command is not a flag test."""
import re

from cases import BUILTINS, LANGUAGE

def flags(synopsis):
    # Parse the short option groups from the reference builtin's own usage.
    # Metavariables such as -sigspec, -N for stack positions, and '+N' are operands.
    text = synopsis.partition(": ")[2].replace("-sigspec", "")
    if synopsis.startswith(("dirs:", "popd:", "pushd:")):
        text = text.replace("-N]", "]")
    return sorted(set("".join(re.findall(r"(?:^|[\s\[|])[-+]([A-Za-z@]+)(?=[\s\]|]|$)", text))))

def comp(tool, flag):
    spec = {"a":"-a", "b":"-b", "c":"-c", "d":"-d", "e":"-e", "f":"-f", "g":"-g", "j":"-j",
            "k":"-k", "s":"-s", "u":"-u", "v":"-v", "o":"-o nospace -W 'test'", "A":"-A keyword",
            "G":"-G 'a*'", "W":"-W 'apple apricot'", "F":"-F f", "C":"-C 'printf test'",
            "X":"-W 'apple apricot' -X '*cot'", "P":"-W apple -P pre", "S":"-W apple -S end"}
    if flag in spec:
        setup = """alias auditname=:; f(){ COMPREPLY=(apple); }; touch apple; mkdir apricot; """
        if tool == "complete":
            return setup + f"complete {spec[flag]} auditname; complete -p auditname"
        # Validate action results through a category-specific sentinel to avoid
        # comparing every executable, username, signal or environment variable.
        prefix = {"a":"auditname","b":"printf","c":"printf","d":"apr","e":"HOME","f":"app","g":"root",
                  "j":"NoSuchJob","k":"whil","s":"TER","u":"root","v":"HOME","A":"whil",
                  "o":"te","G":"a","W":"ap","F":"ap","C":"te","X":"ap","P":"ap","S":"ap"}[flag]
        return setup + f"compgen {spec[flag]} {prefix} 2>/dev/null | sort -u"
    if flag in ("D","E","I"):
        return f"complete -{flag} -W val; complete -p -{flag}"
    if flag == "r": return "complete -W hi app; complete -r app; complete -p app 2>/dev/null; echo $?"
    if flag == "p": return BUILTINS["complete"]
    if flag == "V": return "compgen -V v -W 'apple apricot' ap; printf '<%s>' \"${v[@]}\""

FLAG_SCRIPTS = {
    "alias": {"p": "alias abc='printf ok'; alias -p"},
    "unset": {"f": BUILTINS["unset"], "v": "v=x; unset -v v; echo \"${v-absent}\"",
              "n": "x=val; declare -n r=x; unset -n r; echo \"$x/${r-gone}\""},
    "unalias": {"a": "alias a=: b=:; unalias -a; alias -p"},
    "cd": {"P":"mkdir real; ln -s real link; cd -P link; basename \"$PWD\"",
           "L":"mkdir real; ln -s real link; cd -L link; basename \"$PWD\"",
           "e":"mkdir d; cd -Pe d && basename \"$PWD\""},
    "pwd": {k:f"mkdir d; ln -s d l; cd l; pwd -{k}" for k in "LP"},
    "command": {"p":"PATH=/nonexistent; command -p sh -c 'echo default'",
                "v":"command -v printf", "V":"command -V printf"},
    "echo": {k:rf"echo -{k} 'a\tb'" for k in "neE"},
    "exec": {"a":"exec -a aliasname /bin/sh -c 'echo \"$0\"'", "l":"exec -l /bin/sh -c 'echo \"$0\"'",
             "c":"export v=yes; exec -c /bin/sh -c 'echo \"${v-missing}\"'"},
    "export": {"f":LANGUAGE["function export"], "n":"export v=yes; export -n v; /bin/sh -c 'echo \"${v-unset}\"'",
               "p":"export v=marker; export -p | grep 'v=' | grep marker"},
    "hash": {"p":BUILTINS["hash"], "t":BUILTINS["hash"], "r":"hash -p /bin/echo audit; hash -r; hash -t audit 2>/dev/null; echo $?",
             "d":"hash -p /bin/echo audit; hash -d audit; hash -t audit 2>/dev/null; echo $?",
             "l":"hash -p /bin/echo audit; hash -l"},
    "jobs": {"p":BUILTINS["jobs"], "r":"sleep .04 & jobs -r | grep -q Running; echo $?; wait",
             "s":"sleep .04 & jobs -s; wait", "l":"sleep .04 & p=$!; jobs -l | grep -q \"$p\"; echo $?; wait",
             "x":"sleep .04 & p=$!; jobs -x /bin/echo %1 | grep -q \"$p\"; echo $?; wait"},
    "kill": {"l":"kill -l 143", "s":"trap 'printf yes' USR1; kill -s USR1 $$",
             "n":"trap 'printf yes' TERM; kill -n 15 $$"},
    "printf": {"v":BUILTINS["printf"]},
    "read": {"a":"read -a arr <<< 'a b'; printf '<%s>' \"${arr[@]}\"",
             "r":r"""read -r v <<< 'a\b'; printf '%s' "$v" """,
             "n":"read -n 2 x <<< abc; echo \"$x\"", "N":"read -N 3 x <<< $'a\\nb'; printf %s \"$x\"",
             "d":"read -d : v <<< one:two; echo \"$v\"", "u":"exec 3<<<abc; read -u 3 v; echo \"$v\"",
             "t":"read -t .02 v < <(sleep .08); (( $? > 128 )) && echo timeout"},
    "shopt": {"p":"shopt -p nullglob", "s":BUILTINS["shopt"], "u":BUILTINS["shopt"],
              "q":"shopt -s nullglob; shopt -q nullglob; echo $?", "o":"set -f; shopt -qo noglob; echo $?"},
    "trap": {"l":"trap -l | grep -q SIGTERM && echo listed",
             "p":"trap 'echo signal' USR1; trap -p USR1",
             "P":"trap 'echo signal' USR1; trap -P USR1"},
    "type": {"a":"f() { :; }; type -at f; type -at echo | sort -u",
             "p":"type -p /bin/sh", "P":"echo() { :; }; type -P echo | sed 's@.*/@@'",
             "f":"f() { :; }; type -ft f; echo $?", "t":"type -t printf"},
    "wait": {"n":LANGUAGE["wait -n"], "p":LANGUAGE["wait -p"], "f":"(exit 19) & wait -f $!; echo $?"},
    ".": {"p":"mkdir d; echo 'printf found' >d/file; . -p d file"},
    "source": {"p":"mkdir d; echo 'printf found' >d/file; source -p d file"},
}
FLAG_SCRIPTS['umask'] = {'p':'umask 027; umask -p', 'S':'umask u=rwx,g=rx,o=; umask -S'}
for name in ('mapfile', 'readarray'):
    def records(flags, data="first\\nsecond\\nthird", init=""):
        return init + f"printf '{data}' > input; exec 3<input; {name} {flags} a <&3; " + """printf '[%s]' "${a[@]}" """
    FLAG_SCRIPTS[name] = {'t':records('-t'), 'u':records('-t -u 3'), 'n':records('-tn 2'),
                          's':records('-ts 1'), 'O':records('-tO 1', init="a=(keep); "),
                          'd':records('-td :', "first:second:"),
                          'C':records('-t -C cb -c 1', init="cb() { echo \"$1:$2\"; }; "),
                          'c':records('-t -C cb -c 2', init="cb() { echo \"$1:$2\"; }; ")}
for name in ("declare", "typeset"):
    FLAG_SCRIPTS[name] = {
        "a":f'{name} -a x=(one two); echo "${{x[1]}}"',
        "A":f'{name} -A x; x[foo]=a; x[bar]=b; echo "${{x[foo]}}:${{x[bar]}}"',
        "f":f'f() {{ :; }}; {name} -f f | grep -q "f" && printf found',
        "F":f'f() {{ :; }}; {name} -F f',
        "g":f'f() {{ local x=L; {name} -g x=G; echo "$x"; }}; f; echo "$x"',
        "i":f'{name} -i x=3; x=4+7; echo "$x"',
        "I":f'x=outer; f() {{ {name} -I x; echo "$x"; }}; f',
        "l":f'{name} -l x=UPPER; echo "$x"', "u":f'{name} -u x=lower; echo "$x"',
        "n":f'x=one; {name} -n r=x; r=two; echo "$x"',
        "r":f'{name} -r x=one; (x=two) 2>/dev/null; echo "$?:$x"',
        "t":f'f() {{ :; }}; {name} -ft f; {name} -pF f',
        "x":f'{name} -x x=one; /bin/sh -c \'echo "$x"\'',
        "p":f'{name} x=word; {name} -p x'}
for name in ("complete", "compgen"):
    FLAG_SCRIPTS[name] = {f:comp(name, f) for f in "abcdefgjksuvoAGWFCXPSDEIrpV"}
FLAG_SCRIPTS["compopt"] = {"o":BUILTINS["compopt"], **{f:f"complete -{f} -W yes; compopt -{f} -o nospace; complete -p -{f}" for f in "DEI"}}
SET_FLAGS = frozenset("abefhkmnptuvxBCEHPT")
