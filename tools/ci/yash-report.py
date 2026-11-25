"""Summarize native yash POSIX artifacts, preserving every verdict and timed case."""
import argparse
import csv
import gzip
import hashlib
import json
import io
import re
from collections import Counter, defaultdict
from contextlib import ExitStack
from pathlib import Path
from statistics import median


def read_jsonl(path):
    with (gzip.open(path, "rt") if path.suffix == ".gz" else path.open()) as stream:
        yield from map(json.loads, stream)


def assertion_errors(log):
    # The upstream report includes only the assertions requested by that case.
    with gzip.open(log, "rt", errors="replace") as stream:
        content = stream.read()
    for match in re.finditer(r"^%%% START: ([^\n]+)\n(.*?)(?=^%%% |\Z)", content, re.M | re.S):
        key, body = match.groups()
        errors = []
        status = re.search(r"^% exit status: expected=(\S+) actual=(\d+)(?:\(([^)]+)\))?", body, re.M)
        if status:
            expected, actual, signal = status.groups()
            observed = signal or actual
            if (actual == "0" if expected == "non-zero" else expected not in (actual, observed)):
                errors.append("exit=" + observed + " expected=" + expected)
        for channel in ("output", "error"):
            if re.search(r"^% standard " + channel + r" diff:\n(?:--- |\*\*\* |[0-9]+[acd])", body, re.M):
                errors.append("std" + ("out" if channel == "output" else "err"))
        if re.search(r"^% standard error \(expecting non-empty output\):\n\n(?:\Z|[%@])", body, re.M):
            errors.append("missing stderr")
        if errors:
            yield key, ", ".join(errors)


def ratio_summary(ratios):
    return {"comparable": len(ratios), "at most two thirds": sum(r <= 2/3 for r in ratios),
            "faster median": sum(r < 1 for r in ratios),
            "median case ratio": round(median(ratios), 4) if ratios else ""}


def summarize(root, output, runs):
    cases, sites = defaultdict(dict), None
    output.mkdir(parents=True, exist_ok=True)
    totals = {}
    expected_fixtures = None
    groups = []
    for host in ("ubuntu-24.04", "macos-15"):
        files = sorted(root.glob(host + "-external-yash-*/samples.jsonl"))
        if len(files) != 8:
            raise ValueError(f"{host}: found {len(files)} of 8 shards")
        inputs, paths = {}, {}
        labels = ("tonari", "bash", "previous")
        fixtures = defaultdict(lambda: defaultdict(set))
        rows = defaultdict(lambda: defaultdict(dict))
        declared = set()
        failures = {label: {} for label in labels}
        for file in files:
            selection = json.loads(file.with_name("fixtures.json").read_text())
            if declared.intersection(selection) or len(set(selection)) != len(selection):
                raise ValueError(f"{host}: overlapping fixtures")
            declared.update(selection)
            present = set()
            for label in labels:
                for log in (file.parent / label).glob("*.trs.gz"):
                    failures[label].update(assertion_errors(log))
            observed_sites = json.loads(file.with_name("sites.json").read_text())
            if sites is not None and observed_sites != sites:
                raise ValueError("inconsistent path instrumentation")
            sites = observed_sites
            for record in read_jsonl(file):
                fixture, shell, trial = (record[k] for k in ("fixture", "shell", "trial"))
                present.add(fixture)
                trials = fixtures[fixture][shell]
                if trial in trials:
                    raise ValueError(f"duplicate trial in {fixture}/{shell}")
                trials.add(trial)
                for key, sample in record["cases"].items():
                    rows[key][shell][trial] = sample
            if present != set(selection):
                raise ValueError(f"{file}: missing or extra measured fixture")
            for record in read_jsonl(file.with_name("inputs.jsonl.gz")):
                inputs[record["case"]] = record
            for record in read_jsonl(file.with_name("paths.jsonl")):
                paths[record["case"]] = record
        if len(declared) != 121 or (expected_fixtures is not None and declared != expected_fixtures):
            raise ValueError(f"{host}: unexpected pinned fixture inventory")
        expected_fixtures = declared
        for fixture, obs in fixtures.items():
            if any(obs[label] != set(range(runs)) for label in labels):
                raise ValueError(f"{host}/{fixture}: incomplete shell trials")
        stats = Counter()
        ratios = []
        by_fixture = defaultdict(Counter)
        fixture_ratios = defaultdict(list)
        for key in sorted(rows.keys() | paths.keys() | inputs.keys()):
            series = rows[key]
            statuses, elapsed = {}, {}
            for label in labels:
                seq = [series[label].get(n, ("ABSENT", None)) for n in range(runs)]
                statuses[label] = "|".join(sorted({v[0] for v in seq}))
                elapsed[label] = [round(v[1]/1000, 1) for v in seq if v[1] is not None]
            t, b, p = (statuses[k] for k in labels)
            passed = "OK[PASSED]"
            if "|" in t or "|" in b:
                verdict = "variable"
            elif t == b == passed:
                verdict = "both pass"
            elif t == b:
                verdict = "both " + t
            elif t == passed:
                verdict = "only Tonari passes"
            elif b == passed:
                verdict = "only Bash passes"
            else:
                verdict = "different non-pass"
            stats[verdict] += 1
            fixture = key.split(":", 1)[0]
            by_fixture[fixture][verdict] += 1
            ratio = ""
            if verdict == "both pass" and min(len(elapsed[k]) for k in labels[:2]) == runs:
                ratio = median(elapsed["tonari"])/median(elapsed["bash"])
                ratios.append(ratio)
                fixture_ratios[fixture].append(ratio)
            row = [key, verdict, t, b, p]
            row += [round(median(elapsed[k]), 1) if elapsed[k] else "" for k in labels]
            row += [round(ratio, 4) if ratio != "" else ""]
            path = paths.get(key, {})
            ids = path.get("sites", [])
            row += [path.get("status", "ABSENT"), " ".join(str(i) for i in ids)]
            inp = inputs.get(key)
            row += [hashlib.sha256(json.dumps([inp["argv"], inp["source"]]).encode()).hexdigest()[:16] if inp else ""]
            row += [failures[k].get(key, "") for k in labels]
            row += [",".join(map(str, elapsed[k])) for k in labels]
            stats["inputs saved"] += (inp is not None)
            stats["trace verdicts"] += bool(path)
            cases[host][key] = row
            if ids:
                stats["traced"] += 1
            if t == b == passed and p != passed:
                stats["now passes with Bash"] += 1
            if b == p == passed and t != passed:
                stats["lost or inconsistent pass"] += 1
        stats["unique cases"] = len(cases[host])
        stats["fixtures"] = len(fixtures)
        stats.update(ratio_summary(ratios))
        for fixture in sorted(declared):
            counts = by_fixture[fixture]
            groups.append(dict(host=host, fixture=fixture, cases=sum(counts.values()),
                               **ratio_summary(fixture_ratios[fixture]),
                               **{k: counts[k] for k in ("only Bash passes", "only Tonari passes", "variable")}))
        totals[host] = dict(stats)
        with ExitStack() as stack:
            out = stack.enter_context((output / (host + ".tsv.gz")).open("wb"))
            gz = stack.enter_context(gzip.GzipFile(fileobj=out, mode="wb", mtime=0))
            stream = stack.enter_context(io.TextIOWrapper(gz, encoding="utf-8", newline=""))
            writer = csv.writer(stream, delimiter="\t", lineterminator="\n")
            writer.writerow(["upstream case", "comparison", *labels, *(k + " median us" for k in labels),
                             "tonari/bash", "trace verdict", "site IDs", "input digest",
                             *(k + " first-trial assertions missed" for k in labels), *(k + " samples us" for k in labels)])
            writer.writerows(cases[host].values())
    with (output / "fixtures.tsv").open("w") as stream:
        writer = csv.DictWriter(stream, fieldnames=list(groups[0]), delimiter="\t", lineterminator="\n")
        writer.writeheader()
        writer.writerows(groups)
    with (output / "sites.tsv").open("w") as stream:
        writer = csv.writer(stream, delimiter="\t", lineterminator="\n")
        writer.writerow(["site ID", "function", "work inspected"])
        for i, site in enumerate(sites):
            writer.writerow([i, f'src/{site["module"]}.rs {site["function"]}', site["review"]])
    (output / "summary.json").write_text(json.dumps(totals, indent=2) + "\n")
    return totals


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifacts", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--runs", type=int, default=3)
    args = parser.parse_args()
    print(json.dumps(summarize(args.artifacts, args.output, args.runs), indent=2))
