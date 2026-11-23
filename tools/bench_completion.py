"""Measure keyboard-to-PTY latency for completion and warm history suggestions."""

import argparse
from collections import defaultdict
import math
import os
from pathlib import Path
import re
import statistics
import tempfile
import time

from bench_history import PROMPT, read_until, terminal


def timed_keys(fd, keys, marker):
    start = time.perf_counter_ns()
    os.write(fd, keys)
    read_until(fd, marker)
    return (time.perf_counter_ns() - start) / 1e6


def completion(shell, home, bindir, rc, line, suffix=b"-0"):
    with terminal(shell, home, bindir, rc, prompt=PROMPT) as fd:
        os.write(fd, line)
        read_until(fd, line)
        elapsed = timed_keys(fd, b"\t@", b"@")
        # Readline and Tonari use different incremental redraws. Force a full
        # redraw outside the timed interval to verify that Tab preceded '@'.
        os.write(fd, b"\x0c")
        read_until(fd, PROMPT + line + suffix + b"@")
        return elapsed


def reused_words(shell, home, bindir, rc, metrics):
    with terminal(shell, home, bindir, rc, prompt=PROMPT) as fd:
        for i in range(12):
            line = f"fixed option-{(871 + i * 719) % 10000:05d}".encode()
            os.write(fd, line)
            read_until(fd, line)
            marker = f"@{i:02d}X".encode()
            elapsed = timed_keys(fd, b"\t" + marker, marker)
            os.write(fd, b"\x0c")
            read_until(fd, PROMPT + line + b"-flag " + marker)
            os.write(fd, b"\r")
            read_until(fd, PROMPT)
            stage = "first" if i == 0 else "second" if i == 1 else "warm"
            metrics[f"static reused, {stage}"].append(elapsed)


def listing(shell, home, bash, expected, prefix):
    rc = home / ("bashrc" if shell == bash else "rc")
    line = b": " + prefix
    with terminal(shell, home, home, rc, prompt=PROMPT) as fd:
        os.write(fd, line)
        read_until(fd, line)
        os.write(fd, b"\t")
        read_until(fd, b"\a" if shell == bash else PROMPT + line)
        start = time.perf_counter_ns()
        os.write(fd, b"\t@")
        output = read_until(fd, b"@")
        elapsed = (time.perf_counter_ns() - start) / 1e6
        if set(re.findall(rb"item-\d+\.dat", output)) != expected:
            raise ValueError(f"incomplete listing from {shell}")
        os.write(fd, b"\x0c")
        read_until(fd, PROMPT + line + b"@")
        return elapsed


def typing(shell, home, bindir, metrics):
    with terminal(shell, home, bindir) as fd:
        os.write(fd, b"p")
        read_until(fd, b"\x1b[2mrintf entry-099999")
        for case, word in [("recent hint", "printf entry-"), ("no hint", "zebra42雪abc")]:
            os.write(fd, b"\x15")
            read_until(fd, b"\r\x1b[J" + PROMPT)
            line = bytearray(PROMPT)
            for char in word:
                key = char.encode()
                line.extend(key)
                metrics[case].append(timed_keys(fd, key, line))
        os.write(fd, b"\x15")
        read_until(fd, b"\r\x1b[J" + PROMPT)
        start = time.perf_counter_ns()
        os.write(fd, b"q")
        output = read_until(fd, PROMPT + b"q")
        metrics["oldest hint: typed key"].append((time.perf_counter_ns() - start) / 1e6)
        hint = b"\x1b[2muartz oldest"
        if hint not in output:
            # The marker may straddle two PTY reads.
            read_until(fd, hint, initial=output)
        metrics["oldest hint: visible suggestion"].append((time.perf_counter_ns() - start) / 1e6)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shell", nargs="+", type=Path, help="Tonari executable(s)")
    parser.add_argument("--bash", type=Path, help="Bash reference for the Tab cases")
    parser.add_argument("--files", type=int, default=10000, choices=[10000, 100000], help="executable files in the directory and PATH fixture")
    parser.add_argument("--runs", type=int, default=30)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--word-list", action="store_true", help="measure a registered 10,000-word completion")
    mode.add_argument("--callback", action="store_true", help="queue a key behind a callback that waits 100 ms")
    mode.add_argument("--reuse-words", action="store_true", help="repeated queries to one registered literal list")
    mode.add_argument("--listing", action="store_true", help="measure the second Tab and full listing")
    parser.add_argument("--require-target", action="store_true", help="check the 2/3 median target for cases shared with Bash")
    args = parser.parse_args()
    if args.require_target and not args.bash:
        parser.error("--require-target requires --bash")
    if args.runs < 1:
        parser.error("runs must be positive")
    shells = [p.resolve() for p in args.shell]
    bash = args.bash.resolve() if args.bash else None
    if bash:
        shells.append(bash)
    with tempfile.TemporaryDirectory() as scratch:
        home = Path(scratch)
        bindir = home / "files"
        bindir.mkdir()
        if not (args.word_list or args.reuse_words or args.callback):
            digits = len(str(args.files))
            for i in range(args.files):
                (bindir / f"item-{i:0{digits}d}.dat").touch(mode=0o755)
        if args.callback:
            rc = home / "rc"
            rc.write_text(f"PS1='{PROMPT.decode()}'\n"
                          "_slow() { /bin/sleep 0.1; COMPREPLY=(match); }\ncomplete -F _slow demo\n")
        elif args.word_list or args.reuse_words:
            words = " ".join(f"option-{i:05d}-flag" for i in range(10_000))
            rc = home / "rc"
            rc.write_text(f"PS1='{PROMPT.decode()}'\nHISTSIZE=0\nHISTFILE=/dev/null\nchoices='{words}'\n"
                          'fixed() { :; }\ncomplete -W "$choices" fixed\ncomplete -W \'$choices\' dynamic\n')
        elif args.listing:
            expected = {p.name.encode() for p in bindir.iterdir()}
            few = home / "few"
            few.mkdir()
            small = {name for name in expected if name.startswith(b"item-0000")}
            for name in sorted(small):
                (few / name.decode()).touch(mode=0o755)
            for folder in (few, bindir):
                prompt = f"PS1='{PROMPT.decode()}'\n"
                (folder / "rc").write_text(prompt)
                (folder / "bashrc").write_text(prompt + "bind 'set completion-query-items 0'\n"
                                               "bind 'set page-completions off'\n")
            cases = [(few, small, b"item-0000"), (bindir, expected, b"item-0")]
        else:
            typing_home = home / "typing"
            journal = typing_home / ".local/state/tonari/history"
            journal.parent.mkdir(parents=True)
            with journal.open("w") as f:
                f.write("1\t1\t0\t/tmp\tquartz oldest\n")
                for i in range(1, 100_000):
                    f.write(f"1\t1\t0\t/tmp\tprintf entry-{i:06d}\n")
            rc = home / "rc"
            rc.write_text(f"PS1='{PROMPT.decode()}'\ncomplete -f demo\n")
        results = {s: defaultdict(list) for s in shells}
        for i in range(3 + args.runs):
            shift = i % len(shells)
            for shell in shells[shift:] + shells[:shift]:
                metrics = results[shell] if i >= 3 else defaultdict(list)
                if args.callback:
                    metrics["Tab then key, 100 ms callback"].append(completion(shell, home, bindir, rc, b"demo ma", b"tch "))
                    continue
                if args.reuse_words:
                    reused_words(shell, home, bindir, rc, metrics)
                    continue
                if args.word_list:
                    for label, line, suffix in [
                        ("static, all", b"fixed option", b"-0"),
                        ("static, unique", b"fixed option-09876", b"-flag "),
                        ("dynamic, unique", b"dynamic option-09876", b"-flag "),
                    ]:
                        metrics[label].append(completion(shell, home, bindir, rc, line, suffix))
                    continue
                if args.listing:
                    for folder, names, prefix in cases:
                        elapsed = listing(shell, folder, bash, names, prefix)
                        metrics[f"list {len(names)}"].append(elapsed)
                    continue
                if shell != bash:
                    typing(shell, typing_home, bindir, metrics)
                for case, line in [("files", b": files/item"), ("complete -f", b"demo files/item"),
                                   ("PATH", b"item")]:
                    metrics[case].append(completion(shell, home, bindir, rc, line))
        print("shell | case | samples | median / p95 (ms)")
        for shell, metrics in results.items():
            for case, values in metrics.items():
                p95 = sorted(values)[math.ceil(.95 * len(values)) - 1]
                print(f"{shell} | {case} | {len(values)} | {statistics.median(values):.3f} / {p95:.3f}")

        if args.require_target:
            for shell in shells:
                if shell != bash and any(
                    statistics.median(results[shell][case]) * 3 > statistics.median(baseline) * 2
                    for case, baseline in results[bash].items()
                ):
                    raise SystemExit(1)


if __name__ == "__main__":
    main()
