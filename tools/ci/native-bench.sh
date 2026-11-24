#!/usr/bin/env bash
set -euo pipefail
bin="$RUNNER_TEMP/bin"
current="$bin/current"
out="$PWD/results"
ref=/bin/bash
# These Bash archives match the GNU tests used by the distribution audits.
download_bash() {
  local version=$1 hash=$2 path=$3
  curl --retry 3 -fsSL "https://ftpmirror.gnu.org/bash/bash-$version.tar.gz" -o "$path" ||
    curl --retry 3 -fsSL "https://ftp.gnu.org/gnu/bash/bash-$version.tar.gz" -o "$path"
  echo "$hash  $path" | shasum -a 256 -c -
}
version=5.2.37
digest=9599b22ecd1d5787ad7d3b7bf0c59f312b3396d1e281175dd1f8a4014da621ff
if [[ ( $SUITE == inventory* || $SUITE == budget* ) && $RUNNER_OS == Linux ]]; then
  base=debian:trixie-slim
  if [[ $SUITE == *-ubuntu ]]; then
    base=ubuntu:26.04
    version=5.3
    digest=0d5cd86965f869a26cf64f4b71be7b96f90a3ba8b3d74e27e8e9d9d5550f31ba
  fi
  fixture=$(mktemp -d)
  download_bash "$version" "$digest" "$fixture/bash-src.tar.gz"
  docker build -t tonari-audit -f tests/compat/Dockerfile --build-arg BASE="$base" --build-arg GNU_SHA256="$digest" "$fixture"
  run=(docker run --rm --init --network none --tmpfs /tmp:exec,mode=1777,size=1g -v "$PWD:/repo:ro" -v "$current:/tonari:ro")
  "${run[@]}" --entrypoint sh tonari-audit -ec 'uname -m; /bin/bash --version; cat /etc/os-release' >> "$out/host.txt"
  distro=Debian; [[ $SUITE == *-ubuntu ]] && distro=Ubuntu
  arch=x64; [[ $RUNNER_ARCH == ARM64 ]] && arch=arm64
  if [[ $SUITE == inventory-correctness* ]]; then
    "${run[@]}" tonari-audit --correctness-only > "$out/audit.json"
    exit
  fi
  if [[ $SUITE == inventory-trap* ]]; then
    for attempt in 1 2 3; do
      "${run[@]}" --entrypoint python3 tonari-audit tests/compat/upstream.py --source /gnu --helpers /helpers \
        --bash /bin/bash --tonari /tonari --only run-trap > "$out/trap-$attempt.json"
    done
    exit
  fi
  if [[ $SUITE == budget* ]]; then
    extra=(--runs 9); [[ $SUITE == *-parts* ]] && extra+=(--components)
    [[ $SUITE == budget-extra-parts* ]] && extra+=(--runs 31)
    if [[ $SUITE == budget-extra* ]]; then
      run+=(-v "$RUNNER_TEMP/reuse/timings.json:/timings.json:ro")
      extra+=(--inventory /timings.json)
    fi
    if [[ $SUITE == budget-parity* ]]; then
      run+=(-v "$RUNNER_TEMP/reuse:/replay:ro")
      extra=(--release-only --runs 31 --above-bash /replay/budget/replay.jsonl /replay/inventory/additional.jsonl)
    fi
    "${run[@]}" -v "$bin:/candidates:ro" --entrypoint python3 tonari-audit tests/compat/replay_budget.py \
      --lane "$distro$arch" --bash /bin/bash --bin /candidates "${extra[@]}" > "$out/replay.jsonl"
    exit
  fi
  "${run[@]}" --entrypoint python3 tonari-audit tests/shell.py /tonari -q
  "${run[@]}" tonari-audit --correctness-only > "$out/audit.json"
  "${run[@]}" -v "$out/audit.json:/audit:ro" tonari-audit --reuse-audit /audit > "$out/timings.json"
  "${run[@]}" -v "$bin:/candidates:ro" -v "$out/timings.json:/timings.json:ro" --entrypoint python3 tonari-audit tests/compat/replay_budget.py \
    --lane "$distro$arch" --bash /bin/bash --bin /candidates --inventory /timings.json > "$out/additional.jsonl"
  exit
fi
if [[ $RUNNER_OS == macOS ]]; then
  if [[ $SUITE == inventory || $SUITE == parity-commands || $SUITE == parity-invocation || $SUITE == foreground || $SUITE == navigation || $SUITE == edit-actions || $SUITE == starship || $SUITE == rcfiles || $SUITE == budget* ]]; then
    gnu="$RUNNER_TEMP/gnu"
    mkdir -p "$gnu"
    download_bash "$version" "$digest" "$RUNNER_TEMP/bash.tar.gz"
    tar -xzf "$RUNNER_TEMP/bash.tar.gz" --strip-components=1 -C "$gnu"
    (cd "$gnu"; ./configure --without-bash-malloc > configure.log && make -j3 > build.log)
    ref="$gnu/bash"
  else
    if [[ ! -x /opt/homebrew/bin/bash ]]; then brew install bash; fi
    ref=/opt/homebrew/bin/bash
  fi
  if [[ $SUITE == budget* ]]; then
    extra=(--runs 9); [[ $SUITE == *-parts* ]] && extra+=(--components)
    [[ $SUITE == budget-extra-parts* ]] && extra+=(--runs 31)
    if [[ $SUITE == budget-extra* ]]; then extra+=(--inventory "$RUNNER_TEMP/reuse/timings.json"); fi
    if [[ $SUITE == budget-parity* ]]; then
      extra=(--release-only --runs 31 --above-bash "$RUNNER_TEMP/reuse/budget/replay.jsonl" "$RUNNER_TEMP/reuse/inventory/additional.jsonl")
    fi
    python3 tests/compat/replay_budget.py --lane macOSarm64 --bash "$ref" --bin "$bin" "${extra[@]}" > "$out/replay.jsonl"
    exit
  fi
  if [[ $SUITE == inventory ]]; then
    REFERENCE_BASH="$ref" python3 tests/shell.py "$current" -q
    python3 tests/compat/audit.py --source "$gnu" --bash "$ref" --tonari "$current" > "$out/audit.json"
    python3 tests/compat/bench.py --source "$gnu" --bash "$ref" --tonari "$current" --audit "$out/audit.json" > "$out/timings.json"
    python3 tests/compat/replay_budget.py --lane macOSarm64 --bash "$ref" --bin "$bin" --inventory "$out/timings.json" > "$out/additional.jsonl"
    exit
  fi
fi
"$ref" --version >> "$out/host.txt"
run() {
  local label=$1
  shift
  echo "::group::$label"
  "$@" | tee "$out/$label.txt"
  echo "::endgroup::"
}
history_case() {
  local label=$1
  shift
  run "$label" python3 tools/bench_editor_input.py --require-bash-parity --samples "$out/$label.samples.jsonl" "$@"
}
# Compare the runner's real login files as well as our isolated profile fixtures.
if [[ $RUNNER_OS == Linux && ( $SUITE == functions || $SUITE == throughput ) ]]; then
  check_home="$RUNNER_TEMP/check-home"
  mkdir -p "$check_home"
  cp /etc/skel/.bashrc /etc/skel/.profile "$check_home/"
  for build in baseline current; do
    for profile in /etc/profile /etc/profile.d/*.sh; do
      if ! "$bin/$build" --noprofile --norc -n "$profile" >/dev/null 2>&1; then
        echo "$profile"
      fi
    done > "$out/$build.syntax"
    (cd "$check_home"
     env -i HOME="$PWD" TERM=dumb LC_ALL=C PATH=/usr/bin:/bin "$bin/$build" --login -ic \
       'echo "$SHLVL/$HISTCONTROL/$HISTSIZE"; type -t ll; :' </dev/null
    ) > "$out/$build.stdout" 2> "$out/$build.stderr"
  done
  for stream in stdout stderr syntax; do
    diff -u "$out/baseline.$stream" "$out/current.$stream"
  done
fi
case "$SUITE" in
  starship)
    case "$NATIVE_RUNNER" in
      ubuntu-24.04) platform=x86_64-unknown-linux-musl; sha=10c8eb4cb3b761d9bf59ea828444c2dab36c900b7d69a69e08814b286969df89 ;;
      ubuntu-24.04-arm) platform=aarch64-unknown-linux-musl; sha=341f5cbf24699fa0b27ec88965ad25f862ea09e15d35a543fd6ffdff1d2aa00d ;;
      macos-15) platform=aarch64-apple-darwin; sha=5362da8cb2928f98f3ce0ffda697cf0f8de70081db000dae316d6d3a404f6cf0 ;;
    esac
    archive="$RUNNER_TEMP/starship.tar.gz"
    curl --retry 3 -fsSL "https://github.com/starship/starship/releases/download/v1.27.0/starship-$platform.tar.gz" -o "$archive"
    echo "$sha  $archive" | shasum -a 256 -c -
    tar -xzf "$archive" -C "$RUNNER_TEMP" starship
    starship_bench=(python3 tools/bench_starship.py "$bin/baseline" "$current" "$ref" --starship "$RUNNER_TEMP/starship")
    run starship "${starship_bench[@]}" --runs 25
    run starship-long "${starship_bench[@]}" --long-prompts-only --runs 15 ;;
  edit-actions)
    REFERENCE_BASH="$ref" python3 tests/shell.py "$current" -q -k last_arg -k test_undo
    run actions python3 tools/bench_edit_actions.py "$bin/baseline" "$current" "$ref" --runs 21
    if [[ $RUNNER_OS == Linux ]]; then
      cpu=$(python3 -c 'import os;print(min(os.sched_getaffinity(0)))')
      run actions-one-cpu taskset -c "$cpu" python3 tools/bench_edit_actions.py "$bin/baseline" "$current" "$ref" --runs 15
    fi ;;
  navigation)
    REFERENCE_BASH="$ref" python3 tests/shell.py "$current" -q -k navigation
    run navigation python3 tools/bench_navigation.py "$bin/baseline" "$current" --bash "$ref" --runs 21
    if [[ $RUNNER_OS == Linux ]]; then
      cpu=$(python3 -c 'import os;print(min(os.sched_getaffinity(0)))')
      run navigation-one-cpu taskset -c "$cpu" python3 tools/bench_navigation.py "$bin/baseline" "$current" --bash "$ref" --runs 15
    fi ;;
  history-fields)
    REFERENCE_BASH="$ref" python3 tests/shell.py "$current" -q -k history_word -k history_fragment -k reverse_search -k escaped_journal -k search_paste
    run fields python3 tools/bench_history_fields.py "$bin/baseline" "$current" --runs 21
    if [[ $RUNNER_OS == Linux ]]; then
      cpu=$(python3 -c 'import os;print(min(os.sched_getaffinity(0)))')
      run fields-one-cpu taskset -c "$cpu" python3 tools/bench_history_fields.py "$bin/baseline" "$current" --runs 15
    fi
    run literal python3 tools/bench_editor_input.py "$current" "$ref" --case Ctrl-R --runs 21 ;;
  picker)
    REFERENCE_BASH="$ref" python3 tests/shell.py "$current" -q -k picker -k completion -k abbreviated_paths
    run picker python3 tools/bench_picker.py "$bin/baseline" "$current" --runs 31
    run listing python3 tools/bench_completion.py "$current" --bash "$ref" --listing --runs 21 ;;
  parity-mixed)
    shells=("$current" "$bin/baseline" "$ref")
    history_case oldest "${shells[@]}" --mixed-history --case Ctrl-R --prompt-time --runs 41
    history_case oldest-ascii "${shells[@]}" --mixed-history --case Ctrl-R --ascii-query --prompt-time --runs 41
    history_case missing "$current" "$ref" --mixed-history --case Miss --prompt-time --runs 21 ;;
  parity-history)
    shells=("$current" "$bin/baseline" "$ref")
    history_case oldest "${shells[@]}" --case Ctrl-R --prompt-time --runs 61
    history_case missing "$current" "$ref" --case Miss --prompt-time --runs 41
    history_case oldest-ascii "${shells[@]}" --case Ctrl-R --ascii-query --prompt-time --runs 41
    history_case million "${shells[@]}" --case Ctrl-R --entries 1000000 --prompt-time --runs 21
    history_case missing-ascii "$current" "$ref" --case Miss --ascii-query --prompt-time --runs 21
    for rank in 20 900; do
      history_case "rank-$rank" "${shells[@]}" --case Ctrl-R --history-rank "$rank" --prompt-time --runs 41
    done
    run bursts python3 tools/bench_history.py "$current" "$bin/baseline" --entries 1000000 --bursts --runs 21 ;;
  parity-jobs)
    run jobs python3 tests/compat/job_latency.py "$current" "$bin/baseline" "$ref" --runs 41 ;;
  parity-commands)
    [[ $RUNNER_OS == macOS ]]
    python3 tests/compat/replay_budget.py --release-only --also-single --bin "$bin" --bash "$ref" --lane macOSarm64 --runs 31 \
      --only 1073fad1c5f9 27e15ee9fb24 2d8b87809a57 486d0a5c94f0 48f0f68f324f 4dbd86b7239c \
             ac121d4b6180 b231f00edd90 d2003b0a862e e4c5cb7b4108 e75fc215bb04 > "$out/replay.jsonl" ;;
  parity-invocation)
    [[ $RUNNER_OS == macOS ]]
    python3 tests/compat/replay_budget.py --self-invocation --release-only --bin "$bin" --bash "$ref" --lane macOSarm64 \
      --runs 61 --only b231f00edd90 > "$out/replay.jsonl" ;;
  probe)
    cc -O2 tools/bench_spawn.c -o "$bin/spawn-control"
    run probes python3 tools/bench_scripts.py "$current" --bash "$ref" --previous "$bin/baseline" --timing started --runs 41 \
      --spawn-control "$bin/spawn-control" --case redirected-subshells --case descriptor-moves --case directory-stack \
      --case builtin-output --case builtin-output-long --case external --case external-env --case pipeline --case external-substitutions \
      --case merged-pipeline --case merged-pipeline-redirect --case redirect-stderr --case background-subshell-wait ;;
  foreground)
    run foreground python3 tests/compat/foreground.py "$current" "$ref" ;;
  history-windows)
    shells=("$bin/window-4" "$bin/window-16" "$bin/window-32" "$ref")
    run oldest python3 tools/bench_editor_input.py "${shells[@]}" --case Ctrl-R --runs 61
    for rank in 20 300 900; do
      run "rank-$rank" python3 tools/bench_editor_input.py "${shells[@]}" --case Ctrl-R --runs 41 --history-rank "$rank"
    done
    run history-bursts python3 tools/bench_history.py "$bin/window-4" "$bin/window-16" "$bin/window-32" --bursts --entries 100000 1000000 --runs 21 ;;
  history)
    cc -O2 tools/bench_spawn.c -o "$bin/scan-control"
    run tab python3 tools/bench_editor_input.py "$current" "$bin/baseline" "$ref" --case Tab --scan-control "$bin/scan-control" --runs 41
    run reverse-search python3 tools/bench_editor_input.py "$current" "$bin/baseline" "$ref" --case Ctrl-R --runs 41
    run bursts python3 tools/bench_history.py "$current" "$bin/baseline" --bursts --entries 100000 1000000 --runs 21
    run escaped python3 tools/bench_history.py "$current" "$bin/baseline" --escaped --entries 1000000 --runs 21 ;;
  throughput)
    REFERENCE_BASH="$ref" python3 tests/shell.py "$current" -q -k foreground
    REFERENCE_BASH="$ref" python3 tests/shell.py "$current" -q -k large_glob
    run foreground-external python3 tools/bench_foreground.py "$current" "$ref" --previous "$bin/baseline" --runs 31
    cc -O2 tools/bench_spawn.c -o "$bin/spawn-control"
    run scripts python3 tools/bench_scripts.py "$current" --bash "$ref" --previous "$bin/baseline" \
      --spawn-control "$bin/spawn-control" --timing started --runs 31
    run startup python3 tools/bench.py "$current" --bash "$ref" --compare-tonari Previous "$bin/baseline" \
      --case prompt --case startup --case loop --runs 71 ;;
  functions)
    REFERENCE_BASH="$ref" python3 tests/shell.py "$current" -q -k function
    run functions python3 tools/bench_scripts.py "$current" --bash "$ref" --previous "$bin/baseline" \
      --case function-redefinitions --case function-definitions --case function-wrapper-redefinitions \
      --case function-compound-redefinitions --case function-loop-redefinitions --case functions \
      --case function-redefine-and-call --case function-literal-calls --runs 61
    run startup python3 tools/bench.py "$current" --bash "$ref" --compare-tonari Previous "$bin/baseline" \
      --case prompt --case startup --case loop --runs 101 ;;
  editing)
    for size in 96 65536; do
      run "editing-$size" python3 tools/bench_editing.py "$current" "$ref" --size "$size" --runs 11
    done
    run steady python3 tools/bench_editing.py "$current" "$ref" --size 65536 --steady 512 --runs 3
    run input python3 tools/bench_editor_input.py "$current" "$ref" --large --runs 13
    run history python3 tools/bench_history.py "$current" --bursts --entries 0 1000 100000 1000000 --runs 21
    run escaped python3 tools/bench_history.py "$current" --escaped --entries 1000000 --runs 21
    if [[ $RUNNER_OS == Linux ]]; then
      cpu=$(python3 -c 'import os;print(min(os.sched_getaffinity(0)))')
      run one-cpu-editing taskset -c "$cpu" python3 tools/bench_editing.py "$current" "$ref" --size 65536 --runs 11
      run one-cpu-steady taskset -c "$cpu" python3 tools/bench_editing.py "$current" "$ref" --size 65536 --steady 512 --runs 3
      run one-cpu-input taskset -c "$cpu" python3 tools/bench_editor_input.py "$current" "$ref" --large --runs 13
    fi ;;
  completion)
    for n in 10000 100000; do
      run "files-$n" python3 tools/bench_completion.py "$current" --bash "$ref" --files "$n" --runs 19
    done
    for mode in listing callback scan-key long-line word-list reuse-words; do
      run "$mode" python3 tools/bench_completion.py "$current" --bash "$ref" "--$mode" --runs 21
    done
    run project python3 tools/bench_completion.py "$current" --project --runs 21
    run picker python3 tools/bench_picker.py "$current" --runs 21
    shells=("$current")
    if [[ -f $bin/serial ]]; then shells+=("$bin/serial"); fi
    run branches python3 tools/bench_completion.py "${shells[@]}" --branches --runs 41
    if [[ $RUNNER_OS == Linux ]]; then
      cpu=$(python3 -c 'import os;print(min(os.sched_getaffinity(0)))')
      run branches-one taskset -c "$cpu" python3 tools/bench_completion.py "${shells[@]}" --branches --runs 41
      run scan-key-one taskset -c "$cpu" python3 tools/bench_completion.py "$current" --bash "$ref" --scan-key --runs 21
    fi ;;
  execution)
    curl --retry 3 -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.5/nvm.sh -o "$RUNNER_TEMP/nvm.sh"
    run startup python3 tools/bench.py "$current" --bash "$ref" --nvm-script "$RUNNER_TEMP/nvm.sh" --runs 101
    run direct python3 tools/bench.py "$current" --bash "$ref" --case direct-series --runs 51
    run login python3 tools/bench_login.py "$current" --bash "$ref" --prompt --runs 51
    cc -O2 tools/bench_spawn.c -o "$bin/spawn-control"
    run scripts python3 tools/bench_scripts.py "$current" --bash "$ref" --spawn-control "$bin/spawn-control" --runs 21
    run parent-state python3 tools/bench_scripts.py "$current" --bash "$ref" --state-mib 32 --case pipeline --case external --case external-substitutions --runs 21
    run streams python3 tools/bench_streams.py "$current" "$ref" --runs 17
    run external-io python3 tools/bench_streams.py "$current" "$ref" --case redirect --case pipe --sizes 64 16777216 268435456 --runs 21
    run capture python3 tools/bench_file_capture.py "$current" "$ref" --runs 21
    if [[ $RUNNER_OS == Linux ]]; then run heap python3 tools/bench_heap.py "$current" "$ref" --runs 7; fi ;;
  rcfiles)
    REFERENCE_BASH="$ref" python3 tests/shell.py "$current" -q
    rc="$RUNNER_TEMP/rc"
    mkdir -p "$rc"
    : > "$rc/empty"
    # Same 94,826-byte Git completion source used in the original macOS measurement.
    curl --retry 3 -fsSL \
      https://raw.githubusercontent.com/git/git/v2.54.0/contrib/completion/git-completion.bash -o "$rc/git.bash"
    echo "56264a168d862c2a51305fe2a3450edf04590d8bafa42c2d061e6f992985fc0f  $rc/git.bash" | shasum -a 256 -c -
    printf '. "%s"\ndeclare -F __git_complete >/dev/null || exit 1\n' "$rc/git.bash" > "$rc/git"
    printf '%s\n' 'for ((i=0; i<4000; i++)); do : ready; done' '[[ $i == 4000 ]] || exit 1' > "$rc/builtins"
    cat > "$rc/light" <<'RC'
alias ll='ls -l'
export PATH="$HOME/.local/bin:$PATH"
nvm() { unset -f nvm; . "$HOME/.nvm/nvm.sh" && nvm "$@"; }
alias ll >/dev/null && declare -F nvm >/dev/null || exit 1
RC
    python3 - "$rc" <<'PYRC'
import sys
from pathlib import Path
rc = Path(sys.argv[1])
(rc / "large").write_text("large_fn() {\n return 0\n" +
    "".join(f"  : line-{i:04d}-padding-abcdef\n" for i in range(760)) +
    "}\nlarge_fn || exit 1\n")
body = "if true; then\n" + "  if true; then\n    : nested\n  fi\n" * 300
for name, literal in (("nested", ""), ("quoted", "  : <<'END'\nif true; then\nEND\n")):
    (rc / name).write_text(body + literal + "  verified=loaded\nfi\n")
PYRC
    printf 'eval "$(<"%s")" || exit 1\ndeclare -F large_fn >/dev/null || exit 1\n' "$rc/large" > "$rc/eval-large"
    for name in nested quoted; do
      printf 'eval "$(<"%s")" || exit 1\n[[ $verified == loaded ]] || exit 1\n' "$rc/$name" > "$rc/eval-$name"
    done
    for name in empty light git large eval-large eval-nested eval-quoted builtins; do
      run "rc-$name" python3 tools/bench_login.py "$current" --previous "$bin/baseline" --bash "$ref" --rcfile "$rc/$name" --runs 41
    done
    run eval python3 tools/bench_scripts.py "$current" --previous "$bin/baseline" --bash "$ref" --timing started --runs 25 \
      --case eval-fixed --case eval-changing --case eval-arguments --case eval-alternating \
      --case eval-nested --case eval-multiline --case eval-function --case eval-alias ;;
  profiles)
    run levels python3 tools/bench.py "$current" --bash "$ref" --compare-tonari Old "$bin/previous-profile" --compare-tonari Os "$bin/opt-s" --compare-tonari Oz "$bin/opt-z" --compare-tonari O3 "$bin/opt-3" --runs 101
    for level in s z 3 previous-profile; do
      other="$bin/opt-$level"; [[ $level == previous-profile ]] && other="$bin/previous-profile"
      run "scripts-$level" python3 tools/bench_scripts.py "$current" --previous "$other" --bash "$ref" --timing started --runs 15
    done
    run history python3 tools/bench_history.py "$current" "$bin/previous-profile" "$bin/opt-s" "$bin/opt-z" "$bin/opt-3" --entries 1000000 --bursts --runs 21
    run completion python3 tools/bench_completion.py "$current" "$bin/previous-profile" "$bin/opt-s" "$bin/opt-z" "$bin/opt-3" --files 100000 --runs 19
    run input python3 tools/bench_editor_input.py "$current" "$bin/previous-profile" "$bin/opt-s" "$bin/opt-z" "$bin/opt-3" --case commit --runs 21 ;;
  *) echo "Unknown suite: $SUITE" >&2; exit 2 ;;
esac
