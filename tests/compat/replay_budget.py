"""Replay a frozen set of above-budget Bash observations against every release candidate."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shlex
import socket
import shutil
import subprocess
import tempfile
import time

from audit import Audit
from bench import as_workload

MANIFEST = Path(__file__).with_name("budget-2026-11-24.jsonl")
END = b"__TONARI_BENCH_COMPLETE__"


def floor_args(source, count):
    if source.startswith("unset v; (") and source.count("(printf ") == 3:
        if source.count(") 2>/dev/null") == 3:
            return ["groups", str(count), "redirect"]
        if ">/dev/null" not in source:
            return ["groups", str(count)]
    delay = re.match(r"^\(?sleep (\.[0-9]+)[ ;]", source)
    if delay and re.search(r"(?:^|[;&] *)wait(?: |$)", source):
        # Each of these scripts waits for its one sleep before starting the next iteration.
        return ["wait", str(count), delay.group(1)]
    if delay and "disown" in source and "wait" not in source:
        return ["overlap", str(count), delay.group(1), str(source.count("sleep "))]
    if source.startswith("touch ") and "; touch " not in source:
        setup = source.partition(";")[0]
        return ["tool", str(count), *shlex.split(setup)]
    exported = {
        "declare -x x=one": ("x", "one"), "typeset -x x=one": ("x", "one"),
        "set -a; x=value": ("x", "value"), "v='two words'; export v": ("v", "two words"),
        "export v=yes; export -n v": ("v", ""), "v=outer; v=inner": ("v", "inner"),
    }
    script = re.fullmatch(r"(.+?)[; ]+(/bin/sh -c '[^']+')(?:; echo .+)?", source)
    if script and script[1] in exported:
        return ["exported", str(count), *exported[script[1]], *shlex.split(script[2])]
    cond = re.fullmatch(r'printf data >file; :; \[\[ (-[a-zA-Z]) ([^ ;]+) \]\]; printf %s "\$\?"; \[\[ \1 absent \]\]; printf %s "\$\?"', source)
    if cond:
        return ["file", str(count), *cond.groups()]


def sample_floor(binary, args, root, fixture):
    cwd = root / "control-work"
    if cwd.exists():
        shutil.rmtree(cwd)
    cwd.mkdir()
    if fixture == "socket":
        with socket.socket(socket.AF_UNIX) as sock:
            sock.bind(str(cwd / "socket"))
    expected = b""
    if args[0] == "exported":
        value = (args[3] or "unset").encode()
        expected = (value + (b"\n" if args[-1].startswith("echo") else b"")) * int(args[1])
    env = dict(PATH="/usr/bin:/bin", HOME=str(cwd), TMPDIR=str(cwd), LC_ALL="C", TERM="dumb")
    with subprocess.Popen([binary, *args], cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE) as proc:
        if proc.stdout.readline() != b"__READY__\n":
            raise ValueError("control failed to start")
        start = time.perf_counter_ns()
        out = proc.communicate(b"\n")[0]
        elapsed = time.perf_counter_ns() - start
        if proc.returncode or out != expected:
            raise ValueError("control failed")
        return elapsed


def compare(audits, row, count, runs, floor, *, setup="", body=None):
    argv = row["argv"]
    def run(shell, source, before=setup):
        return audits[shell].call(1, [*argv[:-1], source], fixture=row["fixture"], timing="started", before_ready=before)
    original = run("bash", argv[-1], "")
    if original.timeout or row["stdin"]:
        raise ValueError("missing reference")
    status = f"__TONARI_BENCH_STATUS_{original.code}__".encode()
    script = argv[-1] if body is None else body
    head = f"for __tonari_bench_i in {{1..{count}}}; do\n:\n{script}\n"
    tail = "\ndone\nbuiltin printf __TONARI_BENCH_COMPLETE__"
    checked = head + """builtin printf '__TONARI_BENCH_STATUS_%d__' "$?" >&2""" + tail
    expected = original.out * count + END
    def verify(out, marker):
        return (not out.timeout and out.code == 0 and out.out == expected
                and out.err == (original.err + marker) * count)
    names = list(audits)
    for name in names:
        o = run(name, checked)
        if not verify(o, status):
            raise ValueError(f"{name} iteration check {o.brief()}")
    native = floor_args(argv[-1], count) if floor else None
    if native:
        names.append("direct")
    samples = {name: [] for name in names}
    for i in range(runs + 1):
        for name in names[i % len(names):] + names[:i % len(names)]:
            if name == "direct":
                ns = sample_floor(floor, native, audits["bash"].root, row["fixture"])
            else:
                o = run(name, head + tail)
                if not verify(o, b""):
                    raise ValueError(f"{name} timed output {o.brief()}")
                ns = o.ns
            if i:
                samples[name].append(ns)
    return {"count": count, "samples_ns": samples, "control": native}


def workloads(lane, inventory):
    fixed = [json.loads(line) for line in MANIFEST.read_text().splitlines()]
    if inventory is None:
        yield from (row for row in fixed if lane in row["lanes"])
        return
    # The broad audit selects batches again on the new host. Also try every newly
    # above-budget observation, even when its workload was absent from the frozen set.
    rows = json.loads(inventory.read_text())["rows"]
    old = {(group, feature) for row in fixed for group, feature
           in row["lanes"].get(lane, {}).get("features", [])}
    extra = {}
    for group, entries in rows.items():
        for name, item in entries.items():
            perf = item.get("batch_timing", {})
            if ((group, name) in old or "bash" not in perf or
                    perf["tonari"][0] * 3 <= perf["bash"][0] * 2):
                continue
            work = as_workload(rows, group, name)
            if not work or "argv" not in work:
                raise ValueError(f"{group} / {name}: repeated timing has no shell witness")
            argv, fixture, stdin = (work[k] for k in ("argv", "fixture", "stdin"))
            key = (tuple(argv), fixture, stdin, perf["iterations"])
            if key not in extra:
                digest = hashlib.sha256(json.dumps(key).encode()).hexdigest()[:12]
                extra[key] = {"id": digest, "argv": argv, "fixture": fixture, "stdin": stdin,
                              "lanes": {lane: {"iterations": perf["iterations"], "features": [],
                                               "old_ns": [perf["tonari"][0], perf["bash"][0]]}}}
            extra[key]["lanes"][lane]["features"].append([group, name])
    yield from extra.values()


def isolate(source):
    if source.startswith("unset v; (printf ") and source.count("(printf ") == 3 and "?" not in re.sub(r'\$\?', "", source):
        # Each stage sets v anew. Verify even the grouped variant against the
        # original Bash witness before timing its expansions.
        body = re.sub(r"\(printf ([^()]+)\)", r"{ printf \1; }", source)
        if body.count("{ printf ") == 3:
            return "", body
    if source.startswith("touch ") or source.startswith("printf data >file;") or re.match(r"^printf '[^']+' > input; exec ", source):
        setup, _, body = source.partition(";")
        while body.lstrip().startswith(("touch ", "chmod ")):
            cmd, _, body = body.partition(";")
            setup += ";" + cmd
        return setup, body


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--lane", required=True)
    cli.add_argument("--bash", type=Path, required=True)
    cli.add_argument("--bin", type=Path, required=True)
    cli.add_argument("--runs", type=int, default=9)
    cli.add_argument("--only", nargs="*")
    cli.add_argument("--inventory", type=Path)
    cli.add_argument("--components", action="store_true")
    cli.add_argument("--release-only", action="store_true", help="compare just the two release binaries and Bash")
    cli.add_argument("--self-invocation", action="store_true",
                     help="start each shell by absolute path and verify that $BASH runs the same build")
    args = cli.parse_args()
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        audits = {}
        hashes = {}
        for name in (("bash", "baseline", "current") if args.components or args.release_only else ("bash", "baseline", "current", "opt-3", "opt-s")):
            original = args.bash if name == "bash" else args.bin / name
            staged = root / name / "bash"
            staged.parent.mkdir()
            shutil.copy(original, staged)
            hashes[name] = hashlib.sha256(staged.read_bytes()).hexdigest()
            audit = Audit(staged, staged, root, absolute_argv0=args.self_invocation)
            if args.self_invocation:
                probe = '''[[ "$BASH" -ef "$SHELL" ]] && "$BASH" --noprofile --norc -c '[[ "$BASH" -ef "$SHELL" ]] && printf %s "$BASH_VERSION"' '''
                identity = audit.script(1, probe)
                version = audit.script(1, 'printf %s "$BASH_VERSION"')
                if identity.code or identity.err or not identity.out or identity.out != version.out:
                    raise ValueError(f"{name} self-invocation resolved a different shell: {identity.brief()}")
            audits[name] = audit
        floor = root / "control"
        subprocess.run(["cc", "-O2", str(Path(__file__).with_name("budget_floor.c")), "-o", str(floor)], check=True)
        print(json.dumps({"lane": args.lane, "sha256": hashes, "argv0": "absolute" if args.self_invocation else "bash"}), flush=True)
        for row in workloads(args.lane, args.inventory):
            config = row["lanes"].get(args.lane)
            if not config or (args.only is not None and row["id"] not in args.only):
                continue
            n = config["iterations"]
            result = {"id": row["id"]}
            if args.components:
                source = row["argv"][-1]
                native = floor_args(source, n)
                part = isolate(source)
                if native and native[0] in ("overlap", "tool", "exported", "file", "groups"):
                    result["fixed"] = compare(audits, row, n, args.runs, floor)
                if part:
                    setup, body = part
                    try:
                        result["component"] = compare(audits, row, 128, 9, None, setup=setup, body=body)
                        result.update(setup=setup, body=body)
                    except ValueError as error:
                        result["component"] = {"error": str(error)}
                if len(result) > 1:
                    print(json.dumps(result), flush=True)
                continue
            if args.inventory:
                result["workload"] = row
            for name, count, runs in [("fixed", n, args.runs), ("extended", 1024, 5)]:
                if name == "extended" and (n != 128 or max(config["old_ns"]) > 4_000_000):
                    continue
                try:
                    result[name] = compare(audits, row, count, runs, floor)
                except ValueError as err:
                    result[name] = {"error": str(err)}
            print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
