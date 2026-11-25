#!/usr/bin/env bash
set -euo pipefail
suite=$1 tonari=$2 bash_ref=$3 previous=${4:-} opt3=${5:-}
set --
if [[ -n $previous ]]; then set -- --previous "$previous"; fi
if [[ -n $opt3 ]]; then set -- "$@" --opt3 "$opt3" --raw "$PWD/results/external-raw.json"; fi
if [[ -n ${EXTERNAL_BENCH_CASE:-} || -n ${EXTERNAL_BENCH_RUNS:-} ]]; then
  case "$suite" in
    gnu-hello|fumiyas|unixwin|luish)
      if [[ -n ${EXTERNAL_BENCH_CASE:-} ]]; then set -- "$@" --case "$EXTERNAL_BENCH_CASE"; fi
      if [[ -n ${EXTERNAL_BENCH_RUNS:-} ]]; then set -- "$@" --runs "$EXTERNAL_BENCH_RUNS"; fi ;;
    shellspec) [[ -n ${TONARI_COVERAGE_BINARY:-} ]] || { echo "ShellSpec timing uses its upstream controller" >&2; exit 2; } ;;
    *) echo "Per-case selection and trial counts require a bench_external.py suite" >&2; exit 2 ;;
  esac
fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
trace_paths() {
  if [[ -n ${TONARI_COVERAGE_BINARY:-} ]]; then
    local source=$1
    set --
    if [[ -n ${EXTERNAL_BENCH_CASE:-} ]]; then set -- --case "$EXTERNAL_BENCH_CASE"; fi
    python3 tools/ci/trace-external-paths.py "$suite" "$source" "$TONARI_COVERAGE_BINARY" "$@" > "$PWD/results/paths.jsonl"
    exit
  fi
}
case "$suite" in
  gnu-hello|gnu-profile|gnu-waits)
    curl -fsSL --retry 3 https://ftp.gnu.org/gnu/hello/hello-2.12.2.tar.gz -o "$work/hello.tar.gz"
    echo "5a9a996dc292cc24dcf411cee87e92f6aae5b8d13bd9c6819b4c7a9dce0818ab  $work/hello.tar.gz" | shasum -a 256 -c - >&2
    tar -xzf "$work/hello.tar.gz" -C "$work"
    trace_paths "$work/hello-2.12.2"
    if [[ $suite == gnu-waits ]]; then
      python3 tools/ci/configure-waits.py "$work/hello-2.12.2" "$tonari" "$bash_ref" "${tonari%/*}/observer" "$PWD/results"
    elif [[ $suite == gnu-profile ]]; then
      if [[ -z $previous && -n ${RUNNER_TEMP:-} && -x $RUNNER_TEMP/bin/baseline ]]; then
        previous=$RUNNER_TEMP/bin/baseline
        set -- "$@" --previous "$previous"
      fi
      python3 tools/bench_external.py gnu-hello "$work/hello-2.12.2" "$tonari" "$bash_ref" "$@"
      set --
      if [[ -n $previous ]]; then set -- "$previous"; fi
      python3 tools/ci/configure-profile.py "$work/hello-2.12.2/configure" "$tonari" "$bash_ref" "$PWD/results" "$@"
    else
      python3 tools/bench_external.py gnu-hello "$work/hello-2.12.2" "$tonari" "$bash_ref" "$@"
    fi
    ;;
  fumiyas)
    curl -fsSL --retry 3 https://raw.githubusercontent.com/fumiyas/fumiyas.github.io/d188b4fa60eca66f2bcd9f5a48a3c80a5c4fc307/2013/12/01/sh-benchmark-scripts.sh -o "$work/cases.sh"
    echo "87360d3da6988cbc5542fb339f37377b1802721353b1d89c1ceba5d773d29da7  $work/cases.sh" | shasum -a 256 -c - >&2
    command -v zsh >/dev/null || { echo 'fumiyas fixtures require zsh' >&2; exit 1; }
    trace_paths "$work/cases.sh"
    python3 tools/bench_external.py fumiyas "$work/cases.sh" "$tonari" "$bash_ref" "$@"
    ;;
  shellspec|check-shellspec|unixwin|luish)
    case "$suite" in
      shellspec|check-shellspec) repo=shellspec/shellbench rev=4b04d18ecd170948fcbb52a130c708fd3e08e1af ;;
      unixwin) repo=unixwin/shellbench rev=4fecc23d226ac5854cc2f1bbf74a6acf310c8ba8 ;;
      luish) repo=luispedro/luish rev=b43b78dd25fbd671c90b9e3265525d741089414d ;;
    esac
    git -C "$work" init -q
    git -C "$work" fetch -q --depth 1 "https://github.com/$repo.git" "$rev"
    git -C "$work" checkout -q --detach "$rev"
    trace_paths "$work"
    if [[ $suite == shellspec || $suite == check-shellspec ]]; then
      check=("$bash_ref" "$tonari")
      if [[ -n $previous ]]; then check+=("$previous"); fi
      if [[ -n $opt3 ]]; then check+=("$opt3"); fi
      python3 tools/ci/check-shellspec.py "$work/sample" "${check[@]}" > "$PWD/results/shellspec-witnesses.txt"
      if [[ $suite == check-shellspec ]]; then cat "$PWD/results/shellspec-witnesses.txt"; exit; fi
      mkdir "$work/bin" "$work/home"
      ln -s "$tonari" "$work/bin/tonari"
      ln -s "$bash_ref" "$work/bin/bash-ref"
      if [[ -n $previous ]]; then ln -s "$previous" "$work/bin/previous"; fi
      if [[ -n $opt3 ]]; then ln -s "$opt3" "$work/bin/opt3"; fi
      # The upstream controller signals its own process group to release its workers.
      python3 - "$work" <<'PY'
import os, pathlib, subprocess, sys
w = pathlib.Path(sys.argv[1])
env = dict(PATH=str(w/'bin') + os.pathsep + os.environ['PATH'], HOME=str(w/'home'), LC_ALL='C')
orders = ['tonari,bash-ref']
if (w/'bin/previous').exists():
    orders = ['tonari,bash-ref,previous', 'previous,bash-ref,tonari']
if (w/'bin/opt3').exists():
    orders = ['tonari,bash-ref,previous,opt3', 'opt3,previous,bash-ref,tonari']
for names in orders:
    print('Shell order ' + names, flush=True)
    subprocess.run(['./shellbench', '-s', names, '-w', '1', '-t', '1',
                    *sorted(str(f) for f in (w/'sample').glob('*.sh'))],
                   env=env, cwd=w, start_new_session=True, check=True)
PY
    else
      python3 tools/bench_external.py "$suite" "$work" "$tonari" "$bash_ref" "$@"
    fi
    ;;
  *) exit 2 ;;
esac
