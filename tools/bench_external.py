"""Paired measurements of upstream unixwin, fumiyas and luish workloads."""
import argparse
from collections import Counter
import os
from pathlib import Path
import re
import signal
import statistics
import subprocess
import sys
import tempfile
import time


def run(argv, cwd, env, timeout):
    start = time.monotonic_ns()
    with subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True) as proc:
        try:
            out, err = proc.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.communicate()
            raise
        return (time.monotonic_ns() - start) / 1e6, proc.returncode, out, err


def measure(label, commands, runs, check, internal_clock=False):
    times = [[], []]
    for trial in range(runs + 1):
        outputs = []
        for index in ([0, 1] if trial % 2 == 0 else [1, 0]):
            try:
                ms, rc, out, err = run(*commands[index])
            except subprocess.TimeoutExpired:
                print(f"{label}\tTIMEOUT shell {index + 1}", flush=True)
                return
            if internal_clock:
                match = re.search(rb"(?m)^wall ([0-9]+\.[0-9]+)\s*$", err)
                if not match:
                    print(f"{label}\tNO TIME shell {index + 1}: {err[-200:]!r}", flush=True)
                    return
                ms = float(match[1]) * 1000
                err = err[:match.start()] + err[match.end():]
            detail = check(rc, out, err)
            if detail:
                print(f"{label}\tINVALID shell {index + 1}: {detail}", flush=True)
                return
            outputs.append(out)
            if trial:
                times[index].append(ms)
        if outputs[0] != outputs[1]:
            print(f"{label}\tOUTPUT DIFF: {outputs[0][-160:]!r} / {outputs[1][-160:]!r}", flush=True)
            return
    a, b = map(statistics.median, times)
    print(f"{label}\t{a:.3f}\t{b:.3f}\t{a / b:.3f}", flush=True)


def checked(rc, out, err):
    if rc or err.strip():
        return f"rc={rc} stderr={err[:180]!r}"


def unixwin(args, root, env):
    sys.path.insert(0, str(args.source))
    from benchsuite.cli import load_cases
    from benchsuite.runner import Runner
    extra = set("""startup.biglib_source parse.n_mixed@5000 expand.nested_cmdsubst
        arrays.read_elem funcs.redefine subst.cmd_ext_date redir.pipe_data_10k
        glob.lad_star@1000 data.read_mapfile@1000 mixed.arg_parse@1000 builtin.shift_one
        hotloop.composite_reported_2""".split())
    count = Counter()
    for case in load_cases():
        count[case.category] += 1
        if count[case.category] > 2 and case.id not in extra:
            continue
        commands = []
        for index, shell in enumerate(args.shells):
            work = root / str(index) / case.id
            argv, case_env = Runner(shell, root=str(root))._prepare(case, str(work))
            changes = {k: case_env[k] for k, _ in case.env}
            commands.append((argv, work, {**env, **changes}, min(case.timeout, 45)))

        def verify(rc, out, err):
            if rc != case.expect_rc or (case.expect_out and not re.search(case.expect_out.encode(), out + err)):
                return f"rc={rc} output={(out + err)[-180:]!r}"
            if case.category == "errors":
                if len(err.splitlines()) != case.iters:
                    return f"expected {case.iters} diagnostic lines, got {len(err.splitlines())}"
            elif err:
                return f"stderr={err[:180]!r}"
        measure(case.id, commands, args.runs, verify)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", choices=["unixwin", "fumiyas", "luish"])
    parser.add_argument("source", type=Path)
    parser.add_argument("shells", nargs=2, type=Path, metavar="SHELL")
    parser.add_argument("--runs", type=int, default=3)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")
    args.source = args.source.resolve()
    args.shells = [str(s.absolute()) for s in args.shells]
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        env = dict(HOME=directory, PATH=os.environ["PATH"], LC_ALL="C", TMPDIR=directory)
        print("case\tTonari_ms\tBash_ms\tTonari/Bash", flush=True)
        if args.suite == "unixwin":
            unixwin(args, root, env)
        elif args.suite == "luish":
            for script in sorted((args.source / "bench/scripts").glob("*.sh")):
                commands = [([sh, str(script), "1"], root, {**env, "SH": sh}, 90) for sh in args.shells]
                measure(script.stem, commands, args.runs, checked)
        else:
            for line in args.source.read_text().splitlines():
                if line.startswith("## "):
                    label = line[3:]
                elif line.strip():
                    code = "TIMEFORMAT='wall %3R'; " + line
                    commands = [([sh, "-c", code], root, env, 180) for sh in args.shells]
                    measure(label, commands, args.runs, checked, internal_clock=True)


if __name__ == "__main__":
    main()
