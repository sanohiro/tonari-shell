"""Paired measurements of upstream unixwin, fumiyas and luish workloads."""
import argparse
from collections import Counter
import os
from pathlib import Path
import re
import signal
import shutil
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
            if case.id == "coproc.tr_roundtrip" and "SHELLBENCH_COREUTILS" in os.environ:
                changes["PATH"] = os.environ["SHELLBENCH_COREUTILS"] + os.pathsep + env["PATH"]
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


def gnu_hello(args, root, env):
    shells = args.shells.copy()
    if args.previous:
        shells.append(str(args.previous.absolute()))
    samples = [[] for _ in shells]
    reference = None
    configure = str(args.source / "configure")
    for label, flags in (("configure syntax check", ["-n", configure]),
                         ("configure --help", [configure, "--help"])):
        commands = [([sh, *flags], root, {**env, "CONFIG_SHELL": sh}, 60) for sh in args.shells]
        measure(label, commands, args.runs, checked)
    for trial in range(args.runs + 1):
        order = list(range(len(shells)))
        offset = trial % len(shells)
        for i in order[offset:] + order[:offset]:
            shell = shells[i]
            build_env = {**env, "SHELL": shell, "CONFIG_SHELL": shell, "CC": "/usr/bin/cc"}
            with tempfile.TemporaryDirectory(dir=root) as directory:
                build = Path(directory)
                cmd = [shell, str(args.source / "configure"), "--disable-nls", "--cache-file=/dev/null"]
                ms, rc, out, err = run(cmd, build, build_env, 180)
                if rc:
                    raise RuntimeError(f"configure exited {rc}: {(out + err)[-1000:]!r}")
                config = (build / "config.h").read_bytes()
                if reference is None:
                    reference = config
                if reference != config:
                    raise ValueError("config.h changed across trials")
                interpreter = (build / "config.status").read_bytes().splitlines()[0][2:].strip()
                if os.fsdecode(interpreter) != shell:
                    raise ValueError(f"unexpected config.status interpreter {interpreter!r}")
                if trial:
                    samples[i].append(ms)
                else:
                    subprocess.run(["make", "-j2", "check"], cwd=build, env=build_env,
                                   stdout=subprocess.DEVNULL, check=True, timeout=180)
                    if subprocess.check_output([str(build / "hello")], env=build_env) != b"Hello, world!\n":
                        raise ValueError("unexpected hello output")
    a, b = map(statistics.median, samples[:2])
    if args.previous:
        old = statistics.median(samples[2])
        print(f"Previous configure\t{old:.3f}\t{b:.3f}\t{old / b:.3f}")
    print(f"GNU Hello configure\t{a:.3f}\t{b:.3f}\t{a / b:.3f}")
    target = b * 2 / 3
    verdict = "PASS" if a <= target else "MISS"
    print(f"Full configure target: {verdict} ({a:.3f} ms, budget {target:.3f} ms)", file=sys.stderr)
    print(f"Raw milliseconds {dict(zip(shells, samples))}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", choices=["unixwin", "fumiyas", "luish", "gnu-hello"])
    parser.add_argument("source", type=Path)
    parser.add_argument("shells", nargs=2, type=Path, metavar="SHELL")
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--previous", type=Path, help="previous Tonari build for full GNU configure")
    args = parser.parse_args()
    if args.previous and args.suite != "gnu-hello":
        parser.error("--previous requires gnu-hello")
    if args.runs < 1:
        parser.error("--runs must be positive")
    args.source = args.source.resolve()
    args.shells = [str(s.absolute()) for s in args.shells]
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        env = dict(HOME=directory, PATH=os.environ["PATH"], LC_ALL="C", TMPDIR=directory)
        print("case\tTonari_ms\tBash_ms\tTonari/Bash", flush=True)
        if args.suite == "gnu-hello":
            gnu_hello(args, root, env)
        elif args.suite == "unixwin":
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
                    if "/bin/true" in line and not Path("/bin/true").is_file():
                        print(f"{label}\tUNAVAILABLE /bin/true", flush=True)
                        true = shutil.which("true", path="/usr/bin:/bin")
                        if not true:
                            continue
                        label += f" ({true})"
                        line = line.replace("/bin/true", true)
                    code = "TIMEFORMAT='wall %3R'; " + line
                    commands = [([sh, "-c", code], root, env, 180) for sh in args.shells]
                    measure(label, commands, args.runs, checked, internal_clock=True)


if __name__ == "__main__":
    main()
