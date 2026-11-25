"""Check the two bensch /dev/tty reopen commands under a controlling terminal."""
import base64
import errno
import json
import os
from pathlib import Path
import pty
import select
import shutil
import signal
import sys
import tempfile
import time


def sample(shell, directory, command):
    start = time.perf_counter_ns()
    pid, fd = pty.fork()
    if pid == 0:
        try:
            os.chdir(directory)
            os.execve(shell, ["bash", "-c", command],
                      {"HOME": str(directory), "PATH": "/usr/bin:/bin", "LC_ALL": "C"})
        finally:
            os._exit(125)
    data = bytearray()
    try:
        while select.select([fd], [], [], 5)[0]:
            try:
                part = os.read(fd, 8192)
            except OSError as e:
                if e.errno != errno.EIO:
                    raise
                break
            if not part:
                break
            data.extend(part)
        else:
            raise TimeoutError(command)
        _, status = os.waitpid(pid, 0)
    except BaseException:
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
        raise
    finally:
        os.close(fd)
    return os.waitstatus_to_exitcode(status), bytes(data), (time.perf_counter_ns() - start) / 1e6


def main():
    manifest, bash, tonari = (Path(a).absolute() for a in sys.argv[1:])
    cases = [r for r in map(json.loads, manifest.read_text().splitlines())
             if r["suite"] == "portable/test_exec" and b"/dev/tty" in base64.b64decode(r["code"])]
    assert len(cases) == 2
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp) / "case"
        for case in cases:
            command = base64.b64decode(case["code"]).replace(os.fsencode(case["fixture"]), os.fsencode(work))
            expected = b"captured\r\n" if case["name"].endswith("stdout") else b"err\r\n"
            for trial in range(11):
                pair = [("bash", bash), ("tonari", tonari)]
                if trial % 2:
                    pair.reverse()
                for label, shell in pair:
                    shutil.copytree(case["fixture"], work)
                    code, data, ms = sample(shell, work, command)
                    print(json.dumps(dict(key=case["key"], name=case["name"], trial=trial,
                                          shell=label, status=code, output=data.decode(), ms=ms)), flush=True)
                    if code != 0 or data != expected:
                        raise RuntimeError(f"{case['key']} {label}: {code} {data!r}")
                    shutil.rmtree(work)


if __name__ == "__main__":
    main()
