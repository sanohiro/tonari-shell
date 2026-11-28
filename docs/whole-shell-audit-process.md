# Process execution and descriptor ownership / プロセス実行と記述子の所有

## Source review and witnesses / 実装と照合

The review began with unchanged source at `1754509`. GNU Bash 5.3.20 was the behavioral reference on macOS arm64. The shared witness also records the two function-tail results from Debian GNU Bash 5.2.37. All inputs use the public shell interface. [Witnesses](../tests/review_whole_process.py)

| Path | Work and observation |
|---|---|
| Command preparation, lookup and launch | `start_raw` can prepare literal words in the parent, while `hash::search_in` walks PATH and the external launcher packs NUL-terminated argv and environment pointers. A literal right-hand side still runs arithmetic for an integer or nameref target. At the baseline, `declare -i n; side=0; n=side++ \| :` incremented the parent. A failed background glob or readonly assignment also lost its waitable PID. GNU forks before expansion in `execute_simple_command`. Tonari now prepares observable assignments and async expansions in the stage. The normal foreground and lastpipe cases keep their own assignment semantics. |
| Subshell and external replacement | An asynchronous simple stage expands `BASH_SUBSHELL` after incrementing it. A foreground pipeline expands its words before incrementing. GNU's `execute_disk_command` adjusts `SHLVL` when an external replaces a standalone child. Implicit stdin from /dev/null is distinct from a pipeline. A directly redirected external inside parentheses or a coprocess can replace the child. Explicit timing, a preceding list command with a redirected tail, a function body and genuine pipelines exercise different rules. |
| Functions and wait | GNU 5.3's `optimize_shell_function` considers the body and whether the -c reader has consumed the last input line. Nested function calls, explicit parentheses, coprocesses, command substitutions and standalone async functions exercise the tail decision separately. GNU 5.2 keeps the intermediate process in the two file/command-string controls recorded by the witness. Explicit repeated `wait -n -p name PID` and a later ordinary `wait PID` returned status 7 in both Tonari and GNU 5.3. [Versioned wait controls](../tests/review_runtime.py) |
| I/O, redirection and substitution | The output builtins write through fd 1. Bounded literal producers may fill a fresh pipe before the consumer launches. Larger producers run concurrently. Descriptor saving and restoration use private CLOEXEC copies, and regular-file input can return its unread suffix with lseek. An ordinary delimiter-based read from a shared pipe must leave the next record for a different process. Exact-length raw input can read in blocks. The command-substitution parent drains output before waiting. The small/large stream driver checks every output byte over redirects, pipes, captured data, external capture, `read -N` and heredocs. [Stream driver](../tools/bench_streams.py) |
| Jobs, traps, process substitution and coprocesses | A job table preserves insertion order across waiting and disowning. The next job ID now comes from its final entry. The prior implementation scanned the entire table and stored a separate serial field. Signal handlers record pending events and waits poll with SIGCHLD blocked before suspending. The signal witness interrupts a wait and collects the unchanged child's status on the next wait. A live disowned coprocess retains working read/write descriptors. A targeted wait can reap a process substitution even after its argument descriptor leaves scope. |

通常の文字列に見える代入でも、整数属性や名前参照を通じて算術評価が親を変更しうる。パイプライン・背景実行の展開時期、失敗時の待機可能な PID、親の変数値を個別に照合した。背景ジョブに与える /dev/null と実際のパイプを区別し、外部コマンドが子シェルを置き換える条件を修正した。括弧内の単独コマンド、コプロセス、関数、`time`、スクリプトファイル、残りの入力行を持つ `-c` も検査した。ジョブ番号は順序を保つテーブルの末尾から求める。wait の再実行、シグナルで中断した wait、disown 後に生きているコプロセスの記述子、共有入力の次の行も確認した。

## Native measurements / 実機測定

Apple M1, macOS arm64 distribution builds, GNU Bash 5.3.20, `LC_ALL=C`, no inherited SHLVL. One warmup and seven interleaved checked samples at each size. Values are median wall-clock milliseconds including shell startup. These are local development measurements. CI records the release comparison and full ranges by platform. [Timing driver](../tools/ci/whole-process.py)

| Workload | Sizes | Tonari | GNU Bash |
|---|---|---|---|
| Plain assignment piped to : | 8 / 16 / 32 commands | 4.675 / 4.470 / 4.583 | 19.085 / 26.279 / 43.973 |
| Integer assignment isolated in pipeline stage | 8 / 16 / 32 commands | 9.358 / 14.408 / 24.447 | 18.420 / 28.132 / 44.862 |
| Serial external true | 8 / 16 / 32 commands | 15.947 / 25.219 / 49.026 | 26.738 / 44.041 / 78.076 |
| Builtin producer and two external cat stages | 8 / 16 / 32 pipelines | 26.687 / 43.839 / 81.902 | 37.418 / 62.961 / 114.129 |
| Add background jobs then wait | 8 / 16 / 32 jobs | 7.851 / 9.971 / 14.648 | 13.678 / 16.400 / 22.634 |
| Command substitution from regular file | 256 / 512 / 1024 KiB | 4.517 / 4.522 / 4.750 | 10.409 / 11.325 / 13.095 |
| printf to pipe | 256 / 512 / 1024 KiB | 6.913 / 7.123 / 7.337 | 14.523 / 17.656 / 24.539 |
| Exact count from pipe | 256 / 512 / 1024 KiB | 6.171 / 6.052 / 6.943 | 15.861 / 19.156 / 25.621 |
| Delimited pipe read followed by external cat | 4 / 8 / 16 KiB record | 9.319 / 10.685 / 12.964 | 16.497 / 17.277 / 19.953 |
| Reader exits after one byte | 256 / 512 / 1024 KiB | 30.642 / 31.213 / 32.239 | 39.321 / 39.120 / 40.703 |
| Slow reader | 256 / 512 / 1024 KiB | 35.982 / 40.796 / 51.560 | 42.845 / 50.259 / 62.093 |

At N=32 the baseline integer pipeline took 4.571 ms with the wrong parent value. The first correct stage implementation always forked for assignments or background preparation. Restricting the new child work to observable cases brought a plain 32-command assignment pipeline from 23.596 to 4.337 ms in a separate interleaved comparison (GNU 43.218 ms). Keeping the grouped native spawn path for a literal monitored background external took 44.055 ms, down from 75.668 ms (GNU 76.343 ms). The 32-job insertion control measured 15.291 and 14.226 ms respectively, with overlapping ranges. Its directly verifiable gain is eliminating the linear ID scan.

The 1 MiB direct OS pipeline, with /bin/cat and the identical consumer, took 31.611 ms for the early exit and 49.166 ms for the delayed read. The three-stage N=32 pipeline remains above the two-thirds ratio at 81.902 versus 114.129 ms. The baseline took 81.484 ms. Separate `bench_streams.py --runs 4 --sizes 0 1 4096 262144` checks passed all 24 workload/size combinations. With 2, 4 and 8 deliberately pending process substitutions and 1,000 unrelated literal commands, three-sample readings were 5.857 / 6.270 / 8.732 ms against GNU's 15.133 / 17.160 / 23.320 ms. Pending-PID polling remains proportional to the number of live substitutions. Both the witness and timed reader return the expected unread suffix and check the status of a producer whose consumer exits early.

実測は各規模でウォームアップ 1 回、測定 7 回の交互実行。初版は代入を一律に子で処理した。親から観測される代入・展開に限ると、32 回の通常代入パイプラインは 23.596 ms から 4.337 ms になった。早期終了・遅い読み手では同じ消費側を使った直接 OS パイプも測った。1 MiB の遅い読み手は Tonari 51.560 ms、直接実行 49.166 ms。実プロセスを使う 3 段パイプラインは GNU 比 2/3 の目標を超えた。未終了のプロセス置換を保つ際の PID ポーリングは件数依存のまま。0、1、4096、262144 バイトの出力は全 24 組でバイト単位に照合した。

```sh
python3 tests/review_whole_process.py "$current" "$bash"
python3 tests/review_runtime.py "$current" "$bash"
BENCH_RUNS=7 python3 tools/ci/whole-process.py "$baseline" "$current" "$bash"
python3 tools/bench_streams.py --runs 4 --sizes 0 1 4096 262144 -- "$baseline" "$current" "$bash"
```

## Default read deadlines / read の既定の待ち時間

With a pipe writer still open after sending `partial`, GNU Bash 5.3.20 and 5.2.37 applied `TMOUT=.05` to `read`, assigned the seven bytes, and returned 142. Baseline Tonari kept waiting. The corrected builtin reads the current shell setting, accepts an explicit `-t` override, ignores zero or invalid defaults, and allows regular-file reads to finish. The regression also checks immediate input, EOF and a closed descriptor. An executable text file without a shebang inherited the warmed setting. A temporary `TMOUT` on its command line could enable or disable the child deadline while the parent kept its original value. Two other source hypotheses matched GNU 5.3.20 in the original build: `printf 'hello%n' ref` assigned 5 through a nameref to an indexed element, and entering then leaving POSIX mode gave the same `shopt` state. [Read regression](../tests/shell.py)

Apple M1, macOS 27.2.1, native Cargo release, 4,000 reads per shell, one warmup, then 15 shuffled samples per executable. The table reports median milliseconds from launch to verified output. The candidate and first working build were 2,478,352 bytes each. GNU Bash was 873,872 bytes.

| Input | Original `1754509` | Before correction on the audit branch | First correct | Optimized | GNU 5.3.20 |
|---|---:|---:|---:|---:|---:|
| Regular file, unset TMOUT | 11.81 | 12.02 | 12.31 | 12.25 | 26.86 |
| One short here-string per read, unset TMOUT | 13.05 | 13.31 | 13.36 | 13.13 | 55.19 |
| Regular file, TMOUT=1 | 11.74 | 12.14 | 14.44 | 13.63 | 28.32 |

The common unset case uses the existing inherited-setting cache. Configured regular-file reads use the file-type check to establish descriptor validity. That change reduced the configured case from 14.32 ms with the cache alone to 13.63 ms, with interquartile ranges of 14.12–14.75 and 13.56–13.80 ms. Changes for unset TMOUT overlapped sample variation. The first two Tonari columns did not implement the default deadline on pipes.

パイプに一部の入力だけを残すと GNU Bash 5.2.37 と 5.3.20 はその内容を変数に保存し、指定時間に終了値 142 を返した。Tonari でもシェル変数の既定値、明示的な上書き、通常ファイル、EOF を区別した。通常ファイルで待ち時間を指定した 4,000 回の読み取りでは、記述子の重複検査を省き、中央値が 14.32 ms から 13.63 ms になった。未指定の場合の差は測定のばらつきの範囲内だった。

```sh
REFERENCE_BASH="$bash" python3 tests/shell.py "$current" ShellTests.test_read_uses_tmout_and_preserves_partial_input
python3 -c 'from pathlib import Path; Path("records").write_bytes(b"line of text\n" * 4000)'
"$shell" -c 'n=0; while IFS= read -r line; do ((n++)); done <records; echo "$n/$line"'
"$shell" -c 'n=0; while ((n<4000)); do IFS= read -r line <<<"line of text" || exit 2; ((n++)); done; echo "$n/$line"'
"$shell" -c 'TMOUT=1; n=0; while IFS= read -r line; do ((n++)); done <records; echo "$n/$line"'
```

## Repeated output to an open descriptor / 開いた記述子への反復出力

The original short benchmark starts a shell, writes 32 lines with `echo abc >&7` and launches `wc`. A Linux native sample took 2.041 ms in Tonari and 1.702 ms in GNU. The follow-up removed `wc`, checked every byte in the driver and varied the number of writes independently. The empty loop, a single enclosing redirection, and direct unbuffered regular-file writes through Python each have their own measurement. [Reproduction driver](../tools/ci/whole-process.py)

For eligible echo and printf output of at most 512 bytes, Tonari now validates the source by reserving a duplicate, closes the duplicate, and writes to the source. The usual temporary redirect performs a backup, installs fd 1, writes, restores fd 1 and closes its backup. Removing the two dup2 calls also avoids allocating a tracked backup for each builtin. The reservation preserves a missing-source error even for zero-length output. A closed standard descriptor can provide the one spare slot GNU uses at a low limit. It is closed before rendering, so diagnostics and subsequent commands see the original descriptor state. Full tables with a live stdout, originally closed stdout, read-only destinations and EPIPE are covered. Larger output uses the ordinary builtin path.

With the child’s descriptor table full, the Linux musl build reports `No file descriptors available (os error 24)` and GNU Bash on glibc reports `Too many open files`. The regression accepts both spellings of EMFILE and requires status 1 and an empty output file. Closing one standard descriptor must then allow the write to succeed and preserve its bytes. [Linux CI diagnostic](https://github.com/sanohiro/tonari-shell/actions/runs/46168594507/job/141051637162)

GNU skips both the duplicate and the closing operation when the source and destination of a move are equal. Previously Tonari closed a live fd for `exec 7>&7-` and treated `printf '' >&1` with a closed stdout as a failed redirect. These operations now preserve the descriptor. Real references elsewhere in the same list still move a private shell descriptor out of the way. Malformed hexadecimal and Unicode escapes in printf now flush completed lines before their diagnostic on merged output, for format strings and %b arguments. The direct renderer accepts formats that can run without that diagnostic. [Regression checks](../tests/review_whole_process.py)

Native M1 medians in milliseconds, one warmup and eleven checked shuffled samples. The pre-change executable, candidate and GNU Bash 5.3.20 measured 2,146,192, 2,146,192 and 873,872 bytes.

| Writes | Per-command redirect before | After | GNU | One enclosing redirect after | Empty loop after | Direct writes |
|---:|---:|---:|---:|---:|---:|---:|
| 1,024 | 9.284 | 8.245 | 18.186 | 7.564 | 5.232 | 2.505 |
| 2,048 | 11.819 | 10.646 | 22.242 | 9.419 | 4.409 | 4.777 |
| 4,096 | 18.482 | 16.565 | 34.783 | 14.061 | 4.593 | 9.396 |

At 4,096 writes the candidate ranged from 16.191 to 21.578 ms. The baseline ranged from 17.448 to 19.644 ms. A separate nine-sample comparison of the initial direct-writer prototype measured 16.505 versus 18.961 ms.

短い測定には外部 wc の起動も含まれるため、出力をドライバー側で照合し、反復回数を独立に変えた。512 バイト以下の対象形式では、記述子を一時複製して資源・有効性を確認した後、元の出力先に書き込む。標準出力の付け替え・復元は不要になる。M1 の 4,096 回で 18.482 ms から 16.565 ms になった。閉じた出力、一つだけ空いた記述子、読み取り専用、切断済みパイプ、失敗時の親の状態を確かめた。同じ番号への複製・移動、混在出力での printf エスケープ診断の順序も修正した。

子プロセスが記述子の上限に達したとき、musl と glibc では EMFILE の診断文が異なる。回帰テストは両方の表記を認め、終了値が 1 で出力ファイルが空であることを確認する。標準記述子を一つ閉じた場合は書き込みが成功し、期待した内容が残ることも確認する。
