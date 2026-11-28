"""Check the static Linux formatter across its bundled LC_TIME data."""
import argparse
from collections import Counter
import json
from pathlib import Path
import subprocess

FIELDS = "a A b B h p P x X c r EC Ey EY Ex EX Ec Od Om OH OI OU OV Ow Oy OB Ob Oh % ^B #p".split()
EPOCHS = (0, 1556668800, 1700000000)
NAMES = Path(__file__).resolve().parents[2] / "crates/time-locales/names.txt"
SCRIPT = ('for t in ' + ' '.join(map(str, EPOCHS)) + r'''; do
  for f; do printf "%(%${f})T\0" "$t"; done
done''')


def key(name):
    return name.lower().replace(".utf-8", "").replace(".utf8", "")


def sample(shell, name):
    proc = subprocess.run([shell, "-c", SCRIPT, "_", *FIELDS], check=True, capture_output=True,
                          env={"PATH": "/usr/bin:/bin", "TZ": "UTC0", "LC_ALL": name}, timeout=10)
    values = proc.stdout.split(b"\0")
    if proc.stderr or values.pop() != b"" or len(values) != len(EPOCHS) * len(FIELDS):
        raise ValueError((shell, name, proc.stderr, len(values)))
    return [s.decode() for s in values]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tonari", type=lambda p: Path(p).resolve())
    parser.add_argument("bash", type=lambda p: Path(p).resolve())
    parser.add_argument("--require-match", action="store_true",
                        help="fail if a paired locale differs from the reference")
    args = parser.parse_args()
    candidates = subprocess.check_output(["locale", "-a"], text=True, timeout=10).splitlines()
    installed = {}
    for candidate in candidates:
        encoding = candidate.partition("@")[0].partition(".")[2].upper().replace("-", "")
        if not encoding and candidate != "C":
            encoding = subprocess.check_output(
                ["locale", "charmap"], env={"LC_ALL": candidate, "PATH": "/usr/bin:/bin"},
                text=True, timeout=10,
            ).strip().upper().replace("-", "")
        if candidate == "C" or encoding == "UTF8":
            installed[key(candidate)] = candidate
    mismatches = Counter()
    paired = 0
    names = NAMES.read_text().splitlines()
    for name in names:
        base, modifier, suffix = name.partition("@")
        reference = installed.get(key(name) if name != "POSIX" else "c")
        requested = reference or base + ".UTF-8" + modifier + suffix
        tonari = sample(args.tonari, requested)
        for i, value in enumerate(tonari):
            if FIELDS[i % len(FIELDS)] in {"A", "B", "c", "x", "X", "r"} and not value:
                raise ValueError((name, FIELDS[i % len(FIELDS)], EPOCHS[i // len(FIELDS)], "empty date field"))
        if reference is None:
            continue
        paired += 1
        for i, (actual, expected) in enumerate(zip(tonari, sample(args.bash, reference))):
            if actual != expected:
                field = FIELDS[i % len(FIELDS)]
                mismatches[field] += 1
                print(json.dumps(dict(locale=name, field=field, epoch=EPOCHS[i // len(FIELDS)],
                                      bash=expected, tonari=actual), ensure_ascii=False))
    print(json.dumps(dict(locales=len(names), paired=paired, fields=len(FIELDS), epochs=len(EPOCHS),
                          differences=mismatches), ensure_ascii=False))
    if args.require_match and (mismatches or not paired):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
