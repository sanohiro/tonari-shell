"""Compare clean configure runs with a Tonari build timing its foreground waits."""
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile
import time


# Count known native tools. Shells, make recipes, background sleeps and
# pipelines can overlap shell execution and are excluded from this estimate.
UTILITIES = set("basename cat chmod cmp cp date dirname expr gawk grep hostname install ln ls mkdir mv pwd rm rmdir sed sort touch tr uname wc".split())


def union(spans):
    total = last = 0
    for start, end in sorted(spans):
        total += max(0, end - max(last, start))
        last = max(last, end)
    return total / 1e9


def waits(path):
    groups = {"compiler": [], "probe": [], "utility": []}
    launch = []
    for line in path.read_text().splitlines():
        _, _, before, launched, done, _, program = line.split(" ", 6)
        if program == "/usr/bin/cc":
            group = "compiler"
        elif program == "./conftest":
            group = "probe"
        else:
            name = Path(program)
            if name.name not in UTILITIES or name.parent not in (Path("."), Path("/usr/bin"), Path("/bin")):
                continue
            group = "utility"
        groups[group].append((int(launched), int(done)))
        launch.append((int(before), int(launched)))
    if not all(groups.values()):
        raise ValueError("missing foreground command class")
    return dict(counts={k: len(v) for k, v in groups.items()},
                seconds={k: union(v) for k, v in groups.items()},
                wait_seconds=union(span for spans in groups.values() for span in spans),
                launch_seconds=union(launch))


def measure(script, shell, root, log=None):
    env = dict(HOME=str(root), TMPDIR=str(root), PATH=os.environ["PATH"], LC_ALL="C",
               CC="/usr/bin/cc", SHELL=shell, CONFIG_SHELL=shell)
    if log is not None:
        log.touch(exist_ok=False)
        env["TONARI_CONFIGURE_WAITS"] = str(log)
    cmd = [shell, str(script), "--disable-nls", "--cache-file=/dev/null"]
    start = time.monotonic_ns()
    result = subprocess.run(cmd, cwd=root, env=env, stdin=subprocess.DEVNULL, capture_output=True,
                            start_new_session=True, timeout=180, check=True)
    elapsed = (time.monotonic_ns() - start) / 1e9
    if (root / "config.status").read_bytes().splitlines()[0][2:].strip() != os.fsencode(shell):
        raise ValueError("unexpected config.status interpreter")
    checks = [line for line in result.stdout.splitlines() if line.startswith(b"checking ")]
    if len(checks) < 200:
        raise ValueError("incomplete configure output")
    return elapsed, (checks, (root / "config.h").read_bytes(), result.stderr)


def main():
    source, tonari, bash, observer, output = sys.argv[1:]
    script = Path(source).resolve() / "configure"
    out = Path(output).resolve()
    out.mkdir(parents=True, exist_ok=True)
    shells = {"tonari": tonari, "bash": bash, "observer": observer}
    samples = {name: [] for name in shells}
    trace = []
    reference = None
    for trial in range(4):
        names = list(shells)
        order = trial % len(names)
        for name in names[order:] + names[:order]:
            log = out / f"foreground-{trial}.txt" if name == "observer" else None
            with tempfile.TemporaryDirectory() as root:
                elapsed, result = measure(script, shells[name], Path(root), log)
            if reference is None:
                reference = result
            elif result != reference:
                raise ValueError(f"{name} produced different configure checks, header or diagnostics")
            if trial:
                samples[name].append(elapsed)
                if log:
                    trace.append(waits(log))
            print(trial, name, round(elapsed, 3), flush=True)
    totals = {k: statistics.median(v) for k, v in samples.items()}
    report = dict(samples=samples, medians=totals, target=totals["bash"] * 2 / 3,
                  checks=len(reference[0]), foreground=trace)
    (out / "foreground.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report), flush=True)


if __name__ == "__main__":
    main()
