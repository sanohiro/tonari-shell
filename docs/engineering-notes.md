# Engineering notes / 開発知見

## 2026-11-25 · Reading long configure commands and redirecting logs / configure の複合コマンドとログ出力

An incomplete `case` in GNU `config.guess` made the reader parse 25.6 MB of growing prefixes for one 39 kB command. A quoted here-document can resemble another nested header. When lookahead overshoots, the reader now uses the first statement's parser position to locate its physical line and verifies that command on its own before executing it. A regular file can refill a short read window when a growing command crosses a buffer boundary. The Bash witness checks the next line's alias and `LINENO`. An external command can now copy an inherited log descriptor onto stdout or stderr in `posix_spawn`. The redirections still apply in order and keep private shell descriptors inaccessible. [Measurements](benchmarks.md#configure-latency) · [Compatibility tests](../tests/shell.py)

長い `case` を1行ずつ再解析していた箇所を直しました。ヒアドキュメント内の文字が入れ子の開始に見え、先読みしすぎた場合は、パーサーが返した位置から最初のコマンドの行末に戻って確認します。長いコマンドが読み込みバッファをまたぐ場合は必要な分を読み直します。ログ用にシェルが開いた fd 5 などへの出力も、外部コマンドの起動時に直接設定できます。実測ではコンパイラと生成されたテストの占める時間が大きく、スクリプトの解析やリダイレクトの改善が全体に寄与する範囲も測定できました。[測定条件と全体の結果](benchmarks.md#configure-latency)

## 2026-11-25 · What external shell workloads exposed / 外部ベンチで見つかった実装上の差

A repeated `shift` used to move every remaining positional argument. The session now consumes the front of an owning iterator and exposes its remaining slice to expansion, functions and sourced files. Repeated `a+=(word)` used to copy all existing slots. Contiguous elements now stay dense, sparse indices retain their order, and ordinary appends update the existing buffer. A standalone quoted `"${a[@]}"` can copy stored indexed values directly into argv. Empty strings, whitespace and literal wildcard characters each remain one argument. The real array script checks sorting, a sieve, a queue and string processing in the same execution. [Results](benchmarks.md#external-suites)

`shift` は残りの引数を移動せず、先頭を一つ進めます。配列の末尾追加も全要素をコピーせず、連続した添字の配列をそのまま伸ばします。単独の `"${a[@]}"` は、空文字・空白・ワイルドカードを含む各要素を一つの引数として渡せます。ソート・素数のふるい・キューをまとめて使うスクリプトでも出力を照合しました。[実測](benchmarks.md#external-suites)

When every PATH candidate is missing, an ordinary foreground command can return the lookup result immediately. The next invocation searches PATH again. Permission errors, redirections and active traps retain the execution path needed for their effects. Past 128 nested function calls the evaluator also checks stack usage against its soft limit before admitting deeper calls, up to 256. This lets the Ackermann workload finish with the expected value. [Regression tests](../tests/shell.py)

コマンド検索では、PATH の全候補が存在しないと確認できた場合、その一回の結果を引き継ぎます。次の実行では再検索するので、直前に作られた実行ファイルを発見できます。権限エラー、リダイレクト、シグナルのトラップなどは従来の実行経路で処理します。再帰関数は通常の呼び出しのコストを保ちつつ、128 段を越えた時点でスタックの使用量とソフト上限を確認し、最大 256 段まで進めます。これで外部ベンチの Ackermann 関数も Bash と同じ結果を返しました。[回帰テスト](../tests/shell.py)

GNU Hello's generated configure script is 834,518 bytes. Ordinary noninteractive script files can now look ahead for a complete compound command in their input buffer. Execution still advances one parsed command at a time so that an earlier command can affect how the next one parses. Syntax-only mode streams short complete statements. When an incomplete compound grows beyond 2 KiB in a bounded in-memory script, one parse can finish the remaining source. An unsuccessful attempt uses incremental diagnostics. [Configure timings and release checks](benchmarks.md#external-suites)

GNU Hello の `configure` では構文チェック、ヘルプ表示、キャッシュなしの構成完了を個別に測りました。構文を確認する場合はまとめて解析でき、実行する場合は直前のコマンドによる設定変更を反映します。構成後には両シェルの `config.h` を比較し、実際にビルドしてテストを走らせます。[測定方法](benchmarks.md#external-suites)

## 2026-11-25 · Everyday shell integrations / 常用ツールを組み合わせた端末

For nvm, time a fresh terminal through the first successful `node --version` and the return of the prompt. Resolve `command -v node` after that measurement to verify that the chosen version came from the test installation. On M1 the unmodified startup script launched dozens of small utilities while selecting the default version. This workload exercises both the script evaluator and pipelines that launch external programs. [Measurements and reproduction](benchmarks.md#tool-hooks)

nvm は通常どおり起動時に読み込み、最初の `node --version` が正しい版を返して次の入力ができるまでを通して測ります。計測後に Node のパスも照合します。M1 では、デフォルトの版を選ぶ間に `awk` や `sed` などの外部コマンドを繰り返し起動しており、スクリプト評価とプロセス起動の両方が実際の待ち時間に含まれます。[実測と再現方法](benchmarks.md#tool-hooks)

Test the keystroke, edit its result and execute the edited line. The previous release could load fzf and run Ctrl-T, but its Ctrl-R widget could not enumerate history. Bash clamps a negative `fc` offset to the oldest entry. Its line editor leaves the cursor at the end of a replacement if the binding supplies a non-decimal position. fzf relies on both behaviors. Read the startup journal suffix on the first history builtin, stream `fc` output in blocks, and trim large history imports in batches. An incomplete journal entry remains available to the next incremental import. The terminal test covers both a running session and 100,000 saved entries. [Terminal benchmark](../tools/bench_tool_hooks.py)

fzf は Ctrl-T が使えていても、Ctrl-R の一覧が空になり、選択後の追加入力が行頭に入ることがありました。Bash と同じく大きな負の履歴番号を先頭に丸め、入力行を置き換えた後のカーソル位置を保持します。履歴は起動時にファイルの範囲を記録し、組み込みが参照した時点で `HISTSIZE` に収まる末尾を読みます。明示的な履歴読み込みでは上限をまとめて適用し、`erasedups` の時は各行の削除順を維持します。[実測と再現方法](benchmarks.md#tool-hooks)

direnv saves the user's interrupt handler on every prompt. A subshell can list the parent's handlers until it changes a trap. Literal `$(trap -p SIGINT)` can produce the same listing in memory. Command substitutions with simple `command` and `builtin` prefixes can also reuse direct capture. VS Code's prompt delimiters are nullary shell functions that print literals. Handle that case without a fork when function tracing and the effective builtin allow it. [Bash witnesses](../tests/shell.py)

Mac の Git 補完では、ブランチを一度 Tab 補完するたびに `git config`、ローカル参照の列挙、リモート参照の列挙、`git remote` が走りました。GitHub CLI では毎回 `gh __complete iss` が走ります。別の端末で作ったブランチや設定変更を次の Tab に反映するため、各コールバックは毎回実行します。VS Code 1.144.0 の独立した拡張機能テストウィンドウでは、シェルに `libexec/tonari/bash` を指定すると統合が自動で有効になり、拡張機能から出力と終了値 17 を取得できました。`bin/tonari` でもコマンドの実行は成功し、自動統合のイベントは発生しませんでした。[統合の計測結果](benchmarks.md#tool-hooks)

## 2026-11-25 · Large initialization scripts and repeated eval / 長い初期化と eval の反復

The previous 8 KiB lookahead worked for the Starship hook, but a 22 KiB function generated by an rcfile fell back to parsing every growing prefix. The in-memory reader can now inspect the rest of the input for the end of one command. Failed scans share a budget of one extra pass over the string. Common nested headers defer attempts at inner terminators. A literal `if` inside a here-document can fool that estimate, so the reader also tries the last deferred terminator. Only the ordinary parser can accept a complete command. Reading the next command after execution preserves alias changes, parser options and the effects of commands preceding a later syntax error. The streamed file reader shares the terminator checks.

For repeated `eval`, a session keeps two recent parses whose source is at most 4 KiB each. The key includes the source, starting line, end-of-input mode and `extglob`. Active alias definitions and text containing `<` use the ordinary parser to preserve alias replacement and read-time warnings. Execution still reads current variables, functions, traps and exit status. A single literal scalar assignment can construct the usual command directly, which also helps generated strings whose value changes on every iteration. [Native measurements and reproduction](benchmarks.md#eval-rc)

以前の 8 KiB の先読みでは、`.bashrc` が生成した 22 KiB の関数を `eval` に渡すと、1 行増えるごとに先頭から解析していました。文字列入力では 1 コマンドの末尾まで先読みし、内側の `fi` などでは再解析を保留します。ヒアドキュメントの中の `if` も考慮し、最後に保留した候補もパーサーで確かめます。次のコマンドは直前の実行結果を反映してから読み取るので、途中で定義するエイリアスや、後続行が構文エラーになる前に起きる副作用も維持します。

繰り返す短い `eval` は構文木を 2 件まで再利用し、変数展開や実行は毎回行います。毎回異なる単純代入は、通常の代入と同じ構文木を直接作ります。空の `eval` に付けた一時変数が外側に残る問題と、不正なオプションをコマンドとして実行してしまう問題も Bash と照合して直しました。[測定値と再現方法](benchmarks.md#eval-rc)

Git completion exposed two misleading brace patterns. Many functions put the opening brace on its own line, and their quoted awk and sed scripts also contain braces. The shell now recognizes the separate function opener and validates candidate shell headers. In-memory input passes the original slice to the parser. Sourced regular files smaller than 128 KiB share that path after a checked positional read. Interactive foreground builtins also skip terminal-mode capture. A child process still receives its job label and terminal state when it launches.

Ubuntu’s bash-completion 2.11-8 puts a backslash and newline between `=~` and its parenthesized regular expression. Removing that continuation before reading the operand preserves its regular-expression context. The package also queries `shopt -q cdable_vars`. With the option enabled, `cd` tries CDPATH and the directory argument before looking up the variable and prints the selected variable value. A sourced syntax error identifies the file and the starting line of the command. [Compatibility witnesses](../tests/shell.py)

Git 補完には、関数名の次の行にある開始波括弧と、引用した awk や sed の波括弧の両方があります。実際のファイルを測定して両者を区別しました。Ubuntu の bash-completion は正規表現の手前をバックスラッシュで改行し、`cdable_vars` の有無も調べます。前者は改行後までパターンの文脈を保ち、後者は実際のディレクトリと CDPATH を優先したうえで変数を引きます。端末状態の取得は子プロセスを起動する経路まで遅らせ、組み込みだけで終わる初期化でも無用なシステムコールを減らしました。[測定と版の比較](benchmarks.md#eval-rc)

## 2026-11-25 · Comparing both sides of a Starship prompt / Starship の待ち時間を両シェルで測る

Starship runs two nested evaluations at startup. Tonari's file reader could recognize a complete multiline function, but the string reader used by `eval` kept parsing each growing prefix. The string reader now tests a bounded lookahead and accepts it only when the parser returns a single complete command. Top-level aliases and options therefore take effect before reading the next command. A one-argument `eval` borrows that argument and retains the calling line number. The Bash witnesses include a quoted closing brace, a here-document with closing tokens, an alias declared in a previous compound command and a later syntax error. On the local Apple M1 the four real function registrations together took about 1 µs. Parsing fell from 1.379 to 0.290 ms, compared with 0.265 ms in Bash. [Implementation and witnesses](../tests/shell.py)

Starship の初期化では `eval` の中からさらに `eval` を実行します。Tonari にはファイル読み込みで関数全体を先読みする処理がありましたが、文字列を評価する際には、物理行が増えるたびに定義の先頭からパースしていました。関数を登録する瞬間と、登録までに関数本体を読み取る時間を別々に測ったことで、この差が分かりました。先読みを採用する条件はパーサーが複合コマンド 1 個の完結を確認できることで、後続のコマンドは、先行するエイリアス定義やオプション変更の実行後に解釈します。[実装と検証](../src/input.rs)

The upstream Bash hook gets the start time through a shell function whose body consists of an external command with literal arguments. A command substitution can launch that executable directly when both the call and its body are plain commands and function scope has no observable effect. The normal interpreter handles traps, argument expansion, assignments and redirections. Measuring the actual executable and then replaying the same init and prompt output with lightweight external processes keeps the cost of argument preparation, process creation, pipes, waiting and terminal output visible in both runs. The executable used for replay exists only in the benchmark's temporary directory. [Official Bash hook](https://github.com/starship/starship/blob/v1.27.0/src/init/starship.bash) · [Release results and phase timings](benchmarks.md#starship-phases)

Enter のたびに、公式フックは開始時刻・終了時刻・プロンプトの 3 回、外部プログラムを起動します。最初の呼び出しは引数のないシェル関数を経由するため、以前の Tonari では直接呼ぶ 2 回目よりも時間がかかりました。関数がリテラル引数の外部コマンド 1 個だけを実行し、スコープやトラップに影響しない場合は、パイプに直接出力する子を起動できます。起動から終了待ちまでの合計にはシェルの仕事も含まれます。同じ表示を返す軽い子に差し替えた対照を設けると、シェルが実際に何 ms 短縮したかを検証できます。[実測と再現手順](benchmarks.md#starship-phases)

The published macOS Starship executable links AppKit even when running its time command. On this M1, linking AppKit into a program that only prints the wall clock increased its median launch-to-output-and-exit time by 2.85 ms. Both shells pay for the dependencies of the executable they launch. Tonari's own release links only libSystem. [Matched executable comparison](benchmarks.md#starship-phases)

macOS 版では、時刻を返すだけの小さな C プログラムに AppKit をリンクすると、同じ処理が 2.90 ms から 5.74 ms になりました。公開版 Starship は時刻取得でも AppKit をロードする実行ファイルを起動します。シェル側の起動準備とは別に、起動対象のリンク構成にも実測できるコストがあります。[条件と数値](benchmarks.md#starship-phases)

## 2026-11-25 · Cost of filtering a rendered prompt / 描画後の改行抽出

The upstream Bash hook uses `${PS1//[!$'\n']}` to preserve just the newlines for integrations that position a right prompt. A single bracket expression normally consumes one character. During global deletion, use the compiled matcher on each UTF-8 character and retain the remainder in place. The previous longest-substring search repeatedly tested longer spans that the pattern could never match. Expressions with explicit collating symbols, equivalence classes or case folding follow the general matcher. Inside an isolated subshell a final function call can pass the opportunity to exec through its body. Keep function scope and check traps at each statement, so a wrapper around an external time command can release its intermediate shell process. [Upstream hook](https://github.com/starship/starship/blob/v1.27.0/src/init/starship.bash)

公式フックは、プロンプトを描いた後で改行だけを取り出します。右プロンプトを表示しない場合にもこの式が通ります。角括弧が1文字にしか一致しない場合に最長の部分文字列を探していたため、長いプロンプトほど Tonari の負担が増えていました。対象文字を1回ずつ判定して削除するように変更しました。また、コマンド置換の最後にシェル関数を呼び、その末尾で外部コマンドを実行する場合、条件がそろえばサブシェル自身をそのコマンドに置き換えます。終了状態、関数のローカル変数、トラップを確認し、公式フックが時刻取得に使う関数にも適用できました。[実測と再現方法](benchmarks.md#starship-filter)

## 2026-11-25 · Running Starship's Bash integration / Starship の Bash 初期化と再描画

Starship 1.27.0's Bash hook counts jobs with `$(jobs -p)` and captures an external renderer with `"${ARGS[@]}"`. When the visible job table is empty, its PID list is already known. A literal executable with quoted stored array arguments can write into the capture pipe directly. Use the subshell when a name reference, command location, exported shell options or other expansion can observe the child context. A ready-made ANSI prompt with only `\[` and `\]` width markers takes a literal rendering path. Once the editor has painted the prompt and has an unobstructed cursor at the end, typing extends the visible line. Mid-line edits and history suggestions can still request a redraw. [Upstream Bash hook](https://github.com/starship/starship/blob/v1.27.0/src/init/starship.bash)

公式の初期化を使った状態で、プロンプトの初回表示、1文字が見えるまで、Enter 後に次のプロンプトが見えるまでを別々に測りました。ジョブがなければ PID 一覧は空と確定します。通常の配列を引数として外部コマンドの出力を受け取る場合も、中間のシェルを省けます。サブシェルの深さを参照する名前参照や、子の設定が外部から見える場合は従来のサブシェルを使います。入力中は色と改行を含むプロンプトを再送せず、候補のない行末なら追加した文字だけを端末に送ります。Git 状態、終了コード、パイプ各段の結果は公式フックが毎回受け取ります。[実機の比較と再現方法](benchmarks.md#starship)

## 2026-11-24 · Undoing edits and recalling the last argument / 一手戻す操作と引数の再利用

Appending to a long line requires only the offset and length of the inserted text to undo it. Store removed bytes for deletes and overwrites. For completion and `bind -x`, discard the common prefix and suffix at UTF-8 character boundaries before recording the inverse edit. Consecutive typing extends one entry. Completions, suggestion acceptance and pastes start their own entries. The editor keeps at most 128 entries and a 4 MiB budget for removed text, allowing one larger deletion as the newest undoable edit. Submitting or abandoning the line drops the entire stack.

前のコマンドに数千個の引数があると、最後の一つのために全トークンを構築するコストが見えてきます。通常の単語列と単純な単一引用符付き末尾は元の文字列を直接切り出し、展開やリダイレクトを含む行はシェルの字句解析器で境界を調べます。履歴から挿入する段階では展開を実行しません。取り消しでは挿入範囲と消えた文字だけを記録します。上限は128手・削除データ4 MiBで、単独でそれを超える削除も直後の一手は取り消せます。編集を終えると破棄します。[Native measurements and reproduction](benchmarks.md#edit-actions)

## 2026-11-24 · Prefix recall and directory navigation / 前方一致履歴とディレクトリ移動

Keep the initial draft and cursor for one run of arrow presses. Remember the indices visited during a nonempty prefix search and where the scan exhausted the history. A change of direction can then pick the previous hit directly. Empty-prefix navigation uses the adjacent index. Editing the line discards the filter. The editor builds the cache only while browsing. After painting a recalled command, it waits up to 1 ms for another key before scanning for a longer suggestion. This lets a terminal on the same CPU display the recalled line promptly. Directory visits store resolved absolute paths and reuse the ordinary `cd` bookkeeping after `chdir`.

前方一致で見つけた行番号をキー操作中だけ覚えておくと、離れた2件の間を往復しても再走査は不要です。候補の表示後は最大1ミリ秒だけ次の入力を待ってから追加の候補を探すと、1 CPU でも端末が先に描画できます。ディレクトリの履歴には、実際に移動した論理絶対パスを保存します。戻る・進むときはそのパスを直接 OS に渡し、通常の `cd` と同じ処理で `PWD`・`OLDPWD` とコマンド検索のキャッシュを更新します。端末から届いたエスケープ列は、現在のキーの判別に必要なバイトだけを読みます。キー列が分割されたときは従来のタイムアウトで続きの到着を待ちます。[Optimization measurements and reproduction](benchmarks.md#navigation)

## 2026-11-24 · Partial suggestions and multi-fragment history / 履歴候補の部分採用と複数語検索

Keep a reference to the record that produced the visible suggestion. Alt-F can then copy a token from that exact command without a second history lookup. Plain arguments have a short path to the next blank. Quoted arguments and substitutions use the shell lexer's source spans without evaluating the command. A history pager can paint the query before I/O, show its first hit immediately, then fill the visible page. It tracks a file offset across pages, checks for pending input between reads, and reuses the validated journal window when available. [Editor implementation](../src/editor/picker.rs) · [Benchmark](benchmarks.md#history-fields)

100万件を先にすべて復号する試作では、入力自体がすぐ見えても候補の表示が遅れました。採用版はジャーナルを末尾から読み、検索語を含み得るブロックだけでコマンド欄とエスケープを照合します。PID やディレクトリ名の一致、未完の末尾行を候補から除きます。1ページ分がそろえば読み取りを止め、さらに古い候補への移動時に再開します。通常のブロックをまたぐ長い記録も端末テストで確認しています。[履歴の実装](../src/history/filter.rs) · [実機計測](benchmarks.md#history-fields)

## 2026-11-24 · Filtering completion candidates and ranking history / 補完候補の絞り込みと履歴の頻度

The second Tab builds a snapshot of candidate names. A filter edit scans only far enough to fill the visible page. Moving down resumes that search, and pasting a query uses the existing bracketed-paste reader. At 1,024 file matches the initial list switches to the same packed byte-prefix sort used by pathname expansion. Ties compare the full UTF-8 names. The editor escapes the chosen word using the same completion options as ordinary Tab. Cancellation restores the original buffer and cursor. PTY tests check the argument the command actually receives, including spaces, emoji, and shell metacharacters. [Picker and tests](../tests/shell.py) · [Release timings](benchmarks.md#completion-picker)

履歴は引数補完を要求された時点で末尾 256 KiB を読みます。同じディレクトリで成功した同一のコマンド行をまとめてから構文を一度解析し、引数の使用回数には該当する履歴行の数を加えます。パスの存在と種類は、その1回の補完中だけ再利用します。頻度が同じときは最近の候補が先です。ファイルの候補が1,024件以上ある場合、既存のファイル名展開用ソートを使って、最初の一覧表示も短縮しました。候補一覧を開いてからは、文字入力や上下移動で履歴やディレクトリを読み直したり、Bash の補完関数を再実行したりする必要がありません。[履歴の実装](../src/history/arguments.rs) · [実機での比較](benchmarks.md#completion-picker)

## 2026-11-24 · Required extensions in pathname expansion / ファイル名展開の必須拡張子

Tonari now extracts a trailing dot and ASCII alphanumeric extension once per pathname component when matching requires case-sensitive `fnmatch`. It rejects directory entries with a different suffix before the general match. Every surviving entry still follows the same matcher, dotfile rules and final sort. The simple `*.log` matcher already checks its suffix directly. Case folding and extended pattern groups use their own matchers. The native Linux measurements confirmed the earlier M1 experiment. [Implementation](../src/pattern.rs) · [Release measurements](benchmarks.md#required-glob-extension)

通常の `?` などを含むパターンでは、拡張子が違う名前を安価に除外できます。2万件のうち5%が一致する展開と一致なしの展開で効果を確認し、Mac と Linux に共通の実装を採用しました。Bash との自動比較には、日本語と絵文字、ドットファイル、大文字小文字、引用した `?`、拡張パターン、ディレクトリ名に適用した場合を含めました。NUL 区切りのファイル名がパイプと `mapfile` を通って空白・タブ・改行ごと復元されることも回帰テストに加えました。[Bash とのテスト](../tests/shell.py) · [試作時の履歴検索比較](benchmarks.md#ripgrep-16-trials)

## 2026-11-24 · What transfers from ripgrep 16 / ripgrep 16 の実装から

At review time ripgrep 16.0.0 was the latest release. Its one subsequent commit updated the Homebrew formula. The globset implementation extracts a required extension from some patterns and checks that before applying its general matcher. A Tonari prototype did the same for the case-sensitive native matcher with a literal ASCII extension. On a local M1 it improved a 5%-matching wildcard expansion in 20,000 files. The effect depended on how many names the cheap check rejected. A simple star plus extension already has a direct matcher. [Upstream comparison](https://github.com/BurntSushi/ripgrep/compare/669cb4c755db20b26c3af3d923c042ae224c7496...4a51093f78bfbef86d89275f4916fd6b6039fe9e) · [Glob strategy selection](https://github.com/BurntSushi/ripgrep/blob/16.0.0/crates/globset/src/glob.rs#L51-L70) · [Tonari measurements](benchmarks.md#ripgrep-16-trials)

ripgrep は正規表現で一致しそうな行と、一致が確定した行を区別します。Tonari の Ctrl-R では履歴に含まれる作業ディレクトリやエスケープ表現にも検索語が現れるため、生バイトのヒットだけでは選べません。通常のコマンド欄で文字列が一致し、エスケープもなければ再照合を省けます。ファイル名の試作でも拡張子は候補の除外にだけ使い、通過した名前は従来のパターンで照合しました。[候補行の確認](https://github.com/BurntSushi/ripgrep/blob/16.0.0/crates/searcher/src/searcher/core.rs#L475-L500) · [Tonari の履歴検索](../src/history.rs)

ripgrep selects a single worker for a single file or sorted output. Its default mmap policy considers mapping at most ten explicitly identified files. The source explains that concurrent truncation can cause SIGBUS. Tonari's shared history can be rewritten by another session. Its bounded, owned buffer can be searched safely across such changes, and the underlying file is rechecked on reuse. Tonari already shares large independent completion directories between up to four workers. The editor polls for queued keys during the scan. [ripgrep thread selection and mmap policy](https://github.com/BurntSushi/ripgrep/blob/16.0.0/crates/core/flags/hiargs.rs#L172-L189) · [Mapping conditions](https://github.com/BurntSushi/ripgrep/blob/16.0.0/crates/core/flags/hiargs.rs#L239-L267) · [History window](../src/history/window.rs) · [Completion](../src/complete.rs)

ripgrep 16 の `--in0=-` はパイプ経由で NUL 区切りのファイル名を受け取ります。公式 arm64 バイナリを使い、空白と改行を含む日本語・英字のパスが、外部コマンド同士のパイプと Tonari の `mapfile -d '' -t` の両方で、Bash と同じバイト列になることを確認しました。通常のパイプでデータを走査するのは両端のプログラムです。シェルが区切りを検索するのは、`mapfile` 自身が読み手になる場合です。[16.0.0 release](https://github.com/BurntSushi/ripgrep/releases/tag/16.0.0) · [NUL-delimited input](https://github.com/BurntSushi/ripgrep/blob/16.0.0/crates/core/flags/hiargs.rs#L1269-L1297)

## 2026-11-24 · File expansion, foreground launch and record I/O / ファイル展開・外部起動・行の読み込み

A directory scan already returns names and entry types, including the information needed for most glob matches. Changing the sort alone shortened the measured named-locale expansion on macOS. For large expansions Tonari samples 128 names in byte order. When most adjacent sample pairs also follow locale order, it orders all indices by a packed byte prefix and merges sorted runs using Rust's adaptive sort. With extensive disorder it sorts directly by Darwin's wide-character collation, using up to four workers for 32,768 or more entries. One CPU takes the serial path. Both algorithms compare the full names with the same byte tie-break. A 100,000-entry expansion uses four prefix bytes and a 32-bit index. Linux can use the same packed keys directly for its UTF-8 byte order. Directory contents are reread on each expansion. [Sorting implementation](../src/pattern/order.rs) · [Rust slice sorting](https://doc.rust-lang.org/std/primitive.slice.html#method.sort_by)

Mac のロケール順と UTF-8 順が違うディレクトリでも、一部の名前は同じ順に並びます。サンプルで順序が近ければ、軽い整数キーで並べてから既存の順序を利用できるソートを使います。大小文字や異なる文字種がランダムに混ざり、サンプルの順序が遠ければ、ロケール比較を直接並列化します。最終比較には従来と同じロケールと同順位時の規則を使います。4,200 件の混在名で出力全体を Bash と比較し、連番10万件でも全件の綴りと順序を照合しました。以前の APFS 計測では名前の列挙は `readdir` が速かったため、今回は列挙後の処理を変更しました。[実測](benchmarks.md#native-glob-launch-io) · [ディレクトリ列挙の比較](https://github.com/romkatv/gitstatus/blob/master/docs/listdir.md)

A job-controlled external executable can get its process group and default job signals during POSIX spawn. On Darwin an immediately foregrounded program starts suspended, the parent consumes the initial stop, transfers the terminal and sends SIGCONT. XNU's initial stop has no signal number. A 150-launch C probe on macOS 27 saw it in every trial. An ordinary job-control wait must only see the program's subsequent states. The shell retains the fork path when it must run child setup or when SIGCONT would remain pending. A PTY witness verifies the child's group and first terminal read. [Job control](../src/jobs.rs) · [XNU spawn implementation](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_exec.c)

ジョブ制御付きの単独外部コマンドでは、OS にプロセスグループの作成とシグナル設定を任せられます。Mac は端末を先に渡してから実行を再開するため、起動直後に標準入力を読むプログラムも正しいグループで動きます。通常の非対話起動の残り時間は、同じ実行ファイルを C から直接起動する対照でも計測できます。[端末とプロセスの測定](benchmarks.md#native-glob-launch-io)

The ripgrep author points to byte-oriented buffered record iteration. The linked Rust forum discussion also describes reusing the record buffer, and the scc author measures filesystem traversal separately from content processing. Tonari's full-stream `mapfile` uses its existing memchr dependency to find delimiters in a reusable 64 KiB buffer. A record crossing a read boundary carries only its unfinished bytes forward. Limited reads and callbacks retain the existing file-offset behavior. [ripgrep 16.0.0 line buffer](https://github.com/BurntSushi/ripgrep/blob/16.0.0/crates/searcher/src/line_buffer.rs) · [Rust forum discussion](https://users.rust-lang.org/t/fast-i-o-in-rust/61714/9) · [scc author's measurements](https://boyter.org/posts/sloc-cloc-code/)

ripgrep の実装と元記事で紹介された手法は、シェルがデータ本体を読む `mapfile` で効果がありました。全件読み込みは64 KiB ごとに区切りを探し、ブロックからはみ出した行だけを一時バッファに残します。外部コマンド同士の通常のパイプではデータを両端のコマンドが直接やり取りします。[元の議論](https://www.reddit.com/r/rust/comments/sr02aj/what_makes_ripgrep_so_fast/) · [ripgrep 作者の解説](https://burntsushi.net/ripgrep/) · [バッファサイズ別の結果](benchmarks.md#native-glob-launch-io)

## 2026-11-24 · First reverse search and startup / 最初の履歴検索と起動

A ready prompt does not imply the same history state in both shells. Bash Readline already has the plain history in memory. Tonari's previous release opened its journal, decoded a recent window and searched older records when the first Ctrl-R arrived. In a fresh PTY, measure both key-to-result and launch-through-result, without giving either shell idle time. Include a failed first search, press Enter and check which command actually ran. An empty search pending ahead of a nonmatching first character previously recalled the last command. [Editor and history benchmarks](../tools/bench_editor_input.py) · [Native results](native-2026-11-24/history-parity.md)

Tonari now keeps up to 4 MiB of raw journal data. It checks for pending input between 256 KiB reads and validates file identity, size, mtime and ctime before using the bytes. A completed append under the journal lock can extend the window. The reverse search checks only candidate records and confirms the decoded command. A fully resident journal also tracks which 16 KiB pages contain non-ASCII bytes. A grouped paste or IME commit that fails as a whole can find the longest matching query prefix by binary search. A journal larger than the window remains searchable to the beginning. [Window](../src/history/window.rs) · [Search](../src/history.rs)

Bash は最初のプロンプトまでに履歴を読み込みます。以前の Tonari は読み込みの一部を最初の Ctrl-R で行っていたため、古い履歴を探すときにその待ち時間が現れました。共有履歴に追記された場合やファイルが置き換わった場合も扱いながら、最初の検索に使える生データを最大 4 MiB 保持するようにしました。起動時に移したコストも分かるよう、入力から表示までと、PTY 起動から表示までを同じ試行で測っています。日本語・ASCII・一致なしのほか、日本語が頻出して検索語の途中まで一致する履歴も検証しています。 [実測と再現条件](native-2026-11-24/history-parity.md)

An executable text file without a shebang reaches the shell only after the kernel reports ENOEXEC. For a plain foreground command in a noninteractive script, a forked child can enter the normal script interpreter with a fresh session. A changed environment, close-on-exec descriptors or shell job/trap state can require exec. Drop the descriptor used to identify the text file before examining the inherited descriptors. Keeping it open always selected exec in the first prototype. A final command may replace the interpreter itself. Timing a loop or appending a status marker hides that execution path. On the local M1, entering a fresh session in the clean process reduced a single final command from 5.121 to 2.298 ms (Bash 3.581 ms). For job-control timing, pair the original short sleeps with a child released by a pipe. This measures a real job listing and wait without making the scheduler's timer part of the requested work. [Script fallback](../src/execute/spawn.rs) · [Job-control benchmark](../tests/compat/job_latency.py)

シバンのない実行ファイルでは、条件が揃えば親を複製した子プロセス内で新しいシェル状態を作り、実行ファイルの再ロードを省けます。継承 FD・引数・SHLVL・終了値もテストしました。ジョブ制御は元の sleep を含む計測を残し、子を FIFO で解放する方法でも終了値・ジョブ一覧・disown を確認しています。 [検証結果](native-2026-11-24/history-parity.md)

When the parent is started with the basename `bash` and PATH contains only system directories, GNU Bash can set `$BASH` to the system shell. On macOS the reference parent was 5.2.37 and the child was 3.2.57. Retain the original invocation for historical comparisons. To measure a specific build starting itself, supply its absolute argv[0] and verify the child executable's identity and version before timing. The median for 16 self-invocations was 45.574 ms with Tonari and 117.142 ms with Bash 5.2.37 on the hosted M1. A local prototype caching Tonari's executable path changed the historical whole-command median from 43.426 to 43.894 ms over 101 samples. The release retains the existing lookup. [Pinned child comparison](https://github.com/sanohiro/tonari-shell/actions/runs/45341792891)

macOS の旧ベンチでは GNU Bash 5.2.37 の子として OS 付属の Bash 3.2.57 が起動していました。比較したいバイナリの絶対パスを親の argv[0] に渡し、子が同じ実行ファイルかつ同じ版であることを確認してから計測すると、実際に選択したシェルを再起動するコストを比較できます。 [元の条件と再測定](native-2026-11-24/history-parity.md)

## 2026-11-24 · Four releases against the same Bash commands / 同じコマンドによる4実装の比較

The previous audit measured 852 platform/feature observations over budget. Freeze their command, fixture, input and iteration count before optimizing. This gives 255 distinct inputs and 764 platform/workload pairs. Each of four implementations ran every pair against the same-host Bash, the previous production revision and three optimization levels. The driver rotates process order, checks the exit status, stdout and stderr of each iteration, then times the shell after startup. Re-run the full compatibility inventory separately to catch new misses. [Frozen inputs](../tests/compat/budget-2026-11-24.jsonl) · [Results and reproduction](native-2026-11-24/replay.md)

Literal redirection words and subshell or function bodies can be reused across loop iterations. File descriptors still have to be opened and closed in the order the script specifies. When an external command receives a small here-document or here-string, Tonari can fill a pipe before calling POSIX spawn. A single bounded write fits in an empty pipe. The child receives the read end. On a local M1, four such external-command batches fell from 25.5–35.9 ms to 17.8–24.6 ms. For executable text without a shebang, the ENOEXEC path can immediately spawn Tonari itself with the original arguments. [Execution](../src/execute.rs) · [Spawn actions](../src/execute/spawn.rs) · [Paired measurements](native-2026-11-24/replay-results.tar.gz)

前回目標を超えた852行は、入力・ファイル・反復数を固定すると255種類のコマンドにまとまります。今回は共通経路を変更した4つの実装とコンパイラ設定を各環境で試しました。たとえばループでは構文木の複製を減らし、小さいヒアドキュメントではパイプを準備して外部コマンドを直接起動します。OS 待ちによる例外は、同じ操作をする C 版と時間を照合できた行に限りました。独立させた変数展開が速い場合でも、利用者が実行するコマンド全体の結果を残しました。 [機能別の実測](native-2026-11-24/replay.md)

A 4 KiB initial history window found very recent matches quickly. A match 300 records back took 0.194/0.220 ms on native Linux x86-64 with 4 KiB and 0.144/0.178 ms with 16 KiB (p50/p95). Linux arm64 showed the same tradeoff. Keep the 16 KiB first window and the 240 KiB older-history scan on all three targets. Moving subshell redirections into the child and caching spawn attributes also gave mixed changes at about the scale of the sample variation. The release keeps the smaller shared implementation. 履歴の最初の読み取りを 4・16・32 KiB で変え、直近20・300・900件と10万件前の一致を比較しました。直近だけを速くすると300件前の検索が遅くなったため、CPU共通の16 KiBを維持しました。 [History trials](https://github.com/sanohiro/tonari-shell/actions/runs/45330644209) · [Spawn trials](https://github.com/sanohiro/tonari-shell/actions/runs/45329995676)

## 2026-11-24 · Repeated shell work, process ownership and history / 反復処理・子プロセス・履歴

A single `-c` timing includes the cost of reaching and leaving the interpreter. For each eligible inventory probe above the two-thirds target, the driver now tries 4, 16 or 128 iterations inside one ready shell. A preflight compares stdout, stderr and the status of **each** iteration with the original Bash observation. A probe that changes its own state and fails this check keeps its single-command timings and an explicit reason. A no-op loop of the same length measures the harness floor. For resource-heavy scripts, the native C controls read the same directories, launch the same utilities or connect the same pipe endpoints. The directory counting control omits sorting and argument storage. [Inventory driver](../tests/compat/bench.py) · [C controls](../tools/bench_spawn.c) · [Native measurements](benchmarks.md#shell-workloads)

A foreground pipeline can write a small, bounded `echo` or `printf` result to a fresh pipe in the parent before launching the consumer. The 512-byte limit fits the minimum POSIX pipe capacity. Launch the ordinary stage when redirections, expansion side effects, traps or job control require it. A background `(list) &` already has its own child. Executing the list in that child both removes a fork and makes its `BASHPID` agree with the parent's `$!`. [Execution](../src/execute.rs) · [PID, status and pipe tests](../tests/shell.py)

Large histories benefit from searching bytes before decoding records. The reverse search skips blocks without the encoded needle, finds each candidate's newline boundaries, decodes only the command field, and checks the requested prefix or substring. Metadata hits and repeated hits inside one record do not advance the history selection. The test spans multiple blocks and covers escaped characters and an empty query. Shell execution also benefits from bounded reuse. Linux's static allocator reuses up to 16 small blocks per size class on each thread. The interpreter can reuse literal builtin arguments in loops while checking the current function table and options. Plain scalar writes reuse their string capacity and check variable attributes on every write. [History search](../src/history.rs) · [Allocator tests](../tests/allocator_fork.rs) · [Bash comparisons](../tests/shell.py)

短い機能の実行時間にはシェルを終了するまでの固定費も含まれます。目標を超えた項目は、反復に適したものを起動済みシェルで4・16・128回実行し、毎回の出力・エラー・終了値が元の Bash の観測と一致することを先に確認します。変数の変更や入力の消費で同じ処理を繰り返せない項目には理由が残ります。外部コマンド、パイプ、ディレクトリ列挙には同じ処理を直接行う C 版を用意しました。C 版と同じ時間になるかは処理ごとに確かめます。

短い組み込み出力だけのパイプでは空のパイプに安全に書ける上限を設け、バックグラウンドの括弧では既に作った子プロセスを使います。履歴検索は一致の可能性がある行だけを復号します。ループ内で使い回すのは変化しない引数で、関数の再定義や変数の属性変更は次の実行に反映します。 [計測条件と結果](benchmarks.md#shell-workloads)

## 2026-11-24 · Reading and replacing function definitions / 関数再定義のコスト

On an Apple M1, sampling the previous release while it sourced a million one-command definitions put 1,066 of 1,293 samples under the file reader in the parser. The lexer grew strings for ordinary words, allocated token buffers per read and built a full command tree for every definition, including bodies replaced before their first call. A loop that repeatedly executed an already parsed definition also cloned its body. These are two different costs. The benchmark covers both, as well as a script that calls every new definition and checks all 2,000 outputs. [Benchmark and native measurements](benchmarks.md#function-redefinitions)

Tonari now shares an immutable function definition and its source location between the syntax tree, the function table and an active call. A running function can replace or unset itself and still finish with its original body and source name. For a single command containing only plain literal words, the parser validates the whole definition and retains the command's source range. The first call constructs the ordinary executable words once. Quoted wrapper arguments use the shell's existing quote parser. Aliases, redirects, compound commands and ambiguous syntax go through the full grammar at definition time. The lexer reuses small token buffers, copies runs of ordinary word bytes together and keeps a static spelling for reserved words until an argument actually needs an owned string. [Parser](../src/parser/wrapper.rs) · [Shell tests](../tests/shell.py)

手元の M1 で単純な関数定義を100万件読み込ませると、旧版ではファイル読み取り中の1,293サンプルのうち1,066件が構文解析でした。未使用のまま置き換えられる関数にも実行用の単語列を作っていました。ループ内では、解析済みの関数本体まで毎回コピーしていました。前者はリテラルだけの単純な本体を初回呼び出し時に一度だけ組み立て、後者は定義とソース位置を共有して削減しました。実行中に自身を再定義・削除するケース、定義時のエイリアス、呼び出し時の変数展開とリダイレクトを Bash と照合しています。

Interning reserved spellings reduced the median for 2,000 multiline definitions on both native Linux CPUs in the trial. Storing a pipeline's first stage inline with `smallvec` yielded a smaller Linux gain and added 16,544 bytes to the M1 trial executable. The release uses the common keyword optimization and retains the existing pipeline representation. 予約語の表現を共有する変更を採用しました。パイプラインの先頭要素をインライン化する試作は Linux での改善が小さく、M1 の実行ファイルが16,544バイト増えたため見送りました。 [Keyword trial](https://github.com/sanohiro/tonari-shell/actions/runs/45304913519) · [Pipeline trial](https://github.com/sanohiro/tonari-shell/actions/runs/45304924360)

## 2026-11-24 · Choosing platform specializations / OS・CPU ごとの実装を選ぶ基準

Use one algorithm and release optimization profile across supported CPUs when the same workload benefits. Test a proposed exception against its common implementation on native hardware, recording the native ISA, CPU model, available cores, binary size, fixture, median and tail latency. A translated binary can validate behavior. The earlier translated x86 directory benchmark ranked the implementations in the opposite order to native AMD. The Linux tests include CPU affinity restricted to one core. [Native experiments](benchmarks.md#native-ci)

対応する CPU で同じ処理が速ければ、実装と配布時の最適化設定を共有します。専用処理を残す判断には、同じデータを使ったネイティブ機での比較を添えます。CPU の機種・使えるコア数・サイズ・中央値・裾の遅延を記録します。変換実行は動作確認に使えます。[再計測の条件と結果](benchmarks.md#native-ci)

For an interruptible directory scan, the first read can determine how long a queued character waits. On the native Xeon, reading 2 KiB first and then 32 KiB reduced the 100,000-file common-prefix Tab and queued-key median from 0.174 to 0.070 ms. On Neoverse-N2 it fell from 0.246 to 0.093 ms. Changing the first read did little for the time to an unambiguous match that required the full directory. A key arriving 1 ms into the scan took 0.058 versus 0.046 ms on Intel, and 0.083 versus 0.060 ms on Arm. POSIX `readdir` gave a similarly fast first response, but traversing 12 whole branches took longer on both tested Linux architectures. [Directory reader](../src/pattern/directory.rs) · [First read A/B](https://github.com/sanohiro/tonari-shell/actions/runs/45289001999) · [Portable reader A/B](https://github.com/sanohiro/tonari-shell/actions/runs/45288830220)

Linux のディレクトリ走査は、最初の読み取りを 2 KiB、継続分を 32 KiB にしました。Intel と Arm の両方で、10万件あるディレクトリの共通接頭辞を補完して直後のキーを表示する時間が半分以下になりました。単一候補を確定するために全件を読む場合には大きな差はなく、開始 1 ms 後に届くキーは両機種で約 0.01〜0.02 ms 遅くなりました。通常のパイプとリダイレクトに使う POSIX spawn の経路からは OS 条件を取り除きました。

The portable directory reader, pipe capacity, allocator, LLVM outlining and profile-guided builds have native A/B records. A setting can trade one priority against another on the same CPU. On a local M1 the default LLVM outliner shortened the arithmetic script but extended the first-prompt median from 2.535 to 2.647 ms. Repeated insertion near the start of a 64 KiB line took 0.035 ms either way. 1バイトのパイプを32回実行した結果や、100万件の履歴検索の p95 も保存しました。OS の例外を評価するときは、関数や I/O 全体の平均だけでなく、最初の応答と入力中断を同じ実装間で比較します。[Native comparisons and underlying data](native-2026-11-24/README.md)

The same three-vCPU hosted M1 ranked parallel directory completion first in one job (7.214 versus 9.449 ms serial) and serial first in two others. The local eight-core M1 and native Linux with two, three or four CPUs benefited from workers for the same twelve-directory workload. Keep the worker decision tied to the size of the scan and the available CPU budget. [Worker counts and p95](native-2026-11-24/README.md#directory-workers-under-different-cpu-budgets--cpu-数と補完)

Mac CI で直列が速い回の結果は、同じ3 vCPU の再測定で逆転しました。2・3 CPU に制限した Linux と手元の M1 でも並列が有利でした。補完の並列化はディレクトリの規模と利用可能な CPU 数で決めます。

## 2026-11-24 · Visible text and independent directory scans / 表示範囲と並列走査

A stalled bracketed paste paints its available UTF-8 prefix after an idle interval of 8 ms. Sustained large pastes repaint at most once every 16 ms at 16 KiB checkpoints. Closing the paste inserts the full normalized payload once. The scanner preserves terminal input after the next Enter for foreground commands. A long edit keeps its full UTF-8 buffer and renders a bounded neighbourhood of the caret. Character width, control characters, hard newlines and the terminal dimensions set the display budget. An unchanged image needs only cursor positioning. If the entire buffer is killed, the editor exchanges its storage with the kill buffer. Refining a Ctrl-R search can reuse its newest hit when the longer query still matches it. A newer match would also have matched the earlier query. Repeated Ctrl-R, deletion and failed refinements retain their ordinary search paths. [Renderer](../src/editor/viewport.rs) · [Editing benchmark](../tools/bench_editing.py)

On arm64 and x86-64, independent branches of abbreviated path completion can use scoped workers. Four or more branches and a combined directory-size hint of at least 128 KiB can select up to four participants, counting the editor. Workers own directory I/O and candidate strings. The editor handles terminal input, joins the workers, then sorts and deduplicates their results. Thread creation failure leaves the remaining directories with the other participants. A one-CPU limit selects serial traversal. The x86-64 exception in `0f9ce86` came from a Rosetta measurement. On a native four-vCPU AMD runner the shared policy took 3.523 ms versus 8.687 ms for serial traversal. Both architectures use the same workload and CPU-count thresholds. Ordinary keys, single directory scans and shell startup run on the calling thread. [Completion implementation](../src/complete.rs) · [Native comparison](benchmarks.md#native-ci)

貼り付けの途中で受信が 8 ms 以上途切れた場合も、確定した UTF-8 部分を表示します。大きな貼り付けの途中は 16 KiB ごとに時刻を確認し、約 16 ms 以上経過したときだけ表示を更新します。長い行は UTF-8 の編集バッファ全体を保持し、端末へ送るのはカーソル周辺の表示だけです。行頭・行末へ飛ぶときも見えていない全行を走査する必要はありません。Ctrl-R の検索語を延ばしても直前の最新候補が一致する場合は、その候補を再利用します。階層を略したパスの補完では、arm64・x86-64 共通で大きな独立ディレクトリが複数ある場合だけ最大 4 者で走査します。11 月 24 日、Rosetta の計測を根拠に入れた x86-64 専用の直列化を撤回しました。AMD の4 vCPU でのネイティブ計測では、共通版で中央値が 8.687 ms から 3.523 ms に短縮しました。1 CPU では直列になり、通常のキー入力・単一ディレクトリの走査・シェル起動は呼び出し元のスレッドで処理します。

## 2026-11-24 · Typeahead during completion / 補完中の入力

In a 31-sample macOS arm64 trial scanning 10,001 filenames, polling for input every 256 entries gave a median 4.301 ms to the completed path and 0.115 ms to display a key arriving 1 ms into the scan. Checking the queued byte count with `FIONREAD` at the same interval gave 3.670 ms and 0.100 ms. The previous uninterrupted scan gave 3.800 ms and 2.521 ms. This preserved frequent opportunities to handle input without requesting a terminal read on an empty queue. [Scan and delayed-key benchmark](../tools/bench_completion.py)

macOS arm64 で 1 万件のファイルを走査するときは、256 件ごとに `FIONREAD` で端末の待機バイト数を調べます。同じ頻度の `poll` と比較した試作では、候補の表示までの中央値が 4.301 ms から 3.670 ms に短縮され、走査中に届いたキーも 0.100 ms で表示できました。

A foreground completion function may read from its terminal. With the editor’s `VMIN=0`, an ordinary child read can return zero before a person has time to type. Temporarily setting `VMIN=1` for an opaque callback preserves the behavior of a direct Bash witness. The editor retains input during native scans and explicitly terminal-independent callbacks. A foreground helper wait uses `pselect` with SIGCHLD, and command substitutions drain the output pipe while watching for edits. A gated PTY test releases a helper only after the Japanese input and the subsequent Backspace have appeared. It also covers a shell loop, the final arguments and the number of callback invocations. [PTY tests](../tests/shell.py) · [Measurements](benchmarks.md#completion-typeahead)

補完関数が端末から読む場合、エディタ用の `VMIN=0` を継承すると、人が入力する前に read が 0 を返し得ます。通常の Bash 補完を実行する間は `VMIN=1` に戻します。端末を使わないと明示した補完では、子の終了または入力を待ち、コマンド置換中もパイプを排出しながらキーを表示します。疑似端末の試験では、日本語の確定入力と Backspace の表示を確認してから補完を終了させ、実行順・最終引数・呼び出し回数も確認します。

An append to a long undecorated line can continue at its existing terminal cell position. This avoids sending the entire pasted line again. The editor retains its wrapping, wide-character and control-character rules. A history suggestion, a cursor edit within the line or a deletion at a wrap boundary can still require repainting. [Renderer](../src/editor.rs) · [Long-line benchmark](../tools/bench_completion.py)

長い行の末尾で候補を表示していないときは、既存の表示位置から続きを描けます。大きな貼り付けの直後でも、一文字の追加に必要な出力はその文字と必要な折り返しだけです。履歴候補の表示や行の途中の編集、折り返し位置での削除では表示範囲を再描画します。

<a id="literal-file-capture"></a>

## 2026-11-23 · Reading a file into a shell variable / ファイルのコマンド置換

For a literal `v=$(<file)`, Tonari already has the opened file's metadata. Use its size for a bounded initial allocation and read directly into the result until the kernel returns EOF. A spare byte lets an unchanged file reach EOF without reallocating its buffer. Reads retry after interruption and keep any prefix obtained before an I/O error. A single standalone assignment can take ownership of the resulting string. Multiple assignments, redirections and tracing can still observe assignment values and use the existing path. Trimming a large capture down to a few bytes releases the oversized allocation. [Implementation](../src/subshell.rs) · [Assignment handling](../src/execute.rs) · [Measurements](benchmarks.md#literal-file-capture)

空ファイルで以前は毎回 fork していました。サイズ 0 のときだけファイルシステムを確認し、APFS、HFS、ext4、XFS、Btrfs、F2FS、tmpfs、OverlayFS 上の通常ファイルを親で実際の EOF まで読みます。open 前には名前付きパイプを避け、open 後は実際に受け取った FD の種別を再確認します。仮想ファイルや未知のファイルシステムではサイズ 0 でも子で読み取ります。Linux では名前を親のプロセスで解決して `O_PATH` で保持するので、`/proc/self/stat` をシンボリックリンク経由で読んでも Bash と同じ PID が得られます。FIFO、末尾の改行、NUL、絵文字、見積もりより短い入力と長い入力を検証しました。[Bash との比較テスト](../tests/shell.py) · [サイズ別の計測](benchmarks.md#literal-file-capture)

<a id="pipeline-capacity-and-input"></a>

## 2026-11-23 · Pipeline capacity and input ownership / パイプ容量と入力の読み方

External redirection and pipeline benchmarks include child launch and waiting. Record the CPU of the host and the translator when comparing architectures. On the M1 Docker VM, a native arm64 parent observed Rosetta mapped into the running x86-64 Tonari process. Timing both builds from that same parent showed a difference even in a pipe handshake inside a ready shell. M1 上の変換実行では、シェル起動を除いた短い入出力にも時間差がありました。Intel・AMD 実機での速度は別途計測する必要があります。[External I/O and process controls](benchmarks.md#external-stream-controls) · [Apple's Linux VM documentation](https://developer.apple.com/documentation/virtualization/running-intel-binaries-in-linux-vms)

The Linux release connects external stages directly. A trace of `/bin/true | /bin/true` shows `pipe2(O_CLOEXEC)`. Increasing that pipe to 256 KiB helped a sustained transfer on native arm64. Results depended on the workload and CPU affinity. The first trial kept the kernel default pending comparison with an intermediate size. A later same-day sweep selected 128 KiB for ordinary Linux pipeline connections. With 8 CPUs available it delivered about 90% of the time saved by 256 KiB on the large two-stage transfer. The request is best-effort and preserves larger or quota-reduced defaults. Native x86-64 and WSL2 remain unmeasured. [Three-way comparison](benchmarks.md#balanced-linux-pipes) A shell cannot infer how many bytes a filter will emit from the size of its input. Linux also accounts pipe capacity per user and can reject an increase. [Release comparison](benchmarks.md#linux-pipe-capacity) · [Linux pipe capacity limits](https://man7.org/linux/man-pages/man7/pipe.7.html)

Linux では各段をつなぐパイプを作ったときだけ容量を確認します。現在の容量が 64 KiB 以上 128 KiB 未満なら 128 KiB への拡張を試みます。カーネルが拒否しても、そのパイプをそのまま使えます。短いパイプを32回繰り返す場合と大きな連続転送を別々に測って選びました。[比較結果](benchmarks.md#balanced-linux-pipes)

On macOS, leave ordinary pipe sizing to XNU. The public implementation uses the queued bytes and the incoming write size to choose a buffer, subject to kernel memory limits. On the tested M1, a downstream child saw a 64 KiB block size after a 16 KiB write and drain. Doing that work before launching every pipeline stage slowed down short pipelines, while the bulk results depended on the consumer. [Native arm64 measurements](benchmarks.md#macos-pipe-capacity) · [XNU buffer selection](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/sys_pipe.c#L948-L971)

macOS では通常の書き込みでパイプが自動的に広がります。子プロセスを起動する前にダミーデータを書いて回収する試作は、短いコマンドの繰り返しを遅くしました。空のパイプで見える `st_blksize` と、データが流れているときに実際に書き込める量は別々に測れます。[実測と採用判断](benchmarks.md#macos-pipe-capacity)

Align (`0984346c`) precomputes its launch arguments and descriptors. On macOS it uses `posix_spawn` file actions. Its `io_copy_window` lends an existing reader buffer to a synchronous writer. This applies where the runtime owns the transfer. In Tonari, unbounded `mapfile` without a callback or explicit origin already processes records in a borrowed buffer. A bounded `read` or `mapfile -n 1` on a pipe must leave the remaining bytes available to the next command. The Bash comparison covers these cases and a FIFO that opens only after the other pipeline stage starts. [Mapfile](../src/session/mapfile.rs) · [Pipe reader](../src/session/read.rs) · [Direct Bash witnesses](../tests/shell.py)

通常の `cmd <in >out` では子プロセスがファイルを直接読み書きします。入力のサイズ確認が役に立つのは、たとえば Tonari 自身がファイルを文字列として取り込む `v=$(<file)` です。Debian arm64 で5バイトの通常ファイルを追跡すると、パスの属性確認、open、同じ FD への `fstat` 2回、`lseek(SEEK_CUR)`、読み取りの順でした。開いたファイルの種別確認と Rust の `File::read_to_end` がサイズ取得を重ねています。この観測をもとに、確認済みのサイズを容量の目安として再利用し、実際の EOF まで読む実装を同日採用しました。[採用後の計測](benchmarks.md#literal-file-capture) · [ファイルのコマンド置換](../src/subshell.rs) · [パイプ容量の実測](benchmarks.md#linux-pipe-capacity)

```sh
printf 'abc\n\n' > data.txt
strace -f -yy -e trace=%stat,%fstat,openat,lseek,read ./tonari -c 'v=$(<data.txt); printf "%s" "$v"'
printf 'first\nsecond\n' | { IFS= read -r first; printf '<%s>' "$first"; cat; }
```

## 2026-11-23 · Time to the first prompt / 最初のプロンプトまでの時間

A large function in a sourced profile could outgrow the 8 KiB reader before the parser saw its closing brace. After that, each new line triggered another parse of the growing definition. For a regular sourced file, use its size to choose a read buffer between 8 and 128 KiB. Look ahead for the end of the current compound command, then execute it before parsing the next one with the current aliases and shell options. A possible closing brace inside an unfinished here-document still needs the delimiter. The PTY fixture sources Apple's Git completion script and a separate integration test completes and checks out a newly created branch. [Startup measurements](benchmarks.md#profile-startup) · [Parsing regression](../tests/shell.py) · [Git integration](../tests/compat/tool_hooks.py)

設定を読む前の端末判定を再利用し、シェルが保持する端末のファイル記述子を直接確保します。設定を読んだあとには、設定内のリダイレクトを反映するため、実際の端末を確認します。環境からシェルオプションを継承する場合は実 UID・GID と実効 UID・GID を照合します。Debian arm64 の設定なしの疑似端末で、最初のプロンプトを出すまでの `ioctl` は12回から10回、`fcntl` は5回から3回でした。大きな補完関数を持つ設定では、途中までの定義を繰り返し解析するコストのほうが目立ちました。[起動処理](../src/main.rs) · [設定ファイルの読み込み](../src/input.rs)

The read buffer is released when the source returns. After Git completion loaded and the first prompt appeared, median RSS over seven fresh sessions changed from 4,656 to 4,736 KiB on macOS arm64, and stayed at 2,780 KiB on Debian arm64. プロンプトが出た時点の常駐メモリも確認し、Mac では中央値で80 KiB増えました。

The Mach-O linker trial revisits the November 22 deployment-target experiment at the current release size. Forcing chained fixups while keeping the 11.0 minimum saved 1,104 bytes on the M1 trial binary. Its median launch-to-exit and exec-to-prompt times both increased. The distribution retains its existing fixup encoding. Apple describes the potential for page-in linking with chained fixups. [Paired timings](benchmarks.md#profile-startup) · [Apple on chained fixups](https://developer.apple.com/videos/play/wwdc2022/110362/?time=916)

同一ソースの macOS 版でアドレス解決方式だけを変え、`otool -l` で実行ファイルの形式を検査しました。各241回の比較では通常起動、プロンプト表示とも従来のリンク設定のほうが速く、配布設定を維持しました。

## 2026-11-23 · Escaped text in shared history / エスケープを含む履歴の検索

Typing `foo\ bar` exposed a full journal decode in both autosuggestions and Ctrl-R. The history writer doubles backslashes and represents a newline as `\n`, so the reader can encode the search term once and skip blocks that cannot contain it. Tabs and carriage returns can also occur literally in existing command fields. When the query contains either, the reader filters on the longest part between them. It checks the decoded command before counting a hit. A raw match can start halfway through an escape or occur in the working directory field. [Implementation](../src/history.rs) · [Latency and reproduction](benchmarks.md#escaped-history)

100万件の履歴で、スペースをバックスラッシュでエスケープしたコマンドの検索を計測しました。ブロック単位で候補を絞ったあと、コマンド欄を復元して一致を確かめます。回帰試験では、文字として入力した `\n` と実際の改行を区別し、同じ文字列が作業ディレクトリ欄や壊れた行にあっても、Ctrl-R を押した回数に応じて正しいコマンドを選びます。[端末での回帰試験](../tests/shell.py)

## 2026-11-23 · Matching filenames and GLOBIGNORE / ファイル名と除外条件の照合

Compile each pattern component once and reuse the active shell character locale while visiting entries. A pattern with literal edges and a single star can test the prefix and suffix directly. The matched gap must fit within one pathname component when a suffix follows. A trailing star in Bash's `GLOBIGNORE` can consume a slash. For the file `Sub/K.TXT`, `GLOBIGNORE='S*'` removes it from `Sub/*`, while `GLOBIGNORE='*.TXT'` keeps it. Apply the same rule when case folding is enabled. [Implementation](../src/pattern/extended.rs) · [Measurements](benchmarks.md#nocaseglob)

ディレクトリを列挙する前にパターンを準備し、各ファイルには照合だけを行います。`LC_CTYPE=C` で複雑なパターンを使う場合は、UTF-8 名をバイト単位で照合してから元の綴りを返します。大文字小文字の変換、角括弧の範囲、POSIX 文字クラス、先頭のドット、リテラル部分、複数ディレクトリ、除外条件を Bash と比較しました。macOS の同順位の名前については `strcoll("a🌱", "🌱a")` が 0 になる一例を確認し、Tonari の UTF-8 バイト順をテストに残しました。[動作例](compatibility.md#intentional-differences) · [回帰試験](../tests/shell.py)

## 2026-11-23 · What survives stripping / strip 後に残るもの

Cargo's release profile already strips symbols. Inspect the shipped ELF with `llvm-readelf -SW` and the Mach-O with `otool -l`. The Linux binary has no `.symtab` or `.debug_*` sections. The Mac binary has no `__DWARF` segment. To attribute machine code, produce a separate link map using the same compiler and release settings, with symbols retained in the inspection build. On x86-64, 95,770 of the previous 1,148,345 bytes of `.text` came from named input sections in musl's `libc.a`. The largest remaining contributions included Tonari's builtin implementations, parser and execution code. [Cargo strip](https://doc.rust-lang.org/cargo/reference/profiles.html#strip) · [Measured sizes](benchmarks.md#release-footprint)

旧 Linux x86-64 配布版の機械語のうち、静的リンクした musl に由来するのは約8.3%でした。strip 後にも、Rust のパニック時にアドレスから関数名や行番号を復元するための機械語が残ります。配布用の標準ライブラリでは `backtrace-trace-only` を指定してこの処理を外しました。発生場所を出すためのファイル名や行番号は、診断メッセージの一部として残ります。開発用の通常の Cargo ビルドでは従来どおりの標準ライブラリを使います。[配布ビルド設定](../tools/build-release.sh)

The distribution build retains panic locations and frame capture and omits in-process resolution of addresses to names and lines. To check the actual diagnostic, ignore SIGPIPE, close the reader on stdout before executing `--version` and set `RUST_BACKTRACE=full`. Both the previous and new macOS arm64 and Linux x86-64 releases printed the Rust source location and the broken-pipe error. Neither emitted stack frames on that path. Shell introspection such as `BASH_SOURCE` reports shell script locations independently. [Regression tests](../tests/shell.py)

Store the parsed function body, definition range and source filename in one reference-counted record. An active call holds its record even if a sourced script redefines and then removes the same name. The regression test checks the source reported by each call after these mutations.

関数の本体、定義範囲、定義元を1個のレコードにまとめると、定義時と呼び出し時の検索回数が減ります。呼び出し中はそのレコードを保持するので、同名の関数が別のファイルで再定義され、さらに削除されても、元の呼び出しの `BASH_SOURCE[0]` は正しいファイルを指します。環境に export する表は、親から渡された表現と削除状態を保存するため別途保持します。

For repeated definitions in sourced files, parse the current line before scanning buffered lines for a closing delimiter. Complete one-line definitions can execute immediately. An incomplete header can still use the buffered compound-command parser. Verify the cost with the `function-definitions` and `function-redefinitions` cases of `tools/bench_scripts.py --timing started`. Both cases source 2,000 definitions and check the value from the last definition. [Script benchmarks](../tools/bench_scripts.py)

source したファイルの1行が構文として完結していれば、次の閉じ括弧を探す先読みも省けます。複数行の構文だけが後続行の一括解析を使います。2,000個の異なる関数を定義する場合と、同名の関数を2,000回再定義する場合を個別に測りました。[実測値](benchmarks.md#release-footprint)

The arm64 distributions now use `opt-level=2`. Removing the symbolizer made the faster interpreter smaller than each reference Bash executable. The native-arm64 arithmetic workload justified the extra machine code, while typed-key and completion measurements were similar. On translated amd64, `opt-level=s` gave faster startup and history search. arm64 ではスクリプト処理の高速化を実測しました。x86-64 の配布設定ではサイズの差を確認できましたが、入力・起動の速度差は Rosetta 上の値で、ネイティブ性能の最適値は未確認です。[Profile comparison](benchmarks.md#release-footprint)

When comparing signed Mach-O binaries, stage each revision under a fresh inode. Overwriting an executed test copy in place produced SIGKILL on launch despite a successful `codesign --verify --strict`. Atomically replacing it with an identical fresh copy restored execution. macOS の計測用コピーを更新するときも、配布処理と同じ一時ファイル作成と rename を使うと、この問題を避けられます。[Staging script](../tools/stage.sh)

## 2026-11-23 · Login allocation and history search / ログイン時のメモリ確保と履歴検索

The static Linux runtime increased login latency in the first build. Tracing the shell process on Debian arm64 showed 101 `mmap` and 97 `munmap` calls for `--login -ic :`. Using the pure Rust `dlmalloc` crate for Rust allocations reduced this to five and one in the isolated allocator trial. The final distribution binary made five mappings and zero unmaps in a repeat on each distribution. With the previous release compiler and prebuilt standard library on arm64, this isolated change cost 10,648 bytes. Registration of the crate's fork handlers happens once, before the first allocation on its heap. The release routes blocks of 256 KiB or more to Rust’s System allocator backed by static musl. A resize crossing that boundary copies only the shared prefix and leaves the old allocation intact on failure. This released the 16 MiB variable's pages immediately on unset. A stress run forked 128 children that each allocated memory while four parent threads continuously allocated and freed it. The Linux executable still has zero shared library dependencies. Run `cargo test --locked --test allocator_fork` on a musl host. [Fork regression test](../tests/allocator_fork.rs) · [Allocator fork contract](https://docs.rs/dlmalloc/0.2.14/dlmalloc/fn.enable_alloc_after_fork.html) · [Measurements](benchmarks.md#release-latency)

Debian arm64 のログインで、親プロセスによるメモリのマップ・解除が以前の静的版では101回・97回ありました。メモリ管理だけを切り替えた試作では5回・1回、最終配布版の再検査では Debian・Ubuntu とも5回・0回でした。arm64 でコンパイラと事前ビルド済みの標準ライブラリをそろえた場合、この変更単体のサイズ増は 10,648 バイトでした。小さい割り当てを受けるヒープの初回使用前に fork ハンドラを登録します。256 KiB 以上は静的 musl に渡し、解放した大きなバッファを OS が回収できるようにしました。親の4スレッドがメモリを使い続ける間に128回 fork して、各子プロセスでも確保できることを確認しました。[計測記録](benchmarks.md#release-latency)

For old journal entries, reuse one byte-substring finder over the reverse scan. A block with no matching bytes needs no UTF-8 validation. Matching records still go through decoding, and a pending key can interrupt between reads. A regression test places a malformed record containing the search term between the reader and the valid match. [Byte search](https://docs.rs/memchr/latest/memchr/memmem/struct.Finder.html) · [PTY tests](../tests/shell.py)

古い履歴の検索では、各ブロックで同じバイト列検索器を使い回します。一致するバイト列がないブロックは読み飛ばし、一致の可能性がある行だけ UTF-8 と履歴形式を検証します。検索語を含む壊れた行が先に見つかっても、その前の正しい候補を返すことをテストしています。

Compiling the standard library with the application's LTO profile reduces the shipped size on all three targets. On x86-64-v3, enabling the standard library's smaller algorithms and frame pointers freed enough space to use `opt-level="s"` for the interpreter. The standard library is built with Cargo's `backtrace` feature. Cargo requires nightly for this distribution build, so we pin its date and retain a stable development build. [Cargo build-std](https://doc.rust-lang.org/cargo/reference/unstable.html#build-std) · [Profile comparisons](benchmarks.md#release-latency)

標準ライブラリまで同じ設定でビルドし、3ターゲットとも実行ファイルを縮小しました。x86-64-v3 では標準ライブラリにサイズ重視の実装を使い、フレームポインタを有効にすると、標準ライブラリの `backtrace` 機能をビルドしつつ、シェル本体により速い最適化設定を使えます。配布版用に nightly の日付を固定しています。[配布設定](../tools/build-release.sh)

A raw counted read without a timeout can consume up to the number of characters still requested in **bytes** from a pipe. Append a NUL-free ASCII span directly into the result. For other bytes, skip NULs and count UTF-8 leading bytes across input blocks. Finish the last character before returning the descriptor to its next reader. Tests exercise a regular file and a pipe, including NULs, emoji split across buffers and a zero-length read. [Integration test](../tests/shell.py) · [Byte-checked stream timings](benchmarks.md#release-latency)

タイムアウトを指定しない `read -r -N` では、残りの文字数と同じバイト数まではまとめて読めます。NUL を含まない ASCII は一括コピーし、それ以外はブロックをまたぐ UTF-8 の文字と NUL を区別します。実ファイルとパイプの両方で、後続のコマンドが読み取るバイトまで照合しています。

<a id="runtime-dependencies"></a>

## 2026-11-23 · Runtime dependencies and filename order / 実行時依存とファイル名の順序

Linux arm64 and x86-64-v3 now ship static musl. Rust compares the UTF-8 strings when ordering pathname matches. Adjusting the character locale can still change which names match a bracket expression. The ordered list is deterministic without loading distribution collation tables. macOS uses the locale support already in libSystem. For individual algorithms, use the Rust standard library first, then consider pure Rust crates. Native dependencies need a concrete reason, such as an OS facility or a measured advantage worth its installed cost. [Release checks and results](benchmarks.md#static-linux)

Linux の arm64 と x86-64-v3 は静的 musl に統一しました。ファイル名展開には Rust の文字列比較を使い、ロケールを変えても一致した名前を UTF-8 のバイト順で渡します。文字クラスによるマッチングには引き続き文字ロケールを使います。個別のアルゴリズムには標準ライブラリや pure Rust クレートを優先し、新しいネイティブ依存には必要な OS 機能やコストに見合う実測上の利点を求めます。[意図した互換差](compatibility.md#intentional-differences)

On the rustup aarch64 musl target, the default static entry point yields a fixed-address ELF. Selecting the bundled `rcrt1.o` allows static PIE. Rust 1.88 bundles musl 1.2.3. Its static startup cannot read packed relative relocations, even when the linker accepts the option. Inspect the linked musl archive's version and enable packing from 1.2.4 onward. Both arm64 and x86-64-v3 executed arithmetic with ordinary relocations on Rust 1.88 and packed relocations on Rust 1.96.1. [Rust 1.93 musl update](https://blog.rust-lang.org/2026/01/22/Rust-1.93.0/#update-bundled-musl-to-125) · [musl 1.2.4 release announcement](https://www.openwall.com/lists/musl/2023/05/02/1)

arm64 の rustup ターゲットでは同梱の `rcrt1.o` で静的 PIE を作れます。Rust 1.88 の musl は圧縮再配置を指定すると、リンクに成功しても起動時に失敗します。配布スクリプトは使用するアーカイブの版を調べ、対応版だけで圧縮します。Rust 1.88 と 1.96.1 の両方で、arm64・x86-64-v3 の ELF ヘッダーと起動後の算術演算を確認しました。

Tonari ships one executable per target. Direct dependencies include `unicode-width` for terminal cell widths, `memchr` for byte search, `libc` for native API declarations and, on Linux musl, `dlmalloc` for Rust allocations. The macOS release now uses `-dead_strip_dylibs`. This removes the unused `libiconv` load command and leaves only `libSystem`. The earlier speed experiment found little difference. We adopted the flag to keep the runtime dependency list small. Inspect Rust dependencies with `cargo tree --locked --edges normal` and direct shared libraries with `otool -L` on Mach-O or `readelf -d` on ELF. [Release measurements](benchmarks.md#macos-runtime-dependencies)

POSIX specifies the active locale’s collation for pathname expansion. Bash 5.3 documents the same default when `GLOBSORT` is unset or empty. A script can observe the result directly through positional parameters. The implementation may choose its sorting algorithm. [POSIX.1-2024 §2.14.3](https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html#tag_19_14_03) · [GNU Bash variables](https://www.gnu.org/software/bash/manual/html_node/Bash-Variables.html#index-GLOBSORT)

In the earlier comparison the static musl build supplied byte collation. On Debian 13 and Ubuntu 26.04 arm64, Bash and both Tonari builds expanded the same eight `*.txt` names. With `LC_ALL=C.UTF-8` all three put `B.txt` first. With `LC_ALL=ja_JP.UTF-8` Bash and the glibc build put `😺.txt` first and the static musl build put `B.txt` first. This affects arguments by position and loops whose side effects depend on iteration order. Of the named checks in the November 23 Linux inventory, only the `LC_COLLATE` result differed between these builds. [glibc observations](compatibility-2026-11-23-response/matrix.md) · [musl observations](compatibility-2026-11-23-response/musl/matrix.md)

POSIX はファイル名展開に現在のロケールの照合順序を指定しています。Bash 5.3 で `GLOBSORT` が未設定または空のときも同じです。先行ビルドではこの照合順序も比較しました。Debian・Ubuntu の8ファイルの比較では、どの版でも同じ名前が展開されました。`ja_JP.UTF-8` で `set -- *.txt` を実行すると、Bash と glibc 版の `$1` は `😺.txt`、静的 musl 版は `B.txt` です。`C.UTF-8` では3者の全件の順序が一致しました。この照合順序だけを理由に、新たな共有ライブラリを追加する必要はありません。macOS 版では未使用の `libiconv` への直接リンクを外し、配布バイナリが直接参照する共有ライブラリは `libSystem` だけになりました。[配布設定](../tools/build-release.sh) · [サイズと起動時間](benchmarks.md#macos-runtime-dependencies)

## 2026-11-23 · Input latency, directory order and byte streams / 入力応答・照合順序・ストリーム

- A fresh session can answer a recent history query from the last 16 KiB of complete journal rows. For an older literal match, scan complete blocks backwards and check new input between blocks. An oversized row or an escaped query still has a decoded streaming path. A key queued behind Ctrl-R can cancel the search before the first full scan. The million-row PTY fixture measures a recent and an old match, cancellation, a 512-byte burst and a 16 KiB paste. [PTY benchmark](../tools/bench_history.py)
- 新しいセッションでは、履歴の末尾16 KiBの完全な行から最近の候補を出せます。古い候補は前方へブロックをたどり、各ブロックの間で入力を確認します。長い1行やエスケープが必要な検索には、行を復号する経路を使います。100万件の試験では最古側の一致、検索の取り消し、連続打鍵、貼り付けを個別に測ります。[履歴と補完の計測](benchmarks.md#interactive-response)
- Collect immediately available UTF-8 text before updating the edit buffer and asking for a history hint. A binding or control ends the edit. During reverse search, try the full query first. On failure the newest command matching the longest successful prefix preserves incremental search behavior. A bracketed paste contributes one literal query, including any control characters. The editor can consume the first key after the end marker, which might accept the command. All later terminal bytes must stay available to the next reader. [PTY regressions](../tests/shell.py) · [UTF-8 measurements](benchmarks.md#editor-bulk-input)
- IME の確定文字列はカーソルの位置にまとめて挿入し、履歴候補もその文字列で更新します。未完の UTF-8 が末尾にあれば、完成した部分を先に表示します。Tab・キー割り当て・vi の操作は順序を保ちます。貼り付け終了直後の Enter で起動した外部コマンドが、同じ端末から後続データを読めることまで確認しました。[履歴・補完・貼り付けの計測](benchmarks.md#editor-bulk-input)
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
