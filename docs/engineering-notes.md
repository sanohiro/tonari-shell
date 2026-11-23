# Engineering notes / 開発知見

<a id="runtime-dependencies"></a>

## 2026-11-23 · Runtime dependencies and filename order / 実行時依存とファイル名の順序

Tonari ships one executable per target. Its two direct Cargo dependencies are `unicode-width` for terminal cell widths and `libc` for native API declarations. The macOS release now uses `-dead_strip_dylibs`. This removes the unused `libiconv` load command and leaves only `libSystem`. The earlier speed experiment found little difference. We adopted the flag to keep the runtime dependency list small. Inspect Rust dependencies with `cargo tree --locked --edges normal` and direct shared libraries with `otool -L` on Mach-O or `readelf -d` on ELF. [Release measurements](benchmarks.md#macos-runtime-dependencies)

UTF-8 byte order is an acceptable order for pathname expansion. On Debian 13 and Ubuntu 26.04 arm64, Bash and both Tonari builds expanded the same eight `*.txt` names. With `LC_ALL=C.UTF-8` all three put `B.txt` first. With `LC_ALL=ja_JP.UTF-8` Bash and the glibc build put `😺.txt` first and the static musl build put `B.txt` first. This affects arguments by position and loops whose side effects depend on iteration order. Of the named checks in the November 23 Linux inventory, only the `LC_COLLATE` result differed between these builds. [glibc observations](compatibility-2026-11-23-response/matrix.md) · [musl observations](compatibility-2026-11-23-response/musl/matrix.md)

ファイル名展開の順序は UTF-8 のバイト順を許容します。Debian・Ubuntu の8ファイルの比較では、どの版でも同じ名前が展開されました。`ja_JP.UTF-8` で `set -- *.txt` を実行すると、Bash と glibc 版の `$1` は `😺.txt`、静的 musl 版は `B.txt` です。`C.UTF-8` では3者の全件の順序が一致しました。この照合順序だけを理由に、新たな共有ライブラリを追加する必要はありません。macOS 版では未使用の `libiconv` への直接リンクを外し、配布バイナリが直接参照する共有ライブラリは `libSystem` だけになりました。[配布設定](../tools/build-release.sh) · [サイズと起動時間](benchmarks.md#macos-runtime-dependencies)

## 2026-11-23 · Input latency, directory order and byte streams / 入力応答・照合順序・ストリーム

- A fresh session can answer a recent history query from the last 16 KiB of complete journal rows. For an older literal match, scan complete blocks backwards and check new input between blocks. An oversized row or an escaped query still has a decoded streaming path. A key queued behind Ctrl-R can cancel the search before the first full scan. The million-row PTY fixture measures a recent and an old match, cancellation, a 512-byte burst and a 16 KiB paste. [PTY benchmark](../tools/bench_history.py)
- 新しいセッションでは、履歴の末尾16 KiBの完全な行から最近の候補を出せます。古い候補は前方へブロックをたどり、各ブロックの間で入力を確認します。長い1行やエスケープが必要な検索には、行を復号する経路を使います。100万件の試験では最古側の一致、検索の取り消し、連続打鍵、貼り付けを個別に測ります。[履歴と補完の計測](benchmarks.md#interactive-response)
- Bracketed paste and ordinary typeahead share a terminal with later commands. A read during paste ends at the earliest possible closing marker. At the end of an ordinary input line, a burst stops on the first binding, control or multibyte character. The PTY test sends a pasted assignment, a shell command and the following reader’s payload in one input sequence. [Interactive regressions](../tests/shell.py)
- 貼り付け終端を越えて先読みすると、直後のコマンドが読むべき入力を奪います。貼り付け中に一度に読む長さを、終了マーカーが完成し得る最短位置に合わせました。通常の連続入力も、割り当て済みのキー・制御文字・マルチバイト文字で区切ります。[入力境界の回帰試験](../tests/shell.py)
- A filename or PATH completion examines directory-entry bytes before allocating a name. Executability and symlink targets are checked at use. The Darwin SDK exposes `readdir` for the 64-bit inode mode we build against. On the M1 APFS fixture a name-and-type scan with `getattrlistbulk` took 7.65 ms versus 3.82 ms with `readdir` for 10,000 names. On the Debian VM, 2,929 out of 2,999 consecutive creates left the same directory mtime, ctime and size as the preceding create. These timestamps cannot validate a persistent name list in this workload. The immutable large `complete -W` list is indexed on reuse. [Apple filesystem performance guide](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/FileSystem/Articles/IteratingFiles.html) · [Completion benchmark](../tools/bench_completion.py) · [Apple’s dirent header](https://github.com/apple-oss-distributions/Libc/blob/main/include/dirent.h)
- ファイル補完はディレクトリエントリのバイト列を直接比較し、実行権限とリンク先はその場で確認します。APFS の1万件では名前と種類を一括取得する方が遅く、Debian では新規ファイルを作ってもディレクトリの時刻・サイズが続けて同じになる場合がありました。固定の `complete -W` は再利用時に索引を作れます。[比較条件](benchmarks.md#interactive-response)
- A worker sorting UTF-8 names needs the caller’s thread locale. Counting 4,096 matches missed a wrong order that a byte-for-byte comparison against Bash exposed. The corrected macOS path installs the active locale in the worker, joins it before returning to the interpreter and falls back to serial sorting when appropriate. A cached permutation is checked against the current enumeration and invalidated on a locale change. The test checks every name twice with a mutation between expansions. [Regression suite](../tests/shell.py)
- 並列ソートでは補助スレッドにも呼び出し元のロケールを設定します。件数だけを確認するベンチでは順序の誤りを見逃したため、4,241ファイルの出力全体を Bash と照合し、ファイル追加後とロケール切り替え後も検証します。macOS のワーカーはソートが終わると join します。[ファイル名展開の実測](benchmarks.md#interactive-response)
- A byte-counted lower bound is safe for `read -N` on a pipe. If at least N characters are still needed, consuming at most N bytes leaves subsequent commands’ input intact, including UTF-8 and skipped NUL bytes. A read with a delimiter can finish on its next byte. The stream benchmark checks all output bytes at each size, following the independent output check in Align’s process-capture benchmark. A Linux memfd trial for large here-documents failed to improve the 256 KiB and 4 MiB medians in the local VM. [Byte-stream benchmark](../tools/bench_streams.py) · [Linux memfd semantics](https://man7.org/linux/man-pages/man2/memfd_create.2.html)
- パイプの `read -N` では残りの文字数をバイト単位の読み取り上限にできます。UTF-8 や NUL を含んでも次の読み手のデータを残せます。区切り文字を探す場合は次の1バイトで完了する可能性があります。Align のプロセス出力計測を参考に、データサイズを変えて毎回全バイトを確認しました。Linux の大きなヒアドキュメントを memfd に置く案は、この VM の256 KiB・4 MiBの中央値を改善できませんでした。[入出力の実測](benchmarks.md#interactive-response)
- The instrumented macOS build trained on the interactive and interpreter tests improved the function workload but produced a 1,367,856-byte executable. A PGO size-first build took 16.92 ms for the same functions. Adjusting the amount of machine-code outlining and sharing the expansion lookup code allowed the staged build to fit below the reference Bash size. These measurements apply to the selected workload and LLVM version. [Rust PGO workflow](https://doc.rust-lang.org/rustc/profile-guided-optimization.html) · [Release profiles](../tools/build-release.sh)
- プロファイルを使う最適化では関数実行が速くなる設定と、実行ファイルが小さくなる設定を別々に確認しました。両方を満たすため、共通の機械語列をまとめる量を調整し、展開処理の重複も減らしました。再現用のビルド設定をスクリプトに収録しています。[配布プロファイル](../tools/build-release.sh)


- Linking the GCC unwind archive into the Linux arm64 glibc build leaves only libc in ELF's shared-library dependency list. In 301 interleaved Debian samples this saved 27 µs at startup for 31,616 more bytes. Fully static glibc took 2,169,008 bytes. The arm64 glibc and static musl recipes target different installed locale data and were timed independently. [Release options and measurements](benchmarks.md#interactive-response)
- Linux arm64 では起動のたびに共有ライブラリを1本開く処理を省けました。glibc を丸ごと静的にした場合は Bash のサイズを超えたため、アンワインド処理だけをリンクしました。macOS・Debian・Ubuntu では、日本語・絵文字・結合文字を含む4,241個の名前で、C → 関数ローカルの ja_JP.UTF-8 → C の3回すべてについて Bash と出力を照合しました。[全項目の検証](compatibility.md)


- A pipeline stage that consists of `( commands )` already has a child in which to run the explicit subshell. A second fork increased `BASH_SUBSHELL` to 2 where Bash reports 1. Checking the depth inside a further nested group, the EXIT trap and all pipeline statuses guards the execution change. The upstream IFS test exercises this path thousands of times and supplies a checked output line. [Regression test](../tests/shell.py) · [Timing results](benchmarks.md#interactive-response)
- パイプの右辺にある丸括弧は、パイプのために作った子プロセスで実行できます。余分な fork を省き、リダイレクト先にコマンド置換がある場合も同じ深さになることを確認しました。親のジョブ一覧は明示的なサブシェルから切り離します。[連続実行の計測](../tools/bench_scripts.py)


## 2026-11-23 · Case-insensitive matches and the minimum compiler / 大文字小文字の照合とビルド条件

With `nocasematch`, Bash compares literal characters ignoring case. A POSIX class such as `[[:upper:]]` tests the original character. Captured groups and `&` in a replacement keep the matched spelling. A cached regular expression must include the case flag in its key since `shopt` can change between calls. The integration suite runs the same expression with the option enabled and disabled. [Regression test](../tests/shell.py) · [572-case comparison](../tests/compat/nocasematch.bash) · [Throughput measurements](benchmarks.md#nocasematch)

照合用に文字列全体を小文字にすると、POSIX 文字クラスや置換時の日本語を含む位置が壊れます。元の入力の文字境界をたどり、比較するときだけ大文字・小文字を対応づけます。ワイルドカードが一つのときは接頭辞と接尾辞を直接照合します。空文字にも一致する拡張パターンでの全置換では、次の文字を残して一文字ぶん進みます。

The first Rust 1.88 check selected the newer `rustc` on PATH. An explicitly paired Cargo and rustc 1.88.0 diagnosed three match guards in the bracket parser. Moving their conditions into the match arms or the enclosing lexer loop kept the declared compiler minimum. All 49 unit tests then passed with 1.88.0. [Rust language release notes](https://blog.rust-lang.org/2026/04/16/Rust-1.95.0/)

最小対応版を検証するときは Cargo と rustc の両方を同じツールチェーンに固定します。rustup を使う場合は `rustup run 1.88.0 cargo test --locked` で再現できます。

## 2026-11-23 · Key bindings, inherited functions and arithmetic / キー操作・関数の継承・算術

- A shell command registered by `bind -x` must run with the terminal restored. An external picker reads from the terminal even when its stdout is captured. fzf 0.74.0 installs a hyphenated function and binds Ctrl-T in both Emacs and vi-insert maps. We loaded `fzf --bash` into a staged shell and selected `unique-cat 雪.txt` in each map. The surrounding command received one argument. The regression suite also changes `READLINE_LINE` and `READLINE_POINT` around multibyte text. [Editor tests](../tests/shell.py)
- 端末の raw モードをいったん戻してから外部ピッカーを実行し、戻った行とカーソル位置を再描画します。macOS で Ctrl-T を送る試験は、プロンプトとブラケットペーストの有効化を待つ必要がありました。通常の端末設定で先に送ると、カーネルが Ctrl-T を `SIGINFO` として消費します。エディターの `VMIN=0` と `pselect` により、1バイトごとの非ブロッキング切り替えも省けます。
- Bash passes exported functions in environment entries named `BASH_FUNC_name%%`. Tonari parses a single named function definition on startup and retains its body for re-export. The four-way parent/child test also redefines the function, removes its export and unsets it. Parsed source locations provide `caller`, `LINENO` and `BASH_COMMAND` with the original text or call site. [Function and trace tests](../tests/shell.py)
- 親子を Bash と Tonari の全組み合わせで起動し、再定義後の本文、エクスポート解除後の探索、空白と日本語を含む引数を比較しました。外部プロセスを起動するときはセッションの変更分を合成し、プロセス起動時から不変の環境文字列を再利用します。
- For scalar arithmetic a plain decimal can be read directly from the session. A leading zero, an array, a name reference or an expression stored in a variable needs Bash’s normal evaluation rules. The regression test covers octal, base-16 notation, recursion and shift assignments. The operator scanner uses at most three input bytes to distinguish operators. Use the running-shell arithmetic benchmark to measure the effect. [Benchmarks](benchmarks.md#session-controls)
- 反復ループで毎回文字列を複製して整数式として再解析していた箇所を短縮しました。例えば `017` は8進数の15で、`16#ff` は255です。通常の10進数だけを速い経路に通し、配列と名前参照は元の評価器で処理します。 [算術テスト](../tests/shell.py)
- A coprocess owns two pipe ends and a job PID. A user-duplicated descriptor can still drain output after waiting for the process and clearing its original descriptors. A nested subshell retains visible array values while its endpoints are closed. The integration test exercises both conditions with an explicit exit status of 7.
- `coproc` の終了で閉じるのはシェルが所有している記述子です。呼び出し元が `exec 7<&"$fd"` で複製した側からは、`wait` の後も残った出力を読み出せます。子の終了値、配列の解除、サブシェルでの記述子の扱いを Bash と照合しています。

- Redirection order also determines when a descriptor variable becomes visible. In `exec {out}>file 5>&$out`, opening the first target binds the variable before the second target is expanded. A function can use this binding in its body. Normal builtin redirections restore their original descriptors, and `varredir_close` closes a newly allocated named descriptor after the command. File opens and duplicates must see the caller’s actual descriptor table. Reserving a number by opening `/dev/null` first can make an otherwise invalid duplicate succeed. [Descriptor tests](../tests/shell.py)
- `{out}` の番号は実際のファイルを開いてから OS に割り当てさせます。空き番号を先に予約すると、その番号を読むはずのないリダイレクトが成功してしまいます。後続のリダイレクトで参照するケース、関数、パイプ、ヒアストリング、明示的な解放を Bash と比較しました。
- Parsing every physical line of an unfinished `if` or function repeatedly reparsed the same prefix in distro profiles. Tonari now tries a complete compound command from bytes already in the source file’s read buffer. It consumes those bytes only when the parser returns one command. The next command sees changes to aliases and parsing options. A regression test defines an alias inside an `if`, invokes it on the following line, and checks the line number in a later function.
- 先読み済みの範囲から複合コマンドを一つだけ解析できた場合に限って読み取り位置を進めます。標準入力を子プロセスと共有する経路や詳細表示中は元の読み方を使います。Linux 標準設定を含むログインでは、同じ設定を実行できたことを確認してから時間を比較しました。[ログインの実測値](benchmarks.md#session-controls)

- Apple’s Git completion script contains a zsh parameter expansion inside a zsh-only branch. GNU Bash accepts the function file and raises a bad-substitution error only if that word is expanded. The Git callback also puts a space at the end of a completed branch name and sets `nospace`. In a Bash callback without filename quoting, that space terminates the word. Treating it as a literal filename character makes `git switch` look for a different branch. The Git terminal test creates the branch from another process after startup, completes its name and checks the resulting checkout. [Integration test](../tests/compat/tool_hooks.py)
- Git の補完を読み込むだけでも、実行されない分岐の展開エラーを遅延させる必要がありました。補完関数が付けた区切りとファイル名内部の空白では、引用処理も異なります。同じ端末テストで、direnv のシェルフックがディレクトリに入った時点で変数を設定し、出る際に解除することも確認できます。
- Rosetta’s Linux process translation creates `$HOME/.cache/rosetta`. In a test that uses the same fresh directory for HOME and the glob fixture, both shells then report an additional dotfile. The translator also needs descriptors to start an executable. We booted a Debian x86-64 kernel in QEMU and ran the original tests, including the low-descriptor limits, as an unprivileged user. The glibc x86-64-v3 build passed all 116. Two earlier emulated runs hit the four-second readiness timeout for a background Python worker in the hangup test. The final run completed with the original deadline. [Build and sizes](benchmarks.md#session-controls)
- x86-64 では初期の最小イメージに `en_US.UTF-8` がなく、参照 Bash 自身が `雪` を3バイトと数えていました。ロケールを用意し、Linux カーネルごと起動した Debian ゲストで再検証しました。エミュレーターで得た経過時間は実機性能には使っていません。

## 2026-11-23 — Login profiles and completion / ログイン設定と補完

- Debian 13 and Ubuntu 26.04 source `~/.bashrc` from their skeleton `.profile` when `BASH_VERSION` is set. Both set `HISTCONTROL=ignoreboth` there. Ubuntu also defines `ll`. A skipped rc can look like a fast startup. The distribution test copies both files to a temporary home and checks the same history settings and alias lookup as the packaged Bash. A separate PTY test executes the binary with `argv[0]=-bash`, then sends Ctrl-D and checks `.bash_logout`. [Integration tests](../tests/shell.py)
- Debian 13 と Ubuntu 26.04 の標準 `.profile` は `BASH_VERSION` が設定されていると `.bashrc` を読みます。双方で `HISTCONTROL=ignoreboth`、Ubuntu では `ll` の定義まで比較しました。Ubuntu の色判定にある `>&/dev/null` は、無効な分岐に書かれていてもパーサーが受理する必要があります。両ストリームのファイルへの転送と数値の記述子複製をテストしています。[回帰テスト](../tests/shell.py)
- Tonari exposes `BASH_VERSION=5.2.0(1)-tonari` and a read-only six-element `BASH_VERSINFO` for script feature gates. `--version` reports the Tonari version. Debian bash-completion 2.16.0 uses arithmetic on those array elements, global name references and dynamic function scope. With the package's compatibility wrappers and `gh completion -s bash` from GitHub CLI 2.96.0, Tab after `gh iss` inserts `gh issue ` in a native macOS arm64 PTY. Calling the completion function outside the editor also invokes `compopt` outside an active completion on Bash.
- Bash 用スクリプトの版判定には上記の互換値を返し、製品のバージョンは `--version` で表示します。補完ライブラリが別関数で呼び出し元のローカル変数を `unset -v` すると、既定では外側の束縛が再び見えます。`localvar_unset` を有効にした場合も Bash と比較しました。コールバック単体と端末で Tab を押す試験では `compopt` の実行条件が異なります。[起動ファイルの仕様](https://www.gnu.org/software/bash/manual/html_node/Bash-Startup-Files.html)

- On the tested macOS host, `/etc/profile` sources `/etc/bashrc`, which can source `/etc/bashrc_Apple_Terminal`. An isolated `--login -ic` run with `TERM_PROGRAM=Apple_Terminal`, a test `TERM_SESSION_ID` and an explicit `exit` set `SHELL_SESSION_DID_INIT=1` and created the per-session state files on both shells. The Apple script can explicitly write `~/.bash_history` alongside Tonari's native XDG journal.
- テストした macOS の Apple Terminal 用設定でも、専用の一時ホームでセッション初期化と終了処理を実行し、両シェルで `.bash_sessions` 以下のファイルが作られることを確認しました。OS の設定が明示的に書き出す履歴は、Tonari 自身の履歴ファイルとは別に存在します。

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

## 2026-11-22 · Faster pathname expansion / ファイル名展開の高速化

For `*/*.rs`, the next slash requires a directory. The directory entry already reports a type on APFS and many Linux filesystems. Skip known non-directories before matching or building a pathname. Symlinks and unknown types still go through normal lookup. Recursive traversal uses the same directory reader. On Linux it reads 32 KiB per `getdents64` call and visits names inside each returned batch. Align's file-descriptor scan uses the same record-length approach. On macOS, a names-only trial favored the existing `readdir` reader. In a warm 512-file directory, 50 scans took about 9.4 ms using `readdir` and about 28.5 ms using `getattrlistbulk` with `ATTR_CMN_NAME | ATTR_CMN_RETURNED_ATTRS` and a 32 KiB buffer. [Directory reader](../src/pattern/directory.rs) · [Linux directory entry ABI](https://man7.org/linux/man-pages/man2/getdents.2.html) · [Workloads and syscall counts](benchmarks.md#path-expansion-order)

複数階層のワイルドカードは、次のスラッシュを通れない通常ファイルをディレクトリエントリの型で早めに除外できる。シンボリックリンクと型が不明な項目は探索候補に残す。Linux 版は Align のファイル記述子走査と同様に、OS がまとめて返したレコードを長さに従って読み進める。Mac の APFS で名前だけを集める実験では、512件を50回読むのに `readdir` が約9.4 ms、`getattrlistbulk` が約28.5 ms だったため、Mac は既存の読み取り API を使う。

A pattern made entirely of literals and stars can match its fixed prefix and suffix first, then find each intervening literal in order in the remaining gap. Consuming each literal prevents overlapping matches. `?`, bracket expressions and escaped patterns still go through the locale-aware matcher. The native-matcher test covers adjacent stars, overlapping literals, leading dots, and UTF-8 characters. [Pattern matcher and differential tests](../src/pattern.rs)

固定文字列と複数の `*` からなるパターンは、先頭・末尾の固定部分を外してから、その間の文字列を順に探す。各部分の一致位置を消費してから次へ進めるので、同じ文字を二重に使わない。`?` や文字クラスの処理では従来のロケール対応の照合を使う。

Sorting the same 512 multilingual names repeatedly was expensive on the Mac. Computing a fresh `strxfrm` key per pathname took 47.3 ms for 30 expansions versus 47.2 ms with the previous comparison sort in a paired 18-sample trial. The retained optimization remembers the indices of just one sorted list. Every expansion reads the directory and matches the current pattern. Before reusing the indices it verifies the collation mode and every byte of every matched pathname in enumeration order, including name boundaries. A changed name, order or count triggers a new sort. For 32–2,048 paths with at most 64 KiB of name bytes including terminators, the retained tables use at most 80 KiB of payload on 64-bit targets. Other list sizes sort directly. The end-to-end test compares complete NUL-delimited Bash output across a same-length rename, creation and deletion in one shell. [Ordering code](../src/pattern/order.rs) · [Bash comparisons](../tests/shell.py) · [Apple collation API](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man3/strxfrm.3.html)

Mac で多言語名512件を30回展開すると、各回で照合キーを計算する試作は47.3 ms、従来の比較ソートは47.2 ms だった。採用版は直前1組の並べ替え順を保持する。毎回ディレクトリを読み、今回一致したパスのバイト列・名前の区切り・取得順・照合方式がすべて合う場合に再利用する。32〜2,048件、終端込みで名前が64 KiB以内のリストが対象で、64ビット環境で保持する名前と順序表の合計は最大80 KiB。同じ長さのファイル名への改名、作成、削除を同一シェルで行い、全件の順序を Bash と比較した。

Apple's libc implementation of `strcoll_l` converts both multibyte operands and calls `wcscoll_l` for each comparison. On a Mac with a UTF-8 character locale, Tonari can convert each pathname once to NUL-terminated Darwin wide characters. It then compares those strings with libc's `wcscoll` and uses the original bytes to break ties. Names above the stored-order limit can also use this conversion for a single sort. The wide-character buffer lives only for that sort. Both the old Tonari and GNU Bash agreed with the new output on 128 names with accents, combining marks, ligatures, Japanese characters and emoji under English, French, Japanese and Swedish UTF-8 locales. The comparisons also covered mixed C and English locale categories. [Apple strcoll source](https://github.com/apple-oss-distributions/Libc/blob/main/string/FreeBSD/strcoll.c) · [Apple conversion routine](https://github.com/apple-oss-distributions/Libc/blob/main/locale/FreeBSD/collate.c)

Apple の libc は文字列を比較するたびに両方をワイド文字へ変換している。Mac の UTF-8 ロケールでは、Tonari が各パスを一度だけ変換し、同じ照合関数で並べ替えると初回にも効果がある。照合順位が等しい場合は元のバイト列を比べる。保持する件数の上限を超えた場合も、その回のソート中だけ変換領域を使える。英語・フランス語・日本語・スウェーデン語の UTF-8 と、C を混ぜたロケールで、128件すべての順序が旧版・GNU Bash と一致した。

## 2026-11-22 · Quoting in wildcard patterns / ワイルドカード内の引用

Bash treats a quoted character inside a bracket list as a literal member. On Linux with musl, the previous matcher admitted a backslash as well as an asterisk for `x["*"]*.txt`. On Darwin it missed files beginning with a literal open bracket when an unclosed `[` was followed by an active star. Tonari now parses lists that contain escapes and patterns that contain an unclosed bracket. With `nullglob`, a lone unclosed bracket in a pathname component is literal. The differential test creates filenames containing metacharacters, whitespace, a backslash and emoji. It compares the count and NUL-separated bytes of the arguments in 839 cases under both C and UTF-8 character settings. Run `REFERENCE_BASH=/path/to/bash python3 tests/shell.py target/release/tonari -k glob_escaping`. [Matcher](../src/pattern/bracket.rs) · [End-to-end tests](../tests/shell.py)

Linux の musl 版では、`x["*"]*.txt` が `x` の直後にバックスラッシュのある名前にも一致していた。Mac では、閉じていない `[` に続く `*` を展開すると、一致するはずの名前を取りこぼしていた。Tonari は引用を含む角括弧と閉じていない角括弧を自分で解釈する。`nullglob` を有効にしても、パスの一区間に閉じていない角括弧だけがある場合は元の文字列を引数に残す。回帰テストは記号・空白・改行・バックスラッシュ・絵文字を含む実ファイルを作り、C と UTF-8 の各文字ロケールで839通りを Bash と照合する。表示の見た目に左右されないよう、引数の個数と NUL で区切ったバイト列を比較する。

For a quoted operand in a POSIX collating or equivalence symbol, the references differ. In the C locale `case a in [[."a".]])` matches on Ubuntu Bash 5.3.9. Tonari follows Debian Bash 5.2.37 for this case.

POSIX の照合要素や等価クラスの中身を引用した場合は参照版同士で結果が異なる。C ロケールで `case a in [[."a".]])` は Ubuntu の Bash 5.3.9 で一致する。Tonari はこの場合には Debian の Bash 5.2.37 に合わせている。

## 2026-11-22 · Signals and job exit status / シグナルとジョブの終了値

A foreground program keeps the terminal until it stops or exits. An explicit `wait "$!"` can return early with 128 plus the trapped signal number. The child remains waitable. Tonari records signals atomically and runs the action in shell context, where functions and local variables are available. For interruptible waits it checks child status with signals blocked and releases the mask atomically while sleeping. The PTY test stops a pipeline, resumes it in the background, brings it forward, and interrupts it. It checks the stopped and interrupted statuses and terminal ownership. The script tests also check every pipeline stage, `pipefail`, and cleanup on normal and signaled exit. Run `python3 tests/shell.py target/release/tonari`. [Tests](../tests/shell.py)

`wait` の中で登録済みのシグナルを受けると、その場で 128＋シグナル番号を返し、あとで元の子の終了値も回収できる。ハンドラ内ではシェルコードを実行せず、シグナル到着だけを記録する。実際の処理はコマンドの区切りで行うため、関数のローカル変数も参照できる。疑似端末ではパイプを Ctrl-Z で停止し、`bg`、`fg`、Ctrl-C と操作して前景プロセスグループと終了値を照合する。終了トラップでは通常の終了と TERM による終了を別々に検査する。

## 2026-11-22 — Reference-driven coverage and launch measurements / Bash を起点にした検証と起動計測

The implementation-led suite passed its 54 end-to-end tests on the reference shells. Enumerating the reference executables then exposed 20 missing builtins out of 61 on Ubuntu, along with observable differences in three other builtin probes. An interactive stop/bg/fg/interrupt cycle passes, while obtaining a job PID inside command substitution differs. The inventory exercises these paths separately. GNU golden comparisons also run against the distribution Bash before attributing a failure to Tonari. [Feature matrix and reproducer](compatibility.md)

従来の結合テスト54件が通っていても、Ubuntu の Bash が公開する組み込み61個のうち20個は Tonari に存在せず、さらに3個で直接の検証結果に差があった。実端末に相当する停止・再開・割り込みと、コマンド置換内でジョブの PID を取得する操作でも結果が分かれる。機能の判断には、Bash 側から列挙した名前、比較した具体例、正解を確認できたかどうかを併記する。

In a Debian arm64 VM on an Apple M1, a 40-sample diagnostic running `:` gave median times of 0.642 ms for the image's Bash and 1.514 ms for Tonari executed from a macOS bind mount. With both staged on the same guest tmpfs, Bash took 0.557 ms and Tonari 0.296 ms. The feature benchmark now stages both binaries before timing. The Linux fixture checks that its generated locales coexist with the distribution's C.UTF-8 locale. PTY probes drain redraw output while waiting for a capture file because macOS Readline can block in terminal output before executing the input. [Benchmark implementation](../tests/compat/bench.py) · [Terminal probes](../tests/compat/terminal.py)

Apple M1 上の Debian arm64 で `:` を40回測った。macOS 共有フォルダ経由の Tonari は中央値 1.514 ms、ゲスト内の Bash は 0.642 ms だった。同じゲストの tmpfs に両方を配置すると Tonari は 0.296 ms、Bash は 0.557 ms になった。計測器で配置をそろえ、Linux の試験環境では C.UTF-8 を含む必要なロケールが使えることも検査する。macOS の疑似端末では出力を読みながら完了を待つ。端末出力が滞ると、Readline が入力したコマンドをまだ実行できず、捕捉用ファイルの待機が誤ってタイムアウトすることがあった。

## 2026-11-23 · Subshell visibility, descriptors and bulk input / サブシェルと一括入力

- A command substitution and a foreground pipeline builtin can list the parent shell's jobs. The parent still owns the children and their wait statuses. An explicit parenthesized subshell starts with an empty job list. Tonari keeps a listing snapshot separate from the jobs the child can reap. The `wait -n` implementation blocks `SIGCHLD` before testing the selected children and releases the mask atomically while sleeping. The tests cover a selected PID, repeated waits, interrupted waits, and an unset `-p` destination.
- Tonari checks the entire list before capturing a sequence of simple output builtins in memory. A function override, redirection, arithmetic assignment or process-dependent argument can require a real subshell. In particular, a `printf -v` in one command can change the value consumed by the next. Differential tests exercise these effects and the final exit status. [Regression cases](../tests/shell.py)
- Brace alternatives duplicate the unevaluated word parts. For example, `{x,y}$((++n))` increments twice. Quoted punctuation and braces produced by expanding a variable stay literal. File globbing follows the choice of alternatives. A move such as `9>&8-` closes descriptor 8 even after an ordinary shell builtin returns. A noclobber open checks the opened object before allowing writes to an existing nonregular file.
- A limited `mapfile -n` on a pipe must leave the following record available to another `read`. A callback can also read or replace its input descriptor. The unrestricted reader can consume complete blocks, retain the last partial record across reads and store consecutive array elements in a vector. Deleting a middle element or assigning at a distant index preserves Bash's sparse keys. [Three-platform measurements](benchmarks.md#2026-11-23-runtime)
- 親のジョブを表示できる範囲と、終了状態を回収できる範囲は異なります。`$(jobs -p)` と前景パイプでは親のジョブを表示でき、明示した `(jobs -p)` では空になります。シグナルを待つ処理では、終了確認とスリープの間に通知を取り逃がさないようにしています。
- 純粋な出力だけのコマンド置換はメモリに直接書きます。関数で上書きした `echo` や、先の `printf -v` が後続の展開に影響する場合はサブシェルで実行します。波括弧の各候補には評価前の語を配るので、算術式の副作用と引用の有無も保持されます。
- `mapfile` の全件読み込みでは、行をまとめて読み、連続した添字の配列を直接作れます。`-n` やコールバックを指定した場合には後続の処理が同じファイル記述子を読めるため、先読みした分を考慮する必要があります。大量の行、区切り文字、パイプからの続きの読み取り、配列の途中削除を回帰テストに含めました。[回帰テスト](../tests/shell.py)
- Brace expansion can also extend a variable name. With `v=base`, `vx=X` and `vy=Y`, unquoted `$v{x,y}` yields `X Y`. Both `${v}{x,y}` and `"$v"{x,y}` yield `basex basey`. Only unquoted identifier characters can extend the name. The GNU braces driver exercises all three forms.
- Noninteractive monitor mode assigns a background job its own process group even when there is no controlling terminal. The regression test holds a child on a FIFO while checking its process group, then brings it to the foreground and waits for its real status. Directory-stack tests check `dirs -v`, rotation from both ends, a failed directory change and edits through `DIRSTACK`.
- 引用のない `$v{x,y}` は `vx` と `vy` の値に展開します。`${v}{x,y}` のように変数名の境界を明示した場合とは結果が異なります。非対話の `set -m` では制御端末がなくてもジョブ専用のプロセスグループを作ります。テストでは FIFO で子を生かしたままグループを確認し、前景に戻した後の終了値も照合します。
- A failed condition or update in an arithmetic `for` terminates the loop with status 1. A nonzero command in the body normally permits the next iteration. The regression covers a zero divisor, an invalid increment and semicolons inside a command substitution in the initializer. The GNU arithmetic-loop driver now returns to its golden-file comparison.
- The GNU jobs fixture uses both `kill -n9` and `kill -sHUP`. Failing to recognize an attached argument left real sleepers alive until their original deadlines. A focused test checks signal exit statuses 137 and 129, as well as bringing a job started before `set -m` into the foreground.
- Cache the validated logical working directory on its first use. A relative `chdir` can still succeed after an external rename invalidates the saved pathname. The regression compares `pwd -L`, `pwd -P` and `cd ..` in the same shell. A separate resource-limit probe confirmed that Bash can fail to save the target of a persistent descriptor move when every descriptor is occupied. Its temporary descriptor affects the observable exit status.
- `[[ == ]]` enables extended patterns independently of `shopt extglob`. In a `case` arm the option must already be enabled when the shell reads it. The tests exercise quoted alternatives, an empty alternative under repetition, negation and the special behavior of a plain star followed by a nullable group. The same option controls the expansion of filenames and matching within parameter operations.
- 算術 `for` の条件・更新式でエラーが出たら終了値1でループを抜けます。初期化式の `$(...)` 内にあるセミコロンは式の区切りに使いません。ジョブ試験では `kill -n9` と `kill -sHUP` の両方を使い、子の実際の終了値を確認しています。
- 論理作業ディレクトリの検証結果を初回に保存しました。別プロセスがディレクトリを改名したときは、カーネルが保持する現在位置を起点に相対移動できます。一方、記述子の移動では、保存用の空き記述子がないと Bash も失敗します。この違いは `ulimit -n` で再現できます。
- 拡張パターンは `[[ == ]]` では常に使えます。`case` のリテラルでは構文を読む時点での `extglob` が効き、変数から渡したパターンでは照合時の設定も効きます。正負のグループ、引用した縦棒、繰り返し内の空の選択肢、ドットファイルを Bash と比較しました。[比較テスト](../tests/shell.py) · [速度](benchmarks.md#2026-11-23-runtime)

In a repeated case arm such as `pkg-+([0-9]).@(c|rs)`, the syntax and literal alternatives can be compiled when the command is read. A single variable-width character run between unambiguous edges can be tested directly. Nested alternatives and overlapping endings keep the full matcher. The generated regression compares 400 patterns against ten inputs apiece. Both references agree on a special case where a plain star is followed by an extended group that accepts empty input. GNU's `run-extglob` also exercises `GLOBIGNORE` bracket classes containing colons, explicit matches of `.` and `..`, and malformed bracket expressions in a group. [Pattern tests](../tests/shell.py) · [Measurements](benchmarks.md#2026-11-23-runtime)

固定の `case` パターンはコマンドの読み取り時に解析し、各反復で再利用します。可変長部分が一つで、前後のリテラルで境界が決まるときは候補位置を列挙せずに照合できます。回帰試験では決定的に生成した400パターンを10入力ずつ Bash と比較します。GNU の試験では `GLOBIGNORE` の文字クラス内のコロン、`globskipdots` を無効にした場合の `.` と `..`、不正な括弧を含むグループを確認しました。ファイル記述子が満杯のときは、`exec 6>&-` が失敗を返しても記述子6は閉じられます。次の `exec 6>&7` の成否までテストします。


Before commit `2af2f21`, a path-filter difference reproduced with filenames containing only punctuation. With `extglob` enabled and files `++` and `x`, running `GLOBIGNORE='+([^[:alnum:]])'; printf '<%s>' ./*` gives `<./x>` in the Bash 5.2.37 control. That version of Tonari also listed `<./++>` because it matched the ignore list one pathname component at a time. The mixed case involves an extended group spanning the slash in `./`. The GNU golden matches above cover their own fixtures.

`GLOBIGNORE` に拡張パターンを指定し、句読記号だけのファイル名を `./*` で列挙する組み合わせでは、当時の版に差がありました。修正後の結果を次節に記録しました。

## 2026-11-23 · Variables, glob errors and job ownership / 変数・展開エラー・ジョブ制御

In Bash 5.2 and 5.3, a failed assignment to a read-only variable skips the rest of the current parsed input unit. The following complete command can inspect status 1. With a command prefix such as `x=bad y=ok /bin/sh -c 'echo "$y"'`, the command can still run with the valid binding. The `readonly` attribute belongs to the variable in its dynamic scope. An inner function can create a fresh local over an outer function’s read-only local. A read-only global prevents a local declaration of that name. The tests exercise all three script entry points, compound array initializers, arithmetic writes, `read`, `printf -v`, and function redefinition. [End-to-end witnesses](../tests/shell.py)

Bash の読み取り専用変数への単独代入が失敗すると、同じ行の後続コマンドを飛ばし、次の完全な入力単位で終了値 1 を参照できます。外部コマンドに付けた一時代入では、保護された値を維持し、有効な他の代入を渡して起動します。関数では外側のローカル変数とグローバル変数で同名の `local` を宣言できる条件が異なります。`-c`、ファイル、標準入力でこの差を比較しています。

A subscript can represent an integer expression or an associative key. Expanding the latter once preserves spaces, literal wildcard characters, brackets and UTF-8 text inside the key. The end-to-end test iterates the keys, looks each value up again, writes through `printf -v` and removes a key through `unset`. The declaration and lookup tests also cover integer and read-only associative arrays. The C library’s case mapping can differ between platforms. Both Linux Bash controls preserve lowercase sharp S when uppercasing. With the default ASCII range option, `[a-z]` excludes accented letters even when the host system’s UTF-8 `fnmatch` treats them as members. The pattern matcher handles these ranges explicitly. The UTF-8 test checks this along with a switch to the C locale inside the same script.

連想配列のキーは文字列として展開します。空白や絵文字、`]`、`*` を含むキーを列挙し、そのキーで再度同じ値を引けることを検証しました。英字の変換では `LC_ALL=C` と UTF-8 を同一スクリプト内で切り替え、アクセント付き文字と `ß` を Bash と比較しています。既定の `[a-z]` の範囲はシステムの `fnmatch` と結果が分かれる場合があるので、Tonari のパターン照合で範囲を判定します。

Two files, `++` and `x`, reproduce a difference between the Bash releases. With `extglob` enabled and `GLOBIGNORE='+([^[:alnum:]])'`, `./*` expands to `./x` on Debian 13 and also includes `./++` on Ubuntu 26.04. Tonari follows Bash 5.2 for this bracket-class path filter. Alternatives containing a literal slash, such as `@(sub/file|./keep)`, are tested against the entire pathname on both distributions.

`GLOBIGNORE` が句読点だけを除く設定では、`./++` を残すかどうかが Debian と Ubuntu の Bash で異なりました。この場合は Debian の Bash 5.2 に合わせ、両方の参照結果を回帰テストに残しました。

An `ERR` handler sees the failing status and is skipped in tested conditions. A `RETURN` handler sees the status just before an explicit `return N`, while the caller receives `N`. A PTY test starts three jobs and sends SIGHUP to the interactive shell. It observes receipt in the owned job, then signals the marked and removed jobs through a file so that each can confirm survival. This also checks that `disown -h` keeps a job visible to `jobs`. [Terminal and trap tests](../tests/shell.py)

`ERR` は条件判定中の失敗では発火しません。`return 6` の直前の終了値が 1 なら、`RETURN` ハンドラーで見る値は 1、呼び出し元に返る値は 6 です。端末テストでは SIGHUP を受けたジョブと生存したジョブを子プロセス自身に記録させ、`disown -h` 後も `jobs` から参照できることを確認します。

The four repeated workloads take 0.293–0.635 of Bash time at the median. ASCII-only case conversion can update the original buffer. Single-character locale mappings still apply to non-ASCII input. [Release timings and sizes](benchmarks.md#2026-11-23-compatibility)

修正した4処理の反復計測では、Tonari の中央値は Bash の0.293〜0.635倍でした。ASCII の大文字・小文字は既存バッファ内で変換し、UTF-8 の文字はロケールの1文字単位の規則で変換します。[実測値とサイズ](benchmarks.md#2026-11-23-compatibility)

## 2026-11-23 · UTF-8 contract in release checks / 意図した UTF-8 の差を検証する

The release audit runs both the UTF-8 IFS example and a C-locale slice on each reference and Tonari. With `v=雪🌸`, all three C-locale Bash controls return 7 for `${#v}` and a lone byte `9b` for `${v:1:1}`. Tonari returns 2 and the complete flower character. The end-to-end suite also feeds byte `ff` directly into command text at three entry points and sends every byte from `00` to `ff` unmodified through external commands, pipes and file redirection. These results distinguish a shell text guarantee from byte transport and can be checked independently of the reference's behavior. [Inputs, results and rationale](compatibility.md#intentional-differences)

UTF-8 の IFS と C ロケールでの切り出しを検証器に登録しました。参照先の Bash が将来 Tonari と同じ結果を返せば通常の互換動作として計測できます。Tonari の期待値を破れば、たとえ両者が同じ結果になっても仕様違反として検出します。別のテストでは `-c`・ファイル・標準入力に生の不正な UTF-8 を渡し、パイプとリダイレクトでは256通りのバイトがそのまま通ることを確認します。[比較例と採用理由](compatibility.md#intentional-differences)

## 2026-11-23 · Parity for short operations and OS waits / 短い処理と OS 待ちの許容範囲

The Ubuntu `wait -n` probe waits for a child running `sleep .02`. Its whole-run median is 25.80 ms in Tonari and 26.23 ms in Bash. The requested 20 ms sleep alone exceeds two-thirds of the Bash median, so parity is acceptable for this workload. Startup, interactive response and repeated shell work still have their own measurements. [Ubuntu timings and witnesses](compatibility-2026-11-23/Ubuntu.tsv) · [Performance target](benchmarks.md#performance-target)

Ubuntu の `wait -n` の検証は、子で `sleep .02` を実行します。起動込みの中央値は Tonari が25.80 ms、Bash が26.23 ms で、指定した20 ms の待機だけで Bash の所要時間の 2/3 を超えます。このケースは同等の速さとして許容します。ごく短い単発処理も同じ考え方です。起動や入力応答、反復で積み上がるコストはそれぞれ測定します。[Ubuntu の実測値とスクリプト](compatibility-2026-11-23/Ubuntu.tsv)
