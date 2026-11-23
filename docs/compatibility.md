# Bash compatibility audit / Bash 互換性の検証

Tonari remains a prototype for use from an existing shell. The inventory starts with the reference Bash installations and the GNU manual. The matrix gives the observed behavior and the elapsed time for each matching workload. [Current feature inventory](compatibility/current.tsv) · [Timing matrix](compatibility-2026-11-23-release/matrix.md) · [Debian evidence](compatibility-2026-11-23-release/Debian.tsv) · [Ubuntu evidence](compatibility-2026-11-23-release/Ubuntu.tsv) · [macOS evidence](compatibility-2026-11-23-release/macOS.tsv)

Tonari は現在、既存のシェルから起動して試せる段階です。参照先の Bash と GNU マニュアルを起点に項目を列挙しました。一覧には機能名ごとの判定と速度を載せ、環境別 TSV には実際のテスト内容と差が出た出力例を残しています。[現在の機能別一覧](compatibility/current.tsv) · [環境別の実測](compatibility-2026-11-23-release/matrix.md)

On November 24 the reference audit covered 1,408 macOS, 1,522 Debian and 1,553 Ubuntu rows. The updated interactive defaults and their direct Bash witnesses are listed below. The end-to-end suite passed on Debian and Ubuntu arm64, Debian x86-64 under QEMU, and macOS arm64 (142 passed, two platform skips). Debian arm64 also passed all 144 tests with a one-CPU quota and affinity. [PTY witnesses](../tests/shell.py)

Compared with `7fdf66e`, the inventory changed five names on macOS and Debian and three on Ubuntu. `copy-region-as-kill` and `overwrite-mode` report different default bindings on all three platforms. `beginning-of-line` and `end-of-line` also changed on macOS and Debian. Ubuntu already reported a difference. The `progcomp_alias` option now passes its interface check on all three. The PTY tests below exercise the actual edits and alias lookup. [Updated inventory](compatibility/current.tsv)

11 月 24 日の配布版で macOS・Debian・Ubuntu の計 4,483 行を再確認しました。前回から判定が変わったのは macOS と Debian で各 5 項目、Ubuntu で 3 項目です。変更は編集キーの既定割り当てと補完用のシェルオプションに限られます。

## Editing keys and long commands / 編集キーと長いコマンド

Tonari’s default Emacs keymap binds Insert to `overwrite-mode` and Alt-W to `copy-region-as-kill`. Home and End also accept the terminal sequences `CSI 1~`, `7~`, `4~` and `8~`, respectively. Bash’s default bindings differ, which is visible in `bind -q`. With the same bindings, Bash and Tonari both turn `ABCD` into `A東京D` when typing those two characters over `BC`. Bracketed paste at that position inserts text. The PTY tests also verify overwrite-mode Backspace and copying and yanking a marked region.

Insert は通常入力と上書きの切り替え、Alt-W は選択範囲のコピーに割り当てています。Bash と同じ割り当てにした比較では、上書き・Backspace・範囲コピーと貼り付けの結果が一致します。端末の貼り付けは上書きモード中でも挿入として扱います。長い編集行の `…` は表示範囲外のテキストを示し、コマンド本体・履歴・補完に渡す文字列はすべて保持します。

<a id="interactive-tab"></a>

## Interactive Tab and terminal ownership / 対話補完と端末入力

Tonari enables Bash’s `progcomp_alias` in interactive shells. With `alias g='real sub'`, the completion for `real` sees `COMP_LINE='real sub ma'` when the user types `g ma<Tab>`. The editor keeps `g` on screen. A definition for `g` wins. Use `shopt -u progcomp_alias` to disable alias lookup. Bash 5.2 produces the same callback arguments with the option enabled.

File and command completion can echo newly typed text and Backspace while scanning. The editor applies Tab first and then the queued edits. A Bash completion function can itself read stdin or launch an interactive selector. Tonari gives those callbacks their usual terminal input. To let the editor receive typeahead during a callback that never reads or manages the terminal, register it with `complete -o typeahead -F _pick demo`. For a shared Bash rc, use `complete -o typeahead -F _pick demo 2>/dev/null || complete -F _pick demo`. A slow helper still has to finish before the command line can be accepted. [PTY witnesses](../tests/shell.py)

対話時の `progcomp_alias` は `shopt -u progcomp_alias` で解除できます。エイリアス自身の補完定義があればそちらを優先します。ファイルやコマンドを探している間も文字入力と Backspace を先行表示し、確定時には Tab → 後続キーの順に反映します。外部の選択 UI や `read` を使う Bash 補完には端末を渡します。端末を使わない補完に限り、登録時の `-o typeahead` で待機中の入力表示を有効にできます。

The default argument completer tries exact paths, then abbreviated directory components, then recent arguments. After an option followed by an empty word, a value previously used for that option takes priority. History candidates come from complete, successful journal records in the last 256 KiB for the current directory or Git worktree. A path from another command must still exist. Explicit completion definitions keep their own results, including empty results, unless they request the default fallback. The abbreviated path search stops if a level would retain more than 128 directories. Shell expansions in saved commands are never evaluated.

引数の既定の補完は通常のパス、階層ごとの前方一致、最近の引数の順に探します。オプション直後の空欄ではそのオプションの過去の値を先に出します。対象は履歴末尾 256 KiB にある終了コード 0 の完全な行のうち、現在のディレクトリか同じ Git ワークツリーに属するものです。別コマンドから拾うパスは実在を確認します。保存したコマンド内の式は実行しません。

## Case-insensitive pathname expansion / 大小文字を区別しないファイル名展開

`shopt -s nocaseglob` applies to pattern components and to `GLOBIGNORE`. A literal component still uses the filesystem's lookup rules. With a file named `Sub/K.TXT`, `s*/k*.txt` returns its actual spelling. On a case-sensitive filesystem `s*/k.txt` has no match. `nocasematch` controls separate shell operations. [Integration test](../tests/shell.py) · [Timings](benchmarks.md#nocaseglob)

既定値と `shopt -u nocaseglob` はどちらも大小文字を区別します。`LC_CTYPE` が UTF-8 のとき `?` は1文字、`C` のときは1バイトに一致します。どちらも引数として渡すのはファイル名全体です。大文字・小文字を無視しても、`[[:upper:]]` は元の文字が大文字かどうかで判定します。

Compared with `ed056ca`, the full audits changed one verdict on each of macOS (1,408 rows), Debian (1,522) and Ubuntu (1,553). The `shopt nocaseglob` witness now returns `<UPPER>` for `upp*` on all three. Linux passed all 126 integration tests on Debian arm64, Ubuntu arm64 and the Debian x86-64 QEMU guest. macOS passed 125 with one Linux-only skip. [Current inventory](compatibility/current.tsv)

macOS では325パターンを、大小文字を区別する展開、区別しない展開、およびそれぞれの除外条件に使いました。UTF-8 と C の各文字ロケールで照合順序を C に固定すると、2,600件すべてで Bash 5.2.37 と同じ引数列になりました。名前付き照合順序で同順位になる例は下の意図的な差に記録しています。

## Distribution release checks / 配布バイナリの検証

The arm64 -O2 and x86-64-v3 -Os distribution binaries retained all 1,408 macOS, 1,522 Debian and 1,553 Ubuntu feature verdicts from `9a095b8`. The added regression compares `BASH_SOURCE` with GNU Bash as a running function is redefined and then removed by a nested call. All 125 integration tests passed on Debian and Ubuntu arm64 and in the Debian x86-64 QEMU guest. On macOS arm64, 124 passed and the Linux-only test was skipped. Debian and Ubuntu also passed account login and logout. [Release measurements and executable fingerprints](benchmarks.md#release-footprint)

速度とサイズを比較して選んだ配布版で互換性を再検査しました。関数の再定義・削除をまたぐ `BASH_SOURCE` も参照先の Bash と同じ値になり、既存の機能判定に変化はありませんでした。今回の速度測定は起動・関数などを対象にしています。下記の機能別タイミング行列は一つ前の配布版の記録です。

### Previous release with the new allocator / アロケータ変更時の配布版

The new macOS arm64, Debian arm64 and Ubuntu arm64 builds retained every named verdict from the preceding audit, across 1,408, 1,522 and 1,553 inventory rows respectively. This includes known differences, unverified options and intentional UTF-8 behavior. The macOS integration suite passed 123 tests with one distribution-specific skip. Debian and Ubuntu arm64 and the Debian x86-64 QEMU guest each passed all 124. Both arm64 distributions also passed account login and logout through the private `bash` entry point. During the first timed macOS pass the `PROMPT_DIRTRIM` terminal probe missed its expected output once. With the same binaries, a focused run of two warmups and nine measured samples per shell returned the expected trimmed prompt each time. [Release measurements](benchmarks.md#release-latency)

今回の配布版を前回の監査と照合し、macOS・Debian・Ubuntu の全項目で機能判定が維持されていることを確認しました。以前からある未実装や動作差も一覧に含みます。Linux の統合試験は3環境とも124件が成功し、macOS はディストリビューション固有の1件を除く123件が成功しました。[優先項目の実測](compatibility-2026-11-23-release/priority.tsv)

All matching witnesses have fresh whole-run measurements: 777 on Debian, 773 on Ubuntu and 757 on macOS. The harness also times already-running shells where a witness can use the readiness handshake. Each regular workload had two warmups and nine measured runs per shell. GNU drivers had three measured runs. Relative to the previous platform snapshots, 5 Debian, 8 Ubuntu and 28 macOS names crossed the two-thirds target upward for whole runs. The corresponding counts for already-running shells were 2, 1 and 28. Correctness outcomes and previously measured names were retained. [Feature timings](compatibility-2026-11-23-release/matrix.md) · [Threshold crossings by name](compatibility-2026-11-23-release/target-crossings.tsv) · [Executable fingerprints](compatibility-2026-11-23-release/environment.json)

参照結果に一致した項目はすべて配布バイナリで再計測しました。短い例を含む機能別データと、処理を繰り返す47件のスクリプトでは所要時間の内訳が異なります。上限を新たに超えた項目には Debian の `compgen -X` や Ubuntu の `exec` があり、操作名と計測区間を比較表に残しています。[機能別データ](compatibility-2026-11-23-release/matrix.md) · [スクリプトと対話入力](compatibility-2026-11-23-release/priority.tsv)

## Static Linux release / 静的 Linux 配布版

We compared the static arm64 executable with the Bash in Debian and Ubuntu, and reran the macOS inventory. The only change in the named verdicts from the preceding snapshot is Linux `LC_COLLATE`, now a tested intentional difference. Debian 13 and Ubuntu 26.04 arm64 passed all 124 integration tests and the account login/logout test. Debian x86-64-v3 passed the 124 tests under QEMU. macOS arm64 passed 123 with one Linux-only skip. [Observed changes](compatibility/changes.tsv) · [Linux feature timings](compatibility-2026-11-23-static/matrix.md)

Linux はロケールを変えたときも、ファイル名展開の引数を UTF-8 のバイト順で渡します。GNU の回帰グループも含め、直前の glibc 配布版から判定が変わったのは、両ディストリビューションの `LC_COLLATE` だけでした。[互換差の具体例](#intentional-differences) · [ビルド条件と速度](benchmarks.md#static-linux)

## Earlier interactive release / 対話処理更新時の記録

The binaries built from `8e9963e` matched 762 direct checks on Debian 13, 760 on Ubuntu 26.04 and 757 on macOS. Compared with the preceding snapshot, the Debian and Ubuntu builds now match the `LC_COLLATE` witness. All previously verified observations were retained. All 61 builtins match their primary witness. The GNU upstream regression groups matched completely in 16 of 83 cases on Debian and 14 of 86 on Ubuntu. A passing primary witness covers that invocation and its recorded output. [Current inventory](compatibility/current.tsv) · [Changes](compatibility/changes.tsv) · [GNU regression groups](compatibility/gnu.tsv)

今回の実行ファイルで統合テスト124件を再実行しました。Debian・Ubuntu の arm64 と QEMU の Debian x86-64-v3 は全件通過し、macOS は123件通過、Linux 専用の1件をスキップしました。Linux の一時アカウントで、実際のログイン、プロファイルと rc の読み込み、ログアウト、終了値も確認しました。名前付きロケールでのファイル順は OS の照合テーブルを使うビルドで検証しています。arm64 の静的 musl 版には別の機能別実測表があります。Readline の一部や Bash 5.3 の構文などの差は、一覧から個別の検証内容とともに確認できます。[全項目の判定と時間](compatibility-2026-11-23-response/matrix.md) · [静的 musl 版](compatibility-2026-11-23-response/musl/matrix.md) · [操作別の実測](benchmarks.md#interactive-response)

One timed Debian `run-procsub` attempt produced output that differed from the recorded reference when running GNU Bash itself. We repeated just this driver with the same two executable digests, two warm-ups and seven measured samples per shell. Every repeated sample matched the recorded output. That snapshot's portable integration suite used `TONARI_TEST_LIBC=musl` to check the documented byte-order contract. [Debian measurements](compatibility-2026-11-23-response/Debian.tsv) · [Static build measurements](compatibility-2026-11-23-response/musl/Debian.tsv)

Debian の `run-procsub` 初回計測では参照用 Bash 自身の出力が揺れたため、同じバイナリでこの項目を再計測しました。公開した7回分は両シェルとも記録済みの出力に一致しています。この時点の静的版の統合試験では `TONARI_TEST_LIBC=musl` を指定してバイト順を確認しました。[検証データ](compatibility-2026-11-23-response/matrix.md)

## Case-insensitive matching / 大文字小文字を区別しない照合

On `fc25e76` we compared `nocasematch` in `case`, `[[ == ]]`, `[[ =~ ]]`, parameter replacement and programmable completion filters. The 19 strings and 28 patterns exercise quoting, extended globs, UTF-8, negative classes and POSIX classes. Forty further combinations exercise zero-length replacement with four different anchors. All 572 stdout lines were identical on each of the three platforms. The strict test operator `[ ]`, ordinary filename expansion and prefix/suffix removal retain their case sensitivity. [Reproducible shell script](../tests/compat/nocasematch.bash) · [Shell integration tests](../tests/shell.py)

直接の動作照合は Debian 761件、Ubuntu 759件、macOS 757件が一致しました。Debian と Ubuntu の arm64、および QEMU の Debian x86-64-v3 は統合テスト117件を通過し、macOS は116件通過と Linux 専用の1件スキップでした。Linux の一時アカウントでは最終配布ファイルをログインシェルに登録し、ログアウト後の終了値まで確認しました。GNU の試験群の完全一致は Debian 16/83、Ubuntu 14/86 です。[現在の機能別判定](compatibility/current.tsv) · [GNU グループごとの出力差](compatibility/gnu.tsv) · [全項目の速度](compatibility-2026-11-23-nocasematch/matrix.md)

## Session controls and script execution / 対話操作とスクリプト

The primary checks for all 61 builtins match on Debian, Ubuntu and macOS. The new checks exercise `bind` with a key assignment and a query, `caller` in nested functions, `enable` through command lookup, `help` through a synopsis and `suspend` through an actual stop and resume. Bash and Tonari also inherited exported functions in all four parent/child combinations. Further cases cover `DEBUG` and function tracing, history expansion, `select` with invalid input and EOF, `time` on pipelines, and bidirectional `coproc` communication followed by `wait`. [Integration suite](../tests/shell.py) · [Feature timings](compatibility-2026-11-23-shell/matrix.md)

ログイン時は `/etc/profile` を先に読み、続いて `~/.bash_profile`、`~/.bash_login`、`~/.profile` の順で最初に読める一つを実行します。引数 `--noprofile` でこれを省略できます。通常の非ログイン対話端末では Bash の rc を読み、非対話の `-c` では設定済みの `BASH_ENV` を読みます。実際の Debian・Ubuntu の標準設定と macOS のログイン設定を一時ホームで比較しました。61件は組み込みごとの代表テスト数です。Readline の個々の操作や Bash の各オプションは別の行で検証しています。[最新の機能別判定](compatibility/current.tsv) · [GNU 試験の差分](compatibility/gnu.tsv)

Compared with `1507031`, 84 more direct checks match on Debian, 83 on Ubuntu and 84 on macOS. All previously matching direct checks still pass. Both the 61 builtin and 22 reserved-word primary cases match on each platform. The 116-case integration suite passed on Debian and Ubuntu arm64, and on a Debian x86-64 QEMU guest. On macOS 115 passed and the Linux distribution-profile case was skipped. The GNU driver totals are 16 golden matches of 83 on Debian and 14 of 86 on Ubuntu. Both job-control drivers ran to completion and still show differences. [GNU observations with examples](compatibility/gnu.tsv)

名前付きリダイレクトの `exec {log}>file` と解放、`varredir_close` の動作、消えた実行ファイルを `checkhash` で再探索するケースも実行しました。Ubuntu の `shopt` は59項目中12件が動作まで一致し、8件はインターフェースを確認、2件は差があり、37件は未実装です。GNU のまとまった試験には出力差が残り、全機能互換は未達です。[機能別判定](compatibility/current.tsv)

In disposable Debian and Ubuntu accounts we registered Tonari’s private `bash` entry point as the login shell and entered using `su --pty --login`. The session saw the registered `SHELL`, `$0=-bash`, the user profile and the distro’s `HISTCONTROL=ignoreboth`. An explicit `exit 19` ran `.bash_logout` and returned status 19. [Disposable account test](../tests/compat/login_account.sh)

実ツールでは、macOS の Git でシェル起動後に別プロセスが作ったブランチを Tab 補完して切り替え、Debian の direnv 2.32.1 で移動時の環境変数の適用と解除を確認しました。fzf 0.74.0 はフィルターで選択を固定し、実際の Ctrl-T バインドから空白・日本語入りの一引数が渡ることを Emacs と vi の両モードで照合しました。[Git・direnv の端末テスト](../tests/compat/tool_hooks.py) · [fzf の端末テスト](../tests/compat/fzf_keys.py)

The same macOS release and reference also loaded nvm 0.40.3 and 0.40.5. For each version both shells switched from Node 24.1.0 to 18.13.0 and reported the expected version with `node --version`. We used a temporary home and nvm directory with links to the installed Node versions.

nvm の2バージョンとも一時ホームで読み込み、同じセッションで Node を2回切り替えて確認しました。ログインやツールの端末テストは次のように再実行できます。Git の例ではインストール済みの Bash 補完スクリプトのパスを指定します。

```sh
sh tests/compat/login_account.sh target/dist/linux-arm64/libexec/tonari/bash debian:13
sh tests/compat/login_account.sh target/dist/linux-arm64/libexec/tonari/bash ubuntu:26.04
python3 tests/compat/tool_hooks.py target/release/tonari --git-completion "$git_completion"
python3 tests/compat/tool_hooks.py target/release/tonari --direnv
python3 tests/compat/fzf_keys.py target/release/tonari --fzf fzf
```

## Earlier login checks on 2026-11-23 / ログインシェルの検証

The primary witnesses for `logout`, `history` and `fc` now match on Debian 13, Ubuntu 26.04 and native macOS arm64. Of Ubuntu's 61 builtins, 56 match their primary check. The remaining five absent interfaces are `bind`, `caller`, `enable`, `help` and `suspend`. Direct comparisons cover the distro login path, local variable restoration during completion, arithmetic array subscripts, and shell nesting. The history journal stays under the XDG state directory. The `history` and `fc` checks cover selected file operations, listing and replay. Interactive history event expansion and all Readline bindings still lack full compatibility. [Integration tests](../tests/shell.py) · [Login timings](benchmarks.md#login-profiles)

ログイン時の `/etc/profile` とユーザープロファイルの順序、通常の `-c` での `BASH_ENV`、Ctrl-D による終了処理を検証しました。Debian と Ubuntu は各103件の統合テストを通過し、macOS はディストリビューション固有の1件を除く102件を通過しました。macOS での初回の並列実行では、参照 Bash の Ctrl-C トラップと、前景パイプラインの再開後に Ctrl-C を送る二つのテストがタイムアウトしました。全体を再実行して通過した結果を記録しています。`history` と `fc` の代表ケースは一致し、`bind` など五つの組み込み、`DEBUG` トラップや関数のエクスポートには差が残ります。[互換性一覧](compatibility/current.tsv)

The current direct audit adds 43 matching names on each platform against `f3f5539`. Every previously matching direct check still passes. GNU golden matches remain 16 of 83 groups on Debian and 14 of 86 on Ubuntu. Both `run-jobs` drivers finished. [Current inventory](compatibility/current.tsv) · [GNU observations](compatibility/gnu.tsv)

直前の版と比べて、各環境で43項目の直接比較が新たに一致しました。GNU の回帰試験は両環境とも Tonari のタイムアウトが0件でした。GNU のグループごとの出力差や判定不能の件数は下の表に記載しています。

## Earlier checks on 2026-11-23 / 同日のログイン対応前の結果

The primary checks for 53 of Ubuntu's 61 Bash builtins match on stdout, stderr and exit status. The eight missing builtins are `bind`, `caller`, `enable`, `fc`, `help`, `history`, `logout` and `suspend`. Of the 59 Ubuntu `shopt` names, 11 match their behavior check, three accept the interface and 45 are missing. The `DEBUG` trap, exported shell functions and loading the full Debian bash-completion setup remain open compatibility issues. [Current feature inventory](compatibility/current.tsv)

In each of the three environments, 36 direct checks that failed or lacked an interface at commit `835304b` now match. All previously matching direct checks still pass. Changes include associative keys, read-only bindings, parameter case conversions, `ERR` and `RETURN` traps, and background job ownership. The integration suite exercises 94 cases on each platform, including a hangup on a controlling terminal. Against the November 22 snapshot, the cumulative gains are 162 direct checks on Debian, 161 on Ubuntu and 156 on macOS. The inventory covers 1,554 distinct names, including two explicit UTF-8 contracts. Some names exercise the same behavior. [Changed observations](compatibility/changes.tsv) · [Release measurements](benchmarks.md#2026-11-23-compatibility)

Ubuntu の Bash の組み込み61個のうち、53個の代表ケースで出力と終了値が一致しました。`history`、`fc`、`bind` など8個が未実装です。直前の版で差があった直接比較のうち、各環境で36項目が新たに一致し、以前通ったものはすべて通っています。94件の統合テストでは、空白や絵文字を含む連想配列のキー、読み取り専用変数への書き込み失敗後の再開位置、端末終了時に `disown -h` で保護されたジョブの生存も確認しました。`DEBUG`、関数のエクスポート、Debian の bash-completion 一式の読み込みには引き続き差があります。[機能別結果](compatibility/current.tsv) · [修正後の速度](benchmarks.md#2026-11-23-compatibility)

## GNU regression results on 2026-11-23 / 修正後の GNU 回帰試験

| Platform | Groups | Golden match | Different | Bash control inconclusive | Tonari timeout | Separate exported-function probe |
|---|---:|---:|---:|---:|---:|---:|
| Debian 13 | 83 | 16 | 63 | 3 | 0 | 1 |
| Ubuntu 26.04 | 86 | 14 | 64 | 7 | 0 | 1 |

Both distributions gained `run-nquote1`, which exercises quoted and unquoted control characters. All three extended-glob drivers match on Debian. Two match on Ubuntu, where `run-extglob` reports a difference in the width of `shopt` output. A separate path filter also distinguishes the references. With `extglob` enabled, `GLOBIGNORE='+([^[:alnum:]])'` excludes `./++` from `./*` on Bash 5.2 and retains it on Bash 5.3. Tonari follows 5.2 in this case. The job-control driver finishes on both distributions and still reports output differences. [GNU driver results and first differences](compatibility/gnu.tsv)

引用の有無と制御文字の扱いを調べる `run-nquote1` が両環境で新たに一致しました。ジョブ制御のグループは両環境で最後まで走りましたが、出力差が残っています。個別テストで確認した `GLOBIGNORE` の挙動は Debian と Ubuntu の Bash でも異なり、句読点だけのファイル `./++` を対象とする例では Debian の Bash 5.2 に合わせました。表の「判定不能」は配布版 Bash 自身の結果を正解ファイルと照合できなかったグループです。[GNU の実測結果](compatibility/gnu.tsv)

<a id="intentional-differences"></a>

## Intentional differences / Tonari が採用する動作

Bash defines the expected syntax and tool-facing behavior for supported scripts. Tonari defines its UTF-8 text contract and can support inputs beyond Bash's capabilities. Record a concrete input, the reference outputs and the reason for a deliberate difference. Test extensions against Tonari's promised result. The matrix marks a verified intentional difference with ◎ and a violated Tonari contract with !. A new reference result is flagged for review. The release gate checks the Tonari result and keeps parity and speed counts specific to comparable behavior. [Shell tests](../tests/shell.py)

Bash 互換の基準は、対応するスクリプトとツールが期待する構文・動作です。Tonari の仕様を壊す Bash の不具合は再現せず、Bash が扱えない入力でも Tonari の仕様で正しく扱えるものはサポートします。意図した差には再現例と採用理由を残します。機能の未実装やバージョン間の仕様差は別途記録します。一覧の ◎ は Tonari の期待値を実測して確認した差、! は Tonari 自身の仕様違反です。参照先が未知の結果を返した場合も再確認できるように記録します。

| Input / 入力 | Reference Bash / 参照結果 | Tonari / 採用する動作 |
|---|---|---|
| macOS `LC_ALL=en_US.UTF-8`, only `a🌱` and `🌱a`, expand `*` | Bash 5.2.37 yields `🌱a`, then `a🌱`. Darwin collates them equally. | `a🌱`, then `🌱a`. Equal collation weights fall back to unsigned UTF-8 byte order. |
| `IFS=' 木'; v='a 木b'; set -- $v` in UTF-8 | macOS 5.2.37 yields `a`, two empty arguments, then `b`. Debian 5.2.37 and Ubuntu 5.3.9 prefix `b` with bytes `9c a8`. | Two arguments, `a` and `b`. Consume the entire UTF-8 separator. 区切り文字を途中で分断しません。 |
| Linux, `touch z A é` in an empty directory, then `LC_ALL=en_US.UTF-8` and `printf '<%s>' *` | Debian and Ubuntu Bash print `<A><é><z>`. | `<A><z><é>`. Linux passes UTF-8 names in byte order in every locale. Scripts using the first match or iterating over all arguments observe this ordering. macOS retains locale collation. Linux の引数順は配布元の照合表に依存しません。 |
| `v=雪🌸` under `LC_ALL=C` | `${#v}` is 7 bytes. `${v:1:1}` yields byte `9b`. | Length 2 and substring `🌸`. Positions in shell text count Unicode scalar values across these locales. 文字単位の長さと切り出しを維持します。 |
| A `printf` command with raw byte `ff` inside single quotes, supplied through `-c`, a file or stdin in the C locale | The command runs and writes `ff`. | Reject the invalid command text with status 2 and a UTF-8 diagnostic. 不正な入力単位を実行しません。 |
| `cat <input.bin \| cat >output.bin` with all 256 byte values | The output file equals the input. | The output file equals the input. UTF-8 validation applies to shell text. パイプやリダイレクトでバイナリをそのまま転送できます。 |

Tonari ships a single native executable with built-in editing. `enable -n echo` and `enable echo` change builtin lookup in a running shell. Loading or unloading native GNU Bash builtin modules through `enable -f/-d` is outside Tonari’s design. `help` prints Tonari’s own descriptions and supports builtin synopses. `enable -f/-d` で GNU Bash の共有ライブラリを読み込む機能は対象外です。組み込みの切り替えと実装済みのキー設定はシェル単体で使えます。

The C locale byte result is expected Bash behavior. The UTF-8 `IFS` example exhibits different outcomes even among the reference builds. Tonari keeps valid text intact in both cases. [Encoding observations](engineering-notes.md)

`LC_ALL=C` でバイトを数えるのは Bash の仕様です。一方、UTF-8 の `IFS` の例では参照実装同士でも結果が分かれます。どちらも Tonari では文字の境界を守る理由があり、その期待値を回帰テストにしています。[開発知見](engineering-notes.md)

## How to read the evidence / 判定の読み方

- Debian 13 arm64 uses Bash 5.2.37. Ubuntu 26.04 arm64 uses Bash 5.3.9 with the GNU 5.3 release test files. macOS arm64 uses Bash 5.2.37 for additional native comparisons. The two Linux runs include the upstream regression groups. The macOS run covers the direct feature and terminal probes.
- Builtins, their short-option usage, reserved words, shell options, signal names, Readline commands and Readline variables come from the reference executable. Manual sections, documented variables, compatibility levels and numbered POSIX-mode rules come from the matching GNU sources. The other rows name concrete language operations, invocation forms, prompt escapes, and terminal workflows. [GNU Bash manual](https://www.gnu.org/software/bash/manual/bash.html) · [GNU Bash releases](https://ftp.gnu.org/gnu/bash/)
- A direct script match requires equal stdout bytes, stderr bytes and exit status after replacing the test directory and executable path with fixed labels. PTY tests check the displayed prompt, captured input/completion fields, terminal process group or returned status. Date and time prompts use the documented display format. Network redirection probes check received bytes over loopback.
- Each GNU driver runs once against the distribution shell and once against Tonari using the same executable pathname, fixtures, locale and controlling terminal. A golden match requires a successful driver, a nonempty sequence of successful direct golden-file comparisons, and no reported skips on either side. A failing, skipped or unobservable control remains inconclusive. The ordinary exported-function behavior has its own direct probe. The aggregate, profiling and exported-function drivers are outside the golden totals.
- A matching observation applies to the named case. A working `trap EXIT` can coexist with a failing `trap DEBUG`. Each named Readline command is queried independently. Some names share a command-list or key-lookup witness. A rejected `--posix` leaves the manual’s numbered POSIX rules blocked at entry. A counted name can share a witness with other names. The 132 manual sections form an index. These dependencies make a single percentage a poor measure of how much of Bash works. Unicode tests cover the project's UTF-8 contract. Combinations of features, external plugins, other CPUs and WSL2 require their own evidence.
- 直接の比較では標準出力、標準エラー、終了値を照合します。対話機能は疑似端末から実際にキーを送り、補完コールバックの値や前景プロセスグループも調べます。GNU の回帰テストは配布版 Bash 自身でも実行します。正解ファイルと照合できなかったグループには「判定不能」と明記しました。
- 組み込みやオプションの一致はその検証例が通ったという意味です。たとえば `trap EXIT` が通っても `trap DEBUG` には動作差があります。Readline の各キー操作で、割り当ての照会と実行時の編集動作を区別します。件数には別名や同じ操作を参照する項目も含まれ、全組み合わせを網羅するものではありません。

## What the timing measures / 速度の測り方

Each candidate measurement checks its result again. Changed observations have a visible error and no timing credit. Each shell uses a newly prepared directory and clean environment. In the default run the harness alternates shell order, discards two warmups and uses 15 samples per executable, or seven for GNU drivers. Median and nearest-rank p95 are stored in nanoseconds. Both binaries are staged on the same filesystem. [Feature benchmark](../tests/compat/bench.py)

- **Whole** measures launching the shell, running the case, consuming its output and waiting for termination. PTY and GNU rows time a complete checked dialogue or upstream driver, including its helpers. Fixture creation and copying the GNU tree happen outside the driver timer.
- **Started** measures canonical `-c` cases from a readiness handshake to exit. This includes waking the shell, running the case, reading its output and reaping it. Each sample must still produce the original bytes and status. A numeric ratio of 0.50 means Tonari took half the Bash time. The upper target uses the exact fraction 2/3 before rounding.
- Short one-shot measurements include fixed launcher and scheduler costs. Probes containing `sleep`, a PTY exchange, file creation or an external utility include that wall time. For per-operation throughput, the script suite repeats work in one shell and verifies the result. The editor suite measures keys to visible PTY output and checks the completed line or displayed candidates. [Script workloads](../tools/bench_scripts.py) · [Keyboard workloads](../tools/bench_completion.py)
- 所要時間は Tonari÷Bash です。毎回結果を照合し、起動込みと起動済みを分けます。後者にも同期・出力回収・終了待ちの時間が入ります。入力1文字の遅延は専用の端末ベンチ、展開やループは反復スクリプトでも確認します。外部プログラムや意図的な待ち時間を含む項目も実測値を残します。未実装・動作不一致の項目は互換動作の速度を算出できるまで一覧にその状態を表示します。

## Reproduce on Linux / Linux での再現

Build a Linux Tonari release for the machine running Docker. The image below supplies the distribution's Bash, locales, GNU files and its four test helpers. Set `linux_binary` to the absolute path of the Linux executable. Build each fixture once and run the two distributions sequentially. The container init process reaps children from background and disown tests throughout the timing run. Without it the cgroup can exhaust its PID allowance and even the reference shell can fail to fork. The benchmark record identifies the exact release binaries used for the published measurements. [Release build instructions](benchmarks.md)

```sh
linux_binary=/absolute/path/to/linux/tonari
run_lane() {
    label=$1 base=$2 version=$3 digest=$4
    fixture=$(mktemp -d)
    curl -fL "https://ftp.gnu.org/gnu/bash/bash-$version.tar.gz" -o "$fixture/bash-src.tar.gz" &&
    docker build -f tests/compat/Dockerfile --build-arg BASE="$base" \
        --build-arg GNU_SHA256="$digest" -t "tonari-audit-$label" "$fixture" &&
    docker run --rm --init --network none --pids-limit 128 --tmpfs /tmp:exec,mode=1777,size=384m \
        -v "$PWD:/repo:ro" -v "$linux_binary:/tonari:ro" "tonari-audit-$label" --correctness-only > "$label.audit.json"
}
run_lane debian debian:trixie-slim 5.2.37 9599b22ecd1d5787ad7d3b7bf0c59f312b3396d1e281175dd1f8a4014da621ff
run_lane ubuntu ubuntu:26.04 5.3 0d5cd86965f869a26cf64f4b71be7b96f90a3ba8b3d74e27e8e9d9d5550f31ba
```

On macOS use a native modern Bash and the unpacked GNU source tree matching its feature release.

```sh
python3 tests/compat/audit.py --bash "$reference_bash" --tonari target/release/tonari \
    --source "$gnu_source" > audit-mac.json
python3 tests/compat/bench.py --bash "$reference_bash" --tonari target/release/tonari \
    --audit audit-mac.json > mac.json
```

To time a verified build, mount its saved JSON into the same fixture and pass `--reuse-audit`. The benchmark compares the SHA-256 of both staged executables with the audit and checks each sample against its recorded result.

バックグラウンドジョブを繰り返し実行するため、コンテナは `--init` で起動して親を離れた子プロセスを回収します。修正中は上の `--correctness-only` で機能を再確認します。速度を計測する段階で同じ実行ファイルと結果ファイルを指定します。計測器がどちらかの実行ファイルの変更を検出すると、速度の集計を始める前に終了します。

```sh
label=debian
docker run --rm --init --network none --pids-limit 128 --tmpfs /tmp:exec,mode=1777,size=384m \
    -v "$PWD:/repo:ro" -v "$linux_binary:/tonari:ro" -v "$PWD/$label.audit.json:/checked.json:ro" \
    "tonari-audit-$label" --reuse-audit /checked.json > "$label.json"
```

## Check a release / リリース時の検証

```sh
python3 tests/compat/changes.py --baseline docs/compatibility --inventory current.tsv --gnu gnu.tsv \
    Debian=debian.audit.json Ubuntu=ubuntu.audit.json macOS=audit-mac.json > changes.tsv
python3 tests/compat/report.py --compare docs/compatibility-2026-11-23 \
    Debian=debian.json Ubuntu=ubuntu.json macOS=mac.json
python3 tests/compat/report.py --require-target --require-complete \
    Debian=debian.json Ubuntu=ubuntu.json macOS=mac.json
python3 tests/compat/report.py --output new-report \
    Debian=debian.json Ubuntu=ubuntu.json macOS=mac.json
python3 tools/bench_scripts.py target/release/tonari --bash "$reference_bash" --require-target
python3 tools/bench.py target/release/tonari --bash "$reference_bash" --require-target
python3 tools/bench_completion.py target/release/tonari --bash "$reference_bash" --listing --require-target
```

The comparison exits nonzero if a name disappears, a matching observation or timing is lost, or a previously qualifying median crosses 2/3. The optional `--require-target` check reports the strict 2/3 ratio for every matching feature. For short operations and common OS waits, apply the parity criterion to the reported times. [Performance target](benchmarks.md#performance-target) The completeness check also fails on missing or unverified behavior. Review disputed timings with more runs on the same hardware before updating a snapshot. The `--output` operation writes a new matrix and platform TSVs.

比較ゲートは、一覧から消えた項目、以前通った動作の不一致、計測の消失、以前達成していた上限の超過を終了コードで通知します。`--require-target` は 2/3 を超えた項目をすべて報告します。短い処理や共通の OS 待ちは、実測時間から同等かどうかを判断します。`--require-complete` は未確認・未実装・動作差をすべて検出します。更新時には結果を見比べ、境界付近では同じ機械で回数を増やして確認します。
