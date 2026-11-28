#!/usr/bin/env bash
set -euo pipefail
current=$1
baseline=$2
reference=$3
out=$4
runs=${BENCH_RUNS:-15}
cases=(null-builtin-out runtime-glob-nocase runtime-glob-prefix review-compgen source-functions-mixed core-parse-functions)
while read -r name; do cases+=("$name"); done < <(
  PYTHONPATH=tools python3 -c 'from bench_scripts import CASES; print("\n".join(k for k in CASES if k.startswith("r7-")))'
)
control=$(mktemp)
trap 'rm -f "$control"' EXIT
cc -O2 tools/bench_spawn.c -o "$control"
args=(--spawn-control "$control")
for name in "${cases[@]}"; do args+=(--case "$name"); done
python3 tools/bench_scripts.py "$current" --previous "$baseline" --bash "$reference" \
  --timing started --runs "$runs" "${args[@]}" > "$out/runtime.txt"
python3 tools/bench_history_fields.py "$baseline" "$current" --entries 100000 \
  --deep-page --runs "$runs" > "$out/history-page.txt"
python3 tools/bench_history_fields.py "$baseline" "$current" --entries 100000 --deep-page --mixed-records --runs "$runs" > "$out/history-mixed.txt"
python3 tools/bench_history_fields.py "$baseline" "$current" --entries 100000 --deep-page --mixed-records --alternating --runs "$runs" > "$out/history-alternating.txt"
python3 tools/bench_history_fields.py "$baseline" "$current" --entries 100000 \
  --runs "$runs" > "$out/history-input.txt"

python3 tools/bench_scripts.py "$current" --previous "$baseline" --bash "$reference" --timing sourced --runs "$runs" --case source-functions-mixed --case core-parse-functions --case r7-source-case > "$out/source-return.txt"
