"""Repeat every joint POSIX pass above the time target with the same release."""
import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import re
import runpy
from statistics import median
import subprocess
import sys

HERE = Path(__file__).resolve().parent
RUNNER = HERE / "yash-posix.py"


def selected_cases(directory, tonari, bash):
    host = (directory / "host.txt").read_text()
    binary = re.search(r"^([a-f0-9]{64})  .*/bin/current$", host, re.M)
    reference = subprocess.check_output([bash, "--version"], text=True).splitlines()[0]
    if not binary or binary[1] != hashlib.sha256(tonari.read_bytes()).hexdigest() or reference not in host.splitlines():
        raise ValueError(f"{directory}: timed executable or reference version changed")
    samples = defaultdict(lambda: defaultdict(dict))
    with (directory / "samples.jsonl").open() as stream:
        for record in map(json.loads, stream):
            label = record["shell"]
            if label not in {"tonari", "bash"}:
                continue
            for case, pair in record["cases"].items():
                series = samples[case][label]
                if record["trial"] in series:
                    raise ValueError(f"duplicate sample for {case}")
                series[record["trial"]] = pair
    selected = []
    for case, pair in samples.items():
        if any(set(pair[label]) != {0, 1, 2} for label in ("tonari", "bash")):
            raise ValueError(f"incomplete comparison for {case}")
        if all(v[0] == "OK[PASSED]" and v[1] is not None for group in pair.values() for v in group.values()):
            elapsed = {label: median(v[1] for v in series.values()) for label, series in pair.items()}
            if elapsed["tonari"] * 3 > elapsed["bash"] * 2:
                selected.append(case)
    return sorted(selected)


def run(root, out, tonari, bash, fixtures):
    command = [sys.executable, RUNNER, root, out, "--tonari", tonari, "--bash", bash, "--runs", "11"]
    for name in sorted(fixtures):
        command.extend(["--fixture", name])
    subprocess.run(command, check=True)
    valid = defaultdict(lambda: defaultdict(int))
    with (out / "samples.jsonl").open() as stream:
        for record in map(json.loads, stream):
            for case, (verdict, ns) in record["cases"].items():
                valid[case.split(": ", 1)[0]][record["shell"]] += verdict == "OK[PASSED]" and ns is not None
    return valid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("root", "output", "original", "tonari", "bash"):
        parser.add_argument(name, type=Path)
    args = parser.parse_args()
    root, out = args.root.resolve(), args.output.resolve()
    out.mkdir(exist_ok=True)
    cases = selected_cases(args.original, args.tonari, args.bash)
    (out / "selection.json").write_text(json.dumps(cases) + "\n")
    if not cases:
        return
    driver = runpy.run_path(str(RUNNER))
    driver["prepare"](root)
    controller = root / "tests" / "run-timed.sh"
    marker = root / "tests" / ".repeat"
    available = set(driver["fixtures"](root))
    fixtures = set()
    for case in cases:
        filename, line, _ = case.split(":", 2)
        if filename not in available or not line.isdecimal():
            raise ValueError(case)
        fixtures.add(filename)
        directory = marker / filename
        directory.mkdir(parents=True, exist_ok=True)
        (directory / line).touch()
    # Fixture setup and each chosen test still use the original input and assertions.
    driver["replace"](controller, '    in_file="$test_lineno.in"',
                      '    if [ -d ../.repeat ] && [ ! -f "../.repeat/$test_file/$test_lineno" ]; then return 0; fi\n'
                      '    in_file="$test_lineno.in"')
    valid = run(root, out, args.tonari, args.bash, fixtures)
    incomplete = {case.split(":", 1)[0] for case in cases
                  if any(valid[case.split(": ", 1)[0]][label] != 11 for label in ("tonari", "bash"))}
    if incomplete:
        # Some fixtures consume files produced by earlier tests. Restore the
        # unfiltered fixture whenever the selected replay loses a joint pass.
        marker.rename(root / "saved-selection")
        valid = run(root, out / "whole", args.tonari, args.bash, incomplete)
        for case in cases:
            if case.split(":", 1)[0] in incomplete and any(valid[case.split(": ", 1)[0]][label] != 11 for label in ("tonari", "bash")):
                raise ValueError(f"whole-fixture replay lost joint pass: {case}")


if __name__ == "__main__":
    main()
