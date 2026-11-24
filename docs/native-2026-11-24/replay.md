# Replaying the Bash performance misses / Bash 比で未達だった機能の再検証

The fixed set consists of 852 platform/feature observations, 255 distinct commands and 764 platform/workload pairs. A single command can exercise several Bash names. The same 852 observations were tested at four successive source revisions. At the shipping release 214 met the two-thirds target at the original iteration count and 38 more when repeated 1,024 times. The two allowances account for 128 observations. The other 472 were tried and remain over budget. 新しく閾値を超えた 65 行にも同じ比較をかけました。各行に試した版、コンパイラ設定、絶対時間、判定を記録しています。 [Feature ledger / 機能別 TSV](replay.tsv) · [Raw samples, compatibility, script timings and threshold crossings](replay-results.tar.gz)

| Cohort / 過去・追加 | Same batch ≤ 2/3 | Longer batch ≤ 2/3 | Short and at parity | Matched OS control | Tried, above target | Total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Previous misses / 前回の未達 | 214 | 38 | 3 | 125 | 472 | 852 |
| New misses / 今回追加 | 29 | 8 | 1 | 0 | 27 | 65 |

Each input kept the original arguments, standard input, fixture and repeat count (4, 16 or 128). On native macOS arm64, Debian 13 x86-64 and arm64, and Ubuntu 26.04 x86-64 and arm64, we interleaved Bash, the previous release and the three candidate builds. The ready-shell timer covers the repeated command and process exit. Each candidate first reproduced the original stdout, stderr and exit status of every iteration. The fixed pass used one warmup and nine samples. Historical 128-iteration batches taking at most 4 ms in both shells also ran for 1,024 iterations with five samples. Nine and five samples give a coarse p95. [Workloads](../../tests/compat/budget-2026-11-24.jsonl) · [Driver](../../tests/compat/replay_budget.py)

The target uses exact median times. For the short-operation allowance, both shells must take at most 5 µs per iteration in the longer batch and Tonari must be at least as fast as Bash. For a wait or OS operation, its C control must itself exceed two-thirds of Bash time and measure within 10% of Tonari. Tonari must also finish within 110% of Bash time in the primary and qualifying control runs. Some files and parameters are tested inside a command that creates files or launches subshells. The ledger preserves separate timings of the inner shell operation where the observable output could be kept identical. 短時間の例外には反復実行で Bash と同等以上、OS 処理の例外には同じ操作を直接実行した実測を要求しました。5 µs と 10% は今回の判定用に選んだ境界です。 [Classification](../../tests/compat/replay_report.py) · [C controls](../../tests/compat/budget_floor.c)

## Changes tested / 試した実装

| Runtime | Changes exercised |
| --- | --- |
| [ab0e93d](https://github.com/sanohiro/tonari-shell/commit/ab0e93d) | Reuse parsed commands and loop redirections, reduce UTF-8 directory traversal work, retain the reverse-history scan buffer. |
| [c6e8974](https://github.com/sanohiro/tonari-shell/commit/c6e8974) | Transfer quoted scalar fields directly, reduce descriptor moves and directory prefix work, simplify common printf, read and getopts operations. |
| [216d776](https://github.com/sanohiro/tonari-shell/commit/216d776) | Spawn Tonari directly after ENOEXEC for executable text, retaining arguments, environment, redirection and status. |
| [81a439d](https://github.com/sanohiro/tonari-shell/commit/81a439d) | Send small here-inputs to external commands using spawn actions, bound read-ahead by the first possible delimiter and defer command path lookup. |

O2, O3 and size optimization ran for every frozen workload at each stage. At the final revision, 214 of the 852 fixed batches met the target on O2 and 210 each on O3 and Os. Relative to O2, O3 brought nine more rows within target and moved 13 out. Os brought 55 in and moved 59 out. The release keeps O2. Each feature row gives its round-one through round-three elapsed time divided by that round's Bash time, plus the final paired times and alternative compiler results. New inventory misses entered the final comparison. 複数の機能が同じ実装経路を使うため、候補実装は共有しています。各機能を単独で専用実装に書き換えた件数とは区別して集計しました。

For 223 of the original observations, we could isolate the shell operation from setup and preserve its output. The isolated body met the target in 149. All 223 retained their end-to-end classification. 元コマンドからファイル作成やサブシェルを分離できた223行では、本体だけなら149行が目標を満たしました。利用者が実行する全体の達成件数は上表の通りです。

Of the 472 unresolved historical observations, 464 finished faster than Bash on the original batch. Eight took longer, all on the hosted M1. These were `disown`, `jobs -l/-n/-x`, `mapfile -O`, the `[[ -c ... ]]` probe, running executable text without a shebang, and the `BASH` variable probe. For `mapfile -O` the hosted median was 28.632 ms versus Bash at 22.140 ms. A separate 61-sample local M1 trial gave 9.160 ms versus 11.108 ms. Moving fixture creation outside the timer gave 3.262 ms versus 4.242 ms. This workload remains over the two-thirds target in all three comparisons. [Original and component samples](replay-results.tar.gz)

## Interactive latency and script throughput / 対話操作とスクリプト

Each cell gives **Tonari p50/p95 · Bash p50/p95** in milliseconds. The PTY measures include input to visible output. The script driver starts timing after the shell is ready. Its references were the runner Bash 5.2.21 on Linux and Homebrew Bash on the hosted M1. The feature replay used distribution Bash or GNU Bash 5.2.37 built from source on macOS. [PTY driver](../../tools/bench_editing.py) · [Script driver](../../tools/bench_scripts.py)

| Workload, p50/p95 ms | Linux x86-64 | Linux arm64 | macOS arm64 |
| --- | ---: | ---: | ---: |
| Ctrl-R, 100,000 history entries | 0.367/0.398 · 0.918/1.495 | 0.557/0.582 · 1.200/1.301 | 1.718/2.732 · 1.013/1.734 |
| Tab, unique UTF-8 completion in 10,000 entries | 2.239/2.262 · 2.508/2.772 | 1.298/1.361 · 1.512/1.541 | 4.067/10.703 · 7.678/11.760 |
| Key echo after Tab, 100,000 entries | 0.082/0.100 · 44.832/46.381 | 0.094/0.129 · 56.130/57.470 | 0.240/1.462 · 726.733/1239.028 |
| Display all 10,000 completion candidates | 5.754/5.987 · 24.976/25.649 | 6.449/6.510 · 32.620/32.991 | 13.682/20.438 · 66.380/126.106 |
| Paste 16 KiB, packet to visible marker | 2.394/2.540 · 29.543/30.608 | 1.501/1.599 · 18.622/19.093 | 2.550/3.777 · 19.069/25.172 |
| Exec to first prompt | 0.495/0.535 · 1.046/1.115 | 0.847/0.911 · 1.538/1.643 | 2.177/2.806 · 5.964/7.850 |
| Start and exit, `-c :` | 0.167/0.202 · 0.563/0.636 | 0.389/0.447 · 0.928/0.986 | 2.158/3.130 · 6.343/8.655 |
| Parse 2,000 different multiline function bodies | 3.623/3.673 · 5.946/6.157 | 3.608/3.664 · 7.870/7.930 | 4.447/8.334 · 9.123/13.365 |
| Replace one literal function 2,000 times | 0.557/0.579 · 2.122/2.297 | 0.628/0.665 · 2.900/2.950 | 0.935/2.195 · 3.539/7.345 |
| 4,096 UTF-8 names, one expansion | 1.871/1.915 · 2.894/2.977 | 1.639/1.685 · 3.580/3.617 | 7.198/10.347 · 18.694/21.890 |
| Read lines into an array | 2.038/2.123 · 3.133/3.172 | 0.970/1.014 · 4.264/4.324 | 1.295/1.744 · 4.095/5.565 |
| 2,000 integer increments by assignment | 0.659/0.680 · 2.409/2.456 | 0.644/0.688 · 3.289/3.377 | 0.933/1.166 · 3.650/4.039 |
| List all resource limits 128 times | 0.826/0.837 · 1.295/1.317 | 0.837/0.869 · 2.060/2.112 | 0.848/1.191 · 2.302/3.200 |
| Read 2,000 lines in a shell loop | 4.013/4.049 · 6.802/6.921 | 4.733/4.777 · 9.861/9.926 | 5.785/10.412 · 11.623/20.911 |
| 16 external two-stage pipelines | 6.751/7.034 · 9.136/9.634 | 12.307/12.742 · 15.107/15.866 | 24.324/41.350 · 31.614/44.822 |

| 61 script workloads per host | Median ≤ 2/3 | Above 2/3 with matching C control | Other above-target script workloads |
| --- | ---: | ---: | --- |
| Linux x86-64 | 53 | 8 | — |
| Linux arm64 | 52 | 8 | builtin-output |
| macOS arm64 | 49 | 8 | background-subshell-wait, directory-stack, descriptor-moves, builtin-output |

The stripped distribution executables in the throughput jobs were 1,317,280 bytes on macOS arm64, 1,803,320 on Linux x86-64 and 1,554,280 on Linux arm64. The respective changes from the paired previous builds were +16,512, +12,352 and +11,112 bytes. Linux passed the static-PIE check. The macOS arm64 distribution links only to libSystem. On the local M1, the distribution build was 1,300,800 bytes. 公開用ビルドでは依存ライブラリを追加していません。

The script counts use the displayed medians (0.001 ms resolution). The raw logs include the additional 4,096-output loop and the candidate history-window and spawn trials. スクリプト、PTY 操作、機能別の反復実行は負荷と計測区間が異なるため、表ごとに件数と絶対時間を示しました。

## Validation and reproduction / 検証と再現

The compatibility inventory covered 7,558 observations. Compared with the previous release, 0 changed verdict. The integration suite ran 155 tests, including here-input order, UTF-8 and read boundaries. Two Linux-only tests are skipped on macOS. The macOS signal test now holds its selected child on a FIFO while the test delivers SIGUSR1. The first run returned the child status before the expected trap was observed. The original witness relied on two fixed sleep durations. The complete Mac inventory was rerun. A Debian arm64 additional replay resolved shared Bash witnesses through the same lookup as the inventory. Both first attempts and the replacement measurements are preserved. [Full native run](https://github.com/sanohiro/tonari-shell/actions/runs/45328878562) · [Mac inventory](https://github.com/sanohiro/tonari-shell/actions/runs/45332941218) · [Debian arm64 additional replay](https://github.com/sanohiro/tonari-shell/actions/runs/45332271847)

The archive includes the complete ledger with medians and p95 for every measured build, the full inventory JSON, host and binary fingerprints, the direct controls, and local M1 experiments. Each native job can be reproduced using the workflow's runner and suite with `previous=9040ec3`. To replay one input, pass the ledger's witness ID to `tests/compat/replay_budget.py --only` along with the lane, Bash path and staged binaries directory. [Workflow](../../.github/workflows/native-benchmarks.yml) · [Native build and verification](../../tools/ci/native-build.sh)

To regenerate the feature ledger from the published samples, run these commands from the repository root.

```sh
work=$(mktemp -d)
tar -xzf docs/native-2026-11-24/replay-results.tar.gz -C "$work"
cp -R "$work/45328878562" "$work/verified"
cp -R "$work/45332941218/macos-15-inventory/." "$work/verified/macos-15-inventory/"
python3 tests/compat/replay_report.py "$work/verified" "$work/rebuilt.tsv" \
    --components "$work/45329605058" --recovery "$work/45332271847"
cmp "$work/ledger.tsv" "$work/rebuilt.tsv"
```
