"""Publish the named feature matrix and check a new run against a previous snapshot."""
import argparse
import csv
import json
import re
from pathlib import Path
import sys
import tempfile

PASS = {"match", "golden match"}
CODES = {"intentional difference":"◎", "contract violation":"!", "match":"✓", "golden match":"G", "different":"Δ", "stderr differs":"E",
         "missing interface":"M", "missing default":"V", "interface only":"I", "index":"§",
         "mode unavailable":"P", "control inconclusive":"?", "fixture unavailable":"F",
         "not in reference":"N", "separate export-function probe":"↗", "timeout":"T",
         "context unprobed":"?", "not independently probed":"?", "unprobed":"?", "partial fixture":"F"}
METRICS = ("whole", "started")


def table(path):
    with path.open(newline="") as f:
        return {(r["group"], r["feature"]): r for r in csv.DictReader(f, delimiter="\t", quoting=csv.QUOTE_NONE)}


def acceptable(row, mode, limit):
    if not row.get(mode + "_n"):
        return False
    return int(row[mode + "_tonari_p50_ns"]) * limit[1] <= int(row[mode + "_bash_p50_ns"]) * limit[0]


def observation_lost(before, after):
    if before == "intentional difference":
        return after != before and after not in PASS
    return before in PASS and after not in PASS


def misses(rows, previous, require_target, require_complete):
    errors = []
    for key, before in previous.items():
        after = rows.get(key)
        if after is None:
            errors.append((*key, "dropped from inventory"))
            continue
        if observation_lost(before["status"], after["status"]):
            errors.append((*key, "previously verified observation lost"))
        # The batching policy follows single-command timings, so a feature can
        # legitimately leave the repeated subset or change iteration count.
        if (before.get("batch_n") and after.get("batch_n")
                and int(before["batch_iterations"]) == int(after["batch_iterations"])
                and acceptable(before, "batch", (2, 3)) and not acceptable(after, "batch", (2, 3))):
            errors.append((*key, "repeated command crossed 2/3"))
        for mode in METRICS:
            if before["status"] in PASS and before[mode + "_n"] and not after[mode + "_n"]:
                errors.append((*key, mode + " measurement lost"))
            elif before["status"] in PASS and acceptable(before, mode, (2,3)) and not acceptable(after, mode, (2,3)):
                errors.append((*key, mode + " crossed 2/3"))
    for key, row in rows.items():
        if row["status"] in PASS:
            if require_target and row.get("batch_n") and not acceptable(row, "batch", (2, 3)):
                errors.append((*key, "repeated command exceeds 2/3"))
            for mode in METRICS:
                if require_target and (row[mode + "_error"] or mode == "whole" or row[mode + "_n"]) and not acceptable(row, mode, (2,3)):
                    errors.append((*key, mode + " exceeds 2/3 or lacks verified samples"))
        elif require_complete and row["status"] not in ("index", "not in reference", "separate export-function probe", "intentional difference"):
            errors.append((*key, row["status"]))
    return errors


def records(data):
    result = {}
    for group, names in data["rows"].items():
        for name, item in names.items():
            row = {"group": group, "feature": name, "status": item["status"]}
            batch = item.get("batch_timing", {})
            row["batch_iterations"] = batch.get("iterations", "")
            row["batch_reason"] = batch.get("reason", "")
            for mode in (*METRICS, "batch"):
                measurement = batch if mode == "batch" else item.get("timing", {}).get(mode, {})
                row[mode + "_n"] = measurement.get("n", "")
                for shell in ("bash", "tonari"):
                    for i, quantile in enumerate(("p50", "p95")):
                        row[f"{mode}_{shell}_{quantile}_ns"] = measurement.get(shell, ("", ""))[i]
                row[mode + "_error"] = measurement.get("error", "")
            row["witness"] = json.dumps(item["witness"], ensure_ascii=False)
            if "driver" in item:
                row["evidence"] = json.dumps(item["driver"], sort_keys=True, ensure_ascii=False)
            else:
                evidence = item.get("observed", item.get("samples", None))
                row["evidence"] = json.dumps(evidence, ensure_ascii=False)
                if (group, name) == ("Invocation", "--debugger"):
                    row["evidence"] = re.sub(r'(?<![a-z])/[^\s"]+?/share/bashdb/', "$PREFIX/share/bashdb/", row["evidence"])
            result[group, name] = row
    return result


def cell(row):
    if not row:
        return "—"
    values = []
    for mode in METRICS:
        if row[mode + "_n"]:
            ratio = int(row[mode + "_tonari_p50_ns"]) / int(row[mode + "_bash_p50_ns"])
            # Round only for display; the gate uses integer nanoseconds.
            display = f"{ratio:.2f}" + ("↑" if ratio >= 1 else "")
            values.append(display if acceptable(row, mode, (2, 3)) else f"**{display}**")
        elif row[mode + "_error"]:
            values.append("×")
    return CODES[row["status"]] + (" " + " / ".join(values) if values else "")


def markdown(data):
    keys = sorted(set().union(*(d.keys() for d in data.values())))
    lines = ["# Bash feature matrix / Bash 機能別の結果", "",
             "[Method, scope and reproduction / 検証方法と対象範囲](../compatibility.md)", "",
             " · ".join(f"[{label} TSV]({label}.tsv)" for label in data) + " · [Environment](environment.json)", "",
             "Each cell gives the compatibility result and the median elapsed-time ratio (Tonari/Bash), whole run followed by already-started where applicable. Exact nanoseconds, p95, sample counts and witnesses are in each platform TSV. Bold ratios exceed 2/3 before rounding. An upward arrow marks Bash time or slower. Shared witnesses appear under each public name. Manual sections are index entries.",
             "", "各セルは互換判定と所要時間の中央値の比（Tonari/Bash）です。左が起動込み、右が起動後です。数値が一つなら起動込みのみです。太字は上限 2/3 を超えた値、↑ は Bash と同じか遅い値です。TSV にナノ秒単位の中央値・p95、サンプル数、検証内容を収録しました。同じ処理を参照する項目があり、マニュアルの節は索引として数えています。", "",
             "✓ match / 一致 · ◎ verified Tonari contract / 意図した差 · ! Tonari contract violation / 仕様違反 · G GNU golden match · Δ difference / 動作差 · E stderr difference · M missing interface · V missing default · I interface only · P POSIX mode unavailable · F fixture unavailable · ? inconclusive · N absent from reference · T timeout · ↗ separate function-export probe · § manual index · × observation changed during timing · — outside this lane.", ""]
    for group in sorted({g for g, _ in keys}):
        lines.extend([f"## {group}", "", "| Feature / 項目 | " + " | ".join(data) + " |",
                      "| --- | " + " | ".join("---" for _ in data) + " |"])
        for key in (key for key in keys if key[0] == group):
            label = key[1].replace("&", "&amp;").replace("<","&lt;").replace(">", "&gt;").replace("|", "&#124;").replace("`","&#96;")
            lines.append("| <code>" + label + "</code> | " + " | ".join(cell(d.get(key)) for d in data.values()) + " |")
        lines.append("")
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", nargs="+", metavar="PLATFORM=MEASURED.json")
    parser.add_argument("--output", type=Path, help="directory for the TSVs and matrix")
    parser.add_argument("--compare", type=Path, help="directory of published PLATFORM.tsv baselines")
    parser.add_argument("--require-target", action="store_true", help="require every matching witness to meet 2/3")
    parser.add_argument("--require-complete", action="store_true", help="fail on every missing, different or unverified behavior")
    args = parser.parse_args()
    datasets = {}
    environments = {}
    failed = False
    for spec in args.input:
        label, path = spec.split("=", 1)
        if not label.isalnum() or label in datasets:
            parser.error("use unique alphanumeric platform names")
        data = json.loads(Path(path).read_text())
        if "benchmark" not in data:
            parser.error("the input must contain a completed benchmark")
        environments[label] = {"reference": data["reference"], **data["benchmark"]}
        rows = records(data)
        datasets[label] = rows
        baseline = table(args.compare / (label + ".tsv")) if args.compare else {}
        problems = misses(rows, baseline, args.require_target, args.require_complete)
        if problems:
            failed = True
            for group, name, reason in problems:
                print(f"{label}\t{group}\t{name}\t{reason}", file=sys.stderr)
        print(f"{label}: {len(rows)} rows; {sum(r['status'] in PASS for r in rows.values())} matching witnesses")
        for mode in (*METRICS, "batch"):
            eligible = [r for r in rows.values() if r["status"] in PASS and r[mode + "_n"]]
            counts = [sum(acceptable(r, mode, bound) for r in eligible) for bound in ((1,2),(2,3),(1,1))]
            print(f"  {mode}: {len(eligible)} measured, {counts[0]} at half, {counts[1]} at 2/3, {counts[2]} at parity or better")
    if args.output:
        if failed and args.compare and args.compare.resolve() == args.output.resolve():
            print("Comparison failed. The published snapshot was preserved.", file=sys.stderr)
            sys.exit(1)
        args.output.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=args.output) as temp:
            stage = Path(temp)
            for label, rows in datasets.items():
                with (stage / (label + ".tsv")).open("w", newline="") as f:
                    writer = csv.DictWriter(f, fieldnames=next(iter(rows.values())).keys(),
                                            delimiter="\t", lineterminator="\n",
                                            quoting=csv.QUOTE_NONE, quotechar=None)
                    writer.writeheader()
                    writer.writerows(rows[key] for key in sorted(rows))
            (stage / "matrix.md").write_text(markdown(datasets))
            (stage / "environment.json").write_text(json.dumps(environments, indent=2) + "\n")
            for file in stage.iterdir():
                file.replace(args.output / file.name)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
