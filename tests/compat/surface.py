"""Expand the audit from GNU's runtime registries and the Texinfo manual."""
import re
from pathlib import Path

from cases import BUILTINS
from options import flags, FLAG_SCRIPTS, SET_FLAGS
from terminal import audit_prompts, dialogue

# These are cross-references, not extra successful tests. Every source @node
# is retained, including explanatory sections and release-build appendices.
TOPICS = {
    "Quoting": "Shell language", "Reserved Words": "Reserved word", "Shell Parameters": "Parameter",
    "Positional Parameters": "Parameter", "Special Parameters": "Parameter", "Shell Expansions": "Parameter",
    "Shell Parameter Expansion": "Parameter", "Pattern Matching": "Shell language", "Redirections": "Redirection",
    "Bourne Shell Builtins": "Builtin", "Bash Builtins": "Builtin", "Special Builtins": "Builtin",
    "The Set Builtin": "set -o", "The Shopt Builtin": "shopt", "Bourne Shell Variables": "Variable",
    "Bash Variables": "Variable", "Invoking Bash": "Invocation", "Shell Arithmetic": "Arithmetic",
    "Bash Conditional Expressions": "Conditional", "Controlling the Prompt": "Prompt escape",
    "Job Control Builtins": "Builtin", "Job Control Basics": "Terminal", "Job Control Variables": "Variable",
    "Bash POSIX Mode": "POSIX mode item", "Shell Compatibility Mode": "Compatibility level",
    "Bindable Readline Commands": "Readline command", "Programmable Completion": "Completion action",
    "Programmable Completion Builtins": "Builtin", "Readline Init File Syntax": "Readline variable",
    "Bash History Builtins": "Builtin", "Event Designators": "History expansion", "Word Designators": "History expansion",
    "Modifiers": "History expansion", "Bash Startup Files": "Invocation", "Signals": "Signal",
}


def audit_dialogues(a):
    import tempfile
    contexts = {
        "completion": ["COMP_LINE","COMP_POINT","COMP_CWORD","COMP_TYPE","COMP_KEY","COMP_WORDS"],
        "READLINE": ["READLINE_LINE","READLINE_POINT","READLINE_MARK","READLINE_ARGUMENT"],
        "COLUMNS": ["COLUMNS","LINES"],
        "tty predicate": ["-t"],
    }
    for key in ("PS0","PS1","PS2","PROMPT_COMMAND","PROMPT_DIRTRIM","EMACS","INSIDE_EMACS","IGNOREEOF","TMOUT",
                "FIGNORE","read -p","read -s","read -e","read -E","read -i","MAIL","MAILCHECK","MAILPATH"):
        contexts[key] = [key]
    for key, names in contexts.items():
        if key == "read -E" and "E" not in flags(a.rows["Builtin"]["read"]["synopsis"]):
            a.add("Builtin option", key, "not in reference", a.rows["Builtin"]["read"]["synopsis"])
            continue
        with tempfile.TemporaryDirectory(dir=a.root) as temp:
            pair = [dialogue(sh, Path(temp), key) for sh in a.shells]
        for idx, name in enumerate(names):
            left, right = pair
            if key in ("completion","READLINE","COLUMNS"):
                def part(v):
                    if v == b"no capture": return v
                    fields = v.split(b"," if key == "COLUMNS" else b"\0")
                    return b"\0".join(fields[idx:]) if name == "COMP_WORDS" else (fields[idx] if idx<len(fields) else b"no field")
                left,right = part(left),part(right)
            expected = {"read -p": b"asked", "read -s": b"secret", "read -e": b"Xtail",
                        "read -E": b"Xtail", "read -i": b"Xinitialtail", "FIGNORE": b"audit-a.txt",
                        "PROMPT_DIRTRIM": b"trimmed", "PS0": b"active", "PS2": b"MORE>", "TMOUT": b"expired",
                        "IGNOREEOF": b"continues", "tty predicate": b"ttyother"}
            if left in (b"no capture", b"no field", b"no initial prompt", b"no mail notification") or (key in expected and left != expected[key]):
                status = "control inconclusive"
            else:
                status = "match" if left == right else "different"
            group = "Builtin option" if name.startswith("read ") else "Variable"
            if key == "tty predicate": group = "Conditional"
            if group == "Variable" and name not in a.rows["Variable"]:
                continue
            a.add(group, name, status, "PTY dialogue / " + key)
            if status == "match": a.rows[group][name]["workload"] = dict(dialogue=key, expected=left.hex(), field=idx, item=name)
            if status != "match": a.rows[group][name]["samples"] = [repr(left),repr(right)]


def extra(a, base, source, sections):
    audit_prompts(a)
    # Bash ships a common help string across kernels. Its -a output lists
    # the resource switches actually implemented in this reference build.
    limits = set(re.findall(r"-([A-Za-z])\)", a.script(0, "ulimit -a").out.decode())) | set("aSH")
    for name, row in a.rows["Builtin"].items():
        for flag in flags(row["synopsis"]):
            feature = f"{name} -{flag}"
            test = FLAG_SCRIPTS.get(name, {}).get(flag)
            if name == "ulimit" and flag not in limits:
                a.add("Builtin option", feature, "not in reference", "ulimit -a")
            elif row["status"] == "missing interface":
                a.add("Builtin option", feature, "missing interface", "parent builtin missing: " + name)
            elif name == "set" and flag in SET_FLAGS:
                # Short and long forms can have separate parser paths.
                src = f'set -{flag}; printf "%s" "$-"'
                if flag not in "nt":
                    src = f'set -{flag}; case $- in *{flag}*) printf on;; esac; set +{flag}; case $- in *{flag}*) printf wrong;; *) printf off;; esac'
                a.compare("Builtin option", feature, src, interface=True)
            elif test:
                a.compare("Builtin option", feature, test)
            else:
                a.add("Builtin option", feature, "context unprobed", "reference usage: " + row["synopsis"])
    audit_dialogues(a)
    for name in ("set -o",):
        a.compare("Builtin option", name, BUILTINS["set"])
    a.compare("Builtin option", "jobs -n", "sleep .03 & jobs -n; wait")
    test = a.script(0, "cd -@ .")
    if test.code and b"invalid option" in test.err:
        a.add("Builtin option", "cd -@", "not in reference", "cd -@ .")
    else:
        a.compare("Builtin option", "cd -@", "cd -@ .; pwd")
    for line in a.names("kill -l"):
        for sig in re.findall(r"\bSIG([A-Z][A-Z0-9]*(?:[+\-]\d+)?)", line):
            if sig in ("KILL", "STOP"):
                a.compare("Signal", sig, f"trap : {sig}; trap -p {sig}", interface=True)
            else:
                a.compare("Signal", sig, f"trap 'printf caught' {sig} || exit 3; kill -s {sig} $$; printf end")
    actions = ("alias arrayvar binding builtin command directory disabled enabled export file function group "
               "helptopic hostname job keyword running service setopt shopt signal stopped user variable").split()
    words = dict(alias="audit", arrayvar="audit", builtin="printf", command="printf", directory="audit",
                 disabled="echo", enabled="printf", export="AUDIT", file="audit", function="audit",
                 group="root", helptopic="printf", keyword="whil", service="http", setopt="pipef",
                 shopt="nullg", signal="TER", user="root", variable="audit", binding="beginning-of-line")
    for key in actions:
        setup = "alias audit_alias=:; audit_arr=(one); audit_func(){ :; }; export AUDIT_VALUE=x; mkdir audit_dir; touch audit_file; "
        if key in ("job", "running", "stopped"):
            setup += "sleep .05 & "
        if key == "disabled":
            setup += "enable -n echo; "
        src = setup + f"compgen -A {key} {words.get(key, 'audit')} | LC_ALL=C sort -u"
        if key in ("running", "stopped", "job"):
            src = setup + f"compgen -A {key}; wait"
        a.compare("Completion action", key, src, interface=key in ("hostname", "stopped"))
    for word in ("!!", "!1", "!-1", "!echo", "!?file?", "!!:0", "!!:^", "!!:$", "!!:*", "!!:1-2", "!!:2*",
                 "!!:$:h", "!!:$:t", "!!:$:r", "!!:$:e", "!!:s/file/name/", "!!:p", "!!:q", "!!:x"):
        a.compare("History expansion", word, "history -c; history -s 'echo one dir/file.txt'; history -p '" + word + "'")
    levels = re.findall(r"^@item compat(\d+)", base["Shell Compatibility Mode"], re.M)
    for level in levels:
        script = f'BASH_COMPAT={level[0]}.{level[1:]}; shopt -q compat{level}; echo "$?"'
        if level in ("50", "51", "52"):
            # These levels have no shopt boolean. Exercise an actual rule.
            scripts = {"50": """BASH_COMPAT=5.0; RANDOM=42; printf '%s/%s' "$RANDOM" "$RANDOM" """,
                       "51": """BASH_COMPAT=5.1; declare -A a=([key]=value); test -v 'a[@]'; echo $?; unset 'a[@]'; declare -p a """,
                       "52": """BASH_COMPAT=5.2; test '(' '!' = '!' ')' -a yes; echo $?"""}
            script = scripts[level]
        a.compare("Compatibility level", level, script, interface=level not in ("50","51","52"))
    from posix_rules import audit_rules
    audit_rules(a, base["Bash POSIX Mode"])
    all_nodes = dict(base)
    for name in ("rluser.texi", "hsuser.texi"):
        all_nodes.update(sections(source / "lib/readline/doc" / name))
    for node in all_nodes:
        target = TOPICS.get(node)
        if not target:
            if re.search("Readline|Commands For|Killing|Completion|Init File|Searching|Numeric Arguments|Keyboard Macro", node):
                target = "Readline command"
            elif re.search("History|Designators", node): target = "History expansion"
            elif node in ("Installing Bash","Basic Installation","Compilers and Options","Compiling For Multiple Architectures",
                          "Installation Names","Specifying the System Type","Sharing Defaults","Operation Controls","Optional Features",
                          "Reporting Bugs","GNU Free Documentation License","Indexes","Builtin Index","Reserved Word Index",
                          "Variable Index","Function Index","Concept Index","Top","Introduction","What is Bash?","What is a shell?",
                          "Definitions","Major Differences From The Bourne Shell"): target = "reference material"
            else: target = "Shell language"
        a.add("Manual section", node, "index", target)
