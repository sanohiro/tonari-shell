"""Interleaved native shell timings with fresh, identical filesystem fixtures."""
import argparse
import base64
import json
import os
from pathlib import Path
import selectors
import shutil
import signal
import subprocess
import time

MAX_OUTPUT = 1024 * 1024


def expired(*_):
    raise TimeoutError()


def execute(binary, code, script, cwd, env, timeout):
    args = [str(binary)] if script else [str(binary), "-c", code]
    start = time.perf_counter_ns()
    proc = subprocess.Popen(args, env=env, cwd=cwd, start_new_session=True,
                            stdin=subprocess.PIPE if script else subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE if script else subprocess.STDOUT)
    stdout, stderr = bytearray(), bytearray()
    limit = None
    exited = False
    signal.setitimer(signal.ITIMER_REAL, timeout)
    try:
        with selectors.DefaultSelector() as poll:
            poll.register(proc.stdout, selectors.EVENT_READ, stdout)
            if proc.stderr:
                poll.register(proc.stderr, selectors.EVENT_READ, stderr)
            if script:
                os.set_blocking(proc.stdin.fileno(), False)
                poll.register(proc.stdin, selectors.EVENT_WRITE)
            while poll.get_map():
                for event, _ in poll.select():
                    stream = event.fileobj
                    if stream is proc.stdin:
                        try:
                            count = os.write(stream.fileno(), code[:4096])
                            code = code[count:]
                        except BrokenPipeError:
                            code = b""
                        if not code:
                            poll.unregister(stream)
                            stream.close()
                    else:
                        data = os.read(stream.fileno(), 65536)
                        if data:
                            event.data.extend(data)
                            if len(event.data) > MAX_OUTPUT:
                                raise OverflowError()
                        else:
                            poll.unregister(stream)
                            stream.close()
            # Keep the leader's PID reserved until its process group is cleaned.
            os.waitid(os.P_PID, proc.pid, os.WEXITED | os.WNOWAIT)
            exited = True
    except (TimeoutError, OverflowError) as exc:
        limit = "timeout" if isinstance(exc, TimeoutError) else "output"
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        ms = (time.perf_counter_ns() - start) / 1e6
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except PermissionError:
            # Darwin can report EPERM for a group with only exited members.
            # Preserve the error if the leader itself still requires stopping.
            if not exited:
                proc.kill()
        proc.wait()
        for stream in (proc.stdin, proc.stdout, proc.stderr):
            if stream:
                stream.close()
    return dict(status=proc.returncode, stdout=base64.b64encode(stdout).decode(),
                stderr=base64.b64encode(stderr).decode(), limit=limit), ms


def grade_builtin(case, actual, bash):
    if actual["limit"]:
        return actual["limit"].upper()
    kind = case["kind"]
    # Upstream uses command substitution with 2>&1. Capture the merged stream
    # for builtin cases at the shell boundary below.
    output = base64.b64decode(actual["stdout"]).rstrip(b"\n")
    ref = base64.b64decode(bash["stdout"]).rstrip(b"\n")
    if kind == "check_exit":
        ok = actual["status"] == int(case["expected"])
    elif kind == "check_output":
        ok = output == case["expected"].encode()
    elif kind == "compare_exit":
        ok = actual["status"] == bash["status"]
    elif kind == "compare_output":
        ok = output == ref
    elif kind == "compare_both":
        ok = output == ref and actual["status"] == bash["status"]
    else:
        raise ValueError(kind)
    return "PASS" if ok else "FAIL"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("manifest", "output", "bash", "tonari", "scratch"):
        parser.add_argument(name, type=lambda p: Path(p).absolute())
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--helper", type=Path)
    parser.add_argument("--utility-path", type=Path, action="append", default=[],
                        help="directory of explicit utility overrides for a controlled replay")
    parser.add_argument("--observer", type=Path)
    parser.add_argument("--shard", type=int, default=0)
    parser.add_argument("--shards", type=int, default=1)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--suite")
    args = parser.parse_args()
    signal.signal(signal.SIGALRM, expired)
    cases = [json.loads(s) for s in args.manifest.read_text().splitlines()]
    shell_dir = args.scratch / "shell"
    shell_dir.mkdir(parents=True, exist_ok=True)
    entry = shell_dir / "bash"
    cwd = args.scratch / "case"
    trace = args.scratch / "calls"
    # Keep the caller's GUI tools and login-shell customizations out of PATH.
    # Several exit-status cases literally run X expecting command-not-found.
    runtimes = args.scratch / "interpreters"
    runtimes.mkdir(exist_ok=True)
    for name in ("python2", "python3", "timeout"):
        executable = shutil.which(name)
        link = runtimes / name
        if executable and not link.exists():
            link.symlink_to(Path(executable).resolve())
    directories = ([args.helper.resolve()] if args.helper else []) + [
        shell_dir, args.source.resolve() / "spec/bin", runtimes,
        *(p.resolve() for p in args.utility_path), "/usr/bin", "/bin", "/usr/sbin", "/sbin",
    ]
    path = os.pathsep.join(map(str, directories))
    # Some Oils cases cd into REPO_ROOT and write _tmp/spec-tmp. Give them
    # the checkout layout and a private scratch directory for each invocation.
    repo_view = args.scratch / "repo"
    if not repo_view.exists():
        repo_view.mkdir()
        for child in args.source.resolve().iterdir():
            if child.name not in ("_tmp", ".git"):
                (repo_view / child.name).symlink_to(child)
    with args.output.open("w", buffering=1) as output:
        for n, case in enumerate(cases):
            if not case["include"] or n % args.shards != args.shard or (args.suite and case["suite"] != args.suite):
                continue
            script = "kind" not in case
            code = base64.b64decode(case["code"])
            row = {k: case[k] for k in ("key", "name", "suite", "index")}
            row.update(actual={}, ms={"bash": [], "tonari": []}, unstable=[], sites=[])
            ref = {"bash": args.bash, "tonari": args.tonari}
            if args.observer:
                ref["observer"] = args.observer
            for trial in range(args.runs + bool(args.observer)):
                order = ["bash", "tonari"] if (n + trial) % 2 else ["tonari", "bash"]
                if trial == args.runs:
                    order = ["observer"]
                for label in order:
                    if cwd.exists():
                        shutil.rmtree(cwd)
                    cwd.mkdir()
                    if case.get("legacy"):
                        (cwd / "_tmp").mkdir()
                    entry.unlink(missing_ok=True)
                    entry.symlink_to(ref[label])
                    env = dict(PATH=path, TMP=str(cwd), SH="bash", LC_ALL="C.UTF-8",
                               REPO_ROOT=str(args.source.resolve()))
                    if script and b"REPO_ROOT" in code:
                        tmp = repo_view / "_tmp"
                        if tmp.exists():
                            shutil.rmtree(tmp)
                        (tmp / "spec-tmp").mkdir(parents=True)
                        env["REPO_ROOT"] = str(repo_view)
                    cmd = code
                    if not script:
                        fixture = Path(case["fixture"])
                        shutil.copytree(fixture, cwd / "fixture")
                        cmd = code.replace(os.fsencode(fixture), os.fsencode(cwd / "fixture"))
                        env["HOME"] = str(cwd)
                    if label == "observer":
                        trace.write_bytes(b"")
                        env["TONARI_CASE_TRACE"] = str(trace)
                    result, ms = execute(entry, cmd, script, cwd, env, timeout=6)
                    if label == "observer":
                        row["sites"] = sorted(set(trace.read_bytes()))
                    else:
                        if label not in row["actual"]:
                            row["actual"][label] = result
                        elif row["actual"][label] != result and label not in row["unstable"]:
                            row["unstable"].append(label)
                        row["ms"][label].append(ms)
                # A timeout or oversized output is a functional result. Repeating
                # it does not supply comparable performance samples.
                if any(v["limit"] for v in row["actual"].values()):
                    break
            if not script:
                row["grades"] = {k: grade_builtin(case, v, row["actual"]["bash"])
                                 for k, v in row["actual"].items()}
            print(json.dumps(row), file=output)


if __name__ == "__main__":
    main()
