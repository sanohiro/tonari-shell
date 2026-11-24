# Native benchmark results / ネイティブ実測

The x86-64 jobs used native Intel Xeon and AMD EPYC hosts. Linux arm64 used Neoverse-N2. The macOS arm64 machines reported Apple M1 (Virtual). GitHub assigns the machine per job. The paired release and algorithm comparisons rotate binaries on the same runner and fixture. Cross-job numbers can include a different CPU, kernel, filesystem or VM load. [Host for every job](hosts.tsv) · [Feature matrix](matrix.md) · [Raw measurements and machine details](native-results.tar.gz)

各ジョブで CPU 名、OS、コンパイラ、実行ファイルのサイズとハッシュを記録しました。A/B は同じ機械・同じファイル群で順番を入れ替えながら比較しています。x86-64 の CPU はジョブごとに異なります。PTY の入力遅延はシェルが返した表示を観測するまでの時間です。IME の変換 UI と端末エミュレーターの描画時間は測定範囲外です。

## Runtime decisions / 実装の判断

| Decision | Native evidence | 採用理由 |
| --- | --- | --- |
| Share abbreviated-path completion workers across CPUs | Twelve independent directories, 51 rotating samples. Intel Xeon 6975P-C took 1.839 ms shared vs 3.496 ms serial. AMD EPYC 9V74 took 2.582 vs 6.349 ms. Arm Neoverse-N2 took 1.537 vs 4.415 ms. With one CPU both executed serially. | ディレクトリ数・規模・使える CPU 数で決めます。単一のキー入力にスレッドを起動する必要はありません。 |
| Read 2 KiB first, then 32 KiB through the Linux directory API | With 100,000 files, Tab immediately followed by a key improved from 0.174 to 0.070 ms on Xeon and from 0.246 to 0.093 ms on Neoverse. Replacing the reader with POSIX `readdir` increased the 12-directory full scan from 3.493 to 3.799 ms on EPYC 7763 and from 1.625 to 1.775 ms on Neoverse. | 最初の読み出しを小さくし、継続する場合は一括で読みます。数値には全件走査と入力中断の両方を収録しました。 |
| Request up to 128 KiB from the Linux pipe API | Xeon, 31 samples, 256 MiB between external `dd` processes. Median/p95 was 97.259/106.877 ms with the 64 KiB kernel default, 64.330/65.295 at 128 KiB and 62.243/63.569 at 256 KiB. For 32 one-byte pipes, medians were 13.693, 13.658 and 13.807 ms. | 128 KiB で大きな転送を短縮し、短いパイプでの差は小さくなりました。カーネルが容量を制限している場合はその値に従います。 |
| Keep one POSIX spawn path for external commands with standard descriptor copies | Both supported systems supply POSIX spawn file actions. The execution, script and pipe suites exercise ordered copies, merged stderr, status and early close. | 同じ API を呼ぶ箇所から macOS・Linux の二重の条件を取り除きました。 |
| Use one release optimization level, O2 | On Xeon the 1,000-iteration loop took 0.672 ms at O2, 0.856 at Os, 1.289 at Oz and 0.667 at O3. O2 used 1,777,720 bytes in this controlled experiment, compared with 1,241,304 for the previous size-tuned build and 1,845,208 at O3. Across 51 already-started script workloads the median of the O3/O2 ratios was 1.003 on Xeon and 1.006 on Neoverse. | 速度を優先して、x86 だけのサイズ優先設定とフレームポインター強制を外しました。 |
| Keep the musl allocator adapter | Switching to the system allocator increased current-shell field splitting time by a factor of 1.82 on Xeon and 2.29 on Neoverse. On the hosted M1, enabling the hybrid allocator increased field splitting from 2.457 to 2.678 ms and added 66,224 bytes. The system allocator shortened Neoverse function calls (11.777 → 9.198 ms). | 同じ musl 配布物には CPU を問わず同じ方針を使います。起動時・変数解放後の RSS も併記しました。 |

### Directory workers under different CPU budgets / CPU 数と補完

The test completes one match across 12 independent directories. The default uses at most four workers including the calling thread, bounded by the available CPUs. Entries show median/p95 milliseconds over 51 rotating samples. The two Mac repeats started independently with the same runner label and workload. [CPU limits and repeats](https://github.com/sanohiro/tonari-shell/actions/runs/45295673794) · [First paired runs](https://github.com/sanohiro/tonari-shell/actions/runs/45289569544)

| Native host | Available CPUs | Shared default | Serial | Limit 2 | Limit 3 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Xeon 6975P-C | 1 | 3.530/3.730 | 3.565/3.812 | 3.514/3.668 | 3.560/3.749 |
| Xeon 6975P-C | 2 | 2.329/2.375 | 3.407/3.445 | 2.339/2.379 | 2.341/2.395 |
| Xeon 6975P-C | 3 | 2.089/2.131 | 3.456/3.507 | 2.665/2.712 | 2.091/2.129 |
| Xeon 6975P-C | 4 | 1.800/1.835 | 3.450/3.487 | 2.354/2.406 | 2.076/2.114 |
| Neoverse-N2 | 2 | 2.668/2.730 | 4.477/4.534 | 2.682/2.739 | 2.672/2.730 |
| Neoverse-N2 | 3 | 1.974/2.025 | 4.476/4.523 | 2.661/2.704 | 1.990/2.055 |
| Neoverse-N2 | 4 | 1.659/2.051 | 4.501/4.564 | 2.693/2.752 | 2.016/2.071 |
| M1 local | 8 | 4.623/4.753 | 8.838/9.091 | 6.548/6.801 | 5.268/5.329 |
| M1 hosted, first | 3 | 19.672/51.400 | 10.862/18.124 | — | — |
| M1 hosted, repeat 1 | 3 | 21.365/51.303 | 14.660/24.221 | 16.128/24.221 | 21.200/59.206 |
| M1 hosted, repeat 2 | 3 | 7.214/7.656 | 9.449/10.089 | 8.941/9.173 | 7.183/8.110 |

The same three-vCPU M1 runner label produced both rankings. Serial won two jobs and the shared algorithm won the third. On the local eight-core M1, the shared median was 4.623 ms versus 8.838 ms serial. Linux gained from parallelism with two and three available CPUs. These measurements support a common CPU-budget policy. The largest hosted Mac p95 for the shared path was 51.4 ms.

同じ3 vCPUの Mac CI でも、直列が速い回と並列が速い回の両方がありました。手元の M1 と Linux の2・3 CPUでは並列が速いため、OS 名や3コアという条件で直列に切り替える分岐は追加していません。走査の重い Mac VM で入力後の待ちが伸びる例も表に含めました。

### Pipes and filename expansion / パイプとファイル名展開

On the hosted M1, pre-expanding every pipeline connection increased the median for 32 one-byte pipes from 157.482 to 159.787 ms. For the 256 MiB `dd` pair it reduced 108.784 to 107.765 ms. The Darwin release uses the kernel's automatic pipe growth. Linux keeps its single capacity adapter behind `fcntl`. Both share pipeline construction, descriptor handling and exit status propagation. [Endpoint data](https://github.com/sanohiro/tonari-shell/actions/runs/45287830778)

On local M1 APFS, the median read of 512 names took 0.197 ms with POSIX `readdir` and 0.262 with `getattrlistbulk`. At 10,000 names it took 3.435 and 5.439 ms, respectively, over 61 alternating samples per method. On the same 8-core Apple M1 running macOS 27.2.1, 100,000 names with the same UTF-8 character locale took median/p95 144.309/147.907 ms with en_US.UTF-8 collation and 56.100/57.402 ms with C collation (11 samples). The first argument changed from `雪-000002.txt` to `B-000001.txt`. On the hosted M1, keeping named collation and comparing predecoded scalars took 347.529 ms versus 817.043 ms using pairwise `strcoll` in the 100,000-file plain glob. [Hosted collation A/B](https://github.com/sanohiro/tonari-shell/actions/runs/45286800106) · [Local commands and data](native-results.tar.gz)

macOS のファイル名照合順を選ぶと結果の引数順も変わります。既存の並び順を維持する範囲でも、比較のたびに文字列を変換する処理をまとめれば、同じ結果をより速く得られました。

The release profile lives in Cargo.toml. Plain Cargo, the host-toolchain build and the distribution builder all select O2. After moving the setting into the manifest at `2bb1d33`, all three native CI runners produced byte-for-byte identical distribution executables to the ones measured at `dfd9b77`, and all three startup-file suites passed. [Rebuilt releases](https://github.com/sanohiro/tonari-shell/actions/runs/45298043804)

O2・fat LTO・strip の既定値を Cargo.toml にまとめました。配布バイナリは3ターゲットとも、再ビルド前後の SHA-256 が一致しました。

The historical A/B builds use source `3999b0e` with the same compiler as the candidate. The release comparison for the previous x86 profile also retains that revision’s original build flags. The first-read change is in `dfd9b77`. [Directory workers](https://github.com/sanohiro/tonari-shell/actions/runs/45289569544) · [First reads](https://github.com/sanohiro/tonari-shell/actions/runs/45289001999) · [Directory traversal](https://github.com/sanohiro/tonari-shell/actions/runs/45288830220) · [Pipe endpoints](https://github.com/sanohiro/tonari-shell/actions/runs/45287830778) · [Allocator and code generation](https://github.com/sanohiro/tonari-shell/actions/runs/45286800106)

## Compiler and allocator experiments / コンパイラとメモリアロケータ

Forcing LLVM outlining on Linux x86 saved 1,120 bytes and increased the arithmetic median by 13%. On Linux arm64 it saved 47,168 bytes, with a median script ratio of 1.009 across 51 cases. On macOS we repeated the inverse experiment on a hosted M1 and a local M1. Restoring LLVM's default increased exec-to-prompt from 4.368 to 4.915 ms on the hosted runner and from 2.535 to 2.647 ms locally. It added about 16 KiB. It also improved local arithmetic from 5.401 to 4.844 ms. Local repeated near-head insertion took 0.035 ms with both configurations. The macOS release retains its build-time outlining setting for first-prompt latency. [Linux controls](https://github.com/sanohiro/tonari-shell/actions/runs/45290125202) · [macOS comparisons](https://github.com/sanohiro/tonari-shell/actions/runs/45286800106)

Linux x86-64 のフレームポインター強制とサイズ優先の標準ライブラリ設定は外しました。macOS のコード共通化に関する LLVM 設定は、手元と CI の両方でプロンプトの表示が速く、配布物も小さくなったため維持しています。シェルの入力処理や実行系のソースを分ける変更は要りません。

Profile-guided optimization trained the same O2 source with history, arithmetic, function calls, strings, globs, pipelines and substitutions. On Intel Xeon it reduced the binary from 1,777,720 to 1,585,240 bytes and arithmetic from 5.241 to 3.406 ms, while field splitting went from 2.737 to 2.999 ms. On Neoverse-N2 it reduced the executable from 1,538,776 to 1,409,400 bytes and arithmetic from 7.024 to 4.782 ms. On the local M1 it reduced the executable from 1,251,104 to 1,169,072 bytes. Field splitting took 10–14% longer across these three machines, and the local M1's first prompt increased from 2.55 to 2.66 ms. The run records include the exact training workloads. [Compiler trials](https://github.com/sanohiro/tonari-shell/actions/runs/45288255019) · [Intel profiling repeat](https://github.com/sanohiro/tonari-shell/actions/runs/45288830220)

同じ訓練内容の PGO で3機種ともサイズが減り、算術や関数は速くなりました。一方、展開後の単語分割は3機種とも遅くなったため、この訓練結果を配布設定に固定していません。静的 Linux でサイズに効いている同一コードの統合と再配置表の圧縮も個別に外して比較しました。Xeon の通常版は1,777,720バイトで、コード統合を外すと1,790,584、再配置表の圧縮を外すと1,819,224バイトでした。

## Release performance / 最終版の実測

The cells show milliseconds as Tonari median/p95 · Bash median/p95 on the same host. The first two rows show median/p95/p99/maximum across 1,536 acknowledged edits per shell. The million-row history tests use Tonari's on-disk journal and report Tonari alone. Linux used the runner's system Bash and macOS used Homebrew Bash. The feature matrix below identifies its own reference Bash versions. [Exact sample counts, commands and fixtures](../../tools/ci/native-bench.sh) · [Release measurements](https://github.com/sanohiro/tonari-shell/actions/runs/45289725057)

各セルの左が Tonari、右が同じホストの Bash です。先頭2行は約64 KiB の入力行に対する連続編集です。10万件の行は共通接頭辞まで補完して待機中のキーを表示する時間、1万件の行は候補一覧を出し切る時間です。

| Workload | Linux x86-64 | Linux arm64 | macOS arm64 |
| --- | ---: | ---: | ---: |
| 64 KiB repeated append | 0.049/0.068/0.082/0.102 · 0.875/0.891/0.914/1.422 | 0.032/0.035/0.044/0.070 · 0.812/0.826/0.855/0.978 | 0.016/0.059/0.074/0.109 · 0.746/1.101/1.742/2.736 |
| 64 KiB repeated insert near head | 0.107/0.134/0.148/0.170 · 6.559/7.366/7.563/9.229 | 0.070/0.076/0.080/0.140 · 8.271/8.389/8.419/8.922 | 0.073/0.137/0.190/2.025 · 1.781/3.772/4.147/6.619 |
| IME commit, 17 scalars | 0.128/0.174 · 0.279/0.313 | 0.091/0.102 · 0.163/0.175 | 0.104/0.239 · 0.157/0.291 |
| 1 MiB verified paste | 263.016/269.039 · 4382.900/4811.091 | 139.669/150.967 · 2610.837/2996.458 | 271.352/339.865 · 1966.074/2148.676 |
| Million-row, recent Ctrl-R | 0.192/0.222 | 0.154/0.163 | 0.451/0.763 |
| Million-row, oldest Ctrl-R | 5.245/5.407 | 9.378/9.537 | 11.132/19.284 |
| 100k files, Tab and queued key | 0.104/0.126 · 60.821/61.713 | 0.093/0.105 · 54.913/55.729 | 0.390/2.976 · 818.098/1181.755 |
| List 10,000 candidates | 8.235/8.577 · 55.781/60.956 | 6.374/6.416 · 31.450/31.828 | 17.478/23.362 · 90.411/102.038 |
| Key arriving 1 ms after Tab | 0.105/0.163 · 1.266/1.335 | 0.066/0.105 · 0.868/0.906 | 0.094/0.219 · 3.754/7.225 |
| Exec to first prompt, no rc | 0.495/0.538 · 1.050/1.130 | 0.738/0.799 · 1.333/1.458 | 2.529/3.522 · 6.468/9.482 |
| Empty shell through exit | 0.180/0.212 · 0.571/0.623 | 0.371/0.417 · 0.858/0.924 | 2.183/3.348 · 5.987/9.207 |
| Two-stage external pipeline | 0.713/0.764 · 1.234/1.296 | 1.083/1.169 · 1.765/1.890 | 3.634/7.483 · 8.112/13.557 |
| 16 external commands | 4.858/5.077 · 7.135/7.423 | 7.429/7.749 · 10.599/11.039 | 22.934/48.045 · 35.609/76.301 |
| 100 builtin redirections | 0.374/0.418 · 0.920/0.988 | 0.682/0.737 · 1.443/1.506 | 3.071/5.137 · 7.713/9.462 |
| 1,000 builtin iterations | 0.698/0.735 · 1.671/1.718 | 1.076/1.124 · 2.300/2.362 | 3.255/6.775 · 8.987/15.308 |
| Repeated function definitions | 2.664/2.706 · 2.125/2.188 | 3.287/3.340 · 2.881/2.936 | 2.315/2.714 · 3.278/4.244 |
| Light rc to first prompt | 1.383/1.437 · 2.271/2.412 | 1.435/1.510 · 2.217/2.329 | 3.120/6.170 · 8.127/13.574 |
| Git completion rc to first prompt | 4.462/4.627 · 6.333/6.462 | 4.394/4.653 · 6.288/6.639 | 5.521/7.856 · 11.728/15.895 |
| 760-line function rc to first prompt | 1.872/1.968 · 3.741/3.878 | 2.022/2.110 · 3.804/4.003 | 3.290/5.108 · 10.294/15.522 |

The CI distribution executables were 1,777,688 bytes for Linux x86-64, 1,538,776 for Linux arm64, and 1,251,072 for macOS arm64. The Linux targets build stripped static PIEs with musl. The macOS arm64 executable links libSystem. [Build checks](../../tools/build-release.sh) · [Size and SHA-256 per run](native-results.tar.gz)

The repeated-definition row above measures the version before the function parser changes. A subsequent native run interleaved both Tonari releases and Bash on each machine. 上表の関数再定義は改善前の数値です。変更前後と Bash を同じマシンで比較した結果を別表に掲載しています。 [Function redefinitions / 関数再定義](../benchmarks.md#function-redefinitions)

## Feature coverage / 機能別の到達点

Shell revision `dfd9b77` produced the same 7,558 correctness outcomes as the first native run at `e48a0e5`. The table counts public feature names. Several names share a witness. Only matching cases enter the timing counts. Some operations also have an already-started measurement to isolate their cost inside a script. [Every named feature, witness and p95](matrix.md) · [Reference Bash builds](environment.json)

| Environment | Inventory rows | Whole run at ≤ 2/3 of Bash / timed | Already started at ≤ 2/3 / timed | Already started at parity or faster / timed |
| --- | ---: | ---: | ---: | ---: |
| macOS arm64 | 1,408 | 618 / 758 | 108 / 675 | 644 / 675 |
| Debian 13 x86-64 | 1,522 | 620 / 776 | 36 / 676 | 671 / 676 |
| Debian 13 arm64 | 1,522 | 527 / 778 | 28 / 678 | 673 / 678 |
| Ubuntu 26.04 x86-64 | 1,553 | 517 / 772 | 17 / 674 | 667 / 674 |
| Ubuntu 26.04 arm64 | 1,553 | 498 / 774 | 14 / 676 | 671 / 676 |

起動込みと起動後の反復では結果が変わります。例えば Xeon 上で関数を2,000回再定義すると、起動後の中央値は Tonari 2.664 ms、Bash 2.125 ms でした。各 `-crossings.tsv` は過去の別マシンの保存値と照合して、新たにしきい値を超えた項目です。同一マシンで実装の変更を比較した結果は「実装の判断」に示しました。

## Historical coverage / 過去版の範囲

The replay selects all 46 reachable revisions through `3999b0e` that changed shell source, the Cargo manifest or lockfile, the release builder or a benchmark driver. Each revision runs its own checked-in Python benchmark drivers with the common distribution compiler and optimization settings. Startup uses 11 samples per case and the other historical drivers use seven. Linux-only memory probes require Linux. The first two prototypes predate the project's panic strategy. Their replay uses the same abort setting as every later build. The standalone parser source and Cargo files from tree `119e729` are preserved at the fixture tag `benchmark-data/parser-119e729`. The additional control reconstructs the documented 1/2/2 initial capacities and times nvm, a builtin loop and startup. It also repeats 40 checked PATH lookups with one and 17 directories, with hashing enabled and disabled. [Early controls](https://github.com/sanohiro/tonari-shell/actions/runs/45295429042)

All 138 builds and 659 historical driver runs completed with exit status zero. Each build’s recorded driver list was checked against its Git tree. [Original replay](https://github.com/sanohiro/tonari-shell/actions/runs/45286422405) · [Rebuilt early prototypes](https://github.com/sanohiro/tonari-shell/actions/runs/45287653484)

Rebuilding the parser capacity control reduced the nvm median from 5.556 to 5.364 ms on native x86-64 and from 7.432 to 6.746 ms on Neoverse. On the hosted M1 it increased the median from 6.248 to 6.400 ms. These results apply to the archived parser. With 17 directories in PATH, hashing reduced the median for 40 commands from 174.281 to 78.262 ms on the hosted M1. The Linux runs were within 0.02 ms between the two modes.

The current suites separately time optional historical scenarios with higher sample counts, including a 760-line function, the original 94,826-byte Git completion, pipeline sizes from one byte to 256 MiB, interactive editing, and one-CPU affinity on Linux. [Per-revision exit status and size](history.tsv) · [Historical output and build logs](history-results.tar.gz) · [Replay driver](../../tools/ci/replay-history.py)

過去46版は、当時のソースと計測コードを使い、現在のネイティブ環境で再ビルドしました。各時点の OS イメージや未保存の試作を再現した数字ではありません。軽量な rc は公開用のエイリアス・PATH・遅延 nvm 関数に置き換えました。正しさの照合後に時間を取る現行の機能別計測は、macOS と Debian・Ubuntu の両 Linux アーキテクチャで行っています。機能ごとの未実装・動作差と Bash 比 2/3 を超えた項目は一覧に残しています。[機能別一覧](matrix.md)

## Remaining native interfaces / OS との接点

The editing, history, completion scheduling, interpreter and POSIX descriptor-copy algorithms share their implementations. Platform-specific code supplies the native environment pointer and errno, filesystem and directory-entry interfaces, process file descriptors, resource-limit constants, and OS-specific Bash startup files and identity strings. Linux executables link static musl. macOS executables link the system's libSystem. Linux pathname expansion uses UTF-8 byte order, while the existing macOS contract follows locale collation. A shell script can observe that ordering through positional arguments. [Process launch](../../src/execute/spawn.rs) · [Filesystem checks](../../src/subshell.rs) · [Locale policy](../engineering-notes.md#runtime-dependencies)

編集・履歴・補完の並列化・スクリプト評価・標準記述子の複製は共通実装です。リソース上限の定数、ファイルシステムの識別、Linux の `/proc/self/fd`、ディストリビューションの Bash 初期化ファイルなど、OS が提供する値や API はその接点で扱います。ファイル名展開の並び順はスクリプトから観測できるため、既存の環境別の仕様に従います。

## Reproduction and reruns / 再実行

The native benchmark, historical replay and platform comparison workflows accept manual runs. They record `uname`, CPU model, toolchain versions and binary hashes, with Linux affinity controls for single-CPU experiments. The platform comparison driver names each source substitution and retains each variant’s size and hash. [Release suites](../../.github/workflows/native-benchmarks.yml) · [Historical suites](../../.github/workflows/benchmark-history.yml) · [A/B experiments](../../tools/ci/platform-comparisons.py)

The first large-paste measurement used a visible marker that could wrap at a PTY column boundary. The replacement checks a single-cell marker and verifies the entire pasted buffer. The startup fixture switched download endpoints after an HTTP 403 and checks the same 94,826-byte completion file by SHA-256. The x86 PGO training binary required the compiler runtime during instrumentation. [Paste and profile reruns](https://github.com/sanohiro/tonari-shell/actions/runs/45286891491) · [Final release suites](https://github.com/sanohiro/tonari-shell/actions/runs/45289725057) · [Final startup fixtures](https://github.com/sanohiro/tonari-shell/actions/runs/45290035440) · [Native x86 PGO rerun](https://github.com/sanohiro/tonari-shell/actions/runs/45288830220)

初回の失敗と再実行の両方をジョブ一覧に残しました。貼り付けは表示と全バイトの一致、rc の再取得は SHA-256 で元の入力との一致を確認しています。
