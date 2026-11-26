"""Time input-to-visible-edit in an interactive PTY and verify the entire buffer."""
import argparse
import math
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time

from bench_editor_input import FLAG, visible
from bench_history import PROMPT, read_until, terminal

HOME, END, BACK, FORWARD = b"\x01", b"\x05", b"\x02", b"\x06"
LEFT_WORD, RIGHT_WORD = b"\x1bb", b"\x1bf"
DELETE, INSERT = b"\x1b[3~", b"\x1b[2~"
PASTE, END_PASTE = b"\x1b[200~", b"\x1b[201~"


def cases(text):
    # '@' exposes the final position of each operation, including pure motions.
    before = lambda i, s="@": text[:i] + s + text[i:]
    last_word = text.rindex("last")
    return [
        ("append", b"@", text + "@"),
        ("insert near head", HOME + FORWARD * 2 + b"@", before(2)),
        ("Delete near head", HOME + FORWARD + DELETE + b"@", text[:1] + "@" + text[2:]),
        ("Backspace near head", HOME + FORWARD * 2 + b"\x7f@", text[:1] + "@" + text[2:]),
        ("character left/right", BACK * 3 + FORWARD + b"@", before(-2)),
        ("word left", LEFT_WORD + b"@", before(last_word)),
        ("word right", HOME + RIGHT_WORD + b"@", before(5)),
        ("transpose UTF-8 characters", END + "雪😀".encode() + b"\x14@", text + "😀雪@"),
        ("transpose words", END + b" one two\x1bt@", text + " two one@"),
        ("uppercase word", HOME + b"\x1bu@", "FIRST@" + text[5:]),
        ("Home", HOME + b"@", "@" + text),
        ("End", HOME + END + b"@", text + "@"),
        ("overwrite", HOME + INSERT + b"X" + INSERT + b"@", "X@" + text[1:]),
        ("copy and yank", b"\0" + BACK * 3 + b"\x1bw" + HOME + b"\x19@", text[-3:] + "@" + text),
        ("kill and yank", HOME + b"\x0b\x19@", text + "@"),
        ("IME at head", HOME + "日本語🌱@".encode(), "日本語🌱@" + text),
        ("multiline paste at head", HOME + PASTE + "日本\n語🌱".encode() + END_PASTE + b"@", "日本\n語🌱@" + text),
    ]


def sample(shell, home, rc, content, keys, expected):
    with terminal(shell, home, home, rc, prompt=PROMPT) as fd:
        visible(fd, PASTE + content.encode() + END_PASTE)
        start = time.perf_counter_ns()
        os.write(fd, keys)
        read_until(fd, b"@")
        elapsed = (time.perf_counter_ns() - start) / 1e6
        check_buffer(fd, home, expected, (shell, keys))
        return elapsed


def check_buffer(fd, home, expected, case):
    os.write(fd, b"\x18!")
    read_until(fd, b"captured<yes>")
    got = (home / "buffer").read_bytes()
    expected = expected.encode()
    if got != expected:
        i = next((i for i, (a, b) in enumerate(zip(got, expected)) if a != b), min(len(got), len(expected)))
        raise ValueError(f"{case}: mismatch at {i}: {got[i:i+20]!r} / {expected[i:i+20]!r}")


def steady(shells, home, rc, content, count, runs):
    # A distinct visible scalar acknowledges exactly one edit. No timer sleep
    # or screen-clear is needed between keystrokes.
    warmup = 16
    chars = "".join(chr(0x4e00 + i) for i in range(count + warmup))
    for motion in (False, True):
        timings = {s: [] for s in shells}
        for i in range(runs):
            for s in shells[i % len(shells):] + shells[:i % len(shells)]:
                with terminal(s, home, home, rc, prompt=PROMPT) as fd:
                    visible(fd, PASTE + content.encode() + END_PASTE)
                    if motion:
                        os.write(fd, HOME + FORWARD)
                    for index, char in enumerate(chars):
                        marker = char.encode()
                        packet = BACK + marker if motion else marker
                        start = time.perf_counter_ns()
                        os.write(fd, packet)
                        read_until(fd, marker)
                        stop = time.perf_counter_ns()
                        if index >= warmup:
                            timings[s].append((stop - start) / 1e6)
                    check_buffer(fd, home, chars[::-1] + content if motion else content + chars, s)
        cells = []
        for values in timings.values():
            values.sort()
            percentiles = [values[math.ceil(q * len(values)) - 1] for q in (.5, .95, .99, 1)]
            cells.append("/".join(f"{ms:.3f}" for ms in percentiles))
        label = "repeated near-head insertion" if motion else "repeated append"
        print(label, *cells, sep=" | ", flush=True)


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("shell", type=Path, nargs="+")
    cli.add_argument("--case", action="append", help="time only this named operation (repeatable)")
    cli.add_argument("--runs", type=int, default=11)
    cli.add_argument("--size", type=int, default=65536, help="approximate initial buffer size in bytes")
    cli.add_argument("--steady", type=int, metavar="EDITS", help="time successive acknowledged edits within each session")
    args = cli.parse_args()
    if args.steady is not None and (not 1 <= args.steady <= 4000 or args.case):
        cli.error("--steady accepts 1 to 4000 edits and its own two workloads")
    if args.runs < 1 or args.size < 0:
        cli.error("use a positive run count and nonnegative size")
    shells = [p.resolve() for p in args.shell]
    overwrite = {s: b"overwrite-mode" in subprocess.run([s, "--noprofile", "--norc", "-c", "bind -l"],
                       stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=True).stdout.splitlines() for s in shells}
    with tempfile.TemporaryDirectory() as scratch:
        root = Path(scratch)
        rc = root / "rc"
        locale = "en_US.UTF-8" if os.uname().sysname == "Darwin" else "C.UTF-8"
        rc.write_text(f"PS1='{PROMPT.decode()}'\nexport LC_ALL={locale}\nHISTFILE=/dev/null\n"
                      "bind '\"\\e[2~\": overwrite-mode'\nbind '\"\\ew\": copy-region-as-kill'\n"
                      "save_line(){ printf %s \"$READLINE_LINE\" > buffer; printf 'captured<%s>' yes >&2; }\n"
                      "bind -x '\"\\C-x!\":save_line'\n")
        content = "first " + ("words " * (args.size // 6)) + "日本 last" + FLAG.decode()
        if args.steady:
            print("operation | " + " | ".join(s.name for s in shells) + " (p50/p95/p99/max ms)", flush=True)
            return steady(shells, root, rc, content, args.steady, args.runs)
        print("operation | " + " | ".join(s.name for s in shells) + " (median/p95 ms)", flush=True)
        for label, keys, expected in cases(content):
            if args.case and label not in args.case: continue
            values = {s: [] for s in shells}
            for i in range(args.runs + 2):
                for s in shells[i % len(shells):] + shells[:i % len(shells)]:
                    if label == "overwrite" and not overwrite[s]: continue
                    elapsed = sample(s, root, rc, content, keys, expected)
                    if i >= 2:
                        values[s].append(elapsed)
            print(label, *(f"{statistics.median(v):.3f}/{sorted(v)[math.ceil(.95*len(v))-1]:.3f}"
                           if v else "n/a" for v in values.values()), sep=" | ", flush=True)


if __name__ == "__main__":
    main()
