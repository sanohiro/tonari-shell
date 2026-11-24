# Reverse-history latency and Bash command timing / 履歴検索と Bash の性能比較

The preceding release on the hosted M1 took longer than Bash to recall the oldest line in 100,000 records. The shipping implementation now meets the two-thirds target in 28 of the 30 native interactive comparisons below. All 30 input medians were lower than Bash. The p95 was lower in 30 of 30. A new CI check fails when the median input latency exceeds the same-host Bash. [Per-trial input, prompt and combined times](history-parity-results.tar.gz) · [Native history check](https://github.com/sanohiro/tonari-shell/actions/runs/45343946143)

以前の Mac では初回の Ctrl-R が Bash を上回っていました。修正後は3環境で計30条件を比較し、中央値は30条件すべてで Bash より速く、うち28条件で 2/3 以内になりました。各試行で新しい PTY を起動し、最初のプロンプトを受け取った直後に入力しています。一致した場合は実行した代入の値、一致なしの場合は Enter で別のコマンドが実行されていないことも確認しています。

## Input to result / 入力から検索結果まで

Each cell gives **Tonari p50/p95 · Bash p50/p95**, in milliseconds. Matches finish when the result marker appears on the PTY. A miss includes Enter and the next prompt. The 100,000-row mixed fixture has Japanese paths and partial query matches in every other command. The million-row fixture exceeds the 4 MiB window. Linux used native runners and Bash 5.2.21. The hosted M1 used Homebrew Bash 5.3.20. [Fixture and timing code](../../tools/bench_editor_input.py)

| Fixture / 条件 | macOS arm64 | Linux x86-64 | Linux arm64 |
| --- | --- | --- | --- |
| 100k, 20 rows back | 0.190/0.476 · 0.585/1.060 | 0.094/0.120 · 0.700/0.846 | 0.133/0.151 · 0.698/0.947 |
| 100k, 900 rows back | 0.183/0.329 · 0.621/1.183 | 0.093/0.097 · 0.659/0.799 | 0.131/0.143 · 0.692/0.744 |
| 100k, oldest Japanese match | 0.668/1.357 · 1.063/2.034 | 0.111/0.132 · 1.221/1.596 | 0.152/0.172 · 1.173/1.256 |
| 100k, oldest ASCII match | 0.839/1.178 · 1.068/2.080 | 0.227/0.297 · 1.170/1.569 | 0.317/0.384 · 1.178/1.266 |
| 100k, Japanese miss through Enter | 0.263/0.608 · 9.433/25.873 | 0.101/0.119 · 5.674/5.969 | 0.183/0.196 · 6.478/6.600 |
| 100k, ASCII miss through Enter | 0.633/0.871 · 20.573/25.529 | 0.552/0.629 · 8.724/8.909 | 0.737/0.755 · 10.993/11.102 |
| 1M, oldest Japanese match | 7.817/10.204 · 10.402/14.948 | 2.766/2.854 · 15.023/15.522 | 2.956/3.023 · 10.949/11.365 |
| 100k mixed, oldest Japanese match | 2.060/2.642 · 3.722/4.558 | 1.106/1.153 · 3.151/3.352 | 1.181/1.218 · 3.012/3.080 |
| 100k mixed, oldest ASCII match | 2.110/2.853 · 6.578/9.121 | 0.789/0.848 · 3.759/3.864 | 0.817/0.836 · 3.833/4.042 |
| 100k mixed, Japanese miss through Enter | 1.589/4.251 · 66.397/82.980 | 1.707/1.869 · 30.273/30.917 | 1.679/1.744 · 28.958/29.399 |

On the shipping build, the hosted M1's oldest Japanese match in 100k rows fell from **1.584/2.685 to 0.668/1.357 ms**. The bounded cache moves some work before the prompt. On the same trials, PTY launch to prompt took **6.410 ms** with Tonari, **5.248 ms** with the previous release and **31.768 ms** with Bash. The combined launch-through-search medians were **7.272, 6.954 and 32.901 ms**. The earlier gated build reached two-thirds of Bash in all 30 comparisons. On the shipping build the 100k oldest ASCII match and the 1M oldest Japanese match on macOS exceeded that target. [Shipping measurements](https://github.com/sanohiro/tonari-shell/actions/runs/45343946143) · [Earlier 30 comparisons](https://github.com/sanohiro/tonari-shell/actions/runs/45341203253)

履歴10万件の Mac では、先読みで最初のプロンプトが前版より 1.162 ms 遅くなりました。起動から最古の一致が表示されるまでの中央値は Tonari 7.272 ms、前版 6.954 ms、Bash 32.901 ms です。2/3 の目標に届かなかった2条件も表に含めました。一致なしの場合は旧版が直前のコマンドを実行してしまうため、正しい結果を返す新版と Bash を比較しました。

On a local M1, the median RSS at the first prompt with 0 / 100k / 1M history entries was **2.42 / 5.05 / 6.47 MiB** for Tonari, **2.45 / 2.45 / 2.45 MiB** for the preceding release and **6.30 / 14.78 / 107.22 MiB** for GNU Bash 5.2.37. These are nine rotated samples of the foreground process read with macOS `ps`. 100万件の履歴でも起動時に保持する生データは最大 4 MiB です。 [Local RSS measurements and script](history-parity-results.tar.gz)

## Command timing / コマンドの実行時間

The original workloads keep their arguments, input, fixture and iteration count. Bash, the previous release and the shipping executable ran in alternating order on the same host. The first two rows below are disjoint. The later rows revisit some of the same feature names and use separate runners. Each timed batch first reproduced Bash stdout, stderr and exit status per iteration. [Feature-level ledger](history-parity.tsv) · [Raw samples and ledger builder](history-parity-results.tar.gz)

| Cohort / 比較対象 | Rows | ≤ ⅔ Bash | ≤ Bash | > Bash |
| --- | --- | --- | --- | --- |
| Previous 852 target misses | 852 | 233 | 835 | 17 |
| 65 additions from the preceding audit | 65 | 31 | 64 | 1 |
| Additional target misses selected by this release inventory | 70 | 20 | 70 | 0 |
| Batches slower than Bash at the preceding implementation stage, 31 samples | 19 | 0 | 13 | 6 |
| Batches above Bash in the shipping frozen and new-inventory samples, 31 fresh samples | 17 | 0 | 16 | 1 |
| 12 feature names slower than Bash at the original release, 31 samples | 12 | 0 | 7 | 5 |

The two-thirds column uses the original batch length. It makes no allowance for short operations or external waits. A higher observed median is counted even when two values differ by less than a microsecond. A feature can share its command with other rows, for example the two `wait -n` entries. 917行は機能名と環境の組み合わせです。閾値を上回った原計測も残し、再計測は別の列・行として示しています。 [Inputs](../../tests/compat/budget-2026-11-24.jsonl)

The 31-sample native repeat brought 16 of the 17 frozen rows above Bash back to parity or faster. The remaining row was `sleep .03 & jobs -n; wait` repeated four times. Tonari took **477.516 ms**, Bash **473.841 ms** and the direct C spawn-and-wait control **497.398 ms**. The separate 31-sample repeat of the eighteenth historical row, `${v:-a}`, was also faster with Tonari. The FIFO comparison below shows the latency when the shell controls when the child exits. [Native repeat](https://github.com/sanohiro/tonari-shell/actions/runs/45347565978) · [Parameter replay](https://github.com/sanohiro/tonari-shell/actions/runs/45343946230)

Among the same 917 historical rows, 899 were at or below Bash. All 18 higher medians were on the hosted M1. On a second native M1 the same 18 rows ran for 31 samples. Seventeen were at or below Bash. The remaining `disown` batch took 67.901 ms versus 67.116 ms. One noisy script, `${v:-a}`, creates three redirected subshells per iteration. It took 305.334 ms versus 259.143 ms in the original batch. An independent hosted trial measured 141.046 ms versus 179.882 ms, and the isolated expansion body took 5.025 ms versus 7.740 ms. The ledger keeps the original medians. [Additional native component measurements](https://github.com/sanohiro/tonari-shell/actions/runs/45343946230)

手元の M1 では元の単発・反復監査で Bash より遅かった42行も31回ずつ再確認しました。39行で中央値が Bash 以下になり、3行では高くなりました。内容は元の `$BASH` 起動、sleep を伴う `disown -r` と `jobs -l` です。Ubuntu arm64 の単発では `time :` と出力を読む `cat` の組み合わせが 2.017 対 1.998 ms で、128反復では 246.296 対 275.283 ms でした。1秒の PTY タイムアウトは 1021.749 対 1021.469 ms でした。いずれも左が Tonari です。 [All observations](history-parity-results.tar.gz)

### Previous slow commands on the hosted M1 / 前回遅かったコマンド

| Command / コマンド | Batch iterations | Tonari batch ms | Bash batch ms | Tonari single ms | Bash single ms |
| --- | --- | --- | --- | --- | --- |
| `disown` | 4 | 171.869 | 199.598 | 129.538 | 168.775 |
| `disown -r` | 4 | 187.404 | 178.717 | 100.806 | 89.634 |
| `jobs -l` | 4 | 530.588 | 542.318 | 117.406 | 128.226 |
| `jobs -n` | 4 | 480.987 | 467.940 | 81.575 | 93.287 |
| `jobs -r` | 4 | 467.856 | 487.494 | 125.144 | 150.601 |
| `jobs -x` | 4 | 475.861 | 515.572 | 122.790 | 136.517 |
| `mapfile -O` | 128 | 24.894 | 26.517 | 1.224 | 1.392 |
| `wait -n` | 4 | 437.491 | 436.083 | 97.839 | 79.279 |
| `-c` | 128 | 25.109 | 25.264 | 0.795 | 0.877 |
| `script fallback without shebang` | 16 | 60.840 | 78.578 | 3.615 | 4.268 |
| `BASH` | 16 | 47.136 | 42.716 | 3.442 | 2.998 |

The historical `BASH` witness starts its parent with argv[0] set to `bash` and a system-only PATH. On macOS the GNU Bash 5.2.37 parent then starts Apple Bash 3.2.57. With the absolute path supplied to the parent and the child’s file identity and version verified, 16 invocations took **49.939 ms with Tonari, 52.720 ms with the previous release and 124.935 ms with GNU Bash 5.2.37**. The original witness remains in the table for comparison. [Pinned self-invocation](https://github.com/sanohiro/tonari-shell/actions/runs/45343946143)

シバンのない実行ファイルは、ループ内と最終コマンドで実行経路が異なります。最初の修正はループ内で効果がありましたが、最終コマンドではシェル本体を再ロードしていました。新しいシェル状態に同じプロセス内で入り直せる場合も扱うと、手元の M1 で単発の中央値が **5.121 → 2.298 ms** になりました。同じときの Bash は **3.581 ms** です。入れ子のスクリプトでは必要に応じて新しい実行イメージに移り、インタープリターのスタックが増え続けることを防ぎます。 [Single-command comparison](history-parity-results.tar.gz)

For a job-control comparison without a timer, the child blocks on a FIFO. Each of 32 iterations either lists a live job and waits for its verified status, or disowns a live child and reads its acknowledgment. Results are **Tonari p50/p95 · Bash p50/p95** in ms, 41 samples. The separate original workloads retain their short sleeps, external utilities and the scheduler time they incur. [Driver](../../tests/compat/job_latency.py)

| 32 iterations | macOS arm64 | Linux x86-64 | Linux arm64 |
| --- | --- | --- | --- |
| list-running-and-wait | 39.023/106.739 · 64.658/113.914 | 8.841/9.114 · 16.595/17.856 | 14.968/15.576 · 31.340/32.270 |
| disown-running | 31.065/96.817 · 39.187/88.208 | 7.207/7.533 · 13.238/13.629 | 12.273/12.708 · 24.037/25.139 |

### Script throughput and validation / スクリプトと互換性

| Native host | Scripts | ≤ ⅔ Bash | ≤ Bash | Stripped bytes | Change, bytes |
| --- | --- | --- | --- | --- | --- |
| macOS arm64 | 63 | 53 | 63 | 1,333,984 | +16,704 |
| Linux x86-64 | 63 | 55 | 63 | 1,837,048 | +33,728 |
| Linux arm64 | 63 | 54 | 63 | 1,582,640 | +28,360 |

The 63 already-started scripts include repeated expansions, function redefinition, globbing, process startup, pipelines and redirection. These counts use published medians at 0.001 ms resolution. Linux shipped one static musl PIE and macOS one arm64 executable linked to libSystem. No crate or shared dependency was added. [Full native workflow](https://github.com/sanohiro/tonari-shell/actions/runs/45343946143)

The full feature inventory measures eligible commands both from launch and after the shell is ready. The command intervals use 15 samples. The following cells show counts at or below the Bash median out of all intervals with comparable timings. The separately selected batch replays above can run on different hosts. The individual timings and the existing compatibility differences are in the archive.

| Lane | Inventory rows | Whole ≤ Bash | Started ≤ Bash | Repeated ≤ Bash |
| --- | --- | --- | --- | --- |
| macOSarm64 | 1408 | 831/844 | 739/759 | 520/529 |
| Debianarm64 | 1522 | 864/864 | 762/762 | 561/561 |
| Ubuntuarm64 | 1553 | 844/845 | 742/745 | 576/576 |
| Debianx64 | 1522 | 862/862 | 760/760 | 551/551 |
| Ubuntux64 | 1553 | 843/843 | 743/743 | 568/568 |

Compared with the preceding release, **1 of the 7,558 compatibility verdicts changed**. Locally, 55 Rust tests passed. The integration runner exercised 159 cases on macOS. It reported 157 passes and two Linux-only skips. 機能別監査には従来からの未実装・仕様差も含まれます。 [Compatibility audit](../compatibility.md) · [Recorded trials](history-parity-results.tar.gz)
The Ubuntu arm64 GNU `run-trap` reference emitted an extra CHLD line in the full run. Three independent retries with the same release all reproduced the previous recorded difference. Bash passed its golden output. Tonari still differed at the DEBUG trap. All four observations are retained. Ubuntu arm64 の再実行3回で従来の DEBUG トラップの差を確認しました。 [GNU trap retries](https://github.com/sanohiro/tonari-shell/actions/runs/45346962206)

### Reproduce / 再現

Run the native workflow with the `parity-history`, `parity-mixed`, `parity-commands` or `budget` suites and `previous=1d05246e`. The history gate retains each measured PTY sample. To regenerate the TSV from the published native observations, extract the archive to an empty directory and run the following from the repository root. [Workflow](../../.github/workflows/native-benchmarks.yml)

```sh
work=$(mktemp -d)
tar -xzf docs/native-2026-11-24/history-parity-results.tar.gz -C "$work"
python3 "$work/build_ledger.py" "$work/ledger.tsv"
cmp docs/native-2026-11-24/history-parity.tsv "$work/ledger.tsv"
```
