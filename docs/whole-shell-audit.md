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

## Terminal input / 対話入力

The screen oracle records each PTY read before decoding it. It distinguishes the first output from the frame showing the expected text on both sides of the caret, then separately verifies the shell's entire edit buffer. A negative control feeds a plausible marker-only frame with missing text. For a scrolled line the observer requires a contiguous suffix of the expected input and visible context. Each timing distribution has 100 observations. [PTY driver](../tools/ci/whole-editor.py) · [Edit, resize, completion and foreground-input regressions](../tests/review_whole_editor.py)

小さい分割画面の Home、結合文字を含む検索語の削除、プロセス置換内の Tab 補完、候補を絞り込んでいる間のキャンセルを直した。長い入力では先頭のまとまりを画面に早く返す。Enter の後ろの入力はフォアグラウンドの子に残す。PTY の先頭バイト・表示内容とカーソル・編集バッファを個別に確認した。ここで測れるのは IME 確定文字列の受け渡し以降であり、変換候補ウィンドウと GPU 描画の計測は範囲外である。

## Attribution and limits / 内訳と限界

On a physical M1, Time Profiler took 109 samples in an independent baseline run that appended a Japanese character and requested the growing length 90,000 times. One instruction range accounted for 89 of those samples. Its release disassembly already contained ARM vector shifts (`ushr.2d`). With incremental character counts, the same input yielded 20 samples in total. System Trace on 25 two-stage external pipelines followed by a 150 ms child wait showed 20.75 ms running and 222.28 ms blocked in the shell's main thread, excluding the initial instrumentation stop. These traces locate work; the release comparisons use untraced runs. Linux collects only usable perf events on each host.

The review also followed atomic signal notification into the interpreter, private descriptor relocation and close-on-exec, unnamed temporary files and their fallback, distinctions between arithmetic data and parsed source, directory ownership of physical paths, and Linux allocator registration before the first allocation. The ordinary regression suites exercise state restoration, interrupted waits, literal data, read boundaries and file contents. A short fallback expression flagged by Clippy in environment construction already borrows the default via `Cow`. The parser's static error messages also need no heap allocation. Neither finding justified a change.

M1 の CPU プロファイルと逆アセンブルでは、最も時間のかかった走査に既にベクトル命令が出ていた。追記済み文字列の再走査そのものを省いた。プロセスの待機は別の System Trace で測定した。シグナル通知・一時ファイル・非継承 FD・算術展開・アロケータの fork 前提もソースを追った。ハードウェアカウンターはホストで取得できたものを記録した。全入力や全スケジュールを網羅する証明ではない。
