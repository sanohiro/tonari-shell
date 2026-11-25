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
  bensch-*|spec-builtins)
    git -C upstream fetch -q --depth 1 https://github.com/tenseleyFlow/bensch.git c8e2a1ea493fc0dceb9af645dc445e619cb449e7
    git -C upstream checkout -q --detach FETCH_HEAD
    if [[ $SUITE == spec-builtins ]]; then
      if [[ $(uname -s) == Darwin ]] && ! command -v timeout >/dev/null; then
        brew list coreutils >/dev/null || brew install coreutils
        export PATH="$(brew --prefix coreutils)/libexec/gnubin:$PATH"
      fi
      for label in bash tonari; do
        shell=$ref
        [[ $label != tonari ]] || shell="$lab/bin/bash"
        (cd cwd && env -i HOME="$lab/home" PATH="$PATH" "$lab/upstream/bensch" \
          --shell "$shell" --profile bash --ref-shell "$ref" --suite builtins) \
          > "$out/upstream-$label.log" 2>&1 || echo "$?" > "$out/upstream-$label.status"
      done
      python3 "$repo/tools/ci/bensch-cases.py" upstream "$lab" > "$out/manifest.jsonl"
      python3 "$repo/tools/ci/spec-pairs.py" "$out/manifest.jsonl" "$out/cases.jsonl" \
        "$ref" "$lab/bin/bash" "$lab/pairs" --source upstream --observer "$RUNNER_TEMP/bin/paths"
      exit
    fi
    python3 -m venv venv
    venv/bin/pip -q install pexpect==4.9.0 PyYAML==6.0.3 colorama==0.4.6
    shard=${SUITE##*-}
    cd cwd
    env -i HOME="$lab/home" PATH="$PATH" TERM=xterm-256color PS1='$ ' \
      "$lab/venv/bin/python" "$repo/tools/ci/bensch-pty.py" "$lab/upstream" \
      "$out/pty.jsonl" "$ref" "$lab/bin/bash" --shard "$shard" --shards 8
    ;;
  oils-*)
    git -C upstream fetch -q --depth 1 https://github.com/oils-for-unix/oils.git d114978ca467143f171a835cbcbe8ab6ccc009a7
    git -C upstream checkout -q --detach FETCH_HEAD
    case "$(uname -sm)" in
      'Darwin arm64') arch=macos_arm64; sha=8d9f5d9b440bb4886df07f5e5038b87985e2b977dc048763d6699fa7b7186b19 ;;
      'Linux x86_64') arch=linux64; sha=78674cdf95eb4cb1dd5a9b68a70200387111cb29467f8a472ab7cf07a251653c ;;
      *) exit 2 ;;
    esac
    pypy=pypy2.7-v8.0.1-$arch
    curl -fsSL --retry 3 "https://downloads.python.org/pypy/$pypy.tar.gz" -o pypy.tar.gz
    echo "$sha  pypy.tar.gz" | shasum -a 256 -c -
    tar -xzf pypy.tar.gz
    mkdir pybin
    ln -s "$lab/$pypy/bin/python2" pybin/python2
    export PATH="$lab/pybin:$PATH"
    cd upstream
    PYTHONPATH=. python2 "$repo/tools/ci/oils-cases.py" > "$out/manifest.jsonl"
    python3 "$repo/tools/ci/spec-pairs.py" "$out/manifest.jsonl" "$out/cases.jsonl" \
      "$ref" "$lab/bin/bash" "$lab/pairs" --source . --observer "$RUNNER_TEMP/bin/paths" \
      --shards 4 --shard "${SUITE##*-}"
    PYTHONPATH=. python2 "$repo/tools/ci/oils-cases.py" grade < "$out/cases.jsonl" > "$out/grades.jsonl"
    ;;
esac
