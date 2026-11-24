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
    started = time.perf_counter_ns()
    with terminal(shell, home, home, home / "rc", prompt=PROMPT) as fd:
        prompt_time = (time.perf_counter_ns() - started) / 1e6
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
        if name.startswith("Miss"):
            start = time.perf_counter_ns()
            os.write(fd, packet + b"\r")
            read_until(fd, b"\x1b[?2004h")
            elapsed = (time.perf_counter_ns() - start) / 1e6
        else:
            elapsed = visible(fd, packet)
            os.write(fd, b"\r")
            read_until(fd, b"\x1b[?2004h")
        os.write(fd, b'printf %s "$v" > result\r')
        read_until(fd, b"\x1b[?2004h")
        path = home / "result"
        if path.read_bytes() != expected:
            raise ValueError(f"{shell} {name}: incorrect value")
        path.unlink()
    return elapsed, prompt_time


def scan_sample(binary, folder, query):
    with subprocess.Popen([binary, "--complete", folder, query.decode()],
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE) as control:
        if control.stdout.readline() != b"ready\n":
            raise ValueError("scan control failed to start")
        start = time.perf_counter_ns()
        out = control.communicate(b"\n")[0]
        spent = (time.perf_counter_ns() - start) / 1e6
        if out != query + FLAG + b"\n" or control.returncode:
            raise ValueError("scan control found different candidates")
        return spent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shell", nargs="+", type=Path)
    parser.add_argument("--runs", type=int, default=21)
    parser.add_argument("--entries", type=int, default=100000)
    parser.add_argument("--ascii-query", action="store_true", help="search for an ASCII substring")
    parser.add_argument("--prompt-time", action="store_true", help="also report PTY launch to prompt and launch through search")
    parser.add_argument("--history-rank", type=int, help="place the reverse-search target this many rows before the end")
    parser.add_argument("--scan-control", type=Path, help="also measure a direct unique-name directory scan")
    parser.add_argument("--large", action="store_true", help="also measure a 1 MiB paste")
    parser.add_argument("--case", action="append", choices=["commit", "paste", "Up", "Down", "Ctrl-R", "Miss", "Tab"])
    args = parser.parse_args()
    if args.entries < 4:
        parser.error("history must contain four or more entries")
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
        if args.ascii_query:
            query = b"deployment-region-west"
        target = b"v=" + query + FLAG
        decoy = b"v=" + "東京駅前の日本語確定結別候補".encode()
        if args.ascii_query:
            decoy = b"v=deployment-region-winter"
        recalled = "前の行🌱".encode() * 32 + FLAG
        commands = [target] + [f": padding-{i}".encode() for i in range(args.entries - 3)] + [b"v=" + recalled, decoy]
        if args.history_rank is not None:
            if not 2 <= args.history_rank < len(commands) - 1:
                parser.error("history rank must leave the final two rows in place")
            commands.insert(-args.history_rank, commands.pop(0))
        bash_history = home / ".bash_history"
        bash_history.write_bytes(b"\n".join(commands) + b"\n")
        journal.write_bytes(b"".join(b"1\t1\t0\t/tmp\t" + c + b"\n" for c in commands))
        with (home / "rc").open("a") as rc:
            rc.write(f"HISTSIZE={args.entries}\n")
            if args.case and "Miss" in args.case:
                rc.write("v=unchanged\n")
        folder = home / "candidates"
        if not args.case or "Tab" in args.case:
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
        count = "100k" if args.entries == 100000 else str(args.entries)
        cases.append((f"Ctrl-R ({count} entries)", b"\x12" + query, query + FLAG))
        if args.case and "Miss" in args.case:
            cases.append((f"Miss ({count} entries, through Enter)", b"\x12" + (b"ZzzUnseen" if args.ascii_query else "☃履歴に存在しない".encode()), b"unchanged"))
        cases.append(("Tab (10k files)", b"f candidates/" + query + b"\t", b"candidates/" + query + FLAG))
        for case in cases:
            if args.case and case[0].split()[0] not in args.case:
                continue
            participants = [*shells, None] if args.scan_control and case[0].startswith("Tab ") else shells
            timings = {shell: [] for shell in participants}
            startup = {shell: [] for shell in shells}
            total = {shell: [] for shell in shells}
            for i in range(args.runs + 3):
                shift = i % len(participants)
                for shell in participants[shift:] + participants[:shift]:
                    if shell is None:
                        ms = scan_sample(args.scan_control, folder, query)
                    else:
                        ms, launch = sample(shell, home, *case)
                        if i >= 3:
                            startup[shell].append(launch)
                            total[shell].append(launch + ms)
                        for path, length in sizes.items():
                            os.truncate(path, length)
                    if i >= 3:
                        timings[shell].append(ms)
            cells = []
            for values in timings.values():
                p95 = sorted(values)[math.ceil(.95 * len(values)) - 1]
                cells.append(f"{statistics.median(values):.3f}/{p95:.3f}")
            print(case[0], *cells[:len(shells)], sep=" | ", flush=True)
            if args.prompt_time:
                for name, values in (("PTY to prompt", startup), ("PTY through search", total)):
                    print(name, *(f"{statistics.median(v):.3f}/{sorted(v)[math.ceil(.95 * len(v))-1]:.3f}" for v in values.values()), sep=" | ")
            if None in timings:
                print("Direct scan (10k files)", cells[-1], sep=" | ", flush=True)


if __name__ == "__main__":
    main()
