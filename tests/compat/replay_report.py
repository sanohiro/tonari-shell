"""Create a feature-by-feature ledger from the native replay artifacts."""
import argparse
import csv
import json
import math
from pathlib import Path
from statistics import median

from replay_budget import MANIFEST, workloads


def within(time, bash):
    return time * 3 <= bash * 2


def at_most_parity(samples):
    return median(samples["current"]) <= 1.10 * median(samples["bash"])


def classify(fixed, extended):
    samples = fixed["samples_ns"]
    current, bash = (median(samples[k]) for k in ("current", "bash"))
    if within(current, bash):
        return "fixed batch within target"
    if extended and "samples_ns" in extended:
        extra = extended["samples_ns"]
        new, ref = median(extra["current"]), median(extra["bash"])
        if within(new, ref):
            return "long batch within target"
        # Only grant the short-operation allowance at or below Bash time.
        if new <= ref <= extended["count"] * 5_000:
            return "short at parity"
    if "direct" in samples:
        direct = median(samples["direct"])
        if at_most_parity(samples) and not within(direct, bash) and 0.90 * current <= direct <= 1.10 * current:
            return "matched OS control"
    return "tried, above target"


def component_results(path):
    found = {}
    if path:
        for file in path.glob("*/replay.jsonl"):
            with file.open() as stream:
                header = json.loads(next(stream))
                lane = header["lane"]
                for line in stream:
                    row = json.loads(line)
                    found[lane, row["id"]] = (header["sha256"]["current"], row)
    return found


def replay_files(root, recovery):
    yield from sorted(root.glob("*/replay.jsonl"))
    replacements = {}
    for directory in recovery:
        for file in directory.glob("*/replay.jsonl"):
            with file.open() as stream:
                lane = json.loads(next(stream))["lane"]
            if lane in replacements:
                raise ValueError(f"{lane}: multiple replacement replays")
            replacements[lane] = file

    for timings in sorted(root.glob("*/timings.json")):
        original = timings.with_name("additional.jsonl")
        with original.open() as stream:
            lane = json.loads(next(stream))["lane"]
        file = replacements.pop(lane, original)
        inventory = json.loads(timings.read_text())
        expected = {row["id"]: row for row in workloads(lane, timings)}
        with file.open() as stream:
            hashes = json.loads(next(stream))["sha256"]
            if [hashes["bash"], hashes["current"]] != inventory["benchmark"]["executable_sha256"]:
                raise ValueError(f"{lane}: additional replay differs from the audited executables")
            for line in stream:
                row = json.loads(line)
                if expected.pop(row["id"], None) != row["workload"]:
                    raise ValueError(f"{lane}: additional workload differs from the inventory")
        if expected:
            raise ValueError(f"{lane}: {len(expected)} newly over-budget workloads missing")
        yield file
    if replacements:
        raise ValueError(f"no inventory for replacement lanes: {sorted(replacements)}")


def records(root, parts=None, recovery=()):
    frozen = {row["id"]: row for row in map(json.loads, MANIFEST.read_text().splitlines())}
    expected = {(lane, group, name) for row in frozen.values() for lane, config in row["lanes"].items()
                for group, name in config["features"]}
    seen = set()
    components = component_results(parts)
    for file in replay_files(root, recovery):
        with file.open() as stream:
            header = json.loads(next(stream))
            lane = header["lane"]
            for line in stream:
                result = json.loads(line)
                fixed = result["fixed"]
                extra = result.get("extended", {})
                row = result.get("workload") or frozen[result["id"]]
                config = row["lanes"][lane]
                if "error" in fixed or "error" in extra:
                    raise ValueError((lane, result["id"], fixed.get("error"), extra.get("error")))
                current_hash = header["sha256"]["current"]
                part_hash, part = components.get((lane, result["id"]), (current_hash, {}))
                if part_hash != current_hash:
                    raise ValueError(f"{lane}: component replay measured a different executable")
                component = part.get("component", {})
                control = part.get("fixed", {})
                verdict = classify(fixed, extra)
                if (verdict == "tried, above target" and control and at_most_parity(fixed["samples_ns"])
                        and classify(control, {}) == "matched OS control"):
                    verdict = "matched OS control"
                for group, name in config["features"]:
                    key = (lane, group, name)
                    if key in seen:
                        raise ValueError(f"duplicate feature: {key}")
                    seen.add(key)
                    data = dict(lane=lane, group=group, feature=name, id=result["id"],
                                cohort="previous misses" if key in expected else "new misses",
                                verdict=verdict, iterations=fixed["count"])
                    for label, stage in (("fixed", fixed), ("long", extra), ("control_replay", control), ("component", component)):
                        for build in ("bash", "baseline", "current", "opt-3", "opt-s", "direct"):
                            values = stage.get("samples_ns", {}).get(build, [])
                            data[f"{label}_{build}_p50_ns"] = round(median(values)) if values else ""
                            data[f"{label}_{build}_p95_ns"] = sorted(values)[math.ceil(.95*len(values))-1] if values else ""
                    data.update(long_iterations=extra.get("count", ""), old_tonari_ns=config["old_ns"][0],
                                old_bash_ns=config["old_ns"][1], control=json.dumps(fixed.get("control")),
                                control_replay=json.dumps(control.get("control")), component_iterations=component.get("count", ""),
                                component_error=component.get("error", ""), component_setup=part.get("setup", ""),
                                component_body=part.get("body", ""), argv=json.dumps(row["argv"], ensure_ascii=False))
                    yield data
    missing = expected - seen
    if missing:
        raise ValueError(f"{len(missing)} previous observations missing, e.g. {sorted(missing)[:3]}")


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("artifacts", type=Path)
    cli.add_argument("ledger", type=Path)
    cli.add_argument("--components", type=Path)
    cli.add_argument("--recovery", type=Path, nargs="*", default=[])
    args = cli.parse_args()
    rows = sorted(records(args.artifacts, args.components, args.recovery), key=lambda r: (r["lane"], r["group"], r["feature"]))
    with args.ledger.open("w") as out:
        writer = csv.DictWriter(out, fieldnames=list(rows[0]), delimiter="\t", lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


if __name__ == "__main__":
    main()
