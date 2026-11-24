"""Measure UTF-8 commits, bracketed paste, history and Tab in a ready PTY."""
import argparse
import math
import os
from pathlib import Path
import select
import statistics
import subprocess
import tempfile
import time

from bench_history import PROMPT, read_until, terminal


FLAG = "🏁".encode()
WORD = "日本語🌱".encode()


def visible(fd, packet):
    """Drain redraws while feeding a large packet, stopping at the final visible scalar."""
    pending = memoryview(packet)
    tail = b""
    os.set_blocking(fd, False)
    start = time.perf_counter_ns()
    deadline = time.monotonic() + 20
    try:
        while time.monotonic() < deadline:
            reads, writes, _ = select.select([fd], [fd] if pending else [], [], 1)
            if writes:
                try:
                    pending = pending[os.write(fd, pending[:65536]):]
                except BlockingIOError:
                    pass
            if reads:
                block = os.read(fd, 65536)
                if not block:
                    raise EOFError("editor closed")
                output = tail + block
                if not pending and FLAG in output:
                    return (time.perf_counter_ns() - start) / 1e6
                tail = output[-len(FLAG):]
    finally:
        os.set_blocking(fd, True)
    raise TimeoutError(f"final scalar missing, {len(pending)} input bytes pending")


def sample(shell, home, name, packet, expected):
    with terminal(shell, home, home, home / "rc", prompt=PROMPT) as fd:
        if name.startswith("Up twice"):
            # Up navigates the active session. Seed its two lines before timing.
            for line in (b"v=" + expected, b"v=most-recent"):
                os.write(fd, line + b"\r")
                read_until(fd, b"\x1b[?2004h")
        if name.startswith("Down to draft"):
            os.write(fd, b"v=DOWNREF\r")
            read_until(fd, b"\x1b[?2004h")
            visible(fd, b"v=" + expected)
            os.write(fd, b"\x1b[A")
            read_until(fd, b"DOWNREF")
        elapsed = visible(fd, packet)
        os.write(fd, b"\r")
        read_until(fd, b"\x1b[?2004h")
        os.write(fd, b'printf %s "$v" > result\r')
        read_until(fd, b"\x1b[?2004h")
        path = home / "result"
        if path.read_bytes() != expected:
            raise ValueError(f"{shell} {name}: incorrect value")
        path.unlink()
    return elapsed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shell", nargs="+", type=Path)
    parser.add_argument("--runs", type=int, default=21)
    parser.add_argument("--scan-control", type=Path, help="also measure a direct unique-name directory scan")
    parser.add_argument("--large", action="store_true", help="also measure a 1 MiB paste")
    parser.add_argument("--case", action="append", choices=["commit", "paste", "Up", "Down", "Ctrl-R", "Tab"])
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("runs must be positive")
    shells = [s.resolve() for s in args.shell]
    with tempfile.TemporaryDirectory() as scratch:
        home = Path(scratch)
        (home / "rc").write_text(f"PS1='{PROMPT.decode()}'\nexport LC_ALL=" + ("en_US.UTF-8" if os.uname().sysname == "Darwin" else "C.UTF-8")
                                + "\nf() { v=$1; }; complete -f f\n")
        journal = home / ".local/state/tonari/history"
        journal.parent.mkdir(parents=True)
        query = "東京駅前の日本語確定結果".encode()
        target = b"v=" + query + FLAG
        decoy = b"v=" + "東京駅前の日本語確定結別候補".encode()
        recalled = "前の行🌱".encode() * 32 + FLAG
        commands = [target] + [f": padding-{i}".encode() for i in range(99997)] + [b"v=" + recalled, decoy]
        bash_history = home / ".bash_history"
        bash_history.write_bytes(b"\n".join(commands) + b"\n")
        journal.write_bytes(b"".join(b"1\t1\t0\t/tmp\t" + c + b"\n" for c in commands))
        with (home / "rc").open("a") as rc:
            rc.write("HISTSIZE=100000\n")
        folder = home / "candidates"
        folder.mkdir()
        for i in range(9999):
            (folder / f"item-{i:05d}").touch()
        (folder / (query + FLAG).decode()).touch()
        sizes = {p: p.stat().st_size for p in (journal, bash_history)}
        cases = []
        for repeats in (0, 4, 64):
            value = WORD * repeats + FLAG
            cases.append((f"commit (scalars: {4 * repeats + 1})", b"v=" + value, value))
        for size in ([16384, 1048576] if args.large else [16384]):
            value = (WORD * (size // len(WORD)) + b"a" * (size % len(WORD))) + FLAG
            cases.append((f"paste {size} B + marker", b"\x1b[200~v=" + value + b"\x1b[201~", value))
        cases.append(("Up twice (100k entries)", b"\x1b[A\x1b[A", recalled))
        cases.append(("Down to draft (100k entries)", b"\x1b[B", recalled))
        cases.append(("Ctrl-R (100k entries)", b"\x12" + query, query + FLAG))
        cases.append(("Tab (10k files)", b"f candidates/" + query + b"\t", b"candidates/" + query + FLAG))
        if args.scan_control:
            elapsed = []
            for i in range(args.runs + 3):
                with subprocess.Popen([args.scan_control, "--complete", folder, query.decode()],
                                      stdin=subprocess.PIPE, stdout=subprocess.PIPE) as control:
                    if control.stdout.readline() != b"ready\n":
                        raise ValueError("scan control failed to start")
                    start = time.perf_counter_ns()
                    out = control.communicate(b"\n")[0]
                    spent = (time.perf_counter_ns() - start) / 1e6
                    if out != query + FLAG + b"\n" or control.returncode:
                        raise ValueError("scan control found different candidates")
                    if i >= 3:
                        elapsed.append(spent)
            print(f"Direct scan (10k files) | {statistics.median(elapsed):.3f}/{sorted(elapsed)[math.ceil(.95 * len(elapsed)) - 1]:.3f}", flush=True)
        for case in cases:
            if args.case and case[0].split()[0] not in args.case:
                continue
            timings = {shell: [] for shell in shells}
            for i in range(args.runs + 3):
                shift = i % len(shells)
                for shell in shells[shift:] + shells[:shift]:
                    ms = sample(shell, home, *case)
                    for path, length in sizes.items():
                        os.truncate(path, length)
                    if i >= 3:
                        timings[shell].append(ms)
            cells = []
            for values in timings.values():
                p95 = sorted(values)[math.ceil(.95 * len(values)) - 1]
                cells.append(f"{statistics.median(values):.3f}/{p95:.3f}")
            print(case[0], *cells, sep=" | ", flush=True)


if __name__ == "__main__":
    main()
