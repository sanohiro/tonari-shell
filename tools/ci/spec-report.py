"""Join complete native shards; preserve each verdict, timing and execution-path index."""
import argparse
import csv
import gzip
import io
import json
from collections import Counter
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


def unique(table, key, value):
    if key in table:
        raise ValueError(f"duplicate {key}")
    table[key] = value


def scripts(directory, host, name, target):
    found = {}
    want = set()
    for d in sorted(directory.glob(f"{host}-{name}*")):
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
            fields = dict(name=m["name"], bash=g["bash"], tonari=g["tonari"], pair=pair,
                          observable="same" if same else "different", comparable=usable,
                          samples=min(map(len, r["ms"].values())), bash_ms=times[0],
                          tonari_ms=times[1], ratio=times[1]/times[0] if usable else None,
                          sites=r["sites"], location=m.get("line", m["index"]),
                          assertion="Bash annotations" if oils else m["kind"])
            unique(found, k, fields)
    if not found or set(found) != want:
        raise ValueError(f"{host}/{name} missing cases: {len(want - found.keys())}")
    for k, r in found.items():
        unique(target, (name, k), r)


def interactive(directory, host, inventory, target):
    pairs = {}
    for d in sorted(directory.glob(f"{host}-bensch-*")):
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
        check = inventory[k]
        usable = pair == "both" and len(ae) == len(be) and bool(steps) and check != "none"
        unique(target, ("bensch-pty", k), dict(name=a["name"], bash=gb, tonari=ga, pair=pair,
               observable="upstream PTY matcher", comparable=usable, samples=len(steps),
               bash_ms=times[1], tonari_ms=times[0], ratio=times[0]/times[1] if usable else None,
               sites=sides.get("observer", {}).get("sites", []), assertion=check,
               location=a["index"]))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for arg in ("scripts", "pty", "bensch", "output"):
        p.add_argument(arg, type=Path)
    args = p.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    inventory = {}
    for path in (args.bensch / "suites/interactive").rglob("*.yaml"):
        for i, test in enumerate(yaml.safe_load(path.read_text())["tests"], 1):
            check = next((k for k in ("expect_output", "expect_not") if k in test), None)
            if not check:
                check = "step assertion" if any("expect" in s or "wait_for_prompt" in s for s in test["steps"]) else "none"
            unique(inventory, f"{path.stem}/{i}", check)
    for host in ("ubuntu-24.04", "macos-15"):
        rows = {}
        scripts(args.scripts, host, "oils", rows)
        scripts(args.scripts, host, "spec-builtins", rows)
        interactive(args.pty, host, inventory, rows)
        header = ["suite", "case", "name", "location", "assertion", "bash", "tonari", "pair",
                  "observable", "samples", "bash_ms", "tonari_ms", "ratio", "sites"]
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
        compressed(args.output / (host + ".tsv.gz"), header, table)
        for suite in ("oils", "spec-builtins", "bensch-pty"):
            data = [r for (s, _), r in rows.items() if s == suite]
            ratios = [r["ratio"] for r in data if r["ratio"] is not None]
            print(host, suite, len(data), dict(Counter(r["pair"] for r in data)),
                  "comparable", len(ratios), "within-2/3", sum(v <= 2/3 for v in ratios),
                  "above-bash", sum(v > 1 for v in ratios))


if __name__ == "__main__":
    main()
