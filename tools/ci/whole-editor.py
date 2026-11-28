"""Observe PTY arrival time for the first byte and for the correct visible edit and caret.

Install pyte==0.8.2 in a Python venv, then run
python tools/ci/whole-editor.py --shell main=/path/to/main --shell candidate=/path/to/tonari
--shell bash=/path/to/bash --runs 100 --output editor.jsonl
"""
import argparse
from contextlib import contextmanager
import fcntl
import json
import math
import os
from pathlib import Path
import pty
import select
import signal
import struct
import sys
import tempfile
import termios
import time

import pyte


PROMPT = "ED> "
RIGHT = "TAIL"
CASES = ("ascii", "ime", "multiline", "cancel", "split-home", "tab", "recall", "search-cancel")
KEY_CASES = ("single-key", "single-emoji", "mid-insert", "mid-delete", "mid-backspace", "mid-left")
PICKER_CASES = ("picker-miss", "picker-cancel", "picker-match")


@contextmanager
def terminal(binary, home, rows=24, cols=80):
    pid, fd = pty.fork()
    if pid == 0:
        try:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
            os.chdir(home)
            os.execve(binary, [binary, "--noprofile", "--rcfile", str(home / "rc"), "-i"],
                      dict(HOME=str(home), PATH="/usr/bin:/bin", INPUTRC="/dev/null",
                           TERM="xterm", LC_ALL="en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"))
        finally:
            os._exit(125)
    try:
        os.set_blocking(fd, False)
        yield Display(fd, rows, cols)
    finally:
        os.kill(pid, signal.SIGKILL)
        os.close(fd)
        os.waitpid(pid, 0)


class Display:
    def __init__(self, fd, rows, cols):
        self.fd = fd
        self.screen = pyte.Screen(cols, rows)
        self.stream = pyte.ByteStream(self.screen)

    @property
    def lines(self):
        # Read cell data directly. During redraw a partial frame may contain
        # an isolated wide-character continuation cell with an empty string.
        return ["".join(self.screen.buffer[row][x].data for x in range(self.screen.columns))
                for row in range(self.screen.lines)]

    def caret_text(self):
        # pyte omits the second half of wide cells in display[]. Slice in terminal
        # cells first. The editor can leave an unused cell before wrapping.
        s = self.screen
        y, x = s.cursor.y, s.cursor.x
        before = "".join("".join(s.buffer[row][i].data for i in range(s.columns)).rstrip()
                         for row in range(y))
        before += "".join(s.buffer[y][i].data for i in range(x))
        after = "".join(s.buffer[y][i].data for i in range(x, s.columns)).rstrip()
        after += "".join(line.rstrip() for line in self.lines[y + 1:])
        return before, after

    def adjacent(self, left, right=""):
        before, after = self.caret_text()
        return before.endswith(left) and after.startswith(right)

    def edit_complete(self, value, tail, marker="EDITDONE"):
        # The head may have scrolled or been windowed. Every cell still on
        # screen must be a contiguous part of the expected text, on the correct
        # side of the caret. A terminal split inserts no character in the buffer.
        before, after = self.caret_text()
        visible = before.removeprefix(PROMPT).removeprefix("…")
        expected = value.replace("\n", "")
        if not (visible.endswith(marker) and expected.endswith(visible) and after == tail):
            return False
        if before.startswith(PROMPT) and not before.startswith(PROMPT + "…"):
            return visible == expected
        # Scrolled or windowed input still fills multiple rows in these fixtures.
        # A marker and caret alone can be correct while most input is missing.
        return len(visible) >= min(len(expected), self.screen.columns)

    def observe(self, packet, ready, timeout=2.0):
        # Timestamp reads before interpreting ANSI. Drain the master and feed the
        # terminal model after the burst, so parser cost does not delay those reads.
        start = time.perf_counter_ns()
        deadline = time.monotonic() + timeout
        first = None
        events = []
        while time.monotonic() < deadline:
            left = max(0, deadline - time.monotonic())
            quiet = min(left, 0.003) if events else left
            r, w, _ = select.select([self.fd], [self.fd] if packet else [], [], quiet)
            if r:
                data = os.read(self.fd, 65536)
                if not data:
                    raise EOFError("shell exited")
                now = time.perf_counter_ns()
                if first is None:
                    first = now
                events.append((now, data))
            if w:
                packet = packet[os.write(self.fd, packet[:1024]):]
            if not r and not w and events:
                reached = None
                for now, data in events:
                    self.stream.feed(data)
                    if ready():
                        if reached is None:
                            reached = now
                    else:
                        reached = None
                events.clear()
                if not packet and reached is not None:
                    return tuple((v - start) / 1e6 for v in (first, reached))
        raise TimeoutError((self.lines, self.screen.cursor.x, self.screen.cursor.y))


def rc_file(home, extra=""):
    (home / "rc").write_text(
        "PS1='ED> '\nunset HISTFILE\nset +o history\n"
        "snap() { printf '%s' \"$READLINE_LINE\" > draft; READLINE_LINE=; READLINE_POINT=0; printf '\\nSNAP-DONE\\n'; }\n"
        "bind -x '\"\\C-t\":snap'\n" + extra)


def sample(path, home, case):
    marker = "EDITDONE"
    dims = (6, 36) if case == "split-home" else (24, 80)
    with terminal(path, home, *dims) as d:
        d.observe(b"", lambda: d.adjacent(PROMPT))
        if case in KEY_CASES:
            left = "abc" if case.startswith("single") else "ab" * 2048 + "XYZ"
            d.observe((left + RIGHT).encode(), lambda: d.adjacent(left[-3:] + RIGHT))
            d.observe(b"\x02" * len(RIGHT), lambda: d.adjacent(left[-3:], RIGHT))
            if case in ("single-key", "single-emoji", "mid-insert"):
                char = "😀" if case == "single-emoji" else "@"
                key, left, right = char.encode(), left + char, RIGHT
            elif case == "mid-delete":
                key, right = b"\x04", RIGHT[1:]
            elif case == "mid-backspace":
                key, left, right = b"\x7f", left[:-1], RIGHT
            else:
                key, left, right = b"\x02", left[:-1], left[-1] + RIGHT
            result = d.observe(key, lambda: d.edit_complete(left, right, left[-3:]))
            value = left + right
        elif case in PICKER_CASES:
            value = "pick unit"
            d.observe(b"pick \t\t", lambda: any("filter>" in line for line in d.lines))
            if case == "picker-cancel":
                result = d.observe(b"ZZZZZ\x07", lambda: d.adjacent(PROMPT + value))
            else:
                if case == "picker-miss":
                    result = d.observe(b"ZZZZZ", lambda: "(no matches)" in d.lines[1])
                else:
                    result = d.observe(b"unit0001", lambda: "> unit0001-" in d.lines[1])
                d.observe(b"\x07", lambda: d.adjacent(PROMPT + value))
        elif case == "tab":
            value = "zzuniquecmd "
            d.observe(b"zzuni", lambda: d.adjacent(PROMPT + "zzuni"))
            result = d.observe(b"\t", lambda: d.adjacent(PROMPT + value))
        elif case == "recall":
            value = "printf HISTMARK"
            result = d.observe(b"\x10", lambda: d.adjacent(PROMPT + value))
        elif case == "search-cancel":
            value = "DRAFT"
            d.observe(b"DRAFT\x02", lambda: d.adjacent(PROMPT + "DRAF", "T"))
            d.observe(b"\x12HISTMARK", lambda: any("search" in row and "HISTMARK" in row
                                                    for row in d.lines))
            result = d.observe(b"\x07", lambda: d.adjacent(PROMPT + "DRAF", "T"))
        elif case in ("split-home", "cancel"):
            value = "HEAD" + "abcdefghij" * 32 + marker
            d.observe(value.encode(), lambda: d.adjacent(marker))
            if case == "cancel":
                result = d.observe(b"\x03", lambda: d.adjacent(PROMPT))
                value = ""
            else:
                result = d.observe(b"\x01", lambda: d.adjacent(PROMPT, "HEADabcdefghij"))
        else:
            d.observe(RIGHT.encode() + b"\x01", lambda: d.adjacent(PROMPT, RIGHT))
            if case == "ascii":
                value = "abcdefghijklmnop" * 256 + marker
            elif case == "ime":
                value = "平仮名かなカナ" * 164 + marker
            else:
                value = "".join(f"row-{i:03d} あいうえお\n" for i in range(48)) + marker
            packet = value.encode()
            if case == "multiline":
                packet = b"\x1b[200~" + packet + b"\x1b[201~"
            result = d.observe(packet, lambda: d.edit_complete(value, RIGHT))
            value += RIGHT
        # Check all committed characters through the public bind -x interface.
        # This check is deliberately outside the timed interval.
        d.observe(b"\x14", lambda: any("SNAP-DONE" in line for line in d.lines))
        if (home / "draft").read_text() != value:
            raise AssertionError("submitted editor buffer differs from the input")
        return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--shell", action="append", required=True, metavar="LABEL=PATH")
    parser.add_argument("--case", action="append", choices=CASES + PICKER_CASES + KEY_CASES,
                        help="picker cases apply to Tonari only")
    parser.add_argument("--require", action="append", default=[], metavar="LABEL",
                        help="exit unsuccessfully if a case for this shell fails")
    parser.add_argument("--runs", type=int, default=100)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")
    shells = [(label, str(Path(path).resolve())) for label, path in
              (item.split("=", 1) for item in args.shell)]
    labels = {label for label, _ in shells}
    if len(labels) != len(shells) or set(args.require) - labels:
        parser.error("shell labels must be unique and include all required labels")
    failures = []
    cases = args.case or CASES
    with tempfile.TemporaryDirectory() as root, args.output.open("w") as output:
        home = Path(root)
        for case in cases:
            setup = ""
            if case in PICKER_CASES:
                words = " ".join(f"unit{i:04d}-" + "alpha-beta-" * 5 for i in range(2048))
                setup = f"bind 'set completion-ignore-case on'\ncomplete -W '{words}' pick\n"
            if case == "tab":
                setup = "zzuniquecmd() { :; }\n"
            if case in ("recall", "search-cancel"):
                setup = "set -o history\n" + "".join(f"history -s 'printf row-{i}'\n" for i in range(256))
                setup += "history -s 'printf HISTMARK'\n"
            rc_file(home, setup)
            samples = {label: [] for label, _ in shells}
            failed = {}
            for i in range(args.runs + 1):
                shift = i % len(shells)
                for label, path in shells[shift:] + shells[:shift]:
                    if label in failed:
                        continue
                    try:
                        result = sample(path, home, case)
                    except (TimeoutError, AssertionError, EOFError, OSError) as e:
                        failed[label] = str(e)
                        continue
                    if i:
                        samples[label].append(result)
            for label, values in samples.items():
                record = dict(case=case, shell=label, samples=values, error=failed.get(label))
                print(json.dumps(record, ensure_ascii=False), file=output, flush=True)
                if label in failed:
                    if label in args.require:
                        failures.append(f"{label}/{case}")
                    print(case, label, "FAILED", failed[label], flush=True)
                else:
                    for metric, data in zip(("first", "visible+caret"), zip(*values)):
                        ordered = sorted(data)
                        row = [ordered[math.ceil(q * len(ordered)) - 1] for q in (0.5, .95, .99, 1)]
                        print(case, label, metric, "p50/p95/p99/max ms", *(f"{v:.3f}" for v in row),
                              flush=True)

    if failures:
        raise SystemExit("failed required cases: " + ", ".join(failures))


if __name__ == "__main__":
    main()
