"""Benchmark Tonari against GNU Bash with warm caches on a Unix host."""

import argparse
from contextlib import contextmanager
import math
import os
import pty
import select
import signal
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


PROMPT = b"tonari$ "
COMMANDS = {
    "startup": ("-c :", ":"),
    "external": ("external true", "/usr/bin/true"),
    "pipeline": ("two-stage pipeline", "/usr/bin/printf x | /usr/bin/wc -c"),
    "builtins": ("boolean builtins", "true; false; true"),
    "external-series": ("16 external commands in one shell", "; ".join(["/usr/bin/true"] * 16)),
    "loop": ("1000 iterations of a builtin loop", "for ((i=0;i<1000;i++)); do :; done"),
    "redirects": ("100 builtin redirections", ": >/dev/null;" * 100),
    "session": ("session builtins", 'where=.; export where; cd "$where" && pwd >/dev/null && unset where'),
}
CORE = ["prompt", "startup", "external", "pipeline", "builtins", "redirects", "direct"]


def first_prompt(argv, env, prompt=PROMPT, *, after_prompt=None):
    # Keep a controlling PTY for job-control initialization in every shell.
    # The child timestamps the exec separately from the parent's full launch.
    read_stamp, write_stamp = os.pipe()
    try:
        start = time.perf_counter_ns()
        pid, fd = pty.fork()
    except BaseException:
        os.close(read_stamp)
        os.close(write_stamp)
        raise
    if pid == 0:
        try:
            os.write(write_stamp, time.perf_counter_ns().to_bytes(8, "big"))
            os.execve(argv[0], argv, env)
        finally:
            os._exit(125)
    os.close(write_stamp)
    try:
        data = b""
        answered = 0
        deadline = time.monotonic() + 4
        while prompt not in data:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([fd], [], [], remaining)[0]:
                raise RuntimeError(f"prompt timed out for {argv[0]}: {data!r}")
            chunk = os.read(fd, 4096)
            if not chunk:
                raise RuntimeError(f"{argv[0]} exited before its prompt")
            data += chunk
            # Answer terminal cursor-position queries during prompt rendering.
            queries = data.count(b"\x1b[6n")
            for _ in range(queries - answered):
                os.write(fd, b"\x1b[1;1R")
            answered = queries
        end = time.perf_counter_ns()
        stamp = os.read(read_stamp, 8)
        if len(stamp) != 8:
            raise RuntimeError(f"missing exec timestamp for {argv[0]}")
        times = (end - start) / 1e6, (end - int.from_bytes(stamp, "big")) / 1e6
        return times if after_prompt is None else (*times, after_prompt(fd))
    finally:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.close(read_stamp)
        os.close(fd)
        os.waitpid(pid, 0)


@contextmanager
def null_standard_streams(devnull):
    # Configure the launcher once. A direct 16-child series inherits the same
    # standard descriptors as the shell's 16 children, with no timed dup2 calls.
    saved = [os.dup(i) for i in range(3)]
    try:
        for i in range(3):
            os.dup2(devnull.fileno(), i)
        yield
    finally:
        for dest, original in enumerate(saved):
            os.dup2(original, dest)
            os.close(original)


def round_trip(argv, env, repeats=1):
    start = time.perf_counter_ns()
    for _ in range(repeats):
        pid = os.posix_spawn(argv[0], argv, env)
        _, status = os.waitpid(pid, 0)
        if status:
            raise RuntimeError(f"{argv!r} exited with {os.waitstatus_to_exitcode(status)}")
    return (time.perf_counter_ns() - start) / 1e6


def report(samples):
    summaries = {
        name: (statistics.median(times), sorted(times)[math.ceil(0.95 * len(times)) - 1])
        for name, times in samples.items()
    }
    for name, (median, p95) in summaries.items():
        result = f"  {name:8} median {median:7.3f} ms  p95 {p95:7.3f} ms"
        if "Bash" in summaries and name != "Bash":
            baseline, baseline_p95 = summaries["Bash"]
            ratio = median / baseline
            if ratio <= 0.5:
                outcome = "at or below 1/2"
            elif ratio <= 2 / 3:
                outcome = "within 2/3"
            elif ratio < 1:
                outcome = "above 2/3"
            else:
                outcome = "at or above Bash time"
            result += f"  Bash ratio {ratio:.3f} (p95 {p95 / baseline_p95:.3f})  {outcome}"
        print(result)
    if "Tonari" in summaries and "Bash" in summaries:
        return summaries["Tonari"][0] * 3 <= summaries["Bash"][0] * 2
    return True


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("tonari", type=Path)
    cli.add_argument("--bash", type=Path, default=Path("/bin/bash"))
    cli.add_argument("--compare-tonari", action="append", nargs=2, default=[],
                     metavar=("LABEL", "PATH"), help="include another Tonari build (repeatable)")
    cli.add_argument("--compare-shell", action="append", nargs=2, default=[],
                     metavar=("LABEL", "PATH"), help="include a Bash-compatible shell with --noprofile --norc")
    cli.add_argument("--case", dest="cases", action="append", choices=["core", "prompt", *COMMANDS, "direct", "direct-series", "nvm"],
                     help="select workloads (repeatable); core selects the original suite")
    cli.add_argument("--nvm-script", type=Path,
                     help="also measure sourcing this nvm.sh --no-use in an empty NVM_DIR")
    cli.add_argument("--runs", type=int, default=100)
    cli.add_argument("--require-target", action="store_true", help="exit nonzero if any Tonari median exceeds two thirds of Bash")
    args = cli.parse_args()
    within_target = True
    if args.runs < 1:
        cli.error("--runs must be positive")
    if args.nvm_script:
        args.nvm_script = args.nvm_script.resolve()
        if not args.nvm_script.is_file():
            cli.error("--nvm-script must point to nvm.sh")
    if args.cases and "nvm" in args.cases and not args.nvm_script:
        cli.error("--case nvm requires --nvm-script")
    shells = [
        ("Tonari", args.tonari.resolve(), ["--noprofile", "--norc"]),
        ("Bash", args.bash.resolve(), ["--noprofile", "--norc"]),
    ]
    additional = []
    for label, path in args.compare_tonari:
        # Early prototypes predate these flags. Modern builds should see the
        # same startup files and prompt as the Tonari and Bash controls.
        flags = ["--noprofile", "--norc"]
        probe = subprocess.run([Path(path).resolve(), *flags, "-c", ":"],
                               env={"PATH": os.defpath, "LC_ALL": "C"},
                               stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL)
        additional.append((label, path, flags if probe.returncode == 0 else []))
    additional.extend((label, path, ["--noprofile", "--norc"]) for label, path in args.compare_shell)
    for label, path, flags in additional:
        if any(name == label for name, _, _ in shells):
            cli.error(f"duplicate build label: {label}")
        shells.append((label, Path(path).resolve(), flags))

    requested = args.cases or ["core", "session", "external-series", "loop"]
    if not args.cases and args.nvm_script:
        requested.append("nvm")
    cases = dict.fromkeys(case for item in requested for case in (CORE if item == "core" else [item]))
    with tempfile.TemporaryDirectory() as temp, open(os.devnull, "r+b") as devnull:
        env = {
            "PATH": "/usr/bin:/bin", "HOME": temp, "HISTFILE": "/dev/null",
            "TERM": "xterm-256color", "PS1": PROMPT.decode(), "LC_ALL": "C",
            "BASH_SILENCE_DEPRECATION_WARNING": "1",
        }
        if args.nvm_script:
            env["NVM_DIR"] = os.path.join(temp, "nvm")
            os.mkdir(env["NVM_DIR"])
        for case in cases:
            if case in ("direct", "direct-series"):
                label = "direct /usr/bin/true" if case == "direct" else "direct 16 sequential /usr/bin/true"
                subjects = [("Direct", "/usr/bin/true", [])]
            elif case == "nvm":
                label = "source nvm.sh --no-use"
                subjects = shells
            else:
                label = "first prompt" if case == "prompt" else COMMANDS[case][0]
                subjects = shells
                if case == "external-series":
                    subjects = [*shells, ("Direct", "/usr/bin/true", [])]
            print(label, flush=True)
            samples = {name: [] for name, _, _ in subjects}
            exec_samples = {name: [] for name, _, _ in subjects}
            with null_standard_streams(devnull):
                for i in range(5 + args.runs):
                    offset = i % len(subjects)
                    for name, binary, flags in subjects[offset:] + subjects[:offset]:
                        argv = [binary, *flags]
                        if case == "prompt":
                            value, from_exec = first_prompt(argv, env)
                            if i >= 5:
                                exec_samples[name].append(from_exec)
                        else:
                            if case == "nvm":
                                argv.extend(["-c", '. "$1" --no-use && declare -F nvm >/dev/null',
                                             "probe", args.nvm_script])
                            if case in COMMANDS and name != "Direct":
                                argv.extend(["-c", COMMANDS[case][1]])
                            repeats = 16 if case == "direct-series" or (name == "Direct" and case == "external-series") else 1
                            value = round_trip(argv, env, repeats)
                        if i >= 5:
                            samples[name].append(value)
            within_target = report(samples) and within_target
            if case == "prompt":
                print("  exec to first prompt (same samples)")
                within_target = report(exec_samples) and within_target
    for name, binary, _ in shells:
        print(f"{name:8} {binary.stat().st_size:>9} bytes")

    if args.require_target and not within_target:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
