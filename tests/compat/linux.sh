#!/bin/sh
# Run inside the fixture image. The only stdout payload is the final JSON.
set -eu
# These locales are used by the checked shell workloads and the GNU drivers.
python3 -c 'import locale
for name in ("C.UTF-8", "en_US.UTF-8", "de_DE.UTF-8"):
    locale.setlocale(locale.LC_ALL, name)'
if [ "${1-}" = --reuse-audit ]; then
    audit=${2:?supply the checked audit JSON}
    shift 2
    python3 -c 'import json, sys
if not json.load(open(sys.argv[1])).get("executable_sha256"):
    sys.exit("audit has no executable fingerprints; regenerate it with --correctness-only")' "$audit"
    exec python3 tests/compat/bench.py --source /gnu --helpers /helpers --bash /bin/bash \
        --tonari /tonari --audit "$audit" "$@"
fi
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
python3 tests/compat/upstream.py --source /gnu --helpers /helpers --bash /bin/bash \
    --tonari /tonari --timeout 110 > "$root/upstream.json"
python3 tests/compat/audit.py --source /gnu --bash /bin/bash --tonari /tonari \
    --upstream "$root/upstream.json" > "$root/audit.json"
if [ "${1-}" = --correctness-only ]; then
    cat "$root/audit.json"
    exit 0
fi
python3 tests/compat/bench.py --source /gnu --helpers /helpers --bash /bin/bash \
    --tonari /tonari --audit "$root/audit.json" "$@"
