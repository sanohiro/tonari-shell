#!/usr/bin/env bash
set -euo pipefail
mkdir -p results "$RUNNER_TEMP/bin"
case "$NATIVE_RUNNER/$(uname -s)/$(uname -m)/$RUNNER_ARCH" in
  ubuntu-24.04/Linux/x86_64/X64) target=x86_64-unknown-linux-musl ;;
  ubuntu-24.04-arm/Linux/aarch64/ARM64) target=aarch64-unknown-linux-musl ;;
  macos-15/Darwin/arm64/ARM64) target=aarch64-apple-darwin ;;
  *) echo "Unexpected native runner $NATIVE_RUNNER / $RUNNER_ARCH" >&2; exit 1 ;;
esac
{
  git rev-parse HEAD
  uname -a
  if [[ $RUNNER_OS == Linux ]]; then
    lscpu
    taskset -pc $$
    cat /sys/fs/cgroup/cpu.max || true
    findmnt -T "$RUNNER_TEMP"
  else
    sysctl machdep.cpu.brand_string hw.ncpu
    sw_vers
  fi
} > results/host.txt
toolchain=$(sed -n 's/^ *toolchain=//p' tools/build-release.sh)
rustup toolchain install "$toolchain" --profile minimal --component rust-src --target "$target"
rustc +"$toolchain" -vV >> results/host.txt
export CARGO_TARGET_DIR="$RUNNER_TEMP/build"
cp "$(tools/build-release.sh)" "$RUNNER_TEMP/bin/current"
if [[ $SUITE == external-yash-[0-7] ]]; then
  observer="$RUNNER_TEMP/yash-paths"
  git worktree add -q --detach "$observer" HEAD
  python3 tools/ci/yash-paths.py "$observer" results/sites.json
  (cd "$observer"; cp "$(tools/build-release.sh)" "$RUNNER_TEMP/bin/paths")
fi
if [[ $SUITE == external-shared-input ]]; then
  source_dir="$RUNNER_TEMP/seekable"
  git worktree add -q --detach "$source_dir" HEAD
  for config in small:512 large:32768; do
    size=${config#*:}
    sed "s/BufReader::new(file)/BufReader::with_capacity($size, file)/" src/input.rs > "$source_dir/src/input.rs"
    (cd "$source_dir"; cp "$(tools/build-release.sh)" "$RUNNER_TEMP/bin/${config%:*}")
  done
fi
if [[ $SUITE == external-gnu-waits ]]; then
  observer="$RUNNER_TEMP/wait-source"
  git worktree add -q --detach "$observer" HEAD
  (cd "$observer"; git apply tools/ci/configure-waits.patch; cp "$(tools/build-release.sh)" "$RUNNER_TEMP/bin/observer")
fi
if [[ $SUITE == external-yash-* || $SUITE == external-review-* || $SUITE == external-gnu-profile || $SUITE == parity* || $SUITE == probe || $SUITE == functions || $SUITE == throughput || $SUITE == history || $SUITE == history-fields || $SUITE == navigation || $SUITE == edit-actions || $SUITE == starship || $SUITE == rcfiles || $SUITE == tool-hooks || $SUITE == picker || $SUITE == budget* || $SUITE == inventory* ]]; then
  previous=$(git rev-parse --verify --end-of-options "${PREVIOUS_REVISION:-HEAD^}^{commit}")
  git worktree add -q --detach "$RUNNER_TEMP/base" "$previous"
  (cd "$RUNNER_TEMP/base"; cp "$(tools/build-release.sh)" "$RUNNER_TEMP/bin/baseline")
fi
if [[ $SUITE == external-review-* ]]; then
  cp "$(CARGO_PROFILE_RELEASE_OPT_LEVEL=3 tools/build-release.sh)" "$RUNNER_TEMP/bin/opt-3"
fi
if [[ $SUITE == budget* || $SUITE == inventory* ]]; then
  for level in s 3; do
    cp "$(CARGO_PROFILE_RELEASE_OPT_LEVEL=$level tools/build-release.sh)" "$RUNNER_TEMP/bin/opt-$level"
  done
fi
if [[ $SUITE == history-windows ]]; then
  previous=$(git rev-parse --verify --end-of-options "${PREVIOUS_REVISION}^{commit}")
  git worktree add -q --detach "$RUNNER_TEMP/window" "$previous"
  (
    cd "$RUNNER_TEMP/window"
    cp src/history.rs "$RUNNER_TEMP/history.rs"
    for size in 4 16 32; do
      sed "s/const WINDOW: u64 = 16 \* 1024/const WINDOW: u64 = $size * 1024/" "$RUNNER_TEMP/history.rs" > src/history.rs
      grep -Fq "const WINDOW: u64 = $size * 1024;" src/history.rs
      cp "$(tools/build-release.sh)" "$RUNNER_TEMP/bin/window-$size"
    done
  )
fi
if [[ $SUITE == profiles ]]; then
  git worktree add -q --detach "$RUNNER_TEMP/old" 3999b0e
  (cd "$RUNNER_TEMP/old"; cp "$(tools/build-release.sh)" "$RUNNER_TEMP/bin/previous-profile")
  for level in s z 2 3; do
    cp "$(CARGO_PROFILE_RELEASE_OPT_LEVEL=$level tools/build-release.sh)" "$RUNNER_TEMP/bin/opt-$level"
  done
fi
if [[ $SUITE == completion && $RUNNER_ARCH == X64 ]]; then
  git worktree add -q --detach "$RUNNER_TEMP/serial" 0f9ce86
  cp tools/build-release.sh "$RUNNER_TEMP/serial/tools/build-release.sh"
  (cd "$RUNNER_TEMP/serial"; cp "$(CARGO_PROFILE_RELEASE_OPT_LEVEL=2 tools/build-release.sh)" "$RUNNER_TEMP/bin/serial")
fi
for binary in "$RUNNER_TEMP"/bin/*; do
  file "$binary"
  wc -c "$binary"
  shasum -a 256 "$binary"
done >> results/host.txt
