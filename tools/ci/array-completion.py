"""Exercise the installed bash-completion script directly and through a real Tab."""

import os
from pathlib import Path
import shlex
import statistics
import sys
import tempfile
import time

from bench_history import PROMPT, read_until, terminal
from bench_scripts import sample


def tab_completion(shell, integration, parent):
    with tempfile.TemporaryDirectory(dir=parent) as temp:
        home = Path(temp)
        (home / "r8-target").touch()
        rc = home / "rc"
        rc.write_text(". " + shlex.quote(str(integration)) + "\n" + r'''
PS1='tonari$ '
_r8_test() { local cur=${COMP_WORDS[COMP_CWORD]}; _filedir; }
r8tool() { printf %s "$1" > verified; echo R8_CONFIRMED; }
complete -F _r8_test r8tool
''')
        with terminal(shell, home, "/usr/bin:/bin", rc, PROMPT) as fd:
            os.write(fd, b"r8tool r8-")
            read_until(fd, b"r8-")
            start = time.perf_counter_ns()
            os.write(fd, b"\t")
            read_until(fd, b"target")
            elapsed = (time.perf_counter_ns() - start) / 1e6
            os.write(fd, b"\r")
            read_until(fd, b"R8_CONFIRMED")
            if (home / "verified").read_bytes() != b"r8-target":
                raise RuntimeError(f"{shell} accepted an unexpected completion")
            return elapsed


def compare(name, shells, runs, measure):
    results = [[] for _ in shells]
    for trial in range(5 + runs):
        for offset in range(len(shells)):
            index = (trial + offset) % len(shells)
            elapsed = measure(shells[index])
            if trial >= 5:
                results[index].append(elapsed)
    print(name, *[f"{statistics.median(series):.3f}" for series in results], sep=" | ")


def main():
    integration, *shells = (Path(p).resolve() for p in sys.argv[1:5])
    runs = int(sys.argv[5])
    load = ". " + shlex.quote(str(integration)) + "; "
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        (root / "item-one").mkdir()
        (root / "item-two").touch()
        env = dict(HOME=temp, PATH="/usr/bin:/bin", LC_ALL="C",
                   BASH_COMPLETION_COMPAT_DIR=str(root / "empty"))
        for name, setup, count, directories in (
            ("source and first filename completion", "", 1, False),
            ("filename completion after source", load, 50, False),
            ("directory completion after source", load, 50, True),
        ):
            script = root / "trial.sh"
            check = ('[[ ${COMPREPLY[*]} == item-one ]]' if directories else
                     '[[ ${#COMPREPLY[@]} == 2 && " ${COMPREPLY[*]} " == *" item-one "* && " ${COMPREPLY[*]} " == *" item-two "* ]]')
            body = 'cur=item-; COMPREPLY=(); _filedir' + (' -d' if directories else '') + '; '
            script.write_text(("" if setup else load) + f'for ((i=0;i<{count};i++)); do ' +
                              body + check + ' || exit 1; done; echo ok\n')
            compare(name, shells, runs, lambda shell: sample(shell, script, b"ok\n", env, True, setup))
        compare("Tab to filename visible", shells, runs, lambda shell: tab_completion(shell, integration, root))


if __name__ == "__main__":
    main()
