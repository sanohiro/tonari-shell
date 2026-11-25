#!/usr/bin/env bash
set -euo pipefail
repo=$PWD
out="$repo/results"
lab="$RUNNER_TEMP/shell-spec"
mkdir -p "$lab/home" "$lab/cwd" "$lab/bin" "$out"
ref=/bin/bash
if [[ $(uname -s) == Darwin ]]; then
  if [[ ! -x /opt/homebrew/bin/bash ]]; then brew install bash; fi
  ref=/opt/homebrew/bin/bash
fi
ln -s "$RUNNER_TEMP/bin/current" "$lab/bin/bash"
"$ref" --version | head -1 > "$out/bash-version.txt"
cd "$lab"
git init -q upstream
case "$SUITE" in
  bensch-*)
    git -C upstream fetch -q --depth 1 https://github.com/tenseleyFlow/bensch.git c8e2a1ea493fc0dceb9af645dc445e619cb449e7
    git -C upstream checkout -q --detach FETCH_HEAD
    python3 -m venv venv
    venv/bin/pip -q install pexpect==4.9.0 PyYAML==6.0.3 colorama==0.4.6
    shard=${SUITE##*-}
    cd cwd
    env -i HOME="$lab/home" PATH="$PATH" TERM=xterm-256color PS1='$ ' \
      "$lab/venv/bin/python" "$repo/tools/ci/bensch-pty.py" "$lab/upstream" \
      "$out/pty.jsonl" "$ref" "$lab/bin/bash" --shard "$shard" --shards 8
    ;;
esac
