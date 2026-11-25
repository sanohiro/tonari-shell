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


# Sorted fixture filenames of yash 2.60, each terminated by a newline.
FIXTURE_DIGEST = "979cac7fd9399f5d8265c3c4fa8f6eadb175e0c326051f2f19c4ca214365420b"
# Sorted file:line identities, including skips. The title at test-p.tst:99
# embeds the first block-device path on the host that generated the case.
CASE_DIGEST = "bb901487192a51adbfde13c300bbc52a81936fd537be897c375d097ece60c8c2"


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


def build_identity(directory):
    machine = (directory / "host.txt").read_text()
    hashes = {name: digest for digest, name in re.findall(
        r"^([a-f0-9]{64})  .*/bin/(current|baseline)$", machine, re.M)}
    version = re.search(r"^GNU bash, version ([^\n]+)", machine, re.M)
    if set(hashes) != {"current", "baseline"} or not version:
        raise ValueError(f"{directory}: missing executable identity")
    return machine.splitlines()[0], hashes["current"], hashes["baseline"], version[1]


def repeated_trials(directory):
    cases = defaultdict(lambda: defaultdict(dict))
    fixtures = json.loads((directory / "fixtures.json").read_text())
    trials = defaultdict(set)
    for row in read_jsonl(directory / "samples.jsonl"):
        shell, fixture, n = (row[k] for k in ("shell", "fixture", "trial"))
        if n in trials[fixture, shell]:
            raise ValueError(f"duplicate recheck {fixture}/{shell}/{n}")
        trials[fixture, shell].add(n)
        for key, pair in row["cases"].items():
            cases[key][shell][n] = pair
    if any(trials[f,s] != set(range(11)) for f in fixtures for s in ("tonari", "bash")):
        raise ValueError(f"{directory}: incomplete higher-repeat measurements")
    return cases


def read_recheck(root, host, release, bash):
    if root is None:
        return {}
    directory = root / (host + "-external-yash-recheck")
    _, other_release, _, other_bash = build_identity(directory)
    if (other_release, other_bash) != (release, bash):
        raise ValueError(f"{host}: different executable or Bash version in higher-repeat run")
    if len(json.loads((directory / "fixtures.json").read_text())) != 7:
        raise ValueError(f"{host}: incomplete higher-repeat fixture set")
    return repeated_trials(directory)


def read_shard_repeats(files):
    directories = [file.parent / "repeats" for file in files]
    if not any(directory.exists() for directory in directories):
        return None
    cases = {}
    for directory in directories:
        selected = json.loads((directory / "selection.json").read_text())
        if not selected:
            continue
        by_id = {}
        # An unfiltered rerun supersedes the selected run for its whole fixture.
        for source in (directory, directory / "whole"):
            if source.is_dir():
                for key, series in repeated_trials(source).items():
                    by_id[key.split(": ", 1)[0]] = series
        for key in selected:
            series = by_id[key.split(": ", 1)[0]]
            if key in cases or any(
                set(series[label]) != set(range(11))
                or any(v[0] != "OK[PASSED]" or v[1] is None for v in series[label].values())
                for label in ("tonari", "bash")
            ):
                raise ValueError(f"{directory}: incomplete or duplicate replay for {key}")
            cases[key] = series
    return cases


def summarize(root, output, runs, recheck=None, replacements=(), hosts=("ubuntu-24.04", "macos-15")):
    cases, sites = defaultdict(dict), None
    output.mkdir(parents=True, exist_ok=True)
    totals = {}
    groups, differences, builds, misses = [], [], [], []
    for host in hosts:
        chosen = {}
        for directory in (root, *replacements):
            for file in directory.glob(host + "-external-yash-[0-7]/samples.jsonl"):
                chosen[file.parent.name] = file
        files = [chosen[name] for name in sorted(chosen)]
        digests = set()
        if len(files) != 8:
            raise ValueError(f"{host}: found {len(files)} of 8 shards")
        inputs, paths = {}, {}
        labels = ("tonari", "bash", "previous")
        fixtures = defaultdict(lambda: defaultdict(set))
        rows = defaultdict(lambda: defaultdict(dict))
        declared = set()
        failures = {label: {} for label in labels}
        for file in files:
            revision, release, baseline, bash = build_identity(file.parent)
            digests.add((release, baseline, bash))
            builds.append([file.parent.name, revision, release, baseline, bash])
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
                    if not key.split(":", 2)[1].isdecimal():
                        raise ValueError(f"{file}: missing upstream case identity {key}")
                    rows[key][shell][trial] = sample
            if present != set(selection):
                raise ValueError(f"{file}: missing or extra measured fixture")
            for record in read_jsonl(file.with_name("inputs.jsonl.gz")):
                inputs[record["case"]] = record
            for record in read_jsonl(file.with_name("paths.jsonl")):
                paths[record["case"]] = record
        if len(digests) > 1:
            raise ValueError(f"{host}: timed executable hashes or Bash versions differ across shards")
        release, _, bash = next(iter(digests))
        repeated = read_recheck(recheck, host, release, bash)
        shard_repeats = read_shard_repeats(files)
        if shard_repeats is not None:
            if shard_repeats.keys() - rows.keys():
                raise ValueError(f"{host}: replay contains an unknown case")
            repeated.update(shard_repeats)
        manifest = "\n".join(sorted(declared)) + "\n"
        if hashlib.sha256(manifest.encode()).hexdigest() != FIXTURE_DIGEST:
            raise ValueError(f"{host}: unexpected pinned fixture inventory")
        names = "\n".join(sorted(key.split(": ", 1)[0] for key in rows)) + "\n"
        if hashlib.sha256(names.encode()).hexdigest() != CASE_DIGEST:
            raise ValueError(f"{host}: incomplete or changed upstream case inventory")
        if inputs.keys() != rows.keys() or paths.keys() != rows.keys():
            raise ValueError(f"{host}: generated input or trace verdict missing from case inventory")
        for fixture, obs in fixtures.items():
            if any(obs[label] != set(range(runs)) for label in labels):
                raise ValueError(f"{host}/{fixture}: incomplete shell trials")
        stats = Counter()
        ratios = []
        by_fixture = defaultdict(Counter)
        fixture_ratios = defaultdict(list)
        for key in sorted(rows):
            series = rows[key]
            statuses, elapsed = {}, {}
            for label in labels:
                if len(series[label]) != runs:
                    raise ValueError(f"{host}/{key}: incomplete {label} trials")
                seq = [series[label][n] for n in range(runs)]
                statuses[label] = "|".join(sorted({v[0] for v in seq}))
                elapsed[label] = [v[1] / 1000 for v in seq if v[1] is not None]
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
            for shell, status in statuses.items():
                stats[shell + " stable passes"] += status == passed
                stats[shell + " stable skips"] += status == "SKIPPED"
                stats[shell + " has timeout"] += "TIMEOUT" in status
                stats[shell + " has controller exit"] += "CONTROLLER_EXIT" in status
            stats[verdict] += 1
            fixture = key.split(":", 1)[0]
            by_fixture[fixture][verdict] += 1
            ratio = ""
            if verdict == "both pass" and min(len(elapsed[k]) for k in labels[:2]) == runs:
                ratio = median(elapsed["tonari"])/median(elapsed["bash"])
                ratios.append(ratio)
                fixture_ratios[fixture].append(ratio)
            if shard_repeats is not None:
                above_target = ratio != "" and (3 * median(v[1] for v in series["tonari"].values())
                                                 > 2 * median(v[1] for v in series["bash"].values()))
                if above_target != (key in shard_repeats):
                    raise ValueError(f"{host}/{key}: replay selection differs from measured target")
            row = [key, verdict, t, b, p]
            row += [round(median(elapsed[k]), 1) if elapsed[k] else "" for k in labels]
            row += [round(ratio, 4) if ratio != "" else ""]
            path = paths[key]
            ids = path["sites"]
            row += [path["status"], " ".join(str(i) for i in ids)]
            inp = inputs[key]
            row += [hashlib.sha256(json.dumps([inp["argv"], inp["source"]]).encode()).hexdigest()[:16]]
            row += [failures[k].get(key, "") for k in labels]
            row += [",".join(f"{v:.3f}" for v in elapsed[k]) for k in labels]
            medians = []
            for label in labels[:2]:
                check = repeated.get(key, {}).get(label, {})
                ns = [v[1] for v in check.values() if v[0] == passed and v[1] is not None]
                medians.append(median(ns)/1000 if len(ns) == 11 else None)
            row.extend(round(m, 1) if m is not None else "" for m in medians)
            row.append(round(medians[0]/medians[1], 4) if all(medians) else "")
            if ratio != "" and ratio > 2/3:
                misses.append([host, key, round(ratio, 4), round(median(elapsed["tonari"]), 1),
                               round(median(elapsed["bash"]), 1), row[-1], " ".join(map(str, ids))])
            if all(medians):
                stats["11-trial comparable"] += 1
                stats["11-trial faster median"] += medians[0] < medians[1]
                stats["11-trial at most two thirds"] += medians[0] <= medians[1] * 2/3
            cases[host][key] = row
            if t != b or verdict == "variable":
                differences.append([host, key, verdict, t, b, failures["tonari"].get(key, "—"),
                                    failures["bash"].get(key, "—")])
            if ids:
                stats["traced"] += 1
            if t == b == passed and p != passed:
                stats["now passes with Bash"] += 1
            if b == p == passed and t != passed:
                stats["lost or inconsistent pass"] += 1
        stats["unique cases"] = len(cases[host])
        stats["inputs saved"] = len(inputs)
        stats["trace verdicts"] = len(paths)
        stats["fixtures"] = len(fixtures)
        stats.update(ratio_summary(ratios))
        totals[host] = dict(stats)
        # Yash generates thousands of cases by varying signal dispositions.
        # Publish the ordinary fixtures separately so their costs stay visible.
        ordinary = [name for name in sorted(declared) if not name.startswith("sig")]
        totals[host]["ordinary fixture results"] = {
            "fixtures": len(ordinary),
            "unique cases": sum(sum(by_fixture[name].values()) for name in ordinary),
            **ratio_summary([ratio for name in ordinary for ratio in fixture_ratios[name]]),
        }
        for fixture in sorted(declared):
            counts = by_fixture[fixture]
            groups.append(dict(host=host, fixture=fixture, cases=sum(counts.values()),
                               **ratio_summary(fixture_ratios[fixture]),
                               **{k: counts[k] for k in ("only Bash passes", "only Tonari passes", "variable")}))
        with ExitStack() as stack:
            out = stack.enter_context((output / (host + ".tsv.gz")).open("wb"))
            gz = stack.enter_context(gzip.GzipFile(fileobj=out, mode="wb", mtime=0))
            stream = stack.enter_context(io.TextIOWrapper(gz, encoding="utf-8", newline=""))
            writer = csv.writer(stream, delimiter="\t", lineterminator="\n")
            writer.writerow(["upstream case", "comparison", *labels, *(k + " median us" for k in labels),
                             "tonari/bash", "trace verdict", "site IDs", "input digest",
                             *(k + " first-trial assertions missed" for k in labels), *(k + " samples us" for k in labels),
                             "11-trial Tonari median us", "11-trial Bash median us", "11-trial tonari/bash"])
            writer.writerows(cases[host].values())
    with (output / "fixtures.tsv").open("w") as stream:
        writer = csv.DictWriter(stream, fieldnames=list(groups[0]), delimiter="\t", lineterminator="\n")
        writer.writeheader()
        writer.writerows(groups)
    with (output / "above-target.tsv").open("w") as stream:
        writer = csv.writer(stream, delimiter="\t", lineterminator="\n")
        writer.writerow(["host", "upstream case", "tonari/bash", "tonari us", "bash us",
                         "11-trial tonari/bash", "site IDs"])
        writer.writerows(sorted(misses, key=lambda m: (m[0], -m[2], m[1])))
    with (output / "builds.tsv").open("w") as stream:
        writer = csv.writer(stream, delimiter="\t", lineterminator="\n")
        writer.writerow(["shard", "harness commit", "release SHA-256", "previous SHA-256", "Bash version"])
        writer.writerows(builds)
    with (output / "differences.tsv").open("w") as stream:
        writer = csv.writer(stream, delimiter="\t", lineterminator="\n")
        writer.writerow(["host", "upstream case", "comparison", "tonari", "bash",
                         "tonari first-trial assertions missed", "bash first-trial assertions missed"])
        writer.writerows(differences)
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
    parser.add_argument("--recheck", type=Path, help="higher-repeat native artifacts")
    parser.add_argument("--replacement", type=Path, action="append", default=[],
                        help="whole replacement shards with the same timed binary hashes")
    parser.add_argument("--host", action="append", choices=("ubuntu-24.04", "macos-15"),
                        help="summarize one complete native host")
    args = parser.parse_args()
    print(json.dumps(summarize(args.artifacts, args.output, args.runs, args.recheck, args.replacement,
                               args.host or ("ubuntu-24.04", "macos-15")), indent=2))
