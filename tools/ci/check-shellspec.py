"""Run the pinned ShellSpec sample bodies once and compare their results with Bash."""
from pathlib import Path
import os
import re
import subprocess
import sys
import tempfile


def samples(directory):
    for file in sorted(directory.glob("*.sh")):
        chunks = re.split(r'^#bench "([^"\n]+)"[^\n]*\n', file.read_text(), flags=re.M)
        header = "set -e --\ntrap : EXIT HUP TERM PIPE\nsetup() { :; }\n" + chunks[0] + "\nsetup\n"
        for name, body in zip(chunks[1::2], chunks[2::2]):
            # Keep the workload's lexical scope. For example, the assignment
            # samples put both markers inside the function that owns the local.
            body, begins = re.subn(r'(?m)^\s*@begin[ \t]*$', 'for __review in once; do', body)
            checkpoint = """done
__status=$?
if declare -F foo >/dev/null; then __foo=defined; else __foo=missing; fi
printf '\\036%s\\037%s\\037%s\\037%s\\037%s\\037%s\\n' "$__status" "${var-unset}" "${i-unset}" "$#" "${1-unset}" "$__foo"
"""
            body, ends = re.subn(r'(?m)^\s*@end[ \t]*$', lambda _: checkpoint, body)
            if (begins, ends) != (1, 1):
                raise ValueError(f"unexpected sample markers in {file.name}: {name}")
            yield f"{file.name}: {name}", header + body


def main():
    source, bash, *candidates = sys.argv[1:]
    if not candidates:
        raise SystemExit("usage: check-shellspec.py SAMPLE_DIR BASH TONARI [OTHER_SHELL ...]")
    with tempfile.TemporaryDirectory() as home:
        env = dict(PATH=os.environ["PATH"], HOME=home, LC_ALL="C")
        for name, script in samples(Path(source)):
            def execute(shell):
                p = subprocess.run([shell, "-c", script], env=env, cwd=home,
                                   capture_output=True, timeout=10)
                return p.returncode, p.stdout, p.stderr

            status, out, err = execute(bash)
            if status or err:
                if name not in {"null.sh: blank", "output.sh: print", "stringop2.sh: substr 2 expr"}:
                    raise ValueError(f"{name}: unexpected Bash result {status}, {err!r}")
            for shell in candidates:
                code, actual, diagnostic = execute(shell)
                if (code, actual) != (status, out) or (bool(diagnostic) != bool(err)):
                    raise ValueError(f"{name}: Bash {(status, out, err)!r}, {shell} {(code, actual, diagnostic)!r}")
            print(f"{name}\t" + (f"unavailable ({status})" if status else "match"))


if __name__ == "__main__":
    main()
