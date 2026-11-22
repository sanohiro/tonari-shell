# Engineering notes / 開発知見

## 2026-11-22 · Measuring startup / 起動時間の測り方

A first prompt, a fresh shell running `-c ':'`, and a shell running an external command answer three different questions. The prompt timer includes allocating a pseudo-terminal and reading the prompt. The current `-c` timer starts at `posix_spawn` and ends at `waitpid`. When we changed the launcher, we reran the previous binary alongside the new one, rotating launch order. Use the direct `/usr/bin/true` run to see how much even a small external executable costs on that machine. [Harness](../tools/bench.py) · [Results and timing boundaries](benchmarks.md)

最初のプロンプト表示、何もしない `-c ':'` の起動から終了まで、外部コマンドの実行完了までは、それぞれ別の時間を測っています。プロンプトの計測には疑似端末の作成と表示の読み取りも含みます。現在の `-c` の計測区間は `posix_spawn` から `waitpid` までです。起動方法を変えたときは、旧バイナリも新しい計測器で交互に起動して比較しました。同じマシンで小さな実行ファイルを起動するコストは、`/usr/bin/true` の直接起動で確認できます。

## 2026-11-22 · Choosing the Linux arm64 binary / Linux arm64 配布版の選定

We chose static musl for Linux arm64 to ship a single executable with fast startup. On Debian 13 arm64, the median for `-c ':'` was 0.170 ms with static musl and 0.303 ms with glibc. For 100 builtin file redirections the order reversed, at 0.664 ms and 0.473 ms respectively. Both builds ran the same source. The compiler distribution, libc, link mode and optimization flags differed, so this run measures the combined build choices. The data is from a warm cache on one Apple M1. [Measurements and build command](benchmarks.md)

Linux arm64 では、実行ファイル1本で配布でき、起動が速かった静的 musl 版を選びました。Debian 13 arm64 では `-c ':'` の中央値が静的版 0.170 ms、glibc 版 0.303 ms でした。一方、組み込みのリダイレクト100回は静的版 0.664 ms、glibc 版 0.473 ms でした。同じソースでもコンパイラの配布元、libc、リンク方式、最適化フラグが異なるため、この数値はビルド条件を合わせた結果です。計測環境は Apple M1 の1台で、キャッシュが温まった状態です。

## 2026-11-22 · Starting pipeline stages / パイプの各段を起動する順序

In `cat < feed > data | printf x > feed`, where `feed` is a FIFO, opening the read end can block until the writer starts. Performing that open in the parent would prevent it from launching the writer. Tonari performs blocking opens in the child and starts all stages before waiting. Within each stage it connects the pipe, then applies explicit redirections left to right. Consequently `cmd 2>&1 >data | cat` sends stderr to `cat`, while `cmd >data 2>&1 | cat` sends both streams to the file. Recheck with `test_blocking_fifo_opens_do_not_serialize_a_pipeline` and `test_file_and_descriptor_order`. [Regression tests](../tests/shell.py)

`feed` が名前付きパイプ（FIFO）の場合、`cat < feed > data | printf x > feed` の読み取り側は、書き手が現れるまで `open` で待ちます。ここで親シェルが待つと、後段の書き手を起動できません。Tonari はブロックし得るファイルを子で開き、全段を起動してから終了を待ちます。各段ではパイプを先に接続し、明示的なリダイレクトを左から適用します。`2>&1 >data` なら標準エラーはパイプに流れ、`>data 2>&1` なら標準出力と標準エラーが両方ともファイルに入ります。

The parser accepts UTF-8 command text. The pipe carries the child processes' bytes unchanged. In `test_pipelines`, `printf '\377\000' | cat` produces the bytes `ff 00`.

UTF-8 専用なのはコマンド入力です。子プロセス間を流れるデータはそのまま渡します。`test_pipelines` では `printf '\377\000' | cat` の出力が2バイトの `ff 00` になることを確認しています。

## 2026-11-22 · Redirections on silent builtins / 無出力の組み込みとリダイレクト

`true >data` can skip redirecting stdout, but opening the file has observable effects. It can truncate a file or wait for a FIFO peer. Allocation order also matters. With standard streams open and a descriptor limit of four, Bash 5 truncates `data` before saving stdout fails. With a limit of five it succeeds. Creating a private backup before opening the path could also make `true < /dev/fd/3` see the shell's own backup. These cases apply to a silent builtin with a single file redirection. Recheck with `test_silent_builtin_at_descriptor_limit`, `test_silent_builtin_file_redirections` and `test_silent_builtin_fifo_open`. [Executor](../src/execute.rs) · [Regression tests](../tests/shell.py)

`true >data` は標準出力の付け替えを省けますが、ファイルを開くと切り詰めが起き、FIFO なら相手を待ちます。記述子を確保する順番も互換性に影響します。標準入出力と標準エラーが開いていて記述子上限が4なら、Bash 5 は `data` を切り詰めてから標準出力の退避に失敗します。上限5なら成功します。また、先に退避すると `true < /dev/fd/3` からシェル内部の退避先が見えてしまう場合があります。この最適化を使えるのは、無出力の組み込みにファイルリダイレクトが一つだけある場合です。

## 2026-11-22 · One executable, two names / 実行ファイル1本と二つの起動名

We staged one regular file at `libexec/tonari/bash` and a relative symlink at `bin/tonari` pointing to `../libexec/tonari/bash`. Both paths ran `-c 'printf Tonari | cat'` on macOS, Debian 13 and Ubuntu 26.04, all on arm64. To repeat the check in a staged installation, run `test bin/tonari -ef libexec/tonari/bash` from the prefix and invoke both paths with that pipeline. Shell detection by third-party tools is a separate compatibility question that needs a test with the tool and version concerned. [Distribution decision](../README.md)

`libexec/tonari/bash` に実体を1本置き、`bin/tonari` から `../libexec/tonari/bash` への相対リンクを作って検証しました。arm64 の macOS、Debian 13、Ubuntu 26.04 で、どちらの名前からも `-c 'printf Tonari | cat'` が動きました。再確認するときは配置先のルートで `test bin/tonari -ef libexec/tonari/bash` を実行し、両方のパスで同じパイプラインを起動します。各ツールが Tonari を Bash と判定するかは、そのツールとバージョンで別途検証します。[配布方針](../README.ja.md)

## 2026-11-22 · Dropping an unused macOS library / macOS の未使用ライブラリを外した実験

On an M1 Mac, we built `2874dc8` twice with Rust 1.96.1, native CPU tuning and `panic=abort`, adding `-C link-arg=-Wl,-dead_strip_dylibs` to the second build. `otool -L` confirmed that this removed the libiconv dependency. Both executables were 354,032 bytes. Across three batches of 100 alternating `-c ':'` runs per binary, the flagged build had a lower median once and a higher median twice. We kept the existing linker flags. This is worth revisiting if the dynamic dependencies change.

M1 Mac 上で `2874dc8` を Rust 1.96.1、CPU ネイティブ最適化、`panic=abort` で2回ビルドし、片方にだけ `-C link-arg=-Wl,-dead_strip_dylibs` を加えました。`otool -L` では libiconv への依存が消えましたが、サイズは両方とも 354,032 バイトでした。`-c ':'` を各100回交互に起動する計測を3組行うと、フラグ付きの中央値が速かったのは1組、遅かったのは2組でした。このためフラグの追加は見送りました。動的ライブラリの依存が変わったときに再検討できます。

## 2026-11-22 · Allocations in builtin redirections / 組み込みリダイレクトのメモリ確保

In a static musl arm64 build of `8c0a050`, constructing a one-element vector of descriptor actions for each of 100 `: >/dev/null` commands caused 100 additional 4 KiB memory mappings and unmaps. Keeping the one action on the stack reduced the process totals from 125/124 to 25/24. In two 250-run alternating Debian 13 batches, the stack version took 0.429 and 0.412 ms median. The otherwise identical heap version took 0.734 and 0.689 ms. Compare with `strace -c -e mmap,munmap,fcntl` on a shell running that workload, and `python3 tools/bench.py … --case redirects --runs 250`. Real descriptor backups retain close-on-exec. The short-lived reservation used by a silent builtin closes before shell execution resumes. [Execution path](../src/execute.rs) · [Benchmark](benchmarks.md)

静的 musl の arm64 版で `: >/dev/null` を100回実行すると、記述子操作一つを入れるベクターのために 4 KiB のメモリマッピングと解除が各100回発生しました。要素一つのときだけスタック上に置くと、プロセス全体の mmap / munmap は 125 / 124 回から 25 / 24 回になりました。Debian 13 上で交互に各250回測った2組の中央値は、スタック版が 0.429、0.412 ms、ほかの条件をそろえたヒープ版が 0.734、0.689 ms です。記述子の実際の退避先には close-on-exec を付けています。

## 2026-11-22 · macOS deployment target / macOS のビルド対象

On the M1 Mac, targeting macOS 13 enabled Mach-O chained fixups and saved 400 bytes on `8c0a050`. Across three 150-launch batches at matched binary path lengths, median `-c ':'` was 2.29–2.37 ms with the existing release flags and 2.41–2.45 ms with macOS 13 and optimization level 3. We kept the release settings. To repeat, build the same revision at both settings with `RUSTFLAGS='-C target-cpu=native'`, setting `MACOSX_DEPLOYMENT_TARGET=13.0` for the second build, inspect `otool -l`, and rotate both binaries with the startup benchmark. [Benchmark harness](../tools/bench.py)

M1 Mac で macOS 13 向けにビルドすると、Mach-O の chained fixups が有効になり、実行ファイルが400バイト小さくなりました。同じ長さのファイルパスで各150回起動する計測を3組行うと、`-c ':'` の中央値は既存設定で 2.29〜2.37 ms、macOS 13・最適化レベル3では 2.41〜2.45 ms でした。この結果を受け、ビルド設定は維持しています。

## 2026-11-22 · Pattern matching on Darwin / Darwin のパターン照合

On macOS 27.2.1, the libc pathname glob and fnmatch interfaces disagreed on POSIX character classes. Tonari walks directory components and calls fnmatch for each, so `[[:alpha:]]ord.rs` selects `word.rs` in both pathname and case patterns. Quoted pieces stay literal when adjacent unquoted text supplies a wildcard. Run `cargo test --locked pattern::tests`. [Matcher](../src/pattern.rs)

macOS 27.2.1 では、libc の glob と fnmatch で POSIX 文字クラスの照合結果に差がありました。Tonari はディレクトリごとに fnmatch で判定し、パス名と case 文のどちらでも `[[:alpha:]]ord.rs` が `word.rs` に一致するようにしました。ワイルドカードと隣り合う引用部分はリテラルとして扱います。

## 2026-11-22 · Choosing Rust’s release optimization level / Rust リリースビルドの最適化

On the M1 macOS integration build, changing `opt-level` from `3` to `s` saved 82,416 executable bytes. Going to `z` saved another 81,136 bytes but increased elapsed time for 1,000 iterations of `for ((i=0;i<1000;i++)); do :; done` by 24%. Empty-shell timings alone missed this cost. The default release profile now uses `s`. Rebuild with `CARGO_PROFILE_RELEASE_OPT_LEVEL` set to `3`, `s` or `z` and rotate the results with `tools/bench.py --case loop --compare-shell`. [Build conditions and measurements](benchmarks.md#selecting-release-optimization-on-macos--macos-の最適化レベル)

M1 macOS の統合ビルドで最適化レベルを3から `s` にすると 82,416 バイト小さくなりました。`z` はさらに 81,136 バイト小さくなりますが、算術ループ1,000回が `s` より24%遅くなりました。空のシェル起動時間だけではこの差が出ないため、継続実行の計測もリリース設定の判断に含めています。

## 2026-11-22 · Sharing stdin with child processes / 子に標準入力を引き継ぐ

A shell reading its script from fd 0 shares the stream with commands in that script. For input containing `cat\nremaining\n`, the child must receive `remaining\n`. On a regular file Tonari seeks back over unread buffered bytes before execution. On a pipe the consumed bytes cannot be restored, so Tonari reads only the current command. A named script has a private reader and can use ordinary buffering. The differential test runs both seekable and nonseekable stdin. [Regression tests](../tests/shell.py)

標準入力がスクリプトのとき、同じストリームを子コマンドも読みます。`cat\nremaining\n` の後半は子の入力です。通常ファイルでは先読みした分を実行前にシークで戻せます。パイプでは戻せないため現在のコマンドまで読み、ファイル名を指定したスクリプトには通常のバッファリングを使います。結合テストでは両方の入力を Bash と照合します。

## 2026-11-22 · Here-documents and command substitution / ヒアドキュメントとコマンド置換

A synchronous writer can fill a pipe before its reader starts. Here-documents at most `PIPE_BUF` bytes fit in an empty pipe on the supported systems. Larger documents use an unlinked temporary file that the consumer receives read-only. Command substitution has the complementary dependency. The parent closes its copy of the writer and drains stdout before waiting for the child. Tests cover a 120,000-byte literal here-document and more than 250 KiB of captured output. [Regression tests](../tests/shell.py)

書き手が先にパイプを満たすと、読み手の起動まで進めません。空のパイプに収まる `PIPE_BUF` 以下のヒアドキュメントはそのまま渡し、大きいものは名前を削除した一時ファイルを読取専用で渡します。コマンド置換では親が自分の書き込み口を閉じ、出力を読み切ってから子を待ちます。120,000 バイトのヒアドキュメントと 250 KiB を超えるコマンド置換で検証しています。

## 2026-11-22 · Foreground pipelines and interactive history / フォアグラウンドパイプと対話履歴

An interactive pipeline needs one process group containing all stages. Both shell and child call `setpgid` to cover either scheduler order. When a foreground job stops, the shell saves the job’s terminal settings and takes the terminal back. A pseudo-terminal test suspends a real pipeline, resumes it in the background, foregrounds it, sends Ctrl-C, and checks status 130. A separate two-terminal test checks that a pasted command waits for Enter, a second session can find the first session’s entry, and an excluded command never reaches the journal. [Job control](../src/jobs.rs) · [Regression tests](../tests/shell.py)

対話パイプの全段には共通のプロセスグループが必要です。どちらが先にスケジュールされてもよいよう、親と子の両方で `setpgid` を呼びます。ジョブの停止時には端末設定を保存して制御をシェルに戻します。疑似端末でパイプを停止し、バックグラウンドで再開、再び前面に移して Ctrl-C で終了値 130 になることを確認します。別のテストでは端末を二つ開き、貼り付けの確定、履歴の共有、除外した入力が保存されないことを確認します。

## 2026-11-22 · Repeated PATH lookup / PATH 検索の繰り返し

Bash-compatible command hashing can save time when a persistent shell repeatedly starts an executable near the end of PATH. On macOS arm64, 40 calls with 16 empty directories before the executable took 94.143 ms with hashing disabled and 49.873 ms with hashing enabled. With one directory they took 49.503 and 50.392 ms. Assigning PATH clears the table, including assigning its existing value. A temporary PATH on a command must take precedence over a cached command. Verify the selected path with `hash -t name` and clear the table with `hash -r`. [Benchmark conditions](benchmarks.md#repeated-path-lookup--path-検索の繰り返し)

対話シェルなど同じプロセスから PATH の後方にある実行ファイルを繰り返し起動する場合、Bash 互換のコマンド検索キャッシュが効きます。macOS arm64 では、空のディレクトリ16個を先に並べたとき、40回の呼び出しが94.143 ms から49.873 ms に短縮されました。ディレクトリが一つの場合は49.503 ms と50.392 ms でした。同じ値でも PATH を代入するとキャッシュを破棄します。コマンドの前に一時的な PATH を指定した場合は、その PATH で検索します。`hash -t name` で記録されたパスを確認でき、`hash -r` で消去できます。[計測条件](benchmarks.md#repeated-path-lookup--path-検索の繰り返し)

## 2026-11-22 · Allocation size when reading nvm / nvm 読み込み時のメモリ確保

In the static musl release, sourcing nvm 0.40.5 with `--no-use` triggered 504 `mmap` and 500 `munmap` calls. An instrumented scratch build traced allocation requests alongside system calls. Initial four-element vector allocations frequently requested 128 bytes for command words, 160 for parts of a word, and 416 for statements. Reserving two slots for words and parts and one for a statement list reduced the counts to 309 and 305. The vectors still grow for larger inputs. Compare `strace -c -e mmap,munmap tonari --norc --noprofile -c '. "$1" --no-use' probe /path/to/nvm.sh` with an empty `NVM_DIR` and `python3 tools/bench.py … --case nvm --nvm-script …`. [Elapsed time and sizes](benchmarks.md#parser-allocation-experiment--パーサーのメモリ確保)

静的 musl 版で nvm 0.40.5 を `--no-use` で読み込むと、`mmap` が504回、`munmap` が500回発生しました。計測用ビルドでアロケータとシステムコールを突き合わせると、ベクターの初回確保でコマンドの単語に128バイト、単語の展開部分に160バイト、文リストに416バイトを要求していました。初回を単語・展開部分は2要素、文リストは1要素にすると、`mmap` / `munmap` は 309 / 305 回になりました。長い入力では必要に応じてベクターが拡張されます。

## 2026-11-22 · Threads and interactive latency / スレッドと入力の応答時間

Threads are an option for independent work such as reading a large history journal. In `76cc5f1` the first printable key triggered a synchronous journal refresh and prefix search. On the M1, 100,000 entries put the median first redraw above 20 ms. An experiment with background loading should also measure Enter and Tab, since either can run shell code. Command and process substitutions currently fork and continue interpreting Rust code in the child. POSIX restricts the operations available to that child if other threads are live at the fork. A worker scoped to editing could finish before any such fork, including one triggered by a completion function. [First-key measurements](benchmarks.md#first-key-history) · [Command substitutions](../src/subshell.rs) · [POSIX fork](https://man7.org/linux/man-pages/man3/fork.3p.html)

大きな履歴の読み込みなど、独立した仕事にはスレッドを使える。`76cc5f1` では最初の文字を受け取ってから、履歴を同期的に読み込み、入力に合う候補を探していた。M1 では10万件で再描画までの中央値が20 msを超えた。バックグラウンド化を試す際は、シェルコードが動き得る Enter や Tab で待たされる時間も測る。コマンド置換やプロセス置換では、fork した子が Rust の実行系を引き継ぐ。ほかのスレッドが動いている状態で fork すると子で安全に呼べる処理が制限されるため、編集に限定したワーカーなら、補完関数を含めてそうした fork の前に終了させる設計が考えられる。

The editor now draws the typed character first and checks for incoming keys between buffered journal reads. A pending key pauses the read at a complete record boundary. Ctrl-R can request the full journal immediately. Reading a fixed snapshot of the append-only file and publishing only newline-terminated records lets a user type while another session is appending. The PTY regression covers a writer holding the exclusive lock midway through a row. The updated M1 release reduced the median initial redraw with 100,000 rows to 0.036 ms. The corresponding Tab and Enter trials include the time for completion and command execution. [Measurements](benchmarks.md#first-key-history) · [Regression tests](../tests/shell.py)

エディターは入力した文字を先に描画し、履歴をバッファ単位で読む合間にキー入力を確認する。次のキーが来たら読み込みを中断し、直前に確定したレコードの境界から再開する。Ctrl-R はその場で全履歴を読み込める。追記型ファイルを開始時点の長さまで読み、改行で完結した行だけを採用するので、別端末が書き込みロックを持っている最中も入力できる。疑似端末テストでは実際に別の書き手を行の途中で止めて検証した。10万件の初回再描画は M1 で中央値 0.036 ms となり、Tab の補完と Enter による実行完了までの時間も確認した。

## 2026-11-22 · First Tab and typing during history lookup / 最初の Tab と履歴検索中の入力

The first Tab needs the longest common prefix and whether the match is unique. Tonari reads native filename and command candidates until both answers are settled. It can skip filesystem metadata for names that would leave those answers unchanged. The next Tab reads the directory again and sorts the list for display, so additions between keypresses are visible. Simple Bash registrations such as `complete -f` use the same lookup. Shell functions and external completion commands run when invoked and can still delay the following key. [PTY regression](../tests/shell.py) · [Timing and reproduction](benchmarks.md#completion-latency)

最初の Tab で必要なのは、全候補に共通する先頭の文字列と、候補が一つかどうかの二つ。ファイル名とコマンド名の補完は、その答えが確定したところで探索を終える。結果を変え得ない候補への属性照会も省く。次の Tab で一覧を表示するときはディレクトリを読み直すので、キー入力の間に作成したファイルも一覧に現れる。`complete -f` も同じ実装を使う。シェル関数や外部コマンドを呼び出す補完では、その実行中は次の打鍵が待つことがある。

The editor draws new text before scanning shared history for an older suggestion. It polls for input between blocks of 4,096 entries and reuses the previous match or miss as the prefix grows. A journal update invalidates that result. To test perceived latency, time the typed character and the eventual suggestion separately. For Tab, queue a printable character immediately after it, time its arrival at the PTY, and verify the final line includes the completion followed by that character. [Benchmark driver](../tools/bench_completion.py)

共有履歴をさかのぼるときは、まず入力文字を表示し、4,096 件ごとに次の打鍵が来ているか確認する。入力が前回の続きなら、一致した位置や候補がなかった結果を再利用する。履歴が追記されたときは改めて調べる。応答時間の計測では、文字が見える時点と候補が見える時点をそれぞれ記録する。Tab には直後にもう一文字を送り、疑似端末にその文字が返るまでを測り、補完後の位置に入ったことも確認する。

The second Tab currently prints a static candidate list beneath the command line. The user can extend the prefix and press Tab again. Live filtering and selecting a row from the list remain open parts of the completion UI. Full-list benchmarks need to drain terminal output and verify each candidate, since a first-Tab benchmark exercises only prefix insertion. Readline's confirmation and pager settings affect what Bash emits. [Second-Tab benchmark](benchmarks.md#completion-listing)

現在の2回目の Tab は、入力行の下に候補を一覧表示する。文字を足してもう一度 Tab を押せば、新しい接頭辞で補完できる。一覧を開いたまま絞り込み、候補を選択する UI は未実装。全件表示の計測では端末出力を読み切り、全候補が届いたかを確認する。Bash では、件数確認とページ送りの設定によって出力範囲が変わる。

## 2026-11-22 · Parameter trimming inside scripts / スクリプト内の接頭辞・接尾辞の除去

A file benchmark that included shell startup made a path-processing loop on the M1 look almost tied with Bash. Timing the same file after both interpreters were ready exposed a slower operation inside Tonari. Parameter trimming tried libc fnmatch at successive UTF-8 boundaries even for a fixed suffix such as `.log` or a delimiter such as `*/`. A fixed literal can be tested at the appropriate end once. With a single star at the outer edge, matching the literal at each candidate boundary preserves the order for the shortest and longest matches. Patterns with quoting, brackets, or other wildcards continue through the general matcher. The unit test checks all four removal operators against that matcher across repeated, overlapping and multibyte delimiters. Run `cargo test --locked trim_boundaries`. [Started-shell and whole-script results](benchmarks.md#script-execution)

M1 のファイル名加工ループは、シェル起動込みだと Bash とほぼ同じだった。起動後に同じファイルを読ませると、Tonari は固定の拡張子 `.log` や `*/` に対しても、文字境界ごとに libc の fnmatch を呼んでいた。固定文字列は端で一度だけ照合する。外側に星が一つある場合は、最短・最長に応じて文字境界を順にたどり、区切り文字列を直接比較する。テストでは繰り返し・重なり・日本語を含む値について、4種類の除去演算子の境界を従来の照合結果と比較した。

## 2026-11-22 · Command substitution and sequential launch / コマンド置換と連続起動

For a single builtin `printf` with literal or ordinary variable arguments, Tonari sends the formatter's bytes to a buffer in a private shell session. This preserves the scope of `%n` and `-v`. Function overrides, explicit redirections and process-dependent arguments such as `$BASHPID` take the ordinary subshell path. On macOS a simple external command can write directly into the capture pipe. The parent drains that pipe before waiting and still collects the child's actual exit status. The shell tests cover inherited stdin, separate stderr, output beyond the pipe capacity, trailing newlines and exit status 23. [Substitution tests](../tests/shell.py) · [Measurements](benchmarks.md#capture-and-spawn)

単独の組み込み `printf` で引数がリテラルや通常の変数なら、独立したシェル状態で書式処理し、結果をメモリに受け取れる。`%n` や `-v` の代入は親に漏れない。同名の関数、明示的なリダイレクト、`$BASHPID` などを含む式は従来のサブシェルで評価する。macOS の単純な外部コマンド置換では、実行先の標準出力を直接パイプにつなぐ。親は読み切ってから子の終了を待つ。入力の継承、標準エラー、パイプ容量を超える出力、末尾の改行、終了値 23 をテストした。

On Linux a resolved, unredirected foreground executable can use `posix_spawn` with its existing argument and environment vectors. Consecutive commands can reuse the encoded environment when the shell's exported changes match. Command-prefix assignments receive their own environment. A plain external command already used the OS spawn path before this change. The native C control performs exactly 16 launches and 16 waits on the same executable and checks all child statuses. On Linux, tracing the final binary showed 16 execs of `/usr/bin/true` for that loop and zero child launches for the builtin substitution loop. The exported-environment case checks the output of 16 real `printenv` processes. [Native control](../tools/bench_spawn.c) · [Script driver](../tools/bench_scripts.py)

Linux では、実行パスが確定したリダイレクトなしのフォアグラウンドコマンドに、用意した引数と環境を直接渡せる。export された変更が同じなら環境の再構築を省ける。コマンドの直前だけに付けた代入は個別に扱う。素の外部コマンドは旧版でも OS の高速起動経路を使っていた。C の対照では同じプログラムを起動するたびに終了を確認し、これを16回繰り返す。最終バイナリの Linux トレースでは外部コマンドのループで `/usr/bin/true` の exec が16回あり、組み込みコマンド置換のループでは子の起動は0回だった。環境変数を含む項目では、16個の `printenv` が実際に値を受け取ったことを出力で確認する。

## 2026-11-22 · UTF-8 paths and Bash patterns / UTF-8 のパスと Bash のパターン

In a UTF-8 locale Bash counts `😀`, `👍🏽`, `🇯🇵` and `👩‍💻` as 1, 2, 2 and 3 characters respectively. Length, slices, pathname `?`, parameter substitution and regex captures now have differential tests using those strings. Bash 5.2.37 and 5.3.9 return status 2 on an invalid regex and retain the preceding captures. Ubuntu's Bash 5.3.9 additionally prints a diagnostic. Tonari follows 5.2.37 for that diagnostic. Tonari retains the last successfully compiled POSIX extended regular expression per session. Pathname expansion can use the matching directory entry itself as evidence of existence. A literal suffix or trailing slash still requires a filesystem check. [Regression tests](../tests/shell.py) · [Timing results](benchmarks.md#capture-and-spawn)

UTF-8 ロケールの Bash では `😀`・`👍🏽`・`🇯🇵`・`👩‍💻` の文字数は順に 1・2・2・3 になる。長さ、部分文字列、ファイル名の `?`、パターン置換、正規表現のキャプチャを同じ入力で比較した。無効な正規表現では Bash 5.2.37 と 5.3.9 の両方が以前のキャプチャを保持して終了値 2 を返し、5.3.9 ではエラーも表示する。Tonari の診断は 5.2.37 に合わせた。同じ正規表現の反復ではコンパイル済みのものを一つ再利用する。グロブの最後の要素がディレクトリ列挙で見つかった場合は存在を再照会しない。後続の固定パスや末尾のスラッシュには引き続きファイルシステムの判定を使う。

## 2026-11-22 · Applying Align's launch and string experiments / Align の起動・文字列処理の実験を応用

Align commit `b99015ec` measured Linux launches while varying the parent's resident memory. Its native-process contract requires a private child stack and staging for features such as descriptor-backed executable images. Tonari can use the OS spawn interface for ordinary external commands. In a Linux arm64 trace, a plain two-external-command pipe already used two shared-VM launches. With `|&` the old build used one ordinary fork and one shared-VM launch. The updated build used two shared-VM launches. A simple external command substitution also switched from one ordinary fork to one shared-VM launch. Inspect `../align/docs/impl/49-native-process-contract.md` and `../align/crates/align_runtime/src/process_launch/darwin.rs` at `0984346c`. [Timing and reproduction](benchmarks.md#align-experiments)

Align の `b99015ec` は親の常駐メモリ量を変えて Linux の子プロセス起動を測った。Tonari の通常の外部起動には OS の spawn を使える。Linux arm64 でトレースすると、外部コマンド同士の通常の `|` は旧版から両段とも共有 VM を使っていた。`|&` では旧版の左段だけが通常の fork だったので、その段も spawn にした。単純な外部コマンド置換も同じ方法で起動する。パイプ先で起動エラーを受け取れるように、spawn が成功するまで元のパイプ端を保持する。ジョブ制御や FIFO のオープンを伴う経路は個別に扱う。

Testing with one 16 MiB unexported variable exposed a second cost. Builtin substitutions copied unrelated shell variables even when `printf` only formatted output. The formatter can borrow those variables and make a private copy on the first assignment through `%n` or `-v`. A compound substitution already owns its session after `fork`. It can write to that session directly. The regression suite checks variable and directory isolation, stderr routing, missing executables, pipefail, and the exit status of substitutions. [Executor and shell tests](../tests/shell.py)

16 MiB の未エクスポート変数を持たせると、出力するだけの `$(printf …)` でも無関係な変数をすべてコピーしていた。読み出しだけなら共有し、`%n` や `-v` が書き込むときに初めて変数を分離できる。複合コマンド置換は fork の時点でプロセスが分かれているため、子の中でセッションを複製する手間も省ける。テストでは変数・作業ディレクトリの分離、標準エラーの行き先、起動失敗、pipefail、置換の終了値を確認する。

Align's delimiter-search and field-splitting work (`f5633ac6`, `36008542` and `63b521f1`) separates finding delimiter positions from materializing the fields. Tonari's word expansion and `read` now visit the separator spans in the original string. For one Unicode scalar they use Rust's character search. For a trim such as `${path##*/}`, one search for the literal delimiter gives the cut position. Shell-specific quoting, IFS whitespace and non-whitespace separators, and the final variable of `read` keep their own rules. [Field expansion](../src/word.rs) · [Pattern trimming](../src/pattern.rs)

Align の区切り検索とフィールド分割を参考に、Tonari でも非引用の変数展開と `read` で元の文字列の区切り位置を走査するようにした。区切りが1文字なら Unicode 文字の検索を使う。`${path##*/}` のような展開は固定の区切りを直接検索できる。引用、IFS 空白と通常の区切りの違い、`read` の最後の変数に残りを渡す処理は、シェルの規則に従う。

One UTF-8 difference is reproducible with `IFS=' 木'; v='a 木b'; set -- $v`. In a UTF-8 locale, macOS GNU Bash 5.2.37 produced two extra empty fields, while Debian Bash 5.2.37 and Ubuntu Bash 5.3.9 left two UTF-8 continuation bytes at the start of the second field. Tonari consumes the whole separator and produces `a` and `b`. The regression test records that UTF-8 behavior explicitly.

`IFS=' 木'; v='a 木b'; set -- $v` では参照先の Bash 同士にも差がある。UTF-8 ロケールの macOS 版 Bash 5.2.37 は空の引数を二つ余分に返し、Debian 5.2.37 と Ubuntu 5.3.9 は第2引数の先頭に UTF-8 の継続バイトを二つ残した。Tonari は区切りを一文字として消費し、引数 `a` と `b` を返す。この UTF-8 の扱いを回帰テストに記録した。

## 2026-11-22 · Borrowing completion words and reusing case patterns / 補完候補の読み出しと case パターンの再利用

Align's `36008542` finds literal split fields in one traversal, and `076459f6` borrows text after classifying it. Tonari can apply both ideas to `complete -W`. A registered literal list keeps its original string. Tab reads slices using the current `IFS`. For the first Tab, the editor needs only the common prefix and whether the match is unique. It can stop once two distinct matches share exactly the typed prefix. A second Tab collects the matching words for display. A duplicate alone cannot establish ambiguity. `compgen -W` continues to emit duplicates in source order. Quoted words, substitutions and variable references go through shell expansion when completion runs. [Completion implementation](../src/completion_spec.rs) · [PTY measurements](benchmarks.md#completion-and-case)

Align の `36008542` は文字列を1回走査して分割し、`076459f6` は中身を判定してから元の文字列を参照する。Tonari の `complete -W` でも、固定リストは元の文字列を保持し、Tab を押したときの `IFS` で区切る。最初の Tab で必要なのは共通部分と候補が一意かどうかだけ。異なる2候補の共通部分が入力済みの文字列まで縮めば、残りを調べずに答えを返せる。同じ候補が2回出ても曖昧とは扱わない。2回目の Tab で初めて一覧を作る。`compgen -W` の出力順と重複は保持する。変数やコマンド置換、引用のあるリストは補完する時点で展開する。

Align's constant prefix/suffix experiments (`4d556508`, `../align/bench/string_predicates`) compare fixed predicates with a dynamic control. In Tonari, a shell pattern with a single unescaped `*` and two literal edges can use the same idea. Matching the suffix against the remainder after the prefix prevents `ab*ba` from matching `aba`. Directory traversal prepares the matcher once per component and checks the leading-dot rule separately. A parsed `case` shares its arms across loop iterations and function calls. It prepares static patterns once, including the native pattern string for character classes. Variable and command substitutions still run in order, only when execution reaches their arm. Regression tests compare overlapping edges, emoji, quoted wildcards and skipped substitutions with Bash, and compare the matcher against libc with and without `FNM_PERIOD`. [Shell tests](../tests/shell.py) · [Matcher tests](../src/pattern.rs)

Align の固定接頭辞・接尾辞の実験を参考に、前後が固定文字列で `*` が1個のパターンを直接判定する。先頭を照合した残りに対して末尾を調べるので、`ab*ba` は `aba` に一致しない。ファイル名展開ではディレクトリの各要素で判定方法を1回だけ選び、先頭がドットの名前には `dotglob` の設定を適用する。`case` はループや関数の呼び出しをまたいで分岐を共有し、文字クラスを含む固定パターンも一度だけ準備する。変数やコマンド置換は、それを含む分岐に到達した時点で順に展開する。引用したワイルドカードや絵文字、実行されない置換は Bash と比較し、先頭ドットの扱いは libc の結果とも照合した。

When the inherited collation is explicitly `C` or `POSIX`, pathname expansion can sort its existing UTF-8 strings in byte order. Converting every path into a C string and back added work on each expansion. Other locales retain libc collation. On macOS, `雪` sorted after `z` in `C` and before `A1` in `en_US.UTF-8` in both shells. The differential test varies `LC_ALL`, `LC_COLLATE` and `LANG` to exercise their precedence. [Regression tests](../tests/shell.py)

継承した照合順序が明示的に `C` または `POSIX` なら、ファイル名は UTF-8 の文字列のままバイト順でソートできる。各パスを C 文字列に変換して戻す処理を省いた。ほかのロケールでは libc の照合順序に従う。Mac で `雪` は両シェルとも `C` では `z` の後、`en_US.UTF-8` では `A1` の前に並んだ。回帰テストでは `LC_ALL`・`LC_COLLATE`・`LANG` の優先順位も検証している。

GNU ld 2.45.1 and LLD 22.1.3 produced 1,053,040-byte and 1,006,088-byte Linux arm64 executables from the same source with packed relative relocations. The build recipe now uses LLD. `readelf -Wl` showed 64 KiB load alignment, GNU RELRO, and no program interpreter. `readelf -d` showed RELR entries, PIE and no shared-library dependencies. Runtime tests used a 4 KiB-page Linux VM. [Build recipe](benchmarks.md#linux-arm64-static-pie)

同じソースを相対再配置圧縮付きでリンクすると、Linux arm64 版は GNU ld 2.45.1 で 1,053,040 バイト、LLD 22.1.3 で 1,006,088 バイトだった。配布用ビルド手順に LLD を採用した。ELF のロードセグメントは 64 KiB 整列で、GNU RELRO と PIE を保持し、外部の動的ローダーや共有ライブラリを要求しない。実行テストにはページサイズ 4 KiB の Linux VM を使った。

## 2026-11-22 · Timing external pipelines and case separately / 外部パイプラインと case の時間を切り分ける

With two external commands and job control off, Tonari lowers `|&` to the final `2>&1` redirection on the left stage. Both output streams use the same pipe writer. In an independent control a Python parent launched the system `printf` and `cat` directly with the same descriptors. Most of the Mac/Debian timing gap persisted, particularly in the interval after submitting both launches until both children had finished. A second test with a minimal native executable timestamps entry to `main`. The shell can influence how it launches a program, but these experiments do not attribute the gap to a particular part of the kernel or dynamic loader. [Executor](../src/execute.rs) · [Timings and conditions](benchmarks.md#pipe-process-timing)

外部コマンド2個、ジョブ制御なしの条件では、Tonari は `|&` を左段の最後の `2>&1` として扱い、標準出力と標準エラーを同じパイプにつなぐ。Python 親プロセスから OS の `printf` と `cat` を直接起動しても Mac と Debian の時間差は大きく、特に2個の起動要求から戻った後、両方の終了を確認するまでに差が出た。別の小さなネイティブ実行ファイルでは、子が `main` に入る時刻も切り分けた。カーネルと動的ローダーの内訳を特定するには、この計測とは別の観測が必要になる。

Debian's `case` benchmarks run entirely inside the interpreter. The fixed prefix/suffix path compares UTF-8 bytes; a bracket pattern uses libc `fnmatch` and builds a C string for the input. Comparing `src/u*.rs` and `src/[u]*.rs` keeps the input, matched set, branch position and body identical. Filename expansion additionally visits directory entries, materializes matching paths, sorts them and, in the `set --` fixture, replaces the positional parameters. Selectivity changed the ranking against Bash in the 512-file fixture. [Matcher](../src/pattern.rs) · [Matched controls](benchmarks.md#pipe-process-timing)

Debian の `case` 計測はシェル内で完結する。接頭辞と接尾辞が固定なら UTF-8 のバイト列を比較し、角括弧を含むパターンでは入力を C 文字列にして libc の `fnmatch` を呼ぶ。`src/u*.rs` と `src/[u]*.rs` を比較すると、入力・一致集合・分岐位置・実行内容を揃えられる。ファイル名展開にはさらにディレクトリ走査、一致したパスの生成とソートがあり、今回の `set --` では位置引数も入れ替える。512ファイルのうち全件を返す条件と1件を返す条件で Bash との順位が入れ替わった。

## 2026-11-22 · Launch work inside external Mac commands / Mac の外部コマンド内で生じる起動コスト

The earlier timestamp at C `main` covered both kernel and user-space startup. Direct traces of the two external tools now identify part of that time on the user-space side. For each utility, the median libSystem initializer time was about half of the median dyld launch interval. Apple's libSystem source initializes pthread, libc, malloc and libdispatch there. An independent Instruments trace of a minimal `true` clone in Apple Developer Forums reported 454 µs of libSystem initialization within an 893 µs executable launch. Apple's WWDC22 session explains the dependency loading, fixups, initializers, reuse of cached launch work and page-in linking. The measurements do not apportion the remaining Mac/Linux difference. [Our traces](benchmarks.md#macos-child-loader) · [Apple libSystem initialization](https://github.com/apple-oss-distributions/Libsystem/blob/main/init.c) · [Independent CLI trace](https://developer.apple.com/forums/thread/813195) · [WWDC22 session](https://developer.apple.com/videos/play/wwdc2022/110362/?time=916)

前回は C の `main` の先頭で時刻を取った。今回は OS 付属の `cat` と `printf` を直接トレースし、両方で dyld の起動区間の中央値のおよそ半分を libSystem の初期化が占めることを確認した。Apple が公開する初期化コードにはスレッド、C ライブラリ、メモリアロケータ、libdispatch の準備がある。Apple Developer Forums で公開された `true` 相当の CLI の測定でも、起動区間 893 µs のうち libSystem 初期化が 454 µs だった。WWDC22 の解説には依存ライブラリのマッピング、アドレス修正、初期化と、キャッシュやページ読み込み時の最適化が載っている。この固定費は各外部コマンドのプロセス内で生じる。

## 2026-11-22 · Matching directory entries before copying / ファイル名をコピーする前に照合

Literal punctuation in the first segment of a shell word cannot be inside an active bracket expression. Keeping its hyphen unescaped lets `item-000*.txt` use the prefix/suffix matcher. Ordinary pathname globs now borrow the directory entry until the next `readdir` call. They validate UTF-8 and allocate the output path for matches only. Character classes can pass the original NUL-terminated name to `fnmatch`. The `.` and `..` entries, hidden-file rules and invalid UTF-8 have regression coverage. On musl, the emitted backslash inside a simple bracket list such as `[a"-"z]` could also turn a quoted hyphen into a range. Moving that literal to the leading position gives the Bash result for such lists. [Implementation](../src/pattern.rs) · [Bash comparisons](../tests/shell.py) · [Timings](benchmarks.md#borrowed-glob-names)

単語の先頭の固定部分では文字クラスはまだ開いていない。ハイフンをそのまま保持すると、`item-000*.txt` は前方・後方の文字列比較で処理できる。通常のファイル名展開はディレクトリ内の名前をその場で照合し、一致したものだけ UTF-8 を確認してパスを作る。文字クラスの照合でも OS が返した NUL 終端名を直接使える。ドットで始まる名前、特殊なディレクトリエントリ、UTF-8 で表せない名前も回帰テストに含めた。Linux 配布版の musl では、単純な角括弧の列挙中に引用したハイフンを先頭に移すことで、範囲記号との誤認も防いだ。

## 2026-11-22 · Spawning external commands with ordered descriptor copies / 記述子の複製を伴う外部起動

An external program with a trailing `2>&1` used to fork the shell and set up its streams before exec, even in a noninteractive script with a known executable path. Tonari can pass those descriptor copies to `posix_spawn`. It connects the pipe ends first, then applies the command's copies from left to right. Only copies between distinct standard descriptors qualify. The shell can safely retry these in an ordinary child if spawn fails, so missing-command and permission diagnostics go to the requested stream. The ordinary child also handles file opens that can truncate a file or block on a FIFO, and foreground jobs that need terminal control. The regression test covers both redirect orders, the receiving end of a pipe, a closed source fd and failed launches. [Implementation](../src/execute/spawn.rs) · [Test](../tests/shell.py) · [Apple spawn manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/posix_spawn.2.html)

非対話スクリプトで外部コマンドの出力をまとめるとき、実行パスが確定していても、従来は Tonari 自身を fork してから `2>&1` を設定していた。標準記述子同士の複製は OS に起動と一緒に渡せる。パイプを接続してから指定順に複製し、起動に失敗したら通常の子プロセスで同じ接続を作って診断を出す。ファイルを切り詰めるリダイレクトや FIFO のオープン、端末制御を伴うジョブは従来の経路で処理する。この変更で Mac の単体コマンド・パイプ受信側の両方が短縮した。外部2本をつなぐ通常の `|&` は以前から spawn 経路を使い、その先には実行ファイルごとの dyld とライブラリ初期化の時間がある。 [計測条件と結果](benchmarks.md#external-descriptor-copies) · [Instruments の記録](benchmarks.md#macos-child-loader)

One smaller prototype used Rust's `Command::spawn` for standalone `2>&1`. On this Mac, an executable text file containing `printf plain; exit 17` with no shebang then ran successfully, while the previous Tonari route returned 126. The native spawn route preserves that error path. GNU Bash ran the text and returned 17. Interpreting executable text without a shebang in Tonari remains a compatibility gap on this route.

Rust の `Command::spawn` をそのまま使う小さい試作では、先頭にシバンがない実行可能なテキストまで起動方法が変わった。この Mac では内容が `printf plain; exit 17` のファイルを `./script 2>&1; exit $?` で試すと、試作と GNU Bash は17、旧版と採用した実装は126を返した。この経路でシバンのない実行可能テキストを Tonari 自身で解釈する部分には、Bash との互換性の差が残る。
