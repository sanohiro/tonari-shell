#!/usr/bin/env bash
set -euo pipefail
current=$1
baseline=$2
reference=$3
out=$4
args=()
while read -r name; do args+=(--case "$name"); done < <(
  PYTHONPATH=tools python3 -c 'from bench_scripts import CASES; print("\n".join(k for k in CASES if k.startswith("r8-")))'
)
python3 tools/bench_scripts.py "$current" --previous "$baseline" --bash "$reference" --timing started \
  --runs "${BENCH_RUNS:-15}" "${args[@]}" --case while-arithmetic > "$out/runtime.txt"
python3 tools/bench_scripts.py "$current" --previous "$baseline" --bash "$reference" --timing sourced \
  --runs "${BENCH_RUNS:-15}" --case source-functions-mixed > "$out/source-return.txt"
python3 tools/bench_editor_input.py "$baseline" "$current" "$reference" --entries 5000 \
  --runs "${BENCH_RUNS:-15}" --case paste --case commit > "$out/input.txt"
