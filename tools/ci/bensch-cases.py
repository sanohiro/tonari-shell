"""Extract bensch's shell-expanded builtin cases and preserve their setup files."""
import base64
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

# Preserve quoting, multiline commands, conditional branches and upstream setup.
# All five helper variants pass a description and an already expanded command.
HARNESS = r"""
section() { :; }
print_summary() { :; }
record() { printf '%s\0' "$1" "$2" "$3" "${4-}" >> "$RECORDS"; }
compare_output() { record compare_output "$@"; }
compare_exit() { record compare_exit "$@"; }
compare_both() { record compare_both "$@"; }
check_output() { record check_output "$@"; }
check_exit() { record check_exit "$@"; }
skip() { record skip "$@"; }
"""


def main():
    source, scratch, bash = map(lambda a: Path(a).resolve(), sys.argv[1:])
    suites = scratch / "builtin-suites"
    shutil.copytree(source / "suites/builtins", suites)
    (suites / "test_harness.sh").write_text(HARNESS)
    # test_stress.sh sources a nonexistent adjacent copy in this revision.
    (suites / "portable/test_harness.sh").symlink_to("../test_harness.sh")
    for path in sorted(suites.rglob("test_*.sh")):
        if "integration" in path.parts or path.name == "test_harness.sh":
            continue
        suite = str(path.relative_to(suites).with_suffix(""))
        setup = scratch / "fixtures" / suite
        setup.mkdir(parents=True)
        records = scratch / "records"
        records.write_bytes(b"")
        env = dict(PATH=os.environ["PATH"], TEST_TMPDIR=str(setup), RECORDS=str(records),
                   HOME=str(scratch), BASH_REF=str(bash), FORCE_JOB_TESTS="1", FORCE_COPROC_TESTS="1")
        subprocess.run(["/bin/sh", path], env=env, cwd=setup, stdin=subprocess.DEVNULL,
                       stdout=sys.stderr, check=True)
        values = records.read_bytes().split(b"\0")
        assert values.pop() == b"" and len(values) % 4 == 0
        for i in range(0, len(values), 4):
            kind, title, cmd, expected = values[i:i+4]
            print(json.dumps(dict(key=f"{suite}/{i//4}", suite=suite, index=i//4,
                                  name=title.decode(), kind=kind.decode(), expected=expected.decode(),
                                  include=kind != b"skip", fixture=str(setup),
                                  code=base64.b64encode(cmd).decode())))


if __name__ == "__main__":
    main()
