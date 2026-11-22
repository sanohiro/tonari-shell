"""Compare current correctness probes with the published timing baseline."""
import argparse
import csv
import json
from pathlib import Path
import sys

from report import observation_lost, table


def changes(platform, current, previous):
    for group, features in sorted(current["rows"].items()):
        for name, result in sorted(features.items()):
            before = previous.get((group, name), {}).get("status", "new")
            after = result["status"]
            if before != after:
                yield (platform, group, name, before, after,
                       json.dumps(result["witness"], ensure_ascii=False),
                       json.dumps(result.get("observed", result.get("samples", result.get("reference_result"))), ensure_ascii=False))


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--baseline", type=Path, required=True)
    cli.add_argument("--gnu", type=Path, help="write both observations for every GNU driver")
    cli.add_argument("--inventory", type=Path, help="write all current statuses side by side")
    cli.add_argument("audit", nargs="+", metavar="PLATFORM=JSON")
    args = cli.parse_args()
    writer = csv.writer(sys.stdout, delimiter="\t", lineterminator="\n",
                        quoting=csv.QUOTE_NONE, quotechar=None)
    writer.writerow(("platform", "group", "feature", "previous", "current", "witness", "observed"))
    lost = False
    inventory = {}
    gnu = []
    for audit in args.audit:
        label, path = audit.split("=", 1)
        current = json.loads(Path(path).read_text())
        inventory[label] = {(g, name): r["status"] for g, items in current["rows"].items()
                            for name, r in items.items()}
        for name, row in sorted(current["rows"].get("GNU regression group", {}).items()):
            run = row.get("driver", {})
            for shell in ("bash", "tonari"):
                if shell in run:
                    result = run[shell]
                    gnu.append((label, name, row["status"], shell, result["exit"], result["timeout"],
                                json.dumps(result["diffs"]), json.dumps(result["skip"]),
                                json.dumps(result["example"], ensure_ascii=False)))
        previous = table(args.baseline / (label + ".tsv"))
        for row in changes(label, current, previous):
            writer.writerow(row)
            lost |= observation_lost(row[3], row[4])
        # GNU golden groups are only present when --upstream was supplied.
        for group, name in previous:
            if (group != "GNU regression group" or group in current["rows"]) and (group, name) not in inventory[label]:
                print(f"{label} {group} {name} missing from the new inventory", file=sys.stderr)
                lost = True
    if args.gnu:
        with args.gnu.open("w", newline="") as f:
            rows = csv.writer(f, delimiter="\t", lineterminator="\n", quoting=csv.QUOTE_NONE, quotechar=None)
            rows.writerow(("platform", "driver", "status", "shell", "exit", "timeout", "diffs", "skips", "example"))
            rows.writerows(gnu)
    if args.inventory:
        with args.inventory.open("w", newline="") as f:
            rows = csv.writer(f, delimiter="\t", lineterminator="\n", quoting=csv.QUOTE_NONE, quotechar=None)
            rows.writerow(("group", "feature", *inventory))
            for key in sorted(set().union(*inventory.values())):
                rows.writerow((*key, *(data.get(key, "—") for data in inventory.values())))
    return int(lost)


if __name__ == "__main__":
    sys.exit(main())
