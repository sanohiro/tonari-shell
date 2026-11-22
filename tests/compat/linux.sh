#!/bin/sh
# Run inside the fixture image. The only stdout payload is the final JSON.
set -eu
# These locales are used by the checked shell workloads and the GNU drivers.
python3 -c 'import locale
for name in ("C.UTF-8", "en_US.UTF-8", "de_DE.UTF-8"):
    locale.setlocale(locale.LC_ALL, name)'
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
python3 tests/compat/upstream.py --source /gnu --helpers /helpers --bash /bin/bash \
    --tonari /tonari --timeout 110 > "$root/upstream.json"
python3 tests/compat/audit.py --source /gnu --bash /bin/bash --tonari /tonari \
    --upstream "$root/upstream.json" > "$root/audit.json"
python3 tests/compat/bench.py --source /gnu --helpers /helpers --bash /bin/bash \
    --tonari /tonari --audit "$root/audit.json" "$@"
