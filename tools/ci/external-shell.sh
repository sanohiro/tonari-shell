#!/usr/bin/env bash
set -euo pipefail
suite=$1 tonari=$2 bash_ref=$3
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
case "$suite" in
  gnu-hello|gnu-profile)
    curl -fsSL --retry 3 https://ftp.gnu.org/gnu/hello/hello-2.12.2.tar.gz -o "$work/hello.tar.gz"
    echo "5a9a996dc292cc24dcf411cee87e92f6aae5b8d13bd9c6819b4c7a9dce0818ab  $work/hello.tar.gz" | shasum -a 256 -c - >&2
    tar -xzf "$work/hello.tar.gz" -C "$work"
    if [[ $suite == gnu-profile ]]; then
      python3 tools/ci/configure-profile.py "$work/hello-2.12.2/configure" "$tonari" "$bash_ref" "$PWD/results"
    else
      python3 tools/bench_external.py gnu-hello "$work/hello-2.12.2" "$tonari" "$bash_ref"
    fi
    ;;
  fumiyas)
    curl -fsSL --retry 3 https://raw.githubusercontent.com/fumiyas/fumiyas.github.io/d188b4fa60eca66f2bcd9f5a48a3c80a5c4fc307/2013/12/01/sh-benchmark-scripts.sh -o "$work/cases.sh"
    echo "87360d3da6988cbc5542fb339f37377b1802721353b1d89c1ceba5d773d29da7  $work/cases.sh" | shasum -a 256 -c - >&2
    command -v zsh >/dev/null || { echo 'fumiyas fixtures require zsh' >&2; exit 1; }
    python3 tools/bench_external.py fumiyas "$work/cases.sh" "$tonari" "$bash_ref"
    ;;
  shellspec|unixwin|luish)
    case "$suite" in
      shellspec) repo=shellspec/shellbench rev=4b04d18ecd170948fcbb52a130c708fd3e08e1af ;;
      unixwin) repo=unixwin/shellbench rev=4fecc23d226ac5854cc2f1bbf74a6acf310c8ba8 ;;
      luish) repo=luispedro/luish rev=b43b78dd25fbd671c90b9e3265525d741089414d ;;
    esac
    git -C "$work" init -q
    git -C "$work" fetch -q --depth 1 "https://github.com/$repo.git" "$rev"
    git -C "$work" checkout -q --detach "$rev"
    if [[ $suite == shellspec ]]; then
      mkdir "$work/bin" "$work/home"
      ln -s "$tonari" "$work/bin/tonari"
      ln -s "$bash_ref" "$work/bin/bash-ref"
      # The upstream controller signals its own process group to release its workers.
      python3 - "$work" <<'PY'
import os, pathlib, subprocess, sys
w = pathlib.Path(sys.argv[1])
env = dict(PATH=str(w/'bin') + os.pathsep + os.environ['PATH'], HOME=str(w/'home'), LC_ALL='C')
subprocess.run(['./shellbench', '-s', 'tonari,bash-ref', '-w', '1', '-t', '1',
                *sorted(str(f) for f in (w/'sample').glob('*.sh'))],
               env=env, cwd=w, start_new_session=True, check=True)
PY
    else
      python3 tools/bench_external.py "$suite" "$work" "$tonari" "$bash_ref"
    fi
    ;;
  *) exit 2 ;;
esac
