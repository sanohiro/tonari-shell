"""Measure editor redraw and history suggestions, Tab, or Enter in fresh pseudo-terminals."""

import argparse
import math
import os
import pty
import select
import signal
import statistics
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path


PROMPT = b"tonari$ "
NEW_PROMPT = b"\x1b[?2004h\r\x1b[J" + PROMPT


def read_until(fd, marker, initial=b""):
    output = bytearray(initial)
    deadline = time.monotonic() + 10
    while marker not in output:
        if not select.select([fd], [], [], max(0, deadline - time.monotonic()))[0]:
            raise TimeoutError(f"waiting for {marker!r}: {output[-120:]!r}")
        data = os.read(fd, 4096)
        if not data:
            raise EOFError(marker)
        output.extend(data)
    return output


@contextmanager
def terminal(shell, home, bindir, rc=None, prompt=NEW_PROMPT):
    pid, fd = pty.fork()
    if pid == 0:
        try:
            os.chdir(home)
            args = ["--rcfile", str(rc)] if rc else ["--norc"]
            os.execve(shell, [shell, "--noprofile", *args],
                      {"HOME": str(home), "TERM": "xterm-256color",
                       "PATH": str(bindir), "PS1": PROMPT.decode(), "INPUTRC": "/dev/null"})
        finally:
            os._exit(125)
    try:
        read_until(fd, prompt)
        yield fd
    finally:
        os.kill(pid, signal.SIGKILL)
        os.close(fd)
        os.waitpid(pid, 0)


def sample(shell, home, bindir, action, count):
    with terminal(shell, home, bindir) as fd:
        if action == "Hint":
            start = time.perf_counter_ns()
            os.write(fd, b"p")
            read_until(fd, f"\x1b[2mrintf entry-{count - 1:06d}\x1b[0m".encode())
            return [(time.perf_counter_ns() - start) / 1e6]
        if action == "Old hint":
            start = time.perf_counter_ns()
            os.write(fd, b"printf entry-00000")
            read_until(fd, b"\x1b[2m9\x1b[0m")
            return [(time.perf_counter_ns() - start) / 1e6]
        if action in ("Escaped hint", "Escaped search"):
            prefix = rb"printf entry-000000 foo\ bar"
            if action == "Escaped hint":
                start = time.perf_counter_ns()
                os.write(fd, prefix)
                output = read_until(fd, PROMPT + prefix)
                echo = time.perf_counter_ns()
                read_until(fd, b"\x1b[2m-END\x1b[0m", initial=output)
                return [(echo - start) / 1e6, (time.perf_counter_ns() - start) / 1e6]
            query = rb"foo\ bar"
            start = time.perf_counter_ns()
            os.write(fd, b"\x12" + query)
            read_until(fd, b"(reverse-i-search) `" + query + b"': " + prefix + b"-END")
            return [(time.perf_counter_ns() - start) / 1e6]
        if action in ("Recent search", "Old search"):
            number = count - 1 if action == "Recent search" else 0
            query = f"entry-{number:06d}".encode()
            start = time.perf_counter_ns()
            os.write(fd, b"\x12" + query)
            read_until(fd, b"(reverse-i-search) `" + query + b"': printf " + query)
            return [(time.perf_counter_ns() - start) / 1e6]
        if action == "Cancel search":
            start = time.perf_counter_ns()
            os.write(fd, b"\x12\x07")
            data = read_until(fd, b"(reverse-i-search)")
            read_until(fd, PROMPT, initial=data[data.index(b"(reverse-i-search)"):])
            return [(time.perf_counter_ns() - start) / 1e6]
        if action in ("Burst", "Paste"):
            count = 512 if action == "Burst" else 16384
            value = b"a" * count + b"TAILMARK"
            line = b"v=" + value
            packet = line if action == "Burst" else b"\x1b[200~" + line + b"\x1b[201~"
            start = time.perf_counter_ns()
            while packet:
                packet = packet[os.write(fd, packet):]
            read_until(fd, b"TAILMARK")
            end = time.perf_counter_ns()
            os.write(fd, b"\r")
            read_until(fd, NEW_PROMPT)
            os.write(fd, b'echo "${#v}"\r')
            read_until(fd, f"\x1b[?2004l{len(value)}\r\n".encode())
            return [(end - start) / 1e6]
        tab = action == "Tab"
        first = b"z" if tab else b":"
        start = time.perf_counter_ns()
        os.write(fd, first)
        read_until(fd, PROMPT + first)
        second = time.perf_counter_ns()
        os.write(fd, b"\t" if tab else b"\r")
        read_until(fd, PROMPT + b"zcomplete " if tab else NEW_PROMPT)
        end = time.perf_counter_ns()
        return [(second - start) / 1e6, (end - second) / 1e6, (end - start) / 1e6]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shell", type=Path, nargs="+")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--bursts", action="store_true", help="include typeahead, paste and search cancellation")
    mode.add_argument("--escaped", action="store_true", help="measure escaped queries and plain Ctrl-R controls")
    parser.add_argument("--runs", type=int, default=30)
    parser.add_argument("--entries", type=int, nargs="+", default=[0, 1000, 100000])
    args = parser.parse_args()
    if args.runs < 1 or min(args.entries) < 0:
        parser.error("expected positive runs and nonnegative entry counts")
    shells = [str(p.resolve()) for p in args.shell]
    with tempfile.TemporaryDirectory() as scratch:
        root = Path(scratch)
        bindir = root / "bin"
        bindir.mkdir()
        tool = bindir / "zcomplete"
        tool.touch(mode=0o755)
        jobs = []
        for count in args.entries:
            home = root / str(count)
            journal = home / ".local/state/tonari/history"
            journal.parent.mkdir(parents=True)
            with journal.open("wb") as output:
                for i in range(count):
                    suffix = r" foo\\ bar-END" if args.escaped and i == 0 else ""
                    output.write(f"1\t1\t0\t/tmp\tprintf entry-{i:06d}{suffix}\n".encode())
            size = journal.stat().st_size
            actions = ["Tab", "Enter", "Hint"] if count else ["Tab", "Enter"]
            if args.bursts:
                actions += ["Cancel search", "Burst", "Paste"]
                if count >= 10:
                    actions += ["Old hint", "Recent search", "Old search"]
            if args.escaped and count:
                actions = ["Escaped hint", "Escaped search", "Old search", "Recent search"]
            for action in actions:
                for shell in shells:
                    jobs.append((shell, count, action, home, journal, size))
        samples = {job: [] for job in jobs}
        for i in range(3 + args.runs):
            shift = i % len(jobs)
            for job in jobs[shift:] + jobs[:shift]:
                shell, count, action, home, journal, size = job
                elapsed = sample(shell, home, bindir, action, count)
                if action in ("Enter", "Burst", "Paste"):
                    os.truncate(journal, size)
                if i >= 3:
                    samples[job].append(elapsed)
        print("entries action shell | first key | next key | both (median/p95 ms)")
        print("Hint rows show the complete first key to visible suggestion interval.")
        if args.escaped:
            print("Escaped hint reports prefix echo and visible suggestion, both from input.")
        for (shell, count, action, *_), values in samples.items():
            metrics = []
            for times in zip(*values):
                p95 = sorted(times)[math.ceil(0.95 * len(times)) - 1]
                metrics.append(f"{statistics.median(times):.3f}/{p95:.3f}")
            print(count, action, shell, "|", " | ".join(metrics))


if __name__ == "__main__":
    main()
