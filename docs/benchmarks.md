# Startup and execution / 起動と実行性能

## 2026-11-26 · Bash-passing POSIX cases / POSIX 互換性修正後の性能

The three-run comparison covers 11,079 cases per host, including the generated signal tests. The table gives the median of the per-case Tonari/Bash ratios. The last column counts cases that still took at least as long as Bash when every initial miss of the 2/3 target was repeated 11 times. [Case tables and replay method](compatibility/yash-posix-followup/README.md)

| Native host | Joint passes | Median ratio | At most 2/3 on the first sweep | At least Bash time on replay |
| --- | ---: | ---: | ---: | ---: |
| Linux x86-64 | 9,311 | 0.430 | 9,037 | 5 |
| macOS arm64 | 9,310 | 0.478 | 8,640 | 0 |

Linux の共有標準入力で1,500行の代入を実行する時間は、変更前 3.31 ms、修正後 0.511 ms、Bash 1.997 msでした。`#!` のない実行ファイルを30回続けて呼ぶと、M1 で 90.736 ms から 24.655 ms に短縮しました。Bash は 30.342 ms でした。外部プログラム100回の起動は同じランナーの C 対照と併記しています。[Shared stdin and process launches](compatibility/yash-posix-followup/throughput.tsv) · [Executable text](compatibility/yash-posix-followup/executable-text.tsv)

<a id="yash-posix"></a>

## 2026-11-25 · POSIX sh workloads / sh モードの項目別計測

The same pinned yash inputs run on native Linux x86-64 and macOS arm64. The full record links every case to its assertion results, timed samples and observed Rust functions, with a separate total per fixture. [Comparison and case tables](compatibility/yash-posix/README.md) · [Full native samples](https://github.com/sanohiro/tonari-shell/actions/runs/45615699784) · [Higher-repeat measurements](https://github.com/sanohiro/tonari-shell/actions/runs/45616001148)

The physical Apple M1 comparison uses `a5ea7db` (first working sh-mode implementation), `10e63fd` and Bash 5.2.37. One warmup, 61 shuffled samples for startup and 17 for each repeated workload. Every sample validates stdout, stderr and exit status against Bash. Times are wall-clock medians in milliseconds.

| M1 workload | First | Tonari | Bash | Tonari / Bash |
| --- | ---: | ---: | ---: | ---: |
| Start and execute `:` | 3.335 | 3.300 | 5.894 | 0.560 |
| 10,000 special builtin prefixes | 18.227 | 16.735 | 34.748 | 0.482 |
| 12,000 builtin writes | 27.528 | 27.285 | 51.395 | 0.531 |
| 100 absolute external launches | 102.443 | 102.582 | 163.733 | 0.627 |
| 500 aliases in one compound | 6.706 | 4.253 | 7.454 | 0.571 |
| 3,000 aliases in one compound | 92.554 | 8.517 | 14.122 | 0.603 |
| 1,500 assignment lines on stdin | 5.453 | 5.414 | 7.210 | 0.751 |

変数の接頭代入では準備済みの名前と値を移動し、エイリアスでは後続の全トークンの移動とソース全体の繰り返し保存を省きました。通常の外部起動では不要な `posix_spawn` 属性を省略しましたが、M1 の100回起動では初回実装に対して測定上の改善はありません。3,000回のエイリアスは Bash 比0.603でした。共有標準入力の1,500行は改善後も Bash 比2/3に届きませんでした。M1 のリリースサイズは変更前 `d858d41` の1,450,016バイトから1,466,560バイトになり、動的依存先は引き続き libSystem のみです。[Raw M1 samples](compatibility/yash-posix/throughput-m1.jsonl) · [Reproduction driver](../tools/ci/yash-throughput.py)

On Linux x86-64 the final 3,000-alias group took 5.762 ms against 10.155 ms for Bash. On the macOS arm64 CI runner the corresponding medians were 11.070 and 20.537 ms. One hundred external program launches remained above the 2/3 target on both CI hosts, at 51.455 / 72.122 ms and 230.386 / 300.692 ms respectively. The earlier working implementation on the same runners measured 52.881 and 212.507 ms. The physical M1 and both CI samples therefore give no repeatable external-launch speedup for this change. [All repeated workloads by host](compatibility/yash-posix/throughput-native.tsv)

The shared-stdin experiment keeps the command text and the input descriptor identical and changes the seekable reader's capacity in a disposable build. There are 17 measured runs per executable after warmup. The short workload executes 1,500 scalar assignments. Medium lines carry 600-byte values and long lines carry 16,000-byte values. The shared-read workload consumes the following physical line inside the script. [Native buffer experiment](https://github.com/sanohiro/tonari-shell/actions/runs/45617420882) · [Inputs](../tools/ci/yash-stdin.py)

| Host | Shared stdin | 512 B, ms | 8 KiB, ms | 32 KiB, ms | Bash, ms |
| --- | --- | ---: | ---: | ---: | ---: |
| Linux x86-64 | short | 1.668 | 1.720 | 1.723 | 1.653 |
| Linux x86-64 | medium | 3.728 | 3.516 | 4.212 | 8.402 |
| Linux x86-64 | long | 6.489 | 5.705 | 5.705 | 20.504 |
| Linux x86-64 | shared read | 1.683 | 1.741 | 1.733 | 2.458 |
| macOS arm64 | short | 5.510 | 5.353 | 5.280 | 6.890 |
| macOS arm64 | medium | 8.170 | 8.379 | 10.448 | 18.291 |
| macOS arm64 | long | 12.331 | 10.860 | 10.316 | 34.737 |
| macOS arm64 | shared read | 5.882 | 5.875 | 5.891 | 9.060 |

512バイトは Linux の短い入力を約0.05 ms縮める一方、長い行は両OSで遅くなりました。32 KiBでは600バイト幅の入力が両OSで遅くなり、総合では8 KiBを維持しました。標準入力のオフセットは `read` や子プロセスと共有するため、先読みした内容を実行前に正しく戻す必要があります。

On the Linux recheck the kernel's core pattern points to `systemd-coredump`. The soft core-size limit was zero. A second run emitted a single byte immediately before the real shell builtin `kill`, then timed until the parent reaped the process. With SIGSEGV, Tonari spent 1.477 ms before the marker and 157.781 ms afterwards. Bash spent 2.198 and 175.468 ms respectively. With SIGTERM, the corresponding times were 0.388 and 0.061 ms for Tonari and 0.911 and 0.148 ms for Bash. Thus a fatal-signal case can measure a substantially different operation from ordinary shell startup. The marker-to-reap interval includes the builtin, kernel, handler and scheduler. [Native signals and samples](https://github.com/sanohiro/tonari-shell/actions/runs/45616001148) · [Split measurement](../tools/ci/yash-signals.py)

Linux `kill1-p.tst:24` only prints the name of signal 14. Its three-sample medians were 1.451 ms for Tonari and 1.203 ms for Bash. On the physical M1, the exact stdin script `kill -l 14` took 3.991 / 6.948 ms and a loop of 5,000 lookups with every output line checked took 12.464 / 26.539 ms (17 interleaved samples). The Linux observation remains in the above-target table. In the eleven-trial Linux recheck, `shift-p.tst:19` moved from a ratio of 1.316 to 0.576 and `path-p.tst:54` from 1.222 to 0.588. [Per-case repeats](compatibility/yash-posix/above-target.tsv)

<a id="configure-pairs"></a>

## 2026-11-25 · Why a full configure can lose to Bash / 個別の処理が速くても configure が逆転した理由

The previous macOS CI result (20.484 s Tonari, 19.488 s Bash) measured the entire configure. The three original Tonari samples were 20.763, 19.267 and 20.484 s. Bash took 20.737, 19.488 and 17.419 s. Those samples retain total elapsed time only. We repeated the comparison with the same absolute interpreter path, fresh build path and compiler on both sides. We alternated the order and checked all progress results, the generated header, the config.status interpreter, exit status and stderr. A separate diagnostic copy timed the actual compiler and its generated executables within each run. [Original samples](https://github.com/sanohiro/tonari-shell/actions/runs/45562340445) · [Paired runs before the change](https://github.com/sanohiro/tonari-shell/actions/runs/45579287698) · [Driver](../tools/ci/configure-pairs.py)

変更前の macOS CI では、計測用の6組中3組で Tonari と Bash の順位が逆転しました。以下はその3組の内訳で、正の値は Tonari のほうが長いことを示します。各行は同じ組の実測です。「その他」にはシェル本体、計時対象外の外部コマンド、ファイル操作、計測器、スケジューリングが入ります。両方とも毎回コンパイラを229回、生成した検査プログラムを46回実行しました。別に実行した計測器なしの6組では Tonari が4勝、Bash が2勝でした。[イベントと実行順](https://github.com/sanohiro/tonari-shell/actions/runs/45579287698)

| Previous macOS CI, Tonari minus Bash (s) | Compiler | Generated tests | Other elapsed | Whole run |
|---|---:|---:|---:|---:|
| Pair 3 | +0.461 | +0.083 | +0.536 | +1.080 |
| Pair 4 | +1.055 | −0.013 | +0.128 | +1.170 |
| Pair 5 | +1.281 | +0.031 | −0.218 | +1.094 |

A second native run recorded the compiler’s kernel-reported user and system CPU time with `wait4`. In macOS pair 1, Tonari took 1.991 s longer overall. Compiler elapsed time was 1.762 s longer, and its charged CPU time was 1.645 s higher. Of the 229 compiler calls, 156 took longer elapsed time in invocation order. In pair 2 the compiler was 2.155 s faster under Tonari. In pair 4 the compiler was 0.244 s faster, but the other elapsed time excluding the generated probes was 1.537 s longer. The records localize elapsed time and CPU time to these operations. The underlying host-level cause remains undetermined. On the Linux x86-64 runner, the six uninstrumented pairs all favored Tonari by 0.265–0.349 s. [Both shells’ wall time, CPU time and individual child records](https://github.com/sanohiro/tonari-shell/actions/runs/45582195287)

The release had also been doing extra filesystem work. On the physical M1, GNU Hello invoked `cat` 218 times with an expanded here-document on fd 0 followed by a file open on fd 1. It also used other redirection orders. Across the entire configure, Tonari opened 261 here-inputs and created 128 temporary files, ranging from 514 to 53,335 bytes. The new version created zero for those same 261 inputs. Both builds produced the same header and all 240 checks. The count came from separate diagnostic builds that recorded calls to `here_input::open` and `temporary`. All release timing below excludes these counters. [Implementation and reader behavior](engineering-notes.md) · [Benchmarked code](https://github.com/sanohiro/tonari-shell/commit/026d8a2)

A separate untimed M1 run recorded the compiler argument vector for every invocation and SHA-256 of each named C, assembly or preprocessed source argument. The two shells issued the same 229 compiler argument vectors in the same order. All 222 calls with named source files supplied byte-identical source. The other seven calls had no such named input. The 240 checks and `config.h` matched. The final build repeated that comparison and also inspected the exported environment in a clean startup. The differences were `SHLVL` after configure re-executed the shell, `_` for every call and `PWD` on 16 calls. The working directory was identical. Replaying the two captured environments directly with the same C source and compiler gave medians of 28.62 / 28.96 ms for compilation and 57.94 / 58.83 ms for compilation and linking (Tonari environment / Bash environment, 64 runs). In this M1 control the compiler took slightly less time with the Tonari environment. [Physical samples](native-2026-11-25/configure-m1.json)

起動だけを見ると、最初の修正版の macOS CI で、起動済みシェルから外部プログラムの `main` までの中央値は Tonari **2.128 ms**、Bash **3.030 ms**、子が終了を始めて親が処理を再開するまでは **0.183 / 0.222 ms** でした。Linux x86-64 は順に **0.303 / 0.430 ms**、**0.094 / 0.110 ms** でした。各条件384回の測定です。コンパイラで頻出するログ fd の複製を追加した場合、子に入るまでは macOS **2.120 / 2.928 ms**、Linux **0.304 / 0.442 ms** でした。[子と親で対応づけたタイムスタンプ](https://github.com/sanohiro/tonari-shell/actions/runs/45580598108)

The final release uses the direct launch path when a quoted positional parameter or scalar supplies the absolute executable in a command substitution. The following fresh-directory measurements rotate the previous release (`57e91ba`), the final release (`819a255`) and Bash on the same machine. Each shell has one warmup and six measured full runs. On CI a fourth build with opt-level 3 rotates through the same experiment. Warmup builds the configured program, runs its checks and verifies its output. [Final native CI samples, binaries and host records](https://github.com/sanohiro/tonari-shell/actions/runs/45584252377) · [M1 samples](native-2026-11-25/configure-m1.json)

| Real GNU Hello configure (s) | Previous | Final | Bash | Final / Bash |
|---|---:|---:|---:|---:|
| Physical M1 | 26.337 | 26.032 | 27.583 | 0.944 |
| macOS arm64 CI | 15.701 | 15.434 | 16.042 | 0.962 |
| Linux x86-64 CI | 6.129 | 6.131 | 6.430 | 0.953 |

同じ配布版で、外部コマンドに入力を渡す処理と、外部コマンドの出力を取り込む処理を測り直しました。各欄は「以前の Tonari → 最終版 / Bash」のミリ秒で、実行回数は M1 が各15回、両 CI が各41回です。Bash は M1 が5.2.37、macOS CI のこの短時間テストが5.3.20、Linux が5.2.21です。[短時間テストと p95](https://github.com/sanohiro/tonari-shell/actions/runs/45584252377)

| Workload (repetitions per sample) | Physical M1 | macOS arm64 CI | Linux x86-64 CI |
|---|---:|---:|---:|
| Here-document → external `cat` → file (120) | 350.64 → 231.47 / 242.69 | 619.54 → 348.67 / 399.78 | 55.00 → 55.23 / 67.87 |
| Here-string → captured external `cat` (120) | 341.88 → 134.28 / 298.58 | 587.86 → 339.12 / 624.74 | 38.99 → 39.17 / 102.07 |
| Here-string → `read -r` (120) | 25.18 → 1.10 / 139.06 | 57.95 → 1.38 / 246.13 | 0.68 → 0.67 / 86.69 |
| Captured program passed as `"$1"` (96) | 160.27 → 106.03 / 169.26 | 306.13 → 218.27 / 325.62 | 38.40 → 29.68 / 48.34 |
| Captured program passed as `"$tool"` (96) | 160.90 → 105.54 / 170.05 | 263.95 → 185.94 / 286.10 | 38.43 → 29.88 / 48.51 |

計測器を入れた独立の実験では、実機 M1 の変更前でコンパイラと生成した実行ファイル内の合計が中央値21.00秒、最初の修正版の Linux x86-64 で6.46秒でした。同じ各実験で計測器なしの Bash の2/3はそれぞれ18.96秒、5.69秒です。今回の最終版も実 configure 全体の 2/3 には届いていません。macOS CI ではコンパイラ以外の区間でも秒単位の変動を観測しました。実機の旧版ペアの一部はローカルの開発作業と重なりました。最終版の実機ベンチは順次実行しています。

最終版は Rust 58件を通過し、M1 のシェル試験は229件中227件通過、OS固有の2件をスキップしました。互換性の既存判定は M1 の1,411件、Debian x86-64 の1,525件とも変更前と一致しました。配布用バイナリは M1 で1,450,016バイト、追加の動的ライブラリなし。Linux x86-64 は strip 済みの静的 PIE で1,952,344バイト（変更前より1,760バイト増）です。macOS CI の20ジョブの測定は各31回で Tonari 180.10 ms、Bash 179.94 ms、以前の Tonari 168.23 ms でした。[正確性・バイナリ・ジョブの全記録](https://github.com/sanohiro/tonari-shell/actions/runs/45584252377)

In the previous review, the luish build was almost tied on hosted macOS. A new 15-run interleaved comparison took **2.776 s Tonari / 3.049 s Bash**, with the previous Tonari at 2.861 s. The physical M1's upstream `jobs.bg_many@20` ran in **40.32 / 42.81 ms** (31 samples, Tonari / Bash). Timing within the already-running shell, the launch loop for 20 external `sleep 0.02` processes took **8.40 / 8.13 ms** and the final `wait` took **26.78 / 28.17 ms**. These are medians across trials; the actual children run concurrently. These controls used the first here-input release. [luish samples](https://github.com/sanohiro/tonari-shell/actions/runs/45581338139) · [Physical samples](native-2026-11-25/configure-m1.json)

To repeat the whole comparison, run `python3 tools/ci/configure-pairs.py /path/to/hello-2.12.2/configure /path/to/tonari /path/to/bash results`. For a three-way release comparison use `python3 tools/bench_external.py gnu-hello /path/to/hello-2.12.2 /path/to/tonari /path/to/bash --previous /path/to/previous --case 'GNU Hello configure' --runs 6`. For a focused case use `python3 tools/bench_scripts.py /path/to/tonari --bash /path/to/bash --previous /path/to/previous --timing started --case heredoc-external-file --runs 15`.

<a id="external-case-review"></a>

## 2026-11-25 · Performance and implementation review of 147 workloads / 147項目の実装と実測

Each row in the ledger identifies the operation, its source functions, the implementation decision and the native measurements. The shipping release is `350959e`, the comparison release is `135e4bb`. Each CI timing job rotated Bash, the previous release, the shipping release and an `-O3` build of the same source. The reference was Bash 5.2.37 on macOS and 5.2.21 on Ubuntu x86-64. The counts below are **at or below two-thirds of Bash time / faster than Bash / supported**. [Per-case review](external-shell-review.md) · [Raw values and observed functions](external-shell-review.csv) · [Native timings and release hashes](https://github.com/sanohiro/tonari-shell/actions/runs/45562340445)

全147項目について、スクリプトが実際に行う処理、通る関数、削れる処理と残す理由を記録しました。表は **Bash の所要時間の 2/3 以下 / Bash より速い / 比較成立** の件数です。unixwin は従来から掲載している50項目で、上流の全4,302項目から選んだものです。ShellSpec の空の計時本体と `print` は両環境で比較できません。macOS の `expr substr` と、環境に存在しない外部 `true` のパスも個別に明記しました。[項目別のソースと判断](external-shell-review.md)

| Suite | Published | macOS arm64 | Linux x86-64 |
|---|---:|---:|---:|
| shellspec | 70 | 61 / 67 / 67 | 45 / 68 / 68 |
| fumiyas | 17 | 15 / 16 / 16 | 15 / 16 / 16 |
| unixwin | 50 | 39 / 50 / 50 | 41 / 50 / 50 |
| luish | 7 | 5 / 7 / 7 | 5 / 7 / 7 |
| gnu-hello | 3 | 0 / 2 / 3 | 0 / 3 / 3 |
| **Total** | 147 | 120 / 142 / 143 | 106 / 144 / 144 |

実装を変更した経路を測る項目は74件、既存の経路を確認して維持した項目は62件、複数の経路を個別に調べた統合試験は9件でした。両方の CI で旧版と正しく比較できた141項目のうち、中央値が両方で10%以上短くなったのは40件、両方で10%以上長くなったのは0件です。旧版と Bash を分母にした所要時間の比を各項目に併記しました。[全項目の比較](external-shell-review.md)

| Elapsed ms, Tonari / Bash | macOS arm64 CI | Linux x86-64 CI | Physical M1 |
|---|---:|---:|---:|
| startup.empty_c | 5.59 / 10.52 | 0.23 / 0.64 | 4.94 / 8.27 |
| funcs.redefine | 7.08 / 16.80 | 0.67 / 3.63 | 5.17 / 11.80 |
| expand.nested_cmdsubst | 6.12 / 976.67 | 1.22 / 429.71 | 6.10 / 1,345.36 |
| strings.concat_subst | 9.61 / 83.05 | 1.98 / 23.35 | 6.95 / 61.47 |
| subst.cmd_ext_date | 451.37 / 538.08 | 34.01 / 55.33 | 174.19 / 274.16 |
| jobs.bg_many@20 | 183.97 / 190.63 | 23.79 / 25.27 | 47.31 / 46.15 |
| strings | 388.79 / 1,039.81 | 378.88 / 750.03 | 309.58 / 748.75 |
| functions | 305.98 / 951.86 | 298.30 / 806.28 | 206.68 / 554.09 |
| build | 3,133.60 / 3,147.95 | 434.18 / 621.54 | 1,561.65 / 2,187.93 |
| configure syntax check | 24.90 / 30.00 | 23.55 / 23.85 | 16.25 / 26.14 |
| GNU Hello configure | 20,483.70 / 19,488.42 | 9,337.47 / 9,859.86 | 28,766.37 / 30,121.75 |

On hosted macOS, GNU Hello configure took 1.051 times as long as Bash. Tonari finished every other supported macOS workload and all supported Linux x86-64 workloads faster than Bash. The real GNU Hello configure includes the compiler and generated test executables. Each sample used an empty build directory and verified config.h and the config.status interpreter. Warmup also built the program and ran `make check`. [Direct measurements of compiler and probe waits](#configure-foreground)

直前の段階でも別のランナーで、`adbde41`、`aad5e23`、Bash の順を入れ替えて再検証しました。GNU Hello の実 configure は各7回の中央値で、macOS が **22.905 / 21.195 / 22.382 秒**、Linux が **6.478 / 6.467 / 6.610 秒** です。同じ macOS 再試験で20ジョブの待機を31回測ると **179.98 / 195.84 / 190.68 ms** でした。値はすべて `adbde41` / `aad5e23` / Bash です。[独立した再測定と全サンプル](https://github.com/sanohiro/tonari-shell/actions/runs/45560988354)

全ファイル名を展開する `glob.star_all@300` は、macOS の全項目測定（各16回）で旧版 5.68 ms、採用版 8.43 ms、Bash 37.32 ms でした。同じコミット、同じスクリプトを使い、別の macOS arm64 ランナーで各64回、旧版・採用版・Bash・O3 の順を回して再測定すると、旧版 **3.445 ms**、採用版 **3.065 ms**、Bash **19.635 ms** でした。M1 実機でも旧版 4.90 → 採用版 4.69 ms でした。全項目表には最初の16回の結果をそのまま残しています。[64回の全サンプルとビルド情報](https://github.com/sanohiro/tonari-shell/actions/runs/45565017544)

**実機での変更前後。** 次の数字はミリ秒で、同じ M1 上の配布設定ビルドです。各比較で実行順を回し、標準出力、標準エラー、終了状態を照合しました。「変更前」はそれぞれの改善を入れる直前の版です。小さい引数列・大きい引数列の関数呼び出しも比較し、1024引数の転送は 21.37 → 22.90 ms（Bash 266.53 ms）でした。不要な引数移動を除いても、この条件では経過時間の短縮は確認できませんでした。

| M1 workload | Before | After | Bash | Timed runs per shell |
|---|---:|---:|---:|---:|
| Register the same function 100,000 times | 18.95 | 15.28 | 111.58 | 11 |
| Slice three characters from a 64 Ki-character variable 1,800 times, before/after borrowing | 9.86 | 6.65 | 326.06 | 7 |
| Read that variable’s UTF-8 length 1,800 times | 16.28 | 13.17 | 81.61 | 7 |
| Replace the first delimiter in a short string 40,000 times | 32.43 | 29.99 | 79.39 | 11 |
| Postincrement an ordinary decimal with `let` 40,000 times | 12.23 | 11.14 | 79.91 | 11 |
| Capture external `cut` with a here-string 100 times | 252.53 | 173.23 | 358.58 | 11 |
| Expand 100,000 unquoted words 20 times | 61 | 42 | 252 | 11 |
| Capture a list of three printf calls containing a newline format 250 times, after shell startup | 186.85 | 0.89 | 227.79 | 15 |

前の実装では、大きな変数の冒頭を切り出すために全文を複製し、全文の文字数を数えていました。現在は保存値を借用し、必要な UTF-8 の文字境界まで走査します。関数の再定義では、同じソースの同じ本体を登録済みなら再挿入を省きます。コマンド置換内の外部コマンドは、その呼び出しにだけ有効な探索結果を使って fd を渡せるので、親の hash 表に副作用を残しません。[Bash witnesses](../tests/shell.py) · [Large field-splitting control](https://github.com/sanohiro/tonari-shell/actions/runs/45553655231)

An additional control outside the published 147, with several printf calls in one substitution, exposed a false positive in the child-isolation check. A literal `n` anywhere in a format containing `%` could force a fork. Tonari now checks for a count conversion after each percent using the formatter’s flags and length modifiers. A list that really assigns a byte count via `%n` still runs in its private environment. On the M1, 100 such captures took 74.18 ms versus 72.84 ms previously and 99.18 ms in Bash. Reproduce with `python3 tools/bench_scripts.py ./release/tonari --bash /path/to/bash --case printf-list-newline --case printf-list-count --timing started`. [Paired native controls](https://github.com/sanohiro/tonari-shell/actions/runs/45562343811) · [Output and isolation witnesses](../tests/shell.py)

複数の printf を含む置換で、改行の `\n` を変数への書き込みとして扱い、余計な子プロセスを作る箇所を見つけました。`%n` が本当に書き込む値は後続のコマンドから見え、親の変数は保たれることを照合しました。改行の250回は CI でも旧版から macOS 124.429 → 0.691 ms、Linux 33.939 → 0.491 ms になりました。本当に書き込む100回は macOS 70.450 → 78.830 ms、Linux 14.026 → 13.973 ms でした。追加で、絵文字を含む部分文字列と置換1,920例、算術の境界値・配列・名前参照64例を Bash で照合しました。

Earlier controls during this review also compared pure builtin captures with parent signal traps, short writers on a real pipe, array length and in-place concatenation. On the physical M1, 250 captures of a function returning `echo 1` took 141.19 → 4.03 ms (Bash 178.48), 120 short `printf | cut` captures took 212.51 → 137.04 ms (Bash 338.79), and 40,000 self-prefixed eight-byte concatenations took 315.63 → 22.11 ms (Bash 18,782.32). Discarding complete top-level trees during large syntax checks cut peak RSS from about 24.5 to 15.6 MB. An alternative that chunked input at 32 KiB slowed the M1 syntax check from 17.23 to 25.36 ms and was discarded. [Parser control](https://github.com/sanohiro/tonari-shell/actions/runs/45552486120) · [Engineering notes](engineering-notes.md)

ShellSpec reports operations per second. Every host reverses shell order for a second independent pass, with one warmup second and one scored second for each shell. Elapsed ratios invert throughput. Other suites use medians after one warmup and rotate shell order. CI uses three timed samples per shell, except hosted macOS unixwin (16) and luish (9). Physical M1 adds unixwin (9), luish (5), GNU Hello (3), and two ShellSpec orders with the three distribution/reference binaries. Physical M1 fumiyas does not have a complete timing series. Individual short samples on the hosted runners varied substantially. A 40-sample identical-binary macOS control produced medians of 7.17 and 7.09 ms with individual samples from 4.46 to 16.46 ms. [Identical-binary control](https://github.com/sanohiro/tonari-shell/actions/runs/45556912758)

M1 実機では配布版の対話操作も検証しました。PTY を通じて、入力を送ってから応答が画面に現れるまでを測っています。ファイル補完の行は Tab に続けて送ったキーが表示されるまでの時間で、計時後に補完済みの行も照合しました。起動は51回、入力と履歴は15回、ファイル補完は13回です。[入力試験](../tools/bench_editor_input.py) · [補完試験](../tools/bench_completion.py)

| Physical M1, ms | Tonari median | Tonari p95 | Bash median | Bash p95 |
|---|---:|---:|---:|---:|
| 非対話起動から `-c :` 終了 | 2.763 | 3.035 | 5.822 | 6.740 |
| 対話起動から最初のプロンプト | 5.073 | 10.283 | 8.429 | 14.753 |
| 17文字の一括入力 | 0.092 | 0.194 | 0.156 | 0.556 |
| 16 KiB 貼り付けと後続キー | 2.469 | 5.272 | 18.508 | 99.369 |
| 10万件の履歴で Ctrl-R | 0.261 | 0.400 | 0.648 | 0.807 |
| 10万ファイルで Tab に続けたキーの表示 | 0.199 | 0.443 | 122.527 | 181.251 |

同じソースを一律に `-O3` でビルドした場合も全項目で試しました。配布設定に対し所要時間が10%以上短い件数 / 10%以上長い件数は、macOS は 143件中 20件 / 18件（全項目の比の中央値 0.988）、Linux は 144件中 0件 / 1件（全項目の比の中央値 0.993）でした。対象 CPU による実装分岐は追加していません。[設定別の実測値](external-shell-review.csv)

LLVM coverage was collected separately from release timing. It recorded at least one inspected source function for **143 macOS and 144 Linux cases**. The per-case CSV distinguishes unavailable samples and any function visible only in source inspection. The profiler uses actual source positions and executed function counts. Timings and correctness checks use independent drivers. ShellSpec includes a one-iteration observable-result witness. The fumiyas diagnostic shortens loop counts and runs the timed body in the instrumented process so its counters can be written. The timing runs always use the original upstream workload. [Linux source profiles](https://github.com/sanohiro/tonari-shell/actions/runs/45562373504) · [Coverage driver](../tools/ci/trace-external-paths.py) · [ShellSpec output checks](../tools/ci/check-shellspec.py)

配布バイナリの実時間と、診断ビルドの通過関数は別々に測りました。macOS arm64 と Linux x86-64 の順で、CI 配布バイナリのバイト数は **1,449,872 → 1,466,496 / 1,933,016 → 1,950,584**（旧版 → 採用版）です。Linux は静的 musl PIE、macOS の動的リンク先は libSystem のみです。物理 M1 では Rust の58テストが通り、シェル統合テストは227件中225件成功・OS 固有の2件スキップでした。参照照合は macOS 1,411件、Debian 1,525件。どちらも名前付きユーザーのチルダ展開が Bash と一致するようになり、ほかの判定の変化はありませんでした。

<a id="configure-foreground"></a>

## 2026-11-25 · Child environments and the full configure budget / 子プロセスの環境と configure 全体の目標

GNU Hello 2.12.2 ran its distributed `configure --disable-nls --cache-file=/dev/null` with `CC=/usr/bin/cc` in a fresh directory for every sample. Each host rotated the previous Tonari (`ce1e766`), the release with the inherited-variable lookup change (`c15db28`), and Bash. The table gives the median of three timed runs after one warmup per shell. The warmup also ran `make -j2 check` and verified the executable's output. Every timed run checked `config.h` and the interpreter in `config.status`. [Native CI runs and samples](https://github.com/sanohiro/tonari-shell/actions/runs/45546618314) · [Benchmark driver](../tools/bench_external.py)

| Native host | Previous Tonari (s) | Tonari (s) | Bash (s) | Bash × 2/3 (s) | Tonari / Bash |
|---|---:|---:|---:|---:|---:|
| Linux x86-64 CI | 8.246 | 8.105 | 8.507 | 5.671 | 0.953 |
| macOS arm64 CI | 21.954 | 21.841 | 20.825 | 13.883 | 1.049 |
| Physical M1 | 29.879 | 28.448 | 30.674 | 20.449 | 0.927 |

On the physical M1, the raw release times were 27.292, 29.358 and 28.448 s. Bash took 29.503, 30.674 and 30.675 s. Hosted macOS was sensitive to the run. Its three release times spanned 20.105–22.073 s and Bash spanned 20.614–22.648 s. All three hosts missed the two-thirds target.

In a separate experiment each host rotated the uninstrumented release, Bash and a diagnostic build of the same Tonari source. The diagnostic times synchronous waits after launch returns. It merges overlapping intervals, counts the compiler, its generated test executables and selected system utilities, and excludes shells, make, sleeps launched by the shell and pipelines. A sleep inside a C test is part of that test's runtime. The script, compiler and generated programs run unmodified in all three cases. Each run checks the same header, stderr and all progress checks (251 Linux, 240 macOS). The release and Bash columns below are independent whole-run measurements from this experiment. [Foreground timings and raw events](https://github.com/sanohiro/tonari-shell/actions/runs/45546793668) · [Driver and category definitions](../tools/ci/configure-waits.py)

| Native host | Release (s) | Bash (s) | Bash × 2/3 (s) | Median combined waits (s) | Range of waits (s) |
|---|---:|---:|---:|---:|---:|
| Linux x86-64 CI | 8.709 | 9.107 | 6.071 | 7.332 | 7.315–7.334 |
| macOS arm64 CI | 20.964 | 22.094 | 14.729 | 15.567 | 13.733–15.698 |
| Physical M1 | 28.663 | 30.517 | 20.345 | 25.410 | 24.399–26.903 |

Counting only the waits for the compiler and generated probes gave medians of 6.720 s on Linux (238 compiler calls and 54 probes) and 22.953 s on the physical M1 (225 and 46). Every individual sample on both hosts exceeded the corresponding budget in the second table. A second timer around the actual compiler and generated probes on the Linux profile runner measured 6.321 s in Tonari and 6.367 s in Bash. Both exceed the 5.671 s budget in the first table. On hosted macOS the combined wait including utilities was below budget in one trial. These timings describe the observed hardware, scheduler and compiler. [Compiler timing method](../tools/ci/configure-profile.py)

The inherited-variable optimization also has a direct control. With 2,000 local variables, one exported result and 250 launches of an external `true`, Linux took 90.819 ms versus 117.400 ms previously and 130.961 ms in Bash. Hosted macOS took 427.309 ms versus 419.958 and 646.010 ms, and the physical M1 took 331.797 ms versus 323.322 and 560.645 ms. These component tests used a three-variable input environment and seven timed trials. A physical M1 repeat rotated the shell order through all six permutations for 12 timed runs. It took 300.107 ms versus 318.894 ms previously (Bash 501.042 ms). Inheriting 64 additional variables named `EXPORTED_SETTING_0` through `EXPORTED_SETTING_63`, each set to `yes`, gave 306.837 ms versus 354.890 ms (Bash 513.259 ms). The small-environment Mac measurements varied in direction. Linux and the larger inherited environment on the M1 support keeping the shared implementation.

The stripped x86-64 static PIE grew by 96 bytes to 1,933,016 bytes. The arm64 executables kept their previous sizes (1,433,392 bytes on the physical M1 and 1,449,872 on CI). The physical binary links only to libSystem. All 58 Rust tests passed. The Mac shell suite ran 211 tests with two platform skips. All 1,411 macOS and 1,525 Debian differential verdicts matched the previous release. [Release sizes and Debian audit artifact](https://github.com/sanohiro/tonari-shell/actions/runs/45546618314)

配布版の GNU Hello、システムのコンパイラ、毎回空のビルドディレクトリで実測しました。最初の表は `configure` の起動から終了までの秒数です。Linux と M1 実機では Bash より短く、macOS CI では測定回によって前後しました。Bash の 2/3 にはいずれも届いていません。2つ目の表では、子プロセスの起動がシェルに戻った後から終了を受け取るまでの時間を別ビルドで測っています。Linux ではコンパイラと検査プログラムだけを別の方法で計時しても目標を超えました。変数が多い場合の子プロセスの環境構築には Linux で改善が見られ、両 OS に共通の実装を採用しました。

<a id="configure-latency"></a>

## 2026-11-25 · GNU Hello configure from start to exit / configure 全体の実行時間

GNU Hello 2.12.2 ran its distributed `configure --disable-nls --cache-file=/dev/null` with the same `/usr/bin/cc` in fresh build directories. The columns are elapsed seconds. Each host built and tested the project during warmup, then timed three clean configurations per shell with the first shell rotated each round. The same host ran the previous release (`200a71f`), the new release (`fa9c5cf`) and Bash. Every run checked the generated `config.h` and `config.status` interpreter. The two-thirds target applies to the entire command. The stripped Linux x86-64 static PIE grew from 1,930,008 to 1,932,920 bytes. The physical Mac binary grew from 1,433,376 to 1,433,392 bytes and links only libSystem. [Native results and raw samples](https://github.com/sanohiro/tonari-shell/actions/runs/45536959010)

| Native host | Previous | Tonari | Bash | Target | Tonari / Bash |
|---|---:|---:|---:|---:|---:|
| Linux x86-64 CI | 7.457 | 6.098 | 6.284 | 4.189 | 0.970 |
| macOS arm64 CI | 26.638 | 24.712 | 25.532 | 17.021 | 0.968 |
| Physical M1 | 31.043 | 30.117 | 31.961 | 21.307 | 0.942 |

The macOS CI samples for the new build ranged from 24.11 to 25.13 s, and Bash ranged from 25.28 to 26.53 s. The Linux samples for the new build ranged from 6.090 to 6.114 s.

A separate diagnostic used a small launcher to time the actual compiler and the generated test programs. The intervals start immediately before spawning the real program and end when it finishes. The total is the union of those intervals. The diagnostic instruments a copy of the configure script, verifies the resulting header and logs each invocation. Unmodified full-run timings appear in the table above. On Linux one generated test executes itself again and the outer invocation includes that time. The working-`fcntl.h` check contains a one-second sleep in the compiled C program. [Diagnostic](../tools/ci/configure-profile.py)

| Host and shell | Compiler calls | Compiler seconds | Direct test calls | Test seconds | Combined seconds |
|---|---:|---:|---:|---:|---:|
| Linux x86-64, Tonari | 244 | 3.767 | 54 | 1.023 | 4.789 |
| Linux x86-64, Bash | 244 | 3.747 | 54 | 1.022 | 4.769 |
| macOS CI, Tonari | 229 | 12.320 | 46 | 1.179 | 13.499 |
| macOS CI, Bash | 229 | 10.245 | 46 | 1.213 | 11.458 |
| Physical M1, Tonari | 229 | 9.477 | 46 | 14.872 | 24.349 |
| Physical M1, Bash | 229 | 9.749 | 46 | 13.653 | 23.402 |

On the Linux profiling runner the earlier host detection phase took 761 ms in Tonari and 26 ms in Bash. The reader had parsed a 39,222-byte case statement more than 1,200 times. With the final release the complete upstream helper took 23.9 ms on Linux, down from 559.8 ms in the previous release on the same runner. `config.sub x86_64-linux-gnu` fell from 108.7 to 2.5 ms. An external command writing to inherited fd 5 took 70.7 ms for 250 launches, down from 86.4 ms. On the physical M1 a 17.6 kB loop straddling a 128 KiB buffer boundary took 40.4 ms before repositioning the read window and 5.7 ms afterwards, compared with Bash at 8.8 ms. [Earlier phase trace](https://github.com/sanohiro/tonari-shell/actions/runs/45535633215)

The release passed all 58 Rust tests. The Mac integration suite ran 210 tests with two Linux-only skips, and all 1,411 Mac differential verdicts matched the previous build. The Debian x86-64 audit ran 1,525 feature checks including 83 GNU regression groups and retained every published match from the native comparison inventory. [Debian audit artifact](https://github.com/sanohiro/tonari-shell/actions/runs/45537822602)

Autoconf の結果キャッシュを無効にし、各回で新しいビルドディレクトリを使いました。最初の表は配布版 `configure` の起動から終了までです。修正前・修正後・Bash の順番を回し、各3回の中央値を比較しました。2/3 の合格条件は未達です。Linux ではコンパイラと生成したテストの待ち時間だけで、同じホストの Bash 全体の 2/3 を超えました。2つ目の表は別走行でこの2種類の外部処理を直接計測し、重複する時間を除いた値です。macOS の CI と実機にはテスト実行時間に大きな差があり、それぞれのホストで比較しました。

```sh
python3 tools/bench_external.py gnu-hello "$gnu_hello_source" "$tonari" "$bash" --previous "$previous_tonari"
```

<a id="external-suites"></a>

## 2026-11-25 · Independent shell workloads / 外部シェルベンチマーク

The paired results use native Ubuntu 24.04 x86-64 with its Bash 5.2.21 and native macOS 15 arm64 with Bash 5.2.37. The macOS runner reports an Apple M1 virtual machine with three cores. Counts below refer to individual workloads. The CSV contains both measurements, slower cases and unavailable commands. The local columns cover a physical M1 on macOS 27.2.1. ShellSpec there used a two-second sample and reversed shell order, and the luish and unixwin measurements used five timed trials. The elapsed-time ratio is Tonari / Bash, so a smaller value is better. For ShellSpec throughput it is Bash ops/s divided by Tonari ops/s. [All case measurements](external-shell-benchmarks.csv)

Counts show faster cases / comparable cases, followed in parentheses by the number at or below two thirds of Bash time.

| Suite | Linux x86-64 | macOS arm64 CI | Physical M1 |
|---|---:|---:|---:|
| ShellSpec samples | 68/68 (57) | 45/67 (35) | 67/67 (42) |
| fumiyas sh-benchmark | 16/16 (15) | 16/16 (15) | — |
| unixwin cross-section | 50/50 (42) | 47/50 (37) | 50/50 (40) |
| luish scripts | 7/7 (4) | 6/7 (5) | 7/7 (5) |

- **ShellSpec** ran every directive in all 12 sample files, using the upstream controller and its one-second preparation and measurement settings. Both shells reject the empty loop and lack the `print` builtin. The macOS `expr` also rejects GNU's `substr` syntax. The driver measures loop overhead together with the body. [Upstream samples](https://github.com/shellspec/shellbench/tree/4b04d18ecd170948fcbb52a130c708fd3e08e1af/sample)
- **Fumiyas** ran all 16 original workloads with the original sizes and a shared system zsh to generate the large inputs. The elapsed time comes from each shell's `time` around the original body, after setup. Linux has `/bin/true`. The macOS installation lacks it, so the unmodified fork/exec row is unavailable and a separately labeled row runs the same 40,000 calls to `/usr/bin/true`. A separate check confirmed one million array elements and 50 observable writes by process substitutions. [Original workloads](https://github.com/fumiyas/fumiyas.github.io/blob/d188b4fa60eca66f2bcd9f5a48a3c80a5c4fc307/2013/12/01/sh-benchmark-scripts.sh)
- **unixwin** has 4,302 cases at the pinned revision. We selected the first two in registration order from each of its 19 categories, plus 12 named cases for startup, expansion, functions, pipelines, globbing, data and mixed scripts. The harness uses the upstream fixtures, script generator and original iteration counts. Both shells receive identical work, with one warmup and three timed runs in alternating order on CI. It checks the expected status, output patterns, matching stdout and unexpected diagnostics. The two failing-command loops must each emit 500 diagnostics. Several other cases suppress output in the supplied workload. [Pinned corpus](https://github.com/unixwin/shellbench/tree/4fecc23d226ac5854cc2f1bbf74a6acf310c8ba8)
- **luish** ran all seven scripts at scale one, with the shell under test also set as `SH` for the build recipes. The harness compares stdout byte for byte, checks exit status and requires empty stderr on each run. The scripts cover arithmetic, arrays, a simulated build, a configure-style script, functions, strings and text processing. [Upstream scripts](https://github.com/luispedro/luish/tree/b43b78dd25fbd671c90b9e3265525d741089414d/bench/scripts)
- **GNU Hello 2.12.2** ran its distributed `configure --disable-nls --cache-file=/dev/null` in a fresh directory with `CC=/usr/bin/cc` and `CONFIG_SHELL` set to the shell being measured. The warmup also builds and runs `make check` and checks the resulting executable's output. Each timed run checks the generated header and the interpreter in `config.status`. Compilation and project tests after configuration are outside the timer. [GNU release](https://ftp.gnu.org/gnu/hello/hello-2.12.2.tar.gz)

The complete GNU Hello configuration took **7.27 / 6.15 s** on Linux, **24.15 / 22.90 s** on the macOS runner and **30.15 / 30.66 s** on the physical M1 (Tonari / Bash). On the physical Mac a separate compiler trace counted 229 compiler invocations in each shell. The 229 includes compiler identity and version queries. The real configuration remains a separate headline workload from luish's simulated configure. The physical M1 syntax check of GNU Hello fell from 396.87 to 16.53 ms, compared with 25.15 ms for Bash in that paired run.

All 67 comparable ShellSpec cases favored Tonari in the physical Mac rerun with shell order reversed. On the macOS 15 runner, 21 of its 22 slower results involved external `cut` or `sed` and the other used an explicit subshell. The runner's luish build took 3083.944 / 2630.056 ms, compared with 1342.323 / 1888.728 ms on the physical M1. Both are arm64 measurements on different OS releases and hosts.

ShellSpec and fumiyas used release `7d7d0d9`. luish used `78d762c`, before the final syntax-check-only refinement. Unixwin and GNU Hello used `a234864`. The physical Mac used `ca2bcd3` for the reverse-order ShellSpec pass and `a234864` for unixwin, luish and GNU Hello. Each CI artifact contains the host, binary metadata and full results. [ShellSpec and fumiyas](https://github.com/sanohiro/tonari-shell/actions/runs/45531064181) · [luish](https://github.com/sanohiro/tonari-shell/actions/runs/45532863391) · [unixwin and GNU Hello](https://github.com/sanohiro/tonari-shell/actions/runs/45533610889)

On the physical M1, a five-trial comparison of 100,000 successive shifts took 2.683 s before and 0.022 s after the change, against 0.099 s for Bash. Iterating a million quoted array elements five times took 1.183 s immediately before direct field expansion and 0.101 s afterwards, against 1.835 s for Bash in that run. The array count and low and high elements matched. Repeated appends in the 12,001-element sieve inside luish's arrays script went from 1.616 s to 0.079 s in a diagnostic run. Parent argument scopes, sparse indices, empty fields and literal wildcards have direct shell tests.

The first whole-file syntax-check implementation improved GNU Hello but regressed unixwin's Linux parse workloads. On three separate native runner pairs, the original, whole-file and final Tonari / Bash elapsed ratios for 5,000 mixed statements were 0.77, 1.53 and 0.66. The final reader streams short complete statements and tries a bounded parse of the remaining input after a compound stays incomplete. [Initial survey](https://github.com/sanohiro/tonari-shell/actions/runs/45529486446) · [Whole-file version](https://github.com/sanohiro/tonari-shell/actions/runs/45532863391) · [Final reader](https://github.com/sanohiro/tonari-shell/actions/runs/45533610889)

The original luish functions result was excluded because Tonari emitted an error at 128 nested calls. Both shells now finish the Ackermann workload with identical output. The evaluator still limits deep calls and accounts for the process stack limit. One Mac runner lacked `stdbuf` for the unixwin co-process case. Its rerun used Homebrew's GNU `stdbuf` and `tr` for that case only. On the physical Mac, GNU Hello produced identical results for 240 configuration checks, including 219 logged compiler command lines. There were six passes and one platform skip in each package test run.

Linux x86-64 では ShellSpec の 68 件、fumiyas の 16 件、unixwin の 50 件、luish の 7 本がすべて Bash より短い所要時間でした。Mac は、手元の M1 と macOS 15 の CI で傾向が異なります。CI の ShellSpec では外部の `cut` や `sed` を使う処理で遅れが残りました。GNU Hello の構成全体は手元で 30.15 秒対 30.66 秒と近く、Linux CI では 7.27 秒対 6.15 秒、macOS CI では 24.15 秒対 22.90 秒と Bash の方が速い結果でした。

再現には下のコマンドをプロジェクト直下で実行します。比較する二つのシェルには絶対パスを渡します。ShellSpec は毎秒の処理回数、fumiyas は準備を除いた処理時間、それ以外はプロセスの起動を含む経過時間を記録します。短い測定では実行順やホストの違いでも順位が動くため、個別の値と再測定結果も残しています。[計測コード](../tools/bench_external.py)

```sh
for suite in shellspec fumiyas unixwin luish gnu-hello; do
  bash tools/ci/external-shell.sh "$suite" "$tonari" "$bash"
done
```

<a id="tool-hooks"></a>

## 2026-11-25 · Interactive tool latency / 日常の開発操作

Each cell gives **previous Tonari / current Tonari / Bash**, in median milliseconds. Previous is `ef5a286`. Two warmups preceded the rotating trials, with each shell staged under its own `bash` basename and a disposable home. Measurements end at visible terminal output. Node versions, the applied and removed directory environment, the fzf selection after editing and VS Code's command and exit-status messages were also verified. The combined rc activates nvm, bash-completion, Git, gh, fzf, direnv and mise, then VS Code wraps the prompt. [Benchmark](../tools/bench_tool_hooks.py) · [Native Linux results and p95](https://github.com/sanohiro/tonari-shell/actions/runs/45500490573)

| Operation | Apple M1 | Linux x86-64 | Linux arm64 |
|---|---:|---:|---:|
| All integrations, exec to first prompt | 372.384 / 366.542 / 457.426 | 144.401 / 141.328 / 207.749 | 185.504 / 180.651 / 264.302 |
| All integrations, key to echo | 0.031 / 0.033 / 0.033 | 0.052 / 0.050 / 0.048 | 0.058 / 0.061 / 0.057 |
| All integrations, `false` to next prompt | 33.599 / 32.791 / 35.677 | 12.670 / 12.314 / 15.220 | 16.806 / 16.165 / 19.419 |
| All integrations, enter project | 92.793 / 86.061 / 91.982 | 33.611 / 33.110 / 36.261 | 38.900 / 37.957 / 42.153 |
| All integrations, leave project | 34.376 / 32.596 / 37.298 | 13.767 / 13.474 / 16.538 | 17.860 / 17.481 / 20.873 |
| nvm, activate installed default Node | 252.543 / 234.779 / 308.192 | 101.313 / 101.218 / 154.282 | 131.047 / 130.560 / 194.596 |
| nvm, switch installed Node | 162.725 / 161.452 / 185.429 | 63.751 / 64.698 / 89.319 | 84.539 / 82.808 / 116.906 |
| Git, source completion to first prompt | 6.800 / 6.632 / 12.298 | 4.809 / 4.764 / 7.122 | 4.548 / 4.538 / 7.144 |
| Git, first branch Tab with 1,000 distractor branches | 56.804 / 57.521 / 60.446 | 10.401 / 10.228 / 13.791 | 11.509 / 11.666 / 15.654 |
| Git, repeat branch Tab | 57.384 / 57.290 / 61.091 | 10.271 / 10.206 / 13.715 | 11.835 / 11.699 / 16.207 |
| GitHub CLI and bash-completion, first prompt | 7.665 / 7.133 / 12.325 | 6.125 / 6.181 / 9.627 | 5.912 / 5.924 / 10.064 |
| GitHub CLI, first `gh iss<Tab>` | 34.975 / 34.383 / 35.950 | 33.768 / 34.018 / 35.231 | 32.460 / 32.675 / 34.227 |
| GitHub CLI, repeat Tab | 33.454 / 34.086 / 35.860 | 33.859 / 33.842 / 35.804 | 32.704 / 32.705 / 34.640 |
| direnv, enter and load .envrc | 59.729 / 58.231 / 59.891 | 22.153 / 21.796 / 23.079 | 22.442 / 22.141 / 23.232 |
| direnv, leave project | 9.536 / 8.639 / 10.835 | 2.822 / 2.695 / 3.416 | 2.944 / 2.632 / 3.630 |
| direnv, unchanged directory | 9.237 / 8.495 / 10.433 | 2.457 / 2.244 / 3.114 | 2.568 / 2.319 / 3.284 |
| mise, activate to first prompt | 30.758 / 29.286 / 34.858 | 12.187 / 12.422 / 15.227 | 16.709 / 16.472 / 19.192 |
| mise, enter project | 13.780 / 14.435 / 14.329 | 6.351 / 6.463 / 6.918 | 9.640 / 9.580 / 9.996 |
| mise, leave project | 13.644 / 14.081 / 13.908 | 6.061 / 6.058 / 6.308 | 8.988 / 8.840 / 9.470 |
| mise, unchanged directory | 13.496 / 13.466 / 13.529 | 5.370 / 5.310 / 5.667 | 7.951 / 7.958 / 8.271 |
| VS Code integration, first prompt | 16.166 / 13.550 / 22.414 | 5.952 / 5.213 / 9.345 | 6.842 / 5.923 / 10.963 |
| VS Code, `false` to next prompt with exit status | 5.941 / 5.871 / 7.443 | 2.087 / 2.094 / 3.150 | 2.514 / 2.503 / 3.986 |
| fzf Ctrl-T, display selected file among 1,025 | 28.242 / 27.248 / 30.602 | 19.955 / 19.808 / 20.874 | 20.653 / 20.561 / 21.964 |
| fzf Ctrl-T, return quoted file to editor | 1.778 / 1.862 / 2.408 | 1.078 / 1.079 / 1.432 | 1.238 / 1.271 / 1.529 |
| Explicit import of 100,001 history entries | — / 5.069 / 12.661 | — / 8.784 / 15.926 | — / 9.767 / 18.071 |
| fzf installed, first prompt with saved history | — / 19.164 / 40.626 | — / 10.204 / 29.592 | — / 10.380 / 34.371 |
| fzf Ctrl-R, first candidate from saved history | — / 42.589 / 44.038 | — / 32.311 / 35.760 | — / 32.745 / 37.898 |
| fzf Ctrl-R, return selected command to editor | — / 30.714 / 60.649 | — / 25.975 / 92.611 | — / 27.348 / 73.992 |

The macOS 27.2.1 run used Bash 5.2.37, Apple Git 2.54.0, gh 2.96.0, nvm 0.40.3, Node 24.1.0 and 18.13.0, direnv 2.38.2, mise 2026.11.14, fzf 0.74.0 and VS Code 1.144.0. Linux used Ubuntu 24.04's Bash 5.2.21, and the same completion source and VS Code integration script. The harness applies a project-local variable through mise and direnv. Node installs, remote gh completions and graphical rendering time fall outside these timings. macOS used 15 trials per shell for each individual tool, 17 for the combined rc, 9 for Ctrl-T and 41 for Ctrl-R against saved history. Linux used 15. Both Linux builds passed the stripped static PIE checks. The Mac executable links only to libSystem.

The macOS binary grew from 1,433,360 to 1,433,392 bytes. Linux x86-64 grew from 1,917,304 to 1,927,384 and Linux arm64 from 1,658,088 to 1,667,104. No new runtime dependencies were added. The M1 Bash differential audit retained all 1,411 previous verdicts. The release passed the 204-case shell suite with two platform skips on macOS and one on each Linux runner.

With all integrations loaded, p95 key echo for current Tonari / Bash was 0.087 / 0.178 ms on M1, 0.067 / 0.064 ms on Linux x86-64, 0.089 / 0.068 ms on Linux arm64. The old Ctrl-R widget could not produce a valid editable selection. In the first working M1 implementation importing 100,001 entries took 25.678 ms, compared with 5.069 ms after batching. A separate 41-trial M1 run measured p95 45.887 ms from Ctrl-R to a candidate and 34.219 ms from selection to the editable line.

A diagnostic trace of four M1 Git Tabs saw four Git processes per Tab, including two overlapping remote-reference queries. Each gh Tab launched one `gh __complete iss`. Those four subprocesses took 29–33 ms apiece in the instrumented run. The shell calls the installed helpers on each Tab so another process can change branches and configuration. A separate control that launched the upstream executables directly took 34 ms for gh, 15 ms for mise hook-env and 11 ms for direnv export on this Mac. These controls used independent runs and include process launch. Their times cannot be subtracted from the PTY medians. Several full interactions remain above two thirds of Bash time.

The Linux x86-64 mise directory-entry sample had a 6.46 ms median and 108.50 ms p95 for Tonari, and a 6.92 ms median and 119.12 ms p95 for Bash. A separate native run gave 4.87 / 5.19 ms for Tonari and 5.02 / 5.43 ms for Bash. The long tail did not reproduce in both runs. [Independent run](https://github.com/sanohiro/tonari-shell/actions/runs/45500299066)

On M1, a follow-up ran the stock nvm 0.40.3 startup script with default Node v24.1.0 and no nvm-managed Node on the inherited PATH. The first command was `node --version`. The harness checked both the output and the selected executable path. Times below are median / p95 in milliseconds from the same 15 interleaved trials per shell.

| Operation | Tonari | Bash 5.2.37 |
|---|---:|---:|
| Shell launch to first prompt | 205.54 / 211.41 | 271.93 / 287.68 |
| Shell launch through first `node --version` and next prompt | 223.70 / 229.59 | 291.04 / 307.15 |
| Subsequent `nvm use --silent` between installed versions | 139.56 / 146.78 | 161.97 / 170.74 |

In a separate startup trace, PATH wrappers around common command-line utilities counted 56 invocations in each shell before the first prompt. These included eight each of `dirname`, `awk`, `tail` and `sed`, seven of `grep`, six of `tr`, five each of `head` and `uname`, and one of `which`. The timed trials ran the utilities directly.

```sh
PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin python3 tools/bench_tool_hooks.py \
  "$tonari" "$bash" --case nvm --nvm-script "$NVM_DIR/nvm.sh" --nodes "$node24" "$node18"
```

7 ツールを同時に読み込む条件でも、キーを押して表示されるまでと、Enter を押して次のプロンプトに戻るまでを別々に計測しました。Git と gh は実際の Tab、fzf は候補が描画された時点と選択後に編集へ戻った時点を測っています。履歴の検証では選んだ行に追記して実行し、前のセッションの履歴も対象にしました。VS Code はインストール済みの統合スクリプトでコマンドと終了値を照合し、Mac の実アプリでも自動認識を確認しました。

On macOS, resolve the integration script using `code --locate-shell-integration-path bash`. Supply paths to an installed bash-completion, Git's Bash completion and two installed Node prefixes. The Linux runner downloads the pinned inputs and verifies their digests. [Fixture setup](../tools/ci/tool-hooks.sh)

```sh
python3 tools/bench_tool_hooks.py "$tonari" "$previous" "$bash" --case combined \
  --vscode-script "$(code --locate-shell-integration-path bash)" --nvm-script "$NVM_DIR/nvm.sh" \
  --nodes "$node24" "$node18" --git-completion "$git_completion" --bash-completion "$bash_completion"
python3 tools/bench_tool_hooks.py "$tonari" "$bash" --case fzf --entries 100000 --saved-history
```

<a id="eval-rc"></a>

## 2026-11-25 · Generated startup scripts and eval / 初期化スクリプトと eval

Each cell gives **previous Tonari / current Tonari / Bash** in median milliseconds. Previous is `3713253`. Startup measures exec through the first visible interactive prompt with an isolated home and five warmups, rotating shells for 41 measured launches each. The second table times verified scripts in an already running shell, with 31 repetitions on Apple M1 and 25 in CI. All executables ran natively. The local M1 used macOS 27.2.1 and Bash 5.2.37. The Ubuntu 24.04 runners used the static musl release and Bash 5.2.21. [Linux job logs, p95 and binary inspection](https://github.com/sanohiro/tonari-shell/actions/runs/45495710662)

| .bashrc workload | Apple M1 | Linux x86-64 | Linux arm64 |
|---|---:|---:|---:|
| Empty | 3.699 / 3.664 / 7.493 | 1.126 / 1.106 / 1.865 | 1.559 / 1.544 / 2.401 |
| Light configuration | 3.415 / 3.413 / 6.639 | 1.143 / 1.121 / 1.896 | 1.610 / 1.620 / 2.535 |
| Git completion, 94,826 bytes | 6.187 / 6.236 / 11.035 | 3.762 / 3.535 / 5.398 | 4.444 / 4.301 / 6.741 |
| 760-line function, directly in file | 3.793 / 3.567 / 8.085 | 1.536 / 1.506 / 3.025 | 2.142 / 2.101 / 4.172 |
| Same function through `eval` | 78.909 / 4.225 / 7.777 | 104.048 / 1.938 / 3.418 | 136.790 / 2.510 / 4.388 |
| 300 nested conditionals through `eval` | 107.941 / 4.232 / 7.295 | 154.110 / 2.210 / 2.993 | 206.250 / 2.723 / 3.675 |
| Nested conditionals plus a literal `if` in a here-document | 115.643 / 4.501 / 8.145 | 154.030 / 2.292 / 3.021 | 206.949 / 2.732 / 3.712 |
| 4,000 foreground builtins | 4.826 / 4.433 / 12.910 | 2.847 / 2.302 / 7.364 | 2.887 / 2.684 / 9.644 |

| Running-shell workload (2,000 loop iterations) | Apple M1 | Linux x86-64 | Linux arm64 |
|---|---:|---:|---:|
| Fixed expression | 1.454 / 1.060 / 7.692 | 1.123 / 0.865 / 5.750 | 1.243 / 0.912 / 7.775 |
| Generated literal assignment | 2.337 / 2.004 / 7.017 | 1.996 / 1.664 / 5.329 | 1.864 / 1.394 / 7.214 |
| Multiple arguments with `--` | 1.615 / 1.272 / 9.082 | 1.364 / 1.092 / 6.602 | 1.289 / 0.951 / 8.824 |
| Alternating expressions | 3.303 / 2.444 / 14.429 | 2.611 / 1.957 / 10.074 | 2.807 / 2.026 / 13.654 |
| Nested `eval` | 3.011 / 2.074 / 13.622 | 2.210 / 1.553 / 9.314 | 2.452 / 1.585 / 12.535 |
| Multiline text | 4.030 / 3.154 / 10.594 | 3.172 / 2.416 / 7.980 | 3.178 / 2.440 / 10.663 |
| Function definition and invocation | 4.095 / 2.565 / 12.200 | 3.548 / 2.132 / 9.872 | 3.651 / 2.174 / 13.257 |
| Changing aliases | 4.111 / 4.191 / 15.945 | 3.536 / 3.532 / 12.620 | 3.413 / 3.514 / 16.975 |

The long startup function contains 760 padding statements following an early return. Sourcing and invoking the function checks the registration. Reading the same file with `eval "$(<file)"` also measures capture and parsing. The 300-condition fixture executes the body of each inner `if`. The quoted variant puts an extra literal `if` in a here-document before the outer terminator. All three shells also load the same Git completion source. The light local fixture defines a directory helper and the CI fixture defines a lazy version-manager wrapper.

On the M1 an 8 KiB lookahead alone took 78.989 ms for the large generated function. Looking through the full command took 4.257 ms in the same 41-sample experiment. This change alone still spent 31.538 ms on 300 inner conditionals. Skipping inner terminators reduced that cost. A prototype that failed to reconsider the last deferred terminator took 88.482 ms with the literal here-document versus 4.201 ms after validation. In Git completion, treating every line ending in an opening brace as another shell block pushed startup to 90.951 ms on M1. Restricting recognition to shell headers restored the timing in the table.

Borrowing a cached syntax tree instead of cloning it reduced fixed-expression throughput from 1.161 to 1.025 ms and nested eval from 2.001 to 1.782 ms in an interleaved M1 comparison. Distinct literal assignments measured 1.915 and 1.925 ms. Preserving parse diagnostics and active alias changes requires reparsing those inputs. For foreground builtins, delaying construction of literal job labels reduced the 4,000-call M1 profile from 4.804 to 4.408 ms against the build that already deferred terminal-state capture.

Previously Ubuntu’s 76,898-byte bash-completion 2.11-8 script stopped while defining `_parse_help`. The release handles the continued regex operand and registers `_parse_help`, `_parse_usage` and `_completion_loader`. The startup fixture requires all three. Searching CDPATH lazily and looking up a directory variable after ordinary directory candidates reduced a mixed 400-iteration `cdable_vars` workload from 21.746 to 20.818 ms on M1, versus Bash at 66.707 ms.

On M1 the real Starship 1.27.0 Git prompt took 64.910 / 68.171 / 78.981 ms to first paint (previous / current / Bash). Enter took 39.005 / 38.241 / 42.884 ms and key echo took 0.033 / 0.032 / 0.033 ms. Evaluating its actual precmd function definition 100 times took 10.999 / 6.481 / 20.069 ms. Sourcing pregenerated Starship code and painting the real Git prompt took 50.960 / 49.937 / 58.717 ms, with p95 at 54.111 / 55.591 / 66.622 ms. A dedicated 101-session first-prompt repeat gave 67.357 / 66.551 / 78.058 ms, with p95 of 73.310 / 75.017 / 86.798 ms. The old and new Tonari medians changed order between the two series. The complete Starship launch shows no consistent improvement over the previous release.

The nested first prompt took 73.8% of Bash time on Linux x86-64 and 74.1% on Linux arm64. The upper shell-work target is 66.7%. Eight evaluations in an already running shell took 3.830 ms on x86-64 and 5.584 ms on arm64, versus Bash at 5.980 and 8.164 ms (64.0% and 68.4%). We also tried storing the first arm of an `if` inline. Eight evaluations of the 300-condition block took 3.574 → 3.576 ms on M1, 4.543 → 4.540 ms on Linux x86-64 and 5.653 → 5.494 ms on Linux arm64. We retained the existing representation. The Ubuntu bash-completion first prompt took 5.429 ms on M1, 4.466 ms on Linux x86-64 and 4.891 ms on Linux arm64. [Native conditional-storage experiment](https://github.com/sanohiro/tonari-shell/actions/runs/45494566940)

The shipped runtime is `0d774f0`. Executable sizes changed from 1,416,832 to 1,433,360 bytes on macOS arm64, 1,907,128 to 1,917,304 bytes on Linux x86-64 and 1,650,000 to 1,658,088 bytes on Linux arm64. The signed Mach-O links only libSystem. Both ELF files are stripped static PIE. The macOS Bash 5.2.37 comparison covers 1,411 audit entries. The directory-variable option moved from missing to a behavior match and every other verdict was unchanged. The 203-test integration suite passed on the Mac and both Ubuntu 24.04 CI architectures, and in arm64 Debian 13 (Bash 5.2.37) and Ubuntu 26.04 (Bash 5.3.9) containers. Platform-specific tests were skipped on other operating systems. The shell suite covers temporary bindings, aliases, line numbers, EOF, heredocs, continued regex patterns, declarations, control flow, job labels and terminal handoff. A separate 38-case comparison checked exit status, stdout, diagnostic presence and warning count. [Native release verification](https://github.com/sanohiro/tonari-shell/actions/runs/45495710662)

Reproduce interactive timings with `python3 tools/bench_login.py CURRENT --previous OLD --bash BASH --rcfile FILE --runs 41`. Use `python3 tools/bench_scripts.py CURRENT --previous OLD --bash BASH --timing started --runs 31 --case eval-fixed --case eval-changing --case eval-nested` for throughput. The native workflow’s `rcfiles` suite defines the remaining fixture files, runs each case and records release linkage and size. For the upstream hook use `python3 tools/bench_starship.py OLD CURRENT BASH --starship /path/to/starship --runs 21`. [Fixtures and CI commands](../tools/ci/native-bench.sh)

表は旧 Tonari、新 Tonari、Bash の順の中央値（ms）です。上段は端末の初回表示まで、下段は起動済みシェルでの反復を測定しました。大きな関数を直接読む場合と、同じ関数を文字列として評価する場合を分けています。実際の Git 補完と Ubuntu の bash-completion は、関数が登録されたことまで確かめています。[開発知見](engineering-notes.md)

<a id="starship-phases"></a>

## 2026-11-25 · Where the official Starship hook spends time / 公式フックの所要時間の内訳

Previous Tonari is `4ff9ac3`, current Tonari is `86bead7`. All shells ran the upstream Starship 1.27.0 Bash integration with a clean home, a clean repository of 200 tracked files, a 120 × 32 PTY, and the default theme. The full integration checks success and failure prompts, pipeline status, job count and command duration. The replay fixture runs the same initialization and prompt hooks and launches the same number of subprocesses. A small compiled executable returns the official initialization text, samples the actual clock for each timestamp and returns captured PS1/PS2 bytes for a successful colon command. The harness verifies the replayed prompt values. The difference between genuine and replay timings includes executable loading, Starship initialization, rendering and Git work. Replay still includes process launch, the clock call, pipes and terminal I/O. Run `python3 tools/bench_starship.py OLD CURRENT BASH --starship /path/to/starship --runs 25`. [Native comparisons and artifacts](https://github.com/sanohiro/tonari-shell/actions/runs/45482196177)

### Release builds through the terminal

Each cell reports the median/p95 in milliseconds. Shell order rotated over 21 measured sessions on the local M1 and 25 on each native CI host, with three Enter samples per session and two warmups. macOS used Bash 5.2.37, Linux Bash 5.2.21. Compare releases within one host and run.

| Native host | Workload | Previous Tonari | Current Tonari | Bash |
|---|---|---:|---:|---:|
| Apple M1 (local) | Genuine, first | 57.308/60.653 | 55.651/66.794 | 64.964/69.837 |
| Apple M1 (local) | Replay, first | 14.707/16.467 | 13.614/15.315 | 22.812/25.102 |
| Apple M1 (local) | Genuine, Enter | 33.800/36.558 | 32.640/37.764 | 36.853/39.348 |
| Apple M1 (local) | Replay, Enter | 5.880/7.140 | 5.087/6.062 | 8.687/10.154 |
| Linux x86-64 | Genuine, first | 14.584/15.771 | 13.038/15.294 | 16.192/16.936 |
| Linux x86-64 | Replay, first | 4.969/5.509 | 3.446/3.597 | 6.444/6.688 |
| Linux x86-64 | Genuine, Enter | 8.613/9.329 | 8.511/9.476 | 9.799/10.770 |
| Linux x86-64 | Replay, Enter | 1.262/1.477 | 1.222/1.298 | 2.443/2.676 |
| Linux arm64 | Genuine, first | 16.076/16.546 | 14.876/15.577 | 18.253/18.888 |
| Linux arm64 | Replay, first | 6.159/6.356 | 4.738/4.962 | 8.308/8.771 |
| Linux arm64 | Genuine, Enter | 9.195/10.010 | 9.080/9.739 | 10.730/11.379 |
| Linux arm64 | Replay, Enter | 1.801/1.884 | 1.547/1.648 | 3.304/3.431 |
| macOS arm64 (CI) | Genuine, first | 86.374/183.232 | 95.112/149.061 | 115.194/260.850 |
| macOS arm64 (CI) | Replay, first | 24.173/42.161 | 21.379/32.723 | 35.831/58.541 |
| macOS arm64 (CI) | Genuine, Enter | 57.772/105.948 | 55.818/114.244 | 64.325/138.191 |
| macOS arm64 (CI) | Replay, Enter | 11.706/37.983 | 8.508/14.745 | 13.633/25.593 |

The first M1 series had a higher startup and redisplay p95 in current Tonari than in the previous build despite lower medians. A dedicated repeat rotated the two releases and Bash over 101 fresh sessions each (303 Enter submissions). First-prompt median/p95 was 63.155/79.172 ms previously, 61.822/71.041 ms currently and 72.197/81.502 ms in Bash. Enter measured 37.138/43.989, 36.202/43.970 and 40.334/47.573 ms respectively. The median gain repeated and the previous and current Enter p95 were within 0.02 ms in this larger sample. The hosted macOS arm64 runner showed wider variation. Genuine startup took 86.374 ms previously and 95.112 ms currently, with p95 falling from 183.232 to 149.061 ms. Its genuine startup median therefore did not reproduce the local improvement. Its replay startup and genuine Enter medians improved.

The following elapsed times include shell startup. The first workload repeatedly evaluates the exact multiline upstream precmd definition and checks its presence. The second captures a shell function whose body is a single external printf. Both check the result. The raw artifacts also contain individual keystroke echo, saved initialization, long-prompt and array-expansion timings.

| Native host | Workload | Previous Tonari | Current Tonari | Bash |
|---|---|---:|---:|---:|
| Apple M1 (local) | 100 precmd definitions | 56.606/58.239 | 9.989/10.772 | 18.687/19.345 |
| Apple M1 (local) | 100 wrapper captures | 230.407/264.564 | 154.601/225.389 | 246.571/296.936 |
| Linux x86-64 | 100 precmd definitions | 63.601/65.651 | 5.963/6.074 | 12.493/13.068 |
| Linux x86-64 | 100 wrapper captures | 44.363/46.291 | 34.400/37.022 | 57.479/58.840 |
| Linux arm64 | 100 precmd definitions | 61.126/61.383 | 6.057/6.249 | 12.255/12.435 |
| Linux arm64 | 100 wrapper captures | 65.568/68.224 | 52.037/54.602 | 79.927/83.613 |
| macOS arm64 (CI) | 100 precmd definitions | 79.505/98.110 | 12.726/23.409 | 23.022/29.449 |
| macOS arm64 (CI) | 100 wrapper captures | 441.562/742.840 | 397.543/612.595 | 490.429/766.340 |

A second M1 run rotated four combinations of shell and child implementation in one series (31 measured startup sessions and 93 Enter submissions per combination). With the genuine binary, Enter took 32.022 ms in Tonari and 36.048 ms in Bash. With replay it took 5.026 and 8.660 ms. Switching to the genuine binary added 27.0 and 27.4 ms respectively. Initial display took 54.545 vs 64.514 ms with Starship and 13.522 vs 23.112 ms with replay. Both experiments include child launch and wait. In a reference check with the same release executable, standalone `starship time` took 7.812 ms from Python including launch and output collection, `prompt` took 20.599 ms, and Starship’s own `timings` reported 11 ms in `git_status` on the clean fixture. The executable initializes its logger and Rayon pool before dispatching the time command. A separate 61-sample check gave 7.44 ms at the upstream default and 7.32 ms with one worker. Changing the thread count explained little of the standalone latency on this machine. [Starship startup](https://github.com/starship/starship/blob/v1.27.0/src/main.rs#L161-L175) · [Time command](https://github.com/starship/starship/blob/v1.27.0/src/main.rs#L270-L277)

The release executables had the following sizes in bytes. Apple M1 1,400,320 → 1,416,832, Linux x86-64 1,903,800 → 1,907,128, Linux arm64 1,645,496 → 1,650,000, macOS arm64 (CI) 1,400,272 → 1,433,312. The local Mach-O gained one 16 KiB page in the `__DATA_CONST` segment and 128 bytes in `__LINKEDIT`. Its `__text` section grew by 4,032 bytes within an unchanged `__TEXT` segment size. The only linked library was libSystem. The Linux binaries passed the stripped static PIE checks.

M1 の配布版で Enter から表示完了までを測ると、実際の Starship は Tonari 32.64 ms、Bash 36.85 ms、同じ表示と起動回数の対照は 5.09 ms と 8.69 ms でした。実際の Starship に戻すと、それぞれに約 28 ms が加わります。Linux x86-64 の対照は 1.22 ms と 2.44 ms、実際の表示は 8.51 ms と 9.80 ms です。シェルが稼ぐ時間は実際のフックにも現れます。初回パース、関数実行、引数展開のそれぞれが Bash の 2/3 に達したかは、下の個別計測で判定できます。

### Time inside the two shells on an Apple M1

Separate diagnostic builds of GNU Bash 5.2.37 and Tonari on macOS 27.2.1 recorded parent-side monotonic times at the same two visible boundaries. Shell order rotated over 31 fresh sessions per shell after two warmups. Each session submitted three colon commands. Rows are medians in milliseconds. Timed functions subtract nested instrumented intervals. Parser entry counts refer to each implementation's own parser. Bash's timed parser handles string input and excludes the blocking Readline call. The launch row measures time in the parent handing work to the OS. Bash returns from fork before its child execs. Tonari's direct path uses `posix_spawn`. Pipe drain and child wait can therefore include different amounts of child setup. Each median is taken independently.

| Phase | Previous Tonari | Current Tonari | Bash |
|---|---:|---:|---:|
| First prompt, open and read rcfile (µs, open + read) | 21 + 2 | 21 + 3 | 17 + 1 |
| First prompt, parser (entries) | 1.379 (159) | 0.290 (22) | 0.265 (55) |
| First prompt, register four functions | 0.001 | 0.001 | 0.011 |
| First prompt, prepare command arguments | 0.449 | 0.441 | 0.552 |
| First prompt, function execution excluding measured nested work | 0.107 | 0.110 | 0.125 |
| First prompt, parent launch | 1.659 | 1.598 | 2.253 |
| First prompt, pipe drain and child wait | 53.293 | 52.561 | 58.560 |
| Enter, parser | 0.010 | 0.008 | 0.026 |
| Enter, prepare command arguments | 0.113 | 0.113 | 0.245 |
| Enter, function execution excluding measured nested work | 0.124 | 0.108 | 0.136 |
| Enter, pipe drain and child wait | 33.984 | 33.391 | 36.361 |
| Enter, time outside the three external substitutions | 0.479 | 0.442 | 1.553 |

With only the string-reader change in place, a separate four-way M1 comparison measured the additional wrapper optimization. Enter went from 35.412 to 34.639 ms with the genuine executable and from 6.498 to 5.590 ms with the lightweight control. At initial display the time calls already dispatched directly. [Native run with the parser change alone](https://github.com/sanohiro/tonari-shell/actions/runs/45481626738)

Initial display executes two `init` commands (13.558 ms combined in current Tonari, 15.405 in Bash), two `time` commands (13.346 vs 15.211 ms), and the continuation and main prompts (27.437 vs 29.718 ms). Enter has three external calls. The following wall times start on entry to the outer substitution and end when the shell has its output and status. They overlap the phases above. On Enter, previous Tonari handed off two direct launches and one fork, current Tonari three direct launches (0.803 ms combined), and Bash four forks (1.410 ms, including its jobs substitution).

| Enter, external substitution | Previous Tonari | Current Tonari | Bash |
|---|---:|---:|---:|
| Start time via the PS0 function | 7.641 | 6.862 | 7.703 |
| End time | 6.746 | 6.695 | 7.658 |
| Render prompt | 20.352 | 20.444 | 21.504 |

A separate M1 control tested executable startup for the two timestamp calls. The same tiny C program called `clock_gettime(CLOCK_REALTIME)` and printed milliseconds. Compiled with `cc -O2`, its median/p95 was 2.898/3.213 ms for launch, output and wait from Python. Adding only `-Wl,-needed_framework,AppKit` gave 5.744/6.471 ms. Linking every direct system dependency listed by `otool -L` on the published Starship executable gave 5.849/6.571 ms. The real `starship time` measured 7.221/7.792 ms in the same rotation (81 samples each after three warmups). The clock code and printed data were identical across the three control builds. This isolates nearly 3 ms of extra launch latency in the control with the matching framework set. The remaining difference includes Starship's code, its executable layout and its startup environment.

前回の 98.8% は Tonari が外部コマンド置換に入ってから戻るまでの割合でした。両シェルの内訳を見ると、起動時には Tonari のパースに余分な時間があり、Enter 後には開始時刻を返す関数の起動にも差がありました。時刻を取る前後で同じ実行ファイルを 2 回起動し、さらに Git を含むプロンプトを描画します。ホスト型 macOS CI の初回表示中央値は旧版 86.37 ms、修正版 95.11 ms、Bash 115.19 ms で、ローカル M1 で見られた旧版からの短縮は再現しませんでした。各コマンドの計測値には親の準備、OS による起動、子の処理、出力回収と待機が含まれます。対照実験ではフックと起動回数を保ち、外部プログラムの処理を軽くしたときの応答時間も実際に比較します。[公式フック](https://github.com/starship/starship/blob/v1.27.0/src/init/starship.bash)

<a id="starship-filter"></a>

## 2026-11-25 · Remaining Starship prompt latency / Starship の再表示待ち時間

The same upstream Starship 1.27.0 binary ran for previous Tonari (`65d6acf`), current Tonari (`2f61716`) and Bash. All used the same rcfile, a 120 × 32 PTY and a clean Git repository with 200 tracked files. The main comparison used the default theme. An additional test used the upstream directory, branch and character modules plus 256 or 1,024 literal ASCII characters to vary the work performed on the full prompt string. Compare shells within each row. Medians/p95 are milliseconds. Shell order rotated on each iteration, with two warmups, 31 local or 25 hosted runs for the standard theme and three submissions per session. The longer prompts used 21 local or 15 hosted runs. macOS used Bash 5.2.37 and Linux used Bash 5.2.21. [Standard native runs](https://github.com/sanohiro/tonari-shell/actions/runs/45475684304) · [Long prompts on native Linux](https://github.com/sanohiro/tonari-shell/actions/runs/45476695004)

| Host | Measurement | Previous Tonari | Current Tonari | Bash |
|---|---|---:|---:|---:|
| Local Apple M1 | Default, first prompt | 61.504/67.183 | 61.414/66.975 | 68.789/74.914 |
| Local Apple M1 | Default, typed character | 0.030/0.062 | 0.030/0.052 | 0.027/0.068 |
| Local Apple M1 | Default, next prompt | 36.526/40.809 | 35.977/40.370 | 38.731/44.314 |
| Local Apple M1 | Saved init, first prompt | 45.694/51.228 | 45.464/48.327 | 53.740/59.319 |
| Local Apple M1 | +256 characters, next prompt | 31.934/37.920 | 23.355/27.211 | 27.511/34.374 |
| Local Apple M1 | +1024 characters, next prompt | 116.164/123.294 | 20.550/23.785 | 31.643/35.414 |
| Linux x86-64 | Default, first prompt | 12.481/12.789 | 12.005/12.328 | 13.534/13.884 |
| Linux x86-64 | Default, typed character | 0.042/0.071 | 0.042/0.068 | 0.042/0.061 |
| Linux x86-64 | Default, next prompt | 7.363/7.738 | 6.738/7.303 | 7.869/8.329 |
| Linux x86-64 | Saved init, first prompt | 9.521/9.876 | 9.041/9.520 | 11.168/11.581 |
| Linux x86-64 | +256 characters, next prompt | 12.135/12.313 | 3.678/3.832 | 5.667/5.825 |
| Linux x86-64 | +1024 characters, next prompt | 118.069/118.992 | 3.827/4.036 | 14.221/14.531 |
| Linux arm64 | Default, first prompt | 17.888/19.013 | 17.598/18.200 | 20.490/21.167 |
| Linux arm64 | Default, typed character | 0.056/0.066 | 0.056/0.070 | 0.059/0.081 |
| Linux arm64 | Default, next prompt | 10.466/11.173 | 10.043/10.700 | 11.817/12.463 |
| Linux arm64 | Saved init, first prompt | 14.112/14.704 | 13.650/14.402 | 16.982/17.425 |
| Linux arm64 | +256 characters, next prompt | 11.922/12.264 | 6.183/6.477 | 9.322/9.668 |
| Linux arm64 | +1024 characters, next prompt | 97.601/99.494 | 6.113/7.431 | 20.318/22.249 |
| macOS arm64 CI | Default, first prompt | 75.614/125.325 | 77.376/130.042 | 91.263/140.823 |
| macOS arm64 CI | Default, typed character | 0.053/0.098 | 0.047/0.127 | 0.046/0.116 |
| macOS arm64 CI | Default, next prompt | 46.398/106.407 | 45.803/91.946 | 50.282/99.968 |
| macOS arm64 CI | Saved init, first prompt | 52.343/126.691 | 50.595/103.823 | 60.638/125.833 |

The standard-theme first prompt was effectively flat on the local M1. On the hosted Mac its median rose from 75.61 to 77.38 ms with wide p95 variation. The function capture batch improved on both Linux architectures and was close to Bash on the hosted Mac.

The shell batches below include starting the shell. The filter handles a 256-byte UTF-8 string followed by a newline, repeated 24 times. The wrapper returns the result of `/usr/bin/printf ok` through command substitution 100 times. Both batches check output.

| Host | Batch | Previous Tonari | Current Tonari | Bash |
|---|---|---:|---:|---:|
| Local Apple M1 | Bracket deletion ×24 | 46.239/50.053 | 4.828/5.821 | 12.292/14.114 |
| Local Apple M1 | Function captures ×100 | 245.040/268.599 | 241.343/304.066 | 257.408/286.589 |
| Linux x86-64 | Bracket deletion ×24 | 40.869/41.005 | 0.524/0.555 | 5.862/5.934 |
| Linux x86-64 | Function captures ×100 | 49.859/50.331 | 41.932/42.695 | 53.379/53.896 |
| Linux arm64 | Bracket deletion ×24 | 28.446/28.653 | 0.874/0.921 | 8.498/8.678 |
| Linux arm64 | Function captures ×100 | 84.773/88.088 | 73.540/76.055 | 92.123/95.043 |
| macOS arm64 CI | Bracket deletion ×24 | 63.361/71.658 | 7.129/10.026 | 15.062/22.077 |
| macOS arm64 CI | Function captures ×100 | 361.172/519.692 | 371.800/574.469 | 371.705/451.631 |

On an Apple M1 an instrumented release timed each top-level command substitution from entry to return and marked each primary editor prompt. This includes the launch, capture, wait and any function dispatch inside that substitution. The external time helper ran twice and the renderer once per submission. For 21 warm Git sessions with three submissions each, the median time outside those three calls was 0.397 ms. Their median share of the full PTY interval was 98.788%. These measurements describe that machine and theme. The release benchmark above uses the uninstrumented binary.

With the same current shell, a separate 25-run local startup comparison rotated all three configurations across both shells. The default `eval "$(starship init bash)"` measured 61.19 ms on Tonari and 68.97 ms on Bash. Requesting the full hook directly with `eval "$(starship init bash --print-full-init)"` took 53.75 ms and 62.00 ms respectively. Sourcing the exact upstream code generated in advance took 44.43 ms and 54.33 ms. Regenerate a saved script after upgrading or moving the executable. Each configuration also passed the status, duration and job-count checks.

標準設定では、Enter 後の大半は時刻取得2回とプロンプト描画1回の呼び出しに費やされます。長いプロンプトでは、描画後に Tonari が行う改行抽出にも大きな費用がありました。長さを変えた実験では同じ Starship 設定を3つのシェルに渡しています。初回表示では `--print-full-init` を付けるだけでも起動を1回省けます。この効果は Bash にもあります。

Release sizes in bytes, previous → current.

- Local Apple M1 · 1,400,320 → 1,400,320
- Linux x86-64 · 1,902,648 → 1,903,800
- Linux arm64 · 1,644,344 → 1,645,496
- macOS arm64 CI · 1,400,272 → 1,400,272

Reproduce both terminal comparisons with native release executables.

```sh
python3 tools/bench_starship.py /path/to/previous /path/to/current /path/to/bash --starship /path/to/starship --runs 25
python3 tools/bench_starship.py /path/to/previous /path/to/current /path/to/bash --starship /path/to/starship --runs 15 --long-prompts-only
```

<a id="starship"></a>

## 2026-11-25 · Interactive Starship prompts / Starship 利用時の対話性能

Starship 1.27.0, upstream executable and default configuration. Each shell reads `eval "$(starship init bash)"` from the same isolated rcfile. Previous Tonari is `6756278`, current is `37e275b`. The local Apple M1 ran macOS 27.2.1. The native macOS hosts used GNU Bash 5.2.37 and the Ubuntu 24.04 hosts used GNU Bash 5.2.21. The PTY is 120 columns by 32 rows, with a temporary home, disabled history file and a PATH containing only the fixture tools and `/usr/bin:/bin`. The clean Git fixture contains 200 tracked files on branch `prompt-bench`. Timings include warm filesystem caches, a fresh shell for every first prompt and three submissions of `:` per session. The benchmark rotates shell order, discards two warmups and takes 31 samples locally or 25 on each CI runner. [Native run and host manifests](https://github.com/sanohiro/tonari-shell/actions/runs/45422420218)

The first-prompt clock starts immediately before exec and stops at the visible prompt character. The next-prompt clock starts at Enter and waits for a fresh character after the terminal leaves input mode. All timing cells give median/p95 in milliseconds.

| Native host | Directory | Operation | Previous Tonari | Current Tonari | Bash |
|---|---|---|---:|---:|---:|
| Apple M1, local | Empty | first prompt | 48.188/51.798 | 46.280/50.557 | 55.215/59.016 |
| Apple M1, local | Empty | next prompt | 24.533/26.025 | 22.685/24.521 | 25.588/27.621 |
| Apple M1, local | Git | first prompt | 61.612/98.481 | 60.047/74.092 | 68.962/91.062 |
| Apple M1, local | Git | next prompt | 37.412/50.560 | 36.225/47.444 | 38.302/52.495 |
| macOS arm64, CI | Empty | first prompt | 76.418/94.251 | 65.196/110.472 | 77.106/108.219 |
| macOS arm64, CI | Empty | next prompt | 38.658/67.424 | 30.679/59.039 | 33.836/65.341 |
| macOS arm64, CI | Git | first prompt | 90.753/161.214 | 82.284/149.559 | 82.579/146.793 |
| macOS arm64, CI | Git | next prompt | 51.007/95.887 | 48.541/104.365 | 52.696/114.336 |
| Linux x86-64, CI | Empty | first prompt | 10.316/10.591 | 9.946/10.264 | 11.215/11.471 |
| Linux x86-64, CI | Empty | next prompt | 4.992/5.178 | 4.684/4.827 | 5.425/5.595 |
| Linux x86-64, CI | Git | first prompt | 13.339/13.799 | 12.963/13.277 | 13.918/14.271 |
| Linux x86-64, CI | Git | next prompt | 7.895/8.485 | 7.510/7.911 | 8.115/8.576 |
| Linux arm64, CI | Empty | first prompt | 13.211/13.750 | 12.659/13.273 | 14.547/15.166 |
| Linux arm64, CI | Empty | next prompt | 6.408/6.713 | 5.926/6.141 | 7.187/7.586 |
| Linux arm64, CI | Git | first prompt | 17.017/17.625 | 16.537/17.010 | 18.425/18.864 |
| Linux arm64, CI | Git | next prompt | 10.166/10.725 | 9.756/10.372 | 10.873/11.600 |

With the Git prompt visible, one typed ASCII character at the end of an empty line produced the following response. The byte counts include the terminal control sequences.

| Native host | Previous ms | Current ms | Bash ms | Bytes before → after / Bash |
|---|---:|---:|---:|---:|
| Apple M1, local | 0.031/0.054 | 0.031/0.046 | 0.030/0.059 | 80 → 1 / 1 |
| macOS arm64, CI | 0.049/0.109 | 0.049/0.138 | 0.046/0.143 | 80 → 1 / 1 |
| Linux x86-64, CI | 0.043/0.072 | 0.042/0.068 | 0.042/0.064 | 80 → 1 / 1 |
| Linux arm64, CI | 0.056/0.078 | 0.053/0.068 | 0.057/0.072 | 80 → 1 / 1 |

Each batch starts one shell and repeatedly executes ordinary shell syntax. The first repeats an empty `$(jobs -p)`. The second captures `/usr/bin/printf` with a quoted array containing a space, an empty argument and a literal wildcard. The third expands a UTF-8 prompt with repeated ANSI width markers through `${p@P}`. Each measured batch checks its final output.

| Native host | Batch | Previous Tonari | Current Tonari | Bash |
|---|---|---:|---:|---:|
| Apple M1, local | 200 empty jobs captures | 120.910/134.922 | 4.409/5.226 | 157.249/169.613 |
| Apple M1, local | 200 external array captures | 462.346/489.224 | 316.673/383.075 | 487.308/539.707 |
| Apple M1, local | 2000 styled prompt expansions | 6.258/6.866 | 5.912/6.535 | 16.210/16.901 |
| macOS arm64, CI | 200 empty jobs captures | 174.844/391.690 | 6.232/8.880 | 213.083/478.146 |
| macOS arm64, CI | 200 external array captures | 792.074/1090.149 | 672.941/871.365 | 799.466/1243.355 |
| macOS arm64, CI | 2000 styled prompt expansions | 7.448/12.506 | 7.334/11.064 | 22.804/32.323 |
| Linux x86-64, CI | 200 empty jobs captures | 25.306/25.863 | 0.463/0.542 | 47.874/48.553 |
| Linux x86-64, CI | 200 external array captures | 89.227/90.242 | 69.594/70.624 | 111.478/113.716 |
| Linux x86-64, CI | 2000 styled prompt expansions | 1.920/1.967 | 1.609/1.665 | 9.278/9.365 |
| Linux arm64, CI | 200 empty jobs captures | 33.712/34.705 | 0.723/0.791 | 73.461/75.297 |
| Linux arm64, CI | 200 external array captures | 134.881/137.808 | 107.757/111.463 | 164.744/168.134 |
| Linux arm64, CI | 2000 styled prompt expansions | 2.462/2.536 | 2.022/2.051 | 11.315/11.431 |

Standalone Apple M1 medians (31 warm samples, Python subprocess launch through exit) were 7.5 ms for `starship time`, 9.1 ms for `starship prompt` in an empty directory and 19.9 ms for the renderer in the Git fixture. The official hook calls the time helper and runs the renderer on each submission. The complete prompt still takes more than two thirds of Bash time. The editor emits one byte for a plain append to the Git prompt. An active suggestion or a mid-line edit can require a redraw. On the hosted Mac the Git first-prompt medians were within 0.3 ms of one another, with p95 around 150 ms. The 200 external captures also exceeded the two-thirds target there. Without Starship, a separate 51-run local control measured exec to first prompt at 3.074 → 3.085 ms (Bash 6.090 ms).

単位はミリ秒で、各セルは中央値/p95 です。Starship は公式配布版の標準設定で測りました。起動から初回表示までと Enter 後の再表示には、時刻取得や Git 状態を調べる外部処理の待ち時間も含みます。入力は端末に文字が届いたところまでを測定しました。成功・失敗の色、パイプ各段の終了状態、実行時間、バックグラウンドジョブ数も、同じ疑似端末で確認します。Mac の CI では Git での初回表示は Bash とほぼ同じで、p95 のばらつきも大きくなりました。

Release sizes in bytes (previous → current).

- Apple M1, local · 1,400,304 → 1,400,320
- macOS arm64, CI · 1,400,272 → 1,400,272
- Linux x86-64, CI · 1,900,504 → 1,902,648
- Linux arm64, CI · 1,641,848 → 1,644,344

Reproduce with release binaries and the Starship 1.27.0 executable for the native architecture.

```sh
python3 tools/bench_starship.py /path/to/previous /path/to/current /path/to/bash --starship /path/to/starship --runs 25
```

<a id="edit-actions"></a>

## 2026-11-24 · Per-line undo and last-argument recall / 取り消しと末尾引数の再利用

Median time from input to a visible response in a real PTY, in milliseconds. Each cell gives the first implementation (`5216862`), optimized release (`c8dc765`), and Bash. The long history entry has 4,096 ordinary arguments before a single-quoted final token. The long draft is a recalled variable assignment with 1 MiB of padding. The driver pastes a short suffix into that line, sends Ctrl-_ followed by a marker, waits for the marker, and checks the length and final value in the shell. Each shell starts in a fresh PTY per trial. The executable order rotates and the first two trials are discarded. Local M1 used 31 runs, native CI used 21. Ubuntu used Bash 5.2.21 and macOS used Bash 5.2.37.

| Operation | Local M1 | Linux x86-64 | Linux arm64 | macOS CI arm64 |
| --- | ---: | ---: | ---: | ---: |
| Alt-. short | 0.025 / 0.026 / 0.022 | 0.036 / 0.036 / 0.036 | 0.037 / 0.037 / 0.037 | 0.037 / 0.036 / 0.032 |
| Alt-. many | 0.287 / 0.055 / 0.363 | 0.636 / 0.057 / 0.365 | 0.661 / 0.062 / 0.415 | 0.306 / 0.068 / 0.323 |
| Paste into short line | 0.037 / 0.038 / 0.060 | 0.041 / 0.042 / 0.062 | 0.046 / 0.046 / 0.078 | 0.056 / 0.059 / 0.088 |
| Undo + type into short line | 0.029 / 0.027 / 0.021 | 0.035 / 0.036 / 0.036 | 0.033 / 0.033 / 0.035 | 0.028 / 0.030 / 0.028 |
| Paste into 1 MiB line | 0.233 / 0.144 / 7.605 | 0.428 / 0.059 / 4.717 | 0.402 / 0.079 / 6.422 | 0.449 / 0.074 / 8.088 |
| Undo + type into 1 MiB line | 0.097 / 0.036 / 14.974 | 0.415 / 0.042 / 9.280 | 0.441 / 0.054 / 12.502 | 0.318 / 0.060 / 14.956 |

When the driver and shell share one Linux CPU, the optimized median/p95 and Bash median/p95 over 15 measured trials were as follows.

| Operation | Linux x86-64, 1 CPU | Linux arm64, 1 CPU |
| --- | ---: | ---: |
| Alt-. short | 0.023/0.438 · 0.021/0.318 | 0.051/0.052 · 0.050/0.060 |
| Alt-. many | 0.086/0.467 · 0.371/0.717 | 0.077/0.094 · 0.457/0.497 |
| Paste into short line | 0.032/0.062 · 0.057/0.269 | 0.051/0.067 · 0.097/0.154 |
| Undo + type into short line | 0.026/0.069 · 0.025/0.069 | 0.044/0.060 · 0.053/0.075 |
| Paste into 1 MiB line | 0.091/0.640 · 4.753/4.836 | 0.081/0.109 · 6.418/6.460 |
| Undo + type into 1 MiB line | 0.065/0.550 · 9.319/9.415 | 0.081/0.107 · 12.508/12.579 |

Undo now stores only the range inserted and the bytes removed. This avoids copying a megabyte to make a short edit undoable. Bare history words and a simple single-quoted tail can be sliced from the original command. Complex syntax still goes through the lexer. Short-line recall and undo are generally in the same tens-of-microseconds range as Bash. The large cases all meet the two-thirds target.

With approximately 64 KiB of existing UTF-8 input, ordinary append on the M1 measured 0.027 ms before and after the feature. In five sessions with 128 individually acknowledged edits after warm-up, repeated append p99 was 0.026 ms before and 0.021 ms after, and near-head insertion p99 was 0.051 and 0.046 ms. Insert near the head went from 0.073 to 0.075 ms, a Japanese text commit near the head from 0.066 to 0.069 ms, and a multiline paste near the head from 0.089 to 0.098 ms (15-run medians). The shipping executable is 1,400,304 bytes on macOS arm64, 1,900,504 on Linux x86-64, and 1,641,848 on Linux arm64. The Mac executable grew by 16,528 bytes. It links libSystem. Both Linux builds passed the stripped static PIE checks. [Native run and p95 measurements](https://github.com/sanohiro/tonari-shell/actions/runs/45417604845)

表の数値は初回実装・差分を保存する採用版・Bash の順です。1 MiB の行を計測前に履歴から呼び出し、短い文字列の貼り付け、取り消し直後の一文字が見えるまでを個別に測りました。実行した結果で行の長さと末尾も検証しています。

```sh
python3 tools/bench_edit_actions.py /path/to/first /path/to/final /path/to/bash --runs 21
python3 tools/bench_editing.py /path/to/prior /path/to/final /path/to/bash --size 65536 --runs 15 --case append --case 'insert near head' --case 'IME at head'
```

<a id="navigation"></a>

## 2026-11-24 · Prefix history arrows and directory navigation / 前方一致履歴とディレクトリ移動

Median milliseconds from a keypress to visible output in a PTY. Each cell lists the first working release (`de46191`), the optimized release (`896c2c0`), and Bash where it has an equivalent binding. The 100,000-line active history contains a matching assignment at either end. After recalling the distant match, the driver reverses direction twice and verifies the executed value. The directory case revisits two logical paths below twelve parent directories and waits for the new directory name in the prompt. The Bash comparison uses Readline’s prefix-search bindings. The directory keys are a Tonari extension.

| Operation | Mac M1 | Linux x86-64 | Linux arm64 | macOS CI arm64 |
| --- | ---: | ---: | ---: | ---: |
| Recent match | 0.032 / 0.027 / 0.017 | 0.063 / 0.059 / 0.057 | 0.045 / 0.041 / 0.040 | 0.059 / 0.049 / 0.036 |
| Distant match | 0.266 / 0.253 / 0.517 | 0.504 / 0.419 / 0.981 | 0.330 / 0.336 / 0.741 | 0.336 / 0.328 / 0.823 |
| Forward revisit | 0.254 / 0.017 / 0.473 | 0.465 / 0.055 / 0.925 | 0.318 / 0.046 / 0.667 | 0.308 / 0.031 / 0.656 |
| Backward revisit | 0.246 / 0.018 / 0.476 | 0.478 / 0.051 / 0.939 | 0.311 / 0.034 / 0.650 | 0.283 / 0.024 / 0.689 |
| Back to directory | 0.079 / 0.064 | 0.100 / 0.084 | 0.072 / 0.065 | 0.092 / 0.089 |
| Forward to directory | 0.054 / 0.045 | 0.085 / 0.066 | 0.048 / 0.041 | 0.066 / 0.075 |

Both the shell and the PTY reader were also pinned to the same Linux CPU. Each cell gives the optimized release’s median/p95, followed by the Bash median/p95 when applicable, in milliseconds.

| Operation | Linux x86-64, 1 CPU | Linux arm64, 1 CPU |
| --- | ---: | ---: |
| Recent match | 0.056/0.093 · 0.056/0.061 | 0.053/0.067 · 0.052/0.056 |
| Distant match | 0.415/0.499 · 0.960/1.050 | 0.339/0.369 · 0.730/0.783 |
| Forward revisit | 0.052/0.058 · 0.889/1.136 | 0.052/0.075 · 0.681/0.737 |
| Backward revisit | 0.048/0.069 · 0.911/0.994 | 0.042/0.050 · 0.678/0.690 |
| Back to directory | 0.084/0.134 | 0.064/0.083 |
| Forward to directory | 0.064/0.098 | 0.070/0.084 |

On the local M1 with **one million** entries, the first lookup of the distant match took 3.199/4.132 ms median/p95 (Bash 5.607/6.535). Moving forward then back over the same gap took 0.042/0.063 and 0.025/0.066 ms. Caching visited indices removes the repeated scan. On one CPU an earlier build could keep the terminal waiting about half a millisecond for the newest hit while it looked for an optional longer suggestion. The editor now gives the terminal up to one millisecond to paint the result or supply the next key before starting that optional scan. Reusing the resolved path and reading available escape bytes directly improved both directory directions on the M1 and the Linux runners. On macOS CI the forward direction varied from 0.066 to 0.075 ms. For the newest history match, the optimized build takes 0.001–0.013 ms longer than Bash across the unrestricted runs. The distant lookup meets the two-thirds target on all four hosts.

Local M1 used 31 measured trials (11 at one million rows), CI used 21, and single-CPU CI used 15. The driver rotates executable order and discards the first two trials. macOS used GNU Bash 5.2.37, Ubuntu 24.04 used 5.2.21. The shipping sizes are 1,383,776 bytes on the local Mac, 1,892,856 on Linux x86-64, and 1,633,992 on Linux arm64. macOS links libSystem. Linux releases are stripped static PIEs. [Native run](https://github.com/sanohiro/tonari-shell/actions/runs/45413127931)

10万件の表は初回実装・採用版・Bash の順に中央値を示します。離れた候補を初めて探すときだけ履歴を走査し、往復時は以前見つけた位置を再利用します。空行のディレクトリ移動は保存済みパスを直接使います。入力後の計測から、シェルの起動・履歴の読み込み・ディレクトリの準備を除きました。

```sh
python3 tools/bench_navigation.py /path/to/first /path/to/final --bash /path/to/bash --runs 21
python3 tools/bench_navigation.py /path/to/first /path/to/final --bash /path/to/bash --entries 1000000 --runs 11
```

<a id="history-fields"></a>

## 2026-11-24 · Alt-F and multi-fragment Ctrl-R / 履歴候補の部分採用と複数語検索

Each cell gives median/p95 milliseconds from a keystroke to visible PTY output on a local Mac M1 and two native Linux CI runners, first working implementation (`03f6714`) → final release (`d643c88`). For each binary the benchmark uses three warm-ups and 21 measured fresh shells with a million command records. It alternates binaries, verifies the accepted argument, and times echo separately from the first matching row. The long Alt-F case has 4,096 trailing words. The old match is the first row of the journal. The typeahead case replaces a Japanese search term after the first hit appears for a query with only one matching row. [Native results and host details](https://github.com/sanohiro/tonari-shell/actions/runs/45406685611) · [Benchmark driver](../tools/bench_history_fields.py)

| Workload | Local M1 | Linux x86-64 | Linux arm64 |
| --- | ---: | ---: | ---: |
| Alt-F, short | 0.049/0.058 → 0.035/0.046 | 0.087/0.457 → 0.091/0.406 | 0.072/0.081 → 0.067/0.072 |
| Alt-F, long | 0.295/0.334 → 0.050/0.061 | 0.796/1.017 → 0.298/0.417 | 0.633/0.738 → 0.073/0.088 |
| recent, query echo | 0.031/0.039 → 0.033/0.076 | 0.043/0.051 → 0.047/0.087 | 0.047/0.060 → 0.047/0.053 |
| recent, selected row visible | 133.019/144.445 → 0.120/0.162 | 105.708/106.816 → 0.058/0.094 | 83.232/84.097 → 0.078/0.085 |
| old, query echo | 0.027/0.059 → 0.031/0.075 | 0.047/0.074 → 0.046/0.050 | 0.046/0.048 → 0.045/0.047 |
| old, selected row visible | 194.702/205.510 → 7.354/9.721 | 120.494/121.369 → 6.104/6.256 | 102.745/103.560 → 9.575/10.003 |
| typeahead, query echo | 0.065/0.098 → 0.052/0.078 | 0.092/0.393 → 0.059/0.089 | 0.089/0.097 → 0.072/0.075 |
| typeahead, selected row visible | 0.179/0.202 → 0.073/0.106 | 0.142/0.409 → 0.065/0.092 | 0.130/0.139 → 0.080/0.085 |

Linux also ran the PTY driver and the shell together under the same single-CPU affinity mask. The following cells are final-build median/p95 milliseconds over 15 trials.

| One CPU | Linux x86-64 | Linux arm64 |
| --- | ---: | ---: |
| old, query echo | 2.410/3.293 | 0.059/0.061 |
| old, selected row visible | 6.365/6.811 | 9.412/10.012 |
| typeahead, query echo | 0.074/0.210 | 0.152/0.182 |
| typeahead, selected row visible | 0.117/0.217 | 0.191/0.226 |

手元の M1 では100万件の最古の候補を表示する時間が初回 194.702/205.510 ms、最終版 7.354/9.721 ms（中央値/p95）でした。候補を探している間に打ち直した日本語の表示は 0.052/0.078 ms でした。このため全件を読み込む実装から、先に検索語を描いて必要な候補数だけ履歴を逆向きに読む実装へ切り替えました。今回の配布バイナリは手元の M1 で 1,383,744 バイト（前版 1,367,216）、ネイティブ CI の Linux x86-64・Linux arm64 でそれぞれ 1,884,888, 1,628,832 バイトです。

A literal single-term Ctrl-R lookup through 100,000 rows also ran alongside GNU Bash (31 trials on the local M1 and 21 on Linux). Tonari/Bash median milliseconds were 0.215/0.537 on the local M1, 0.113/1.183 on Linux x86-64, and 0.145/1.015 on Linux arm64. The feature comparison uses Tonari's first implementation for the multi-fragment list and partial autosuggestion. The Mac audit retained all 1,408 previous verdicts. The PTY suite also compares ordinary Alt-F word motion with Bash.

Ordinary UTF-8 commit medians on the local M1 were 0.039 ms for one scalar and 0.430 ms for 257, compared with 0.042 and 0.431 ms in the release before these features. In the same 31-trial run Bash took 0.052 and 1.254 ms. On a single logical CPU the Python PTY reader shares its scheduling time with the shell. The query-echo column includes that scheduling delay.

Reproduce the feature timings with `python3 tools/bench_history_fields.py FIRST CURRENT --entries 1000000 --runs 21` and the Bash control with `python3 tools/bench_editor_input.py CURRENT BASH --case Ctrl-R --runs 21`.

<a id="completion-picker"></a>

## 2026-11-24 · Filtering Tab completions and ranking learned arguments / 補完候補の絞り込みと使用頻度

Each cell shows median/p95 milliseconds, first working picker (`e616cea`) → page-limited search and grouped history (`6102a03`). A fresh interactive shell receives real keys through a PTY. Trials use 20,000 files or 2,001 successful journal records, three warm-ups and 31 interleaved samples per binary. The driver verifies the selected argument after Enter returns it to the command line. “Many matches” types `item`; the next edit targets the lexicographically last filename. The distinct-history fixture adds a unique tag to every recorded command. [Native CI and raw results](https://github.com/sanohiro/tonari-shell/actions/runs/45401078101)

| Operation | macOS arm64 | Linux x86-64 | Linux arm64 |
| --- | ---: | ---: | ---: |
| Open file choices | 11.349/17.936 → 12.192/19.525 | 9.160/9.765 → 8.527/8.636 | 8.600/8.695 → 8.535/8.612 |
| Filter, many matches | 0.290/1.104 → 0.052/0.172 | 0.205/0.250 → 0.044/0.064 | 0.504/0.527 → 0.052/0.062 |
| Filter, late match | 0.829/2.539 → 0.125/0.397 | 0.648/0.673 → 0.132/0.187 | 0.823/0.862 → 0.274/0.293 |
| Repeated history, first Tab | 5.528/13.435 → 1.540/4.454 | 2.440/2.465 → 0.318/0.334 | 2.687/2.735 → 0.394/0.416 |
| Repeated history, open choices | 5.523/12.939 → 1.595/2.982 | 2.399/2.424 → 0.279/0.300 | 2.652/2.687 → 0.351/0.391 |
| Distinct history, first Tab | 6.171/14.566 → 5.790/14.520 | 2.694/2.712 → 2.359/2.417 | 2.959/2.989 → 2.398/2.443 |
| Distinct history, open choices | 6.180/13.212 → 5.802/12.640 | 2.663/2.683 → 2.235/2.256 | 2.923/2.952 → 2.277/2.345 |

画面に収まる候補まで探し、次のページに進んだ時点で続きを探します。候補末尾の一件だけが一致する場合も計測しました。履歴は重複コマンドが多い場合と、すべて異なる行で同じ引数を使う場合を分けています。初回版と高速化版は同じ候補を同じ順で表示し、最終的にコマンドに渡った引数まで確認しています。

A second comparison isolated the cost of ordering filenames on open. The final build (`0430a2b`) uses packed prefix keys for large filename lists on all three targets. Each comparison below interleaves two binaries on one host. The 10,000-name listing uses the final build and the host's GNU Bash. [Native comparison, release sizes and Bash versions](https://github.com/sanohiro/tonari-shell/actions/runs/45401825828)

| Operation, median/p95 ms | macOS arm64 | Linux x86-64 | Linux arm64 |
| --- | ---: | ---: | ---: |
| Open 20,000 names, before → after sorting change | 10.639/11.901 → 10.347/11.685 | 8.590/8.835 → 6.913/7.242 | 8.562/8.697 → 6.285/6.421 |
| Filter many, final | 0.052/0.073 | 0.044/0.058 | 0.054/0.074 |
| Filter to last name, final | 0.114/0.157 | 0.125/0.251 | 0.283/0.369 |
| Print 10 names, Tonari / Bash medians | 0.190 / 0.223 | 0.063 / 0.070 | 0.080 / 0.089 |
| Print 10,000 names, Tonari / Bash medians | 13.890 / 63.529 | 5.739 / 23.341 | 5.692 / 31.731 |
| Final executable, bytes | 1,367,168 | 1,862,712 | 1,605,824 |

手元の M1 でも、初回実装と最終版を31回交互に計測しました。中央値は、2万ファイルの一覧を開くまでが 9.212 → 8.678 ms、数多く一致する部分入力が 0.196 → 0.030 ms、末尾にある一件への絞り込みが 0.623 → 0.095 ms でした。成功履歴2,001件から最初の Tab が応答するまでは、重複が多い場合で 4.572 → 0.815 ms、全行が異なる場合で 4.860 → 4.141 ms でした。ローカル配布版は機能追加前の 1,350,672 バイトから 1,367,216 バイトになりました。リンク先は引き続き libSystem 一つで、Linux は両アーキテクチャとも静的 PIE です。Mac の既存の Bash 互換判定1,408件を前回と照合し、変化はありませんでした。

To reproduce with two release executables, run `python3 tools/bench_picker.py FIRST CURRENT --files 20000 --runs 31`. For the full listing shared with Bash, run `python3 tools/bench_completion.py CURRENT --bash BASH --listing --runs 21`. The latter types Alt-? in Tonari and a second Tab in Bash, verifies every name, and includes the echo of the following keystroke. The filtered picker itself is a Tonari extension. [Picker benchmark](../tools/bench_picker.py) · [Listing benchmark](../tools/bench_completion.py)

<a id="required-glob-extension"></a>

## 2026-11-24 · Required filename extensions in the release build / 必須拡張子による候補の絞り込み

Each cell gives **previous Tonari → current Tonari / Bash**. Release binaries at `baa07c6` and `4c78f6d` ran on the same machine and filesystem. Each row used five warm-ups and 31 interleaved measurements per shell after a readiness handshake. The directory contained 20,000 names with fixed-width numbers and an ASCII, Japanese or emoji character. On the local M1, the complete NUL-delimited lists from both builds and Bash matched in all five cases. Native CI also compared Bash's full argument list for case folding, extended patterns, quoted metacharacters, directory components and hidden files. [Benchmark cases](../tools/bench_scripts.py) · [Release regression test](../tests/shell.py) · [All native timings](native-2026-11-24/required-extension.tsv)

| 20,000 entries, median ms | Local M1 | macOS arm64 CI | Linux x86-64 CI | Linux arm64 CI |
| --- | ---: | ---: | ---: | ---: |
| `entry-?????-?.log` · 5% | 11.422 → **9.258** / 26.252 | 11.714 → **9.492** / 27.433 | 4.703 → **3.798** / 8.418 | 5.466 → **4.370** / 10.805 |
| `entry-?????-?.zip` · 0% | 10.994 → **8.725** / 22.256 | 12.419 → **9.677** / 25.282 | 4.381 → **3.457** / 8.019 | 5.274 → **4.136** / 10.374 |
| `entry-?????-?.txt` · 95% | 19.399 → **19.309** / 130.572 | 20.758 → **20.971** / 147.640 | 8.448 → **8.231** / 17.002 | 8.407 → **8.388** / 20.641 |
| `*.log` · 5% | 9.268 → **9.135** / 25.617 | 9.378 → **9.380** / 26.547 | 3.861 → **3.878** / 8.051 | 4.409 → **4.406** / 10.306 |
| `entry-?????-?.*` · 100% | 19.185 → **19.250** / 137.084 | 20.853 → **20.610** / 159.910 | 8.829 → **8.496** / 17.360 | 8.616 → **8.607** / 21.120 |

2万件のうち1,000件または0件が一致するケースのほか、ほぼ全件が一致する場合、単純な星印だけの場合、固定拡張子が存在しない場合も同じ方法で測りました。各 OS の UTF-8 ロケールで比較し、Linux は x86-64 と arm64 のネイティブランナーを使用しました。 [実機 CI とバイナリ情報](https://github.com/sanohiro/tonari-shell/actions/runs/45381485844) · [Linux 互換性監査](https://github.com/sanohiro/tonari-shell/actions/runs/45381518013)

Across the 72 script workloads, all three native CI runners recorded lower medians than Bash. Tonari took at most two-thirds of Bash time in 58/72 on macOS arm64, 63/72 on Linux x86-64 and 62/72 on Linux arm64. The 95%-matching pattern was 0.213 ms (1.0%) slower than the previous build on the hosted Mac.

The Mac release executable measured 1,350,672 bytes locally, exactly the previous size. The native Linux x86-64 release was 1,838,008 bytes (+544) and arm64 was 1,584,808 bytes (+384), both stripped static PIE executables. The current and previous feature-inventory verdicts agreed in all 7,558 platform/case observations (macOS 1,408, Debian 1,522 and Ubuntu 1,553 on each Linux architecture). All five large fixtures also produced the same ordered NUL-delimited results as Bash in functional checks on both Linux targets.

8ファイルだけのディレクトリでも、手元の M1 で256回の展開を一組として31回測定しました。全件一致する `x?.txt` は前版6.535 ms、今回6.460 ms、一致しない `x?.log` は5.676 → 5.656 ms、単純な `*.txt` は6.352 → 6.350 msでした。

<a id="ripgrep-16-trials"></a>

## 2026-11-24 · ripgrep 16 transfer experiments / ripgrep 16 の手法を使った試作

On a native Apple M1, build both the control at `7342e88` and each separate experimental change with `tools/build-release.sh --host-toolchain` (Homebrew rustc 1.96.1, arm64). These timings compare experimental builds. They are separate from the distribution-build timings below. For the glob experiment, create 20,000 files named `entry-{i:05}-{mark}.{ext}`, cycling through `a`, `雪` and `🐟`. Use `log` every 20th entry and `txt` elsewhere. Run `shopt -s nullglob; set -- fixture/PATTERN; echo "$#"` after the ready marker under `en_US.UTF-8`. Rotate the two Tonari builds and Bash 5.2.37 for three warm-ups and 15 samples each. Repeat with the Tonari order reversed. Check the complete NUL-separated path list against Bash before timing each workload. [Script timer](../tools/bench_scripts.py) · [Upstream GlobSet strategies](https://github.com/BurntSushi/ripgrep/blob/16.0.0/crates/globset/src/glob.rs#L51-L70)

| Pattern | Matches | Current / prototype / Bash, median ms | Repeat, current / prototype ms |
| --- | ---: | ---: | ---: |
| `entry-?????-?.log` | 1,000 | 11.450 / 9.341 / 25.912 | 11.542 / 9.302 |
| `entry-?????-?.txt` | 19,000 | 18.830 / 18.757 / 118.761 | 18.663 / 18.404 |
| `entry-?????-?.zip` | 0 | 11.171 / 8.780 / 22.196 | 11.106 / 8.692 |
| `*.log` | 1,000 | 9.311 / 9.197 / 25.309 | 9.304 / 9.205 |
| `entry-?????-?.*` | 20,000 | 18.696 / 18.286 / 124.250 | 18.527 / 18.920 |

試作では、大文字・小文字を区別する一般の照合器に渡す前に、パターンが要求する英数字の拡張子を一度取り出し、末尾が違う候補を除外しました。一致が5%以下の2条件で短縮が再現しました。通常の `*.log` はすでに末尾を直接比較します。ドットファイル、引用、文字クラス、拡張パターンなどを組み合わせた別の37条件でも、試作・現行・Bash の出力を照合しました。2つの Tonari 実験ビルドはともに1,566,800バイトでした。Linux 実機での速度は今回の計測範囲外です。

A separate Ctrl-R experiment accepted a plain substring hit directly when it lay in the unescaped command field of a valid history record. On 100,000 mixed entries, the oldest Japanese match took 1.368 → 1.374 ms (p95 1.624 → 1.587 ms). Swapping execution order gave 1.386 → 1.332 ms (p95 1.565 → 1.522 ms). For a missing query through Enter, medians were 1.187 → 1.141 ms and 1.134 → 1.157 ms. Each run rotated 41 fresh PTYs per variant and checked the executed command after recall. These comparisons did not establish a repeatable latency reduction. [PTY workload](../tools/bench_editor_input.py) · [Engineering notes](engineering-notes.md)

Ctrl-R の試作では、履歴の一致箇所がコマンド欄内で、エスケープを含まない場合に再照合を省きました。入力から表示までの時間を測ると、10万件の日本語検索では初回と再計測で中央値の差の符号が変わりました。

<a id="native-glob-launch-io"></a>

## 2026-11-24 · Large globs, short-lived processes and bulk input / 大量展開・外部起動・一括読み込み

On a local Apple M1, tests alternated the new build, `be82bcf` and GNU Bash 5.2.37 on the same filesystem. Values are median milliseconds. Interactive timings end at the verified output of the 16th external command. Script timings start at a readiness handshake. The glob fixture uses a literal directory prefix and six-digit Japanese filenames. The mapfile fixture contains exactly 16 MiB. Each named script ran 17 measured trials. The final foreground build ran 81 PTY trials per shell. [Script driver](../tools/bench_scripts.py) · [Foreground driver](../tools/bench_foreground.py)

| Local M1 | Previous | Tonari | Bash |
| --- | ---: | ---: | ---: |
| `set -- numbered/*.txt`, 100,000 files | 291.689 | 87.121 | 799.336 |
| 40,000 unstructured multilingual filenames | 115.436 | 83.240 | 479.585 |
| 4,096 mixed filenames, one expansion | 6.379 | 4.073 | 17.192 |
| 16 foreground launches in a terminal | 25.661 | 17.405 | 26.897 |
| 16 MiB into `mapfile -t`, 4,096 records | 11.327 | 4.761 | 23.706 |

端末内で16個の外部コマンドを実行する計測には、キー送信から結果が疑似端末に届くまでを含みます。設定なしの非対話シェルでは、別の41回の比較で外部起動16回が 16.777 ms、同じ処理を直接書いた C が 16.527 ms でした。2段パイプ16回は 21.654 / 21.431 ms、標準エラーの複製付き16回は 18.240 / 18.010 ms でした。C の対照には起動したプログラム自身の実行と終了待ちを含みます。[C の対照](../tools/bench_spawn.c)

A broader fixture exposed a regression in the initial byte-presort prototype. On 40,000 randomly intermixed Latin, Greek, Cyrillic, Japanese and emoji names in the English locale, it took 99.605 ms versus 66.357 ms previously. The final sample-based choice took **52.905 ms**. In the Japanese locale the three times were 235.863, 141.439 and **100.619 ms**. A trial building full `wcsxfrm` keys took 115.803 ms (English) and 160.528 ms (Japanese) in a separate rotation. The exact filename order matched the previous build and Bash in English, Swedish and Japanese. The CI workload uses the same alphabet and seed, with a literal directory prefix.

連番だけの結果で判断すると、無関係な綴りが混在するディレクトリで遅くなることがわかりました。両者を判別する処理を加え、最速だった連番の経路を維持しつつ、混在する4万件でも前版を上回りました。判別はソート方法だけを選び、最終的な順序は全件を実際のロケールで比較して決めます。

A separate paired mapfile experiment varied buffer size after switching the delimiter scan to memchr. For 16 MiB of 4 KiB records, 8 KiB versus 64 KiB took 5.475 versus 4.653 ms. With 1 MiB records the medians were 5.956 and 5.059 ms. For 65,536 small records they were 3.938 and 3.871 ms. A separate 2,000-line fixture repeated 16 times took 1.208 ms with either buffer.

起動では Mach-O の12シンボルを先頭にまとめるリンク配置を2回比較しました。最初の201回は `-c ':'` が 2.790 → 2.717 ms、順序を逆にした241回は 2.173 → 2.172 ms でした。チェーン形式の dyld 修正情報も試し、201回で 2.176 → 2.193 ms。10万件の履歴を予約済み領域に直接読み込む試作でも、71回のプロンプト表示は 5.191 → 5.251 ms でした。既定のリンク配置と履歴読み込みを維持しました。別の201回の起動から終了までの比較では最小 C 実行ファイルが 2.524 ms、Tonari のバージョン表示が 3.027 ms、`-c ':'` が 3.026 ms でした。これらは温まったファイルキャッシュでの観測です。

On the M1, exec to a configuration-free prompt took 2.689 ms in Tonari, 2.730 ms in the previous build and 5.309 ms in Bash. Starting and exiting with `-c :` took 2.229, 2.264 and 4.522 ms. A separate 101-trial check used a temporary HOME with a copy of the user's 2,052-byte Bash rc file. Exec through its final prompt took 2.837 ms in Tonari, 2.905 ms in the previous build and 5.418 ms in Bash. The generated local arm64 distribution executable is 1,350,672 bytes (+16,640). It passes strict signature verification, contains no DWARF segment and directly links only to libSystem. [Startup driver](../tools/bench_login.py) · [Release checks](../tools/build-release.sh)

通常の外部 `cat` についても、入出力リダイレクトと3段のパイプの全出力を23回ずつ照合しました。64 B のパイプは現行5.760 ms、前版5.753 ms、Bash 8.459 ms。16 MiB では順に12.610 / 12.514 / 15.687 ms。16 MiB のリダイレクトは14.085 / 13.881 / 16.341 msで、今回の読み込み改善はデータを処理する組み込みコマンドで計測されました。[転送と全バイト検証](../tools/bench_streams.py)

| Native medians, ms (previous → Tonari / Bash) | macOS arm64 | Linux x86-64 | Linux arm64 |
| --- | ---: | ---: | ---: |
| 100,000 numbered UTF-8 filenames | 327.446 → **95.283** / 881.375 | 77.028 → **65.337** / 140.970 | 62.438 → **57.295** / 125.448 |
| 40,000 mixed UTF-8 filenames | 136.370 → **135.839** / 551.523 | 38.796 → **33.154** / 71.488 | 28.005 → **21.678** / 60.839 |
| 16 external commands, ready PTY | 34.195 → **29.354** / 37.358 | 9.227 → **9.210** / 11.292 | 9.776 → **9.715** / 11.870 |
| 16 MiB via mapfile | 12.814 → **5.538** / 56.120 | 19.543 → **17.195** / 68.080 | 16.772 → **10.854** / 62.690 |
| Exec to first prompt | 2.669 → **2.694** / 7.120 | 0.709 → **0.716** / 1.479 | 0.859 → **0.867** / 1.570 |

実機 CI では各ランナーで67種類のスクリプトを各31回、シェルの起動を各71回、起動済み端末での外部コマンドを各31回測定しました。各列の前版・今回・Bash は同じランナーで交互に実行したものです。Linux x86-64 の配布形式は strip 済みの静的 PIE で 1,837,464 バイト（前版比 +416 バイト）です。 [全216件の計測結果](native-2026-11-24/glob-launch-io.tsv) · [macOS arm64](https://github.com/sanohiro/tonari-shell/actions/runs/45374234907) · [Linux x86-64](https://github.com/sanohiro/tonari-shell/actions/runs/45373366933/job/138373038689) · [Linux arm64](https://github.com/sanohiro/tonari-shell/actions/runs/45373366933/job/138373038104)

All 67 script medians were faster than Bash on each native runner. The two-thirds target held for 55/67 on macOS, 58/67 on Linux x86-64 and 57/67 on Linux arm64. On the hosted Mac, the mixed 40,000-name expansion was essentially unchanged from the previous version. Interactive launching improved by 14% there, reaching 79% of Bash time, versus 65% of Bash time on the local Mac. The hosted Mac's previous release returned status 1 from `true` in an earlier trial. The benchmark now identifies the offending executable and retries an invalid historical control at most twice. A failure from the current release or Bash fails the run. All foreground samples were valid on the final hosted run. Empty measurements in the TSV are marked `NA`. [Earlier historical-control failure](https://github.com/sanohiro/tonari-shell/actions/runs/45373366933/job/138373038934)

Comparing the feature inventory to the preceding release gave the same verdict for all 1,408 macOS cases, all 1,522 Debian cases on each Linux architecture and all 1,553 Ubuntu x86-64 cases. On Ubuntu arm64, one of 1,553 labels changed from an inconclusive Bash control to the known GNU `run-trap` DEBUG difference, with the same first differing line as the three preceding-release retries. The local suite passed 55 Rust tests and 162 integration tests, with the two Linux-only cases skipped on macOS. [Native Linux compatibility runs](https://github.com/sanohiro/tonari-shell/actions/runs/45374000358) · [Previous trap results](native-2026-11-24/history-parity.md)

Linux 実機で前版と照合した結果、新しい互換性の差は見つかりませんでした。Ubuntu arm64 では、前回 Bash 自身の出力が安定しなかった1件で既知の DEBUG トラップの差を再確認しました。Mac の端末試験では、子が最初に端末を読めること、停止と再開、シグナルマスクの継承も確認しています。

## 2026-11-24 · Reverse-history latency against Bash / Bash との履歴検索比較

In a fresh PTY with 100,000 entries on a native M1 runner, recalling the oldest Japanese match took **0.668/1.357 ms** in Tonari and **1.063/2.034 ms** in Bash (median/p95). The previous Tonari took 1.584/2.685 ms on the same host. Including startup, the medians were 7.272 ms for Tonari and 32.901 ms for Bash. The native comparisons also cover mixed-language journals, misses through Enter, and a million entries. [Results, regressions and release sizes](native-2026-11-24/history-parity.md) · [Command timings](native-2026-11-24/history-parity.tsv)

Mac・Linux x86-64・Linux arm64 の履歴検索は計30条件すべてで中央値・p95 ともに Bash より速く、28条件で中央値が 2/3 以内でした。同じ試行で起動時間と入力応答を記録し、Bash より中央値が遅ければ CI の履歴検証が失敗するようにしました。シェルスクリプトと機能別ベンチの結果も掲載しています。 [実測と再現手順](native-2026-11-24/history-parity.md)

## 2026-11-24 · Matched trials for the previous misses / 速度未達の実装比較

The four native implementation trials cover all 852 previous misses. The fixed batches met the two-thirds target in **214** observations, with **38** more in a 1,024-iteration batch. **472** tried observations remain above target after applying the measured allowances. 前回未達の全行について、変更前後・コンパイラ別・Bash 比と入力から表示までの実測を公開しました。 [Final results / 最終結果](native-2026-11-24/replay.md) · [Per-feature timings](native-2026-11-24/replay.tsv)

<a id="shell-workloads"></a>

## 2026-11-24 · Native shell workloads / シェルの実行時間

Each cell shows **Tonari p50/p95 · Bash p50/p95**, in milliseconds. The production build at `bef7103` ran natively on three CPU/OS combinations. The throughput driver rotates shells, waits for readiness, checks the output and status, then reports 31 timed samples per shell. Startup uses 71 samples and Ctrl-R uses 41. PTY latency ends when the requested result becomes visible. Both Tab cases open a fresh shell per sample. With 10,000 files the timer ends when the unique UTF-8 completion is visible. With 100,000 files it ends when the character queued after Tab is echoed. The completed line is checked on a subsequent redraw. [Native CI](https://github.com/sanohiro/tonari-shell/actions/runs/45320000020) · [All measurements](native-2026-11-24/workload-budget.tar.gz)

| Workload, p50/p95 ms | Linux x86-64 | Linux arm64 | macOS arm64 |
| --- | ---: | ---: | ---: |
| Ctrl-R, 100,000 history entries | 0.480/0.570 · 1.083/1.630 | 0.568/0.624 · 1.014/1.113 | 1.121/1.971 · 0.924/1.685 |
| Tab, unique UTF-8 completion in 10,000 entries | 2.757/2.848 · 2.975/3.001 | 1.857/1.891 · 2.046/2.063 | 3.943/5.860 · 7.951/10.336 |
| Key echo after Tab, 100,000 entries | 0.119/0.142 · 76.483/77.223 | 0.099/0.107 · 57.916/60.087 | 0.226/0.906 · 659.553/902.687 |
| Display all 10,000 completion candidates | 9.140/9.291 · 56.591/60.324 | 6.399/6.512 · 32.350/33.229 | 12.961/14.563 · 66.236/86.970 |
| Paste 16 KiB, packet to visible marker | 3.107/3.629 · 44.824/47.732 | 1.992/2.240 · 38.735/40.332 | 4.987/9.348 · 31.452/42.610 |
| Exec to first prompt | 0.519/0.560 · 1.074/1.128 | 0.835/0.885 · 1.480/1.562 | 2.658/4.667 · 7.085/11.342 |
| Start and exit, `-c :` | 0.173/0.215 · 0.576/0.637 | 0.376/0.437 · 0.874/0.952 | 2.785/4.926 · 8.182/11.364 |
| Parse 2,000 different multiline function bodies | 3.647/3.703 · 5.985/6.088 | 3.744/3.799 · 7.871/7.939 | 3.916/4.596 · 7.704/9.298 |
| Replace one literal function 2,000 times | 0.564/0.584 · 2.130/2.237 | 0.635/0.672 · 2.896/2.931 | 0.724/0.795 · 2.786/3.052 |
| 4,096 UTF-8 names, one expansion | 1.918/1.992 · 2.912/3.037 | 1.660/1.701 · 3.603/3.658 | 10.754/18.832 · 25.209/41.526 |
| Read lines into an array | 2.005/2.037 · 3.105/3.200 | 0.993/1.028 · 4.228/4.314 | 1.476/3.838 · 4.651/10.622 |
| 2,000 integer increments by assignment | 0.695/0.721 · 2.393/2.432 | 0.633/0.666 · 3.275/3.357 | 0.912/1.921 · 3.536/8.294 |
| List all resource limits 128 times | 0.807/0.831 · 1.290/1.334 | 0.830/0.854 · 2.041/2.077 | 0.934/1.694 · 2.608/4.881 |
| Read 2,000 lines in a shell loop | 4.198/4.285 · 6.740/6.923 | 4.873/4.928 · 9.842/9.939 | 7.026/13.709 · 16.069/28.618 |
| 16 external two-stage pipelines | 6.972/7.898 · 9.323/9.582 | 11.821/12.104 · 14.527/14.949 | 42.702/56.151 · 49.119/64.457 |

On Linux x86-64, launching and waiting for 16 external `true` processes took **4.808 / 4.819 / 6.976 ms** for Tonari, the direct C program and Bash respectively. Across the script workloads, the counts above two-thirds of Bash time were Linux x86-64 9/61 · Linux arm64 9/61 · macOS arm64 9/61. The full scripts file identifies which of these workloads have an equivalent C measurement. The feature inventory covers additional workloads. A separate native arm64 trial on Debian measured 128 full limit listings at 1.542 ms before direct label formatting, 1.170 ms after, and 3.152 ms in Bash. Each output byte was checked against the same shell’s untimed result. The C controls are provided for individual process, pipe and directory workloads. Other rows remain open when the measured shell exceeds the target. Stripped release sizes in column order were 1,790,968/1,543,168/1,300,768 bytes. [C workload definitions](../tools/bench_spawn.c) · [All measurements](native-2026-11-24/workload-budget.tar.gz)

各セルは「Tonari の中央値/p95 · Bash の中央値/p95」で単位は ms です。履歴と補完は実際の疑似端末で、起動は exec から最初のプロンプトまで、関数再定義は2,000件を読み込んで最後に定義した関数の結果が返るまでを測りました。外部コマンドには同じ実行ファイルを同じ回数起動する C 版も含めました。反復可能な機能のうち Bash の2/3以内は 1,928/2,780 件でした。未達 852 件の各行には絶対時間も残しています。 [機能別結果](compatibility.md#repeated-workloads) · [All measurements](native-2026-11-24/workload-budget.tar.gz)

On the local M1, reading the oldest of one million records took 6.208/7.241 ms with 128 KiB blocks, 5.074/5.527 ms with 240 KiB and 4.425/5.001 ms with 512 KiB (median/p95). The 512 KiB build increased the native x86-64 Ctrl-R median for 100,000 entries from 0.424 to 0.558 ms. At 240 KiB the same-run comparison was 0.394 to 0.417 ms, and Linux arm64 measured 0.647 to 0.641 ms. On the hosted M1, 240 KiB reduced the same cold 100,000-entry search from 1.881/3.223 to 1.361/2.454 ms. Bash took 0.878/1.878 ms. Tonari uses 240 KiB on all three targets. Starting a fresh PTY session on the local M1 and finding the oldest matching entry in a 100,000-row journal took 6.032/6.656 ms with the 240 KiB build and 22.208/23.173 ms with Bash 5.2.37. This interval includes startup and history loading. [240 KiB trial](https://github.com/sanohiro/tonari-shell/actions/runs/45316981578) · [All measurements](native-2026-11-24/workload-budget.tar.gz)

履歴の読み取り単位は全 CPU 共通の 240 KiB にしました。512 KiB では手元の Mac で古い履歴の検索時間がさらに短くなる一方、Linux 実機では10万件の Ctrl-R が arm64 と x86-64 の両方で遅くなりました。表の Ctrl-R は入力から表示まで、PTY を開くところから測った値にはシェル起動と履歴の読み込みも含みます。 [All measurements](native-2026-11-24/workload-budget.tar.gz)

<a id="function-redefinitions"></a>

## 2026-11-24 · Function definitions and calls / 関数の再定義と呼び出し

Each cell gives **previous Tonari → current Tonari / Bash**, in milliseconds. The driver starts the shell, waits for its ready marker, then times sourcing the script through process exit. It checks every byte of output and the exit status. Each shell ran five warmups and 61 measured samples per workload, rotating the execution order on the same native CI runner. Linux used Bash 5.2.21 and macOS used Bash 5.3.20. The two Tonari revisions are `fc45276` and `5958ee7`. [Workload definitions](../tools/bench_scripts.py) · [Native jobs and host details](https://github.com/sanohiro/tonari-shell/actions/runs/45306401686)

| Workload, median ms | Linux x86-64, Xeon 6975P-C | Linux arm64, Neoverse-N2 | macOS arm64, Apple M1 |
| --- | ---: | ---: | ---: |
| Replace one name with 2,000 different literal bodies, then call the last | 2.654 → **0.730** / 2.104 | 3.314 → **0.975** / 2.917 | 2.189 → **0.805** / 3.108 |
| Replace a quoted `"$@"` wrapper 2,000 times | 2.994 → 1.753 / 2.937 | 3.971 → 2.154 / 4.005 | 2.520 → 1.354 / 4.250 |
| Define 2,000 distinct names | 3.268 → 1.405 / 3.409 | 4.226 → 1.416 / 4.724 | 2.863 → 1.259 / 3.870 |
| Replace 2,000 multiline bodies containing `local` and `if` | 6.711 → 4.941 / 5.874 | 8.797 → 6.382 / 7.881 | 7.331 → 5.221 / 9.532 |
| Execute one parsed compound definition 2,000 times in a loop | 3.231 → 0.816 / 3.045 | 3.906 → 1.162 / 4.072 | 2.374 → 1.053 / 3.891 |
| Call every new literal body and verify all 2,000 results | 7.099 → 4.510 / 7.762 | 9.420 → 6.623 / 11.452 | 7.576 → 4.156 / 17.119 |
| Call one literal function 2,000 times | 3.698 → 3.675 / 6.347 | 4.396 → 4.426 / 9.797 | 4.105 → 4.331 / 13.880 |

For the original redefinition case, Tonari's p95 was 0.760, 1.007 and 1.817 ms in column order. Replacing distinct multiline compound bodies still took 84% and 81% of Bash time on Linux. On the hosted M1 the warmed literal-call median increased by 0.226 ms over 2,000 calls and its p95 changed from 7.996 to 8.011 ms. The stripped CI executables grew by 13,088, 9,192 and 16,544 bytes respectively. The TSV includes whole-script timings, the general function-call control, startup (101 samples), sizes and all p95 values. [All 60 measurements](native-2026-11-24/functions.tsv)

On a separate local Apple M1 with Bash 5.2.37, exec to the first interactive prompt with Git's 94,826-byte completion script took 5.400 → 5.220 / 9.465 ms. With a 760-line function in `.bashrc` it took 3.003 → 2.807 / 6.334 ms. Both fixtures use 61 interleaved samples per shell. [PTY benchmark](../tools/bench_login.py) · [Reproducible rc files](../tools/ci/native-bench.sh)

On the same M1, supplying the 2,000 literal definitions through `-c` took 5.154 → 3.682 / 8.179 ms including launch and exit. Writing the script to a pipe on standard input took 15.694 → 14.165 / 18.700 ms. For the multiline compound fixture those two entry points took 13.757 → 10.944 / 12.606 ms and 53.613 → 50.483 / 52.546 ms. Each test rotated three shells, discarded three warmups, measured 41 samples and verified the final output. 同じ台本でも入力元で差が出ます。単純な2,000回再定義の起動込み中央値は `-c` で 3.682 ms、標準入力のパイプ経由で 14.165 ms でした。

On that M1, sourcing a 20,888,892-byte file which replaced the same function a million times took 0.815 → 0.203 / 1.065 seconds. Median peak resident size was 3,080,192 → 2,834,432 / 27,164,672 bytes (`/usr/bin/time -l`, one warmup and seven interleaved samples per shell). The file used the literal fixture above with its count increased to one million and printed `999999` on every run. 同じ名前を100万回再定義する試験では、Tonari の中央値が 0.815 秒から 0.203 秒に短縮し、ピーク常駐メモリの中央値も約 3.08 MB から 2.83 MB に減少しました。

表はシェル起動後の中央値で、各欄は「旧 Tonari → 新 Tonari / Bash」(ms) です。単純な関数の再定義は Bash の26～35%の時間になり、定義の直後に毎回呼んで出力を検証するケースでも3環境とも短縮しました。`local` と `if` を含む異なる複数行の本体を毎回読み直す場合は、Linux では Bash より速いものの目標の 2/3 に届いていません。手元の M1 では Git 補完を読み込む対話起動も 5.400 ms から 5.220 ms に短縮しました。

The full inventory preserved all 7,558 earlier feature verdicts. Against the published snapshot, 146 whole-process medians crossed above the 2/3 target and 91 crossed back within it. In the already-started measurements those counts were 79 and 83. The Debian x86-64 feature snapshot ran on an Intel Xeon 6975P-C and its rerun on an AMD EPYC 9V45. Each function comparison in the table above used one host for all three binaries. The archive contains the platform matrices, every crossing with both shells' times, host details and the raw function reports. 全機能一覧も5環境で再計測しました。以前の計測から目標の境界をまたいだ項目は、両方向とも機能名・実測値つきで収録しています。 [Full audit and native reports](native-2026-11-24/function-audit.tar.gz)

<a id="native-x86-completion"></a>
<a id="native-ci"></a>

## 2026-11-24 · Native CPU measurements / ネイティブ CPU での再計測

The x86-64 jobs have run on AMD EPYC 7763 and 9V74, Intel Xeon 6975P-C, and Intel Xeon Platinum 8573C. The Linux arm64 runner reports Arm Neoverse-N2, and the macOS arm64 runner reports Apple M1. Linux exposed four vCPUs and macOS three. Historical Rosetta timings below describe the Apple silicon translation environment. Use the native data for algorithm and compiler decisions. [Timings, hardware and design decisions](native-2026-11-24/README.md) · [Full feature matrix](native-2026-11-24/matrix.md) · [Workflow](../.github/workflows/native-benchmarks.yml)

With 12 independent directory branches on AMD, the shared completion implementation reduced median/p95 to 3.523/3.612 ms, from 8.687/8.778 ms in the former serial-only implementation. With affinity restricted to one CPU, the shared policy ran serially at 8.727/8.827 ms versus 8.643/8.696 ms. The benchmark interleaved 41 samples per binary and verified the inserted path. [Native completion job](https://github.com/sanohiro/tonari-shell/actions/runs/45286039424/job/138082515521)

x86-64 専用の直列化を撤回しました。AMD の4 vCPU では12ディレクトリの補完が約2.47倍速くなり、1 CPU では両方が直列で中央値の差は0.084 msでした。以下の旧表に含まれる Rosetta の値は、当時の測定条件を追跡するために残しています。

<a id="editing-operations"></a>

## 2026-11-24 · Editing latency and CPU budget / 編集遅延と CPU 数

The first table measures a key sequence and its final visible marker on an approximately 64 KiB command. For example the Delete sample sends Home, Forward, Delete and `@`. After every sample the driver reads the actual editor buffer with `bind -x` and compares every byte with the expected result. Tonari and Bash use the same Insert and Alt-W bindings. The previous release had no overwrite mode. Each cell gives the previous Tonari median → current Tonari median / p95, followed by the Bash median in parentheses, all in milliseconds. [Editing benchmark](../tools/bench_editing.py)

Linux measurements in this section used one available CPU with both affinity and cgroup quota, except where the directory table lists a different count. macOS ran on an Apple M1. For the large and short commands, macOS used 9 and 13 samples per shell and operation, Debian arm64 used 9 and Debian x86-64 used 7. The driver discarded two warmups and rotated the shell order. With these sample counts p95 is the largest observed sample. These are PTY measurements on a ready shell. They exclude graphical rendering and IME preedit.

| Edit at ~64 KiB | macOS arm64 | Debian arm64, 1 CPU | Debian x86-64, 1 CPU (Rosetta) |
| --- | ---: | ---: | ---: |
| append | 0.519 → 0.027/0.029 (0.613) | 0.878 → 0.063/0.067 (0.620) | 1.176 → 0.398/0.424 (1.417) |
| insert near head | 1.852 → 0.068/0.074 (2.595) | 2.296 → 0.122/0.137 (5.646) | 2.843 → 0.474/0.477 (5.008) |
| Delete near head | 1.898 → 0.091/0.105 (10.104) | 2.345 → 0.160/0.166 (82.231) | 2.800 → 0.553/0.563 (12.632) |
| Backspace near head | 2.700 → 0.088/0.092 (10.622) | 3.246 → 0.164/0.179 (82.767) | 3.762 → 0.608/0.619 (13.678) |
| character left/right | 3.615 → 0.052/0.058 (2.754) | 3.860 → 0.088/0.103 (2.754) | 4.319 → 0.453/0.456 (5.496) |
| word left | 1.095 → 0.037/0.057 (1.158) | 1.438 → 0.078/0.086 (1.287) | 1.993 → 0.514/0.519 (2.634) |
| word right | 1.133 → 0.065/0.072 (2.106) | 1.690 → 0.126/0.131 (5.638) | 2.155 → 0.551/0.649 (4.118) |
| Home | 0.565 → 0.055/0.063 (1.501) | 0.893 → 0.122/0.161 (4.326) | 1.097 → 0.432/0.445 (3.826) |
| End | 1.783 → 0.068/0.080 (1.755) | 2.304 → 0.091/0.106 (1.691) | 2.581 → 0.502/0.512 (3.593) |
| overwrite | — → 0.102/0.112 (3.421) | — → 0.156/0.170 (9.596) | — → 0.644/0.670 (6.562) |
| copy and yank | 5.712 → 0.118/0.121 (11.352) | 5.554 → 0.231/0.240 (80.320) | 6.377 → 0.768/0.780 (15.465) |
| kill and yank | 1.821 → 0.074/0.085 (3.537) | 2.204 → 0.089/0.136 (2.880) | 2.714 → 0.601/0.624 (5.248) |
| IME at head | 0.552 → 0.063/0.069 (1.607) | 0.912 → 0.125/0.138 (4.432) | 1.184 → 0.435/0.452 (3.433) |
| multiline paste at head | 1.192 → 0.090/0.097 (9.516) | 1.553 → 0.161/0.186 (80.300) | 2.097 → 0.533/0.541 (11.292) |

On the short fixture (about 96 bytes plus the sentinel text) the ranges below give the smallest and largest per-operation medians across the same 14 operations. The last column identifies the largest per-operation p95.

| Platform | Current median range (ms) | Highest current p95 (ms) |
| --- | ---: | ---: |
| macOS arm64 | 0.016–0.059 | 0.065 (copy and yank) |
| Debian arm64 | 0.060–0.091 | 0.103 (overwrite) |
| Debian x86-64 (Rosetta) | 0.316–0.610 | 0.646 (Backspace near head) |

To measure occasional stalls, the steady-state run sent 512 individually acknowledged UTF-8 edits in each of three sessions, after 16 warmup edits per session. Each acknowledgement is a distinct visible character. The near-head sequence moves left one character before each insertion. After each session the driver checks the entire 64 KiB command including all insertions. Cells give previous Tonari / current Tonari / Bash p99, followed by the maximum observed for current Tonari in parentheses, all in milliseconds. Run with `--size 65536 --steady 512 --runs 3`. [Steady-state driver](../tools/bench_editing.py)

| Repeated edit (1,536 samples each) | macOS arm64 | Debian arm64, 1 CPU | Debian x86-64, 1 CPU (Rosetta) |
| --- | ---: | ---: | ---: |
| Append | 3.996 / 0.020 / 0.579 (0.051) | 1.044 / 0.070 / 0.622 (0.102) | 1.093 / 0.073 / 1.155 (0.105) |
| Left and insert near the head | 14.821 / 0.049 / 9.754 (0.054) | 3.108 / 0.222 / 119.217 (0.771) | 2.831 / 0.138 / 17.687 (0.169) |

キーを連続入力した試験では、1 操作ごとにその文字が表示されたことを確認して次を送っています。3 セッション計 1,536 操作の p99 と最大値を記録しました。

The following fixture contains 100,000 history records. Up recalls active-session history, Down restores a draft and Ctrl-R searches for the oldest matching saved command. The paste clock starts with the first write and stops when the last scalar is visible. The driver checks the entire value after Enter. The table uses the same previous median → current median / p95 (Bash median) format. [History and paste benchmark](../tools/bench_editor_input.py)

| History or paste | macOS arm64 | Debian arm64, 1 CPU | Debian x86-64, 1 CPU (Rosetta) |
| --- | ---: | ---: | ---: |
| paste 16384 B + marker | 2.473 → 2.333/2.341 (16.940) | 2.837 → 2.655/3.114 (21.947) | 5.133 → 4.797/5.364 (23.620) |
| paste 1048576 B + marker | 154.766 → 143.454/149.800 (1094.225) | 157.768 → 148.396/162.197 (1881.410) | 155.886 → 135.123/138.411 (1827.504) |
| Up twice (100k entries) | 0.042 → 0.041/0.044 (0.056) | 0.724 → 0.432/0.752 (0.088) | 2.487 → 2.297/2.374 (0.424) |
| Down to draft (100k entries) | 0.022 → 0.022/0.053 (0.044) | 0.302 → 0.302/0.311 (0.080) | 0.413 → 0.416/0.433 (0.131) |
| Ctrl-R (100k entries) | 0.825 → 0.831/0.878 (0.573) | 0.837 → 0.544/0.942 (0.811) | 3.773 → 3.336/3.962 (3.058) |

The branching fixture resolves one abbreviated path across 12 directories with 2,048 unrelated files per directory. The sequential build already supports this path syntax. Both builds return the same unique result. Cells give median / p95 in milliseconds. [Completion benchmark](../tools/bench_completion.py)

| Host and available CPUs | Sequential implementation | Current implementation | Samples |
| --- | ---: | ---: | ---: |
| macOS arm64 · host | 8.956 / 9.438 | 4.731 / 5.144 | 21 |
| Debian arm64 · 8 CPUs | 2.951 / 3.003 | 1.018 / 1.354 | 19 |
| Debian arm64 · 1 CPU | 2.911 / 2.971 | 2.928 / 2.968 | 17 |
| Debian x86-64 (Rosetta) · 8 CPUs | 5.302 / 5.438 | 5.100 / 5.587 | 19 |
| Debian x86-64 (Rosetta) · 1 CPU | 5.229 / 5.731 | 5.070 / 5.206 | 17 |

The arm64 release selects up to four participants. At eight available CPUs, the translated x86-64 prototype took 9.039 ms against 5.325 ms for the sequential comparison on this fixture. The serial-only x86-64 binary in this table was 38,632 bytes smaller than that prototype. The correction above restores the shared scan policy.

During a separate 10,001-file scan, the driver injects a key 1 ms after Tab. The first pair of measurements covers the entire scan on an uninterrupted invocation. The next pair covers key arrival to visible echo on a separate invocation. The final column is a committed IME string after a declared terminal-independent callback has started its 100 ms helper. Values are current Tonari median / p95 in milliseconds. macOS used 31 scan and 21 callback samples. The single-CPU Linux runs used 13 and 9.

| Host | Tab to filename | Key during scan | IME during helper |
| --- | ---: | ---: | ---: |
| macOS arm64 | 3.475 / 3.721 | 0.080 / 0.110 | 1.197 / 1.543 |
| Debian arm64, 1 CPU | 1.363 / 1.409 | 0.062 / 0.074 | 1.116 / 1.390 |
| Debian x86-64 (Rosetta), 1 CPU | 2.987 / 3.390 | 0.106 / 0.760 | 3.157 / 3.224 |

64 KiB の編集表は操作列を送ってから最後の文字が疑似端末に届くまでを測り、毎回、画面外を含む全文を検証しました。Linux の 1 CPU 条件では CPU 割り当てと使用時間の上限をどちらも 1 に固定しています。大容量ペーストは最後の文字までの転送時間を含み、途中の描画更新とは別の指標です。日本語は IME が確定した UTF-8 文字列を測っています。

<a id="completion-typeahead"></a>

## 2026-11-24 · Typeahead, project arguments and long lines / 補完中の入力・引数履歴・長い行

Each cell below is previous Tonari / current Tonari / Bash 5.2.37, in median milliseconds. The PTY measures delivery of visible text. The callback waits 100 ms in an external helper. The driver verifies that Tab took effect before the following key. For the IME row it sends committed UTF-8 only after receiving the callback’s readiness marker. For the long line it times a single `@` after the paste is visible and checks all 65,536 payload bytes plus the suffix after Enter. All three current binaries emitted one byte for this edit, compared with 68,048 for their predecessors. [Completion driver](../tools/bench_completion.py) · [Unicode input driver](../tools/bench_editor_input.py) · [Startup and loop driver](../tools/bench.py)

The M1 runs macOS natively and hosts a Debian arm64 VM. Debian x86-64 timings use Rosetta. Callback, long-line and project cases rotated binaries after three warmups and used 21 / 19 / 13 samples on macOS / Debian arm64 / Debian x86-64. The Unicode and filename rows used 17 / 19 / 13. Startup and loop rows used 51 samples per binary. The project fixture has 100,000 history padding records and 10,001 files in the final directory. These PTY measurements exclude the terminal emulator’s painting and IME composition time.

| Workload | macOS arm64 | Debian arm64 | Debian x86-64 (Rosetta) |
| --- | ---: | ---: | ---: |
| Tab + key, ordinary Bash callback | 117.665 / 116.735 / 117.116 | 107.829 / 110.237 / 112.590 | 140.210 / 135.310 / 134.382 |
| Tab + key, terminal-independent callback | 117.669 / 1.294 / 116.993 | 109.774 / 0.483 / 110.499 | 133.555 / 7.430 / 130.726 |
| IME commit after callback starts | 117.362 / 1.197 / 116.976 | 110.684 / 0.273 / 111.432 | 133.821 / 3.164 / 129.715 |
| Append one ASCII character after a 64 KiB paste | 0.456 / 0.022 / 0.597 | 0.613 / 0.065 / 0.620 | 1.098 / 0.389 / 1.415 |
| Single emoji | 0.035 / 0.034 / 0.051 | 0.083 / 0.076 / 0.087 | 1.141 / 1.477 / 1.472 |
| 17 Unicode scalars | 0.060 / 0.058 / 0.135 | 0.097 / 0.089 / 0.137 | 1.074 / 1.112 / 1.324 |
| 257 Unicode scalars | 0.429 / 0.422 / 1.255 | 0.311 / 0.312 / 0.877 | 0.644 / 0.720 / 2.493 |
| Tab with 10,000 filenames | 3.457 / 3.429 / 6.753 | 1.321 / 1.325 / 1.408 | 4.406 / 4.208 / 5.369 |
| Exec to first interactive prompt | 2.730 / 2.678 / 5.422 | 0.339 / 0.336 / 0.700 | 8.738 / 9.348 / 27.838 |
| Startup through exit, `-c :` | 2.328 / 2.251 / 4.697 | 0.197 / 0.203 / 0.411 | 8.754 / 9.170 / 22.354 |
| 1,000 builtin loop iterations | 2.775 / 2.693 / 6.013 | 0.703 / 0.705 / 2.201 | 12.323 / 12.658 / 27.116 |

Current Tonari median / p95, in milliseconds. The Backspace row measures the erase and the immediately following marker after the first edit has appeared. The project rows wait for the actual candidate to appear.

| Workload | macOS arm64 | Debian arm64 | Debian x86-64 (Rosetta) |
| --- | ---: | ---: | ---: |
| IME commit after callback starts | 1.197 / 1.543 | 0.273 / 0.371 | 3.164 / 3.330 |
| Backspace and a marker following the IME commit | 0.108 / 0.143 | 0.236 / 0.272 | 0.251 / 0.275 |
| Append after a 64 KiB paste | 0.022 / 0.029 | 0.065 / 0.073 | 0.389 / 0.484 |
| Abbreviated multi-level path to one of 10,001 files | 3.528 / 3.625 | 1.364 / 1.414 | 3.442 / 5.503 |
| Previous path argument from another command | 1.180 / 1.339 | 0.946 / 0.988 | 3.493 / 4.708 |
| Previous value for the same tool and option | 1.014 / 1.078 | 0.882 / 0.940 | 1.665 / 2.229 |

The 10,000-file Tab medians were 51% of Bash on macOS, 94% on Debian arm64 and 78% on translated x86-64. In a separate run of 31 fresh sessions per binary, the first committed character on translated x86-64 took 0.767 ms (previous 0.648 ms), with the next ten ASCII keys at a pooled median of 0.067 ms (previous 0.067 ms). On native Debian arm64 the first and subsequent medians were 0.067 and 0.059 ms in both builds. The ordinary callback keeps terminal ownership throughout its wait. For an explicitly terminal-independent callback the editor can display edits during that wait. [Completion option and scope](compatibility.md#interactive-tab)

11 月 24 日の配布物は macOS arm64 が 1,251,104 バイト（前回比 +66,496）、Linux arm64 が 1,462,632 バイト（+98,800）、Linux x86-64-v3 が 1,202,032 バイト（+38,304）です。macOS の動的リンクは libSystem のみで、Linux 版は共有ライブラリなしの static PIE です。Linux x86-64 の速度は Rosetta 上の計測です。QEMU 上の Debian x86-64 では 144 件の統合テストを別途実行しました。 [互換性の検証](compatibility.md)

<a id="editor-bulk-input"></a>

## 2026-11-23 · Committed UTF-8 text, paste and history / 日本語入力・貼り付け・履歴・補完

The PTY sends committed UTF-8 in a burst or encloses a paste in `CSI 200~` and `CSI 201~`. The timer starts before the first write to a ready shell and stops when the PTY produces the final marker. The driver drains output while writing large packets. After timing, it accepts the line and compares the entire resulting variable byte for byte. The interval excludes IME composition and terminal-emulator painting. [Benchmark driver](../tools/bench_editor_input.py) · [xterm bracketed paste](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Bracketed-Paste-Mode)

Each shell starts with the same 100,000 history commands and 10,000 filename candidates. Before timing Up twice, the driver executes two lines in the active session. The second Up recalls the first of those lines. Ctrl-R searches for the oldest command, with a recent decoy matching all but the final scalar of the query. Tab receives the whole UTF-8 prefix immediately before the Tab key. Timing includes reading each fresh shell's history or directory as needed. The paste sizes exclude the assignment prefix, final four-byte scalar and protocol markers.

Each cell gives previous Tonari / adopted Tonari / Bash 5.2.37 median milliseconds. Three warmups preceded 23 samples per binary on native M1 macOS, 21 in the native Debian arm64 VM and 11 on Debian x86-64 under Rosetta. Shell order rotated after each sample. The translated lane omitted the 1 MiB paste.

| Workload | macOS arm64 | Debian arm64 | Debian x86-64 (translated) |
| --- | ---: | ---: | ---: |
| One emoji | 0.064 / 0.065 / 0.087 | 0.096 / 0.104 / 0.101 | 0.704 / 0.437 / 1.559 |
| 17 Unicode scalars | 0.122 / 0.070 / 0.146 | 0.145 / 0.118 / 0.153 | 0.759 / 0.457 / 1.594 |
| 257 Unicode scalars | 2.152 / 0.465 / 1.338 | 2.318 / 0.328 / 0.945 | 3.221 / 0.675 / 2.823 |
| 16 KiB paste | 2.365 / 2.238 / 17.417 | 2.250 / 1.985 / 20.232 | 3.633 / 2.821 / 21.174 |
| 1 MiB paste | 154.822 / 139.979 / 1144.632 | 162.580 / 136.785 / 1464.821 | — |
| Up twice, 100k history entries | 0.048 / 0.046 / 0.064 | 0.094 / 0.100 / 0.110 | 0.174 / 0.245 / 0.490 |
| Ctrl-R, 100k entries | 0.945 / 0.935 / 0.705 | 0.997 / 0.910 / 1.075 | 4.256 / 4.074 / 3.519 |
| Tab, 10k files | 3.875 / 3.849 / 7.375 | 1.450 / 1.425 / 1.517 | 3.497 / 3.838 / 5.979 |

On macOS, p95 for the 257-scalar commit, 1 MiB paste, Ctrl-R and Tab, in the same binary order, was 2.355 / 0.494 / 1.389 ms, 166.298 / 156.583 / 1196.291 ms, 1.229 / 1.132 / 0.831 ms, 3.998 / 4.078 / 7.565 ms. In a separate empty-history check, all 21 one-write sessions on the M1 produced 258 full redraws in the previous build and one in the adopted build for the 257-scalar commit. For 17 scalars the counts were 18 and one. The assignment prefix is included in these counts.

The cold oldest-entry Ctrl-R exceeded two-thirds of Bash time on all three lanes. Bash loads its history before the first prompt and Tonari reads the journal on demand. Other misses on native arm64 were the single emoji and session recall on both operating systems, and the 17-scalar commit and Tab on Debian. All 1,408 macOS, 1,522 Debian and 1,553 Ubuntu compatibility verdicts matched the previous snapshot. Distribution sizes were 1,184,608 bytes on macOS arm64 (+16), 1,363,832 on Linux arm64 (+4,032) and 1,163,728 on Linux x86-64-v3 (+3,456).

確定済みの日本語を受け取った時点から、編集後の表示が疑似端末に届くまでを計測します。通常入力ではすでに届いた範囲をまとめて処理します。ブラケットペーストでは末尾の改行も文字列に含め、終端を受け取ってから1回で挿入します。終端直後の1バイトが Enter の可能性があるため、端末からの読み取りは最大7バイトに制限します。これにより、同じ書き込みで届いた後続データを外部コマンドが自分で読めます。[実装](../src/editor.rs) · [疑似端末テスト](../tests/shell.py)

~~~sh
python3 tools/bench_editor_input.py ./previous ./current /path/to/bash --large --runs 23
~~~

<a id="literal-file-capture"></a>

## 2026-11-23 · File command substitution / ファイルを読むコマンド置換

**Previous Tonari / adopted Tonari / Bash 5.2.37**, median milliseconds. Each sample starts in a fresh shell that has signalled readiness. It assigns the complete file to the same variable the indicated number of times. The timer stops at a second marker before writing the final value to disk. Every sample checks the output bytes and shell status. ASCII input at least two bytes long ends in two newlines. The zero-byte input is genuinely empty.

| Input | Captures | macOS arm64 | Debian x86-64 (translated) | Debian arm64 |
| --- | ---: | ---: | ---: | ---: |
| 0 B | 24 | 13.074 / 0.484 / 0.508 | 30.811 / 7.040 / 6.451 | 3.540 / 0.174 / 0.245 |
| 5 B | 256 | 3.872 / 3.682 / 4.267 | 8.537 / 7.848 / 8.515 | 0.946 / 0.842 / 1.665 |
| 4 KiB | 128 | 2.089 / 1.985 / 3.883 | 8.018 / 7.391 / 9.224 | 0.576 / 0.519 / 3.369 |
| 256 KiB | 16 | 1.007 / 0.913 / 13.211 | 9.159 / 8.364 / 19.610 | 1.371 / 1.254 / 21.972 |
| 4 MiB | 4 | 4.421 / 3.405 / 54.818 | 16.098 / 14.718 / 59.818 | 7.741 / 7.355 / 82.839 |
| 16 MiB | 1 | 5.834 / 5.187 / 64.062 | 16.023 / 13.970 / 63.196 | 7.272 / 6.151 / 81.042 |
| 64 MiB | 1 | 23.421 / 20.381 / 430.420 | 35.779 / 30.878 / 226.443 | 26.910 / 23.155 / 324.253 |

Warm filesystem cache, Apple M1, macOS 27.2.1 and Debian 13 on Linux 6.11.11-linuxkit. The Linux binaries and fixtures ran on the guest filesystem. The previous build was `36a580f`. The binaries ran in rotating order, with three warm-ups per binary followed by 19 samples (nine for 64 MiB). The timer includes the gate, script loop and marker. Close timings for short commands on translated x86-64 also include scheduler variation.

The two-thirds-of-Bash target was missed at 0 and 5 B on macOS, at 0 B on Debian arm64, and at 0 B, 5 B and 4 KiB on translated x86-64. In a longer 21-sample check on translated x86-64, 2,048 five-byte captures took 16.781 / 14.992 / 19.904 ms. For 1,024 four-KiB captures the times were 12.873 / 11.574 / 25.326 ms. The previous-snapshot correctness comparison kept all 1,408 macOS, 1,522 Debian and 1,553 Ubuntu statuses.

5 バイトと 16 MiB のファイルで、開いた FD に対する `fstat` は2回から1回、`lseek` は1回から0回になりました。空ファイルでは通常のストレージかを追加確認してから読みます。サイズ0を返す仮想ファイルでは実際の EOF まで読みます。初期確保の上限は 64 MiB で、それ以上の入力も最後まで取り込みます。配布バイナリのサイズは macOS arm64 が 1,184,592 バイト（+96）、Linux x86-64-v3 が 1,160,240 バイト（+2,400）、Linux arm64 が 1,359,752 バイト（+1,816）でした。実機 x86-64 と WSL2 は未計測です。[開発知見](engineering-notes.md#literal-file-capture)

```sh
python3 tools/bench_file_capture.py ./previous ./tonari /path/to/bash --runs 19
python3 tools/bench_file_capture.py ./previous ./tonari /path/to/bash --case 67108864 1 --runs 9
```

<a id="external-stream-controls"></a>

## 2026-11-23 · External redirections and pipes after the capture change / 通常のリダイレクトとパイプ

**Previous Tonari (`36a580f`) / current Tonari (`e7cd570`) / Bash 5.2.37**, median milliseconds from shell launch to exit. Each shell read the same script and each output file was compared byte for byte. The redirect runs `/bin/cat <input >out`. The pipeline adds two more `/bin/cat` stages. The kernel supplied the default pipe capacity throughout. The binaries ran in rotating order with two warm-ups and 31 measured samples per configuration on the M1. The Linux executables and data ran on the Debian 13 guest filesystem.

| Input and operation | macOS arm64 | Debian arm64 | Debian x86-64 under Rosetta |
| --- | ---: | ---: | ---: |
| 64 B, redirect | 6.023 / 6.107 / 9.562 | 0.593 / 0.562 / 0.978 | 26.769 / 26.936 / 42.030 |
| 64 B, pipeline | 6.818 / 6.905 / 9.682 | 0.744 / 0.703 / 1.147 | 31.296 / 31.240 / 45.652 |
| 16 MiB, redirect | 14.896 / 15.039 / 18.328 | 3.789 / 3.772 / 3.978 | 32.067 / 31.927 / 47.155 |
| 16 MiB, pipeline | 15.477 / 15.050 / 18.864 | 11.818 / 11.210 / 11.831 | 47.720 / 47.709 / 62.950 |

In a separate control, one native arm64 Python parent launched both release binaries inside the same Linux VM. The median elapsed time over 81 alternating warm samples for `--noprofile --norc -c ':'` through exit was 0.355 ms on arm64 and 8.543 ms under Rosetta. A pipe handshake in a ready shell (send newline to `read`, receive the marker after `:`) took 0.048 and 2.808 ms. Adding 256 five-byte captures inside the handshake gave 0.853 and 7.817 ms. The ready-shell measurements exclude process launch and exit. Inspection of `/proc/PID/maps` from the native parent found `/run/rosetta/rosetta` in the x86-64 Tonari process. These figures characterize the M1 VM and its translator. Native Intel and AMD hardware remains unmeasured.

外部コマンドはシェルが接続した FD を直接読み書きします。今回の計測でも、ファイルをシェル変数に取り込む最適化による一貫した差は確認できませんでした。小さいパイプでは接続と起動の時間が効きます。Linux で大きなストリームを流す場合は、別の試作でパイプ容量を広げる効果が出ています。[計測スクリプト](../tools/bench_streams.py) · [パイプ容量の比較](#linux-pipe-capacity)

```sh
python3 tools/bench_streams.py ./previous ./current /path/to/bash --case redirect --case pipe --sizes 64 16777216 --runs 31
```

<a id="macos-pipe-capacity"></a>

## 2026-11-23 · Native macOS pipe capacity / macOS のパイプ自動拡張

Apple M1, macOS 27.2.1, Darwin 27.2.0, all three executables native arm64. The release at `9d879ac` lets XNU size each pipe. An isolated build used the same distribution profile and primed each new pipeline connection before starting the children. It temporarily enabled nonblocking writes, wrote up to 16 KiB of zeros, restored blocking mode and drained exactly the bytes written. A downstream child running after an empty producer observed `fstat(0).st_blksize` of 16 KiB in the release and 64 KiB in the trial. A separate nonblocking 128 KiB write to a fresh empty pipe accepted 64 KiB with the reader idle. Both Tonari binaries occupied 1,184,592 bytes.

Median milliseconds from shell launch to exit, warm cache, no startup files. Three warm-ups preceded the measured samples for each binary. The launcher shuffled all six execution orders in blocks. Both external `dd` processes used a 64-byte block for 64 B and 64 KiB blocks for the larger transfers. The test verified the byte count reported by the receiving `dd`, all bytes of the three-stage file output and the one-byte loops, and the exit statuses of the early-close pipeline (141 and 0).

| Workload | Samples per binary | Tonari, automatic | Tonari, pre-expanded | GNU Bash 5.2.37 |
| --- | ---: | ---: | ---: | ---: |
| 32 one-byte external pipelines in one shell | 25 | 51.207 | 53.320 | 83.388 |
| 64 B, two external `dd` processes | 49 | 5.741 | 5.763 | 9.296 |
| 4 MiB, two `dd` processes | 31 | 6.649 | 6.854 | 10.180 |
| 256 MiB, two `dd` processes | 25 | 54.393 | 53.408 | 58.222 |
| 16 MiB file through three `cat` processes | 31 | 13.628 | 13.466 | 16.877 |
| `yes` piped to `head -n 1` | 31 | 5.370 | 5.521 | 9.283 |

A second workload with `wc -c` as the consumer took 45.945 / 46.182 / 49.442 ms for 256 MiB in the same column order. In that session the repeated one-byte pipelines took 55.866 / 57.655 / 89.492 ms. The trial slowed down this short-command workload in both sessions. The automatic release missed the two-thirds-of-Bash target on the 256 MiB transfer and the three-stage file copy.

Mac では OS の自動拡張を採用。子プロセスの起動前に毎回 64 KiB まで拡張すると、短いパイプ32回が両方の計測で約3〜4%遅くなりました。256 MiB 転送は受け手が `dd` のときに約1 ms 短縮し、`wc -c` のときは差が 0.3 ms 未満でした。[Apple XNU の書き込み時の拡張処理](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/sys_pipe.c#L948-L971)

<a id="balanced-linux-pipes"></a>

## 2026-11-23 · Choosing a Linux pipeline capacity / Linux のパイプ容量の選定

Each timing cell gives **kernel default (64 KiB) / 128 KiB / 256 KiB** median milliseconds from shell launch to exit. All three used Tonari `e7cd570` with the same distribution build profile. The two candidates queried each new pipeline's capacity and requested an increase for capacities from 64 KiB up to the target. A downstream process verified all three capacities. This kernel rounded a 96 KiB request to 128 KiB and a 192 KiB request to 256 KiB. Single-stage file redirections use a separate execution path.

| Workload | Debian arm64, 8 CPUs available | Debian arm64, pinned to 1 CPU | Debian x86-64 under Rosetta |
| --- | ---: | ---: | ---: |
| 64 B, two external `dd` processes | 0.672 / 0.662 / 0.661 | 2.975 / 2.983 / 2.975 | 30.847 / 30.783 / 30.891 |
| 256 KiB, two `dd` processes | 0.764 / 0.732 / 0.634 | 2.999 / 2.990 / 3.028 | 30.052 / 29.888 / 29.701 |
| 4 MiB, two `dd` processes | 3.084 / 1.600 / 1.470 | 3.672 / 3.895 / 3.812 | 31.352 / 31.171 / 30.771 |
| 256 MiB, two `dd` processes | 121.520 / 51.726 / 43.699 | 45.100 / 43.204 / 41.856 | 73.810 / 72.818 / 71.778 |
| 16 MiB file through three `cat` processes | 12.692 / 9.209 / 7.404 | 11.652 / 11.059 / 10.626 | 44.585 / 40.343 / 38.851 |
| `yes` piped to `head -c 64` | 0.640 / 0.645 / 0.632 | 2.926 / 2.963 / 2.986 | 32.697 / 32.242 / 32.668 |
| 32 one-byte external pipelines in one shell | 10.975 / 10.857 / 11.050 | 10.909 / 10.737 / 10.719 | 479.772 / 478.165 / 478.405 |

The M1 ran a Debian 13 VM with Linux 6.11.11-linuxkit, 4 KiB pages and warm caches. Each workload cycled through all six execution orders in shuffled blocks. After three warm-ups, the seven rows used 61, 31, 25, 17, 21, 41 and 19 samples per binary per session (13 for the last row under Rosetta). Each arm64 column pools two sessions. The translated column has one. The one-CPU restriction applied to the launcher, shell and both ends of each pipe. The 64 B transfer used one 64-byte block. The larger `dd` transfers used 64 KiB blocks from `/dev/zero` to `/dev/null` with `pipefail`. The three-stage test verified every file byte. The early-close test verified the 64 output bytes and stage statuses 141 and 0.

128 KiB は、8 CPU を使える arm64 で 256 MiB 転送の短縮分の約 90% を得つつ、256 KiB 案の半分の容量で済みました。32本の短いパイプでは差が小さく、同じ条件の p95 は 12.643 / 13.014 / 12.387 ms でした。1 CPU での 4 MiB 転送は 128 KiB の方が中央値で 0.223 ms 長くなりました。Linux の主対象であるネイティブ x86-64 と WSL2 は未計測です。[採用した実装](../src/execute.rs) · [バイト単位の検証](../tools/bench_streams.py)

A separate session added Debian Bash 5.2.37 to the rotation. Each cell gives **128 KiB Tonari / Bash** median milliseconds. Bold cells took more than two-thirds of Bash time. Each case ran 27 times per binary, or 61 for the translated 256 MiB transfer.

| Transfer | arm64, 8 CPUs available | arm64, pinned to 1 CPU | x86-64 under Rosetta |
| --- | ---: | ---: | ---: |
| 64 B | **0.627 / 0.928** | **2.010 / 2.998** | 26.648 / 41.250 |
| 4 MiB | 1.426 / 3.377 | **3.999 / 4.756** | **28.248 / 41.900** |
| 256 MiB | 43.794 / 108.020 | **42.035 / 44.969** | **68.568 / 83.730** |

Under translation the 128 KiB build's 256 MiB p95 changed from 176.685 ms in the first 17 samples to 72.851 ms in the separate 61-sample run. The kernel-default build's p95 went from 75.577 to 154.967 ms across those sessions. The tail timings varied across sessions for both builds.

On arm64, a separate test ran as a private UID and held empty pipes near the kernel's per-user soft limit. Both trial binaries completed a pipeline with the original 64 KiB when an increase was denied. After crossing the limit, the child observed the kernel's reduced 8 KiB capacity and the test still transferred all bytes. [Linux pipe capacity accounting](https://man7.org/linux/man-pages/man7/pipe.7.html)

<a id="linux-pipe-capacity"></a>

## 2026-11-23 · Linux pipeline capacity / Linux のパイプ容量

Tonari at `36a580f` and a distribution build from the same source were run on an M1 in a Debian 13 VM with Linux 6.11.11-linuxkit and 4 KiB pages. The trial added one `fcntl(reader, F_SETPIPE_SZ, 256 * 1024)` immediately after Tonari created each pipeline connection. A child queried its stdin with `F_GETPIPE_SZ` and observed 65,536 bytes in the release and in Bash 5.2.37, and 262,144 in the trial. The x86-64 binaries and external tools ran under translation on the same arm64 kernel.

Each cell gives **release / trial / Bash** median milliseconds from shell launch through exit. Executables ran in rotating order with three warm-ups for the two-stage workloads. The byte-checked three-stage workload used two warm-ups. Each architecture ran separately.

| Workload | Samples per binary | arm64, 8 CPUs available | arm64, pinned to 1 CPU | x86-64 translated, 8 CPUs available |
| --- | ---: | ---: | ---: | ---: |
| `dd \| dd`, 64 B | 61 | 0.849 / 0.815 / 1.260 | 2.995 / 2.993 / 3.991 | 31.884 / 31.855 / 46.475 |
| `dd \| dd`, 4 MiB | 25 | 2.802 / 2.224 / 3.770 | 3.986 / 3.995 / 4.975 | 32.250 / 32.211 / 46.828 |
| `dd \| dd`, 256 MiB | 13 | 116.865 / 51.826 / 131.376 | 47.793 / 44.519 / 49.932 | 75.395 / 75.162 / 91.826 |
| `yes \| head -c 64` | 41 | 0.725 / 0.718 / 1.188 | 2.893 / 2.988 / 3.983 | 33.641 / 34.007 / 51.398 |
| 32 one-byte external pipelines in one shell | 23 | 12.742 / 13.353 / 15.806 | 11.608 / 11.818 / 18.651 | 533.204 / 547.302 / 591.060 |
| Three `cat` stages, 16 MiB file to file | 13 | 12.322 / 5.667 / 12.064 | — | 50.283 / 42.001 / 62.272 |

The 256 MiB command used `set -o pipefail` and two external `dd` processes. The early-close case checked the 64 output bytes and both stage statuses (141 and 0). The three-stage case checked every output byte after timing. The one-byte case ran `/usr/bin/printf x | /bin/cat` 32 times and checked the resulting 32 bytes. The native arm64 256 MiB samples had wide tails with eight CPUs available (release/trial p95 about 217/190 ms). A second run on the same VM gave release/trial medians of 124.827/45.514 ms for 256 MiB, 13.403/6.827 ms for the 16 MiB file, and 10.373/10.520 ms for 32 one-byte pipes. CPU affinity and host scheduling matter when interpreting these numbers.

```sh
set -o pipefail
/bin/dd if=/dev/zero bs=65536 count=4096 status=none | /bin/dd of=/dev/null bs=65536 status=none
python3 tools/bench_streams.py ./base ./trial /bin/bash --case pipe --sizes 64 4096 262144 4194304 16777216 --runs 13
```

256 KiB は Debian arm64 の連続転送で効果が大きく、16 MiB のファイルを3段の `cat` で渡すケースでも短縮しました。1 CPU に固定すると 256 MiB 転送での差は約 3 ms でした。短いパイプを32回起動するケースでは、どの条件でも試作版の中央値が少し長くなりました。実機 x86-64 と WSL2 は未計測です。[設計上の判断](engineering-notes.md#pipeline-capacity-and-input)

<a id="profile-startup"></a>

## 2026-11-23 · Startup with Bash profiles / 設定を含むシェルの起動

Each cell lists previous Tonari (`8ec0b7d`), current Tonari and GNU Bash 5.2.37, in that order. Each pair is median/p95 milliseconds. The reference executables ran in rotating order on the same M1 with warm filesystem caches. Linux ran in a Debian VM. The amd64 userspace used translation. PTY measurements start immediately before exec and end when the launcher reads the prompt. For the three rc cases, the last line sets the marker after loading the configuration.

| Workload | macOS arm64 | Debian x86-64 (translated) | Debian arm64 |
| --- | ---: | ---: | ---: |
| Fresh shell through exit, `-c ':'` | 2.677/3.165 · 2.615/3.217 · 5.505/6.260 | 9.504/10.066 · 9.204/9.828 · 24.057/25.246 | 0.223/0.408 · 0.217/0.350 · 0.475/0.860 |
| Exec to first prompt, no rc | 3.129/3.528 · 3.078/3.687 · 6.163/6.859 | 9.374/10.093 · 9.223/9.935 · 29.406/30.732 | 0.285/0.385 · 0.284/0.387 · 0.642/0.832 |
| Aliases, PATH and a lazy nvm function | 3.394/3.808 · 3.363/3.938 · 6.406/7.090 | 27.550/28.584 · 27.927/29.416 · 44.643/46.566 | 0.554/0.869 · 0.581/0.785 · 1.102/1.456 |
| Apple Git completion in the rc | 14.468/19.434 · 6.364/8.008 · 11.030/15.638 | 54.547/56.020 · 39.304/40.849 · 57.810/59.153 | 10.482/10.835 · 2.718/3.010 · 4.499/4.903 |
| One 760-line function in the rc | 108.282/109.464 · 3.542/4.088 · 7.233/7.815 | 237.314/240.306 · 21.809/22.584 · 42.389/43.400 | 106.080/109.749 · 0.803/0.964 · 2.253/2.696 |
| System and user login profiles | 4.878/5.331 · 4.723/5.422 · 8.999/9.744 | 74.219/77.098 · 74.728/75.802 · 94.412/96.891 | 1.590/1.952 · 1.582/1.955 · 2.410/2.748 |

Mac の軽量 rc は前版とほぼ同じで、Git 補完を読む場合は中央値が56%短縮しました。変換実行の Debian x86-64 では Git 補完が Bash の68%、ログインが79%で、2/3 の目標を超えています。前版と比べると、この環境の軽量 rc は0.377 ms、ログインは0.509 ms遅くなりました。互換性監査の判定は macOS の1,408項目、Debian の1,522項目、Ubuntu の1,553項目すべてで前版と一致しました。

設定なしは各201回、rc の3条件は各41回、ログインは各31回の測定で、それぞれ最初の5回を除きました。軽量な rc は2,052バイトの設定を一時ホームにコピーし、`ll` と `nvm` の定義を確認しました。Git の rc は Apple Command Line Tools の `git-completion.bash`（94,826バイト）を読み込み、`__git_complete` の定義を確認しました。大きな関数の rc は760行の組み込みコマンドを含む定義を読み、関数の先頭で return します。ログインでは実際の `/etc/profile` と、Debian では配布版のスケルトン設定を使いました。[プロンプトの計測](../tools/bench_login.py) · [設定なしの計測](../tools/bench.py)

A separate link-format trial used identical interpreter sources, equal-length executable paths and 241 warm samples per binary on macOS arm64. With the existing dyld encoding, `-c ':'` and exec to the no-rc prompt took 2.516 and 2.956 ms. Forcing `-Wl,-fixup_chains` gave 2.656 and 3.054 ms. Both binaries targeted macOS 11.0. In a separate 301-run control, a local minimal C executable took 1.805 ms, Tonari's version command took 2.558 ms and `-c ':'` took 2.556 ms from spawn to exit, with stdout redirected to the null device. [Link-format investigation](engineering-notes.md)

The distribution binaries are 1,184,496 bytes on macOS arm64, 1,157,840 bytes on Linux x86-64-v3 and 1,357,936 bytes on Linux arm64. The Mac binary links directly to libSystem. Both Linux binaries passed the static PIE check and executed in an otherwise empty root filesystem.

```sh
printf '%s\n' '. /Library/Developer/CommandLineTools/usr/share/git-core/git-completion.bash' \
  'declare -F __git_complete >/dev/null || exit 1' > git-profile.bash
python3 tools/bench_login.py /path/to/tonari --previous /path/to/previous \
  --bash /path/to/bash --rcfile git-profile.bash --runs 41
```

<a id="escaped-history"></a>

## 2026-11-23 · History queries containing backslashes / バックスラッシュを含む履歴検索

One million history entries, one matching command at the beginning of the journal. A fresh PTY received `foo\ bar` for Ctrl-R or the command prefix for autosuggestion. Each cell gives median / p95 milliseconds from input to the verified PTY output. The table compares the `dc96c84` distribution with the updated build. Each workload ran three warmups and 21 measured sessions per executable, with warm filesystem caches. Debian x86-64 uses translated userspace on the Apple M1. [Benchmark driver](../tools/bench_history.py)

| Workload | macOS arm64 previous → current | Debian x86-64 previous → current | Debian arm64 previous → current |
| --- | ---: | ---: | ---: |
| Escaped prefix visible | 0.054/0.073 → 0.052/0.071 | 0.559/0.702 → 0.584/0.803 | 1.078/2.337 → 1.048/1.660 |
| Escaped prefix to visible suggestion | 110.822/119.850 → 6.289/7.237 | 95.495/105.934 → 8.178/9.158 | 62.733/67.004 → 5.962/7.281 |
| Escaped Ctrl-R to oldest match | 144.470/154.036 → 6.496/7.771 | 132.426/141.818 → 8.717/9.881 | 86.506/90.499 → 6.083/7.339 |
| Plain Ctrl-R to oldest match | 6.423/7.716 → 6.587/8.125 | 8.576/9.493 → 8.736/9.515 | 6.343/7.271 → 6.423/7.503 |
| Plain Ctrl-R to newest match | 0.246/0.322 → 0.249/0.321 | 2.062/2.196 → 2.239/2.597 | 0.185/0.231 → 0.189/0.223 |

入力文字の表示と、その後の履歴候補の表示を別々に確認しました。改善幅が大きいのは、最近の履歴に一致がなく、古いコマンドのエスケープを検索する場合です。通常の検索は旧版に近く、変換実行の Debian x86-64 では直近の検索が中央値で 0.177 ms 遅くなりました。配布サイズは macOS arm64 が 1,184,496 バイト、Linux x86-64 が 1,157,360 バイト（+800）、Linux arm64 が 1,357,360 バイト（+336）です。

```sh
python3 tools/bench_history.py /path/to/previous /path/to/current --escaped --entries 1000000 --runs 21
```

A separate PTY trial sent Ctrl-R, `foo\ bar` and Enter in one write after the first prompt. The oldest of one million commands was `printf 'found:%s\n' foo\ bar`. The timer stopped when the PTY received the output line `found:foo bar`. Each fresh shell used its native history format. Bash 5.2.37 had `HISTSIZE=1000001` and loaded its history before the measured interval. Each shell ran three warmups and 21 measured sessions in rotating order. The release comparison found the same named compatibility outcomes as `dc96c84` for all 1,408 macOS, 1,522 Debian and 1,553 Ubuntu observations. [Shared Ctrl-R regression](../tests/shell.py)

| Ctrl-R and Enter to command output, median / p95 ms | Previous Tonari | Tonari | Bash |
| --- | ---: | ---: | ---: |
| macOS arm64 | 91.250/93.475 | 3.858/4.476 | 22.859/24.576 |
| Debian x86-64 (translated) | 140.618/143.755 | 17.153/18.466 | 49.773/53.061 |
| Debian arm64 | 84.335/85.569 | 7.342/9.076 | 49.639/53.582 |

こちらは起動済みシェルに検索語と Enter をまとめて送り、選択したコマンドの出力まで測った値です。上の表で測った候補表示とは操作の終点も、履歴の最古のコマンドも異なります。

<a id="nocaseglob"></a>

## 2026-11-23 · Pathname filters and case-insensitive globs / ファイル名の除外と大小文字の照合

Previous Tonari / current Tonari / reference Bash, median milliseconds in a running shell. Measurements alternate executables after warmup. Each script checks the result count, stderr and exit status. The previous release comes from `ed056ca`. The Bash reference is 5.2.37. The Debian arm64 VM runs on the M1. Debian x86-64 is translated userspace on that machine. All Linux timings use the guest filesystem. The Mac uses `en_US.UTF-8` and Linux uses `C.UTF-8`.

| Workload | macOS arm64 | Debian arm64 | Debian x86-64 (translated) |
| --- | ---: | ---: | ---: |
| 4,096 files, one expansion | 6.940 / 6.969 / 17.892 | 1.718 / 1.728 / 3.879 | 10.244 / 10.550 / 11.430 |
| 4,096 files, eight expansions | 21.329 / 21.474 / 133.378 | 8.218 / 8.195 / 29.055 | 21.030 / 21.738 / 41.164 |
| 4,096 files, eight expansions with GLOBIGNORE | 25.150 / 23.284 / 123.046 | 9.417 / 8.435 / 33.357 | 24.758 / 23.020 / 45.591 |
| 4,096 files, eight case-insensitive expansions | — / 20.882 / 132.556 | — / 8.645 / 31.030 | — / 23.753 / 44.670 |
| 100,000 files, one expansion | 231.836 / 233.632 / 590.653 | 43.094 / 43.355 / 115.585 | 94.546 / 95.327 / 130.466 |
| 100,000 files, one expansion with GLOBIGNORE | 199.181 / 190.166 / 490.969 | 42.380 / 38.051 / 117.358 | 93.268 / 82.088 / 134.973 |
| 100,000 files, one case-insensitive expansion | — / 233.796 / 595.698 | — / 43.429 / 119.248 | — / 115.922 / 157.150 |

4,096件は21回、10万件は9回の中央値です。10万件は `large/` 配下に `a-`、`B-`、`雪-`、`🌱-` で始まる番号付きの `.txt` を同数作りました。`GLOBIGNORE=large/B-*.txt` の結果は75,000件、通常の展開と `shopt -s nocaseglob` を使った `large/*.TXT` は100,000件です。大小文字を無視する設定に未対応だった旧版は、その行では計測対象から外しています。[再現用の4,096件ベンチマーク](../tools/bench_scripts.py)

The feature harness re-timed 15 matching witnesses per reference platform, including all five extended glob operators and `GLOBIGNORE`. It interleaved 15 measured samples per shell after two warmups. For the one-file `nocaseglob` witness, starting after shell readiness, Tonari / Bash took 2.675 / 4.307 ms on the Mac, 0.385 / 0.508 ms on Debian arm64, and 1.465 / 1.777 ms on Ubuntu arm64. The witness also launches `touch`. Compared with the earlier published snapshot, four short witnesses moved above the two-thirds time target. These were Debian’s already-started `failglob`, the Mac’s already-started `?(...)` and `GLOBIGNORE`, and the Mac’s whole-run `failglob`. Each still completed faster than Bash. The TSV includes absolute times for both snapshots. [Per-feature medians and p95](compatibility/pathname-timings.tsv)

10万件を除外条件なしで展開する中央値は、3環境とも前版の約1%以内でした。除外条件を使うと3環境とも短縮しました。大小文字を無視する10万件の展開は macOS と Debian arm64 で Bash の半分以下、変換実行の Debian x86-64 では Bash の約74%で、2/3 の目標を上回っています。

The shipped executable sizes in bytes are macOS arm64 1,184,496, Debian arm64 1,357,024, Debian x86-64-v3 1,156,560. [Compatibility witnesses](compatibility.md) · [Matching algorithm](engineering-notes.md)

<a id="release-footprint"></a>

## 2026-11-23 · Stripped executable size and sourced functions / 配布サイズと関数定義の読み込み

The distribution builds already stripped symbols. Rebuilding the Rust standard library without in-process backtrace symbolization released space for faster arm64 code. Compare the trial sizes at equal optimization levels, then the two candidate profiles on the same source. Sizes cover the executable alone. The reference is GNU Bash 5.2.37 on the Mac and the distribution package on Debian. [Build settings](../tools/build-release.sh) · [Link-map analysis](engineering-notes.md)

| Target / 対象 | Previous Tonari (`9a095b8`) | Trace-only change at `-Os` | Full changes at `-Os` | Full changes at `-O2` | Shipped (bytes) | Bash (bytes) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| macOS arm64 | 1,103,088 | 986,768 | 986,720 | 1,184,480 | **1,184,480** | 1,208,904 |
| Debian x86-64-v3 | 1,297,424 | 1,158,736 | 1,153,872 | 1,553,424 | **1,153,872** | 1,298,416 |
| Debian arm64 | 1,218,640 | 1,089,160 | 1,085,232 | 1,353,392 | **1,353,392** | 1,412,096 |

全配布版で strip を確認しました。旧 x86-64 版の機械語 1,148,345 バイトのうち musl 由来は 95,770 バイトでした。Rust のバックトレース・DWARF 解析関連のシンボルには約 102 KB ありました。後者は LTO 後の関数名に基づく概算です。配布版の機械語は x86-64 で 1,017,404 バイト、macOS で 1,018,792 バイトです。Debian の Bash の機械語は 819,477 バイトで、libc と libtinfo は共有ライブラリから読み込みます。

Both Linux releases executed a builtin pipeline through each entry point in a chroot containing only the staged distribution. ELF inspection confirmed static PIE with no interpreter or shared library requirements. The Mac executable passed signature verification and directly links only to libSystem. The release SHA-256 fingerprints are `77291dd9800aaad38f45a46080855990aae42249bd53b2e406c3f8c900409c30` (Mac), `2014c336f28b0bc4cbb011fa92be904f5d6465a44c356ae2d018defa923af74d` (x86-64), and `f5aced640afff70fea714499b8e151d9f9907350ca0633c569c95a7aa8d977c1` (Linux arm64).

Each cell gives **`-Os` / `-O2` / Bash** in milliseconds. Each executable ran 31 times in rotating order after four warmups on an M1. Debian arm64 ran in a native-arm64 VM. Debian amd64 used translation. All Linux executables were on the guest filesystem. The first prompt used a controlling PTY without startup files. Script timings start after the readiness handshake and check stdout, stderr and exit status. The Mac glob used `en_US.UTF-8` and Linux used `C.UTF-8`.

| Workload / 操作 | macOS arm64 | Debian arm64 | Debian x86-64 (translated) |
| --- | ---: | ---: | ---: |
| Start and exit | 4.173 / 4.026 / 7.687 | 0.294 / 0.299 / 0.716 | 11.597 / 13.585 / 32.959 |
| Exec to first prompt | 3.249 / 3.224 / 6.711 | 0.302 / 0.304 / 0.828 | 7.548 / 8.645 / 32.695 |
| Source 2,000 distinct definitions | 3.254 / 2.971 / 3.845 | 3.022 / 3.036 / 4.013 | 12.723 / 13.552 / 10.226 |
| Source 2,000 redefinitions | 2.744 / 2.568 / 3.360 | 2.413 / 2.348 / 2.792 | 10.907 / 12.122 / 8.851 |
| Call a function 4,000 times | 8.858 / 7.877 / 17.843 | 7.723 / 7.555 / 26.044 | 23.453 / 24.035 / 35.074 |
| Arithmetic loop | 7.418 / 5.672 / 15.361 | 5.610 / 4.439 / 23.489 | 15.892 / 15.540 / 30.134 |
| Glob 4,096 UTF-8 filenames | 7.188 / 7.024 / 17.440 | 1.800 / 1.705 / 3.814 | 10.331 / 12.224 / 10.539 |
| Echo command substitutions | 0.659 / 0.617 / 55.430 | 0.229 / 0.219 / 15.176 | 9.430 / 10.664 / 151.313 |
| 16 external commands | 21.508 / 21.134 / 35.268 | 3.554 / 3.560 / 4.894 | 162.503 / 165.127 / 173.851 |

At equal `-Os` settings, eliminating lookahead on complete source lines reduced the 2,000 distinct definitions from 7.506 to 3.036 ms on the Mac, 6.816 to 3.019 ms on Debian arm64 and 21.410 to 13.293 ms under amd64 translation. The original binaries took 8.077, 7.262 and 19.525 ms respectively in that series. On both arm64 targets the selected profile beats Bash on this case while still exceeding two-thirds of Bash time. Under translation it takes longer than Bash. Native x86-64 timings remain open.

| Interactive workload (median / p95 ms) | macOS `-Os` | macOS `-O2` | Debian x86-64 `-Os` | Debian x86-64 `-O2` |
| --- | ---: | ---: | ---: | ---: |
| Typed key, recent prefix | 0.015 / 0.028 | 0.014 / 0.025 | 0.069 / 0.106 | 0.069 / 0.358 |
| Oldest Ctrl-R match, 1M entries | 7.404 / 8.751 | 6.974 / 9.602 | 9.986 / 10.649 | 11.288 / 12.774 |
| File completion, 100k files | 40.147 / 49.304 | 40.763 / 50.709 | 17.467 / 28.860 | 17.206 / 24.345 |
| PATH completion, 100k commands | 41.001 / 57.704 | 40.776 / 50.892 | 18.255 / 22.137 | 17.981 / 19.753 |

The first 21-run Mac comparison gave file-completion p95 of 49 / 155 ms. Reversing binary order and taking 61 samples gave the values above. The Debian arm64 61-run repeat gave file-completion median / p95 of 16.272 / 20.747 ms at `-Os` and 15.863 / 23.515 ms at `-O2`. Oldest Ctrl-R search used 17 sessions per Mac build and 13 on each Linux platform. Translated x86-64 completion used 15 sessions per build. [History harness](../tools/bench_history.py) · [Completion harness](../tools/bench_completion.py)

arm64 はスクリプトが速くなる `-O2` を採用しました。入力や10万件の補完では、中央値はほぼ同じでした。x86-64 の変換実行では `-Os` の方が起動・履歴検索・ファイル名展開で速いため、こちらを採用しました。サイズの上限は設けず、キー入力・補完の遅延と実行速度を優先しています。

<a id="release-latency"></a>

## 2026-11-23 · Distribution binaries and interactive latency / 配布バイナリと入力応答

| Target / 対象 | Previous Tonari (bytes) | Tonari (bytes) | Reference Bash (bytes) | Tonari SHA-256 |
| --- | ---: | ---: | ---: | --- |
| macOS arm64 | 1,202,848 | 1,103,088 | 1,208,904 | `73892b708dc1da885a258b938bde9e85d17661dc1527c89855ffcea8e1798193` |
| Debian x86-64-v3 | 1,420,928 | 1,297,424 | 1,298,416 | `e14a5dec6002a5aa0a049381794b1b019cf66149163c539afaf1bd7aa2679509` |
| Debian arm64 | 1,369,056 | 1,218,640 | 1,412,096 | `4d732a9e9a064c4273440441a8b04fbe582a2f1f4c5870051ba371b9f1e76550` |

On x86-64, switching from the former glibc binary to the first static musl release grew the file by 141,744 bytes. Its `.text` grew by 123,502 bytes and `.rodata` by 29,464, while `.eh_frame` grew by 212. The static executable contains runtime code and data that would otherwise be provided partly by shared libraries. These section deltas measure the complete Rust target change. Bash sizes in the table exclude shared libraries. The final standalone x86-64 executable is 18,240 bytes larger than the former Tonari binary which required glibc. For the new release, rebuilding the Rust standard library and changing compiler settings reduced unwind metadata enough to spend more bytes on faster interpreter code. The final x86-64 `.eh_frame` and `.eh_frame_hdr` total 2,476 bytes, down from 163,124. Its `.text` is 1,148,345 bytes, up from 1,100,867. The x86-64 disassembly includes AVX2 byte comparisons and BMI2 shifts. On macOS, Mach-O `__text` fell from 1,006,052 to 945,256 bytes and the arm64 disassembly contains NEON vector operations. The smaller `-Oz` trial was 1,173,296 bytes. In an interleaved translated Linux run, arithmetic took 26.99 ms with that profile and 15.65 ms with the selected profile, and functions took 34.67 and 23.73 ms respectively. On arm64, full standard-library sorting with `-Os` fit both targets. macOS `-O2` improved arithmetic in the trial but produced 1,317,328 bytes. [Release profile](../tools/build-release.sh)

x86-64 の旧 glibc 版から初回の静的版への増加は、主に機械語の 123,502 バイトと読み取り専用データの 29,464 バイトでした。今回の配布版では標準ライブラリも同じ設定でビルドし、x86-64 のフレームポインタと最適化設定を比較しました。速度を優先した設定でも、実行ファイルは Debian の Bash より 992 バイト小さくなりました。x86-64 のアンワインド情報は合計 163,124 バイトから 2,476 バイトに減り、逆アセンブルでも AVX2 と BMI2 の命令を確認しました。macOS は署名・arm64・libSystem への直接リンク、Linux は ELF の機種・PIE・動的ローダーと共有ライブラリ依存がゼロであることを検査します。両方の Linux バイナリで空のルートから二つの起動名を使い、計算結果をパイプで受け取れました。

On Debian arm64, the first allocator trial retained 35,484 KiB of RSS after a 16 MiB variable was unset. The final build uses the reusable Rust heap for allocations below 256 KiB and the statically linked musl allocator for larger blocks. This lets the OS reclaim the large buffers on free. The median RSS of seven sessions is shown below. The 10,000 glob results were held as positional parameters, then cleared before creating the variable. [RSS checkpoints](../tools/bench_heap.py)

| RSS (KiB) | Ready | 10k glob matches | Cleared | 16 MiB variable | Unset |
| --- | ---: | ---: | ---: | ---: | ---: |
| Previous Tonari | 1,172 | 2,200 | 1,664 | 18,024 | 1,636 |
| Rust heap for every size (trial) | 1,240 | 2,644 | 2,648 | 35,484 | 35,484 |
| Tonari | 1,228 | 2,636 | 2,640 | 19,028 | 2,640 |
| Bash | 2,952 | 4,924 | 4,924 | 21,312 | 4,924 |

最終版は16 MiBの変数を解放すると、作成前の 2,640 KiB まで常駐メモリが戻りました。1万件のファイル名展開後に再利用用として保持するメモリは前の配布版より約1 MiB多く、同じ処理後の Bash より約2.2 MiB少なくなっています。大きな割り当てに使う musl は実行ファイルに含まれます。x86-64 の Debian でも同じ手順を5回実行し、変換実行下の `unset` 後の RSS は試作の 38,636 KiB から 5,800 KiB に低下しました。

The M1 ran macOS 27.2.1 and an arm64 Linux VM. The Debian x86-64 timings use translated amd64 userspace on this machine. The separate QEMU system guest checks real x86-64 Linux process and terminal semantics. The results below give medians in milliseconds. The detailed table includes p95 when collected and uses `NA` for unavailable measurements. Previous refers to the distribution binaries from `5b47da7`. Bash is 5.2.37 on macOS and Debian. [All priority measurements](compatibility-2026-11-23-release/priority.tsv)

| Workload / 操作 | macOS arm64 (previous / Tonari / Bash) | Debian x86-64 translated (previous / Tonari / Bash) |
| --- | ---: | ---: |
| Keystroke echo, recent prefix (100k history) | 0.013 / 0.015 / — | 0.069 / 0.068 / — |
| Oldest suggestion (1M history) | 9.463 / 7.165 / — | 15.120 / 11.053 / — |
| Oldest Ctrl-R match (1M history) | 9.538 / 6.939 / — | 14.374 / 11.632 / — |
| Cancel search (1M history) | 0.048 / 0.043 / — | 0.921 / 1.147 / — |
| File completion (100k files) | 39.317 / 39.584 / 110.349 | 17.703 / 17.362 / 65.435 |
| Command completion (100k PATH entries) | 39.249 / 39.423 / 1620.146 | 17.851 / 18.885 / 850.326 |
| Login, exec to first prompt | 4.836 / 4.671 / 8.778 | 71.318 / 74.330 / 94.763 |
| 16 external commands | 23.281 / 23.262 / 37.712 | 162.369 / 163.581 / 189.239 |
| 4,096 UTF-8 filenames | 7.109 / 6.970 / 17.851 | 11.107 / 10.640 / 11.102 |
| Functions | 8.965 / 8.874 / 16.939 | 33.995 / 24.435 / 34.144 |
| 4 MiB through three external processes | 8.950 / 9.663 / 12.503 | 39.465 / 42.155 / 51.731 |
| 4 MiB through counted read | 30.367 / 12.805 / 85.864 | 95.617 / 54.753 / 162.292 |

100万件の履歴は PTY でキー入力から表示までを21回、10万件のファイル・PATH 補完は9回計測しました。スクリプト47件は起動後の実行時間を取り、すべての出力と終了値を検査します。ストリームは0バイトから4 MiBまで、次の読み手に渡る内容も含めて確認しました。機能ごとの目標は Bash の所要時間の 2/3 以下です。x86-64 の変換実行の値には、実行ファイルの起動やシステムコールにかかる変換コストが含まれます。

- macOS arm64 had 37/47 script workloads at or below two-thirds of Bash time, 47/47 faster than Bash.
- Translated Debian x86-64 had 12/47 script workloads at or below two-thirds of Bash time, 35/47 faster than Bash. The full feature audit also times the individual matching witnesses. [Feature matrix](compatibility-2026-11-23-release/matrix.md) · [Changes against the preceding timing snapshots](compatibility-2026-11-23-release/target-crossings.tsv)

On the M1, an interleaved comparison of otherwise identical builds before and after bulk copying in `read -r -N` measured 33.753 / 12.575 ms at 4 MiB (Bash 86.612 ms), 9.212 / 7.660 ms at 256 KiB, and 7.212 / 7.220 ms at 4 KiB. Each binary ran 31 times per size after warmup.

| Native arm64 Linux VM / 操作 | Former glibc | Previous static | Tonari | Bash |
| --- | ---: | ---: | ---: | ---: |
| Debian start and exit | 0.377 | 0.250 | 0.245 | 0.492 |
| Debian interactive login and exit | 1.763 | 1.943 | 1.573 | 2.213 |
| Debian exec to first login prompt | 1.825 | — | 1.650 | 2.485 |
| Ubuntu start and exit | 0.371 | 0.247 | 0.245 | 0.485 |
| Ubuntu interactive login and exit | 2.603 | 2.792 | 2.426 | 3.335 |
| Ubuntu exec to first login prompt | 2.703 | — | 2.599 | 3.668 |

Under x86-64 translation, 31 rotating samples gave 10.51 ms for plain start/exit with the selected build and 10.70 ms with the smaller `-Oz` trial. Interactive login through exit took 73.43 and 69.83 ms respectively. The `-Oz` trial predates the bulk counted-read and large-allocation changes. The selected profile favored arithmetic and function throughput. Native x86-64 performance remains unmeasured.

x86-64 の変換実行では、サイズをさらに削った試作にログイン時間で優位がありました。通常起動と繰り返し実行する算術・関数は採用した設定の方が速く、実機での判断は今後の課題です。両ターゲットとも、2/3 を超えた項目や、Bash・前の Tonari が速かった項目を実測表に残しています。

The arm64 Linux login comparison used 201 alternating runs, with a private HOME copied from each distribution’s skeleton. The PTY measurement (101 runs) ends when the prompt set by the last profile line appears. The interactive-login row times `--login -ic :` through exit. [Login harness](../tools/bench_login.py) · [History](../tools/bench_history.py) · [Completion](../tools/bench_completion.py) · [Scripts](../tools/bench_scripts.py) · [Byte streams](../tools/bench_streams.py)

<a id="static-linux"></a>

## First static Linux releases / 初回の Linux 静的配布版

The release script produced static PIEs on both GNU/Linux rustup hosts using Rust 1.96.1. Each ELF had zero `PT_INTERP` and `DT_NEEDED` entries. We copied just the executable into an empty root, then ran arithmetic, a function and a builtin pipeline under `chroot`. Each architecture printed `static 42`. Linux pathname expansion now sorts in Rust by UTF-8 bytes in every locale. [Build](../tools/build-release.sh) · [Behavioral comparison](compatibility.md#intentional-differences)

| Build / ビルド | Tonari bytes | Previous glibc bytes | Debian Bash bytes | Tonari SHA-256 |
| --- | ---: | ---: | ---: | --- |
| Linux arm64 | 1,369,056 | 1,257,592 | 1,412,096 | `1a783c0b803c1abfa1fe37d71ccd9de808e8d245c332ec8d1da261f7555c4055` |
| Linux x86-64-v3 | 1,420,928 | 1,279,184 | 1,298,416 | `71a43c60a2c3d57eea8a63ee4528d9c228d8f270c571ef59ef0db1263df9b2c5` |

On x86-64-v3 the standalone executable exceeds Debian Bash by 122,512 bytes. An otherwise identical build forcing LLVM machine outlining grew to 1,439,104 bytes. The native Alpine arm64 toolchain produced a 1,348,816-byte static PIE and passed the Debian integration suite. The timings below use the rustup arm64 release on the same Apple M1 Linux VM. Bash is 5.2.37 on Debian 13 and 5.3.9 on Ubuntu 26.04. The tables give median / p95 in milliseconds. Five warmups preceded 151 rotating process-launch samples or 45 already-started glob samples per shell. The glob fixtures use `LC_ALL=C.UTF-8`. [Launch benchmark](../tools/bench_login.py) · [Glob benchmark](../tools/bench_scripts.py)

| OS | Workload / 操作 | Previous glibc | Static Tonari | Bash |
| --- | --- | ---: | ---: | ---: |
| Debian | Start and exit | 0.361 / 0.585 | 0.240 / 0.409 | 0.465 / 0.704 |
| Debian | Login and exit | 1.163 / 1.389 | 1.162 / 1.453 | 1.422 / 1.739 |
| Debian | Interactive login and exit | 1.684 / 2.051 | 1.804 / 2.169 | 2.100 / 2.480 |
| Debian | 16 external launches | 3.758 / 4.301 | 3.403 / 3.986 | 4.816 / 5.577 |
| Debian | 4,096 UTF-8 filenames | 1.946 / 2.127 | 1.962 / 2.284 | 3.743 / 4.139 |
| Debian | Same glob, eight times | 9.421 / 10.306 | 9.034 / 9.870 | 28.645 / 29.433 |
| Ubuntu | Start and exit | 0.373 / 0.726 | 0.246 / 0.460 | 0.475 / 0.898 |
| Ubuntu | Login and exit | 1.014 / 1.278 | 1.036 / 1.242 | 1.322 / 1.590 |
| Ubuntu | Interactive login and exit | 2.427 / 3.010 | 2.544 / 3.136 | 3.060 / 3.726 |
| Ubuntu | 16 external launches | 3.742 / 4.309 | 3.373 / 4.010 | 4.734 / 5.230 |
| Ubuntu | 4,096 UTF-8 filenames | 1.950 / 2.361 | 2.015 / 2.280 | 3.721 / 4.097 |
| Ubuntu | Same glob, eight times | 9.396 / 9.725 | 8.997 / 9.697 | 27.747 / 29.499 |

空のルートで実行できることを両アーキテクチャで確認しました。arm64 の通常起動は両環境で旧 glibc 版より約34%短くなりました。プロファイルを読む対話ログインは旧版より遅く、外部コマンド16回の連続起動は Bash の約71%です。x86-64-v3 のサイズは Debian Bash を約120 KiB 上回っています。x86-64 の機能試験には QEMU を使い、実機や WSL2 の速度は測定していません。[Linux 機能別の計測](compatibility-2026-11-23-static/matrix.md)

With the same 4,096 filenames and `--utf8-locale ja_JP.UTF-8`, the chosen ordering changes the amount of work. In a separate byte-for-byte check all three shells returned the same set, Tonari in byte order and the other two in the Japanese locale's collation order. The timings use 45 samples in already-started shells. [Glob workload and locale option](../tools/bench_scripts.py)

| OS | ja_JP.UTF-8 | Previous glibc | Static Tonari | Bash |
| --- | --- | ---: | ---: | ---: |
| Debian | One glob | 21.976 / 22.552 | 1.963 / 2.248 | 20.864 / 21.521 |
| Debian | Eight globs | 29.225 / 32.163 | 9.161 / 10.461 | 164.699 / 168.002 |
| Ubuntu | One glob | 21.983 / 23.066 | 2.015 / 2.273 | 20.905 / 21.544 |
| Ubuntu | Eight globs | 29.330 / 31.247 | 9.025 / 10.316 | 165.078 / 170.049 |

日本語ロケールの4,096件では、各ファイル名が1回ずつ含まれることと、Tonari の全引数がバイト順であることも確認しました。Debian の単発展開は旧版 21.976 ms から 1.963 ms に短縮しました。旧版と Bash はロケールの照合順序を計算しています。

The complete Linux feature run verified every timed observation. With shell startup included, 510 of 777 matching Debian checks and 495 of 773 matching Ubuntu checks ran in at most two-thirds of Bash time. The comparison with the preceding static arm64 snapshot found 133 feature/mode pairs moving above the two-thirds threshold. These include short process and synchronization measurements. The TSV gives both shells’ medians in each run. [Full matrix](compatibility-2026-11-23-static/matrix.md) · [Threshold crossings](compatibility-2026-11-23-static/target-crossings.tsv)

Linux の全項目計測で起動時間を含めた中央値が Bash の 2/3 以下だったのは Debian 510/777件、Ubuntu 495/773件です。前回の静的版より閾値を超えた133組についても、両者の絶対時間を記録しました。

<a id="macos-runtime-dependencies"></a>

## macOS release linkage / macOS 配布版のリンク

On 2026-11-23 we rebuilt the release from `52e2148` on an Apple M1 running macOS 27.2.1 with Rust 1.96.1 and added `-dead_strip_dylibs` to the release profile. Both builds were 1,202,848 bytes. For `-c :`, 201 interleaved samples per executable gave median / p95 times of 2.660 / 3.165 ms with the flag, 2.702 / 3.198 ms for the previous release and 5.567 / 6.324 ms for Bash 5.2.37. The new executable's SHA-256 is `270d22e641edbfabefc6fe73c432c98e7f5116f38b6608e25981517573d60241`. The feature matrix below identifies the earlier binary by its own digest. [Startup benchmark](../tools/bench.py) · [Release profile](../tools/build-release.sh)

同一のシェル実装で未使用の共有ライブラリを除く設定を比較しました。macOS の統合テストは123件通過、Linux 専用の1件はスキップでした。起動時間の差は小さく、採用理由は直接リンクするライブラリが1本減ったことです。[依存方針と検証方法](engineering-notes.md#runtime-dependencies)

<a id="interactive-response"></a>

## Interactive response and sustained work / 対話応答と連続実行

We timed the staged binaries from `8e9963e` on an Apple M1 and on Debian 13 and Ubuntu 26.04 arm64 in a Linux VM on the same machine. The table gives elapsed milliseconds. Parentheses give the 95th percentile. In the TSV, `NA` means the benchmark reported only a median. The Previous column uses the release preceding this series, including its static musl build on Linux arm64. Each history measurement opens a fresh PTY with a million journal entries. Keystroke and completion times end when the expected bytes arrive at the PTY. The whole-prompt measurement starts in the launching parent. [Measurements including sample counts](compatibility-2026-11-23-response/priority.tsv) · [Benchmark programs](../tools)

| macOS workload / 操作 | Previous Tonari | Tonari | GNU Bash 5.2.37 |
| --- | ---: | ---: | ---: |
| First visible recent suggestion, 1M entries / 最近の履歴候補 | 107.444 (117.439) | 0.176 (0.187) | — |
| First visible oldest suggestion, 1M entries / 最古の履歴候補 | 112.680 (119.178) | 8.657 (9.073) | — |
| Ctrl-R, oldest match / 最古の履歴検索 | 213.210 (224.823) | 8.395 (9.332) | — |
| Cancel Ctrl-R / 検索の取り消し | 71.979 (72.738) | 0.037 (0.050) | — |
| 512 bytes of typeahead / 連続入力 | 6.709 (7.063) | 0.284 (0.301) | — |
| 16 KiB bracketed paste / 貼り付け | 22.995 (49.680) | 2.523 (3.751) | — |
| Filename Tab among 10,000 / ファイル補完1万件 | 5.168 (5.410) | 3.639 (3.857) | 10.023 (10.478) |
| Filename Tab among 100,000 / ファイル補完10万件 | 51.489 (53.491) | 37.635 (38.493) | 105.384 (105.838) |
| List 10,000 candidates / 候補1万件の一覧 | 11.162 (15.082) | 8.760 (11.586) | 39.522 (49.462) |
| Reused 10,000-word completion / 固定候補の反復 | 0.237 (0.475) | 0.023 (0.067) | 24.357 (26.204) |
| Start and show first prompt / 初回プロンプト | — | 4.189 (4.446) | 6.740 (6.967) |
| Expand 4,096 mixed UTF-8 names once / ファイル名展開 | 11.217 (18.195) | 9.408 (12.850) | 19.524 (26.742) |
| Same expansion eight times / 同じ展開を8回 | 83.482 (125.611) | 25.829 (38.246) | 145.854 (211.414) |
| Read 4 MiB from a pipe with `read -N` | 2403.384 | 35.275 | 94.657 |
| Capture a 4 MiB file with `$(<file)` | 15.628 | 9.273 | 62.941 |
| String expansion workload / 文字列展開 | 5.114 (5.290) | 5.296 (5.556) | 11.598 (11.711) |
| Function workload / 関数呼び出し | 8.917 (9.211) | 9.011 (9.156) | 17.485 (17.899) |

普段の1打鍵も別に測りました。macOS では最近の候補がある場合、入力から再描画まで中央値0.013 ms、p95 は0.018 ms でした。初めて古い履歴を探す場合と、既に表示中の行を編集する場合とでは仕事量が違います。短い履歴の最古候補を表示する別の試験では、旧版0.651 ms、今回0.909 msでした。10万ファイルの初回補完は38.493 ms（p95）かかり、利用者が登録した補完関数が100 ms待つ例では、Tab の直後の文字が見えるまで中央値113.459 msでした。端末エミュレーターが描画を終えるまでの時間は含みません。[入力と補完の全結果](compatibility-2026-11-23-response/priority.tsv)

| Debian arm64 workload / 操作 | Tonari (glibc) | GNU Bash 5.2.37 | Tonari / Bash |
| --- | ---: | ---: | ---: |
| First prompt / 初回プロンプト | 0.759 | 0.968 | 0.78 |
| Filename Tab among 100,000 / ファイル補完10万件 | 15.365 | 54.303 | 0.28 |
| Expand 4,096 mixed UTF-8 names / ファイル名展開 | 1.882 | 4.031 | 0.47 |
| Launch and wait for 16 external commands / 外部コマンド16連続 | 3.488 | 4.785 | 0.73 |
| 4 MiB redirected through three external commands / 3段パイプ | 5.279 | 5.584 | 0.95 |
| Read 4 MiB with `read -N` / パイプ入力 | 28.003 | 95.637 | 0.29 |
| 4 MiB input redirect / 入力と出力のリダイレクト | 2.869 | 3.037 | 0.94 |
| 4 MiB `$(<file)` / ファイルのコマンド置換 | 9.599 | 71.233 | 0.13 |
| Function workload / 関数呼び出し | 9.803 | 24.320 | 0.40 |
| Unicode field splitting / 絵文字を区切りにした単語分割 | 3.616 | 23.544 | 0.15 |

The direct C launch/wait control took 3.500 ms for those same 16 children. All 47 longer scripts had lower median times than Bash on each of the three platforms. The counts at or below two-thirds were 39, 35 and 31 respectively. The per-feature audit also times short invocations, external helpers, sleeps, and tests shared by several names. Its raw ratios differ substantially from the longer scripts. OS caches were warm. Separate audit lanes used the same host. After the audits finished, we repeated macOS history and first Tab and repeated startup and first prompt on all three platforms. A second TSV retains the measurements during the concurrent run. The seven-run 100,000-file p95 is the slowest measured sample. The byte-stream benchmark checks every byte and reports medians at 0 bytes, 1 byte, 4 KiB, 256 KiB and 4 MiB. [Whole and already-started feature matrix](compatibility-2026-11-23-response/matrix.md) · [Measurements during concurrent audits](compatibility-2026-11-23-response/priority-concurrent.tsv) · [Per-feature threshold crossings](compatibility-2026-11-23-response/target-crossings.tsv)

機能別の短い呼び出しも起動込みで再計測しました。中央値が Bash の2/3以下になったのは、macOS 620/757件、Debian 46/778件、Ubuntu 28/774件です。[判定した全項目の実測](compatibility-2026-11-23-response/matrix.md)

CPUを1個に制限した Debian でも、100万件中の最古側の検索は10.906 ms、UTF-8 ファイル4,096件の展開は1.906 ms（Bash 4.500 ms）でした。外部プロセスの起動や大容量パイプでは OS の時間も含めて測っています。同じホスト上で監査を並行実行した計測もあります。実機の x86-64、WSL2、低速ストレージ上のコールドスタートは今回の計測対象外です。[CPU 1個と各環境の実測](compatibility-2026-11-23-response/priority.tsv)

GNU’s `ifs-posix.tests` performs 6,856 assertions and repeatedly uses a parenthesized pipeline stage inside command substitution. Running that stage in the child already created for the pipe also fixes the observable subshell depth. On Ubuntu the seven-run median was 2031.257 ms, compared with 2995.191 ms before this fix and 2465.075 ms with Bash. Every run matched the upstream output. [Pipeline and subshell regression](../tests/shell.py)

### Release binaries / 配布バイナリ

| Target | Bytes | Reference Bash bytes | SHA-256 |
| --- | ---: | ---: | --- |
| macOS arm64 | 1,202,848 | 1,208,904 | `c6f84927cc5352311cc74315e01393126aa43e08de93227799cdd952b0f49ac2` |
| Linux arm64 glibc | 1,257,592 | 1,412,096 Debian · 1,609,176 Ubuntu | `52116bd709de110790d9f2b20a3ae91daebdaf6418be14de69268ab63e600ca0` |
| Linux arm64 static musl | 1,359,104 | 1,412,096 Debian · 1,609,176 Ubuntu | `67b9facb2bc45a40e9515826f9ed1bfc93414ea7d082b5c05c4125b3575960ec` |
| Linux x86-64-v3 glibc | 1,279,184 | 1,298,416 Debian | `0964bc2cdc5d38cbf9897adb0524771dd22fcaeeaa9030e4ed09a0f45eaa7957` |

The Linux glibc build uses the installed locale data for filename order. In the same Debian startup run, `-c :` took 0.341 ms with glibc, 0.241 ms with static musl and 0.447 ms with Bash. The previous release used musl on arm64. At the time of this comparison, `linux-arm64` contained the glibc build and `linux-arm64-musl` contained the static build. A fully static glibc trial started in 0.370 ms compared with 0.467 ms for dynamic glibc in that run, and grew to 2,169,008 bytes. In a separate 301-sample build comparison, linking GCC’s unwind support into the executable reduced the glibc startup median from 0.352 to 0.325 ms and p95 from 0.439 to 0.391 ms. The release script records the compiler and linker options used for each target. [Build recipe](../tools/build-release.sh) · [Static musl measurements](compatibility-2026-11-23-response/musl/matrix.md)

Linux arm64 では日本語を含む名前付きロケールのファイル順を扱える glibc 版を標準としました。起動の速さを優先してバイト順でよい環境には、静的 musl 版があります。どちらもインストールする実行ファイルは1本です。検証した4本はそれぞれの比較用 Bash より小さく、アカウントでのログイン試験には配布用の実体を使いました。[互換性と適用条件](compatibility.md) · [採用・見送りの根拠](engineering-notes.md)

On macOS set `REFERENCE_BASH` to a modern GNU Bash for the full script suite. On the two Linux reference systems the packaged interpreter is `/bin/bash`. macOS では全スクリプト計測に使う GNU Bash 5.2 以降のパスを `REFERENCE_BASH` に設定します。

```sh
binary=$(tools/build-release.sh)
reference_bash=${REFERENCE_BASH:-/bin/bash}
python3 tools/bench_history.py "$binary" --entries 1000000 --bursts --runs 17
python3 tools/bench_completion.py "$binary" --bash "$reference_bash" --files 100000 --runs 7
python3 tools/bench_completion.py "$binary" --bash "$reference_bash" --reuse-words --runs 15
python3 tools/bench_scripts.py "$binary" --bash "$reference_bash" --timing started --runs 17
python3 tools/bench_streams.py "$binary" "$reference_bash" --case 'read -N' --sizes 4096 262144 4194304 --runs 7
```

<a id="performance-target"></a>

## Performance target / 性能目標

For each workload, divide Tonari's elapsed time by GNU Bash's elapsed time on the same machine and in the same run. A ratio of **0.50** is the goal and **2/3** is the upper target. For very short operations and workloads dominated by the same OS call or wait, parity with Bash is acceptable. Use the absolute time difference and observed variation to judge parity. A loop in an already-started shell can show whether a small per-operation difference accumulates. The harness reports both median and p95 ratios, and labels the median result. Direct executable launch time and binary size are reported separately.

各処理で、同じマシン・同じ計測での Tonari の所要時間を GNU Bash の所要時間で割ります。目標は **0.50**、許容する上限は **2/3** です。ごく短い単発処理や、共通の OS 呼び出し・待ち時間が支配的な場合は Bash と同程度なら許容します。同等かどうかは絶対時間差と測定のばらつきから判断し、頻繁に繰り返す操作は起動済みシェルでの反復時間も見ます。計測器は中央値と p95 の比を表示し、中央値に対する達成状況を示します。実行ファイルの直接起動とバイナリサイズは別項目です。

```sh
python3 tools/bench.py target/release/tonari --bash /bin/bash --runs 300
python3 tools/bench.py target/release/tonari --case session --runs 300
python3 tools/bench.py target/release/tonari --case core --compare-tonari Previous /path/to/previous/tonari --runs 300
```

The default run includes a session workload covering assignment, export, parameter expansion, `cd`, `pwd`, `&&`, redirection and `unset`. It also times 1,000 arithmetic-loop iterations and 16 consecutive external commands inside one shell. For the latter it rotates in a Python parent launching the same 16 executables with `posix_spawn` and waiting for each. The launcher configures standard descriptors before timing; each child inherits them. Use `--case core` for comparisons with binaries predating session support. The primary prompt time spans the complete PTY launch. The harness also timestamps the child immediately before exec to show how much of that same sample follows the launcher's terminal setup and fork.

既定の計測には、代入・export・変数展開・`cd`・`pwd`・`&&`・リダイレクト・`unset` を同じシェルで実行する項目が含まれます。算術ループ1,000回と、一つのシェルから外部プログラム16個を順に起動する項目もあります。後者では同じプログラム16個を Python 親プロセスから直接 `posix_spawn` し、その都度終了を待つ条件も交互に計測します。標準入出力は計時前に用意し、各子プロセスに継承させます。セッション機能の実装前との比較には `--case core` を使います。プロンプト表示の主指標は疑似端末での起動全体です。子プロセスの exec 直前にも時刻を記録し、同じ測定サンプルのうち、端末の準備と fork の後に経過した時間も表示します。

<a id="nocasematch"></a>

## Case-insensitive matching and login / 大文字小文字の照合とログイン

We tested release `fc25e76` against Bash on the same Apple M1 and its Linux guests. Each login and prompt row used 101 samples. The running-shell scripts used 51 samples and verified their output on every run. After the full feature audit and its timings finished, we reran these focused benchmarks one platform and one workload at a time. The full feature matrices used parallel Linux guests. The Bash and Tonari binaries were on the same local filesystem. [Login harness](../tools/bench_login.py) · [PTY harness](../tools/bench.py) · [Script workloads](../tools/bench_scripts.py)

各セルは中央値 / p95（ms）です。`--login -ic :` はシステムとユーザーの起動設定を読んで終了するまで、PTY は設定を無効にした実際の端末で最初のプロンプトが見えるまでを計ります。照合スクリプトは1,500回、通常の `case` は3,000回繰り返しました。

| Platform | Operation | Tonari | Bash | Median ratio |
|---|---|---:|---:|---:|
| Debian | `-c :` | 0.252 / 0.369 | 0.476 / 0.686 | 0.529 |
| Debian | `--login -ic :` | 1.847 / 2.173 | 2.118 / 2.391 | 0.872 |
| Debian | first prompt (PTY) | 0.659 / 1.023 | 1.045 / 1.664 | 0.631 |
| Debian | `nocasematch` | 3.918 / 4.361 | 9.093 / 10.316 | 0.431 |
| Debian | case-edges | 3.485 / 3.884 | 9.591 / 10.241 | 0.363 |
| Ubuntu | `-c :` | 0.254 / 0.550 | 0.469 / 1.281 | 0.541 |
| Ubuntu | `--login -ic :` | 2.659 / 3.482 | 3.179 / 3.912 | 0.836 |
| Ubuntu | first prompt (PTY) | 0.724 / 1.306 | 1.129 / 2.007 | 0.642 |
| Ubuntu | `nocasematch` | 3.893 / 4.208 | 7.715 / 8.146 | 0.505 |
| Ubuntu | case-edges | 3.524 / 3.878 | 8.039 / 8.316 | 0.438 |
| macOS | `-c :` | 3.778 / 4.594 | 6.572 / 7.360 | 0.575 |
| macOS | `--login -ic :` | 5.652 / 6.756 | 9.355 / 10.278 | 0.604 |
| macOS | first prompt (PTY) | 4.457 / 5.074 | 7.326 / 8.120 | 0.608 |
| macOS | `nocasematch` | 4.871 / 6.320 | 7.617 / 9.245 | 0.640 |
| macOS | case-edges | 4.083 / 4.490 | 7.529 / 8.108 | 0.542 |

On macOS the earlier run, alongside the audit, recorded 7.234 ms for the 1,500-iteration script, 8.762 ms before the single-star optimization and 10.945 ms for Bash. After that change, whole-string comparison for common prefix/suffix patterns avoids constructing candidate endpoints. The serial sample above uses the same executables. 起動設定を含む対話ログインは、並列検証中の中央値でも Debian 0.922、Ubuntu 0.792、macOS 0.600（Bash 比）でした。

In a running macOS PTY with 100,000 history entries, typing a key with no matching suggestion took 0.021 ms median and 0.113 ms p95. Tab with 10,000 PATH executables took 5.744 / 7.253 ms for Tonari and 181.275 / 223.520 ms for Bash over 41 samples. [Input and completion benchmark](../tools/bench_completion.py)

Upper-target crossings compare to the preceding session-controls release. The table counts named checks. More than one name may use the same workload. Exact timings include any waits, process launches and terminal I/O in that check. [Before/after timings for crossings](compatibility-2026-11-23-nocasematch/target-crossings.tsv) · [Complete matrix](compatibility-2026-11-23-nocasematch/matrix.md)

| Platform | Mode | Measured | ≤ 1/2 | ≤ 2/3 | ≤ Bash | Crossed 2/3 |
|---|---|---:|---:|---:|---:|---:|
| Debian | whole | 777 | 14 | 457 | 766 | 80 |
| Debian | started | 677 | 6 | 42 | 570 | 20 |
| Ubuntu | whole | 773 | 16 | 430 | 758 | 63 |
| Ubuntu | started | 675 | 7 | 25 | 579 | 22 |
| macOS | whole | 757 | 2 | 629 | 756 | 38 |
| macOS | started | 674 | 8 | 76 | 655 | 25 |

前の版で一致した項目の再判定と再計測に成功しました。すべての処理が目標の2/3以内に達したわけではありません。特に Linux の起動設定を含む対話ログインでは上限を超えています。

| Executable | Tonari bytes | Reference Bash bytes | SHA-256 |
|---|---:|---:|---|
| macOS arm64 | 1,202,336 | 1,208,904 | `272154ae6c75fa1a7dfffdb6a393380d0d4d83d2df613aa6d9824f9c876eba42` |
| Debian / Ubuntu arm64 | 1,411,976 | 1,412,096 / 1,609,176 | `864a1e2d32aef82cea7609cb7f89334c25ca850780b7ed92d405572894bb414c` |
| Debian x86-64-v3 | 1,296,328 | 1,298,416 | `4d03b17446eff610f1d7e414b878fd3b8ba48ed40a06fbf1d04ec9b2948a1ca4` |

Linux arm64 shares one static musl executable. On Debian its on-disk size is 120 bytes below Bash. The x86-64-v3 binary uses the system glibc and libgcc. x86-64 functional tests ran under QEMU. 実機 x86-64 と WSL2 の性能は未検証です。

<a id="session-controls"></a>

## 2026-11-23 · Session controls and process descriptors / 対話操作とプロセスの計測

We tested `67c48d6` on Apple M1 arm64 with macOS and two Linux guests. The macOS reference was GNU Bash 5.2.37, Debian 13 used its Bash 5.2.37 and Ubuntu 26.04 used 5.3.9. Every core timing below used the same Tonari binary as its compatibility audit. [Per-feature matrix](compatibility-2026-11-23-shell/matrix.md) · [Fingerprints and run settings](compatibility-2026-11-23-shell/environment.json)

For startup we copied the three executables to the same filesystem under the name `bash`, rotated launch order, used five warm-ups and kept 101 samples per shell. The previous Tonari was `1507031`. Login cases read the real system profile and a temporary home with the distribution skeleton. The parent measured process launch through exit with stdin disconnected. Values are **median / p95 in milliseconds**. [Login benchmark](../tools/bench_login.py)

起動試験では同じファイルシステム上に各実行ファイルを配置し、実行順序を交替しました。ログイン試験にはシステムとユーザーの設定を読む時間も含まれます。Linux の対話ログインは `--login -ic :` の起動から終了までで、端末の最初の画面を描く時間は別の指標です。3環境は同じ物理マシンを共有し、検証作業の一部は並行して実行しました。

```sh
python3 tools/bench_login.py target/release/tonari --bash "$reference_bash" --previous "$previous_tonari" --runs 101
```

| Platform | Workload | Bash | Previous | Tonari | Tonari / Bash median |
|---|---|---:|---:|---:|---:|
| Debian | `-c :` | 0.850/1.911 | 0.428/0.947 | 0.451/1.100 | 0.531 |
| Debian | `-lc :` | 2.558/5.023 | 2.505/3.729 | 2.157/3.311 | 0.843 |
| Debian | `-lic :` | 4.383/7.984 | 4.331/7.632 | 3.653/6.837 | 0.833 |
| Debian | 16 × `/usr/bin/true` | 11.345/25.519 | 8.177/17.520 | 8.329/16.659 | 0.734 |
| Ubuntu | `-c :` | 0.708/1.937 | 0.368/1.385 | 0.388/1.713 | 0.548 |
| Ubuntu | `-lc :` | 1.804/3.115 | 1.487/2.753 | 1.336/2.218 | 0.741 |
| Ubuntu | `-lic :` | 3.759/4.994 | 4.111/5.920 | 3.252/4.212 | 0.865 |
| Ubuntu | 16 × `/usr/bin/true` | 5.438/7.382 | 3.841/6.000 | 3.858/5.047 | 0.709 |
| macOS | `-c :` | 7.503/8.675 | 4.356/4.970 | 4.326/5.194 | 0.577 |
| macOS | `-lc :` | 10.562/11.922 | 6.536/7.488 | 6.525/7.590 | 0.618 |
| macOS | `-lic :` | 10.887/11.928 | 6.654/7.840 | 6.763/7.883 | 0.621 |
| macOS | 16 × `/usr/bin/true` | 44.142/49.372 | 26.470/30.191 | 26.712/32.969 | 0.605 |

The time from creating a controlling terminal to seeing the first prompt was measured separately with 101 fresh sessions, with user profiles and rc files disabled. Times are median / p95 in milliseconds. 実際の疑似端末を開いてから最初のプロンプトが届くまでの時間です。 [Prompt benchmark](../tools/bench.py)

| Platform | Tonari | Bash | Median ratio |
|---|---:|---:|---:|
| Debian | 0.970 / 2.170 | 1.527 / 2.926 | 0.635 |
| Ubuntu | 1.009 / 1.757 | 1.515 / 3.634 | 0.666 |
| macOS | 4.948 / 6.126 | 8.178 / 10.450 | 0.605 |

We also measured 16 consecutive external commands inside an already-running shell against a C program issuing the same `posix_spawn` and `waitpid` calls, with 51 samples. On Debian the median was 3.866 ms for Tonari, 3.894 ms for C and 5.374 ms for Bash. On Ubuntu it was 3.950 ms, 3.868 ms and 5.229 ms respectively. The Linux launch series exceeded the two-thirds ratio on both guests and ran within 0.09 ms of the direct C control. [Native control](../tools/bench_spawn.c)

外部プロセスを16回直列に起動した場合、Tonari の中央値は Debian で Bash 比0.719、Ubuntu で0.755でした。各起動を待つ C の対照と照合すると、16回全体での差は Debian で −0.028 ms、Ubuntu で +0.082 ms でした。

The script harness measured 21 checked samples in a running shell. Cells show **Tonari median / Bash median in milliseconds**, followed by the ratio in parentheses. The source specifies the loop counts and checks the output after every launch. [Workload definitions](../tools/bench_scripts.py)

スクリプトの反復処理では起動済みシェルを使い、実行結果も毎回確認しました。各セルは Tonari と Bash の中央値とその比率です。

| Workload | Debian | Ubuntu | macOS |
|---|---:|---:|---:|
| arithmetic | 7.526 / 27.217 (0.277) | 6.599 / 22.438 (0.294) | 9.188 / 20.148 (0.456) |
| functions | 10.252 / 26.760 (0.383) | 10.459 / 25.285 (0.414) | 15.796 / 27.364 (0.577) |
| case-edges | 4.288 / 10.645 (0.403) | 3.875 / 8.642 (0.448) | 5.235 / 11.428 (0.458) |
| substitutions | 0.523 / 18.459 (0.028) | 0.481 / 15.607 (0.031) | 0.812 / 72.677 (0.011) |
| read-lines | 7.625 / 11.642 (0.655) | 7.274 / 10.743 (0.677) | 7.186 / 11.154 (0.644) |

For input latency we measured from a key sent to a running macOS PTY to the visible result. The history journal contained 100,000 entries. The directory contained 10,000 executable files and the completion trials ran 41 times. [Completion benchmark](../tools/bench_completion.py)

キーを送ってから画面に反映されるまでを測りました。古い履歴候補の検索中でも入力できるため、入力文字と候補の描画時間を個別に記録しています。単位は中央値 / p95 のミリ秒です。

| Keystroke or result | Tonari | Bash |
|---|---:|---:|
| recent hint | 0.016 / 0.051 | — |
| no hint | 0.017 / 0.049 | — |
| oldest hint: typed key | 0.018 / 0.039 | — |
| oldest hint: visible suggestion | 0.766 / 1.287 | — |
| files | 5.857 / 7.848 | 11.839 / 13.314 |
| complete -f | 5.902 / 8.300 | 12.366 / 15.062 |
| PATH | 5.655 / 6.688 | 179.967 / 236.761 |

For programmable completion and the second-Tab listing we used 31 samples per shell. Times are median / p95 in milliseconds. 登録済み候補と2回目の Tab による一覧を、起動済み端末で測っています。

| Operation | Tonari | Bash |
|---|---:|---:|
| static, all | 0.348 / 0.920 | 26.904 / 48.136 |
| static, unique | 0.243 / 0.512 | 26.040 / 34.223 |
| dynamic, unique | 1.170 / 2.019 | 2.692 / 4.675 |
| list 10 | 0.085 / 0.118 | 0.122 / 0.187 |
| list 10000 | 8.146 / 9.193 | 38.336 / 40.772 |

| Executable bytes | Tonari | Reference Bash |
|---|---:|---:|
| macOS arm64 | 1,202,304 | 1,208,904 |
| Debian arm64 | 1,403,584 | 1,412,096 |
| Ubuntu arm64 | 1,403,584 | 1,609,176 |
| Debian x86-64-v3 | 1,289,112 | 1,298,416 |

The arm64 Linux guests share one static musl executable. The x86-64-v3 build links to the distro’s `libc.so.6` and `libgcc_s.so.1`. macOS links to system libraries. The table measures executable bytes on disk. Debian x86-64 passed all 116 integration tests under its own kernel in QEMU. Native x86-64 and WSL2 performance measurements are pending.

Linux arm64 は静的 musl、x86-64-v3 は OS の共有ライブラリにリンクして、各ターゲットに実行ファイルを1本配置しました。x86-64 の機能試験では QEMU 上の Debian カーネル 6.12.117 と Bash 5.2.37 を使用しました。最終版の x86-64 実行ファイルの SHA-256 は `1235d91e038cac44e9f1813eff8f0ba8cfd96d1f8e8752082da842168b9ee4d1` です。

Build the x86-64 artifact on Debian 13 with Rust 1.96.1 and the system linker. The executable requires an x86-64-v3 CPU and a runtime that supports packed relative relocations. x86-64-v3 版の再ビルド条件は次のとおりです。

```sh
CARGO_PROFILE_RELEASE_OPT_LEVEL=z \
  RUSTFLAGS='-C target-cpu=x86-64-v3 -C link-arg=-Wl,-z,pack-relative-relocs' \
  cargo build --release --locked
```

### Per-feature timings / 機能別の計測

Each count is a named compatibility check with verified timing samples. Some names exercise the same operation. The thresholds use raw median times, including waits and external processes. Every previously matched and timed case still matched and produced samples. [All measurements](compatibility-2026-11-23-shell/matrix.md) · [Cases that crossed the upper target since the login snapshot](compatibility-2026-11-23-shell/target-crossings.tsv)

| Platform | Mode | Measured | ≤ 1/2 | ≤ 2/3 | ≤ Bash |
|---|---|---:|---:|---:|---:|
| Debian | whole | 776 | 16 | 456 | 763 |
| Debian | started | 676 | 11 | 42 | 594 |
| Ubuntu | whole | 772 | 19 | 448 | 757 |
| Ubuntu | started | 674 | 6 | 37 | 594 |
| macOS | whole | 756 | 1 | 631 | 756 |
| macOS | started | 673 | 10 | 59 | 655 |

以前の測定で 2/3 以内だった項目のうち、今回上限を超えた件数は、起動込み／起動後の順で Debian 75／30、Ubuntu 50／26、macOS 22／14 です。各機能の正しい出力を確認してから同じ処理を計時しました。個別テストには子プロセスや端末、固定の待ち時間を含むものがあります。Bash を超えた時間や計測回ごとの揺れも比較できるように、環境別 TSV に中央値と p95 を掲載しました。[前回と今回の中央値](compatibility-2026-11-23-shell/target-crossings.tsv)

<a id="path-expansion-order"></a>

## Wildcard expansion and collation / ワイルドカード展開と並べ替え

On 2026-11-22 we rotated Tonari `c2bd297`, the updated native release and GNU Bash 5.2.37 on an M1 running macOS 27.2.1 and a Debian 13 arm64 VM on the same computer. Each fresh noninteractive shell signaled readiness before sourcing a script using `set -- pattern`. Times include expansion, setting the positional arguments, printing the checked result, and shell exit. The local filesystem cache was warm. Five warm-ups preceded each set of samples. Mac used 45 samples per multilingual workload and 15 otherwise. Debian used 65 per workload. Values are **median / p95 ms per script**. [Script definitions](../tools/bench_scripts.py)

M1 の Mac と同じマシンの Debian VM で、新版・旧版・GNU Bash の実行順を交替した。起動済みの新しい非対話シェルで展開し、結果を位置パラメーターに設定して件数を確認した。ファイルシステムのキャッシュは温めた。各条件5回のウォームアップ後、Mac の多言語名は45回、他は15回、Debian は各65回を集計。表の回数は1スクリプト内での展開回数で、初回の行は毎回新しいシェルで1回だけ展開した。値はスクリプト全体の中央値 / p95（ms）。

| Expansion / 展開 | Mac previous | Mac Tonari | Mac Bash | Debian previous | Debian Tonari | Debian Bash |
|---|---:|---:|---:|---:|---:|---:|
| `*`, 512 names × 30 | 8.198 / 9.586 | 7.873 / 8.850 | 1096.298 / 1149.446 | 5.581 / 6.116 | 4.076 / 4.544 | 8.335 / 9.003 |
| `*.txt`, 512 matches × 30 | 8.126 / 8.871 | 7.872 / 8.205 | 1096.822 / 1325.614 | 5.585 / 6.389 | 4.125 / 4.595 | 8.754 / 9.155 |
| `item-000*.txt`, one match × 30 | 6.592 / 7.195 | 6.545 / 7.255 | 1085.448 / 1163.648 | 2.172 / 2.453 | 2.002 / 2.204 | 2.704 / 2.986 |
| `*em*07.txt`, six matches × 30 | 8.702 / 9.166 | 6.521 / 7.101 | 1077.425 / 1085.483 | 2.799 / 3.164 | 2.021 / 2.277 | 3.219 / 3.686 |
| `*/*.rs`, eight matches × 30 | 26.757 / 31.335 | 10.387 / 11.076 | 1144.234 / 1296.926 | 11.520 / 12.257 | 2.583 / 3.042 | 11.051 / 11.642 |
| `**/*.rs`, 80 matches × 10 | 44.780 / 46.499 | 41.330 / 44.253 | 1061.469 / 1101.253 | 8.495 / 9.537 | 6.787 / 7.434 | 23.588 / 25.369 |
| UTF-8 collation, 512 matches × 1 | 2.237 / 2.413 | 1.681 / 1.880 | 3.044 / 3.475 | 0.366 / 0.441 | 0.374 / 0.460 | 0.587 / 0.651 |
| UTF-8 collation, 512 matches × 30 | 46.986 / 48.267 | 8.949 / 9.419 | 55.039 / 57.364 | 6.250 / 7.349 | 4.335 / 4.701 | 12.181 / 13.614 |
| UTF-8 collation, 4,096 matches × 1 | 15.104 / 15.733 | 9.806 / 10.958 | 17.664 / 18.478 | 1.789 / 2.195 | 1.703 / 1.899 | 3.846 / 4.211 |

On Mac, the first UTF-8 expansion took 55% of Bash's median at both 512 and 4,096 names. On Debian, dense `*.txt` took 47% of Bash's median, while the one-match prefix still took 74%. In 150 separate Mac startup samples, startup through exit for `-c ':'` took 2.549 / 3.103 ms (previous 2.634 / 3.077). PTY launch through first prompt took 4.350 / 5.000 ms (previous 4.381 / 4.968). [Startup harness](../tools/bench.py)

Mac の UTF-8 名の初回展開は512件・4,096件の両方で Bash の中央値の約55%。Debian の `*.txt` 全件一致は約47%、固定の接頭辞で1件に絞る場合は約74%だった。Mac の起動を別に150回計測し、最初のプロンプトまでの中央値は旧版 4.381 ms、新版 4.350 ms だった。

Flat ASCII fixtures contain 512 files. The two-component case contains 512 ordinary files alongside eight directories, each with a `lib.rs`. The recursive case enables `globstar` and visits a tree with 16 parents and 80 leaf directories, each containing one Rust source and 15 other files. These rows use `LC_ALL=C`. Multilingual names mix case, accents, Japanese text and emoji in `en_US.UTF-8` on Mac or `C.UTF-8` on Debian. The 4,096-name workload exceeds Tonari's retained-order limit. The separate compatibility test checks the complete sorted output after renaming, adding or removing a name. A 4,096-name check compared all 55,478 bytes of NUL-delimited output against GNU Bash on both systems. [Tests](../tests/shell.py)

通常の条件は `LC_ALL=C`。2階層の条件は通常ファイル512件とディレクトリ8件を混在させ、再帰条件は親16個・末端80個の各末端に Rust ファイル1件と他のファイル15件を置いた。多言語名は大小のラテン文字・アクセント付き文字・日本語・絵文字を混在させ、Mac は `en_US.UTF-8`、Debian は `C.UTF-8`。4,096件はソート順を保持する上限を超える。別の4,096件の検証では NUL 区切りの出力55,478バイトが両 OS で GNU Bash と一致した。

A separate Linux `strace -qq -c -e trace=openat,getdents64` run expanded one pattern in a directory of 512 files and one subdirectory holding a Rust source. For `item*.txt` the number of directory-read syscalls fell from 10 to 2 including EOF. For `*/*.rs` the previous build attempted 514 opens, with 512 errors. The new build opened the two directories successfully and read them in four calls including EOF. The Mac binary measured 869,632 bytes, previously 852,992. The Linux static executable measured 1,020,616 bytes, previously 1,009,640. [Design and comparison with Apple's libc](engineering-notes.md)

Linux のシステムコールを別に数えると、512ファイルと1ディレクトリでの `item*.txt` はディレクトリ読み取りが EOF 込みで10回から2回になった。`*/*.rs` は旧版で open が514回、うち512回は通常ファイルへの失敗だった。新版はディレクトリ2件を開き、各2回の読み取りで完了した。単一バイナリの増分は Mac が16,640バイト、Linux が10,976バイト。

```sh
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /path/to/bash --timing started --case glob --case glob-all --case glob-sparse --case glob-multi-star --case glob-subdirs --case glob-recursive --runs 15
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /path/to/bash --timing started --case glob-utf8-once --case glob-utf8-sort --case glob-utf8-large --runs 45
```


<a id="external-descriptor-copies"></a>

## External commands with standard-stream redirection / 標準入出力をつなぎ替える外部コマンド

On 2026-11-22 an M1 ran macOS 27.2.1 natively and Debian 13 arm64 in a VM. We rotated the new release, Tonari `d471f0f` and GNU Bash 5.2.37. Each script ran a foreground command or pipeline 16 times after shell startup with job control off. The driver checked all 38 output bytes, stderr and exit status, with five warm-ups then 65 samples on Mac and 85 on Debian. Times are **median / p95 in ms per 16 iterations**. Both external commands use absolute paths. [Scripts and harness](../tools/bench_scripts.py)

M1 上の macOS と Debian 13 arm64 VM で、新旧 Tonari と Bash 5.2.37 を交互に実行した。起動済みの非対話シェルで各コマンドを16回実行し、毎回スクリプト全体の出力38バイト・標準エラー・終了値を照合した。最初の5回を除き、Mac は65回、Debian は85回の中央値 / p95 を示す。外部の `printf` と `cat` はどちらも絶対パスで起動した。

| Workload / 処理 | Mac previous | Mac Tonari | Mac Bash | Debian previous | Debian Tonari | Debian Bash |
|---|---:|---:|---:|---:|---:|---:|
| External `printf … 2>&1` | 34.714 / 42.517 | 22.760 / 29.488 | 35.731 / 44.748 | 4.451 / 4.919 | 3.917 / 4.544 | 5.141 / 5.771 |
| External `printf … \|& cat 2>&1` | 35.461 / 45.837 | 25.781 / 34.749 | 39.645 / 53.063 | 5.965 / 6.673 | 5.356 / 6.019 | 6.652 / 7.729 |
| External `printf … \|& cat` | 27.142 / 33.982 | 26.974 / 33.238 | 43.303 / 52.806 | 5.446 / 6.217 | 5.512 / 6.220 | 6.729 / 7.549 |
| Builtin `printf … \|& /bin/cat` | 31.430 / 43.813 | 30.712 / 49.884 | 45.136 / 80.124 | — | — | — |
| Builtin `printf …`, direct output | 0.555 / 0.868 | 0.544 / 0.710 | 0.765 / 1.072 | — | — | — |

The Mac median fell 34% for standalone stderr duplication and 27% with the same duplication on the receiving command. The plain merged pipe already used a spawn path. In a separate 24-sample trial, passing its descriptor copies directly to libc gave 26.700 ms versus 27.231 ms using Rust's existing spawn path. For the built-in producer, Tonari still forks to run it as a pipeline stage. Writing the same 16 lines directly from the builtin took 0.544 ms in the final build. [Direct launches](#pipe-process-timing) · [Loader traces](#macos-child-loader)

標準エラーを標準出力にまとめる単体コマンドで旧版比34%、パイプの受信側でもまとめる場合は27%短縮した。通常の `|&` はすでに spawn で起動していた。別の24回の試作比較でも、パイプの複製操作を libc に直接渡すと 26.700 ms、既存経路が 27.231 ms だった。組み込みの `printf` もパイプの一段として実行する場合は fork する。同じ16行を端末や呼び出し側に直接出す用途ではパイプを省ける。

We also rotated 200 Mac launches per shell. From launch through exit, `-c ':'` took 2.735 / 3.211 ms (previous 2.756 / 3.225, Bash 5.866 / 6.669). PTY launch to first prompt took 4.515 / 5.181 ms (previous 4.596 / 5.160, Bash 7.817 / 8.427). The Mac binary grew from 836,416 to 852,992 bytes. Its machine-code section grew by 1,116 bytes, and the Mach-O `__TEXT` segment crossed a 16 KiB boundary. The Linux static PIE grew from 1,008,248 to 1,009,640 bytes.

Mac の起動も各200回測った。新版の `-c ':'` は中央値 2.735 ms、疑似端末の起動からプロンプトまでは 4.515 ms だった。Mac の機械語部分は1,116バイト増え、セグメントの16 KiB境界をまたいだためファイル全体は16,576バイト増えた。Linux の静的リンク版は1,392バイト増えた。

```sh
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /path/to/bash --timing started --case redirect-stderr --case merged-pipeline-redirect --case merged-pipeline --case builtin-merged-pipeline --case builtin-output --runs 65
python3 tools/bench.py /path/to/tonari --compare-shell Previous /path/to/previous --bash /path/to/bash --case startup --case prompt --runs 200
```

<a id="borrowed-glob-names"></a>

## Hyphenated path prefixes and directory scans / ハイフン付きファイル名とディレクトリ走査

On 2026-11-22 we compared revision `3c03267`, the updated release and GNU Bash 5.2.37 on the M1 with macOS 27.2.1 and its Debian 13 arm64 VM. Each shell signaled readiness before sourcing the script. The driver rotated shell order, checked output, stderr and exit status, and ran five warm-ups followed by 35 samples on Mac or 85 on Debian. Each glob script expands its pattern 30 times in a directory with 512 files on the local filesystem. The character-class `case` control runs 3,000 iterations without reading that directory. Values are **median / p95 ms**. The single executables grew by 48 bytes on Mac (836,416 bytes) and 2,160 bytes on Linux (1,008,248 bytes). [Script definitions](../tools/bench_scripts.py)

同じ M1 の macOS と Debian 13 arm64 VM で旧版、新版、Bash 5.2.37 の順番を入れ替えながら測定した。起動済みのシェルにスクリプトを読ませ、出力・標準エラー・終了値を毎回確認した。各5回のウォームアップ後、Mac は35回、Debian は85回を集計。ファイル名展開は512ファイルがあるディレクトリで30回繰り返し、対照の文字クラス付き `case` はディレクトリを読まずに3,000回繰り返す。単位は中央値 / p95（ms）。

| Workload / 処理 | Mac previous | Mac Tonari | Mac Bash | Debian previous | Debian Tonari | Debian Bash |
|---|---:|---:|---:|---:|---:|---:|
| `item-000*.txt`, one match | 10.289 / 13.354 | 6.927 / 8.412 | 1119.627 / 1175.226 | 3.447 / 3.783 | 2.237 / 2.439 | 2.733 / 3.029 |
| `*.txt`, 512 matches | 10.665 / 13.329 | 8.373 / 9.758 | 1128.396 / 1186.428 | 6.310 / 6.829 | 5.721 / 6.300 | 8.865 / 9.568 |
| `case` with `[0-9]` | 5.356 / 6.313 | 5.351 / 7.506 | 7.223 / 9.050 | 4.949 / 5.740 | 4.939 / 5.495 | 8.891 / 9.498 |

The Debian one-match workload fell from 126.1% to 81.8% of Bash's median. All 512 matches took 64.5% of Bash's median and 65.8% of its p95. In a separate Debian ablation, changing only the internal quoting of the initial literal reduced the one-match median from 3.468 to 2.904 ms (65 samples, Bash 2.802). The final build also borrows the names during matching.

Debian で1件だけ一致する条件は Bash の中央値比 126.1% から 81.8% に短縮した。全512件が一致する条件は中央値で 64.5%、p95 で 65.8% だった。別に先頭の固定文字のエスケープだけ直した中間版を65回測ると、1件一致は旧版 3.468 ms、中間版 2.904 ms、Bash 2.802 ms だった。最終版ではディレクトリ内の名前のコピーも省いた。

```sh
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/old-tonari --bash /path/to/bash --timing started --case glob-sparse --case glob --case case-classes --runs 85
```

<a id="completion-and-case"></a>

## Registered completion, case and globs / 登録した補完・case・ファイル名展開

On 2026-11-22 we compared `0a3a293`, the updated native release, and GNU Bash 5.2.37 on an M1 Mac (macOS 27.2.1) and Debian 13 arm64 in a Linux VM on the same machine. Each OS used local binaries and fixtures, warm caches, and rotating shell order. All times below are **median / p95 in milliseconds**. The Mac binary grew from 836,336 to 836,368 bytes. The Linux static PIE grew from 987,496 to 1,006,088 bytes. The Bash binaries measured 1,208,904 and 1,412,096 bytes respectively.

同じ M1 の macOS と Debian 13 arm64 VM で、旧 Tonari・新 Tonari・Bash 5.2.37 を交互に測定した。実行ファイルと入力ファイルは各 OS のローカルディスクに置き、キャッシュを温めた。表は中央値 / p95（ms）。旧版とのサイズ差は Mac が +32 バイト、静的リンクの Linux 版が +18,592 バイト。

| Workload / 処理 | Mac previous | Mac Tonari | Mac Bash | Debian previous | Debian Tonari | Debian Bash |
|---|---:|---:|---:|---:|---:|---:|
| Tab, 10,000 fixed words, all match | 2.370 / 3.378 | 0.321 / 0.592 | 26.589 / 30.556 | 2.692 / 4.407 | 0.373 / 0.421 | 270.014 / 280.553 |
| Tab, 10,000 fixed words, one match | 2.229 / 2.551 | 0.231 / 0.271 | 26.214 / 28.864 | 2.279 / 3.540 | 0.319 / 0.403 | 268.520 / 279.456 |
| Tab, 10,000 words in a variable, one match | 0.964 / 1.259 | 1.005 / 1.490 | 2.563 / 2.859 | 1.235 / 1.726 | 1.237 / 1.588 | 3.779 / 4.911 |
| 3,000 `case` decisions, literal prefix and suffix | 8.599 / 10.305 | 4.673 / 6.365 | 7.821 / 9.563 | 6.853 / 8.746 | 4.243 / 5.434 | 10.350 / 13.731 |
| 3,000 `case` decisions, `[0-9]` and wildcard | 7.029 / 9.938 | 5.167 / 7.492 | 7.240 / 10.112 | 6.985 / 9.759 | 5.302 / 7.684 | 9.548 / 11.058 |
| 30 × `set -- fixtures/*.txt`, 512 files | 14.455 / 18.523 | 11.115 / 12.950 | 1206.226 / 1517.805 | 8.577 / 11.883 | 6.882 / 8.913 | 9.590 / 11.384 |
| Shell launch through exit, `-c ':'` | 2.854 / 4.030 | 2.832 / 5.511 | 6.058 / 10.257 | 0.208 / 0.422 | 0.207 / 0.334 | 0.469 / 1.039 |
| PTY launch through first prompt | 4.581 / 5.362 | 4.505 / 5.189 | 7.714 / 8.688 | 0.608 / 0.990 | 0.585 / 1.771 | 1.034 / 2.245 |

The three Tab rows use a fresh interactive shell and a list named `option-00000-flag` through `option-09999-flag`. After the rc file registers the list, the driver sends the prefix and waits for its echo. It then times Tab plus a following `@` until that character comes back through the PTY. A separate redraw verifies the entire completed line. Each case has three warm-ups and 40 timed samples per shell. The variable control registers the literal expression `$choices`, expanded on Tab. Its median was within 0.05 ms of the previous build on both systems. [Completion driver](../tools/bench_completion.py)

補完は各回、新しい対話シェルの設定ファイルで1万件を登録してから入力した。接頭辞が表示されたところで Tab と続く `@` を送り、PTY に `@` が戻るまでを測った。計時後に行全体を再描画して、補完結果と後続の入力を照合した。ウォームアップ3回、計測40回。変数を使う対照では `$choices` という式を登録して Tab ごとに展開する。この対照の中央値は両 OS とも旧版との差が 0.05 ms 未満だった。

The script rows use `LC_ALL=C`, five warm-ups and 35 timed samples. The driver sources each script after the shell signals readiness and checks stdout, stderr and exit status. The literal-edge `case` workload tries two nonmatching patterns before the matching `src/*.rs`. Its Tonari/Bash median ratio was 0.597 on Mac and 0.410 on Debian. The Mac character-class case (0.714) and Debian pathname expansion (0.718) exceeded the two-thirds target. Startup rows use five warm-ups, 200 runs and `--noprofile --norc` for every participant. The Debian run also timed the same revision linked with GNU ld; its startup and first-prompt times were 0.207 / 0.320 and 0.578 / 1.088 ms respectively. [Script driver](../tools/bench_scripts.py) · [Startup driver](../tools/bench.py)

スクリプトは `LC_ALL=C`、ウォームアップ5回、計測35回。起動済みのシェルにスクリプトを読ませ、毎回出力・診断・終了値を検証した。固定の前後を持つ `case` は、先に2個の不一致を判定してから `src/*.rs` に一致する。Bash に対する中央値の比は Mac で 0.597、Debian で 0.410。Mac の文字クラス付き `case`（0.714）と Debian のファイル名展開（0.718）には 2/3 の目標まで差がある。起動の2項目は全シェルに `--noprofile --norc` を渡し、ウォームアップ5回、計測200回。Debian では同じソースを GNU ld でもリンクし、起動が 0.207 / 0.320 ms、最初のプロンプトまでが 0.578 / 1.088 ms だった。

In a second batch of 300 samples the Mac `-c ':'` p95 was 3.161 ms (previous 3.206), and the Debian first-prompt p95 was 0.815 ms (previous 0.880, same-source GNU ld 0.809). Neither higher p95 from the first batch repeated. The Debian first-prompt medians were 0.588, 0.585 and 0.596 ms respectively.

起動をさらに300回測ると、Mac の `-c ':'` の p95 は 3.161 ms（旧版 3.206）、Debian の最初のプロンプトの p95 は 0.815 ms（旧版 0.880、同じソースの GNU ld 版 0.809）だった。最初の計測で見られた両項目の p95 の増加は再現しなかった。Debian のプロンプトまでの中央値は順に 0.588、0.585、0.596 ms。

```sh
python3 tools/bench_completion.py /path/to/tonari /path/to/previous --bash /path/to/bash --word-list --runs 40
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /path/to/bash --timing started --case case-edges --case case-classes --case glob --runs 35
python3 tools/bench.py /path/to/tonari --compare-shell Previous /path/to/previous --bash /path/to/bash --case startup --case prompt --runs 200
```

<a id="pipe-process-timing"></a>

### Where merged-pipe time is spent / 標準エラーをつなぐパイプの時間内訳

Tonari `98e1614`, GNU Bash 5.2.37, macOS 27.2.1 and Debian 13 arm64 on the same M1, 2026-11-22. The workload prints the numbers 0–15 with 16 separate `/usr/bin/printf` and `/bin/cat` pairs. All 38 output bytes and the exit statuses were checked. Shell timings begin after startup. An already running Python parent also made the pipe, wired stdout and stderr with `POSIX_SPAWN_DUP2`, called `os.posix_spawn` twice, and waited for both children before the next pair. The parent captured the same output. Five warm-ups and 35 measured samples per condition in rotating order. Values are **median / p95 ms**. Each statistic is computed independently.

同じ M1 の2環境で外部の `printf` と `cat` を16組起動し、38バイトの出力と終了値を確認した。シェルの起動は計時外。対照では起動済みの Python 親プロセスから同じパイプを作り、2プロセスを直接起動して両方の終了を待った。各条件を交互に実行し、最初の5回を除く35回の中央値 / p95（ms）を示す。区間ごとに計算しているため、中央値の和は全体の中央値と一致する必要がない。

| 16 pairs / 16組 | macOS | Debian |
|---|---:|---:|
| Tonari, ordinary pipe | 27.498 / 42.688 | 5.302 / 6.289 |
| Tonari, merged pipe | 28.827 / 37.007 | 5.457 / 6.548 |
| Tonari, explicit stderr duplication then pipe | 29.147 / 39.010 | 5.483 / 6.596 |
| Bash, merged pipe | 46.001 / 81.871 | 6.465 / 8.963 |
| Direct merged pipe, total | 27.712 / 35.754 | 5.894 / 7.086 |
| Direct, inside the 32 spawn calls | 5.665 / 7.761 | 3.478 / 4.241 |
| Direct, close pipe ends and wait after launching each pair | 22.160 / 27.790 | 2.429 / 2.782 |

In a separate control, a small native C parent launched the same executable as its child. The first statement of `main` read `CLOCK_MONOTONIC`; the child wrote that timestamp to a pipe and returned. The parent recorded time immediately before and after `posix_spawn` and after `waitpid`. Each OS used its local C compiler with `-O2`, normal dynamic linking, 15 warm-ups, then 200 launches. Most of the cross-host difference in this control accumulated before entry to `main`. Each system runs different executable and loader formats. These measurements do not give separate costs for loading, kernel work or scheduling.

別の対照では、それぞれの OS で同じ小さな C ソースを `-O2` で動的リンクした。親が自分自身を起動し、子は `main` の先頭で時刻を取り、パイプにその値を書いて終了した。親は `posix_spawn` の直前・直後と `waitpid` 完了後に時刻を取った。すべて `CLOCK_MONOTONIC` を使い、最初の15回を除く200回を計測した。この対照の OS 間の差は大部分が子の `main` に入るまでに生じた。ローダー・カーネル・スケジューラごとの寄与は未計測。

| Single native child / 子プロセス1個 | macOS | Debian |
|---|---:|---:|
| Launch request to child entering `main` | 1.470 / 1.920 | 0.158 / 0.206 |
| Entry to parent observing exit | 0.255 / 0.330 | 0.038 / 0.064 |
| Entire launch and wait | 1.728 / 2.242 | 0.197 / 0.274 |
| Inside the parent's spawn call | 0.136 / 0.233 | 0.102 / 0.128 |

The same Debian run compared equivalent `case` patterns on the same input. Replace the case arms in `case-edges` with `src/u*.rs) last=$i ;; *) exit 1 ;;`, then change only `u` to `[u]`. A separate pathname control changed `fixtures/*.txt` to `fixtures/item-000*.txt` in the 512-file fixture and checked the resulting count and first name. In both pathname cases the shell still scans the directory 30 times. [Script and fixture definitions](../tools/bench_scripts.py)

Debian の同じ計測で、同じ入力・分岐数の `case` を用意し、固定の `u` を同じ文字だけに一致する `[u]` に替えた。ファイル名展開では512ファイルを置いたまま、全件に一致する `*.txt` と1件だけ一致する `item-000*.txt` を比較した。どちらもディレクトリを30回走査する。

| Debian workload / 処理 | Tonari | Bash |
|---|---:|---:|
| 3,000 × `case`, `src/u*.rs` | 4.435 / 5.793 | 8.733 / 9.983 |
| 3,000 × `case`, `src/[u]*.rs` | 4.614 / 5.323 | 8.751 / 10.006 |
| 30 globs, 1 of 512 names returned | 3.477 / 4.125 | 2.771 / 3.241 |
| 30 globs, all 512 names returned | 6.427 / 7.478 | 8.764 / 9.932 |

A follow-up used two directories of 512 files, changing the separator in every name and its pattern together. Each glob returned index 000. Each script made 30 expansions; the eight shell/script combinations rotated through five warm-ups and 45 measurements. For literal source spelling, Tonari took 3.853 / 5.434 ms for `item-000*.txt` and 3.134 / 4.430 for `item_000*.txt` (Bash 3.091 / 4.413 and 3.008 / 5.984). With the same patterns supplied via an unquoted variable, Tonari took 3.264 / 7.106 and 3.052 / 5.601 ms. Tonari escapes a hyphen in the accumulated literal prefix when it first encounters the star; the resulting backslash selects the generic matcher for each directory entry. The lower median with an underscore or a variable is consistent with that code path.

追加計測では512個すべてのファイル名と対応するパターンで、区切りを一緒にハイフンまたはアンダースコアに替えた。各スクリプトで30回展開し、どれも番号000の1件に一致する。各条件45回の計測で、直接書いた場合の Tonari の中央値はハイフンで 3.853 ms、アンダースコアで 3.134 ms、同じパターンを非引用の変数に入れた場合は 3.264 ms と 3.052 ms だった。ソースに直接書いたハイフンは最初の星印に達した段階でエスケープされ、各候補の照合が汎用経路に入る。

The later build avoids escaping the initial hyphen and copying every candidate. 更新後は先頭のハイフンをそのまま照合し、一致した候補だけをコピーする。[Updated glob measurements / 更新後の計測](#borrowed-glob-names)

<a id="macos-child-loader"></a>

### Loader activity inside the two external commands / 外部2コマンドのローダー内訳

On the same M1 (macOS 27.2.1 build 26B131), Xcode 27.2 Instruments recorded three separate launches each of `/bin/cat /dev/null` and `/usr/bin/printf x`. The App Launch template reports a dyld interval named *Launch Executable*, with nested *Static Initializer* and *Apply Fixups* events. The initializer row below selects the event for `libSystem.B.dylib`. Values are **median [min–max] milliseconds** from the three traces. These are instrumented standalone launches; the earlier pipeline stopwatch includes overlap between the two children. Both executables list libSystem as their only direct dylib dependency in `otool -L`.

同じ M1 上で Xcode 27.2 の Instruments を使い、実際にパイプにつなぐ2本のコマンドを個別に各3回記録した。表は中央値［最小–最大］、単位は ms。dyld の起動区間の内側に、システムライブラリの初期化やアドレスの修正が含まれる。プロファイラを付けた単独起動と、子プロセスが並行して動くパイプ全体では測定区間が異なる。

| Instruments event / 区間 | `/bin/cat` | `/usr/bin/printf` |
|---|---:|---:|
| dyld Launch Executable | 1.089 [1.080–1.183] | 0.949 [0.923–2.849] |
| Run static initializer, libSystem | 0.572 [0.563–0.668] | 0.492 [0.487–0.533] |
| Apply Fixups | 0.128 [0.113–0.134] | 0.122 [0.115–0.619] |

```sh
xcrun xctrace record --template 'App Launch' --no-prompt --output cat.trace --launch -- /bin/cat /dev/null
xcrun xctrace export --input cat.trace --xpath '/trace-toc/run[@number="1"]/data/table[@schema="dyld-activity-interval"]'
```

<a id="align-experiments"></a>

## Parent memory, pipes and delimiter search / 親シェルのメモリ・パイプ・区切り検索

On 2026-11-22, on an Apple M1 with macOS 27.2.1 and a Debian 13 arm64 VM, we compared Tonari `0e091ca`, the updated native release and GNU Bash 5.2.37. Each case used five warm-ups followed by 30 samples per shell in rotating order. The fixtures and binaries were local to each OS. The timer starts after the ready marker from a fresh, noninteractive shell and includes sourcing the script and collecting the child and its output. The driver checks stdout, stderr and exit status every time. Values are **median / p95 in milliseconds**. The Unicode IFS row uses `en_US.UTF-8` on macOS and `C.UTF-8` on Debian. Other rows use `LC_ALL=C`. [Script driver](../tools/bench_scripts.py)

同じ M1 上の macOS と Debian 13 arm64 VM で、旧 Tonari、新 Tonari、Bash 5.2.37 を順番に入れ替えながら各30回測定した。各条件の最初の5回を除き、起動済みのシェルにスクリプトを読ませて完了するまでを計時している。毎回、標準出力・標準エラー・終了値を照合した。表は中央値 / p95（ms）。絵文字の区切りだけ各 OS の UTF-8 ロケールで測った。

### Fresh shell / 通常の起動状態

| Workload / 処理 | Mac previous | Mac Tonari | Mac Bash | Debian previous | Debian Tonari | Debian Bash |
|---|---:|---:|---:|---:|---:|---:|
| 16 sequential `/usr/bin/true` | 16.540 / 16.975 | 16.449 / 17.043 | 25.659 / 26.740 | 3.172 / 3.977 | 3.179 / 3.502 | 4.380 / 4.732 |
| 16 × `$(/usr/bin/printf …)` | 18.206 / 19.130 | 18.258 / 19.060 | 28.772 / 32.272 | 4.256 / 4.630 | 3.557 / 3.918 | 5.314 / 5.796 |
| 16 × `/usr/bin/printf … |& /bin/cat` | 29.434 / 32.600 | 21.667 / 23.309 | 33.463 / 37.443 | 5.096 / 5.351 | 5.154 / 5.570 | 6.289 / 6.829 |
| 16 × `/usr/bin/printf … | /bin/cat` | 23.364 / 34.105 | 22.066 / 34.865 | 34.326 / 42.694 | 5.006 / 5.611 | 4.992 / 5.435 | 6.117 / 6.798 |
| 400 splits, 32 fields, `IFS=:` | 4.816 / 5.100 | 3.224 / 3.266 | 16.505 / 16.803 | 5.907 / 6.375 | 4.428 / 4.790 | 18.204 / 19.136 |
| 400 splits, 32 fields, `IFS=😀` | 7.524 / 7.880 | 3.192 / 3.381 | 18.707 / 18.946 | 8.457 / 8.802 | 4.283 / 4.508 | 21.912 / 22.381 |
| 2,000 short path trims (basename and extension) | 5.136 / 5.291 | 4.954 / 5.271 | 10.750 / 10.865 | 4.567 / 4.829 | 4.472 / 4.784 | 13.756 / 14.542 |
| 400 basename trims, 2,048-byte final component | 4.863 / 4.929 | 1.529 / 1.631 | 615.173 / 688.967 | 3.519 / 3.677 | 1.445 / 1.510 | 821.893 / 827.673 |
| 2,000 lines via `IFS= read -r` | 6.067 / 6.161 | 5.993 / 6.102 | 9.528 / 9.765 | 4.955 / 5.247 | 4.924 / 5.175 | 10.033 / 10.368 |

### Shell holding a 16 MiB variable / 16 MiB の変数を保持した状態

The driver fills a private, unexported shell variable before the ready marker. Its contents stay out of the child process environment. This deliberately increases parent memory to distinguish process creation and session copying from the work done by the launched program. The pipe workloads start 32 real external processes per sample. [Subshell implementation](../src/subshell.rs)

開始合図の前に未エクスポートのシェル変数を16 MiBのデータで埋める。変数を用意する時間は計測外で、起動する外部コマンドの環境にも含めない。親のメモリ量に依存するコストを見るための条件である。パイプの項目では1サンプルにつき外部プロセスを32個起動する。

| Workload / 処理 | Mac previous | Mac Tonari | Mac Bash | Debian previous | Debian Tonari | Debian Bash |
|---|---:|---:|---:|---:|---:|---:|
| 16 × `$(/usr/bin/printf …)` | 29.121 / 31.489 | 19.100 / 20.138 | 29.923 / 35.012 | 32.208 / 33.367 | 3.903 / 4.349 | 5.899 / 6.490 |
| 16 × `/usr/bin/printf … |& /bin/cat` | 30.251 / 31.605 | 22.500 / 24.389 | 34.495 / 37.449 | 5.647 / 5.903 | 5.404 / 6.223 | 6.855 / 7.652 |
| 64 × `$(printf …)` | 31.379 / 38.031 | 0.954 / 0.991 | 45.333 / 47.300 | 73.942 / 80.318 | 0.422 / 0.547 | 14.949 / 15.185 |
| 16 × `$(printf v; printf …)` | 60.884 / 64.197 | 9.622 / 11.125 | 12.303 / 12.623 | 28.288 / 29.697 | 2.693 / 2.765 | 4.162 / 4.370 |

The ordinary Debian launch and plain pipe controls changed little. For `|&` with a fresh Debian shell, the new median was 5.154 ms versus 5.096 ms previously and 6.289 ms for Bash. With the larger variable the median improved, but its p95 rose from 5.903 to 6.223 ms. On macOS the fresh-shell median for `|&` fell from 29.434 to 21.667 ms. The binaries measured 836,336 bytes on macOS and 987,496 bytes on Linux (static musl with packed relative relocations). [Design and syscall counts](engineering-notes.md)

Debian の通常の連続起動と `|` の差は小さい。変数を追加しない場合の `|&` は旧版 5.096 ms、新版 5.154 ms、Bash 6.289 ms で、2/3 目標に届いていない。大きな変数がある場合は中央値が改善したものの p95 は 5.903 ms から 6.223 ms になった。Mac の通常状態の `|&` は 29.434 ms から 21.667 ms に短縮した。単一実行ファイルは macOS が 836,336 バイト、Linux が 987,496 バイト。

```sh
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /path/to/bash --timing started --runs 30 --case external --case external-substitutions --case merged-pipeline --case pipeline --case field-split --case field-split-unicode --case strings --case long-trim --case read-lines
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /path/to/bash --timing started --runs 30 --state-mib 16 --case external-substitutions --case merged-pipeline --case substitutions --case compound-substitutions
```

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

The static build links musl into a position-independent executable. It runs on the Debian and Ubuntu arm64 test systems without a separate musl installation. The tested build uses Alpine's native Rust 1.96.1 package (`aarch64-alpine-linux-musl`), musl development files, a C compiler driver and LLD 22.1.3. Build on the target CPU when using `target-cpu=native`. LLD packs the segments with 64 KiB load alignment. [File sizes and startup timings](#completion-and-case)

静的リンク版は musl を実行ファイルに組み込んだ PIE（位置独立実行形式）です。検証した Debian と Ubuntu の arm64 環境では musl の追加インストールなしで動きます。下記は Alpine のネイティブ Rust パッケージ 1.96.1（`aarch64-alpine-linux-musl`）、musl の開発ファイル、C コンパイラドライバ、LLD 22.1.3 を使ったビルド手順です。`target-cpu=native` はビルドした CPU 向けの設定です。LLD はロード時に 64 KiB 境界で整列できるよう、ファイル内に各セグメントを配置します。

```sh
CARGO_PROFILE_RELEASE_OPT_LEVEL=s \
RUSTFLAGS='-C target-cpu=native -C target-feature=+crt-static' \
cargo rustc --locked --release --target aarch64-alpine-linux-musl -- \
  -C link-arg=-fuse-ld=lld -C link-arg=-Wl,-z,pack-relative-relocs,-z,max-page-size=65536
objcopy --remove-section=.comment target/aarch64-alpine-linux-musl/release/tonari tonari-linux-arm64
```

For the case-insensitive matching release we used GNU objcopy from Debian 13 for arm64. Removing the compiler comment saved 200 bytes. The executable code segment and writable load segments had the same sizes and byte contents. コンパイラの識別文字列を外した配布ファイルで動作と速度を再検証しました。

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

<a id="feature-audit"></a>

## 2026-11-22 — Timings across the Bash feature inventory / Bash 機能一覧の速度

The shell code was `39b0e6e`. We used native ARM64 release binaries on an Apple M1 with macOS 27.2.1 and a Docker Desktop Linux 6.11.11 VM. Debian 13 used GNU Bash 5.2.37, Ubuntu 26.04 used 5.3.9 and macOS used 5.2.37. The Linux Tonari was a 1,053,592-byte static musl executable. The macOS executable was 886,400 bytes. Bash was 1,412,096, 1,609,176 and 1,208,904 bytes respectively. This is a warm-cache run. The two shell executables were staged on the same filesystem, and shell order alternated between samples. [Environment and binary hashes, Bash then Tonari](compatibility/environment.json)

Each row below counts names with a verified behavioral match, including shared witnesses and GNU golden matches. The denominators for the two timings differ because only canonical command strings support the readiness handshake. Interface-only checks are available in the detailed matrix. The comparison uses the median per executable, with two warmups and 15 measured samples per executable (seven for a GNU driver). The TSVs retain absolute medians and p95 values for every measured feature. [Full matrix](compatibility/matrix.md) · [Method and release gates](compatibility.md)

各セルは「達成した名前の数 / 計測できた名前の数」です。左側は起動込み、右側は起動済みシェルに処理を渡してから終了までです。名前には同じテストを参照する予約語等が含まれます。シェル起動後に再度同期できる形式だけが右側の母数に入ります。中央値で 1/2 が目標、2/3 が上限です。

| Platform | Whole ≤ 1/2 | Whole ≤ 2/3 | Whole ≤ Bash | Started ≤ 1/2 | Started ≤ 2/3 | Started ≤ Bash |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Debian 13 | 7/481 | 324/481 | 471/481 | 6/429 | 23/429 | 360/429 |
| Ubuntu 26.04 | 6/481 | 312/481 | 473/481 | 6/429 | 14/429 | 358/429 |
| macOS | 4/473 | 434/473 | 471/473 | 9/432 | 62/432 | 424/432 |

The checked command substitution `v=$(printf 'a\n\n')` took 0.34× Bash's started time on both Linux systems. The three-stage `printf | tr | wc` pipeline took 0.85× on Debian, 0.88× on Ubuntu and 0.72× on macOS. Waiting for a background subshell’s exit status took 1.05×, 1.07× and 1.17× respectively. These are elapsed times for small, complete programs, so the scheduler, external commands and shell exit contribute to each result. A script that repeats work inside one shell answers a different performance question.

起動済みの Bash と比べると、末尾改行を除去するコマンド置換は両 Linux で 0.34 倍でした。3段のパイプは Debian で 0.85 倍、Ubuntu で 0.88 倍、macOS で 0.72 倍です。バックグラウンドのサブシェルの終了値を `wait` で取得する例はそれぞれ 1.05、1.07、1.17 倍でした。この単発計測には、OS による起床や終了処理も含まれます。

The companion workload run covers repeated scripts (15 samples per shell), startup and command execution (65), and completion, candidate listing and keyboard latency (12 sessions; individual keystroke counts are recorded). The shared editor cases verify the completed text or the set of displayed candidates. Tonari-only history suggestions have absolute latency measurements. [All script, startup and editor measurements](compatibility/workloads.tsv)

反復スクリプト、起動と外部実行、入力・補完・候補一覧の数値も別表に収録しました。キーボード操作では入力が画面に返るまで測り、補完結果の文字列または候補集合を照合しました。履歴からの入力候補については Tonari 自身の遅延を記録しています。

All 30 repeated scripts finished faster than Bash after startup on each platform. At the 2/3 target, Debian and Ubuntu passed 21 of 30 and macOS passed 25 of 30. For an interactive example, listing 10,000 candidates took a median of 4.24 ms in Tonari and 21.52 ms in Bash on Ubuntu. Ordinary filename Tab completion in the 10,000-file fixture took 2.32 ms versus 3.06 ms. The first operation clears the upper target and the second exceeds it.

起動済みシェルで繰り返し処理を行うスクリプト30本は、3環境ともすべて Bash より速く完了しました。上限の 2/3 まで達したのは Debian と Ubuntu で各21本、macOS で25本です。Ubuntu での候補1万件の一覧表示は Tonari が中央値 4.24 ms、Bash が 21.52 ms でした。同じ規模のディレクトリで通常のファイル名補完を行うと 2.32 ms と 3.06 ms で、こちらは上限に届いていません。

<a id="2026-11-23-runtime"></a>

## 2026-11-23 · Corrected operations / 修正した経路

Each cell reports median / p95 in milliseconds. Each script runs inside an already-started shell. Five warmups preceded 25 measured runs per executable, with rotating execution order and verified stdout and status. The previous release is the executable used for the November 22 inventory. All three binaries in each comparison were copied to the same local filesystem. macOS arm64 uses Bash 5.2.37. The arm64 Linux guests use Debian 13 Bash 5.2.37 and Ubuntu 26.04 Bash 5.3.9. Tonari uses the native release configuration on macOS and the static size-optimized musl release on Linux.

修正した経路を、起動済みシェルで同じスクリプトを実行して比較しました。表の値は中央値 / p95（ms）、比は Tonari の中央値を Bash の中央値で割った値です。以前の実装では結果が一致しない新機能の欄は空欄です。パイプには外部の `/usr/bin/printf` と `/bin/cat` の起動・実行・終了待ちが含まれます。

| Platform | Workload | Bash | Previous Tonari | Current Tonari | Current / Bash |
|---|---|---:|---:|---:|---:|
| macOS | `echo-substitutions` | 43.824 / 45.487 | 35.080 / 35.719 | 0.507 / 0.525 | 0.012 |
| macOS | `compound-substitutions` | 11.418 / 11.691 | 9.150 / 9.989 | 0.452 / 0.501 | 0.040 |
| macOS | `pipeline` | 33.457 / 36.377 | 21.711 / 22.222 | 21.605 / 22.175 | 0.646 |
| macOS | `merged-pipeline` | 33.558 / 34.964 | 21.693 / 23.210 | 21.731 / 22.781 | 0.648 |
| macOS | `external-substitutions` | 28.928 / 29.421 | 18.358 / 18.622 | 18.335 / 19.232 | 0.634 |
| macOS | `extended-patterns` | 1.659 / 1.719 | — | 1.021 / 1.065 | 0.615 |
| macOS | `resource-limits` | 4.636 / 4.664 | — | 3.730 / 3.940 | 0.805 |
| macOS | `brace-range` | 4.743 / 5.055 | — | 1.481 / 1.607 | 0.312 |
| macOS | `getopts-clusters` | 1.762 / 1.803 | — | 1.016 / 1.048 | 0.576 |
| macOS | `mapfile-lines` | 3.154 / 3.252 | — | 1.877 / 1.911 | 0.595 |
| macOS | `case-fallthrough` | 2.992 / 3.033 | — | 1.864 / 1.903 | 0.623 |
| macOS | `directory-stack` | 6.154 / 6.302 | — | 5.219 / 5.348 | 0.848 |
| macOS | `descriptor-moves` | 3.628 / 3.677 | — | 3.139 / 3.187 | 0.865 |
| macOS | `integer-assignments` | 3.274 / 3.308 | — | 2.191 / 2.221 | 0.669 |
| Debian | `echo-substitutions` | 14.606 / 24.716 | 8.652 / 9.760 | 0.253 / 0.330 | 0.017 |
| Debian | `compound-substitutions` | 3.865 / 4.856 | 2.540 / 3.350 | 0.230 / 0.365 | 0.060 |
| Debian | `pipeline` | 6.815 / 8.640 | 5.666 / 8.273 | 5.892 / 7.541 | 0.865 |
| Debian | `merged-pipeline` | 6.362 / 7.511 | 5.300 / 6.132 | 5.142 / 6.074 | 0.808 |
| Debian | `external-substitutions` | 5.462 / 6.322 | 3.655 / 4.341 | 3.606 / 4.799 | 0.660 |
| Debian | `extended-patterns` | 1.705 / 1.782 | — | 0.732 / 0.798 | 0.429 |
| Debian | `resource-limits` | 2.090 / 2.185 | — | 1.097 / 1.204 | 0.525 |
| Debian | `brace-range` | 6.336 / 7.006 | — | 1.457 / 1.581 | 0.230 |
| Debian | `getopts-clusters` | 1.757 / 1.781 | — | 0.762 / 0.784 | 0.433 |
| Debian | `mapfile-lines` | 4.261 / 4.472 | — | 1.858 / 1.948 | 0.436 |
| Debian | `case-fallthrough` | 3.564 / 3.947 | — | 1.572 / 1.727 | 0.441 |
| Debian | `directory-stack` | 2.210 / 2.260 | — | 1.216 / 1.248 | 0.550 |
| Debian | `descriptor-moves` | 1.140 / 1.186 | — | 0.987 / 1.002 | 0.865 |
| Debian | `integer-assignments` | 4.259 / 4.758 | — | 1.819 / 2.074 | 0.427 |
| Ubuntu | `echo-substitutions` | 12.487 / 13.228 | 7.732 / 8.187 | 0.220 / 0.260 | 0.018 |
| Ubuntu | `compound-substitutions` | 3.591 / 3.702 | 2.381 / 2.489 | 0.203 / 0.236 | 0.057 |
| Ubuntu | `pipeline` | 18.821 / 19.333 | 16.795 / 17.235 | 16.869 / 19.291 | 0.896 |
| Ubuntu | `merged-pipeline` | 18.879 / 19.387 | 16.895 / 17.295 | 16.833 / 17.307 | 0.892 |
| Ubuntu | `external-substitutions` | 16.051 / 16.495 | 14.347 / 14.595 | 14.265 / 14.604 | 0.889 |
| Ubuntu | `extended-patterns` | 1.442 / 1.466 | — | 0.701 / 0.734 | 0.486 |
| Ubuntu | `resource-limits` | 1.956 / 2.119 | — | 1.078 / 1.236 | 0.551 |
| Ubuntu | `brace-range` | 6.607 / 6.775 | — | 1.399 / 1.424 | 0.212 |
| Ubuntu | `getopts-clusters` | 1.506 / 1.548 | — | 0.758 / 0.795 | 0.503 |
| Ubuntu | `mapfile-lines` | 4.853 / 4.909 | — | 1.835 / 1.891 | 0.378 |
| Ubuntu | `case-fallthrough` | 2.963 / 3.000 | — | 1.559 / 1.608 | 0.526 |
| Ubuntu | `directory-stack` | 2.095 / 2.162 | — | 1.210 / 1.239 | 0.577 |
| Ubuntu | `descriptor-moves` | 1.073 / 1.121 | — | 0.988 / 1.001 | 0.921 |
| Ubuntu | `integer-assignments` | 3.730 / 3.832 | — | 1.820 / 1.944 | 0.488 |

The two builtin substitutions run 16 compound or 64 single-echo captures. The pipeline and external substitution scripts run 16 times each. The range workload expands 1,000 zero-padded numbers 16 times. The option workload parses a cluster and a value across 128 loops. The array workload reads 2,000 lines into an indexed array 16 times. The extended-pattern workload tests 500 case arms containing a repeated character class and a choice of suffixes. The resource-limit workload performs 256 soft-limit updates and queries. The full inventory’s earlier ratios remain attached to its original binary. [Workload definitions](../tools/bench_scripts.py) · [Compatibility changes](compatibility/changes.tsv)
### Startup and executable sizes / 起動時間とサイズ

The tested shell is commit `7144562`, source tree `d3eb7e7529fb8cc293923cc281408bacb2208a2f`. The macOS executable SHA-256 is `1079993cd86465306eacf1b69dcb62f45ec99e15fa4558ffb41a49624606bf35` and the static Linux executable SHA-256 is `03882e6c7cfceff66607382451a02d540b8c6abab3e2ceb99000a622c4e4e52e`. The host was an Apple M1 running macOS 27.2.1. The Linux guests ran native arm64 code. The empty-command and PTY benchmarks rotated each binary through five warmups and 51 measured launches. The Linux PTY fixture provided an empty system bashrc so the prompt marker was stable. The Bash column below is the executable size. Dynamic system libraries are outside that figure.

起動は各実行ファイルを51回測定しました。表は中央値 / p95（ms）です。最初のプロンプトには PTY 子プロセスの起動を含みます。同じ試行から exec の直前から表示までの区間も計測しました。

| Platform | Interval | Bash | Previous Tonari | Current Tonari | Current / Bash |
|---|---|---:|---:|---:|---:|
| macOS | empty command | 4.781 / 5.220 | 2.327 / 2.613 | 2.234 / 2.544 | 0.467 |
| macOS | first prompt | 6.356 / 6.772 | 3.791 / 4.057 | 3.571 / 3.969 | 0.562 |
| macOS | exec to first prompt | 5.268 / 5.615 | 2.571 / 2.727 | 2.470 / 2.692 | 0.469 |
| Debian | empty command | 0.420 / 0.635 | 0.195 / 0.277 | 0.196 / 0.279 | 0.468 |
| Debian | first prompt | 0.903 / 1.181 | 0.548 / 0.685 | 0.524 / 0.760 | 0.580 |
| Debian | exec to first prompt | 0.647 / 0.833 | 0.277 / 0.335 | 0.264 / 0.418 | 0.408 |
| Ubuntu | empty command | 0.414 / 0.488 | 0.190 / 0.232 | 0.194 / 0.226 | 0.468 |
| Ubuntu | first prompt | 0.924 / 1.298 | 0.567 / 0.839 | 0.553 / 1.151 | 0.598 |
| Ubuntu | exec to first prompt | 0.652 / 0.835 | 0.284 / 0.401 | 0.280 / 0.673 | 0.429 |

| Platform | Bash bytes | Previous Tonari bytes | Current Tonari bytes |
|---|---:|---:|---:|
| macOS | 1,208,904 | 886,400 | 969,360 |
| Debian | 1,412,096 | 1,053,592 | 1,146,920 |
| Ubuntu | 1,609,176 | 1,053,592 | 1,146,920 |

Across the 42 script and platform pairs, 17 reached half of Bash's elapsed time and 31 met the two-thirds upper target. These counts apply to these 14 checked workloads on three platforms. The following medians exceeded the upper target.

- macOS — `resource-limits` (0.805), `directory-stack` (0.848), `descriptor-moves` (0.865), `integer-assignments` (0.669).
- Debian — `pipeline` (0.865), `merged-pipeline` (0.808), `descriptor-moves` (0.865).
- Ubuntu — `pipeline` (0.896), `merged-pipeline` (0.892), `external-substitutions` (0.889), `descriptor-moves` (0.921).

The intermediate macOS directory-stack run took 10.902 ms (1.687× Bash) before caching the logical directory. The first implementation of the extended-pattern workload took 1.669 ms (1.008× Bash). An additional 41-run comparison of the compiled matcher measured 1.157 ms against 1.853 ms for Bash. The tables use a fresh 25-run series on each platform. Each external-pipeline workload includes the child programs, their startup and the final wait.

macOS のディレクトリスタックは論理パスをキャッシュする前は Bash の1.687倍、拡張パターンの反復は構文と照合結果を再利用する前は1.008倍かかっていました。上の表には修正後の同じ実行ファイルで計測した結果を掲載しています。全1,552項目の速度表は11月22日のバイナリの記録です。

```sh
python3 tools/bench_scripts.py "$tonari" --bash "$bash" --timing started --runs 25 \
  --case echo-substitutions --case compound-substitutions --case pipeline --case merged-pipeline \
  --case external-substitutions --case extended-patterns --case resource-limits --case brace-range \
  --case getopts-clusters --case mapfile-lines --case case-fallthrough --case directory-stack \
  --case descriptor-moves --case integer-assignments
python3 tools/bench.py "$tonari" --bash "$bash" --runs 51 --case startup --case prompt
```

<a id="2026-11-23-compatibility"></a>

## 2026-11-23 · Associative arrays, read-only locals and traps / 連想配列・読み取り専用変数・トラップ

Source commit `2af2f21` was measured on Apple M1 arm64. The macOS 27.2.1 and Debian 13 controls used Bash 5.2.37. Ubuntu 26.04 used Bash 5.3.9. For the first four workloads the driver alternates shells, discards five warmups and measures 30 checked scripts after startup. For the remaining three intervals it measures 150 launches per shell. The two prompt intervals use the same PTY samples. Each cell is **Tonari median / Bash median in milliseconds (elapsed-time ratio)**. [Workloads and expected output](../tools/bench_scripts.py) · [Startup harness](../tools/bench.py)

Apple M1 で同じバイナリを使い、先頭の4項目は起動済みのシェルから30回ずつ、起動とプロンプトは150回ずつ測定しました。表は「Tonari の中央値 / Bash の中央値（比）」です。最初のプロンプトまでの全体と、そのうち exec の直前から表示までの区間を同じ試行で記録しています。

| Workload | macOS | Debian | Ubuntu |
|---|---:|---:|---:|
| Associative assignments and lookups | 2.422 / 3.918 (0.618) | 2.500 / 4.972 (0.503) | 2.456 / 4.388 (0.560) |
| Read-only function locals | 2.965 / 5.544 (0.535) | 2.752 / 6.811 (0.404) | 2.779 / 6.066 (0.458) |
| ASCII upper/lower expansions | 2.701 / 4.252 (0.635) | 2.112 / 6.054 (0.349) | 2.068 / 5.589 (0.370) |
| Handling 600 failures with `ERR` | 1.278 / 3.133 (0.408) | 0.944 / 3.220 (0.293) | 0.961 / 3.233 (0.297) |
| Empty command `-c :` | 2.927 / 5.970 (0.490) | 0.225 / 0.495 (0.455) | 0.233 / 0.494 (0.472) |
| First interactive prompt | 4.613 / 8.008 (0.576) | 0.577 / 0.987 (0.585) | 0.613 / 1.000 (0.613) |
| Exec to first interactive prompt | 3.196 / 6.516 (0.490) | 0.285 / 0.702 (0.406) | 0.312 / 0.694 (0.450) |

The four script medians range from 0.293 to 0.635 of Bash time. ASCII conversion on macOS measured 0.693 before the in-place change in a seven-sample trial and 0.635 in the release run. The elapsed interval includes parsing and running the loop. UTF-8 mapping and the C locale have separate correctness tests. The widest first-prompt tail in the Linux runs was Ubuntu at 1.246 ms for Tonari and 1.357 ms for Bash (p95). [Unicode and locale checks](../tests/shell.py)

反復した4処理の中央値は各環境とも Bash の3分の2以内でした。macOS の大小文字変換は予備計測で0.693倍でした。ASCII の文字列をその場で変換するようにした版では0.635倍です。Ubuntu での対話起動の p95 は Tonari が1.246 ms、Bash が1.357 ms でした。

| Executable bytes | Previous Tonari | Current Tonari | Reference Bash |
|---|---:|---:|---:|
| macOS | 969,360 | 1,019,504 | 1,208,904 |
| Debian | 1,146,920 | 1,202,144 | 1,412,096 |
| Ubuntu | 1,146,920 | 1,202,144 | 1,609,176 |

Adding this behavior grew the macOS executable by 50,144 bytes and the static Linux executable by 55,224 bytes. The macOS release SHA-256 is `2c1a1e4cadbda45058988cc014b9641d637712b4a7bc5f28eb23eadd633c3c55`. Both Linux guests ran `1135d98a18a792f9be7a0f6fd3042a4ea7f2e4bfd613b2be4c5f297cd6ea4897`. The same releases ran the direct checks and the full feature timing matrix. [Current results for individual features](compatibility-2026-11-23/matrix.md)

機能追加による増分は macOS で50,144バイト、Linux の静的バイナリで55,224バイトです。両方とも配布版 Bash の実行ファイルより小さく収まっています。[全機能の判定と計測値](compatibility-2026-11-23/matrix.md)

```sh
python3 tools/bench_scripts.py "$tonari" --bash "$bash" --timing started --runs 30 \
  --case associative-lookup --case readonly-locals --case case-conversion --case error-trap
python3 tools/bench.py "$tonari" --bash "$bash" --runs 150 --case startup --case prompt
```

### Timings across the current inventory / 現行版の機能別計測

The inventory contains 1,554 distinct names across all three platforms. Each denominator below counts matching names with checked timing samples. Several names can use one script. The two verified UTF-8 differences are listed with their observed bytes. A complete run uses 15 samples per shell for each direct case and seven for a matching GNU group. Each sample must reproduce the earlier observation. [Full per-feature results, median and p95](compatibility-2026-11-23/matrix.md) · [Release fingerprints](compatibility-2026-11-23/environment.json)

| Platform | Whole ≤ 1/2 | Whole ≤ 2/3 | Whole ≤ Bash | Started ≤ 1/2 | Started ≤ 2/3 | Started ≤ Bash |
|---|---:|---:|---:|---:|---:|---:|
| Debian | 13/649 | 450/649 | 639/649 | 7/584 | 26/584 | 558/584 |
| Ubuntu | 16/646 | 441/646 | 638/646 | 7/583 | 20/583 | 561/583 |
| macOS | 2/629 | 562/629 | 629/629 | 8/581 | 70/581 | 567/581 |

All matching names received a verified whole-run timing. Compared with the November 22 measurements, four Debian and four Ubuntu whole-run medians crossed above 2/3. On macOS, 12 whole-run and 33 already-started medians crossed above it. The snapshot comparison found no lost matching observations or missing earlier measurements. These counts can include multiple names for one script. The absolute target remains unmet for a number of existing workloads, particularly one-shot commands measured from a ready shell through exit.

機能が一致した全項目で起動込みの計測ができました。起動済みの計測では、短い処理に同期・出力の回収・シェルの終了待ちまで含まれます。新たに一致した36項目のうち起動込みの中央値が Bash の3分の2以内だったのは Debian 25件、Ubuntu 30件、macOS 28件です。この36項目と反復スクリプトの4処理は別の測定条件です。11月22日の計測で達成していた上限を今回超えたのは、Debian と Ubuntu の起動込みが各4件、macOS の起動込みが12件・起動済みが33件です。[現行版の各項目](compatibility-2026-11-23/matrix.md) · [11月22日の比較元](compatibility/matrix.md)


<a id="login-profiles"></a>

## 2026-11-23 — Login profiles / ログイン設定

Median and p95 in milliseconds from parent launch to shell exit. Each environment ran 101 samples after five warmups per executable, rotating Bash, the previous Tonari release (`f3f5539`) and the new release. All three executables were named `bash` in separate directories on the same filesystem. The Python parent used a temporary working directory and home, `TERM=dumb`, `LC_ALL=C`, `PATH=/usr/bin:/bin` and standard descriptors connected to `/dev/null`. Linux used the distribution's `/etc/profile` and copies of its skeleton `.profile` and `.bashrc`. macOS used its system profile and an empty user home. These figures include Python's subprocess setup. On Linux the earlier binary skipped the Bash-gated rc body, so its interactive login did less work. Two GNU correctness runs were active in the Linux VM during sampling.

各セルは起動から終了までの中央値 / p95（ms）です。通常の `-c :`、`-lc :`、対話フラグを付けた `-lic :`、同一シェルからの外部コマンド16回を比較しました。対話フラグを付けた行では端末描画を測定していません。Linux の旧版では標準設定の一部を読み飛ばしていたため、この行の新旧では実行した仕事が異なります。

| Platform | Workload | Bash | Previous Tonari | Tonari |
|---|---|---:|---:|---:|
| macOS arm64 | `-c :` | 6.942/7.762 | 3.999/4.572 | 3.867/4.694 |
| macOS arm64 | `-lc :` | 9.827/11.107 | 5.888/7.299 | 5.865/6.923 |
| macOS arm64 | `-lic :` | 9.691/10.668 | 5.685/6.507 | 5.686/6.677 |
| macOS arm64 | 16 × `/usr/bin/true` | 37.743/46.777 | 23.853/29.467 | 23.527/26.695 |
| Debian 13 arm64 | `-c :` | 0.513/0.953 | 0.246/0.784 | 0.244/0.478 |
| Debian 13 arm64 | `-lc :` | 1.509/1.881 | 1.409/1.861 | 1.445/1.753 |
| Debian 13 arm64 | `-lic :` | 2.216/2.800 | 1.675/2.125 | 2.325/2.777 |
| Debian 13 arm64 | 16 × `/usr/bin/true` | 5.011/6.761 | 3.547/4.021 | 3.505/4.307 |
| Ubuntu 26.04 arm64 | `-c :` | 0.535/1.276 | 0.262/0.606 | 0.260/0.656 |
| Ubuntu 26.04 arm64 | `-lc :` | 1.528/2.484 | 1.337/1.837 | 1.282/2.471 |
| Ubuntu 26.04 arm64 | `-lic :` | 3.642/4.560 | 1.954/3.257 | 3.795/5.168 |
| Ubuntu 26.04 arm64 | 16 × `/usr/bin/true` | 4.989/6.258 | 3.542/4.308 | 3.589/4.115 |

The macOS executable is 1,052,928 bytes (+33,424), compared with 1,208,904 for the native reference Bash. The static arm64 Linux executable is 1,243,040 bytes (+40,896), compared with 1,412,096 for Debian Bash and 1,609,176 for Ubuntu Bash. Plain command startup meets the two-thirds target in all three environments here. Both Linux interactive logins with stock profiles take about as long as Bash. The 16-process Linux timings are also above two-thirds. [Session checks](../tests/shell.py)

macOS 版は 1,052,928 バイト、静的 Linux 版は 1,243,040 バイトで、各参照 Bash の実行ファイルより小さいサイズです。通常のコマンド起動はいずれの環境でも目標内でした。Linux の標準設定を読む対話ログインは Bash とほぼ同じ所要時間で、16プロセスの連続起動も両ディストリビューションで 2/3 を超えました。


In a macOS arm64 PTY, the completion harness separately timed the typed key, its grey history suggestion and Tab completion over 10,000 names. Values are median / p95 in milliseconds, 15 samples per executable. The Bash measurements cover Tab. macOS でキー入力から表示までと Tab 補完を分けて測定しました。[PTY benchmark](../tools/bench_completion.py)

| Operation | Previous Tonari | Tonari | Bash 5.2.37 |
|---|---:|---:|---:|
| recent hint | 0.021 / 0.070 | 0.027 / 0.092 | — |
| no hint | 0.038 / 0.117 | 0.036 / 0.117 | — |
| oldest hint: typed key | 0.027 / 0.100 | 0.031 / 0.138 | — |
| oldest hint: visible suggestion | 1.238 / 1.685 | 1.230 / 2.329 | — |
| files | 6.411 / 8.457 | 6.720 / 8.742 | 12.899 / 15.840 |
| complete -f | 6.286 / 10.557 | 6.222 / 6.892 | 14.077 / 15.662 |
| PATH | 6.121 / 11.437 | 6.390 / 8.192 | 200.342 / 207.434 |

### Full timing matrix for the login changes / ログイン対応後の全件計測

The candidate is `c9f74e5`. The check compared its full matrix with the previous release snapshot before publication. Each cell counts named observations, including shared workloads. The three counts are **half or better / two-thirds or better / parity or better**. The denominator counts cases with valid timing. The raw comparisons apply no short-operation allowance. [Current matrix and paired observations](compatibility-2026-11-23-login/matrix.md) · [Previous matrix](compatibility-2026-11-23/matrix.md)

| Platform | Whole command | Measured | Already started | Measured | Previously within two-thirds, now above (whole / started) |
|---|---:|---:|---:|---:|---:|
| Debian | 27 / 419 / 682 | 692 | 8 / 50 / 531 | 623 | 86 / 7 |
| Ubuntu | 41 / 413 / 674 | 689 | 5 / 40 / 531 | 622 | 60 / 9 |
| macOS | 1 / 580 / 672 | 672 | 5 / 38 / 602 | 620 | 29 / 45 |

The first macOS started sample set for `set -o monitor` stopped when the Bash reference observation changed. A separate run with the same executable hashes, warmups and 15 samples passed the output checks. The published started interval uses that run. All previously measured matching names retained a validated interval. The complete benchmark passes correctness for those timed cases. The raw two-thirds target still has misses on every platform.

全環境で照合できた処理について、起動込みと起動後の測定を更新しました。macOS の `set -o monitor` は最初の実行で参照 Bash の結果が途中で変わったため、同一バイナリでの独立した再計測を採用しました。表の右端は前版で 2/3 以内だった項目のうち今回上回った件数です。短い処理を含むので、各件の絶対時間とばらつきは TSV に記録しています。

The three full timing runs overlapped on one arm64 host and its Linux VM. A subsequent serial Linux check ran 51 samples for 12 selected workloads. On Debian, the started `${v##a*}` probe took 0.736 ms against 0.735 ms previously and 0.982 ms on Bash. The complete interactive `COMP_TYPE` callback probe took 3.177 ms against 2.955 ms previously and 2.920 ms on Bash. On Ubuntu those callback medians were 2.880, 2.806 and 2.394 ms. The latter includes PTY startup and command dispatch. The key-to-display measurements above exercise a running editor.

Linux の再計測では、短い変数展開や記述子操作の一部で並列計測時と異なる比率が出ました。入力への応答と、シェル起動を含む補完コールバックは別々に測定しており、コールバックの起動込み所要時間には改善余地が残ります。
