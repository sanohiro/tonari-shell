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
if [[ $SUITE == inventory* && $RUNNER_OS == Linux ]]; then
  base=debian:trixie-slim
  if [[ $SUITE == inventory-ubuntu ]]; then
    base=ubuntu:26.04
    version=5.3
    digest=0d5cd86965f869a26cf64f4b71be7b96f90a3ba8b3d74e27e8e9d9d5550f31ba
  fi
  fixture=$(mktemp -d)
  download_bash "$version" "$digest" "$fixture/bash-src.tar.gz"
  docker build -t tonari-audit -f tests/compat/Dockerfile --build-arg BASE="$base" --build-arg GNU_SHA256="$digest" "$fixture"
  run=(docker run --rm --init --network none --tmpfs /tmp:exec,mode=1777,size=1g -v "$PWD:/repo:ro" -v "$current:/tonari:ro")
  "${run[@]}" --entrypoint sh tonari-audit -ec 'uname -m; /bin/bash --version; cat /etc/os-release' >> "$out/host.txt"
  "${run[@]}" --entrypoint python3 tonari-audit tests/shell.py /tonari -q
  "${run[@]}" tonari-audit --correctness-only > "$out/audit.json"
  "${run[@]}" -v "$out/audit.json:/audit:ro" tonari-audit --reuse-audit /audit > "$out/timings.json"
  exit
fi
if [[ $RUNNER_OS == macOS ]]; then
  if [[ $SUITE == inventory ]]; then
    gnu="$RUNNER_TEMP/gnu"
    mkdir -p "$gnu"
    download_bash "$version" "$digest" "$RUNNER_TEMP/bash.tar.gz"
    tar -xzf "$RUNNER_TEMP/bash.tar.gz" --strip-components=1 -C "$gnu"
    (cd "$gnu"; ./configure --without-bash-malloc > configure.log && make -j3 > build.log)
    ref="$gnu/bash"
    REFERENCE_BASH="$ref" python3 tests/shell.py "$current" -q
    python3 tests/compat/audit.py --source "$gnu" --bash "$ref" --tonari "$current" > "$out/audit.json"
    python3 tests/compat/bench.py --source "$gnu" --bash "$ref" --tonari "$current" --audit "$out/audit.json" > "$out/timings.json"
    exit
  fi
  if [[ ! -x /opt/homebrew/bin/bash ]]; then brew install bash; fi
  ref=/opt/homebrew/bin/bash
fi
"$ref" --version >> "$out/host.txt"
run() {
  local label=$1
  shift
  echo "::group::$label"
  "$@" | tee "$out/$label.txt"
  echo "::endgroup::"
}
case "$SUITE" in
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
    rc="$RUNNER_TEMP/rc"
    mkdir -p "$rc"
    # Same 94,826-byte Git completion source used in the original macOS measurement.
    curl --retry 3 -fsSL -H 'Accept: application/vnd.github.raw+json' \
      https://api.github.com/repos/git/git/git/blobs/a8e7c6ddbfb2b13659587ba7ab4adb7cf4c63127 -o "$rc/git.bash"
    echo "56264a168d862c2a51305fe2a3450edf04590d8bafa42c2d061e6f992985fc0f  $rc/git.bash" | shasum -a 256 -c -
    printf '. "%s"\ndeclare -F __git_complete >/dev/null || exit 1\n' "$rc/git.bash" > "$rc/git"
    cat > "$rc/light" <<'RC'
alias ll='ls -l'
export PATH="$HOME/.local/bin:$PATH"
nvm() { unset -f nvm; . "$HOME/.nvm/nvm.sh" && nvm "$@"; }
alias ll >/dev/null && declare -F nvm >/dev/null || exit 1
RC
    python3 - "$rc/large" <<'PYRC'
import sys
from pathlib import Path
Path(sys.argv[1]).write_text("large_fn() {\n return 0\n" +
    "".join(f"  : line-{i:04d}-padding-abcdef\n" for i in range(760)) +
    "}\nlarge_fn || exit 1\n")
PYRC
    for name in light git large; do
      run "rc-$name" python3 tools/bench_login.py "$current" --bash "$ref" --rcfile "$rc/$name" --runs 41
    done ;;
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
