"""Join complete native shards; preserve each verdict, timing and execution-path index."""
import argparse
import base64
import csv
import gzip
import io
import json
import re
from collections import Counter, defaultdict
from pathlib import Path
from statistics import median
import yaml


def records(path):
    with path.open() as f:
        for line in f:
            yield json.loads(line)


def compressed(path, header, rows):
    with gzip.GzipFile(filename=str(path), mode="wb", mtime=0) as gz:
        with io.TextIOWrapper(gz, encoding="utf-8", newline="") as f:
            csv.writer(f, delimiter="\t", lineterminator="\n").writerows([header, *rows])


def verdict(bash, tonari):
    invalid = {"FAIL", "TIME", "TIMEOUT", "OUTPUT", "OUTPUT_LIMIT"}
    if bash not in invalid:
        return "both" if tonari not in invalid else "bash-only"
    return "tonari-only" if tonari not in invalid else "neither"


def detail(a, b):
    parts = [f"status {a['status']}/{b['status']}"]
    for field in ("stdout", "stderr"):
        left, right = [base64.b64decode(actual[field]) for actual in (a, b)]
        if left == right:
            continue
        i = next((i for i, (x, y) in enumerate(zip(left, right)) if x != y), min(len(left), len(right)))
        start = max(0, i - 16)
        parts.append(f"{field}[{i}] {left[start:i+80]!r} / {right[start:i+80]!r}")
    return " | ".join(parts)


def unique(table, key, value):
    if key in table:
        raise ValueError(f"duplicate {key}")
    table[key] = value


def scripts(directory, host, name, target):
    found = {}
    want = set()
    pattern = "oils-[0-3]" if name == "oils" else name
    for d in sorted(directory.glob(f"{host}-{pattern}")):
        manifest = {r["key"]: r for r in records(d / "manifest.jsonl")}
        want.update(k for k, r in manifest.items() if r["include"])
        oils = name == "oils"
        grading = {r["key"]: r["grades"] for r in records(d / "grades.jsonl")} if oils else {}
        for r in records(d / "cases.jsonl"):
            k = r["key"]
            m = manifest[k]
            g = grading[k] if oils else r["grades"]
            a, b = r["actual"]["tonari"], r["actual"]["bash"]
            same = all(a[f] == b[f] for f in ("status", "stdout", "stderr"))
            pair = verdict(g["bash"], g["tonari"])
            times = [median(r["ms"][s]) for s in ("bash", "tonari")]
            usable = (pair == "both" and same and not r["unstable"] and
                      min(len(t) for t in r["ms"].values()) >= 3)
            helpers = sorted(set(re.findall(rb"\b(?:[A-Za-z0-9_-]+\.py|python2|sleep)\b",
                                            base64.b64decode(m["code"]))))
            fields = dict(source_mentions=",".join(h.decode() for h in helpers), name=m["name"], bash=g["bash"], tonari=g["tonari"], pair=pair,
                          observable="same" if same else "different", unstable=",".join(r["unstable"]), comparable=usable,
                          samples=min(map(len, r["ms"].values())), bash_ms=times[0],
                          tonari_ms=times[1], ratio=times[1]/times[0] if usable else None,
                          bash_case_ms=times[0], tonari_case_ms=times[1], sites=r["sites"], location=m.get("line", m["index"]),
                          assertion="Bash annotations" if oils else m["kind"],
                          detail=detail(b, a) if pair != "both" else "")
            unique(found, k, fields)
    if not found or set(found) != want:
        raise ValueError(f"{host}/{name} missing cases: {len(want - found.keys())}")
    for k, r in found.items():
        unique(target, (name, k), r)


def interactive(directory, host, inventory, target):
    pairs = {}
    for d in sorted(directory.glob(f"{host}-bensch-[0-7]")):
        for r in records(d / "pty.jsonl"):
            k = f"{r['suite']}/{r['index']}"
            unique(pairs.setdefault(k, {}), r["shell"], r)
    if pairs.keys() != inventory.keys():
        raise ValueError(f"{host} PTY inventory mismatch: {len(pairs)} of {len(inventory)}")
    for k, sides in pairs.items():
        a, b = sides["tonari"], sides["bash"]
        ga, gb = ["PASS" if v["passed"] else "FAIL" for v in (a, b)]
        pair = verdict(gb, ga)
        ae, be = a["first_byte_us"], b["first_byte_us"]
        steps = [(x / 1000, y / 1000) for x, y in zip(ae, be) if x is not None and y is not None]
        times = [median(v) for v in zip(*steps)] if steps else [None, None]
        check, step_types = inventory[k]
        usable = pair == "both" and len(ae) == len(be) and bool(steps) and check != "none"
        unique(target, ("bensch-pty", k), dict(name=a["name"], bash=gb, tonari=ga, pair=pair,
               observable="upstream PTY matcher", unstable="single run", comparable=usable, samples=len(steps),
               bash_ms=times[1], tonari_ms=times[0], ratio=times[0]/times[1] if usable else None,
               bash_case_ms=b["seconds"] * 1000, tonari_case_ms=a["seconds"] * 1000,
               sites=sides.get("observer", {}).get("sites", []), assertion=check,
               location=a["index"], source_mentions=step_types, detail=" / ".join(v["error"][:320] for v in (b, a)) if pair != "both" else ""))


def helper_control(directory, host, helper):
    def load(name):
        rows = {}
        for r in records(directory / (host + "-oils-" + helper) / (name + ".jsonl")):
            unique(rows, r["key"], r)
        return rows

    upstream, native = load("upstream"), load("native")
    if upstream.keys() != native.keys() or not upstream:
        raise ValueError(f"{host} {helper} helper inventory mismatch")
    for key, a in sorted(upstream.items()):
        b = native[key]
        times = [median(r["ms"][shell]) for r in (a, b) for shell in ("bash", "tonari")]
        unchanged = a["actual"] == b["actual"]
        comparable = (unchanged and a["actual"]["tonari"] == a["actual"]["bash"]
                      and not any(r["unstable"] or any(v["limit"] for v in r["actual"].values())
                                  for r in (a, b)))
        yield [host, helper, key, int(unchanged), int(comparable), *[f"{v:.6f}" for v in times],
               *([f"{times[1]/times[0]:.6f}", f"{times[3]/times[2]:.6f}"] if comparable else ["", ""])]


def profile_trials(directory, host, expected):
    seen = set()
    for file in sorted(directory.glob(f"{host}-*/profiles.jsonl")):
        suite = "oils" if "-oils-profile-" in file.parent.name else "spec-builtins"
        for r in records(file):
            key = (suite, r["key"])
            if key in seen:
                raise ValueError(f"duplicate optimization trial {key}")
            seen.add(key)
            a, b = r["actual"]["bash"], r["actual"]["tonari"]
            valid = a == b and not a["limit"] and not r["unstable"]
            o2, o3 = [median(r["ms"][k]) for k in ("bash", "tonari")]
            yield [host, *key, int(a == b), ",".join(r["unstable"]).replace("bash", "O2").replace("tonari", "O3"),
                   f"{o2:.6f}", f"{o3:.6f}", f"{o3/o2:.6f}" if valid else ""]
    if seen != expected:
        raise ValueError(f"{host} optimization trial inventory mismatch")


def baseline_changes(before, after, host):
    if before.keys() != after.keys():
        raise ValueError(f"{host} baseline inventory mismatch")
    for key, old in sorted(before.items()):
        new = after[key]
        if (old["bash"], old["tonari"]) != (new["bash"], new["tonari"]):
            yield [host, *key, new["name"], old["bash"], old["tonari"], new["bash"], new["tonari"]]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for arg in ("scripts", "pty", "bensch", "output"):
        p.add_argument(arg, type=Path)
    p.add_argument("--controls", type=Path)
    p.add_argument("--profiles", type=Path)
    p.add_argument("--baseline-scripts", type=Path)
    p.add_argument("--baseline-pty", type=Path)
    args = p.parse_args()
    if bool(args.baseline_scripts) != bool(args.baseline_pty):
        p.error("provide both baseline artifact directories")
    args.output.mkdir(parents=True, exist_ok=True)
    inventory = {}
    for path in (args.bensch / "suites/interactive").rglob("*.yaml"):
        for i, test in enumerate(yaml.safe_load(path.read_text())["tests"], 1):
            check = next((k for k in ("expect_output", "expect_not") if k in test), None)
            if not check:
                check = "step assertion" if any("expect" in s or "wait_for_prompt" in s for s in test["steps"]) else "none"
            steps = test["steps"]
            kinds = sorted({step["send_key"] if "send_key" in step else op
                            for step in steps for op in step})
            unique(inventory, f"{path.stem}/{i}", (check, ",".join(kinds)))
    changes = []
    comparisons = []
    controls = []
    profiles = []
    summaries = []
    gaps = []
    sites = []
    for root in (args.scripts, args.pty):
        for file in root.glob("*/sites.json"):
            found = json.loads(file.read_text())
            count = min(len(found), len(sites))
            if found[:count] != sites[:count]:
                raise ValueError(f"site definitions changed in {file}")
            if len(found) > len(sites):
                sites = found
    with (args.output / "sites.tsv").open("w", newline="") as f:
        writer = csv.writer(f, delimiter="\t", lineterminator="\n")
        writer.writerow(["site", "module", "function", "review"])
        writer.writerows([i, r["module"], r["function"], r["review"]] for i, r in enumerate(sites))

    for host in ("ubuntu-24.04", "macos-15"):
        rows = {}
        scripts(args.scripts, host, "oils", rows)
        scripts(args.scripts, host, "spec-builtins", rows)
        if args.profiles:
            profile_host = "physical-m1" if host == "macos-15" else host
            profiles.extend(profile_trials(args.profiles, profile_host, set(rows)))
        interactive(args.pty, host, inventory, rows)
        if args.controls:
            for helper in ("argv", "printenv"):
                controls.extend(helper_control(args.controls, host, helper))
        if args.baseline_scripts:
            before = {}
            for suite in ("oils", "spec-builtins"):
                scripts(args.baseline_scripts, host, suite, before)
            interactive(args.baseline_pty, host, inventory, before)
            changes.extend(baseline_changes(before, rows, host))
            for key in sorted(rows):
                values = [v[field] for v in (before[key], rows[key])
                          for field in ("bash_ms", "tonari_ms", "ratio")]
                comparisons.append([host, *key, *["" if v is None else f"{v:.6f}" for v in values]])
        header = ["suite", "case", "name", "location", "assertion", "bash", "tonari", "pair",
                  "observable", "unstable", "samples", "bash_ms", "tonari_ms", "ratio", "bash_case_ms", "tonari_case_ms", "sites", "source_mentions"]
        groups = defaultdict(list)
        table = []
        for (suite, key), r in sorted(rows.items()):
            def cell(k):
                v = r[k]
                if k == "sites":
                    return ",".join(map(str, v))
                if isinstance(v, float):
                    return f"{v:.6f}"
                return v
            table.append([suite, key, *[cell(k) for k in header[2:]]])
            groups[(suite, key.rsplit("/", 1)[0])].append(r)
            if r["pair"] != "both":
                gaps.append([host, suite, key, r["name"], r["bash"], r["tonari"], r["detail"]])
        for (suite, group), group_rows in sorted(groups.items()):
            tally = Counter(r["pair"] for r in group_rows)
            ratios = [r["ratio"] for r in group_rows if r["ratio"] is not None]
            summaries.append([host, suite, group, len(group_rows),
                              *[tally[k] for k in ("both", "bash-only", "tonari-only", "neither")],
                              len(ratios), sum(v <= 2/3 for v in ratios), sum(v > 1 for v in ratios)])
        compressed(args.output / (host + ".tsv.gz"), header, table)
        for suite in ("oils", "spec-builtins", "bensch-pty"):
            data = [r for (s, _), r in rows.items() if s == suite]
            ratios = [r["ratio"] for r in data if r["ratio"] is not None]
            print(host, suite, len(data), dict(Counter(r["pair"] for r in data)),
                  "comparable", len(ratios), "within-2/3", sum(v <= 2/3 for v in ratios),
                  "above-bash", sum(v > 1 for v in ratios))

    if args.profiles:
        compressed(args.output / "profiles.tsv.gz", ["host", "suite", "case", "equal", "unstable_build",
                   "o2_ms", "o3_ms", "o3_over_o2"], profiles)
    if args.controls:
        compressed(args.output / "helpers.tsv.gz",
                   ["host", "helper", "case", "unchanged", "comparable", "original_bash_ms", "original_tonari_ms",
                    "native_bash_ms", "native_tonari_ms", "original_ratio", "native_ratio"], controls)
    if args.baseline_scripts:
        compressed(args.output / "before-after.tsv.gz", ["host", "suite", "case", "previous_bash_ms",
                   "previous_tonari_ms", "previous_ratio", "final_bash_ms", "final_tonari_ms", "final_ratio"],
                   comparisons)
        compressed(args.output / "changes.tsv.gz", ["host", "suite", "case", "name", "previous_bash",
                   "previous_tonari", "final_bash", "final_tonari"], changes)
    compressed(args.output / "differences.tsv.gz", ["host", "suite", "case", "name", "bash", "tonari", "bash / tonari evidence"], gaps)
    with (args.output / "groups.tsv").open("w", newline="") as stream:
        out = csv.writer(stream, delimiter="\t", lineterminator="\n")
        out.writerow(["host", "suite", "group", "cases", "both", "bash_only", "tonari_only", "neither",
                      "timing_pairs", "within_two_thirds", "above_bash"])
        out.writerows(summaries)


if __name__ == "__main__":
    main()
