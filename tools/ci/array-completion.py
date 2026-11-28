"""Time the installed bash-completion implementation before and after initialization."""
from pathlib import Path
import shlex
import statistics
import sys
import tempfile

from bench_scripts import sample


def main():
    integration, *shells = map(Path, sys.argv[1:5])
    runs = int(sys.argv[5])
    load = ". " + shlex.quote(str(integration.resolve())) + "; "
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
            results = [[] for _ in shells]
            for trial in range(5 + runs):
                for k in range(len(shells)):
                    index = (trial + k) % len(shells)
                    elapsed = sample(shells[index], script, b"ok\n", env, True, setup)
                    if trial >= 5:
                        results[index].append(elapsed)
            print(name, *[f"{statistics.median(series):.3f}" for series in results], sep=" | ")


if __name__ == "__main__":
    main()
