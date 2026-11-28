"""Compare complete script launches and sourcing the same files after shell startup."""

import argparse
from pathlib import Path
import random
import select
import sys
import subprocess
import tempfile
import time

from bench import report


def split_case(separator):
    row = separator.join(["x" * 128] * 32)
    source = (f"IFS='{separator}'; row='{row}'\n"
              'for ((i=0; i<400; i++)); do set -- $row; done\necho "$# ${#1} ${#32}"')
    return source, "32 128 128"


def eval_loop(body, check, expected, setup="", count=2000):
    return (f'{setup}\nfor ((i=0; i<{count}; i++)); do {body}; done\n{check}', expected)


# The last command prints a result that the driver checks on every run.
def assoc_names(count):
    # Bash visits its hash buckets in order and prepends each new key.
    def order(pair):
        index, name = pair
        bucket = 2166136261
        for byte in name.encode("ascii"):
            bucket = ((bucket * 16777619) ^ byte) & 1023
        return bucket, -index

    names = [f"package-component-dependency-{i}" for i in range(count)]
    return [name for _, name in sorted(enumerate(names), key=order)]


def redirected_compound(header, footer):
    return header + "\n" + "v=1\n" * 1000 + footer + '\necho "$v"', "1"


def function_source():
    return "".join(f"func{k}() {{\n  if :; then\n    v={k}\n  fi\n}}\n" for k in range(200)) + "func199"

def listing_loop(command, count):
    return (f'for ((i=0; i<{count}; ++i)); do {command} || exit 1; done >/dev/null\necho "$i"',
            str(count))


CASES = {
    **{f"witness-declare-readonly-{n}": listing_loop("declare -pr", 256)
       for n in (0, 256, 512, 1024)},
    **{f"witness-inherited-{cmd}-{n}": listing_loop("declare -p" if cmd == "declare" else "set", 40)
       for cmd in ("declare", "set") for n in (256, 512, 1024)},
    "witness-set-exported-options": listing_loop("set", 256),
    "witness-declare-pairs": eval_loop("m=(first '[kept]=word' third value)",
                                       'echo "${m[first]}"', "[kept]=word", "declare -A m", 1000),
    "witness-nameref-length": eval_loop('n=${#ref[@]}', 'echo "$n"', "1024",
                                        "a=({1..1024}); declare -n ref=a", 512),
    "flat-assoc-pairs": eval_loop('m=(key [literal]=$x)', 'echo "${m[key]}"',
                                  "[literal]=ab", "declare -A m; x=ab", 1000),
    "witness-element-append": eval_loop('a[0]+=$chunk', 'echo "${#a[0]}"', "524288",
                                        "a=(''); printf -v chunk '%0256d' 0", 2048),
    "witness-bulk-append": eval_loop('a=(); a+=("${src[@]}")', 'echo "${#a[@]} ${#a[0]}"',
                                     "1024 2048", "printf -v chunk '%02048d' 0; "
                                     "for ((j=0;j<1024;j++)); do src[j]=$chunk; done", 32),
    "witness-dir-rotate-one": eval_loop("pushd -n +1 || exit", 'echo "${#DIRSTACK[@]}"',
                                        "129", "for ((j=0;j<128;j++)); do pushd -n entry$j >/dev/null; done", 2400),
    "witness-dir-rotate-zero": eval_loop("pushd -n +0 || exit", 'echo "${#DIRSTACK[@]}"',
                                         "129", "for ((j=0;j<128;j++)); do pushd -n entry$j >/dev/null; done", 2400),
    **{f"r8-append-{kind}-{n}": eval_loop(
        'm+=([k$i]=$i)' if kind == "assoc" else 'm+=([$i]=$i)',
        'echo "${#m[@]} ${m[k$((i-1))]}"' if kind == "assoc" else 'echo "${#m[@]} ${m[i-1]}"',
        f"{n} {n-1}", "declare -A m" if kind == "assoc" else "m=()", n)
        for kind in ("assoc", "indexed") for n in (500, 2000, 5000)},
    **{f"r8-declare-append-{kind}": eval_loop(
        'declare -A m+=([k$i]=$i)' if kind == "assoc" else 'declare -a m+=([$i]=$i)',
        'echo "${#m[@]}"', "2000", "declare -A m" if kind == "assoc" else "m=()", 2000)
        for kind in ("assoc", "indexed")},
    "r8-physical-cd": eval_loop('cd -P inside && cd -P .. || exit',
        '[[ $PWD == "$orig" && $OLDPWD == "$orig/inside" ]] && echo yes', "yes",
        'mkdir -p inside; cd -P .; orig=$PWD', 500),
    **{f"r8-compgen-{kind}": eval_loop(f'COMPREPLY=( $(compgen {opts}) )', check, "yes", setup, count)
       for kind, opts, check, setup, count in [
           ("words", '-W "$words" -- abc', '[[ ${COMPREPLY[*]} == "abc abcd" ]] && echo yes', "words='abc abcd banana'", 50),
           ("variables", "-v -- tonari_variable", '[[ ${COMPREPLY[*]} == tonari_variable ]] && echo yes', "tonari_variable=1", 50),
           ("commands", "-c -- printf", '[[ " ${COMPREPLY[*]} " == *" printf "* ]] && echo yes', "", 20),
           ("files", "-f -- small.sh", '[[ ${COMPREPLY[*]} == small.sh ]] && echo yes', "touch small.sh", 20)]},
    **{f"r8-{builtin}-here-{n}": eval_loop(
        ('mapfile -t a' if builtin == "mapfile" else 'IFS= read -r v')+' <<EOF\n$s\nEOF\n:',
        'echo "${#a[0]}"' if builtin == "mapfile" else 'echo "${#v}"', str(n),
        f"printf -v s '%{n}s' ''", 200) for n in (256, 512, 4096, 16384, 65536)
        for builtin in ("mapfile","read")},
    **{f"r8-test-{label}": eval_loop(body, 'echo "$x"', "y", "printf -v s '%065536d' 0")
       for label,body in [("empty",'[[ -n $s ]] && x=y'), ("less",'[[ $s < a ]] && x=y'),
          ("hit",'[[ $s == 0* ]] && x=y'), ("miss",'[[ $s == z* ]] || x=y'),
          ("case",'case "$s" in 0*) x=y;; esac')]},
    **{f"r8-group-{label}": eval_loop(body, 'echo "$i"', "50", count=50)
       for label,body in [("status","( : )"), ("echo","( echo hi ) >/dev/null"),
           ("assign","( x=1; y=2 )"), ("cwd","( cd / && pwd ) >/dev/null")]},
    **{f"r8-quote-{label}": eval_loop('x=${s@'+kind+'}', check, result, setup, 200)
       for label,kind,setup,check,result in [
           ("spaces","Q","printf -v s '%65536s' ''",'echo "${#x}"',"65538"),
           ("ticks","Q","printf -v s '%32768s' ''; s=${s// /a}; s=${s//a/\"a'\"}",'echo "${#x}"',"163842"),
           ("escapes","E","printf -v s '%65536s' ''",'echo "${#x}"',"65536"),
           ("assignment","A","printf -v s '%65536s' ''",'echo "${#x}"',"65540")]},
    **{f"r8-case-{label}": eval_loop('x=${s'+op+'}', 'echo "${x:0:4} ${#x}"', out+" 2000",
        'export LC_ALL=C.UTF-8; s=; for ((i=0;i<500;i++)); do s+=' + word + '; done', 200)
       for label,op,word,out in [("upper","^^","éüαβ","ÉÜΑΒ"), ("lower",",,","ÉÜΑΒ","éüαβ"), ("toggle","~~","ÉüΑβ","éÜαΒ")]},
    "r8-test-regex": eval_loop('[[ $s =~ ^(0) ]] || exit', 'echo "${BASH_REMATCH[*]}"', "0 0", "printf -v s '%065536d' 0", 200),
    "r8-case-varied": eval_loop('x=${s^^}', 'echo "${#x}"', "2048",
        'export LC_ALL=C.UTF-8; s=' + ''.join(chr(0x4e00 + n) for n in range(2048)), 200),
    "r8-control-shift": ("set -- {1..5000}; while (($#)); do shift; done; echo $#", "0"),
    "r8-control-stat": eval_loop('[[ -f /bin/ls ]] && [ -f /bin/ls ] || exit', 'echo $i', "2000"),
    "r8-control-pipe": eval_loop('true | true', 'echo $i', "50", "shopt -s lastpipe", 50),
    "r8-control-glob": eval_loop('compgen -G "*.txt" >/dev/null || exit', 'echo $i', "50", "touch f{1..100}.txt", 50),
    **{f"r7-query-{label}": eval_loop(f'x=$({command})', check, expected, count=50)
       for label, command, check, expected in [
           ("pwd", "pwd", '[[ $x == "$PWD" ]] && echo yes', "yes"),
           ("physical", "pwd -P", '[[ -d $x && $x -ef . ]] && echo yes', "yes"),
           ("command", "command -v printf", 'echo "$x"', "printf"),
           ("type", "type -t ls", 'echo "$x"', "file")]},
    **{f"r7-helper-{label}": eval_loop('x=$(f "$i")', 'echo "$x"', expected, f'f() {{ {body}; }}', 50)
       for label, body, expected in [
           ("direct", 'local a=$1; echo "$a"', "49"),
           ("math", 'local n=$(($1 + 1)); echo "$n"', "50"),
           ("test", 'local a=$1; if [ "$a" = 49 ]; then echo yes; else echo no; fi', "yes"),
           ("glob", 'if [[ $1 == 4* ]]; then echo yes; else echo no; fi', "yes")]},
    "r7-source-case": ("".join(f"f{k}() {{\n case $1 in\n a) echo a;;\n b|c) echo bc;;\n esac\n}}\n" for k in range(400)) + "f399 c", "bc"),
    "r7-control-source": eval_loop(". ./small.sh", 'echo "$v"', "ok", count=200),
    "r7-control-files": ('cd brackets; for ((i=0;i<20;i++)); do compgen -f file_ >/dev/null || exit; done; echo "$i"', "20"),
    "r7-control-process": ("for ((i=0;i<50;i++)); do exec 3< <(echo value; exit 7); p=$!; "
                           'IFS= read -ru3 v; wait "$p"; rc=$?; exec 3<&-; done; echo "$v/$rc"', "value/7"),
    "r7-control-names": eval_loop('a=("${!pvar_@}")', 'echo "${#a[@]}"', "1000",
                                  'for ((i=0;i<1000;i++)); do printf -v "pvar_$i" %s x; done', 100),
    "r7-trim": eval_loop('x=${s##*tail}', 'echo "$x"', "ok", "printf -v s '%65536s' ''; s+=tailok", 500),
    "r7-replace": eval_loop('x=${s//0000/1111}', 'echo "${#x} ${x: -4}"', "65536 1111",
                            "printf -v s '%065536d' 0", 20),
    "r7-replace-miss": eval_loop('x=${s//ZZZZ/1111}', 'echo "${#x}"', "65536", "printf -v s '%65536s' ''", 500),
    "r7-slice": eval_loop('x=${s:60000:10}', 'echo "${#x}"', "10", "export LC_ALL=C.UTF-8; printf -v s '%65536s' ''; s=雪$s", 500),
    "r7-array-join": eval_loop('x="${a[*]}"', 'echo "${#x}"', "1889", "a=({0..499})", 500),
    "r7-array-sparse": eval_loop('x="${a[*]}"', 'echo "${#x}"', "1889", 'for ((i=0;i<500;i++)); do a[i*2]=$i; done', 500),
    "r7-array-assoc": eval_loop('x="${a[*]}"', 'echo "${#x}"', "1889", 'declare -A a; for ((i=0;i<500;i++)); do a[k$i]=$i; done', 500),
    "r7-array-keys": eval_loop('set -- "${!a[@]}"', 'echo "$# ${500}"', "500 499", "a=({0..499})", 500),
    "r7-case-fold": eval_loop('case $s in foo*[0-9]*bar) x=y;; *) exit 1;; esac', 'echo "$x"', "y",
                              'shopt -s nocasematch; s=Foo_x7x_Bar', 5000),
    "r7-test-fold": eval_loop('[[ $s == foo*[0-9]*bar ]] || exit 1', 'echo "$i"', "5000",
                              'shopt -s nocasematch; s=Foo_x7x_Bar', 5000),
    "r7-declare": ("v=abc; " + listing_loop("declare -p v", 5000)[0], "5000"),
    "r7-read": ('printf "%s\\n" {1..10000}:x:y:z >records; n=0; while IFS=: read -r a b c; do ((++n)); done <records; echo "$n"', "10000"),
    "r7-trap": eval_loop('x=$i', 'trap - DEBUG; echo "$x"', "1999", "trap ':' DEBUG", 2000),
    "r7-prompt": eval_loop('x=${p@P}', '[[ -n $x ]] && echo yes', "yes", r"p='\w\$ '", 5000),
    **{f"r7-printf-{label}": eval_loop(f'printf -v x "{fmt}" "$i"', 'echo "$x"', expected, count=10000)
       for label, fmt, expected in [("decimal", "%08d", "00009999"), ("hex", "0x%08x", "0x0000270f")]},
    "runtime-current-echo": eval_loop('x=${ echo "v_$i"; }', 'echo "$x"', "v_199", count=200),
    "runtime-brace-comma": eval_loop('set -- {a,b,c,d,e}_{1,2,3,4,5}_{x,y,z,w}', 'echo "$# ${100}"', "100 e_5_w", count=100),
    "runtime-brace-range": eval_loop('set -- {1..5}_{1..5}_{1..4}', 'echo "$# ${100}"', "100 5_5_4", count=100),
    "runtime-brace-nested": eval_loop('set -- pre_{a,b_{1,2,3},c_{x,y}}_suf', 'echo "$# $6"', "6 pre_c_y_suf", count=200),
    **{f"runtime-regex-{n}": eval_loop('p=${patterns[i%n]}; [[ test == test && test =~ $p ]] || exit 1',
            'echo "${BASH_REMATCH[0]}"', "test", f"n={n}; patterns=(); for ((j=0;j<n;++j)); do patterns+=(\"^(test|item$j)$\"); done", 4000)
       for n in (4, 5, 20)},
    "runtime-glob-prefix": eval_loop('a=(brackets/file_05*.txt)', 'echo "${#a[@]}"', "100", count=15),
    "runtime-glob-nocase": eval_loop('a=(brackets/FILE_05*.TXT)', 'echo "${#a[@]}"', "100", "shopt -s nocaseglob", 15),
    "runtime-glob-ext": eval_loop('a=(brackets/file_0@(5|6)*.txt)', 'echo "${#a[@]}"', "200", "shopt -s extglob", 15),
    "runtime-case-filter": eval_loop('x=${s^^[aeiou]}', 'echo "$x"', "hEllO_Utf8_AbcdE", "s=hello_utf8_abcde", 5000),
    "runtime-self-append": eval_loop('s="${s}x"', 'echo "${#s}"', "10000", "s=", 10000),
    "runtime-arithmetic-word": eval_loop('x=$((x+1))', 'echo "$x"', "10000", "x=0", 10000),
    "runtime-for-words": eval_loop('for w in a b c d e f g h; do v=$w; done', 'echo "$v"', "h"),
    "runtime-ref-read": eval_loop('x=$r', 'echo "$x"', "hello", "v=hello; declare -n r=v; x=", 10000),
    "runtime-ref-write": eval_loop('r=hello', 'echo "$v"', "hello", "v=; declare -n r=v", 10000),
    "runtime-unset-index": eval_loop('unset "a[$i]"', 'echo "${#a[@]}"', "0", "a=({0..4999})", 5000),
    "runtime-unset-key": eval_loop('m[k$i]=v; unset "m[k$i]"', 'echo "${#m[@]}"', "0", "declare -A m", 5000),
    "runtime-hash-path": eval_loop('hash -p /bin/ls ls', 'hash -t ls', "/bin/ls"),
    "runtime-echo-options": listing_loop('echo -ne "one\\ttwo" three', 5000),
    "runtime-file": eval_loop('x=$(<data)', 'echo "$x"', "contents", "echo contents >data", 500),
    **{f"runtime-ifs-{len(chars)}": eval_loop('set -- $row', 'echo "$# ${#32}"', "32 128",
        f"IFS='{chars}'; row='" + "".join("x"*128 + chars[j % len(chars)] for j in range(31)) + "x"*128 + "'", 400)
       for chars in (":,", ":,/", ":,/|;")},
    "runtime-ifs": eval_loop('set -- $v', 'echo "$# $4"', "4 d", "v=a:b,c/d; IFS=,:/", 5000),
    "runtime-eval": eval_loop('eval "${code[i%6]}"', 'echo "$x"', "1", "code=('x=0; :' 'x=1; :' 'x=2; :' 'x=3; :' 'x=4; :' 'x=5; :')", 5000),
    "review-helper-case": eval_loop('x=$(f "$i")', 'echo "$x"', "v49",
        'f() { local v="$1"; case "$v" in v*) echo "$v";; *) echo "v$v";; esac; }', 50),
    "review-helper-if": eval_loop('x=$(f "/$i")', 'echo "$x"', "49",
        '''f() { local p="$1"; if [ -n "$p" ]; then printf '%s\\n' "${p#/}"; else echo empty; fi; }''', 50),
    "review-repeated-words": eval_loop('b=("${a[@]//+(foo|bar)/X}")', 'echo "${#b[@]} ${b[499]}"',
        "500 preX499X", 'shopt -s extglob; for ((j=0;j<500;j++)); do a[j]=prefoobar${j}bar; done', 20),
    "review-overlapping-words": eval_loop('b="${s//+(f|fo|foo)/X}"', 'echo "$b"', "X-X",
        "shopt -s extglob; s=foofof-ffoo", 500),
    "review-array-extglob": eval_loop('b=("${a[@]//+([0-9])/X}")', 'echo "${#b[@]} ${b[499]}"',
        "500 preX-post", 'shopt -s extglob; for ((j=0;j<500;j++)); do a[j]=pre$j-post; done', 20),
    "review-array-trim": eval_loop('b=("${a[@]##+([0-9])}")', 'echo "${#b[@]} ${b[499]}"',
        "500 -post", 'shopt -s extglob; for ((j=0;j<500;j++)); do a[j]=$j-post; done', 20),
    "review-compgen": ('for ((i=0;i<20;i++)); do compgen -c l || exit; done >/dev/null\n'
                       'compgen -c ls | grep -x ls | head -1', "ls"),
    "review-command": listing_loop("command -v ls", 2000),
    "review-type": listing_loop("type -t ls", 2000),
    "review-static-glob": eval_loop('[[ $s == hello_*_123 ]] || exit 1', 'echo "$i"', "10000",
                                     "s=hello_X_123", 10000),
    "review-static-extglob": eval_loop('[[ $s == @(foo|baz)_+([0-9])_bar ]] || exit 1', 'echo "$i"',
                                     "5000", "s=foo_123_bar", 5000),
    "review-read-file": ('exec 3<short-records\nfor ((i=0;i<2000;i++)); do IFS= read -ru3 -n10 x || exit 1; done\necho "$x"', "0123456789"),
    "review-read": eval_loop('IFS= read -r x <<<"$s"', 'echo "$x"', "a b", "s='a b'"),
    "review-mapfile": eval_loop('mapfile -t a <<<"$s"', 'echo "${a[0]}"', "a b", "s='a b'"),
    "review-read-n": eval_loop('read -r -n 5 x <<<"$s"', 'echo "$x"', "abcde", "s=abcdefg"),
    "review-read-d": eval_loop('read -r -d : x <<<"$s"', 'echo "$x"', "abc", "s=abc:def"),
    "review-read-a": eval_loop('read -r -a a <<<"$s"', 'echo "${a[1]}"', "b", "s='a b'"),
    "review-alternate": eval_loop('x="${s:+yes}"', 'echo "$x"', "yes", "printf -v s '%65536s' x", 5000),
    "review-default": eval_loop('x="${s:-default}"', 'echo "$x"', "default", "unset s", 20000),
    "review-array-copy": eval_loop('b=("${a[@]}")', 'echo "${#b[@]} ${b[999]}"', "1000 999",
                                       "a=({0..999})", 500),
    "review-array-slice": eval_loop('b=("${a[@]:50:20}")', 'echo "${#b[@]} ${b[19]}"', "20 69",
                                       "a=({0..99})", 5000),
    "review-negative-index": eval_loop('x=${a[-1]}', 'echo "$x"', "99", "a=({0..99})", 10000),
    "review-length": eval_loop('x=${#s}', 'echo "$x"', "256", "printf -v s '%256s' x", 20000),
    "glob-ignore-simple-class": eval_loop('a=(brackets/*.txt)', 'echo "${#a[@]}"', "800",
                                         "GLOBIGNORE='brackets/file_0[56]*.txt'", 5),
    "conditional-simple-folded-class": eval_loop('[[ $s == +([aBbC]) ]] || exit 1',
        'echo "$i"', "10000", "shopt -s nocasematch; s=CbABcACBa", 10000),
    "review-length-ascii-wide": eval_loop('x=${#s}', 'echo "$x"', "65536",
                                        "printf -v s '%65536s' x", 250),
    "review-length-mixed-wide": eval_loop('x=${#s}', 'echo "$x"', "65537",
        "export LC_ALL=C.UTF-8; printf -v s '%65536s' x; s+=雪", 250),
    "review-pwd-physical": listing_loop("pwd -P", 5000),
    "review-pwd": ("for ((i=0;i<5000;i++)); do pwd || exit; done >/dev/null\n"
                   '[[ $(pwd) == "$PWD" ]] && echo ok', "ok"),

    "source-functions-repeat": eval_loop(". ./functions.sh", 'echo "$v"', "199", count=25),
    "source-literal-wrappers": ("".join(f"fn{k}() {{\n echo ok\n}}\n" for k in range(400))
                                + "fn399", "ok"),
    "source-functions-mixed": ("".join(
        f'fn{k}() {{\n local a=$1 b=${{2:-default{k}}}\n'
        ' if [[ $a == arg* ]]; then\n  REPLY="$a:$b"\n else\n  REPLY="$b:$a"\n fi\n}\n'
        for k in range(400)) + 'fn399 arg; echo "$REPLY"', "arg:default399"),
    "capture-position": eval_loop('v=$(echo "$1")', 'echo "$v"', "one two", "set -- 'one two'", 50),
    "capture-trim": eval_loop('v=$(echo "${p##*/}")', 'echo "$v"', "end", "p=/a/b/end", 50),
    "capture-default": eval_loop('v=$(echo "${p:-def}")', 'echo "$v"', "def", "unset p", 50),
    "capture-math": eval_loop('v=$(echo "$((i+1))")', 'echo "$v"', "50", count=50),
    "capture-helper": eval_loop('v=$(f "$i" "$@")', 'echo "$v"', "<49><a b><>",
                                """set -- 'a b' ''; f() { builtin printf '<%s>' "$@"; }""", 50),
    "capture-helper-lines": eval_loop('v=$(f "$i")', 'echo "$v"', "49",
                                     """f() { printf '%s\\n' "$1"; }""", 50),
    "capture-helper-stderr": eval_loop('v=$(f "$i" ok)', 'echo "$v"', "49 ok",
                                      """f() { command printf '%s\\n' "$*" 2>/dev/null; }""", 50),
    "null-builtin-out": eval_loop("echo hi >/dev/null || exit 1", 'echo "$i"', "2000"),
    "null-builtin-err": eval_loop(": 2>/dev/null || exit 1", 'echo "$i"', "2000"),
    "redirected-group": redirected_compound("{ :", ":; } 2>/dev/null"),
    "redirected-if": redirected_compound("if :; then", ":; fi 2>/dev/null"),
    "redirected-loop": redirected_compound("for k in pass; do", ":; done >out"),
    "redirected-case": redirected_compound("case x in x)", ": ;; esac 2>&1"),
    "redirected-group-control": redirected_compound("{ :", ":\n} 2>/dev/null"),
    "assoc-values": eval_loop('set -- "${m[@]}"', 'echo "$#"', "500", count=200),
    "assoc-keys": eval_loop('set -- "${!m[@]}"', 'echo "$#"', "500", count=200),
    "assoc-unquoted": eval_loop('set -- ${m[@]}', 'echo "$#"', "500", count=200),
    "assoc-churn": eval_loop('unset "m[key7]"; m[key7]=$i; set -- "${m[@]}"',
                             'echo "$# ${m[key7]}"', "500 199", count=200),
    "complete-list": listing_loop("complete -p", 100),
    "hash-list": listing_loop("hash", 200),
    "dirs-list": listing_loop("dirs -v", 200),
    "function-list": listing_loop("declare -f", 50),
    "export-function-list": listing_loop("export -fp", 50),
    "trap-list": listing_loop("trap -p", 200),
    "trap-list-named": listing_loop("trap -p HUP USR1 USR2", 200),
    "shopt-list": eval_loop("shopt -p >/dev/null || exit 1", "echo ok", "ok", count=200),
    "export-list": eval_loop("export -p >/dev/null || exit 1", 'echo "$V49"', "value49", count=200),
    "declare-indexed-list": eval_loop("declare -p a >/dev/null || exit 1", 'echo "${a[999]}"', "999", count=50),
    "brace-product": eval_loop("set -- p_{a..z}_{0..99}", 'echo "$# $1 ${2600}"', "2600 p_a_0 p_z_99", count=20),
    "local-ten": eval_loop("f 7 8", 'echo "${a-outer}"', "outer", count=1000),
    "local-eighty": eval_loop("f", 'echo "${x79-outer}"', "outer", count=150),
    "glob-bracket-list": eval_loop("set -- file_0[56]*.txt", 'echo "$# $1 ${200}"',
                                   "200 file_0500.txt file_0699.txt", count=50),
    "scalar-bracket-list": eval_loop("[[ $v == file_0[56]*.txt ]] || exit 1", "echo ok", "ok", count=4000),
    "type-list": eval_loop("type -t printf pwd >/dev/null || exit 1", "echo ok", "ok", count=400),
    "uncached-path-spawn": eval_loop("cat /dev/null || exit 1", "echo done", "done", "set +h", count=32),
    "current-shell-file-capture": eval_loop('value=${ printf "%s" "$i"; }', 'echo "$value"', "127", count=128),
    "history-long-event-word": (
        """printf -v pad '%65536s' x
history -s "cmd '$pad' 2>&31- last"
for ((i=0; i<300; i++)); do history -p '!cmd:$' >/dev/null; done
history -p '!cmd:$'""", "last",
    ),
    "eval-fixed": eval_loop("""eval '((sum += i))'""", 'echo "$sum"', "1999000"),
    "eval-changing": eval_loop('eval "n=$i"', 'echo "$n"', "1999"),
    "eval-arguments": eval_loop("""eval -- '((sum' += 'i))'""", 'echo "$sum"', "1999000"),
    "eval-alternating": eval_loop("""eval '((sum += i))'; eval 'last=$sum'""", 'echo "$last"', "1999000"),
    "eval-nested": eval_loop("""eval 'eval "$op"'""", 'echo "$sum"', "1999000", "op='((sum += i))'"),
    "eval-multiline": eval_loop('eval "$hook"', 'echo "$sum:$last"', "1999000:1999000",
                                "hook='((sum += i))\nlast=$sum'"),
    "eval-function": eval_loop('eval "$fn"; f', 'echo "$sum"', "1999000",
                               "fn='f() {\n  ((sum += i))\n}'"),
    "eval-conditionals": ("code='if true; then\n" + "  if true; then\n    : nested\n  fi\n" * 300
                          + "  ((calls++))\nfi'\nfor ((i=0; i<8; i++)); do eval \"$code\"; done\necho \"$calls\"", "8"),
    "eval-alias": eval_loop("""alias act='((sum+=i))'; eval act; alias act='((sum++))'; eval act""",
                            'echo "$sum"', "2001000", "shopt -s expand_aliases"),
    "cdable-vars": ("""\
mkdir -p dirs/actual
shopt -s cdable_vars || exit 1
CDPATH=dirs:unused:more:paths:to:search
printf -v actual '%32768s' x
dest=dirs/actual
for ((i=0; i<400; i++)); do
    cd actual >/dev/null && cd "$HOME" && cd dest >/dev/null && cd "$HOME" || exit 1
done
echo "$i"
""", "400"),
    "arithmetic": ("""\
sum=0
for ((i=1; i<=10000; i++)); do ((sum += i)); done
echo "$sum"
""", "50005000"),
    "functions": ("""\
add() { local n=$1; ((sum += n)); }
sum=0
for ((i=1; i<=4000; i++)); do add "$i"; done
echo "$sum"
""", "8002000"),
    "function-definitions": (
        "".join(f"f{i}() {{ echo {i}; }}\n" for i in range(2000)) + "f1999", "1999",
    ),
    "function-redefinitions": (
        "".join(f"f() {{ echo {i}; }}\n" for i in range(2000)) + "f", "1999",
    ),
    "function-wrapper-redefinitions": (
        "".join(f"""f() {{ printf '%s:%s\\n' '{i}' "$@"; }}\n""" for i in range(2000))
        + "f '雪 x'", "1999:雪 x",
    ),
    "function-redefine-and-call": (
        "".join(f"f() {{ echo {i}; }}\nf\n" for i in range(2000)), "\n".join(map(str, range(2000))),
    ),
    "function-literal-calls": (
        "f() { echo valid; }\nfor ((i=0; i<2000; i++)); do f; done", "\n".join(["valid"] * 2000),
    ),
    "function-compound-redefinitions": (
        "".join(f'f() {{\n  local n=$1\n  if ((n)); then echo {i}; else echo other; fi\n}}\n'
                for i in range(2000)) + "f 1", "1999",
    ),
    "function-loop-redefinitions": (
        'for ((i=0; i<2000; i++)); do\n'
        '  f() { local n=$1; if ((n)); then echo "$i:$n"; else echo other; fi; }\n'
        'done\nf 7', "2000:7",
    ),
    "strings": ("""\
for ((i=0; i<2000; i++)); do
    path="/var/log/worker-${i}.log"
    base=${path##*/}
    stem=${base%.log}
done
echo "$stem"
""", "worker-1999"),
    "regex": ("""\
re='^([^/]+)/([[:digit:]]+)[.]txt$'
for ((i=0; i<1000; i++)); do
    [[ dir/$i.txt =~ $re ]] || exit 1
done
echo "${BASH_REMATCH[1]}/${BASH_REMATCH[2]}"
""", "dir/999"),
    "printf-assign-number": (
        "for ((i=0;i<2048;i++)); do printf -v x '%+08d' -31; done; echo \"$x\"",
        "-0000031",
    ),
    "umask-symbolic-query": (
        "umask 077; for ((i=0;i<1024;i++)); do umask g+x; umask g-x; done; umask",
        "0077",
    ),
    "history-at-limit": (
        'HISTSIZE=8192; for ((i=0;i<16384;i++)); do history -s "row $i"; done; history 1',
        "16384  row 16383",
    ),
    "arithmetic-live-bound": (
        'limit=16000; sum=0; for ((i=0;i<limit;i+=2)); do ((sum+=i)); done; echo "$sum"',
        "63992000",
    ),
    "printf-formats": (
        "for ((i=0; i<1024; i++)); do printf '<%s:%02x:%.1f>\\n' item 31 1.5; done",
        "\n".join(["<item:1f:1.5>"] * 1024),
    ),
    "completion-registration": (
        'for ((i=0; i<512; i++)); do complete -o nospace -W "one two" app; complete -p app; done',
        "\n".join(["complete -o nospace -W 'one two' app"] * 512),
    ),
    "while-arithmetic": (
        'i=0; while ((i<2000)); do ((++i)); done; echo "$i"', "2000",
    ),
    "background-subshell-wait": (
        'for ((i=0; i<32; i++)); do (exit 13) & wait "$!"; s=$?; done; echo "$s"', "13",
    ),
    "case-edges": ("""\
for ((i=0; i<3000; i++)); do
    case src/unit-$i.rs in
        docs/*.md|tests/*.py) exit 1 ;;
        src/*.rs) last=$i ;;
        *) exit 1 ;;
    esac
done
echo "$last"
""", "2999"),
    "nocasematch": ("""
shopt -s nocasematch
for ((i=0; i<1500; i++)); do
    name=Src/Node-$i.RS
    case $name in src/*.rs) last=${name/#src/lib};; *) exit 1;; esac
    [[ $last == LIB/* ]] || exit 1
done
echo "$last"
""", "lib/Node-1499.RS"),
    "case-classes": ("""\
for ((i=0; i<3000; i++)); do
    case src/unit-$i.rs in
        src/unit-[0-9]*.rs) last=$i ;;
        *) exit 1 ;;
    esac
done
echo "$last"
""", "2999"),
    "glob": ("""\
for ((i=0; i<30; i++)); do set -- fixtures/*.txt; done
echo "$# $1"
""", "512 fixtures/item-000.txt"),
    "glob-sparse": ("""\
for ((i=0; i<30; i++)); do set -- fixtures/item-000*.txt; done
echo "$# $1"
""", "1 fixtures/item-000.txt"),
    "glob-all": ("""\
for ((i=0; i<30; i++)); do set -- fixtures/*; done
echo "$# $1"
""", "512 fixtures/item-000.txt"),
    "glob-multi-star": ("""\
for ((i=0; i<30; i++)); do set -- fixtures/*em*07.txt; done
echo "$# $1"
""", "6 fixtures/item-007.txt"),
    "glob-subdirs": ("""\
for ((i=0; i<30; i++)); do set -- mixed/*/*.rs; done
echo "$# $1"
""", "8 mixed/pkg-0/lib.rs"),
    "glob-utf8-sort": ("""\
for ((i=0; i<30; i++)); do set -- localized/*.txt; done
echo "$#"
""", "512"),
    "glob-utf8-once": ('set -- localized/*.txt; echo "$#"', "512"),
    "glob-extension-sparse": ('set -- extensions/entry-?????-?.log; echo "$#"', "1000"),
    "glob-extension-miss": ('shopt -s nullglob; set -- extensions/entry-?????-?.zip; echo "$#"', "0"),
    "glob-extension-dense": ('set -- extensions/entry-?????-?.txt; echo "$#"', "19000"),
    "glob-extension-star": ('set -- extensions/*.log; echo "$#"', "1000"),
    "glob-extension-wild": ('set -- extensions/entry-?????-?.*; echo "$#"', "20000"),
    "glob-utf8-large-repeat": ('for ((i=0; i<8; i++)); do set -- large/*.txt; done; echo "$#"', "4096"),
    "glob-utf8-mixed-40k": ('set -- unstructured/*.txt; echo "$#"', "40000"),
    "glob-utf8-100k": ('set -- numbered/*.txt; echo "$# $1 ${100000}"',
                      "100000 numbered/記録-000000.txt numbered/記録-099999.txt"),
    "mapfile-wide": ('mapfile -t records <wide; echo "${#records[@]} ${#records[4095]}"', "4096 4095"),
    "external-monitored": (
        'set -m; for ((i=0; i<16; i++)); do /usr/bin/true || exit 1; done; echo "$i"', "16",
    ),
    "glob-utf8-large": ('set -- large/*.txt; echo "$#"', "4096"),
    "glob-ignore-large": ('GLOBIGNORE="large/B-*.txt"; for ((i=0; i<8; i++)); do set -- large/*.txt; done; echo "$#"', "3584"),
    "glob-nocase-large": ('shopt -s nocaseglob; for ((i=0; i<8; i++)); do set -- large/*.TXT; done; echo "$#"', "4096"),
    "glob-recursive": ("""\
shopt -s globstar
for ((i=0; i<10; i++)); do set -- tree/**/*.rs; done
echo "$# $1"
""", "80 tree/d00/s0/lib.rs"),
    "read-lines": ("""\
count=0
while IFS= read -r row; do
    ((count++))
    last=$row
done < rows
echo "$count $last"
""", "2000 record 1999"),
    "substitutions": ("""\
for ((i=0; i<64; i++)); do value=$(printf 'v%s' "$i"); done
echo "$value"
""", "v63"),
    "echo-substitutions": ("""
for ((i=0; i<64; i++)); do value=$(echo "v$i"); done
echo "$value"
""", "v63"),
    "mapfile-exit-pipe": (
        'trap : EXIT; mapfile -t table < <(cat rows); echo "${#table[@]}:${table[1999]}"',
        "2000:record 1999",
    ),
    "mapfile-trapped-pipe": (
        'trap : USR1; mapfile -t table < <(cat rows); echo "${#table[@]}:${table[1999]}"',
        "2000:record 1999",
    ),
    "mapfile-lines": ("""
for ((i=0; i<16; i++)); do mapfile -t table <rows; done
echo "${#table[@]}:${table[1999]}"
""", "2000:record 1999"),
    "case-fallthrough": ("""
for ((i=0; i<1000; i++)); do
    case x in x) v=1 ;& y) v=2 ;;& y) v=bad ;; x) v+=3 ;; esac
done
echo "$v"
""", "23"),
    "extended-patterns": ("""
shopt -s extglob
for ((i=0; i<500; i++)); do case pkg-${i}.rs in pkg-+([0-9]).@(c|rs)) last=$i;; esac; done
echo "$last"
""", "499"),
    "resource-limit-list": (
        'for ((i=0; i<128; i++)); do ulimit -a || exit 1; done >/dev/null; echo "$i"', "128",
    ),
    "resource-limits": ("""
for ((i=0; i<256; i++)); do ulimit -Sn 64; ulimit -Sn > /dev/null; done
ulimit -Sn
""", "64"),
    "directory-stack": ("""
for ((i=0; i<200; i++)); do pushd -n dir >/dev/null; popd -n >/dev/null; done
echo "${#DIRSTACK[@]}"
""", "1"),
    "descriptor-moves": ("""
for ((i=0; i<128; i++)); do exec 7<>rows; exec 6<&7-; exec 6<&-; done
test -s rows && echo moved
""", "moved"),
    "associative-lookup": ("""
declare -A values=([left]=1 ["right hand"]=2)
for ((i=0; i<1200; i++)); do k="right hand"; values[left]=${values[$k]}; done
echo "${values[left]}:${#values[@]}"
""", "2:2"),
    "readonly-locals": ("""
check(){ local -r item="$1"; result=$item; }
for ((i=0; i<1000; i++)); do check "$i"; done
echo "$result"
""", "999"),
    "error-trap": ("""
n=0; trap '((n+=1))' ERR
for ((i=0; i<600; i++)); do false; done
echo "$n"
""", "600"),
    "case-conversion": ("""
v='two WORDS plus 42'; for ((i=0; i<1200; i++)); do upper=${v^^}; lower=${upper,,}; done
echo "$lower"
""", "two words plus 42"),
    "integer-assignments": ("""
declare -i n=1
for ((i=0; i<2000; i++)); do n+=3; done
echo "$n"
""", "6001"),
    "brace-range": ("""
for ((i=0; i<16; i++)); do set -- {0001..1000}; done
echo "$# $1 ${1000}"
""", "1000 0001 1000"),
    "getopts-clusters": ("""
for ((i=0; i<128; i++)); do
    OPTIND=1
    while getopts 'ab:c' opt -abvalue -c; do last=$opt; done
done
echo "$OPTIND:$last"
""", "3:c"),
    "compound-substitutions": ("""\
for ((i=0; i<16; i++)); do value=$(printf v; printf '%s' "$i"); done
echo "$value"
""", "v15"),
    "printf-list-newline": eval_loop(
        r"""value=$(printf '%s' abc; printf '%s\n' first second; printf x)""",
        'echo "$value"', "abcfirst\nsecond\nx", count=250,
    ),
    "heredoc-external-file": ("""\
printf -v text '%4095s' x
for ((i=0;i<120;i++)); do
    cat <<EOF > out
$text
EOF
done
[[ $(<out) == "$text" ]] && echo ok
""", "ok"),
    "here-string-capture": eval_loop('value=$(cat <<<"$text")',
                                     '[[ $value == "$text" ]] && echo ok', "ok",
                                     setup="printf -v text '%4095s' x", count=120),
    "mapfile-here": eval_loop('mapfile -t rows <<<"$text"', 'echo "${#rows[@]}:${rows[2]}"',
                              "3:third", "text=$'first\\nsecond\\nthird'", count=1200),
    "external-null-output": eval_loop('/usr/bin/true >/dev/null || exit 1', 'echo done', 'done', count=48),
    "conditional-chain": eval_loop('[[ ' + ' && '.join(['$value'] * 16) + ' ]] || exit 1',
                                   'echo done', 'done', 'value=yes', count=2000),
    "redirected-pwd-null": eval_loop('pwd >/dev/null || exit 1', 'echo done', 'done', count=1000),
    "redirected-pwd": eval_loop('pwd >working-dir', 'read -r line <working-dir; [[ $PWD == "$line" ]] && echo yes',
                                "yes", count=1000),
    "substitute-class": eval_loop('result=${text//[a-c]/X}', 'echo "$result"', "X9XX9X", "text=a9bc9a"),
    "trim-middle-star": eval_loop('result=${text##head*end}', 'echo "$result"', "last",
                                  "text=head_first_end_second_endlast"),
    "here-string-builtin-read": eval_loop('IFS= read -r value <<<"$text"',
                                          '[[ $value == "$text" ]] && echo ok', "ok",
                                          setup="printf -v text '%4095s' x", count=120),
    "capture-positional-program": eval_loop('value=$("$1" ok)', 'echo "$value"', "ok",
                                            setup="set -- /usr/bin/printf", count=96),
    "capture-scalar-program": eval_loop('value=$("$tool" ok)', 'echo "$value"', "ok",
                                        setup="tool=/usr/bin/printf", count=96),
    "printf-list-count": eval_loop(
        r"""value=$(printf 'xy%n' n; printf '%s' "$n")""",
        'echo "$value:$n"', "xy2:parent", setup="n=parent", count=100,
    ),
    "external-substitutions": ("""\
for ((i=0; i<16; i++)); do value=$(/usr/bin/printf 'v%s' "$i"); done
echo "$value"
""", "v15"),
    "parenthesized-pipeline": ("""\
for ((i=0; i<32; i++)); do
    printf '%s\\n' "$i" | ( read -r value && [[ $value == "$i" ]] ) || exit 1
done
echo "$i"
""", "32"),
    "pipeline": (
        "for ((i=0; i<16; i++)); do /usr/bin/printf '%s\\n' \"$i\" | /bin/cat || exit 1; done",
        "\n".join(map(str, range(16))),
    ),
    "merged-pipeline": (
        "for ((i=0; i<16; i++)); do /usr/bin/printf '%s\\n' \"$i\" |& /bin/cat || exit 1; done",
        "\n".join(map(str, range(16))),
    ),
    "redirect-stderr": (
        "for ((i=0; i<16; i++)); do /usr/bin/printf '%s\\n' \"$i\" 2>&1 || exit 1; done",
        "\n".join(map(str, range(16))),
    ),
    "merged-pipeline-redirect": (
        "for ((i=0; i<16; i++)); do /usr/bin/printf '%s\\n' \"$i\" |& /bin/cat 2>&1 || exit 1; done",
        "\n".join(map(str, range(16))),
    ),
    "builtin-merged-pipeline": (
        "for ((i=0; i<16; i++)); do printf '%s\\n' \"$i\" |& /bin/cat || exit 1; done",
        "\n".join(map(str, range(16))),
    ),
    "builtin-output": (
        "for ((i=0; i<16; i++)); do printf '%s\\n' \"$i\" || exit 1; done",
        "\n".join(map(str, range(16))),
    ),
    "builtin-output-long": (
        "for ((i=0; i<4096; i++)); do printf '%s\\n' \"$i\" || exit 1; done",
        "\n".join(map(str, range(4096))),
    ),
    "redirected-subshells": (
        "v=aB; for ((i=0; i<128; ++i)); do (printf %s \"${v^^}\") 2>/dev/null; done; echo",
        "AB" * 128,
    ),
    "field-split": split_case(":"),
    "field-split-unicode": split_case("😀"),
    "long-trim": (
        "path='dir/" + "x" * 2048 + "'\n"
        'for ((i=0; i<400; i++)); do base=${path##*/}; done\necho "${#base}"',
        "2048",
    ),
    "array-assoc-order": (
        'for ((i=0; i<24; i++)); do set -- "${!items[@]}"; done\n'
        'printf "%s\\n" "$@"',
        "\n".join(assoc_names(768)),
    ),
    "array-assoc-small": (
        'for ((i=0; i<240; i++)); do set -- "${!items[@]}"; done\n'
        'printf "%s\\n" "$@"',
        "\n".join(assoc_names(4)),
    ),
    "array-index-fields": (
        'for ((i=0; i<8; i++)); do set -- "${!items[@]}"; done\n'
        'printf "%s\\n" "$@"',
        "\n".join(str(i) for i in range(8192)),
    ),
    "array-slice-reference": (
        'for ((i=0; i<32; i++)); do set -- "${ref[@]:8000:3}"; done\n'
        'printf "%s\\n" "$#" "$@"',
        "3\n8000\n8001\n8002",
    ),
    "array-slice-dense": (
        'for ((i=0; i<32; i++)); do set -- "${items[@]:8000:3}"; done\n'
        'printf "%s\\n" "$#" "$@"',
        "3\n8000\n8001\n8002",
    ),
    "array-slice-sparse": (
        'for ((i=0; i<32; i++)); do set -- "${items[@]:16000:3}"; done\n'
        'printf "%s\\n" "$#" "$@"',
        "3\n8000\n8001\n8002",
    ),
    "substitute-single-scalar": (
        "text='" + "雪🌸x" * 1024 + "'\n"
        'for ((i=0; i<16; i++)); do value=${text//?/x}; done\necho "${#value}"',
        "3072",
    ),
    "substitute-fixed-class": (
        "text='" + "ab-12:" * 128 + "'\n"
        'for ((i=0; i<8; i++)); do value=${text//[a-z][a-z]-[0-9][0-9]/X}; done\necho "$value"',
        "X:" * 128,
    ),
    "trim-fixed-width": (
        "text='ab-12" + "Z" * 2048 + "cd-34'\n"
        'for ((i=0; i<32; i++)); do a=${text##??-??}; b=${a%%[a-z][a-z]-??}; done\necho "${#b}"',
        "2048",
    ),
    "substitute-fixed-width": (
        "text='" + "ab-12:" * 128 + "'\n"
        'for ((i=0; i<8; i++)); do value=${text//??-??/X}; done\necho "$value"',
        "X:" * 128,
    ),
    "external-wide-argv": (
        'set -- {1..4096} "" "日本語"\n'
        'for ((i=0; i<11; i++)); do /bin/echo "$@" >/dev/null || exit 1; done\n/bin/echo "$@"',
        " ".join([*(str(i) for i in range(1, 4097)), "", "日本語"]),
    ),
    "external-many-locals": (
        'for ((i=0; i<48; i++)); do /usr/bin/printenv BENCH_EXPORT || exit 1; done',
        "\n".join(["visible"] * 48),
    ),
    "external-env": (
        "export " + " ".join(f"BENCH_KEY_{i}=value{i}" for i in range(32)) + "\n"
        "for ((i=0; i<16; i++)); do /usr/bin/printenv BENCH_KEY_31 || exit 1; done\n",
        "\n".join(["value31"] * 16),
    ),
    "external": ("""\
for ((i=0; i<16; i++)); do /usr/bin/true || exit 1; done
echo "$i"
""", "16"),
}


# Populate the session before the ready handshake so setup cannot explain scaling.
for bindings in (0, 200, 1000):
    for label, body, check, expected, setup, count in (
        ("copy", "x=$y", 'echo "$x"', "blue", "x=; y=blue", 20000),
        ("sum", "((c+=i))", 'echo "$c"', "199990000", "c=0", 20000),
        ("locals", "f 7 8", 'echo "${a-outer}"', "outer",
         "f() { local a=$1 b=$2 c=3 d=4 e=5 h=1 i0=2 j=3 k=4 m=5; }", 1000),
    ):
        CASES[f"bindings-{bindings}-{label}"] = eval_loop(body, check, expected, setup, count)

for count in (0, 200, 1000):
    CASES[f"functions-{count}-call"] = eval_loop("myf", 'echo "$i"', "10000", "myf() { :; }", 10000)
    CASES[f"functions-{count}-printf"] = eval_loop('printf -v x %d "$i"', 'echo "$x"', "4999", count=5000)
    CASES[f"functions-{count}-colon"] = eval_loop(":", 'echo "$i"', "20000", count=20000)
    CASES[f"aliases-{count}-miss"] = eval_loop("eval ': 7'", 'echo "$i"', "1000", count=1000)

CASES["functions-1000-prefix"] = eval_loop("compgen -A function bench_fn_99 >/dev/null",
                                          "compgen -A function bench_fn_999", "bench_fn_999", count=300)
CASES["aliases-200-hit"] = eval_loop("eval selected", 'echo "$x"', "999", count=1000)
CASES["aliases-200-braced"] = CASES["aliases-200-hit"]
CASES["aliases-200-list"] = listing_loop("alias -p", 100)

CASES.update({
    "bindings-1000-prefix": eval_loop('compgen -A variable bench_var_99 >/dev/null',
                                     'compgen -A variable bench_var_999', "bench_var_999", count=500),
    "core-parse-functions": (function_source() + '; echo "$v"', "199"),
    "syntax-control": ("echo ok", "ok"),
    "core-copy-arithmetic": eval_loop('x=$((i+1))', 'echo "$x"', "20000", "x=0", 20000),
    "core-trim": eval_loop('x=${p##*/}', 'echo "$x"', "name", "p=dir/path/name", 20000),
    "core-default": eval_loop('x=${y:-fallback}', 'echo "$x"', "fallback", "unset y", 20000),
    "core-printf": eval_loop('printf -v x %s "$i"', 'echo "$x"', "19999", count=20000),
    "core-function": eval_loop('f "$i"', 'echo "$x"', "9999", 'f() { x=$1; }', 10000),
    "core-indexed": eval_loop('((a[i%128]+=1))', 'echo "${a[0]} ${a[127]}"',
                              "157 156", 'a=()', 20000),
    "core-arithmetic": eval_loop('((x += (i*3+7)^(i>>1)))', 'echo "$x"',
                                 str(sum((i*3+7) ^ (i>>1) for i in range(20000))), "x=0", 20000),
    "core-case": eval_loop('case "$s" in hello_*) ((k++));; esac', 'echo "$k"',
                           "20000", 's=hello_world; k=0', 20000),
    "core-test": eval_loop('[[ $s == hello_* ]] || exit 1', 'echo "$i"', "20000",
                           's=hello_world', 20000),
})


def sample(binary, script, expected, env, started, setup="", syntax=False, sourced=False):
    argv = [binary, "--noprofile", "--norc"]
    if syntax:
        argv.extend(["-n", script])
        expected = b""
    elif started:
        trailer = '; rc=$?; echo SOURCE_DONE; IFS= read -r gate; exit "$rc"' if sourced else ""
        argv.extend(["-c", setup + 'printf "ready\\n"; IFS= read -r gate && . "$1"' + trailer, "bench", script])
    elif setup:
        argv.extend(["-c", setup + '. "$1"', "bench", script])
    else:
        argv.append(script)
    start = time.perf_counter_ns()
    with subprocess.Popen(argv, env=env, cwd=script.parent, stdin=subprocess.PIPE,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0 if sourced else -1) as child:
        try:
            if started:
                if not select.select([child.stdout], [], [], 10)[0] or child.stdout.readline() != b"ready\n":
                    raise RuntimeError(f"{binary} did not reach the startup marker")
                start = time.perf_counter_ns()
            if sourced:
                child.stdin.write(b"\n")
                child.stdin.flush()
                observed = bytearray()
                while True:
                    if not select.select([child.stdout], [], [], 10)[0]:
                        raise RuntimeError(f"{binary} did not finish sourcing {script.name}")
                    line = child.stdout.readline()
                    if not line:
                        raise RuntimeError(f"{binary} exited before the source-return marker")
                    if line == b"SOURCE_DONE\n":
                        break
                    observed.extend(line)
                elapsed = (time.perf_counter_ns() - start) / 1e6
                tail, err = child.communicate(b"\n")
                out = observed + tail
            else:
                out, err = child.communicate(b"\n" if started else None)
                elapsed = (time.perf_counter_ns() - start) / 1e6
        finally:
            if child.poll() is None:
                child.kill()
    if (child.returncode, out, err) != (0, expected, b""):
        raise RuntimeError(f"{binary} {script.name}: status={child.returncode}, stdout={out!r}, stderr={err!r}")
    return elapsed


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("tonari", type=Path)
    cli.add_argument("--bash", type=Path, default=Path("/bin/bash"))
    cli.add_argument("--previous", type=Path, help="include a previous Tonari build")
    cli.add_argument("--spawn-control", type=Path, help="compiled tools/bench_spawn.c for native process and directory controls")
    cli.add_argument("--timing", choices=["whole", "started", "both", "syntax", "sourced"], default="both")
    cli.add_argument("--utf8-locale", default="en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8",
                     help="LC_ALL for Unicode field-splitting and pathname workloads")
    cli.add_argument("--runs", type=int, default=30)
    cli.add_argument("--case", choices=CASES, action="append")
    cli.add_argument("--state-mib", type=int, default=0,
                     help="fill an unexported shell variable before the ready marker")
    cli.add_argument("--require-target", action="store_true", help="exit nonzero if any Tonari median exceeds two thirds of Bash")
    args = cli.parse_args()
    within_target = True
    if args.state_mib < 0 or args.state_mib and args.spawn_control:
        cli.error("--state-mib requires a nonnegative size and a shell-only comparison")
    size = args.state_mib * 1024 * 1024
    setup = f"printf -v bench_resident '%*s' {size} x || exit 1; " if size else ""
    if args.runs < 1:
        cli.error("runs must be positive")
    shells = [("Tonari", args.tonari.resolve()), ("Bash", args.bash.resolve())]
    if args.previous:
        shells.append(("Previous", args.previous.resolve()))
    with tempfile.TemporaryDirectory() as temp:
        home = Path(temp)
        env = {"HOME": temp, "PATH": "/usr/bin:/bin", "LC_ALL": "C"}
        if not args.case or "r7-control-source" in args.case:
            (home / "small.sh").write_text("v=ok\n")
        if not args.case or "review-read-file" in args.case:
            (home / "short-records").write_bytes(b"0123456789" * 2000)
        (home / "rows").write_text("".join(f"record {i}\n" for i in range(2000)))
        if not args.case or "source-functions-repeat" in args.case:
            (home / "functions.sh").write_text(function_source())
        if not args.case or {"r7-control-files", "glob-bracket-list", "glob-ignore-simple-class", "runtime-glob-prefix", "runtime-glob-nocase", "runtime-glob-ext"}.intersection(args.case):
            directory = home / "brackets"
            directory.mkdir()
            for i in range(1000):
                (directory / f"file_{i:04}.txt").touch()
        if not args.case or any(case.startswith("glob-extension-") for case in args.case):
            directory = home / "extensions"
            directory.mkdir()
            for i in range(20000):
                mark = ("a", "雪", "🐟")[i % 3]
                extension = "log" if i % 20 == 0 else "txt"
                (directory / f"entry-{i:05}-{mark}.{extension}").touch()
        if not args.case or any(case in ("glob", "glob-all", "glob-sparse", "glob-multi-star") for case in args.case):
            fixtures = home / "fixtures"
            fixtures.mkdir()
            for i in range(512):
                (fixtures / f"item-{i:03}.txt").touch()
        if not args.case or "glob-utf8-mixed-40k" in args.case:
            unstructured = home / "unstructured"
            unstructured.mkdir()
            rng = random.Random(81043)
            alphabet = "AaBbZzÊéÉöäÅßØôn.-0129ΣσЖЯあキ漢字🚀"
            for i in range(40000):
                stem = "".join(rng.choices(alphabet, k=14))
                (unstructured / f"p{stem}-{i:05}.txt").touch()
        if not args.case or "glob-utf8-100k" in args.case:
            numbered = home / "numbered"
            numbered.mkdir()
            for i in range(100000):
                (numbered / f"記録-{i:06}.txt").touch()
        if not args.case or "mapfile-wide" in args.case:
            (home / "wide").write_bytes((b"x" * 4095 + b"\n") * 4096)
        if not args.case or "glob-subdirs" in args.case:
            mixed = home / "mixed"
            mixed.mkdir()
            for i in range(512):
                (mixed / f"item-{i:03}.txt").touch()
            for i in range(8):
                package = mixed / f"pkg-{i}"
                package.mkdir()
                (package / "lib.rs").touch()
        for directory, count, cases in [("localized", 512, ("glob-utf8-sort", "glob-utf8-once")),
                                        ("large", 4096, ("glob-utf8-large", "glob-utf8-large-repeat",
                                                         "glob-ignore-large", "glob-nocase-large"))]:
            if args.case and not any(case in args.case for case in cases):
                continue
            localized = home / directory
            localized.mkdir()
            prefixes = ["a", "B", "é", "雪", "🌱", "Z", "Å", "c"]
            for i in range(count):
                (localized / f"{prefixes[i % 8]}-{i:03}.txt").touch()
        if not args.case or "glob-recursive" in args.case:
            for i in range(16):
                for j in range(5):
                    leaf = home / "tree" / f"d{i:02}" / f"s{j}"
                    leaf.mkdir(parents=True)
                    (leaf / "lib.rs").touch()
                    for k in range(15):
                        (leaf / f"item-{k:02}.txt").touch()
        for case in args.case or CASES:
            case_env = env
            if case.startswith("glob-extension-") or case in (
                "substitute-single-scalar", "field-split-unicode", "glob-utf8-sort", "glob-utf8-once", "glob-utf8-large", "glob-utf8-large-repeat",
                "glob-ignore-large", "glob-nocase-large", "glob-utf8-100k", "glob-utf8-mixed-40k",
            ):
                case_env = dict(env, LC_ALL=args.utf8_locale)
            source, expected = CASES[case]
            script = home / f"{case}.sh"
            script.write_text(source)
            participants = shells
            if case in ("current-shell-file-capture", "runtime-current-echo") and subprocess.run(
                [args.bash.resolve(), "--noprofile", "--norc", "-c", 'x=${ :; }'],
                env=env, capture_output=True,
            ).returncode:
                print("Skipping the Bash timing for current-shell substitution (requires Bash 5.3).")
                participants = [(name, shell) for name, shell in shells if name != "Bash"]
            if case == "cdable-vars" and args.previous and subprocess.run(
                [args.previous.resolve(), "--noprofile", "--norc", "-c", "shopt -s cdable_vars"],
                env=env, capture_output=True,
            ).returncode:
                print("cdable-vars: previous shell lacks the option", flush=True)
                participants = [(name, shell) for name, shell in shells if name != "Previous"]
            if args.spawn_control and case in (
                "null-builtin-out", "r7-control-source", "r7-control-files",
                "external", "external-env", "redirect-stderr", "pipeline", "merged-pipeline",
                "merged-pipeline-redirect", "builtin-merged-pipeline",
                "background-subshell-wait", "descriptor-moves", "external-substitutions", "runtime-file",
                "glob-sparse", "glob-multi-star", "glob-utf8-once", "glob-utf8-large",
                "glob-ignore-simple-class", "runtime-glob-prefix", "runtime-glob-nocase", "runtime-glob-ext",
            ):
                participants = [*shells, ("Direct C", args.spawn_control.resolve())]
            case_setup = setup
            if case.startswith("bindings-"):
                count = int(case.split("-")[1])
                case_setup += "".join(f"bench_var_{j}={j}; " for j in range(count))
            if case.startswith("functions-"):
                count = int(case.split("-")[1])
                case_setup += "".join(f"bench_fn_{j}() {{ :; }}; " for j in range(count))
            if case.startswith("aliases-"):
                count = int(case.split("-")[1])
                case_setup += "shopt -s expand_aliases; " + "".join(f"alias bench_alias_{j}=:; " for j in range(count))
                if case.endswith("-hit"):
                    case_setup += """alias selected='x=$i'; """
                elif case.endswith("-braced"):
                    case_setup += """alias selected='x="${i}"'; """
            if case.startswith("assoc-"):
                case_setup += 'declare -A m; for ((j=0;j<500;++j)); do m[key$j]=value$j; done; '
            elif case == "complete-list":
                case_setup += 'for ((j=0;j<300;++j)); do complete -F "_comp_$j" "cmd_$j"; done; '
            elif case == "hash-list":
                case_setup += 'for ((j=0;j<50;++j)); do hash -p "/tmp/tool$j" "tool$j"; done; '
            elif case == "dirs-list":
                case_setup += 'for ((j=0;j<9;++j)); do pushd . >/dev/null; done; '
            elif case in ("function-list", "export-function-list"):
                case_setup += 'for ((j=0;j<100;++j)); do eval "fn$j() { :; }"; export -f "fn$j"; done; '
            elif case in ("trap-list", "trap-list-named"):
                case_setup += "trap : HUP USR1 USR2; "
            if case.startswith("witness-inherited-"):
                count = int(case.rsplit("-", 1)[1])
                case_env = dict(case_env, **{f"BENCH_{i:04d}": "value" for i in range(count)})
            if case.startswith("witness-declare-readonly-"):
                count = int(case.rsplit("-", 1)[1])
                case_setup += f'readonly bench_ro=ok; for ((j=0;j<{count};j++)); do alias bench_$j=true; done; '
            elif case == "witness-set-exported-options":
                case_setup += "export SHELLOPTS BASHOPTS; set -o pipefail; shopt -s nullglob; "
            if case == "export-list":
                case_setup += 'for ((j=0;j<50;++j)); do export "V$j=value$j"; done; '
            elif case == "declare-indexed-list":
                case_setup += "a=({0..999}); "
            elif case == "local-ten":
                case_setup += "f() { local a=$1 b=$2 c=3 d=4 e=5 h=1 i0=2 j=3 k=4 m=5; }; "
            elif case == "local-eighty":
                names = " ".join(f"x{j}={j}" for j in range(80))
                case_setup += f"f() {{ local {names}; }}; "
            elif case == "glob-bracket-list":
                case_setup += 'cd "$HOME/brackets" || exit 1; '
            elif case == "scalar-bracket-list":
                case_setup += "v=file_0533.txt; "
            if case == "external-many-locals":
                case_setup += ('for ((i=0; i<8192; i++)); do printf -v "bench_local_$i" %d "$i"; done; '
                               'export BENCH_EXPORT=visible; ')
            if case in ("array-assoc-order", "array-assoc-small"):
                count = 768 if case == "array-assoc-order" else 4
                case_setup += (f'declare -A items=(); for ((j=0; j<{count}; j++)); '
                               'do items[package-component-dependency-$j]=$j; done; ')
            elif case in ("array-slice-dense", "array-index-fields", "array-slice-reference"):
                case_setup += 'items=({0..8191}); '
                if case == "array-slice-reference":
                    case_setup += 'declare -n ref=items; '
            elif case == "array-slice-sparse":
                case_setup += 'for ((j=0; j<8192; j++)); do items[j*2]=$j; done; '
            modes = (False, True) if args.timing == "both" else (args.timing in ("started", "sourced"),)
            for started in modes:
                samples = {name: [] for name, _ in participants}
                for i in range(5 + args.runs):
                    offset = i % len(participants)
                    for name, binary in participants[offset:] + participants[:offset]:
                        elapsed = sample(binary, script, (expected + "\n").encode(), case_env, started, case_setup, args.timing == "syntax", args.timing == "sourced")
                        if i >= 5:
                            samples[name].append(elapsed)
                label = "source returned" if args.timing == "sourced" else "syntax check" if args.timing == "syntax" else "already started" if started else "whole script"
                print(f"{case} ({label})", flush=True)
                within_target = report(samples) and within_target

    if args.require_target and not within_target:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
