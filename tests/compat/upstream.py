"""Run GNU Bash regression drivers against a control and a candidate in a disposable Linux container."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import signal
import select
import time
import pty
import termios
import subprocess
import tempfile


def run(driver, shell, root, source, helpers, timeout):
    # Drivers use relative paths and create files under both HOME and TMPDIR.
    shutil.copytree(source / "tests", root / "tests")
    shutil.copy2(source / "y.tab.c", root / "y.tab.c")
    if driver == "run-heredoc":
        for name in ("config.h", "version.h"):
            shutil.copy2(helpers / name, root / name)
    if driver == "run-glob-bracket":
        makefile = Path("examples/loadables/Makefile")
        (root / makefile.parent).mkdir(parents=True)
        shutil.copy2(helpers / makefile, root / makefile)
    for name in ("recho", "zecho", "printenv", "xcase"):
        shutil.copy2(helpers / name, root / "tests" / name)
    for name in ("bin", "home", "tmp"):
        (root / name).mkdir()
    shutil.copy2(shell, root / "bin/bash")
    (root / "bash").symlink_to("bin/bash")
    (root / "bin/diff").write_text("""#!/bin/sh
/usr/bin/diff "$@"
code=$?
if [ "$DRIVER_PID" = "$PPID" ]; then printf '%s\\n' "$code" >> "$DIFF_LOG"; fi
exit "$code"
""")
    (root / "bin/diff").chmod(0o755)
    log = root / "diff-status"
    log.touch()
    env = dict(PATH=f"{root}/bin:{root}/tests:/usr/bin:/bin", HOME=str(root / "home"),
               TMPDIR=str(root / "tmp"), LC_ALL="C", TERM="dumb", BUILD_DIR=str(root),
               BASH_TSTOUT=str(root / "tmp/output"), THIS_SH=str(root / "bin/bash"), DIFF_LOG=str(log))
    timed_out = False
    with (root / "output").open("wb") as output:
        master, slave = pty.openpty()
        def attach_terminal():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
            os.close(slave)
        start = time.perf_counter_ns()
        proc = subprocess.Popen(["/bin/sh", "-c", 'export DRIVER_PID=$$; . "./$1"', "sh", driver],
                                cwd=root / "tests", env=env, stdin=subprocess.DEVNULL, stdout=output,
                                stderr=subprocess.STDOUT, pass_fds=(slave,), preexec_fn=attach_terminal)
        os.close(slave)
        try:
            fd = os.pidfd_open(proc.pid)
            try:
                timed_out = not select.select([fd], [], [], timeout)[0]
                if not timed_out:
                    proc.wait()
            finally:
                os.close(fd)
            elapsed = time.perf_counter_ns() - start
        finally:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            os.close(master)
            proc.wait()
    counts = [int(n) for n in log.read_text().splitlines()]
    with (root / "output").open("rb") as f:
        lines = f.read(128 * 1024).decode("utf-8", "replace").replace(str(root), "$WORK").splitlines()
    detail = next((s[:160] for s in lines if s.startswith(("< ", "> ", "diff:"))), "")
    skips = [s[:160] for s in lines if "skipping" in s.lower() and not s.startswith(("< ", "> "))]
    return dict(ns=elapsed, exit=proc.returncode, timeout=timed_out, diffs=counts, skip=skips, example=detail)


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ("bash", "tonari", "source", "helpers"):
        cli.add_argument("--" + name, type=Path, required=True)
    cli.add_argument("--timeout", type=int, default=110)
    cli.add_argument("--only", help="run one named regression group")
    args = cli.parse_args()
    if os.getuid() == 0 or not Path("/.dockerenv").exists():
        cli.error("use a disposable non-root Docker container for the upstream test drivers")
    result = {}
    for file in sorted((args.source / "tests").glob("run-*")):
        name = file.name
        if args.only and args.only != name:
            continue
        if name in ("run-all", "run-minimal", "run-gprof"):
            continue  # orchestration and profiling drivers
        if name == "run-exportfunc":
            result[name] = {"status": "separate export-function probe"}
            continue
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "work"
            pair = []
            for shell in (args.bash, args.tonari):
                root.mkdir()
                pair.append(run(name, shell.resolve(), root, args.source, args.helpers, args.timeout))
                shutil.rmtree(root)
            ref, target = pair
            if any(x["timeout"] for x in pair):
                status = "timeout"
            elif (ref["exit"] != 0 or not ref["diffs"] or any(ref["diffs"]) or ref["skip"]):
                status = "control inconclusive"
            elif (target["exit"] != 0 or ref["diffs"] != target["diffs"] or target["skip"]):
                status = "different"
            else:
                status = "golden match"
            result[name] = dict(status=status, bash=ref, tonari=target)
    print(json.dumps(result, ensure_ascii=True, indent=2))


if __name__ == "__main__":
    main()
