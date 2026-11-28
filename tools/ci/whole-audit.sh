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
if [[ $SUITE == whole-editor || $SUITE == whole-keys || $SUITE == whole-completion ]]; then
  python3 -m venv "$RUNNER_TEMP/editor-env"
  python="$RUNNER_TEMP/editor-env/bin/python"
  "$python" -m pip install pyte==0.8.2
fi
case "$SUITE" in
  whole-startup)
    python3 tests/shell.py "$current" -v -k whole_audit
    python3 tools/ci/whole-startup.py "$baseline" "$current" "$ref" > "$out/startup.jsonl"
    if [[ -f /usr/share/bash-completion/bash_completion ]]; then
      rc="$RUNNER_TEMP/completion.rc"
      printf '%s\n' '. /usr/share/bash-completion/bash_completion' 'declare -F _init_completion >/dev/null || declare -F _comp_initialize >/dev/null || exit 1' > "$rc"
      sha256sum /usr/share/bash-completion/bash_completion > "$out/completion-source.txt"
      python3 tools/ci/completion-startup.py /usr/share/bash-completion/bash_completion "main=$baseline" "candidate=$current" "bash=$ref" > "$out/completion-stages.jsonl"
      python3 tools/bench_login.py "$current" --previous "$baseline" --bash "$ref" --rcfile "$rc" --runs 21 > "$out/completion-rc.txt"
    fi
    if [[ $RUNNER_OS == Linux ]] && command -v strace >/dev/null; then
      # Attribute system-profile work on a real PTY. Keep traced runs out of
      # the timing table because ptrace changes launch and wait costs.
      study=$(mktemp -d)
      export TONARI_TRACE_SHELL="$current" TONARI_TRACE_HOME="$study"
      printf '%s\n' 'PS1="ready~ "' > "$study/.bash_profile"
      for mode in login clean; do
        export TONARI_TRACE_MODE="$mode" TONARI_TRACE_OUT="$out/profile-$mode"
        script -qec 'set -- --login; if [ "$TONARI_TRACE_MODE" = clean ]; then set -- --noprofile --login; fi; exec env -i PATH=/usr/bin:/bin HOME="$TONARI_TRACE_HOME" TERM=xterm strace -f -qq -e trace=execve,wait4,openat -T -o "$TONARI_TRACE_OUT" "$TONARI_TRACE_SHELL" "$@" -ic ":"' /dev/null
      done
    fi
    ;;
  whole-editor)
    "$python" tests/review_whole_editor.py "$current" "$ref"
    "$python" tools/ci/whole-editor.py --shell "main=$baseline" --shell "candidate=$current" --shell "bash=$ref" --require candidate --runs 100 --output "$out/editor.jsonl" > "$out/editor.txt"
    "$python" tools/ci/whole-editor.py --shell "main=$baseline" --shell "candidate=$current" --require candidate --case picker-cancel --case picker-miss --case picker-match --runs 100 --output "$out/picker.jsonl" > "$out/picker.txt"
    if [[ $RUNNER_OS == Linux ]]; then
      cpu=$(python3 -c 'import os; print(min(os.sched_getaffinity(0)))')
      taskset -c "$cpu" "$python" tools/ci/whole-editor.py --shell "candidate=$current" --require candidate --runs 100 --output "$out/editor-cpu1.jsonl" > "$out/editor-cpu1.txt"
    fi
    python3 tools/bench_editor_input.py "$baseline" "$current" "$ref" --entries 100000 --runs 11 --mixed-history --case Ctrl-R --case Miss --case Tab --samples "$out/large-history.jsonl" > "$out/large-history.txt"
    python3 tools/bench_completion.py "$baseline" "$current" --bash "$ref" --command-cache --runs 15 > "$out/command-cache.txt"
    ;;
  whole-completion)
    python3 tests/shell.py "$current" -v -k command_completion_through_path_aliases
    "$python" tools/ci/completion-alias.py "main=$baseline" "candidate=$current" "bash=$ref" --runs 50 > "$out/completion-alias.txt"
    ;;
  whole-keys)
    cases=(--case single-key --case single-emoji --case mid-insert --case mid-delete --case mid-backspace --case mid-left)
    "$python" tools/ci/whole-editor.py --shell "main=$baseline" --shell "candidate=$current" --shell "bash=$ref" --require candidate "${cases[@]}" --runs 100 --output "$out/keys.jsonl" > "$out/keys.txt"
    ;;
  whole-language)
    python3 tools/ci/whole-language.py "$baseline" "$current" "$ref" > "$out/language.tsv"
    regex_locale=C.UTF-8
    if [[ $RUNNER_OS == macOS ]]; then regex_locale=en_US.UTF-8; fi
    cc -O2 tools/ci/regex-engine.c -o "$RUNNER_TEMP/regex-engine"
    LC_ALL="$regex_locale" "$RUNNER_TEMP/regex-engine" > "$out/host-libc-regex.tsv"
    if [[ $RUNNER_OS == Linux ]] && command -v musl-gcc >/dev/null; then
      musl-gcc -O2 -static tools/ci/regex-engine.c -o "$RUNNER_TEMP/musl-regex"
      LC_ALL=C.UTF-8 "$RUNNER_TEMP/musl-regex" > "$out/musl-regex.tsv"
    fi
    if [[ $RUNNER_OS == Linux ]] && command -v strace >/dev/null; then
      python3 - <<'PY' > "$RUNNER_TEMP/quote-pipe.sh"
import runpy
cases = runpy.run_path("tools/ci/whole-language.py")["workloads"]()
print(next(source for label, n, source, _, _ in cases if label == "double quote $v/pipe" and n == 768))
PY
      for program in current ref; do
        cat "$RUNNER_TEMP/quote-pipe.sh" |
          strace -qq -c -e read -o "$out/quote-pipe-reads-$program.txt" "${!program}" --noprofile --norc > "$out/quote-output-$program.txt"
      done
      cmp "$out/quote-output-current.txt" "$out/quote-output-ref.txt"
    fi
    ;;
  whole-process)
    python3 tests/shell.py "$current" -v -k read_uses_tmout
    python3 tests/review_whole_process.py "$current" "$ref"
    python3 tools/ci/whole-process.py "$baseline" "$current" "$ref" > "$out/process.tsv"
    python3 tools/bench_streams.py "$baseline" "$current" "$ref" --case redirect --case pipe --case 'quoted heredoc' --runs 7 --sizes 0 1 4096 262144 4194304 > "$out/streams.txt"
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
