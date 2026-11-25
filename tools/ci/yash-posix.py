"""Run pinned yash POSIX fixtures using their original controller and verdicts."""
import argparse
from collections import Counter
import gzip
import json
import os
from pathlib import Path
import re
import shutil
import signal
import resource
import subprocess
import time

REVISION = "1cdb509a617894ed96a12c33b43bcd7adb6f1ff5"
VERDICT = re.compile(r"^%%% (START|OK\[[A-Z_]+\]|ERROR\[[A-Z_]+\]|SKIPPED): (.+)$")

# Add a monotonic clock to the *controller*. A wrapper around TESTEE changes
# PPID, job groups and signal delivery. Neither shell under test is instrumented.
CLOCK = r"""
    if (argc == 2 && (wcscmp(ARGV(1), L"__test_begin") == 0 ||
                      wcscmp(ARGV(1), L"__test_end") == 0)) {
        struct timespec now;
        if (clock_gettime(CLOCK_MONOTONIC, &now) != 0)
            return 1;
        return dprintf(1, "@clock %c %ld %lld\n",
                       wcscmp(ARGV(1), L"__test_begin") == 0 ? 'B' : 'E',
                       (long)getpid(), (long long)now.tv_sec * 1000000000 + now.tv_nsec) < 0;
    }
"""


def replace(path, original, new):
    source = path.read_text()
    if source.count(original) != 1:
        raise ValueError(f"unexpected upstream source in {path}")
    path.write_text(source.replace(original, new))


def prepare(root):
    if subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip() != REVISION:
        raise ValueError("unexpected yash revision")
    path = root / "exec.c"
    replace(path, "#include <sys/times.h>", "#include <sys/times.h>\n#include <time.h>")
    signature = "int times_builtin(int argc __attribute__((unused)), void **argv)\n{"
    replace(path, signature, signature + CLOCK)
    subprocess.run(["/bin/sh", "./configure", "--disable-nls"], cwd=root, check=True)
    subprocess.run(["make", "-j3", "yash"], cwd=root, check=True)
    subprocess.run(["make", "-C", "tests", "tester"], cwd=root, check=True)
    controller = root / "tests" / "run-timed.sh"
    shutil.copyfile(root / "tests" / "run-test.sh", controller)
    replace(controller, '    chmod u+r "$in_file"', '    chmod u+r "$in_file"\n'
            '    if ! "$remove_work_dir"; then for arg do printf \'%s\\0\' "$arg"; done >"$test_lineno.argv"; fi')
    replace(controller, "    log_stdout START\n    set +e",
            "    log_stdout START\n    set +e\n    times __test_begin")
    replace(controller, '    actual_exit_status="$?"\n', '    actual_exit_status="$?"\n    times __test_end\n')


def fixtures(root):
    makefile = (root / "tests" / "Makefile").read_text()
    def names(variable):
        return re.search(r"^" + variable + r" = (.+)$", makefile, re.M)[1].split()
    return sorted([*names("POSIX_SIGNAL_TEST_SOURCES"),
                   *[v for v in names("POSIX_TEST_SOURCES") if v.endswith(".tst")]])


def children_of(root, parents):
    children = [p for p, pp in parents.items() if pp == root]
    return children + [desc for child in children for desc in children_of(child, parents)]


def terminate_children(owner, process):
    parents = dict(tuple(map(int, line.split())) for line in
                   subprocess.check_output(["ps", "-A", "-o", "pid=", "-o", "ppid="], text=True).splitlines())
    if owner != process.pid and owner not in children_of(process.pid, parents):
        raise RuntimeError("timed case controller is outside its fixture")
    for pid in reversed(children_of(owner, parents)):
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def measure(tests, subject, fixture, environment, timeout, keep):
    report = tests / Path(fixture).with_suffix(".trs")
    report.unlink(missing_ok=True)
    command = ["./resetsig", "../yash", "./run-timed.sh", *(("-r",) if keep else ()), str(subject), fixture]
    if "%REQUIRETTY%" in (tests / fixture).read_text():
        command[:0] = ["./ptwrap"]
    timed_out = set()
    stalled = False
    started = time.monotonic()
    with subprocess.Popen(command, cwd=tests, env=environment, start_new_session=True,
                          stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                          stderr=subprocess.DEVNULL) as process:
        last_activity, size = started, -1
        while process.poll() is None:
            try:
                process.wait(timeout=.25)
            except subprocess.TimeoutExpired:
                pass
            current = report.stat().st_size if report.exists() else 0
            if current != size:
                last_activity, size = time.monotonic(), current
            if time.monotonic() - last_activity < timeout:
                continue
            lines = report.read_text(errors="replace").splitlines() if current else []
            clocks = [s.split() for s in lines if s.startswith("@clock ")]
            if clocks and clocks[-1][1] == "B":
                key = next(s.split(": ", 1)[1] for s in reversed(lines) if s.startswith("%%% START: "))
                if key not in timed_out:
                    timed_out.add(key)
                    terminate_children(int(clocks[-1][2]), process)
                    last_activity = time.monotonic()
                    continue
            terminate_children(process.pid, process)
            if process.poll() is None:
                process.kill()
            stalled = True
            break
        if process.returncode and not stalled:
            raise RuntimeError(f"{fixture}: controller exited {process.returncode}")
    if not report.is_file():
        raise RuntimeError(f"{fixture}: no upstream report")
    rows, begin, elapsed, active = {}, None, None, None
    for line in report.read_text(errors="replace").splitlines():
        if match := VERDICT.match(line):
            status, key = match.groups()
            if status == "START":
                if active is not None:
                    raise ValueError(f"missing verdict for {active}")
                active, begin, elapsed = key, None, None
            else:
                if key in rows or (status != "SKIPPED" and (active != key or elapsed is None)):
                    raise ValueError(f"unexpected verdict for {key}")
                rows[key] = ("TIMEOUT" if key in timed_out else status, elapsed)
                active, begin, elapsed = None, None, None
        elif line.startswith("@clock "):
            _, kind, pid, ns = line.split()
            if kind == "B":
                begin = (pid, int(ns))
            elif begin and begin[0] == pid:
                elapsed = int(ns) - begin[1]
    if active is not None:
        rows[active] = ("INCOMPLETE", None)
    if stalled or active is not None:
        rows[fixture + ": controller"] = ("INCOMPLETE", None)
    return rows, time.monotonic() - started, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path, help="disposable checkout of yash 2.60")
    parser.add_argument("output", type=Path)
    parser.add_argument("--tonari", type=Path, required=True)
    parser.add_argument("--bash", type=Path, required=True)
    parser.add_argument("--previous", type=Path)
    parser.add_argument("--prepare", action="store_true")
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--shard", default="0/1")
    parser.add_argument("--fixture", action="append", default=[])
    parser.add_argument("--timeout", type=int, default=8)
    args = parser.parse_args()
    root, out = args.root.resolve(), args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    if args.prepare:
        prepare(root)
    shard, count = map(int, args.shard.split("/"))
    if not 0 <= shard < count or args.runs < 1:
        parser.error("invalid shard or run count")
    sources = fixtures(root)
    if set(args.fixture) - set(sources):
        parser.error("unknown fixture")
    sources = [s for i, s in enumerate(sources) if i % count == shard and (not args.fixture or s in args.fixture)]
    if not sources:
        parser.error("empty fixture selection")
    shells = {"tonari": args.tonari.resolve(), "bash": args.bash.resolve()}
    if args.previous:
        shells["previous"] = args.previous.resolve()
    tests = root / "tests"
    subject = root / "subject"
    env = dict(PATH="/usr/bin:/bin:/usr/sbin:/sbin", HOME=str(root), TMPDIR=str(root), LC_ALL="C")
    resource.setrlimit(resource.RLIMIT_CORE, (0, resource.getrlimit(resource.RLIMIT_CORE)[1]))
    rows = {}
    with gzip.open(out / "inputs.jsonl.gz", "wt") as inputs, (out / "samples.jsonl").open("w") as records:
        for number, fixture in enumerate(sources):
            for trial in range(args.runs):
                labels = list(shells)
                offset = (trial + number) % len(labels)
                for label in labels[offset:] + labels[:offset]:
                    subject.unlink(missing_ok=True)
                    subject.symlink_to(shells[label])
                    keep = label == "bash" and trial == 0
                    before = set(tests.glob("tmp.*")) if keep else set()
                    obs, wall, report = measure(tests, subject, fixture, env, args.timeout, keep)
                    if keep:
                        for directory in set(tests.glob("tmp.*")) - before:
                            for key in obs:
                                line = key.split(":", 2)[1]
                                path = directory / (line + ".in")
                                if path.is_file():
                                    flags = path.with_suffix(".argv")
                                    arguments = flags.read_bytes().split(b"\x00")[:-1] if flags.exists() else []
                                    inputs.write(json.dumps(dict(case=key, source=os.fsdecode(path.read_bytes()),
                                                                 argv=[os.fsdecode(v) for v in arguments])) + "\n")
                            subprocess.run(["chmod", "-R", "u+rwX", directory], check=True)
                            shutil.rmtree(directory)
                    records.write(json.dumps(dict(fixture=fixture, shell=label, trial=trial,
                                                  wall=wall, cases=obs)) + "\n")
                    records.flush()
                    if trial == 0:
                        dest = out / label
                        dest.mkdir(exist_ok=True)
                        shutil.copyfile(report, dest / report.name)
                    for key, value in obs.items():
                        rows.setdefault(key, {}).setdefault(label, []).append(value)
            print(f"{fixture} {len(obs)} cases", flush=True)
    for label in shells:
        dest = out / label
        logs = sorted(dest.glob("*.trs"))
        with (dest / "summary.log").open("wb") as handle:
            subprocess.run(["/bin/sh", tests / "summarize.sh", *logs], cwd=tests, stdout=handle, check=True)
        for log in logs:
            with log.open("rb") as inp, gzip.open(str(log) + ".gz", "wb") as output:
                shutil.copyfileobj(inp, output)
            log.unlink()
    scores = Counter()
    for key, series in rows.items():
        def passed(label):
            return len(series.get(label, [])) == args.runs and all(v[0] == "OK[PASSED]" for v in series[label])
        if passed("bash") and passed("tonari"):
            scores["comparable"] += 1
        if args.previous and passed("previous") and passed("bash") and not passed("tonari"):
            scores["new differences"] += 1
            print("review new difference", key)
    print(dict(scores), flush=True)
    # Preserve all measurements before returning the comparison gate.
    return int(bool(scores["new differences"] or any(
        v[0] == "INCOMPLETE" for series in rows.values() for events in series.values() for v in events)))


if __name__ == "__main__":
    sys_exit = main()
    raise SystemExit(sys_exit)
