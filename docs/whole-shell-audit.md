# Whole-shell review / シェル全体の監査

The starting point was `1754509`, the first main commit after PR #58. Separate reviewers traced terminal input, language execution and child processes in a source snapshot before seeing earlier findings. A fourth pass followed startup, regular stdin and builtins. A fresh reviewer checked the combined changes. The expanded Clippy pass covered all 109 Rust sources in the workspace. The manual review followed the execution paths below. [Language and pathname results](whole-shell-audit-language.md) · [Process and I/O results](whole-shell-audit-process.md)

PR #58 のマージ直後の main を基準に、端末、言語処理、プロセスを別々にソースから追い、起動・標準入力・組み込みも確認した。修正後は別の担当が差分と呼び出し元を調べた。Rust 全109ファイルには静的解析をかけた。以下は実際に観測した範囲である。

## Startup and shared input / 起動と標準入力

- Importing functions before seeing the entire environment made POSIX identifier checks depend on the startup path. The import now uses explicit options and both POSIX environment flags regardless of entry order. Bash starts its `sh` invocation mode and imports `SHELLOPTS` later. Both behaviors have separate tests. Syntax-only invocations skip the function bodies, and privileged invocation skips imports.
- An explicitly interactive shell without a controlling terminal advertised job control. It now clears the startup monitor flag and reports when it cannot acquire the terminal. Tonari accepts a later explicit `set -m`. It also honors an explicit noninteractive `-m` for child process groups even in a detached session, where GNU Bash can disable startup monitoring. [Detached process-group witness](../tests/compat/source_review.py)
- An interactive shell launched from its caller's foreground group left the terminal pointing at the exited replacement after `exec`. It now hands the foreground back before replacement and resumes its own job control if replacement fails. The private PTY regression checks the replacement's group, the caller's foreground group and the surviving shell's group after failure.
- A long compound command read from regular stdin can inspect up to 64 KiB with a positional read. The parser certifies the command's physical end before the shell advances its shared offset. One trial used only bytes already present in the 8 KiB reader. Its 1,024-branch Mac median was 7.35 ms versus 7.32 ms on main. A larger positional window in a separate 15-sample comparison took 6.85 ms versus 7.44 ms on main. Pipes keep the line reader so the next command can use its input.
- The final reviewer found that a quoted line resembling an `if` and a later comment resembling a closing `fi` could make the trial consume data meant for the command's `read`. The regression checks the actual data read and the next command with short and 12 KiB inputs, including another command on the closing line. Alias expansion can change source locations, so shared input uses the ordinary reader in that case. [Startup, job-control and stream-boundary regressions](../tests/shell.py)

環境から取り込む関数は実際の起動モードで名前を検査する。環境変数の順序、`sh` 名での起動、`-n`、特権モードを個別に確認した。制御端末を取得できない強制対話起動ではジョブ制御の無効を通知する。標準入力が通常ファイルの場合だけ複合コマンドの先を調べ、パーサーが確定した境界で読み位置を同期する。末尾のコメントがデータであるケースも検証した。

端末のない非対話起動でも、明示した `-m` でジョブのプロセスグループを作るのは Tonari の仕様である。親と同じフォアグラウンドグループから起動した対話シェルでは、`exec` に成功しても親に端末が戻り、失敗してもそのシェルでジョブ制御を続けられることを専用 PTY で確かめる。

On Linux, Yash case 8116 was the first program in its sequence to leave the terminal pointing at the wrong group. That broke SIGINT expectations in later cases even though each passed on a fresh PTY. All 61 cases from 8116 to 8176 passed on one PTY after the repair. A second regression sends diagnostics to another PTY during a failed replacement. Recovery uses the original close-on-exec descriptor. In 31 interleaved trials on a physical M1, launching an interactive shell through successful `exec /usr/bin/true` took 6.80 / 6.69 / 13.05 ms on main / candidate / GNU Bash 5.3.20. Failed replacement followed by `:` took 5.85 / 5.98 / 12.31 ms. The driver restored the controller's foreground group between trials if needed.

Linux の信号試験は、単独で通るケースでも前のケースが端末を返さないと失敗していた。同じ PTY を引き継ぐ 61 件で修正を確認した。`exec` 失敗時に標準エラーを別端末へ向けた場合も元の端末を保持する。[GNU Bash との差と回帰試験](compatibility.md)

## Terminal input / 対話入力

The screen oracle records each PTY read before decoding it. It distinguishes the first output from the frame showing the expected text on both sides of the caret, then separately verifies the shell's entire edit buffer. A negative control feeds a plausible marker-only frame with missing text. For a scrolled line the observer requires a contiguous suffix of the expected input and visible context. Each timing distribution has 100 observations. [PTY driver](../tools/ci/whole-editor.py) · [Edit, resize, completion and foreground-input regressions](../tests/review_whole_editor.py)

小さい分割画面の Home、結合文字を含む検索語の削除、プロセス置換内の Tab 補完、候補を絞り込んでいる間のキャンセルを直した。長い入力では先頭のまとまりを画面に早く返す。Enter の後ろの入力はフォアグラウンドの子に残す。PTY の先頭バイト・表示内容とカーソル・編集バッファを個別に確認した。ここで測れるのは IME 確定文字列の受け渡し以降であり、変換候補ウィンドウと GPU 描画の計測は範囲外である。

## Attribution and limits / 内訳と限界

On a physical M1, Time Profiler took 109 samples in an independent baseline run that appended a Japanese character and requested the growing length 90,000 times. One instruction range accounted for 89 of those samples. Its release disassembly already contained ARM vector shifts (`ushr.2d`). With incremental character counts, the same input yielded 20 samples in total. System Trace on 25 two-stage external pipelines followed by a 150 ms child wait showed 20.75 ms running and 222.28 ms blocked in the shell's main thread, excluding the initial instrumentation stop. These traces locate work; the release comparisons use untraced runs. Linux collects only usable perf events on each host.

The review also followed atomic signal notification into the interpreter, private descriptor relocation and close-on-exec, unnamed temporary files and their fallback, distinctions between arithmetic data and parsed source, directory ownership of physical paths, and Linux allocator registration before the first allocation. The ordinary regression suites exercise state restoration, interrupted waits, literal data, read boundaries and file contents. A short fallback expression flagged by Clippy in environment construction already borrows the default via `Cow`. The parser's static error messages also need no heap allocation. Neither finding justified a change.

M1 の CPU プロファイルと逆アセンブルでは、最も時間のかかった走査に既にベクトル命令が出ていた。追記済み文字列の再走査そのものを省いた。プロセスの待機は別の System Trace で測定した。シグナル通知・一時ファイル・非継承 FD・算術展開・アロケータの fork 前提もソースを追った。ハードウェアカウンターはホストで取得できたものを記録した。全入力や全スケジュールを網羅する証明ではない。

## Native results at fd906ad / fd906ad の実機測定

All 11 jobs passed in the comparison of `1754509` and `fd906ad`. The macOS 15.8.2 runner used a three-vCPU Apple M1 VM and GNU Bash 5.3.20. Ubuntu 24.04 used four x86-64 vCPUs and GNU Bash 5.2.21. Its startup, terminal and process jobs ran on Intel Xeon 6975P-C hosts. Language and profiling ran on AMD EPYC 7763 and 9V74 hosts respectively. The arm64 Mach-O grew from 2,129,648 to 2,146,176 bytes and linked only libSystem. The stripped x86-64 static PIE grew from 2,808,216 to 2,819,448 bytes. [Run and artifacts](https://github.com/sanohiro/tonari-shell/actions/runs/46167162358)

**Terminal latency in milliseconds.** Each cell gives p95 / p99 / maximum across 100 observations of the candidate. The first response and the frame containing the expected text and caret were measured independently. The IME case sends committed Japanese text. [PTY driver at fd906ad](https://github.com/sanohiro/tonari-shell/blob/fd906ad/tools/ci/whole-editor.py)

| Input / 操作 | macOS first | macOS frame | Linux first | Linux frame |
|---|---:|---:|---:|---:|
| Single ASCII key | 0.224 / 1.045 / 2.514 | 0.224 / 1.045 / 2.514 | 0.066 / 0.174 / 0.288 | 0.066 / 0.174 / 0.288 |
| Mid-line insertion | 0.319 / 0.624 / 0.676 | 0.319 / 0.624 / 0.796 | 0.102 / 0.291 / 0.373 | 0.102 / 0.291 / 0.373 |
| Long ASCII insertion | 1.174 / 1.483 / 4.056 | 7.668 / 9.313 / 9.941 | 0.142 / 0.177 / 0.335 | 1.213 / 1.260 / 1.422 |
| Committed Japanese text | 1.363 / 1.754 / 7.146 | 7.272 / 10.308 / 12.870 | 0.152 / 0.161 / 0.188 | 1.070 / 1.128 / 1.167 |
| Tab completion | 2.795 / 2.884 / 2.889 | 2.795 / 2.884 / 2.889 | 1.136 / 1.205 / 1.285 | 1.136 / 1.205 / 1.285 |

On macOS the Japanese frame p99 rose from 7.895 to 10.308 ms versus the baseline. Bash recorded 19.934 ms. Linux Bash completed the Tab frame sooner, with p99 1.005 ms. Pinning the Linux editor and observer to one allowed CPU raised ASCII first-response p95 / p99 / max to 1.360 / 1.402 / 1.502 ms and frame latency to 1.498 / 1.579 / 1.591 ms. Japanese frame latency on one CPU was 1.380 / 1.436 / 1.507 ms. The candidate passed the frame and edit-buffer checks. Bash failed the Linux multiline and split-Home oracles, and the baseline failed split-Home on both systems. Those trials have no comparable timing.

**Startup and pathname expansion.** Values are candidate / Bash medians in milliseconds, with 15 measured runs per shell. The glob fixture contains 2,048 names and runs in the C locale. [Startup and glob driver at fd906ad](https://github.com/sanohiro/tonari-shell/blob/fd906ad/tools/ci/whole-startup.py)

| Measurement / 項目 | macOS | Linux |
|---|---:|---:|
| Noninteractive `-c :`, launch through exit | 5.067 / 10.761 | 0.254 / 0.643 |
| Clean interactive launch to prompt | 4.052 / 12.264 | 0.503 / 1.011 |
| Clean first command after prompt | 0.189 / 0.278 | 0.077 / 0.089 |
| Login prompt, system profile and 80-function user rc | 5.040 / 10.321 | 9.966 / 10.584 |
| Login prompt with profiles disabled | 3.289 / 8.270 | 0.489 / 1.035 |
| One glob | 8.337 / 234.183 | 1.143 / 1.768 |
| Four globs in one shell | 9.873 / 574.604 | 2.861 / 4.617 |

The Linux login miss against the two-thirds target included five profile utilities in a separate syscall trace. The clean trace recorded only the shell's exec. Linux three-stage pipelines at 32 repetitions also missed at 19.399 / 24.548 ms. The 1 MiB slow-reader test took 33.282 / 37.092 ms on Linux, with a direct OS pipeline at 32.001 ms. On macOS the three times were 204.498 / 205.249 / 209.444 ms. For a CPU-bound control, appending and measuring a growing Japanese string 24,000 times reduced Linux user task-clock from 24.47 to 4.80 ms. Hardware cycles, instructions and branch events were unavailable. Bash 5.2 lacked numeric GLOBSORT, so its Linux numeric-sort timings are omitted.

両 OS のネイティブ実行で 11 ジョブが完走した。端末の表は各 100 回の先頭応答と、文字・カーソルを確認したフレームの p95 / p99 / 最大値。macOS の日本語入力は先頭描画後も処理が続き、フレーム p99 は旧版 7.895 ms から 10.308 ms に増えた。Linux の 1 CPU 制限では長い ASCII 入力のフレーム p99 が 1.579 ms になった。起動表ではプロンプトと最初のコマンドを分けている。Linux のログインではシステムプロファイルが外部コマンドを 5 本起動し、3 段パイプラインも Bash 比 2/3 の目標を超えた。遅い読み手には同じ読み手を用いた OS 直接実行、文字数計算には task-clock を対照として用いた。数値はすべて中間リビジョン `fd906ad` の結果である。

## Command completion through PATH aliases / 同じディレクトリを指す PATH の補完

When two PATH entries open the same directory, Tonari reuses its complete name listing. It checks the device, nonzero inode and directory timestamps on the opened descriptor. Before sharing the list, Tonari opens the alternate spelling. It applies executable and `EXECIGNORE` checks to candidates through the current PATH entry. This preserves repeated names and their PATH order in `compgen -c`, including after a symlink is retargeted or a command is renamed. [Cache implementation](../src/complete/command_cache.rs) · [Regression tests](../tests/shell.py)

On a physical Apple M1, 60 interleaved fresh shells per case completed the same command on the first and second Tab. Values below are p50 / p95 milliseconds to the completed text and caret. The fixture used 1,024 names per directory. The native macOS `/usr/bin` and `/bin` are distinct directories. Both Tonari arm64 executables were 2,146,192 bytes and linked only libSystem. Reproduce with `python tools/ci/completion-alias.py before=/path/to/ee00503 after=/path/to/d38b4ae bash=/path/to/bash --runs 60` in the PTY driver's pyte environment. [Measurement driver](../tools/ci/completion-alias.py)

| PATH and Tab / 操作 | Before / 変更前 | After / 変更後 | GNU Bash 5.3.20 |
|---|---:|---:|---:|
| Directory + symlink, first Tab | 1.022 / 1.256 | 0.586 / 0.732 | 2.379 / 3.178 |
| Directory + symlink, second Tab | 0.061 / 0.162 | 0.081 / 0.140 | 1.582 / 1.774 |
| Two distinct directories, first Tab | 1.059 / 2.198 | 1.024 / 2.573 | 2.453 / 3.879 |
| One directory, first Tab | 0.640 / 1.426 | 0.729 / 1.692 | 1.875 / 3.786 |
| Native system PATH, first Tab | 0.658 / 1.576 | 0.652 / 1.809 | 1.755 / 4.262 |

A separate running-shell control repeated `compgen -c zzuni` 256 times and checked all 256 outputs. Across seven interleaved runs, the symlink case rose from 7.926 to 10.265 ms, or about 9 µs per call, including the check that the alias can still be opened. The single-directory case took 7.345 and 7.192 ms, and two distinct directories took 8.832 and 8.827 ms.

同一ディレクトリを指す PATH では、実際に開いた FD のデバイス・inode・変更時刻を確かめ、完全な名前一覧を再利用する。実行権限と `EXECIGNORE` は各パスで判定し、重複する候補の順序を保つ。物理 M1 で最初の Tab の中央値は 1.022 ms から 0.586 ms、p95 は 1.256 ms から 0.732 ms になった。2 回目の Tab の中央値は 0.061 ms から 0.081 ms に増えた。別名でも毎回ディレクトリを開けるか確認するため、`compgen` を 256 回繰り返した場合の増分は 1 回あたり約 9 µs だった。

On Ubuntu's real `/usr/bin:/bin` PATH, 50 interleaved fresh shells gave first-Tab medians of 1.105 / 0.606 / 0.816 ms for the original main, revised Tonari and GNU Bash 5.2.21. The second Tab took 0.054 / 0.055 / 0.757 ms. One distinct directory took 0.289 / 0.289 / 0.235 ms on first use, and two distinct directories took 0.479 / 0.483 / 0.383 ms. These fixtures distinguish reusing an opened directory's names from the cost of building a new index. Ubuntu の初回 Tab は、同一ディレクトリを指す標準 PATH で Bash を下回った。個別のディレクトリだけの初回は依然 Bash のほうが速く、2 回目は Tonari のほうが速い。[Native checks and samples](https://github.com/sanohiro/tonari-shell/actions/runs/46169194265)

## Checked release workloads / 修正後の実測

The following Linux medians include startup. All three programs ran on the same native Xeon 6975P-C runner for each comparison. Data size, iteration count and output were checked by the drivers. [Release artifacts](https://github.com/sanohiro/tonari-shell/actions/runs/46169194265)

| Workload / 条件 | Main 1754509 (ms) | Revised d38b4ae (ms) | GNU Bash 5.2.21 (ms) |
|---|---:|---:|---:|
| Double-quoted file with 768 lines of parameter defaults | 75.323 | 1.261 | 2.155 |
| 150 anchored regex misses with 2,048 leading characters | 3.370 | 0.445 | 3.590 |
| 4,096 short builtin writes to an open descriptor | 5.209 | 3.936 | 12.657 |
| 32 pipelines with three stages | 19.942 | 19.894 | 24.664 |

Linux で 4,096 回の書き込みをループの外で一度だけリダイレクトすると 2.331 ms、直接ファイルへ書く対照は 1.745 ms だった。共有パイプから標準入力を読む 768 行の別試験では、先読みで後続コマンドからデータを奪わないよう両シェルとも細かく読み、`strace` では Tonari 13,099 回、Bash 13,095 回の read を記録した。先頭からの構文再解析を省くことと、共有入力の境界を保つことは別々に検証した。[Stream attribution](https://github.com/sanohiro/tonari-shell/actions/runs/46168594507)
