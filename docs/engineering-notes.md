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
