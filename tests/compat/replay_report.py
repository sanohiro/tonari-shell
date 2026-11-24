"""Create a feature-by-feature ledger from the native replay artifacts."""
import argparse
import csv
import json
import math
from pathlib import Path
from statistics import median

from replay_budget import MANIFEST


def within(time, bash):
    return time * 3 <= bash * 2


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
        if not within(direct, bash) and 0.90 * current <= direct <= 1.10 * current:
            return "matched OS control"
    return "tried, above target"


def records(root):
    frozen = {row["id"]: row for row in map(json.loads, MANIFEST.read_text().splitlines())}
    expected = {(lane, group, name) for row in frozen.values() for lane, config in row["lanes"].items()
                for group, name in config["features"]}
    seen = set()
    for file in sorted([*root.glob("*/replay.jsonl"), *root.glob("*/additional.jsonl")]):
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
                times = fixed["samples_ns"]
                for group, name in config["features"]:
                    key = (lane, group, name)
                    if key in seen:
                        raise ValueError(f"duplicate feature: {key}")
                    seen.add(key)
                    data = dict(lane=lane, group=group, feature=name, id=result["id"],
                                cohort="previous misses" if key in expected else "new misses",
                                verdict=classify(fixed, extra), iterations=fixed["count"])
                    for label, stage in (("fixed", fixed), ("long", extra)):
                        for build in ("bash", "baseline", "current", "opt-3", "opt-s", "direct"):
                            values = stage.get("samples_ns", {}).get(build, [])
                            data[f"{label}_{build}_p50_ns"] = round(median(values)) if values else ""
                            data[f"{label}_{build}_p95_ns"] = sorted(values)[math.ceil(.95*len(values))-1] if values else ""
                    data.update(long_iterations=extra.get("count", ""), old_tonari_ns=config["old_ns"][0],
                                old_bash_ns=config["old_ns"][1], control=json.dumps(fixed.get("control")),
                                argv=json.dumps(row["argv"], ensure_ascii=False))
                    yield data
    missing = expected - seen
    if missing:
        raise ValueError(f"{len(missing)} previous observations missing, e.g. {sorted(missing)[:3]}")


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("artifacts", type=Path)
    cli.add_argument("ledger", type=Path)
    args = cli.parse_args()
    rows = sorted(records(args.artifacts), key=lambda r: (r["lane"], r["group"], r["feature"]))
    with args.ledger.open("w") as out:
        writer = csv.DictWriter(out, fieldnames=list(rows[0]), delimiter="\t", lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


if __name__ == "__main__":
    main()
