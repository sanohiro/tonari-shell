"""Print one pinned upstream review skill or reference as Markdown."""
import argparse
import csv
import hashlib
import os
from pathlib import Path, PurePosixPath
import sys
from urllib.error import URLError
from urllib.parse import quote
from urllib.request import urlopen

CATALOG = Path(__file__).resolve().parents[1] / ".agents/skills/tonari-review/references/sources.tsv"
SYSTEM_REVIEW_SHA256 = "07079efd0dc76f05fade424e5dfb048dce1de2df7626e1a4f56292a4f3f92228"


def main():
    with CATALOG.open() as source:
        skills = {row["name"]: row for row in csv.DictReader(source, delimiter="\t")}
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--list", action="store_true", help="print all pinned sources and licenses")
    parser.add_argument("name", choices=skills, nargs="?")
    parser.add_argument("document", nargs="?", default="SKILL.md")
    args = parser.parse_args()
    if args.list:
        sys.stdout.write(CATALOG.read_text())
        return
    if args.name is None:
        parser.error("choose a skill or --list")
    doc = PurePosixPath(args.document)
    if doc.is_absolute() or ".." in doc.parts or doc.suffix != ".md":
        parser.error("choose a Markdown file within the upstream skill")
    if args.name == "review-agent" and str(doc) == "SKILL.md":
        codex_dir = Path(os.environ.get("CODEX_HOME", Path.home() / ".codex"))
        local = codex_dir / "skills/.system/review-agent/SKILL.md"
        try:
            data = local.read_bytes()
        except OSError:
            pass
        else:
            if hashlib.sha256(data).hexdigest() == SYSTEM_REVIEW_SHA256:
                sys.stdout.buffer.write(data)
                return
    row = skills[args.name]
    url = f"https://raw.githubusercontent.com/{row['repository']}/{row['revision']}/{row['path']}/{quote(str(doc))}"
    try:
        with urlopen(url, timeout=30) as response:
            sys.stdout.buffer.write(response.read())
    except (URLError, TimeoutError) as error:
        parser.exit(1, f"Unable to read {url}: {error}\n")


if __name__ == "__main__":
    main()
