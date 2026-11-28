#!/usr/bin/env bash
set -euo pipefail
current=$1
baseline=$2
reference=$3
out=$4
runs=${BENCH_RUNS:-15}
cases=(current-shell-file-capture glob-ignore-simple-class review-length-ascii-wide
       review-length-mixed-wide source-functions-mixed core-parse-functions core-arithmetic)
while read -r name; do cases+=("$name"); done < <(
  PYTHONPATH=tools python3 -c 'from bench_scripts import CASES; print("\n".join(k for k in CASES if k.startswith("runtime-")))'
)
args=()
for name in "${cases[@]}"; do args+=(--case "$name"); done
python3 tools/bench_scripts.py "$current" --previous "$baseline" --bash "$reference" \
  --timing started --runs "$runs" "${args[@]}" > "$out/runtime.txt"
python3 tools/bench_history_fields.py "$baseline" "$current" --entries 100000 \
  --deep-page --runs "$runs" > "$out/history-page.txt"
python3 tools/bench_history_fields.py "$baseline" "$current" --entries 100000 \
  --runs "$runs" > "$out/history-input.txt"
