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


def timed_keys(fd, keys, marker, initial=b""):
    start = time.perf_counter_ns()
    os.write(fd, keys)
    read_until(fd, marker, initial=initial)
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


def review_completion(shell, home, bindir, rc, metrics):
    def settled(fd, label, line, expected):
        os.write(fd, line)
        read_until(fd, line)
        # The bound key prints READLINE_LINE after Tab. This also settles an
        # ambiguous completion whose first Tab produces no visible extension.
        marker = b"<draft>" + expected + b"</draft>"
        start = time.perf_counter_ns()
        os.write(fd, b"\t\x14")
        output = read_until(fd, marker)
        metrics[label].append((time.perf_counter_ns() - start) / 1e6)
        read_until(fd, PROMPT, initial=output.split(marker, 1)[1])

    for count in (1, 4):
        path = os.pathsep.join([str(bindir)] * count)
        with terminal(shell, home, path, rc, prompt=PROMPT) as fd:
            if count == 1:
                line = b": files/mprobe"
                settled(fd, "folded ambiguous", line, line)
                settled(fd, "folded unique", b": files/zznee", b": files/zzneedleResolved ")
            for i in (1, 2):
                settled(fd, f"PATH x{count}, Tab {i}", b"zznee", b"zzneedleResolved ")


def delayed_callback(shell, home, bindir, rc, metrics):
    with terminal(shell, home, bindir, rc, prompt=PROMPT) as fd:
        os.write(fd, b"declared ma\t")
        read_until(fd, b"helper<wake>")
        start = time.perf_counter_ns()
        os.write(fd, "雪".encode())
        read_until(fd, "雪".encode())
        metrics["IME during helper"].append((time.perf_counter_ns() - start) / 1e6)
        elapsed = timed_keys(fd, b"\x7f@", b"@")
        metrics["Backspace then @"].append(elapsed)
        os.write(fd, b"\x0c")
        read_until(fd, PROMPT + b"declared match @")


def scan_keys(shell, home, bindir, rc, metrics):
    line = b": files/needle"
    for delayed in (False, True):
        with terminal(shell, home, bindir, rc, prompt=PROMPT) as fd:
            os.write(fd, line)
            read_until(fd, line)
            if delayed:
                os.write(fd, b"\t")
                time.sleep(.001)
                metrics["key sent 1 ms after Tab"].append(timed_keys(fd, b"@", b"@"))
                os.write(fd, b"\x0c")
                read_until(fd, PROMPT + line + b"-target @")
            else:
                metrics["Tab to completed filename"].append(timed_keys(fd, b"\t", b"-target "))


def long_line(shell, home, bindir, rc, metrics):
    from bench_editor_input import FLAG, visible

    with terminal(shell, home, bindir, rc, prompt=PROMPT) as fd:
        value = b"x" * 65536 + FLAG
        visible(fd, b"\x1b[200~v=" + value + b"\x1b[201~")
        start = time.perf_counter_ns()
        os.write(fd, b"@")
        output = read_until(fd, b"@")
        metrics["append after 64 KiB (ms)"].append((time.perf_counter_ns() - start) / 1e6)
        metrics["append PTY output (bytes)"].append(len(output))
        os.write(fd, b"\r")
        read_until(fd, b"\x1b[?2004h")
        os.write(fd, b'printf %s "$v" > value\r')
        read_until(fd, b"\x1b[?2004h")
        if (home / "value").read_bytes() != value + b"@":
            raise ValueError(f"{shell}: incorrect long-line edit")


def reused_words(shell, home, bindir, rc, metrics):
    with terminal(shell, home, bindir, rc, prompt=PROMPT) as fd:
        for i in range(12):
            line = f"fixed option-{(871 + i * 719) % 10000:05d}".encode()
            os.write(fd, line)
            read_until(fd, line)
            marker = f"@{i:02d}X".encode()
            elapsed = timed_keys(fd, b"\t", b"-flag ")
            os.write(fd, marker)
            os.write(fd, b"\x0c")
            read_until(fd, PROMPT + line + b"-flag " + marker)
            os.write(fd, b"\r")
            read_until(fd, PROMPT)
            stage = "first" if i == 0 else "second" if i == 1 else "warm"
            metrics[f"static reused, {stage}"].append(elapsed)


def duplicate_word_listing(shell, home, bindir, rc):
    line = b"dup pick-"
    with terminal(shell, home, bindir, rc, prompt=PROMPT) as fd:
        os.write(fd, line)
        read_until(fd, line)
        start = time.perf_counter_ns()
        os.write(fd, b"\x1b?@")
        output = read_until(fd, b"@")
        elapsed = (time.perf_counter_ns() - start) / 1e6
        if re.findall(rb"pick-[zam]", output) != [b"pick-z", b"pick-a", b"pick-m"]:
            raise ValueError(f"{shell}: incorrect ordered literal list")
        return elapsed


def listing(shell, home, bash, expected, prefix):
    rc = home / ("bashrc" if shell == bash else "rc")
    line = b": " + prefix
    with terminal(shell, home, home, rc, prompt=PROMPT) as fd:
        os.write(fd, line)
        read_until(fd, line)
        os.write(fd, b"\t")
        read_until(fd, b"\a" if shell == bash else PROMPT + line)
        start = time.perf_counter_ns()
        os.write(fd, b"\t@" if shell == bash else b"\x1b?@")
        output = read_until(fd, max(expected))
        end = output.index(max(expected)) + len(max(expected))
        output = output[:end] + read_until(fd, b"@", initial=output[end:])
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
            line = PROMPT
            for char in word:
                key = char.encode()
                metrics[case].append(timed_keys(fd, key, line + key, initial=line))
                line += key
        os.write(fd, b"\x15")
        read_until(fd, b"\r\x1b[J" + PROMPT)
        start = time.perf_counter_ns()
        os.write(fd, b"q")
        output = read_until(fd, PROMPT + b"q", initial=PROMPT)
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
    mode.add_argument("--reply-list", action="store_true", help="measure ordered callback replies")
    mode.add_argument("--word-list", action="store_true", help="measure a registered 10,000-word completion")
    mode.add_argument("--review-completion", action="store_true", help="measure folded filenames and repeated PATH directories")
    mode.add_argument("--command-cache", action="store_true", help="time four command-name Tabs in a single session")
    mode.add_argument("--branches", action="store_true", help="complete one filename across 12 independent directories")
    mode.add_argument("--scan-key", action="store_true", help="distinguish scan completion from a key arriving 1 ms into it")
    mode.add_argument("--project", action="store_true", help="time completed abbreviated paths and project history arguments")
    mode.add_argument("--long-line", action="store_true", help="append to a 64 KiB paste and verify its value")
    mode.add_argument("--callback", action="store_true", help="queue a key behind a callback that waits 100 ms")
    mode.add_argument("--reuse-words", action="store_true", help="repeated queries to one registered literal list")
    mode.add_argument("--listing", action="store_true", help="measure the complete candidate listing")
    parser.add_argument("--nosort", action="store_true", help="preserve declaration order in the literal list fixture")
    parser.add_argument("--require-target", action="store_true", help="check the 2/3 median target for cases shared with Bash")
    args = parser.parse_args()
    if args.nosort and not (args.word_list or args.reuse_words):
        parser.error("--nosort requires --word-list or --reuse-words")
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
        if not (args.review_completion or args.command_cache or args.reply_list or args.word_list or args.reuse_words or args.callback or args.long_line or args.branches):
            digits = len(str(args.files))
            for i in range(args.files):
                (bindir / f"item-{i:0{digits}d}.dat").touch(mode=0o755)
        if args.review_completion:
            for i in range(args.files):
                (bindir / f"mprobe{chr(65 + i % 26)}{i:05d}").touch()
            (bindir / "zzneedleResolved").touch(mode=0o755)
            rc = home / "rc"
            rc.write_text(f"PS1='{PROMPT.decode()}'\nHISTFILE=/dev/null\n"
                          "bind 'set completion-ignore-case on'\n"
                          'snapshot() { printf "\\n<draft>%s</draft>\\n" "$READLINE_LINE"; READLINE_LINE=; READLINE_POINT=0; }\n'
                          "bind -x '\"\\C-t\":snapshot'\n")
        elif args.command_cache:
            directories = [home / f"bin-{i}" for i in range(6)]
            for i, directory in enumerate(directories):
                directory.mkdir()
                for j in range(338):
                    (directory / f"tool-{i}-{j}").touch(mode=0o755)
            commands = (b"quartz-first", b"topaz-next", b"onyx-last", b"quartz-first")
            for name, directory in zip(commands, directories[::2]):
                (directory / name.decode()).touch(mode=0o755)
            command_path = os.pathsep.join(map(str, directories))
        elif args.branches:
            if bash: parser.error("--branches uses Tonari abbreviated paths")
            basedir = home / "src"
            for i in range(12):
                directory = basedir / f"module-{i:02}" / "kernels"
                directory.mkdir(parents=True)
                for j in range(2048):
                    (directory / f"padding-{j:04}").touch()
            (directory / "needle-target").touch()
            rc = home / "rc"
            rc.write_text(f"PS1='{PROMPT.decode()}'\nHISTFILE=/dev/null\n")
        elif args.scan_key:
            (bindir / "needle-target").touch()
            rc = home / "rc"
            rc.write_text(f"PS1='{PROMPT.decode()}'\nHISTFILE=/dev/null\n")
        elif args.project:
            if bash:
                parser.error("--project measures Tonari extensions")
            (home / ".git").mkdir()
            branch = home / "source/backend/kernels"
            branch.parent.mkdir(parents=True)
            bindir.rename(branch)
            bindir = branch
            (branch / "fir-filter").touch()
            (home / "build/device-release").mkdir(parents=True)
            journal = home / ".local/state/tonari/history"
            journal.parent.mkdir(parents=True)
            with journal.open("w") as f:
                for i in range(100000):
                    f.write(f"1\t1\t0\t{home}\t: padding-{i:06d}\n")
                f.write(f"1\t1\t0\t{home}\tcmake --build build/device-release\n")
                f.write(f"1\t1\t0\t{home}\trun --model mistral-large\n")
            rc = home / "rc"
            rc.write_text(f"PS1='{PROMPT.decode()}'\n")
        elif args.long_line:
            rc = home / "rc"
            rc.write_text(f"PS1='{PROMPT.decode()}'\nHISTSIZE=0\nHISTFILE=/dev/null\n")
        elif args.reply_list:
            numbers = [(i * 73) % 10_000 for i in range(10_000)]
            words = " ".join(f"option-{i:05d}" for i in numbers + numbers[::32])
            rc = home / "rc"
            rc.write_text(f"PS1='{PROMPT.decode()}'\nHISTFILE=/dev/null\nchoices=({words})\n"
                          '_reply() { COMPREPLY=("${choices[@]}"); }\n'
                          'demo() { :; }\ncomplete -o nosort -F _reply demo\n')
        elif args.callback:
            rc = home / "rc"
            rc.write_text(f"PS1='{PROMPT.decode()}'\n"
                          "_slow() { printf 'helper<%s>' wake >&2; /bin/sleep 0.1; COMPREPLY=(match); }\n"
                          "complete -F _slow demo\n"
                          "complete -o typeahead -F _slow declared 2>/dev/null || complete -F _slow declared\n")
        elif args.word_list or args.reuse_words:
            order = " -o nosort" if args.nosort else ""
            order_indices = range(10_000)
            if args.nosort:
                mixed = [(i * 73) % 10_000 for i in order_indices]
                order_indices = mixed + mixed[::128]
            words = " ".join(f"option-{i:05d}-flag" for i in order_indices)
            rc = home / "rc"
            rc.write_text(f"PS1='{PROMPT.decode()}'\nHISTSIZE=0\nHISTFILE=/dev/null\nchoices='{words}'\n"
                          'fixed() { :; }\n'
                          f'complete{order} -W "$choices" fixed\ncomplete -W \'$choices\' dynamic\n')
            if args.word_list and args.nosort:
                duplicates = " ".join(["pick-z", "pick-a", "pick-m"] * 3334)
                with rc.open("a") as f:
                    f.write(f"complete -o nosort -W '{duplicates}' dup\n")
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
        if args.callback or args.long_line:
            with rc.open("a") as settings:
                locale = "en_US.UTF-8" if os.uname().sysname == "Darwin" else "C.UTF-8"
                settings.write(f"export LC_ALL={locale}\n")
        results = {s: defaultdict(list) for s in shells}
        for i in range(3 + args.runs):
            shift = i % len(shells)
            for shell in shells[shift:] + shells[:shift]:
                metrics = results[shell] if i >= 3 else defaultdict(list)
                if args.review_completion:
                    review_completion(shell, home, bindir, rc, metrics)
                    continue
                if args.command_cache:
                    with terminal(shell, home, command_path, prompt=PROMPT) as fd:
                        for order, name in enumerate(commands, 1):
                            prefix = name[:-2]
                            os.write(fd, prefix)
                            read_until(fd, prefix)
                            metrics[f"Tab {order}"].append(timed_keys(fd, b"\t", name + b" ", initial=prefix))
                            if order < len(commands):
                                os.write(fd, b"\x03")
                                read_until(fd, PROMPT)
                    continue
                if args.branches:
                    with terminal(shell, home, bindir, rc, prompt=PROMPT) as fd:
                        line = b": s/mod/k/needle"
                        os.write(fd, line)
                        read_until(fd, line)
                        metrics["12 directories, one match"].append(timed_keys(fd, b"\t", b"module-11/kernels/needle-target "))
                    continue
                if args.scan_key:
                    scan_keys(shell, home, bindir, rc, metrics)
                    continue
                if args.project:
                    with terminal(shell, home, bindir, rc, prompt=PROMPT) as fd:
                        for label, line, result in [
                            ("abbreviated path", b": s/b/k/fir", b": source/backend/kernels/fir-filter "),
                            ("cross-command path", b"ctest --test-dir dev", b"ctest --test-dir build/device-release/"),
                            ("empty option value", b"run --model ", b"run --model mistral-large "),
                        ]:
                            os.write(fd, line)
                            read_until(fd, PROMPT + line, initial=PROMPT)
                            metrics[label].append(timed_keys(fd, b"\t", PROMPT + result))
                            os.write(fd, b"\x15")
                            read_until(fd, b"\r\x1b[J" + PROMPT)
                    continue
                if args.long_line:
                    long_line(shell, home, bindir, rc, metrics)
                    continue
                if args.reply_list:
                    line = b"demo opt"
                    with terminal(shell, home, bindir, rc, prompt=PROMPT) as fd:
                        os.write(fd, line)
                        read_until(fd, line)
                        elapsed = timed_keys(fd, b"\t", b"ion-0")
                        os.write(fd, b"@\x0c")
                        read_until(fd, PROMPT + line + b"ion-0@")
                    metrics["ordered callback, 10000 replies"].append(elapsed)
                    continue
                if args.callback:
                    metrics["Tab then key, 100 ms callback"].append(completion(shell, home, bindir, rc, b"demo ma", b"tch "))
                    metrics["Tab then key, declared helper"].append(completion(shell, home, bindir, rc, b"declared ma", b"tch "))
                    delayed_callback(shell, home, bindir, rc, metrics)
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
                    if args.nosort and shell != bash:
                        metrics["display 10002 entries as 3 names"].append(duplicate_word_listing(shell, home, bindir, rc))
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
        print("shell | case | samples | median / p95 (ms unless labeled bytes)")
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
