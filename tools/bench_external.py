"""Compare upstream shell workloads and check their observable results."""
import argparse
import json
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


def measure(label, commands, runs, check, internal_clock=False, *, samples, invalid=()):
    # Preserve failures in the artifact while letting the other cases run.
    samples[label] = None
    times = [[] for _ in commands]
    for trial in range(runs + 1):
        outputs = []
        for offset in range(len(commands)):
            index = (trial + offset) % len(commands)
            if index in invalid:
                continue
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
        different = next((out for out in outputs[1:] if out != outputs[0]), None)
        if different is not None:
            print(f"{label}\tOUTPUT DIFF: {outputs[0][-160:]!r} / {different[-160:]!r}", flush=True)
            return
    samples[label] = times
    medians = [statistics.median(t) if t else None for t in times]
    a, b = medians[:2]
    extra = "".join(f"\t{v:.3f}" if v is not None else "\tINVALID" for v in medians[2:])
    print(f"{label}\t{a:.3f}\t{b:.3f}\t{a / b:.3f}{extra}", flush=True)


def checked(rc, out, err):
    if rc or err.strip():
        return f"rc={rc} stderr={err[:180]!r}"


# Some upstream loops discard stdout, and their loop epilogue masks the
# last command's status. Inspect the result after an untimed copy has run.
UNIXWIN_OBSERVATIONS = {
    "expand.tilde_home": 'test "$x" = "$HOME/sub/dir"',
    "expand.tilde_user": 'printf "%s" "$x"',
    "expand.nested_cmdsubst": 'printf "%s" "$x"',
    "strings.append_assign": 'printf "%s" "$s"',
    "strings.concat_subst": 'printf "%s" "$s"',
    "arrays.append_loop": 'printf "<%s>" "${a[@]}"',
    "arrays.assign_idx_mod": 'printf "<%s>" "${!a[@]}" "${a[@]}"',
    "arrays.read_elem": 'printf "<%s>" "$x" "${a[@]}"',
    "control.if_true": 'printf "%s" "$x"',
    "control.if_false_else": 'printf "%s" "$x"',
    "funcs.redefine": 'type -t g',
    "subst.cmd_builtin_echo": 'printf "%s" "$x"',
    "subst.cmd_true": 'printf "<%s>" "$x"',
    "subst.cmd_ext_date": 'case $x in (""|*[!0-9]*) exit 1;; esac',
    "redir.file_append": 'cat out.log',
    "glob.star_txt@200": 'printf "%s" "$c"',
    "glob.star_all@300": 'printf "%s" "$c"',
    "glob.lad_star@1000": 'printf "%s" "$c"',
    "jobs.bg_wait_one": 'jobs -pr',
    "jobs.bg_many@20": 'jobs -pr',
    "mixed.log_tally@20000": 'for k in "${!cnt[@]}"; do echo "$k ${cnt[$k]}"; done | sort',
    "mixed.config_parse@2000": 'printf "<%s>" "$x" "$C_key500"',
    "mixed.arg_parse@1000": 'printf "<%s>" "$v" "$o" "$c" "$t"',
    "coproc.cat_roundtrip": 'printf "%s" "$x"',
    "coproc.tr_roundtrip": 'printf "%s" "$x"',
    "builtin.shift_one": 'printf "<%s>" "$@"',
    "arith.expand_add": 'printf "%s" "$x"',
    "arith.cmd_add": 'printf "%s" "$x"',
    "data.read_mapfile@1000": 'printf "<%s>" "$x" "${a[0]}" "${a[999]}"',
    "hotloop.composite_reported_2": 'printf "%s" "$i"',
}


def observe_unixwin(commands, observation):
    results = []
    for argv, cwd, env, timeout in commands:
        script = Path(cwd) / "observe.sh"
        script.write_bytes(Path(argv[1]).read_bytes() + observation.encode() + b"\n")
        _, code, out, err = run([argv[0], str(script)], cwd, env, timeout)
        results.append((code, out, err))
    return [code == 0 and not err and (code, out, err) == results[1]
            for code, out, err in results]


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
        if args.case and case.id not in args.case:
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
        invalid = []
        if observation := UNIXWIN_OBSERVATIONS.get(case.id):
            valid = observe_unixwin(commands, observation)
            if not all(valid[:2]):
                args.samples[case.id] = None
                print(f"{case.id}\tINVALID observed result", flush=True)
                continue
            if len(valid) > 3 and not valid[3]:
                raise ValueError(f"Optimization level changed the result of {case.id}")
            if len(valid) >= 3 and not valid[2]:
                print(f"Previous output differs for {case.id}", file=sys.stderr)
                invalid.append(2)
        measure(case.id, commands, args.runs, verify, samples=args.samples, invalid=invalid)


def gnu_hello(args, root, env):
    shells = args.shells
    samples = [[] for _ in shells]
    reference = None
    configure = str(args.source / "configure")
    for label, flags in (("configure syntax check", ["-n", configure]),
                         ("configure --help", [configure, "--help"])):
        if args.case and label not in args.case:
            continue
        commands = [([sh, *flags], root, {**env, "CONFIG_SHELL": sh}, 60) for sh in args.shells]
        measure(label, commands, args.runs, checked, samples=args.samples)
    if args.case and "GNU Hello configure" not in args.case:
        return
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
    args.samples["GNU Hello configure"] = samples
    a, b = map(statistics.median, samples[:2])
    if args.previous:
        old = statistics.median(samples[2])
        print(f"Previous configure\t{old:.3f}\t{b:.3f}\t{old / b:.3f}")
    print(f"GNU Hello configure\t{a:.3f}\t{b:.3f}\t{a / b:.3f}" + "".join(f"\t{statistics.median(values):.3f}" for values in samples[2:]))
    target = b * 2 / 3
    verdict = "PASS" if a <= target else "MISS"
    print(f"Full configure target: {verdict} ({a:.3f} ms, budget {target:.3f} ms)", file=sys.stderr)
    print(f"Raw milliseconds {dict(zip(shells, samples))}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", choices=["unixwin", "fumiyas", "luish", "gnu-hello"])
    parser.add_argument("source", type=Path)
    parser.add_argument("shells", nargs=2, type=Path, metavar="SHELL")
    parser.add_argument("--case", action="append", default=[], help="exact published case name, repeatable")
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--previous", type=Path, help="previous Tonari build on the same host")
    parser.add_argument("--opt3", type=Path, help="same source built with opt-level=3")
    parser.add_argument("--raw", type=Path, help="write per-shell samples in command-line order")
    args = parser.parse_args()
    args.case = set(args.case)
    args.samples = {}
    if args.opt3 and not args.previous:
        parser.error("--opt3 requires --previous")
    if args.runs < 1:
        parser.error("--runs must be positive")
    args.source = args.source.resolve()
    args.shells = [str(s.absolute()) for s in args.shells]
    if args.previous:
        args.shells.append(str(args.previous.absolute()))
    if args.opt3:
        args.shells.append(str(args.opt3.absolute()))
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        env = dict(HOME=directory, PATH=os.environ["PATH"], LC_ALL="C", TMPDIR=directory)
        print("case\tTonari_ms\tBash_ms\tTonari/Bash" + ("\tPrevious_ms" if args.previous else "") + ("\tOpt3_ms" if args.opt3 else ""), flush=True)
        if args.suite == "gnu-hello":
            gnu_hello(args, root, env)
        elif args.suite == "unixwin":
            unixwin(args, root, env)
        elif args.suite == "luish":
            for script in sorted((args.source / "bench/scripts").glob("*.sh")):
                if args.case and script.stem not in args.case:
                    continue
                commands = [([sh, str(script), "1"], root, {**env, "SH": sh}, 90) for sh in args.shells]
                measure(script.stem, commands, args.runs, checked, samples=args.samples)
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
                    if args.case and label not in args.case:
                        continue
                    code = "TIMEFORMAT='wall %3R'; " + line
                    commands = [([sh, "-c", code], root, env, 180) for sh in args.shells]
                    measure(label, commands, args.runs, checked, internal_clock=True, samples=args.samples)

    if args.raw:
        args.raw.write_text(json.dumps(dict(shells=args.shells, milliseconds=args.samples)) + "\n")
    missing = args.case.difference(args.samples)
    if missing:
        parser.error("unknown or unavailable cases: " + ", ".join(sorted(missing)))
    if any(v is None for v in args.samples.values()):
        raise SystemExit("One or more workloads failed validation or timing.")


if __name__ == "__main__":
    main()
