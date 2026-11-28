#!/usr/bin/env bash
set -euo pipefail
current=$1
baseline=$2
out=$3
ref=/bin/bash
if [[ $RUNNER_OS == macOS ]]; then
  ref=$(brew --prefix bash)/bin/bash
  [[ -x $ref ]] || brew install bash
  otool -L "$current" > "$out/link.txt"
  nm -u "$current" >> "$out/link.txt"
else
  readelf -ldW "$current" > "$out/link.txt"
fi
"$ref" --version > "$out/bash-version.txt"
wc -c "$baseline" "$current" > "$out/binary-sizes.txt"
file "$current" "$ref" > "$out/format.txt"
export REFERENCE_BASH="$ref" PYTHONPATH="$PWD/tools"
case "$SUITE" in
  whole-startup)
    python3 tests/shell.py "$current" -v -k whole_audit
    python3 tools/ci/whole-startup.py "$baseline" "$current" "$ref" > "$out/startup.jsonl"
    ;;
  whole-editor)
    python3 -m venv "$RUNNER_TEMP/editor-env"
    python="$RUNNER_TEMP/editor-env/bin/python"
    "$python" -m pip install pyte==0.8.2
    "$python" tests/review_whole_editor.py "$current" "$ref"
    "$python" tools/ci/whole-editor.py --shell "main=$baseline" --shell "candidate=$current" --shell "bash=$ref" --runs 100 --output "$out/editor.jsonl" > "$out/editor.txt"
    if [[ $RUNNER_OS == Linux ]]; then
      cpu=$(python3 -c 'import os; print(min(os.sched_getaffinity(0)))')
      taskset -c "$cpu" "$python" tools/ci/whole-editor.py --shell "candidate=$current" --runs 100 --output "$out/editor-cpu1.jsonl" > "$out/editor-cpu1.txt"
    fi
    python3 tools/bench_completion.py "$baseline" "$current" --bash "$ref" --command-cache --runs 15 > "$out/command-cache.txt"
    ;;
  whole-language)
    python3 tools/ci/whole-language.py "$baseline" "$current" "$ref" > "$out/language.tsv"
    ;;
  whole-process)
    python3 tests/review_whole_process.py "$current" "$ref"
    python3 tools/ci/whole-process.py "$baseline" "$current" "$ref" > "$out/process.tsv"
    ;;
  whole-profile)
    program='LC_ALL=C.UTF-8; for ((i=0;i<24000;i++)); do v+=あ; n=${#v}; done; test "$n" = "$i"'
    if [[ $RUNNER_OS == Linux ]]; then
      # Probe the events provided by this CPU and guest. Preserve unavailable
      # counters in the artifact rather than substituting zero.
      policy=$(cat /proc/sys/kernel/perf_event_paranoid)
      if (( policy > 2 )) && sudo -n sysctl -w kernel.perf_event_paranoid=2; then
        trap 'sudo -n sysctl -w "kernel.perf_event_paranoid=$policy"' EXIT
      fi
      for version in baseline current; do
        path=$current
        [[ $version != baseline ]] || path=$baseline
        "$path" -c "$program"
        bash tools/ci/review-perf.sh "$out/$version" "$path" -c "$program"
      done
    fi
    ;;
  *) exit 2 ;;
esac
