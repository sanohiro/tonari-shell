#!/usr/bin/env bash
# Collect available native counters for a previously verified workload.
set -euo pipefail
out=$1
shift
mkdir -p "$out"
lscpu > "$out/cpu.txt"
cat /proc/sys/kernel/perf_event_paranoid > "$out/perf-event-paranoid.txt"
perf version > "$out/perf-version.txt"
perf list > "$out/perf-list.txt"
events=()
for event in task-clock cycles instructions branches branch-misses cache-misses; do
  probe="$out/probe-$event.txt"
  if LC_ALL=C perf stat --no-big-num -x ';' -e "$event" -- true 2>"$probe" &&
     awk -F';' -v event="$event" '$3 == event && $1 ~ /^[0-9]+([.][0-9]+)?$/ {found=1} END {exit !found}' "$probe"; then
    events+=("$event")
  fi
done
if (( ${#events[@]} == 0 )); then
  echo "No usable perf events on this runner. See the per-event probes." | tee "$out/counters.txt"
  exit 0
fi
selected=$(IFS=,; echo "${events[*]}")
LC_ALL=C perf stat -e "$selected" -r 3 -o "$out/counters.txt" -- "$@"
