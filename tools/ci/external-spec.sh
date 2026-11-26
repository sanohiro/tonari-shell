#!/usr/bin/env bash
set -euo pipefail
repo=$PWD
out="$repo/results"
lab="$RUNNER_TEMP/shell-spec"
mkdir -p "$lab/home" "$lab/cwd" "$lab/bin" "$out"
ref=/bin/bash
if [[ $SUITE == oils-control ]]; then
  source "$repo/tools/ci/bash-source.sh"
  download_bash "$version" "$digest" "$lab/bash.tar.gz"
  tar -xf "$lab/bash.tar.gz" -C "$lab"
  ref="$lab/bash-$version/bash"
  (cd "$lab/bash-$version" && ./configure --without-bash-malloc --disable-nls >/dev/null && make -j4 bash >/dev/null)
elif [[ $(uname -s) == Darwin ]]; then
  if [[ ! -x /opt/homebrew/bin/bash ]]; then brew install bash; fi
  ref=/opt/homebrew/bin/bash
fi
ln -s "$RUNNER_TEMP/bin/current" "$lab/bin/bash"
"$ref" --version | head -1 > "$out/bash-version.txt"
cd "$lab"
git init -q upstream
case "$SUITE" in
  bensch-*|spec-builtins|spec-tty|spec-profile-builtins|spec-previous-builtins)
    git -C upstream fetch -q --depth 1 https://github.com/tenseleyFlow/bensch.git c8e2a1ea493fc0dceb9af645dc445e619cb449e7
    git -C upstream checkout -q --detach FETCH_HEAD
    if [[ $SUITE != bensch-* ]]; then
      python3 "$repo/tools/ci/bensch-cases.py" upstream "$lab" "$ref" > "$out/manifest.jsonl"
      if [[ $SUITE == spec-builtins || $SUITE == spec-previous-builtins ]] &&
         [[ $(uname -s) == Darwin ]] && ! command -v timeout >/dev/null; then
        brew list coreutils >/dev/null || brew install coreutils
        export PATH="$(brew --prefix coreutils)/libexec/gnubin:$PATH"
      fi
      if [[ $SUITE == spec-previous-builtins ]]; then
        python3 "$repo/tools/ci/spec-pairs.py" "$out/manifest.jsonl" "$out/cases.jsonl" \
          "$ref" "$lab/bin/bash" "$lab/pairs" --source upstream
        exit
      fi
      if [[ $SUITE == spec-profile-builtins ]]; then
        python3 "$repo/tools/ci/spec-pairs.py" "$out/manifest.jsonl" "$out/profiles.jsonl" \
          "$RUNNER_TEMP/bin/current" "$RUNNER_TEMP/bin/opt-3" "$lab/pairs" --source upstream --runs 3
        exit
      fi
      python3 "$repo/tools/ci/bensch-tty.py" "$out/manifest.jsonl" "$ref" "$lab/bin/bash" > "$out/tty.jsonl"
      if [[ $SUITE == spec-tty ]]; then exit; fi
      for label in bash tonari; do
        shell=$ref
        [[ $label != tonari ]] || shell="$lab/bin/bash"
        (cd cwd && env -i HOME="$lab/home" PATH="$PATH" "$lab/upstream/bensch" \
          --shell "$shell" --profile bash --ref-shell "$ref" --suite builtins) \
          > "$out/upstream-$label.log" 2>&1 || echo "$?" > "$out/upstream-$label.status"
      done
      python3 "$repo/tools/ci/spec-pairs.py" "$out/manifest.jsonl" "$out/cases.jsonl" \
        "$ref" "$lab/bin/bash" "$lab/pairs" --source upstream --observer "$RUNNER_TEMP/bin/paths"
      exit
    fi
    python3 -m venv venv
    venv/bin/pip -q install pexpect==4.9.0 PyYAML==6.0.3 colorama==0.4.6
    shard=${SUITE##*-}
    shards=8
    options=(--observer "$RUNNER_TEMP/bin/paths")
    if [[ $SUITE == bensch-review-* || $SUITE == bensch-control-* ]]; then
      shards=2
      options=(--review "$repo/docs/compatibility/external-shell")
      [[ $SUITE != bensch-control-* ]] || options+=(--control)
    fi
    cd cwd
    env -i HOME="$lab/home" PATH="$PATH" TERM=xterm-256color PS1='$ ' \
      "$lab/venv/bin/python" "$repo/tools/ci/bensch-pty.py" "$lab/upstream" \
      "$out/pty.jsonl" "$ref" "$lab/bin/bash" --shard "$shard" --shards "$shards" "${options[@]}"
    ;;
  spec-fixes)
    REFERENCE_BASH="$ref" python3 "$repo/tests/shell.py" "$lab/bin/bash" -q
    python3 "$repo/tools/ci/spec-fix-timings.py" "$lab/bin/bash" "$ref" > "$out/fixes.tsv"
    ;;
  spec-response)
    python3 "$repo/tools/ci/bensch-response.py" "$ref" "$lab/bin/bash" > "$out/response.tsv"
    ;;
  spec-hot)
    python3 "$repo/tools/ci/spec-hot.py" "$ref" "$RUNNER_TEMP/bin/baseline" "$RUNNER_TEMP/bin/current" > "$out/hot.tsv"
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
    if [[ $SUITE == oils-fixes ]]; then
      python3 - "$out/manifest.jsonl" > "$lab/selected.jsonl" <<'PYTHON'
import json, sys
for line in open(sys.argv[1]):
    if json.loads(line)["suite"] in {"array-assoc", "builtin-kill", "dbracket", "regex"}:
        print(line, end="")
PYTHON
      python3 "$repo/tools/ci/spec-pairs.py" "$lab/selected.jsonl" "$out/cases.jsonl" \
        "$ref" "$lab/bin/bash" "$lab/pairs" --source .
      PYTHONPATH=. python2 "$repo/tools/ci/oils-cases.py" grade < "$out/cases.jsonl" > "$out/grades.jsonl"
      exit
    fi
    if [[ $SUITE == oils-control ]]; then
      utilities=()
      if [[ $(uname -s) == Darwin ]]; then
        brew list coreutils >/dev/null || brew install coreutils
        brew list gnu-sed >/dev/null || brew install gnu-sed
        utilities+=(--utility-path "$(brew --prefix gnu-sed)/libexec/gnubin"
                    --utility-path "$(brew --prefix coreutils)/libexec/gnubin")
      fi
      python3 "$repo/tools/ci/spec-pairs.py" "$out/manifest.jsonl" "$out/cases.jsonl" \
        "$ref" "$lab/bin/bash" "$lab/pairs" --source . --runs 1 "${utilities[@]}"
      PYTHONPATH=. python2 "$repo/tools/ci/oils-cases.py" grade < "$out/cases.jsonl" > "$out/grades.jsonl"
      exit
    fi
    if [[ $SUITE == oils-profile-* ]]; then
      python3 "$repo/tools/ci/spec-pairs.py" "$out/manifest.jsonl" "$out/profiles.jsonl" \
        "$RUNNER_TEMP/bin/current" "$RUNNER_TEMP/bin/opt-3" "$lab/pairs" --source . \
        --shards 4 --shard "${SUITE##*-}" --runs 3
      exit
    fi
    if [[ $SUITE == oils-argv || $SUITE == oils-printenv ]]; then
      fixture=${SUITE#oils-}
      mkdir -p "$lab/native-helper"
      cc -std=c99 -O2 "$repo/tools/ci/oils-$fixture.c" -o "$lab/native-helper/$fixture.py"
      python3 - "$out/manifest.jsonl" "$fixture.py" > "$lab/helper.jsonl" <<'PYTHON'
import base64
import json
import sys

for line in open(sys.argv[1]):
    row = json.loads(line)
    if row["include"] and sys.argv[2].encode() in base64.b64decode(row["code"]):
        print(line, end="")
PYTHON
      for helper in upstream native; do
        set -- --source .
        [[ $helper != native ]] || set -- "$@" --helper "$lab/native-helper"
        python3 "$repo/tools/ci/spec-pairs.py" "$lab/helper.jsonl" "$out/$helper.jsonl" \
          "$ref" "$lab/bin/bash" "$lab/pairs" "$@"
      done
      exit
    fi
    python3 "$repo/tools/ci/spec-pairs.py" "$out/manifest.jsonl" "$out/cases.jsonl" \
      "$ref" "$lab/bin/bash" "$lab/pairs" --source . --observer "$RUNNER_TEMP/bin/paths" \
      --shards 4 --shard "${SUITE##*-}"
    PYTHONPATH=. python2 "$repo/tools/ci/oils-cases.py" grade < "$out/cases.jsonl" > "$out/grades.jsonl"
    ;;
esac
