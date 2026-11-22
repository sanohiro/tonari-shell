# Startup and execution / 起動と実行性能

## Performance target / 性能目標

For each workload, divide Tonari's elapsed time by GNU Bash's elapsed time on the same machine and in the same run. A ratio of **0.50** is the goal and **2/3** is the upper target. Ratios between 2/3 and 1.00 identify work that is faster than Bash but still above target. The harness reports both median and p95 ratios, and labels the median result. Direct executable launch time and binary size are reported separately.

各処理で、同じマシン・同じ計測での Tonari の所要時間を GNU Bash の所要時間で割ります。目標は **0.50**、許容する上限は **2/3** です。比が 2/3 より大きく 1.00 未満なら、Bash より速くても目標には未達です。計測器は中央値と p95 の比を表示し、中央値に対する達成状況を示します。実行ファイルの直接起動とバイナリサイズは別項目です。

```sh
python3 tools/bench.py target/release/tonari --bash /bin/bash --runs 300
python3 tools/bench.py target/release/tonari --case session --runs 300
python3 tools/bench.py target/release/tonari --case core --compare-tonari Previous /path/to/previous/tonari --runs 300
```

The default run includes a session workload covering assignment, export, parameter expansion, `cd`, `pwd`, `&&`, redirection and `unset`. It also times 1,000 arithmetic-loop iterations and 16 consecutive external commands inside one shell. For the latter it rotates in a Python parent launching the same 16 executables with `posix_spawn` and waiting for each. The launcher configures standard descriptors before timing; each child inherits them. Use `--case core` for comparisons with binaries predating session support. The primary prompt time spans the complete PTY launch. The harness also timestamps the child immediately before exec to show how much of that same sample follows the launcher's terminal setup and fork.

既定の計測には、代入・export・変数展開・`cd`・`pwd`・`&&`・リダイレクト・`unset` を同じシェルで実行する項目が含まれます。算術ループ1,000回と、一つのシェルから外部プログラム16個を順に起動する項目もあります。後者では同じプログラム16個を Python 親プロセスから直接 `posix_spawn` し、その都度終了を待つ条件も交互に計測します。標準入出力は計時前に用意し、各子プロセスに継承させます。セッション機能の実装前との比較には `--case core` を使います。プロンプト表示の主指標は疑似端末での起動全体です。子プロセスの exec 直前にも時刻を記録し、同じ測定サンプルのうち、端末の準備と fork の後に経過した時間も表示します。

<a id="capture-and-spawn"></a>

## Command substitution, process launch and patterns / コマンド置換・プロセス起動・パターン

On 2026-11-22 an Apple M1 ran macOS 27.2.1 natively and Debian 13 arm64 in a VM. We rotated the previous release (`dbde5fe`), the updated release and GNU Bash 5.2.37. Each condition used five warm-ups, then 30 measured runs on macOS or 250 on Debian. The temporary home, scripts, fixtures and executables were on each OS's local filesystem. All shells used `LC_ALL=C`, warm caches and disabled startup files. Every run checked stdout, stderr and exit status. Tables show **median / p95 in milliseconds**. Whole spans launch through exit. Started begins after the ready marker and includes sourcing the script and collecting the result. [Script driver](../tools/bench_scripts.py)

2026-11-22、M1 のネイティブ macOS と同じマシン上の Debian 13 arm64 VM で、旧版・新版・Bash 5.2.37 を交互に実行した。各条件で5回ウォームアップした後、Mac は30回、Debian は250回測定した。各 OS のローカルディスクと温まったキャッシュを使い、出力・標準エラー・終了値を毎回確認している。表は中央値 / p95（ms）。Whole は起動から終了まで、Started は起動済みシェルへ実行許可を送ってからスクリプトを読み込み、終了結果を受け取るまで。

### macOS arm64

| Workload / 処理 | Timing / 区間 | Previous | Tonari | Bash |
|---|---|---:|---:|---:|
| 64 × `$(printf …)` | Whole | 45.225 / 57.215 | 4.391 / 6.480 | 59.866 / 74.905 |
| 64 × `$(printf …)` | Started | 44.217 / 56.791 | 0.695 / 1.542 | 52.581 / 65.089 |
| 16 × `$(/usr/bin/printf …)` | Whole | 35.486 / 51.407 | 25.173 / 34.495 | 43.085 / 53.650 |
| 16 × `$(/usr/bin/printf …)` | Started | 32.277 / 46.950 | 21.006 / 28.854 | 35.450 / 48.887 |
| 1,000 × `[[ =~ ]]` | Whole | 12.054 / 14.025 | 6.570 / 8.419 | 15.664 / 19.997 |
| 1,000 × `[[ =~ ]]` | Started | 7.819 / 9.031 | 2.611 / 3.720 | 8.310 / 9.527 |
| 30 × `set -- fixtures/*.txt` (512 files) | Whole | 31.911 / 48.623 | 17.759 / 21.911 | 1148.808 / 1662.967 |
| 30 × `set -- fixtures/*.txt` (512 files) | Started | 28.573 / 40.128 | 14.095 / 18.297 | 1162.199 / 1529.108 |
| 2,000 basename / extension trims | Whole | 8.890 / 9.811 | 9.094 / 10.132 | 18.058 / 19.634 |
| 2,000 basename / extension trims | Started | 5.227 / 5.812 | 5.436 / 5.821 | 11.286 / 13.308 |

### Debian arm64

| Workload / 処理 | Timing / 区間 | Previous | Tonari | Bash | Direct C |
|---|---|---:|---:|---:|---:|
| 16 × `/usr/bin/true` | Whole | 3.774 / 5.269 | 3.749 / 5.221 | 5.391 / 7.330 | 3.438 / 4.471 |
| 16 × `/usr/bin/true` | Started | 3.659 / 6.302 | 3.621 / 5.071 | 5.031 / 7.347 | 3.503 / 5.106 |
| 32 exports, 16 × `/usr/bin/printenv` | Whole | 5.095 / 7.201 | 4.733 / 8.043 | 6.573 / 11.040 | — |
| 32 exports, 16 × `/usr/bin/printenv` | Started | 4.448 / 7.569 | 4.191 / 7.561 | 5.538 / 8.930 | — |

The native C control was built with Alpine musl and used the same pipes, environment and ready protocol as Tonari. It directly launches `/usr/bin/true` 16 times and waits for each. In the started-shell run the C control also exceeded two-thirds of Bash's median. A loop with a fixed executable bounds what we can save in the shell on this machine. With 32 exports, the started-shell median fell from 4.448 to 4.191 ms. The whole-script p95 rose from 7.201 to 8.043 ms. [Native control](../tools/bench_spawn.c)

C の対照は Alpine の musl でビルドし、同じパイプ・環境・開始合図を使った。起動後の計測では C で直接起動して毎回終了を待っても Bash の中央値の 2/3 を超えた。ここでは実行ファイルと待ち方が同じなので、シェル内部で短縮できる時間は限られる。32変数を export し、子が値を受け取るケースでは起動後の中央値が 4.448 ms から 4.191 ms に短縮した。起動込みの p95 は 7.201 ms から 8.043 ms に増えた。

The macOS release was 836,336 bytes, up from 819,776. The Linux static PIE was 987,496 bytes with packed relative relocations. The ordinary link of the same source was 1,053,032 bytes. Packing produced an ELF `.relr.dyn` section, avoided the extra file padding and retained the single-executable layout. Debian and Ubuntu both ran the packed binary through the shell test suite. [Linux release recipe](#linux-arm64-static-pie)

macOS 版は 819,776 バイトから 836,336 バイトになった。Linux arm64 静的 PIE は相対再配置情報を圧縮して 987,496 バイト。同じソースの通常リンクで観測した 1,053,032 バイトには、セグメント境界のパディングも含まれていた。圧縮後の単一バイナリは Debian と Ubuntu でテストした。

```sh
# Build the native control with the Alpine musl compiler for the Linux run.
cc -Os -static-pie tools/bench_spawn.c -o /tmp/bench-spawn
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /bin/bash --spawn-control /tmp/bench-spawn --case external --case external-env --runs 250
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /path/to/bash-5.2.37 --case substitutions --case external-substitutions --case regex --case glob --case strings --runs 30
```

<a id="script-execution"></a>

## Running shell scripts / シェルスクリプトの実行

Each named script ran in two modes. Whole-script timing spans the Python launch request through child exit. In the started-shell mode, the driver first waits for a ready marker from a fresh noninteractive shell. It then starts the clock, releases the shell from a builtin read, and sources the same file. That interval includes opening, parsing, executing the script and collecting its result. Both modes verify stdout, stderr and exit status every time. [Reproducible scripts and driver](../tools/bench_scripts.py)

同じスクリプトを、ファイル名を渡して起動する場合と、起動済みのシェルで source する場合で測った。後者は準備完了を受け取ってから計時を始め、ファイルの読み込み・解析・実行・終了の確認までを含む。各回で出力と終了値を照合した。

The Apple M1 ran native macOS and a Debian 13 arm64 VM. Each host ran locally stored release binaries and fixtures, GNU Bash 5.2.37, a temporary home, `LC_ALL=C` and warm filesystem caches. The driver rotated the updated Tonari, Bash and Tonari `903f975` for five warm-ups and 30 measured runs per case and mode. Cells report median / nearest-rank p95 in milliseconds.

M1 のネイティブ macOS と Debian 13 arm64 の VM で、各 OS のローカルファイルシステムにバイナリと入力を置いた。各条件で旧版・新版・Bash 5.2.37 の起動順を入れ替え、5回のウォームアップ後に各30回計測した。表は中央値 / 95 パーセンタイル（ms）。

### Script timings on macOS 27.2.1 arm64

| Script / 処理 | Whole Tonari | Whole Bash | Started Tonari | Started Bash |
|---|---:|---:|---:|---:|
| 10,000 integer additions | 13.838 / 19.157 | 21.354 / 23.313 | 10.122 / 11.643 | 14.787 / 15.288 |
| 4,000 function calls with local variables | 13.034 / 14.418 | 23.487 / 25.129 | 9.593 / 10.248 | 16.820 / 17.338 |
| 2,000 basename and extension trims | 8.996 / 9.423 | 17.680 / 19.701 | 5.137 / 5.654 | 11.168 / 12.381 |
| 2,000 lines via `IFS= read -r` | 10.315 / 15.211 | 17.618 / 34.060 | 6.679 / 9.624 | 10.713 / 13.918 |
| 64 builtin command substitutions | 44.078 / 45.762 | 58.038 / 63.804 | 40.756 / 44.604 | 52.720 / 57.455 |
| 16 sequential `/usr/bin/true` calls | 23.576 / 25.424 | 39.243 / 41.398 | 20.342 / 23.535 | 33.466 / 40.545 |

### Script timings on Debian 13 arm64

| Script / 処理 | Whole Tonari | Whole Bash | Started Tonari | Started Bash |
|---|---:|---:|---:|---:|
| 10,000 integer additions | 10.233 / 11.905 | 23.842 / 24.354 | 9.408 / 10.483 | 23.417 / 25.234 |
| 4,000 function calls with local variables | 12.116 / 13.933 | 24.925 / 25.676 | 10.228 / 12.063 | 24.232 / 27.205 |
| 2,000 basename and extension trims | 5.232 / 6.933 | 14.748 / 15.521 | 4.929 / 5.559 | 14.299 / 15.117 |
| 2,000 lines via `IFS= read -r` | 5.360 / 6.003 | 11.053 / 12.477 | 5.249 / 6.129 | 10.609 / 11.415 |
| 64 builtin command substitutions | 8.562 / 9.853 | 14.691 / 15.467 | 8.228 / 9.209 | 14.096 / 16.344 |
| 16 sequential `/usr/bin/true` calls | 3.628 / 3.966 | 5.176 / 5.939 | 3.302 / 4.151 | 4.604 / 5.069 |

The previous M1 build took 13.837 ms median on the started-shell string workload. The updated build took 5.137 ms, versus 11.168 ms for Bash. The 64 command substitutions on that host still took over two-thirds of Bash's time in both timing modes. The selected fast path kept the release binaries at 819,776 bytes on macOS and 987,496 bytes for static arm64 musl. [Engineering notes](engineering-notes.md)

M1 で起動後にパスを加工する処理は、変更前 13.837 ms、変更後 5.137 ms、Bash 11.168 ms だった。M1 のコマンド置換64回は起動込み・起動後ともに Bash の所要時間の 2/3 を超えている。

A separate 50-run check loaded the real nvm 0.40.5 with `--no-use` from an empty `NVM_DIR` and verified the resulting function definition. Time includes shell startup.

実際の nvm 0.40.5 を空の `NVM_DIR` から読み込み、関数が定義されたことを確認した。起動を含む各50回の値。

| Host | Tonari | Bash |
|---|---:|---:|
| macOS 27.2.1 arm64 | 6.013 / 6.953 | 12.385 / 12.997 |
| Debian 13 arm64 | 4.502 / 5.065 | 7.620 / 8.755 |

```sh
python3 tools/bench_scripts.py target/release/tonari --bash /path/to/bash --runs 30
python3 tools/bench_scripts.py target/release/tonari --bash /path/to/bash --previous /path/to/old/tonari --case strings
python3 tools/bench.py target/release/tonari --bash /path/to/bash --case nvm --nvm-script /path/to/nvm.sh --runs 50
```

## Integrated release build / 統合リリースビルド

Measured on 2026-11-22, after process substitution, shell command hashing, parameter modifiers, and parser allocation changes. The source subtree is `527055de2275e45b0ef7c5a83bcb72d826a96e94`. From a checkout, run `git rev-parse HEAD:src` to compare. The harness is blob `1f7f36916222e276727e0cdd8802381c9e15fdf1`. Build flags, hardware, reference binaries and workloads match the interactive-shell milestone below. Each workload had five warm-ups and 300 rotated samples per shell, with 100 measured for nvm. Time cells are **median / p95 in milliseconds**, ratio cells are the two corresponding Tonari/Bash ratios.

プロセス置換、PATH 検索キャッシュ、追加のパラメータ展開とパーサーのメモリ確保改善を含む統合版を、下記マイルストーンと同一条件で測定しました。各5回のウォームアップ後、起動順を入れ替えて各300回、nvm は各100回計測しました。時間は **中央値 / p95（ミリ秒）**、比はそれぞれの統計量を Bash で割ったものです。

### Debian 13 arm64

| Metric / 項目 | Tonari | GNU Bash 5.2.37 | Tonari/Bash |
|---|---:|---:|---:|
| First prompt / プロンプト | 0.623 / 0.847 | 0.969 / 1.360 | 0.643 / 0.623 |
| Exec to prompt / exec 以降 | 0.334 / 0.462 | 0.681 / 0.967 | 0.490 / 0.477 |
| `-c ':'` | 0.196 / 0.252 | 0.418 / 0.566 | 0.468 / 0.446 |
| One external / 外部1個 | 0.303 / 0.384 | 0.526 / 0.642 | 0.576 / 0.598 |
| Pipeline / パイプ | 0.542 / 0.672 | 0.839 / 1.057 | 0.646 / 0.636 |
| Boolean builtins / 真偽値 | 0.194 / 0.247 | 0.414 / 0.510 | 0.467 / 0.485 |
| 100 redirects / リダイレクト100回 | 0.451 / 0.563 | 0.886 / 1.066 | 0.510 / 0.529 |
| Session builtins / セッション | 0.219 / 0.279 | 0.429 / 0.511 | 0.510 / 0.546 |
| 16 external programs / 外部16個 | 3.396 / 4.778 | 4.930 / 7.573 | 0.689 / 0.631 |
| 1,000 loop iterations / ループ1,000回 | 0.914 / 1.135 | 2.338 / 2.934 | 0.391 / 0.387 |
| nvm 0.40.5 `--no-use` | 4.377 / 5.444 | 7.584 / 9.600 | 0.577 / 0.567 |

### macOS 27.2.1 arm64

| Metric / 項目 | Tonari | GNU Bash 5.2.37 | Apple Bash 3.2.57 | Tonari/5.2.37 | Tonari/3.2.57 |
|---|---:|---:|---:|---:|---:|
| First prompt / プロンプト | 4.806 / 5.926 | 7.906 / 9.752 | 5.573 / 6.818 | 0.608 / 0.608 | 0.862 / 0.869 |
| Exec to prompt / exec 以降 | 3.236 / 4.070 | 6.443 / 8.295 | 4.049 / 5.072 | 0.502 / 0.491 | 0.799 / 0.802 |
| `-c ':'` | 2.587 / 3.195 | 5.654 / 7.148 | 3.248 / 4.356 | 0.458 / 0.447 | 0.796 / 0.733 |
| One external / 外部1個 | 3.892 / 5.351 | 7.165 / 9.058 | 4.640 / 6.127 | 0.543 / 0.591 | 0.839 / 0.873 |
| Pipeline / パイプ | 4.332 / 5.894 | 8.450 / 10.715 | 5.819 / 7.435 | 0.513 / 0.550 | 0.744 / 0.793 |
| Boolean builtins / 真偽値 | 2.693 / 3.928 | 5.701 / 7.212 | 3.293 / 4.339 | 0.472 / 0.545 | 0.818 / 0.905 |
| 100 redirects / リダイレクト100回 | 3.744 / 5.045 | 7.028 / 8.447 | 4.627 / 5.960 | 0.533 / 0.597 | 0.809 / 0.846 |
| Session builtins / セッション | 2.779 / 4.120 | 5.894 / 8.271 | 3.370 / 5.158 | 0.472 / 0.498 | 0.825 / 0.799 |
| 16 external programs / 外部16個 | 22.234 / 31.298 | 37.592 / 46.769 | 34.522 / 43.780 | 0.591 / 0.669 | 0.644 / 0.715 |
| 1,000 loop iterations / ループ1,000回 | 3.619 / 4.600 | 7.303 / 9.829 | 5.559 / 7.454 | 0.496 / 0.468 | 0.651 / 0.617 |
| nvm 0.40.5 `--no-use` | 5.907 / 7.854 | 12.068 / 14.147 | 9.494 / 11.518 | 0.489 / 0.555 | 0.622 / 0.682 |

Against GNU Bash 5.2.37, every macOS median meets the 2/3 target. Debian meets it on every case except 16 consecutive external commands, at 0.689. In this build nvm loads in 4.377 ms on Debian, ratio 0.577. Against Apple’s system Bash 3.2.57, every Tonari median is faster and the 16-external series, arithmetic loop and nvm meet 2/3. Direct single `/usr/bin/true` launch was 0.210 / 0.257 ms on Debian and 1.174 / 1.888 ms on macOS.

GNU Bash 5.2.37 比の中央値は、macOS では全項目が 2/3 以下、Debian では外部16個（0.689）以外が 2/3 以下でした。Debian の nvm 読み込みは 4.377 ms、Bash 比 0.577 です。Apple 同梱の Bash 3.2.57 と比べても中央値は全項目で短く、外部16個・算術ループ・nvm が 2/3 以下でした。`/usr/bin/true` 一つの直接起動は Debian が 0.210 / 0.257 ms、macOS が 1.174 / 1.888 ms です。

| Host / ホスト | Tonari (bytes) | GNU Bash 5.2.37 | Apple Bash 3.2.57 |
|---|---:|---:|---:|
| Debian 13 arm64 | 987,496 | 1,412,096 | — |
| macOS 27.2.1 | 819,760 | 1,119,952 | 1,961,872 universal / 667,584 arm64e slice |

The measured Tonari executables have SHA-256 `09e9f4e0ce62a47cb916a3f476d5abae4c360019a8a894f6e2769858779c7c52` on Linux and `96bc503b5575fac344da3d6cdb17e335d3ef27533dc022a286028723075b86fb` on macOS.

A separate 300-sample rotation timed a persistent C launcher as well as the Python control for 16 child processes. C uses the same libc as Tonari; its own startup is outside the clock. The [control methodology](#starting-16-real-child-processes--外部16プロセスの直接起動) gives the exact timing boundaries.

外部16個の再計測では、同じ libc の常駐 C プロセスも加えて各300回交互に測りました。C 版の計時区間には親自身の起動は含まれません。

| Host / ホスト | Tonari | GNU Bash 5.2.37 | Apple Bash 3.2.57 | Direct C | Direct Python |
|---|---:|---:|---:|---:|---:|
| Debian 13 arm64 | 3.338 / 4.148 | 4.845 / 6.399 | — | 3.062 / 4.091 | 3.454 / 4.693 |
| macOS 27.2.1 | 21.912 / 28.357 | 37.052 / 45.151 | 34.643 / 42.971 | 19.007 / 24.170 | 19.260 / 23.717 |

<a id="first-key-history"></a>

## First keystroke with shared history / 共有履歴と最初のキー入力

On an Apple M1 running macOS 27.2.1, each sample launched the native release build of `76cc5f1` in a fresh PTY and a temporary home with `--noprofile --norc`, `TERM=xterm-256color` and `PS1='tonari$ '`. After reading the initial prompt, the driver timed writing `z` until reading the redrawn `tonari$ z`. Each journal contained 31-byte ASCII rows of the form `1\t1\t0\t/tmp\tprintf entry-000000\n`, with actual tabs and a newline and a six-digit counter starting at zero. No row matched the prefix. Sizes rotated between launches, with three warm-ups then 30 samples per size. p95 uses the nearest rank.

| Journal entries / 履歴件数 | Bytes | Median (ms) | p95 (ms) |
|---:|---:|---:|---:|
| 0 | 0 | 0.063 | 0.091 |
| 1,000 | 31,000 | 0.301 | 0.363 |
| 100,000 | 3,100,000 | 20.635 | 21.060 |

macOS 27.2.1 の M1 で、各回新しい疑似端末と一時ホームを使いました。起動済みのプロンプトに最初の `z` を送ってから、その文字が再描画されるまでを測っています。初回の履歴読み込みと、一致する候補がない場合の検索を含みます。履歴件数を入れ替え、各条件3回のウォームアップ後に30回計測しました。

### Input-priority history refresh / 履歴読み込み中のキー応答

We reran the preceding warm-cache M1 test with the old and updated releases and the same journals. Three warm-ups and 30 measured runs rotated through the builds, entry counts and actions. Immediately after observing the first character, the PTY driver sent Tab to complete the sole command `zcomplete`, or Enter to run `:` and return to the next prompt. A separate run sent `p` and waited for the newest matching history suggestion to become visible. Each cell is **median / p95 in milliseconds**. [Reproducible harness](../tools/bench_history.py)

同じ M1 と履歴で旧版・新版を交互に起動しました。各条件3回のウォームアップ後に30回計測しています。最初の一文字を読み取った直後に Tab でコマンド名を補完するか、Enter で `:` を実行して次のプロンプトを待ちました。別の試行で `p` を入力してから履歴候補そのものが表示されるまでも測りました。

| Entries / 件数 | Interval / 計測区間 | Previous | Updated |
|---:|---|---:|---:|
| 0 | First `z` → redraw | 0.067 / 0.086 | 0.029 / 0.041 |
| 1,000 | First `z` → redraw | 0.300 / 0.345 | 0.024 / 0.038 |
| 100,000 | First `z` → redraw | 20.499 / 21.161 | 0.036 / 0.050 |
| 100,000 | Tab → completed word | 0.355 / 0.399 | 0.090 / 0.167 |
| 100,000 | First `z` → completed word after Tab | 20.858 / 21.512 | 0.132 / 0.207 |
| 100,000 | Enter → next prompt | 0.299 / 0.352 | 0.191 / 0.236 |
| 100,000 | First `:` → next prompt after Enter | 20.953 / 21.982 | 0.218 / 0.271 |
| 100,000 | First `p` → visible history suggestion | 20.270 / 21.711 | 12.448 / 13.258 |

Repeat with `python3 tools/bench_history.py /path/to/previous/tonari target/release/tonari`. Both macOS arm64 executables were 819,760 bytes.

<a id="completion-latency"></a>

## Tab followed by typing / Tab 直後の打鍵

Each fresh PTY received a prefix, then Tab and `@` in a single write. The timer stopped when the PTY delivered `@`. A full redraw outside the timed interval verified the line ended in `item-0@`. One directory contained 10,000 executable files named `item-00000.dat` through `item-09999.dat` and was the only directory on PATH. The completion cases used an empty history and an rc file registering `complete -f demo`. Caches were warm. The builds rotated through three warm-ups and 30 measured sessions per case. All times are **median / p95 milliseconds**, with nearest-rank p95. [Benchmark driver](../tools/bench_completion.py)

各回新しい疑似端末を開き、入力済みの文字列に続けて Tab と `@` を同時に送った。`@` が端末に返るまでを測り、計測後に行を再表示して、補完した文字列の後ろに `@` が入ったことを確認した。一つのディレクトリに実行可能ファイル1万個を用意し、PATH にもそのディレクトリだけを指定した。各条件3回のウォームアップ後、順番を入れ替えながら各30回計測した。表示値は中央値 / 95 パーセンタイル（ms）。

| Host / ホスト | Tab case / 対象 | Tonari `c23b986` | Updated / 変更後 | GNU Bash 5.2.37 |
|---|---|---:|---:|---:|
| macOS M1 | File | 23.541 / 24.566 | 5.306 / 5.775 | 10.466 / 11.204 |
| macOS M1 | Registered `complete -f` | 36.043 / 37.955 | 5.316 / 5.622 | 11.241 / 11.681 |
| macOS M1 | Command on PATH | 22.024 / 23.707 | 5.160 / 5.539 | 159.168 / 162.314 |
| Debian 13 arm64 | File | 15.485 / 17.599 | 2.165 / 2.345 | 4.603 / 4.907 |
| Debian 13 arm64 | Registered `complete -f` | 22.118 / 23.763 | 2.125 / 2.287 | 5.932 / 6.718 |
| Debian 13 arm64 | Command on PATH | 14.892 / 16.178 | 1.919 / 2.115 | 66.534 / 70.168 |

A separate Tonari-only run started after loading 100,000 journal entries. The PTY sent one character per redraw for a prefix matching the newest entry (390 keystrokes) and a prefix with no match (330 keystrokes including UTF-8). The oldest-entry case measured the same keystroke twice, first to the appearance of the typed character and then to its suggestion. These timings end at the PTY and exclude terminal-emulator rendering and display refresh.

履歴候補の計測では10万件の読み込みを終えてから、一文字が再描画されるたびに次の文字を送った。最新履歴に一致する390打鍵と、日本語を含み候補がない330打鍵を集計した。最古の履歴だけに一致する試行は、同じ打鍵から入力文字と履歴候補がそれぞれ返るまでを測定した。計測の終点は疑似端末の出力で、端末アプリの描画やディスプレイの更新時間は含まない。

| Host / ホスト | Interval / 計測区間 | Tonari `c23b986` | Updated / 変更後 |
|---|---|---:|---:|
| macOS M1 | Keystroke, newest match | 0.013 / 0.027 | 0.012 / 0.023 |
| macOS M1 | Keystroke, no match | 0.269 / 0.384 | 0.013 / 0.026 |
| macOS M1 | Oldest match, typed character | 0.378 / 0.410 | 0.015 / 0.047 |
| macOS M1 | Oldest match, visible hint | 0.381 / 0.424 | 0.682 / 0.852 |
| Debian 13 arm64 | Keystroke, newest match | 0.061 / 0.094 | 0.060 / 0.086 |
| Debian 13 arm64 | Keystroke, no match | 0.269 / 0.335 | 0.060 / 0.258 |
| Debian 13 arm64 | Oldest match, typed character | 0.269 / 0.331 | 0.231 / 0.273 |
| Debian 13 arm64 | Oldest match, visible hint | 0.276 / 0.338 | 0.263 / 0.305 |

The M1 ran macOS 27.2.1 natively and Debian 13 arm64 in a Linux VM with the fixture on the VM filesystem. The updated release executables were 819,776 bytes on macOS (16 additional bytes) and 987,496 bytes on Debian (static musl). [Implementation notes](engineering-notes.md)

M1 の macOS 27.2.1 と、その上の Linux VM で動く Debian 13 arm64 で計測した。Debian のテストデータは VM 内に作成した。変更後の実行ファイルは macOS が 819,776 バイト（16 バイト増）、Debian の静的 musl 版が 987,496 バイト（同サイズ）。

```sh
python3 tools/bench_completion.py /path/to/previous/tonari target/release/tonari --bash /path/to/bash
```

<a id="completion-listing"></a>

### Second Tab and full candidate list / 2回目の Tab と候補一覧

In a fresh PTY, the driver typed the full common prefix and pressed Tab once. After the shell acknowledged it, the driver sent a second Tab and `@` together. It timed the interval to the echoed `@`, then verified that every expected filename had arrived and that the editable line still ended in `@`. For full output at 10,000 entries, Bash ran with Readline's `completion-query-items` set to zero and `page-completions` off. Tonari used its default listing. Each shell ran three warm-ups and 30 measured sessions with warm local filesystems, alternating shell order. These are PTY timings, in milliseconds, for Tonari `130ecd8` and GNU Bash 5.2.37.

共通部分をすべて入力して最初の Tab を押し、その応答後に2回目の Tab と `@` を一緒に送った。全候補の後に `@` が返るまでを計り、ファイル名の集合と入力行を検証した。1万件を最後まで出力できるよう、Bash は件数確認とページ送りを無効にし、Tonari は既定設定で測った。各条件は3回のウォームアップ後に各30回。端末アプリの描画時間は含まない。

| Host / ホスト | Files / 件数 | Tonari median / p95 | Bash median / p95 |
|---|---:|---:|---:|
| macOS M1 | 10 | 0.068 / 0.108 | 0.097 / 0.127 |
| macOS M1 | 10,000 | 7.954 / 8.559 | 34.246 / 36.265 |
| Debian 13 arm64 | 10 | 0.088 / 0.101 | 0.101 / 0.141 |
| Debian 13 arm64 | 10,000 | 5.938 / 6.576 | 27.616 / 31.348 |

```sh
python3 tools/bench_completion.py target/release/tonari --bash /path/to/bash --listing
```

## Interactive shell milestone / 対話シェル統合版

Source tree `119e729ae01cef7d8d0883fdd92bb5fbd0bf447f`, measured on 2026-11-22. The harness is Git blob `1f7f36916222e276727e0cdd8802381c9e15fdf1`. Both arm64 release builds use Rust 1.96.1, native CPU tuning, `opt-level=s`, fat LTO, stripped symbols and one codegen unit. The Linux distribution build statically links musl 1.2.6; its host was Debian 13 userspace on a Linux 6.11.11 VM on Apple M1. The macOS run used the same M1 with macOS 27.2.1. All shells received `--noprofile --norc`, a temporary home, a fixed prompt, and used their normal interactive editor. Cases had five warm-ups and 300 measured samples per shell in rotating order, except nvm (100). Each time cell is **median / p95 in milliseconds**. Each ratio cell is **median / p95**, divided by the corresponding Bash statistic.

2026-11-22、上記の同一ソースを Apple M1 の macOS と Linux VM で計測しました。Rust 1.96.1、CPU ネイティブ最適化、`s`、fat LTO、strip、codegen unit 一つを使用し、Linux 版は musl 1.2.6 を静的リンクしています。どのシェルも設定ファイルを無効にし、一時ホーム・同一のプロンプト・通常の対話エディターで起動しました。ウォームアップ5回後、各条件・各シェルを順番を入れ替えながら300回、nvm は100回測定しました。時間は **中央値 / p95（ミリ秒）**、比は各々を同じ統計量の Bash で割った値です。

### Debian 13 arm64 · GNU Bash 5.2.37

| Metric / 項目 | Tonari | GNU Bash | Tonari/Bash |
|---|---:|---:|---:|
| First prompt / プロンプト | 0.685 / 1.034 | 1.108 / 1.492 | 0.618 / 0.693 |
| Exec to prompt / exec 以降 | 0.362 / 0.504 | 0.776 / 1.088 | 0.466 / 0.464 |
| `-c ':'` | 0.188 / 0.229 | 0.416 / 0.500 | 0.451 / 0.458 |
| One external / 外部1個 | 0.308 / 0.403 | 0.550 / 0.720 | 0.560 / 0.561 |
| Pipeline / パイプ | 0.563 / 0.704 | 0.917 / 1.099 | 0.614 / 0.641 |
| Boolean builtins / 真偽値 | 0.194 / 0.243 | 0.431 / 0.515 | 0.449 / 0.472 |
| 100 redirects / リダイレクト100回 | 0.450 / 0.522 | 0.894 / 1.024 | 0.503 / 0.510 |
| Session builtins / セッション | 0.220 / 0.285 | 0.447 / 0.562 | 0.493 / 0.507 |
| 16 external programs / 外部16個 | 3.314 / 3.707 | 4.820 / 5.441 | 0.688 / 0.681 |
| 1,000 loop iterations / ループ1,000回 | 0.981 / 1.090 | 2.295 / 2.555 | 0.427 / 0.427 |
| nvm 0.40.5 `--no-use` | 5.482 / 10.953 | 8.179 / 15.695 | 0.670 / 0.698 |

Direct `/usr/bin/true` measured 0.208 / 0.246 ms. Tonari was 987,496 bytes, Bash was 1,412,096. Two medians exceeded 2/3 of Bash elapsed time, the 16-external-command series at 0.688 and nvm at 0.670. The overall prompt met the median target; its p95 ratio was 0.693.

`/usr/bin/true` 直接起動は 0.208 / 0.246 ms。サイズは Tonari 987,496 バイト、Bash 1,412,096 バイト。中央値で 2/3 を超えたのは外部16個の 0.688 と nvm の 0.670 です。プロンプト表示は中央値が目標範囲で、p95 の比は 0.693 でした。

### macOS 27.2.1 arm64 · GNU Bash 5.2.37 and system Bash 3.2.57

| Metric / 項目 | Tonari | GNU Bash 5.2.37 | System Bash 3.2.57 | Tonari/5.2.37 | Tonari/3.2.57 |
|---|---:|---:|---:|---:|---:|
| First prompt / プロンプト | 4.572 / 5.217 | 7.588 / 8.403 | 5.409 / 6.110 | 0.603 / 0.621 | 0.845 / 0.854 |
| Exec to prompt / exec 以降 | 3.071 / 3.565 | 6.156 / 6.845 | 3.938 / 4.468 | 0.499 / 0.521 | 0.780 / 0.798 |
| `-c ':'` | 2.707 / 3.124 | 5.658 / 6.339 | 3.160 / 3.699 | 0.478 / 0.493 | 0.857 / 0.845 |
| One external / 外部1個 | 3.356 / 3.892 | 6.256 / 7.039 | 3.841 / 4.554 | 0.537 / 0.553 | 0.874 / 0.855 |
| Pipeline / パイプ | 3.872 / 5.596 | 7.719 / 9.175 | 5.166 / 7.047 | 0.502 / 0.610 | 0.750 / 0.794 |
| Boolean builtins / 真偽値 | 2.671 / 6.006 | 5.755 / 10.886 | 3.258 / 7.529 | 0.464 / 0.552 | 0.820 / 0.798 |
| 100 redirects / リダイレクト100回 | 3.968 / 6.639 | 7.560 / 11.709 | 4.869 / 7.432 | 0.525 / 0.567 | 0.815 / 0.893 |
| Session builtins / セッション | 2.561 / 2.985 | 5.467 / 6.313 | 3.022 / 3.708 | 0.468 / 0.473 | 0.847 / 0.805 |
| 16 external programs / 外部16個 | 21.085 / 32.224 | 34.903 / 51.203 | 32.256 / 51.388 | 0.604 / 0.629 | 0.654 / 0.627 |
| 1,000 loop iterations / ループ1,000回 | 3.503 / 5.261 | 7.213 / 10.757 | 5.408 / 8.037 | 0.486 / 0.489 | 0.648 / 0.655 |
| nvm 0.40.5 `--no-use` | 5.616 / 6.010 | 11.787 / 12.582 | 9.327 / 9.944 | 0.476 / 0.478 | 0.602 / 0.604 |

Direct `/usr/bin/true` measured 1.200 / 1.865 ms. Tonari was 770,096 bytes. Native GNU Bash 5.2.37 was 1,119,952 bytes. Apple’s system Bash was 1,961,872 bytes including three architecture slices, with 667,584 bytes in the arm64e slice. Every measured Tonari median fell within 2/3 of native GNU Bash 5.2.37. Against system Bash, the 16-external series, the 1,000-iteration loop and nvm met that threshold. Every other median was faster than system Bash and above the target.

`/usr/bin/true` 直接起動は 1.200 / 1.865 ms。サイズは Tonari 770,096 バイト、ネイティブの GNU Bash 5.2.37 が 1,119,952 バイト、OS 同梱 Bash は3アーキテクチャ合計 1,961,872 バイト（arm64e 部分 667,584 バイト）です。GNU Bash 5.2.37 比では全項目の中央値が 2/3 以下でした。OS 同梱版と比べて 2/3 以下になったのは外部16個・ループ1,000回・nvm で、それ以外の中央値も同梱版より速く、目標には届きませんでした。

### Starting 16 real child processes / 外部16プロセスの直接起動

A separate 300-sample run added a persistent native C parent. It timed 16 calls to `posix_spawn` on `/usr/bin/true`, each followed by a blocking `waitpid`, using `CLOCK_MONOTONIC` inside the parent. The C process inherited the same environment and configured `/dev/null` once before timing. It used the same musl libc as Tonari on Linux and libSystem on macOS. The Python harness rotated the four launch paths, adding system Bash as a fifth on macOS. The C parent’s own startup and communication with the harness were outside the timer. The shell cases include their own startup. These are empirical controls for this workload.

別計測では、常駐する C の親プロセスから `/usr/bin/true` を16回、`posix_spawn` と `waitpid` で一つずつ起動・待機しました。親の起動と計測器との通信は計時外で、シェル側にはシェル自身の起動も含まれます。Linux の C 版は Tonari と同じ musl を使用しました。各条件を交互に300回実行しています。

| Host / ホスト | Tonari | GNU Bash 5.2.37 | System Bash 3.2.57 | Direct C | Direct Python |
|---|---:|---:|---:|---:|---:|
| Debian 13 arm64 | 3.492 / 5.535 | 5.048 / 8.063 | — | 3.257 / 5.663 | 3.576 / 5.349 |
| macOS 27.2.1 | 20.905 / 28.666 | 34.625 / 48.050 | 32.015 / 44.899 | 17.972 / 24.555 | 18.116 / 25.018 |

For the nvm case, source `nvm.sh` v0.40.5 (`cd6f374433a22ec01919a834cf59d03e0ba07c226bed460a50ef5eecc38c39ef` SHA-256) with `--no-use` and an empty `NVM_DIR`. The shell checks that the `nvm` function was defined before returning success. The figures cover sourcing with warm filesystem caches. [nvm v0.40.5](https://github.com/nvm-sh/nvm/tree/v0.40.5)

nvm の項目は、空の `NVM_DIR` を用意して v0.40.5 の `nvm.sh` を `--no-use` で読み込み、最後に `nvm` 関数の定義を確認します。キャッシュが温まった状態での読み込み時間です。

```sh
python3 tools/bench.py target/release/tonari --bash /path/to/bash-5.2.37 --compare-shell System /bin/bash --runs 300
python3 tools/bench.py target/release/tonari --bash /path/to/bash-5.2.37 --compare-shell System /bin/bash --nvm-script /path/to/nvm.sh --case nvm --runs 100
```

## Parser allocation experiment / パーサーのメモリ確保

Starting from `119e729` on Debian 13 arm64, we changed the initial capacity of three parser vectors. A statement list starts with one slot, command words and expansion parts start with two. In the same rotating 250-sample run, sourcing nvm 0.40.5 improved by 12% at the median. Both static musl binaries were 987,496 bytes. The table shows **median / p95 in milliseconds**.

Debian 13 arm64 で `119e729` を基に、文リストの初期確保を1要素、コマンドの単語列と展開部分列を2要素に変えました。各250回の交互計測で nvm 読み込みの中央値が12%短くなりました。静的 musl バイナリはどちらも 987,496 バイトです。各欄は **中央値 / p95（ミリ秒）** です。

| Metric / 項目 | Tonari before / 変更前 | Tonari after / 変更後 | GNU Bash 5.2.37 |
|---|---:|---:|---:|
| Source nvm `--no-use` | 5.044 / 6.755 | 4.436 / 5.247 | 7.716 / 9.471 |
| 1,000 loop iterations / ループ1,000回 | 0.858 / 1.167 | 0.831 / 1.019 | 2.447 / 2.922 |
| `-c ':'` | 0.183 / 0.250 | 0.181 / 0.233 | 0.422 / 0.598 |

## Selecting release optimization on macOS / macOS の最適化レベル

The following Apple M1 run compares three builds of Git tree `dffc90bd24207a49ce4d148aa506f07128ff2c81`. Rust 1.96.1 used native CPU tuning, fat LTO, one codegen unit, symbol stripping and panic abort throughout. Each cell is a median in milliseconds. The basic cases used 200 measured iterations, the last two used 100. All five binaries ran in rotating order on macOS 27.2.1. GNU Bash 5.2.37 and system Bash 3.2.57 are distinct macOS references. Bash 5.2.37 used the bundled Readline and Apple Clang 21.0.0 with `CFLAGS='-O3 -mcpu=native -flto' LDFLAGS='-flto' ./configure --prefix=…`, `make -j4`, and `strip`. Its tarball SHA-256 is `9599b22ecd1d5787ad7d3b7bf0c59f312b3396d1e281175dd1f8a4014da621ff`. [GNU Bash release source](https://ftp.gnu.org/gnu/bash/bash-5.2.37.tar.gz)

Apple M1 の macOS 27.2.1 で同一ソースの最適化レベルを比較しました。Rust 1.96.1、CPU ネイティブ最適化、fat LTO、codegen unit 一つ、strip、panic abort は共通です。各欄はミリ秒の中央値です。通常項目は200回、末尾2項目は100回、5本の実行順を入れ替えて計測しました。上流ソースからビルドした Bash 5.2.37 と OS 同梱の Bash 3.2.57 はそれぞれ独立した比較対象です。

| Metric / 項目 | Tonari `3` | Tonari `s` | Tonari `z` | Bash 5.2.37 | System Bash 3.2.57 |
|---|---:|---:|---:|---:|---:|
| First prompt / プロンプト | 4.836 | 4.880 | 4.816 | 8.113 | 5.711 |
| `-c ':'` | 2.681 | 2.574 | 2.617 | 5.748 | 3.232 |
| One external / 外部1個 | 3.877 | 3.968 | 3.889 | 7.314 | 4.649 |
| Pipeline / パイプ | 4.256 | 4.278 | 4.240 | 8.370 | 5.750 |
| 100 redirects / リダイレクト100回 | 3.676 | 3.658 | 3.702 | 7.004 | 4.541 |
| 1,000 loop iterations / ループ1,000回 | 3.350 | 3.338 | 4.143 | 7.274 | 5.327 |
| 16 external programs / 外部16個 | 26.525 | 26.201 | 26.386 | 44.889 | 41.702 |
| Executable bytes / バイト数 | 769,504 | 687,088 | 605,952 | 1,119,952 | 1,961,872 |

The system executable includes three architectures. Its arm64e slice occupies 667,584 bytes. Level `s` reduced Tonari by 82,416 bytes compared with level 3 and kept the loop at 3.338 ms. Level `z` made the loop 24% slower than `s`. The release profile now uses `s`, which was already the static Linux arm64 distribution setting.

OS 同梱版には3種のアーキテクチャが含まれ、arm64e の部分は 667,584 バイトです。レベル `s` はレベル3より 82,416 バイト小さく、ループは 3.338 ms でした。`z` ではループが `s` より24%遅くなります。この結果を受け、Linux arm64 の静的配布版ですでに使っていた `s` を共通のリリース設定にしました。

## Executor baseline / 実行系の初期計測

The tables compare release builds on an Apple M1, with 5 warm-ups and 300 measured runs per case and executable. Each timing is **median / p95 in milliseconds**. The harness rotates the executables each round. The original Tonari is commit `2874dc8`, rebuilt with the same Rust version (1.96.1) and native CPU settings. Bash skips profile and rc files.

`-c` timing now starts at a native `posix_spawn` call and ends after `waitpid`. The direct `/usr/bin/true` measurement uses the same launcher. Prompt timing includes the Python parent creating a pseudo-terminal, forking and reading the first prompt. Use results from the same run to compare builds.

Apple M1 上でリリースビルドを使い、各項目・実行ファイルにつきウォームアップ5回の後、起動順を入れ替えながら300回計測しました。各欄はミリ秒で **中央値 / p95** を示します。旧版はコミット `2874dc8` を同じ Rust 1.96.1、同じ CPU 向け設定で再ビルドしたものです。Bash は profile と rc を無効にしています。

今回から `-c` は OS の `posix_spawn` 呼び出し直前から `waitpid` の完了までを測ります。`/usr/bin/true` の直接起動も同じ方法です。プロンプト表示には Python の親が疑似端末を作り、fork して最初のプロンプトを読み取る時間を含みます。変更前後は同じ回の測定値で比較してください。

### Debian 13 arm64

| Metric / 計測項目 | Tonari @ 2874dc8 | Tonari (glibc) | Tonari (static musl) | Bash 5.2.37 |
|---|---:|---:|---:|---:|
| First prompt / プロンプト表示 | 0.732 / 0.920 | 0.725 / 0.902 | 0.602 / 0.746 | 1.027 / 1.229 |
| `-c ':'` | 0.300 / 0.372 | 0.303 / 0.379 | 0.170 / 0.212 | 0.430 / 0.519 |
| `/usr/bin/true` | 0.412 / 0.499 | 0.408 / 0.503 | 0.286 / 0.358 | 0.547 / 0.660 |
| `/usr/bin/printf x \| /usr/bin/wc -c` | 0.688 / 0.831 | 0.679 / 0.840 | 0.509 / 0.628 | 0.878 / 1.074 |
| `true; false; true` | 0.863 / 1.024 | 0.300 / 0.376 | 0.170 / 0.214 | 0.439 / 0.550 |
| `: >/dev/null` × 100 | 0.643 / 0.756 | 0.473 / 0.571 | 0.664 / 0.778 | 0.927 / 1.082 |
| Size / バイト数 | 397,968 | 397,968 | 397,560 | 1,412,096 |

Direct `/usr/bin/true` / 同じランチャーからの直接起動 **0.219 / 0.262 ms**。

Both new builds use the same source. The GNU build uses the release profile and glibc. The static build uses Alpine's compiler, musl and size optimization. It reached the prompt sooner and finished short commands faster. The GNU build completed the 100 file redirections faster.

新版2本は同じソースです。通常版は GNU ツールチェーンと glibc、静的リンク版は Alpine のコンパイラと musl、サイズ最適化を使用しました。今回、静的リンク版はプロンプト表示と短いコマンドの完了が速く、ファイルへのリダイレクト100回は通常版の方が速い結果でした。

### macOS 27.2.1 arm64

| Metric / 計測項目 | Tonari @ 2874dc8 | Tonari | System Bash 3.2.57 |
|---|---:|---:|---:|
| First prompt / プロンプト表示 | 4.478 / 5.217 | 4.422 / 5.071 | 5.501 / 6.392 |
| `-c ':'` | 2.489 / 2.887 | 2.412 / 2.891 | 3.149 / 3.635 |
| `/usr/bin/true` | 3.591 / 4.375 | 3.575 / 4.300 | 4.445 / 5.098 |
| `/usr/bin/printf x \| /usr/bin/wc -c` | 4.089 / 4.843 | 4.107 / 4.864 | 5.781 / 6.479 |
| `true; false; true` | 6.221 / 7.313 | 2.379 / 2.894 | 3.111 / 3.610 |
| `: >/dev/null` × 100 | 3.772 / 4.384 | 3.587 / 4.268 | 4.734 / 5.743 |
| Size / バイト数 | 370,736 | 354,048 | 1,961,872 |

Direct `/usr/bin/true` / 同じランチャーからの直接起動 **1.172 / 1.493 ms**。

The shell now executes `true` and `false` internally. File redirections on builtins save fewer descriptors. The release profile uses `panic = "abort"`; the macOS executable is smaller. The shell also copies single-quoted arguments in one span, which reduces parsing time for long literal arguments.

`true` と `false` は組み込みで実行し、組み込みのファイルリダイレクトでは記述子の退避を減らしました。リリースビルドには `panic = "abort"` を使い、macOS の実行ファイルも小さくなりました。単一引用符の中身はまとめてコピーするため、長いリテラル引数の解析時間も短縮されています。

## Linux arm64 static PIE

The static build links musl into a position-independent executable. It runs on the Debian and Ubuntu arm64 test systems without a separate musl installation. The tested compiler is the native Alpine Rust 1.96.1 package (`aarch64-alpine-linux-musl`) with musl development files and a C linker installed. Build on the target CPU when using `target-cpu=native`.

静的リンク版は musl を実行ファイルに組み込んだ PIE（位置独立実行形式）です。検証した Debian と Ubuntu の arm64 環境では musl の追加インストールなしで動きます。下記は Alpine のネイティブ Rust パッケージ 1.96.1（`aarch64-alpine-linux-musl`）、musl の開発ファイル、C リンカーを使ったビルド手順です。`target-cpu=native` はビルドした CPU 向けの設定です。

```sh
CARGO_PROFILE_RELEASE_OPT_LEVEL=s \
RUSTFLAGS='-C target-cpu=native -C target-feature=+crt-static' \
cargo rustc --locked --release --target aarch64-alpine-linux-musl -- -C link-arg=-Wl,-z,pack-relative-relocs
```

## First prototype / 初回試作

First prototype, measured 2026-11-22. Each timing is **median / p95 in milliseconds**. The prompt test ends when the first prompt is visible on a pseudo-terminal. The other tests start a fresh shell for each command and wait for its exit.

初回試作を 2026-11-22 に計測しました。時間の単位はミリ秒で、各欄に **中央値 / p95** を示します。プロンプトの計測は、疑似端末に最初のプロンプトが出力されるまでです。ほかの計測では毎回新しいシェルを起動し、その終了までを測っています。

### Debian 13 arm64

Linux VM on Apple M1, Debian userspace, GNU Bash 5.2.37, brush 0.4.0. The executables ran on the VM's local filesystem.

Apple M1 上の Linux 仮想マシンで Debian のユーザーランドを使い、VM 内のファイルシステムに置いた実行ファイルで計測しました。

| Metric / 計測項目 | Tonari | GNU Bash | brush |
|---|---:|---:|---:|
| First prompt / プロンプト表示 | 0.895 / 1.381 | 1.148 / 1.444 | 1.761 / 2.362 |
| `-c ':'` | 0.335 / 0.416 | 0.451 / 0.559 | 1.149 / 1.398 |
| `-c '/usr/bin/true'` | 0.457 / 0.587 | 0.602 / 0.750 | 1.664 / 2.077 |
| `-c '/usr/bin/printf x \| /usr/bin/wc -c'` | 0.722 / 0.885 | 0.902 / 1.339 | 2.009 / 3.101 |
| Executable bytes / 実行ファイルのバイト数 | 397,968 | 1,412,096 | 6,381,288 |

### macOS 27.2.1 arm64

Apple M1, system Bash 3.2.57, brush 0.4.0. The macOS system Bash is older than the project's compatibility baseline. Its universal executable contains three architecture slices. The arm64e slice is 667,584 bytes. Tonari and brush are single-architecture arm64 builds.

Apple M1 で OS 同梱の Bash 3.2.57 と brush 0.4.0 を使用しました。macOS 同梱の Bash は互換性の基準にしている版より古く、実行ファイルには三つのアーキテクチャが含まれます。うち arm64e 部分は 667,584 バイトです。Tonari と brush は arm64 単体のビルドです。

| Metric / 計測項目 | Tonari | GNU Bash | brush |
|---|---:|---:|---:|
| First prompt / プロンプト表示 | 4.905 / 7.973 | 5.810 / 9.159 | 9.129 / 12.498 |
| `-c ':'` | 3.856 / 4.467 | 4.597 / 5.327 | 7.536 / 8.894 |
| `-c '/usr/bin/true'` | 5.183 / 6.447 | 5.975 / 6.877 | 9.960 / 11.260 |
| `-c '/usr/bin/printf x \| /usr/bin/wc -c'` | 5.609 / 6.594 | 7.144 / 8.168 | 10.915 / 13.067 |
| Executable bytes / 実行ファイルのバイト数 | 370,736 | 1,961,872 | 5,621,680 |

### Initial measurement conditions / 初回の計測条件

- Tonari was built with Rust 1.96.1, fat LTO, one codegen unit and symbol stripping. The reference shells were the OS-provided Bash and the upstream brush release binary. File sizes exclude shared libraries. [brush 0.4.0 binaries](https://github.com/reubeno/brush/releases/tag/brush-shell-v0.4.0)
- The harness uses five warm-ups and 100 measured launches per shell per case, rotating shell order. All shells receive the same minimal environment, temporary home, prompt and `PATH`. Bash and brush skip profile and rc files. brush also skips its own config. Interactive editors retain their defaults.
- Timings include the Python parent's process-launch overhead. First-prompt timing starts before the PTY fork. The harness answers cursor-position queries and stops timing when it reads the prompt. This run covers warm caches, an empty home and one arm64 machine. Real rc files, history, completion, sustained workloads and agent end-to-end latency need separate measurements.

Tonari は Rust 1.96.1 で CPU ネイティブ向けにビルドし、fat LTO、codegen unit 一つ、シンボル除去を使用しました。Bash は各 OS の配布物、brush は上流のリリースバイナリです。サイズは実行ファイル本体の値です。

計測項目ごとに各シェルを5回ウォームアップし、起動順を入れ替えながら各100回計測しました。最小限の環境変数、一時ホーム、プロンプト、`PATH` は共通です。Bash と brush は profile と rc の読み込みを無効にし、brush 固有の設定も無効にしました。対話エディタはそれぞれの既定値です。

時間には Python の親プロセスが起動要求を出すコストも含みます。プロンプト表示の計時は疑似端末の fork 直前に始まり、カーソル位置の問い合わせにはハーネスが応答し、プロンプトを受信した時点で終了します。今回は arm64 の1台で、キャッシュが温まりホームが空の条件です。実際の rc、履歴、補完、連続実行、エージェント全体の待ち時間は別途計測する対象です。

## Benchmark against GNU Bash / GNU Bash との比較

GNU Bash is the ongoing reference for speed and executable size. The initial three-shell run used the benchmark script at commit `33204d5`.

今後は速度と実行ファイルのサイズを GNU Bash と比較します。初回に3種類のシェルを計測したスクリプトはコミット `33204d5` にあります。

```sh
RUSTFLAGS='-C target-cpu=native' cargo build --release --locked
python3 tools/bench.py target/release/tonari --bash /bin/bash --runs 300
```

Use `--compare-tonari Original /path/to/previous/tonari` and, on Linux arm64, `--compare-tonari Static /path/to/static/tonari` to interleave the builds in the same run.

旧版は `--compare-tonari Original /path/to/previous/tonari`、Linux arm64 の静的リンク版は `--compare-tonari Static /path/to/static/tonari` で同じ計測に加えられます。

## Repeated PATH lookup / PATH 検索の繰り返し

On macOS 27.2.1 arm64 (Apple M1), the same Tonari release executable ran 40 commands named `helper` followed by `:`. The last directory on PATH contained a symlink named `helper` to `/usr/bin/true`. Every preceding directory was empty. The benchmark set PATH in the parent environment and ran `tonari --noprofile --norc -c` with either `set +h` or `set -h` at the start of the command string. Each cell is the median wall time for a whole session over 24 paired runs. Launch order alternated and the first pair was discarded. Rust 1.96.1, release profile, `-C target-cpu=native`.

| PATH directories / ディレクトリ数 | Cache disabled / `set +h` | Cache enabled / `set -h` |
| ---: | ---: | ---: |
| 1 | 49.503 ms | 50.392 ms |
| 17 | 94.143 ms | 49.873 ms |

macOS 27.2.1 arm64（Apple M1）で、同じリリース版を使い、`helper` を40回呼び出して最後に `:` を実行しました。親から渡す PATH の末尾に `/usr/bin/true` へのリンクを置き、それより前のディレクトリは空にしています。シェル起動から終了までを計り、キャッシュ有効・無効の実行順を毎回交互に入れ替えました。最初の1組を除いた24組の中央値です。1ディレクトリでは差は約0.9 ms、17ディレクトリではキャッシュにより約44.3 ms 短縮しました。
