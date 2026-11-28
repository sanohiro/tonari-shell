"""Measure accepting one argument and filtering a million history rows in a real PTY."""
import argparse
from collections import defaultdict
import math
import os
from pathlib import Path
import statistics
import tempfile
import time

from bench_history import PROMPT, terminal, read_until


def elapsed(start, end):
    return (end - start) / 1e6


def sample(shell, home, mode):
    selected = home / "selected"
    selected.unlink(missing_ok=True)
    with terminal(shell, home, "/usr/bin:/bin", home / "rc", prompt=PROMPT) as fd:
        if mode in ("short", "long"):
            prefix = b" save " + mode.encode() + b"-"
            os.write(fd, prefix)
            read_until(fd, b"\x1b[2m" + b"accepted")
            start = time.perf_counter_ns()
            os.write(fd, b"\x1bf")
            read_until(fd, prefix + b"accepted\x1b[2m")
            values = {"Alt-F, " + mode: elapsed(start, time.perf_counter_ns())}
            expected = mode.encode() + b"-accepted"
            os.write(fd, b"\r")
        else:
            name = "桜🌱" if mode == "old" else "東京🌱"
            fragment = name.encode()
            cmd = b" save " + fragment + b" --port=15432"
            os.write(fd, b"\x12")
            read_until(fd, b"(reverse-i-search)")
            query = b"15432 " + fragment
            if mode == "typeahead":
                # The first hit is visible while the picker continues searching
                # older rows for the rest of its page.
                os.write(fd, "15432 限定".encode())
                read_until(fd, ">  save 限定 --port=15432".encode())
                packet = b"\x7f\x7f" + fragment
            else:
                packet = query
            start = time.perf_counter_ns()
            os.write(fd, packet)
            data = read_until(fd, b"history> " + query)
            echo = time.perf_counter_ns()
            read_until(fd, b"> " + cmd, initial=data)
            end = time.perf_counter_ns()
            values = {mode + ", query echo": elapsed(start, echo),
                      mode + ", selected row visible": elapsed(start, end)}
            os.write(fd, b"\r")
            read_until(fd, PROMPT + cmd)
            expected = fragment
            os.write(fd, b"\r")
        read_until(fd, b"\x1b[?2004h")
        if selected.read_bytes() != expected:
            raise ValueError(f"wrong selected argument in {mode}")
        return values


DEEP_ROW = 128


def deep_command(index):
    return f" save pick token-{index:03d}".encode()


def deep_sample(shell, home):
    with terminal(shell, home, "/usr/bin:/bin", home / "rc", prompt=PROMPT) as fd:
        os.write(fd, b"\x12")
        read_until(fd, b"(reverse-i-search)")
        os.write(fd, b"pick token")
        read_until(fd, b"> " + deep_command(0))
        for index in range(1, DEEP_ROW):
            os.write(fd, b"\x1b[B")
            read_until(fd, b"> " + deep_command(index))
        start = time.perf_counter_ns()
        os.write(fd, b"\x1b[B")
        read_until(fd, b"> " + deep_command(DEEP_ROW))
        duration = elapsed(start, time.perf_counter_ns())
        os.write(fd, b"\r")
        read_until(fd, PROMPT + deep_command(DEEP_ROW))
        return {"Down past repeated history entries": duration}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("shell", type=Path, nargs="+")
    p.add_argument("--runs", type=int, default=21)
    p.add_argument("--entries", type=int, default=1000000)
    p.add_argument("--deep-page", action="store_true",
                   help="select the first new match beyond a long run of duplicate history")
    p.add_argument("--mixed-records", action="store_true",
                   help="vary journal version, timestamp, process and directory across duplicate commands")
    p.add_argument("--alternating", action="store_true", help="alternate two repeated commands in a mixed journal")
    args = p.parse_args()
    if args.alternating and not args.mixed_records:
        p.error("--alternating requires --mixed-records")
    if args.mixed_records and not args.deep_page:
        p.error("--mixed-records requires --deep-page")
    shells = [s.resolve() for s in args.shell]
    if args.entries < 20 or args.runs < 1:
        p.error("need at least 20 entries and a positive number of trials")
    with tempfile.TemporaryDirectory() as d:
        home = Path(d)
        (home / "rc").write_text("PS1='tonari$ '\nHISTCONTROL=ignorespace\n"
                                 'save() { printf %s "$1" >selected; }\n')
        journal = home / ".local/state/tonari/history"
        journal.parent.mkdir(parents=True)
        head = b"1\t9\t0\t/tmp\t"
        with journal.open("wb") as out:
            if args.deep_page:
                out.write(head + deep_command(DEEP_ROW) + b"\n")
                duplicate = head + deep_command(DEEP_ROW - 1) + b"\n"
                for i in range(args.entries):
                    if args.mixed_records:
                        version = f"2\t{1800000000+i}\t" if i % 2 else "1\t"
                        prefix = f"{version}{i % 13}\t{i % 3}\t/dir{i % 5}\t".encode()
                        out.write(prefix + deep_command(DEEP_ROW - 1 - (i % 2 if args.alternating else 0)) + b"\n")
                    else:
                        out.write(duplicate)
                for index in reversed(range(DEEP_ROW)):
                    out.write(head + deep_command(index) + b"\n")
            else:
                out.write(head + " save 桜🌱 --port=15432\n".encode())
                for i in range(args.entries - 12):
                    out.write(head + f": log --port=15432 --task {i}\n".encode())
                for i in range(8):
                    out.write(head + f" save 東京🌱 --port=15432 {i}\n".encode())
                out.write(head + " save 限定 --port=15432\n".encode())
                for mode, extra in [(b"short", 1), (b"long", 4096)]:
                    out.write(head + b" save " + mode + b"-accepted" + b" tail" * extra + b"\n")
        for mode in (("deep",) if args.deep_page else ("short", "long", "recent", "old", "typeahead")):
            values = defaultdict(lambda: defaultdict(list))
            for trial in range(args.runs + 3):
                offset = trial % len(shells)
                for shell in shells[offset:] + shells[:offset]:
                    observed = deep_sample(shell, home) if mode == "deep" else sample(shell, home, mode)
                    for key, value in observed.items():
                        if trial >= 3:
                            values[key][shell].append(value)
            for key, by_shell in values.items():
                row = []
                for sh in shells:
                    times = sorted(by_shell[sh])
                    row.append(f"{statistics.median(times):.3f}/{times[math.ceil(.95*len(times))-1]:.3f}")
                print(key, *row, sep=" | ", flush=True)


if __name__ == "__main__":
    main()
