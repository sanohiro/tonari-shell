# GNU regression cases / GNU 回帰試験の個別判定

Tonari uses the packaged GNU Bash releases as controls. A GNU driver can run several scripts in sequence and compare their combined transcripts. The result for a driver and the behavior of an individual script are recorded separately.

| Input | Observable behavior | Decision |
|---|---|---|
| `shopt -s globstar` followed by `**/a/**` in a tree with nested directories named `a` | A pathname can have more than one matching traversal. | Preserve each match. GNU 5.3's full `run-globstar` transcript agrees. |
| `history -p '!?a.h?:%'` after `cmd a.html a.h` | The selected argument is `a.h`. | Select the word at the last matching occurrence in the most recent matching entry. |
| `set -H -o history`, an unknown event on a line inside an `if`, then a valid command | Bash diagnoses the bad line. The valid command executes. Text in a here-document remains literal for history purposes. | Expand each physical input line once and keep the surrounding compound command. GNU 5.3's `run-histexpand` then differed only in the text of two diagnostics. |
| A second `local -` in the same function | Ubuntu 24.04 Bash 5.2.21 leaks an option after function return. Debian Bash 5.2.37 and Bash 5.3 restore the caller. | Keep the first snapshot. GNU 5.3 CHANGES item ccc documents this correction. |
| `readonly 'a=(3)'` with an existing indexed array | Element zero contains the three characters `(3)`. With explicit `-a`, it contains `3`. | Preserve quoting, the declaration operand and the scope. The remaining GNU 5.3 `run-attr` differences concerned diagnostic text. |
| `declare -A a=([one]=1 bad [two]=2)` | Bash 5.3 retains `one`, diagnoses the invalid member and discards the rest of the input line. Bash 5.2 diagnoses the member and continues assigning `two`. | Follow 5.3. A quoted empty key in alternating key/value syntax is diagnosed and skipped by both reference versions. |
| `declare -i x=1; y=2; declare -n x=y` | `x` refers to `y`. Removing the reference attribute exposes the literal name `y`. | Clear the previous integer conversion before binding the name. Reject an invalid new target without altering the reference. |
| `test -t x` | GNU 5.3 Bash mode reports an error and status 2. The existing Tonari `sh` contract treats an invalid descriptor as false. | Status 2 in Bash mode, 1 with empty stderr in POSIX mode. Negative or closed valid descriptor numbers are false. |
| `! false & wait "$!"` | Both reference Bash releases return 1 from `wait`. Tonari returns 0. The asynchronous list immediately returns 0 in each shell. | Preserve the inverted pipeline result in the subshell. POSIX specifies that `wait` obtains that subshell's exit status. [Shell command language §§2.9.2 and 2.9.3.1](https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html#tag_19_09_03_03) |
| `IFS=:; set -- "a b" c; unset v; printf '<%s>' ${v-$@}` | Bash joins the two positional parameters and prints `<a b c>`. Tonari prints `<a b><c>`. | Preserve the initial fields of `$@` and split each using the actual IFS. [POSIX §2.5.2](https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html#tag_19_05_02) |
| A line beginning `EOF) middle` inside a here-document in `$(...)`, followed by a line containing only `EOF` | Bash 5.2 and 5.3 terminate the substitution early and try to run `middle`. Tonari returns the whole line as data. | Preserve the valid document body. POSIX §2.7.4 requires a line containing only the delimiter. [Here-document syntax](https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html#tag_19_07_04) |

Ubuntu の GNU 配布試験では、履歴展開は診断文 2 件を除いて最終行まで一致し、再帰グロブは全文一致した。配列初期化エラーは Bash 5.2 と 5.3 で、その行の継続と代入済み要素の扱いが異なる。Tonari は 5.3 に合わせ、正常に代入した値を残す。UTF-8 以外のバイトで途中終了する `posixpat.tests` は、後半の照合要素と等価クラスを独立して実行した。そこで見つけた `space`、`tab`、`ascii` の不一致にも回帰試験を追加した。

The three POSIX decisions at the end of the table also have Tonari-only regression witnesses. Bash との差を維持した項目は、ヒアドキュメントの実データ、引数の区切り、非同期ジョブの終了値で検証する。

## Values, side effects and reference versions / 値・副作用・参照版

| Input | GNU Bash observation | Tonari guarantee |
|---|---|---|
| Assign an output descriptor to a readonly variable with `{fd}>existing` | Both controls report failure and truncate the file. | Validate the receiver first. The failure leaves the existing file contents intact. |
| Export a global, declare a local of the same name, then remove its export attribute | A child receives the hidden global value. | An unexported local shadows the global for the duration of the call, including in child processes. |
| `declare -n r; declare r=123` | Both controls reject the target and remove the declaration. | Reject the assignment and preserve the unbound reference. |
| A named coprocess with `declare -n job=ends` | Bash creates `ends_PID` when the array did not exist. An existing array produces `job_PID`. An element receiver can report success despite the invalid destination. | Store the descriptors in the resolved array and the child PID in `job_PID`. An invalid destination fails before launching a child. |
| Empty `x`, no positional arguments, `f "$x$@"` | Bash passes zero arguments. | Preserve one explicitly quoted empty scalar. Both shells pass zero for `f "$@"` and one for `f "$x""$@"`. The standard permits either count inside a single pair of quotes. [POSIX special parameters](https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html#tag_19_05_02) |
| `declare -f` for a function named `if` | Bash 5.2 prints the `function` keyword. Bash 5.3 drops it. | Emit a definition that can be read back and called. |
| `shopt -u assoc_expand_once` | Historic Bash versions can evaluate a subscript more than once in some contexts. | Both public option names share their queryable state. Evaluate the subscript once per access. |
| `${#r}` when the nameref denotes a computed indexed element | Bash 5.2 can return 0 for a nonempty value. Bash 5.3 counts characters in the selected value. | Count UTF-8 characters in the resolved element. `${#r}` for a reference to the whole array expansion counts elements. |

読み取り専用 FD、ローカル変数の環境継承、未束縛の参照とコプロセスは、終了値に加えてファイル内容、子に渡す値、子を起動したかを検査する。関数名が予約語の場合は、表示された定義を再読込して呼び出す。引数ゼロの `"$x$@"` は POSIX がどちらも許すため、Tonari は明示的に引用した空文字列を残す。

The timing table includes the first working release, the tuned release and Bash on the same native host. The sample archive keeps all three sets of measurements in the order Bash, tuned Tonari and first working Tonari. Each workload includes shell startup. OS scheduling and filesystem operations remain part of elapsed time. [Timing table](native-case-review.tsv) · [Raw samples](native-case-samples.jsonl.gz) · [Reproduction harness](../../../tests/compat/source_review.py)

## November 27 source and regression review / 11月27日のソース照合

The following behaviors have independent witnesses. Most compare output, exit status and the presence of a diagnostic. Where those observations could hide a bug, the tests also inspect the destination file, restored variable, received signal, terminal response, or history entry. [Bash witnesses and Tonari contract assertions](../../../tests/compat/source_review.py) · [Individual POSIX-mode rules](../../../tests/compat/posix_rules.py)

| GNU source / Tonari implementation | Behavior checked |
|---|---|
| `parse.y`, `subst.c` / parser, input and word expansion | Alias boundaries in substitutions and arithmetic, ANSI quotes, syntax recovery, positional field boundaries, scalar and array `@A`, filenames containing spaces or a literal asterisk. |
| `expr.c`, `arrayfunc.c`, `builtins/declare.def` / arithmetic and declarations | A plain assignment evaluates its subscript after the RHS. Compound assignments resolve it before the RHS. Both evaluate it once. Check the final index, array contents, readonly destinations, global references under a local shadow, and partial failures. |
| `execute_cmd.c`, `trap.c`, `builtins/caller.def` / traps, execution and source locations | A DEBUG handler can call a function. RETURN sees the correct function or source frame and arguments. Independent checks cover nested substitutions, process substitutions, line numbers, redirections and trace output. |
| `jobs.c`, `jobs.def`, `wait.def` / jobs and spawn | Wait for a newly stopped child, inspect current and previous job markers, resume it, then wait for its exit. On macOS the spawn operation installs the new process group before the child runs. Compare caught and ignored dispositions in the external child. |
| `bashhist.c`, `lib/readline/histfile.c`, `builtins/fc.def` / history and editor | History event words, physical input lines, comments, replay ranges and substitutions, filters, timestamped multiline records and blank lines, emacs and vi external-editor keys. Verify the edited command after it returns from the editor. |
| `shell.c`, `flags.c`, `print_cmd.c` / startup, input and CLI | `-in` parses and records the input without performing its redirections. Interactive `set -n` is ignored as in the controls. `--pretty-print` never sources BASH_ENV or executes the input. The opt-in debugger startup tests cover an installed hook and a missing hook. |
| `builtins/printf.def` / printf | Wide-character conversions count UTF-8 characters. For an out-of-range dynamic width or precision, finish the format, consume the correct arguments, preserve the result of `-v`, diagnose the error and return 1. |
| `pathexp.c`, `lib/glob/sm_loop.c` / glob, bracket and regex matchers | Pathname separators, bracket punctuation and equivalence classes, extglob literals, recursive matches, POSIX regex captures and locale selection. The GNU bracket driver loads a C builtin. Its shell-visible pathname filtering was exercised independently through GLOBIGNORE and an actual tree, including names containing literal brackets. |
| `builtins/cd.def`, `variables.c` / directories, settings and startup | Relative and physical directories past PATH_MAX, renamed directories, restoration of POSIX-only settings, completion quoting, and word boundaries. A physical directory check verifies the opened directory identity. |

個々の試験は、ジョブ番号の表示だけでなく、停止を受け取ってからの再開・終了まで、履歴なら空行を含むファイルへの書き戻しまで確認する。デバイス判定は両 Linux 環境でブロックデバイスのメタデータを調べ、通常ファイルとの違いを確認した。GNU の手引きにある POSIX モードの各項目にも独立した入力を用意した。Bash のバージョンによって結果が変わる場合は、その版で照合できる試験と Tonari 固有の期待値を併用する。

### Decisions visible to users / 利用時に現れる差

- Bash 5.2 discards blank lines inside a timestamped history entry. Bash 5.3 and Tonari preserve them after the first nonempty line. This matters when recalling a here-document.
- Associative keys can contain an apostrophe or a closing bracket. With `assoc_expand_once` enabled, `read`, `printf -v` and `unset` accept the expanded key as data. With the option disabled, an invalid destination is rejected before reading stdin or assigning a variable through `%n`. Tests check which line the next read gets and whether that variable exists.
- On `fc -s`, GNU Bash can remove an unrelated previous entry even when there is no current input entry to replace. Tonari replaces the recorded current command when present and applies HISTIGNORE and HISTCONTROL to the replay.
- Bash 5.3 lists all aliases for `alias -p`, including when additional operands are supplied. Bash 5.2 can then process those operands again. Tonari uses the display-only behavior.
- A forced interactive shell reports its selected keymap with a pipe or a minimal terminal. GNU sets the Emacs option for `-i` and clears it for `--noediting` in both cases. Tonari checks terminal capabilities when it opens the editor.
- A descriptor moved for a single builtin or compound command is restored when that command ends. This includes failure in a later redirection. A successful `exec` commits the move. In the Oils case the output file contains both `hello` and `world`. The ordinary fixture expects no output and status 1; the Bash annotation expects only `hello`. The focused test checks the actual file after both scoped and permanent moves.
- In declaration builtin arguments, an unquoted space can end an operand; a quoted space or a closing bracket inside a quoted associative key is data. The test passes through the normal parser and checks which elements were stored.
- Listing variables with `set` preserves the next value of `RANDOM`, reports the last observed dynamic scalar, and prints complete indexed and associative arrays. Listing an unread debugger argument array leaves its lazy state intact. Tests inspect pipeline status elements and reseed the generator to verify the next number independently.
- In GLOBIGNORE, a bracket expression cannot consume a directory separator. A literal slash inside a one-alternative extglob can. Tonari treats `*@(/)cd/efg` and `*/cd/efg` alike. Both GNU controls skip the slash in the former when searching after the star. The accepted filenames and the no-match result are tested explicitly.
- An editor running on a terminal receives keys such as Ctrl-R. With forced interactive input from a pipe, Tonari reads command text and reports diagnostics on stderr. GNU Readline can echo the pipe contents and interpret editing keys there. The PTY suite separately checks the real editing operations.
- Tonari prints its own help, version and diagnostics. It does not provide the GNU C loadable-builtin ABI, restricted/privileged-shell isolation, mailbox polling, gettext catalog extraction, or the entire GNU Readline command and variable catalog. The inventory retains those distinctions. UTF-8 text, Linux glob ordering and the documented data-preservation rules apply throughout.

一時的な FD 移動では、後続の書き込み先を復元する。変数一覧の表示は乱数列を進めず、配列の各要素と添字を表示する。どちらも表示の比較に加えて、次の書き込みや乱数まで確認した。

履歴本体の空行を残すこと、`fc -s` で無関係な履歴を消さないこと、パス区切りを引用した選択肢でも普通のスラッシュと同様に扱うことは、利用者のデータや入力の意味を守る判断である。GNU 固有のネイティブ拡張や全 Readline 設定など、Tonari の対象外の項目も一覧上に残してある。

The distribution reports retain the original driver verdict, exit statuses and captured diff output (up to 128 KiB per shell and driver). A difference in indentation, error wording, job identifiers, associative-key order, or enabled GNU-only facilities can affect a whole driver. The inventory's “interface only” classification describes the probe in that row. Separate behavioral tests exercise many such settings, including history, completion, options, terminal edits and traps. The external table includes every Oils case where either shell missed an ordinary or annotated expectation. It records both exits, the failed assertions, upstream dialect alternatives and decisions on shared BUG annotations. [Debian raw audit](case-review-debian.json.gz) · [Ubuntu raw audit](case-review-ubuntu.json.gz) · [GNU driver results](case-review-drivers.tsv) · [Oils case decisions](external-case-decisions.tsv) · [bensch and Oils behavior review](../external-shell/shared-failures.md)

## Distribution and external results / 配布版・外部試験の結果

| Reference | Inventory rows | Original GNU drivers | Exact golden match | Output differs | GNU control differs or skips | Separate exported-function probe |
|---|---:|---:|---:|---:|---:|---:|
| Debian 13 / 5.2.37 | 1524 | 83 | 28 | 51 | 3 | 1 |
| Ubuntu 26.04 / 5.3.9 | 1555 | 86 | 25 | 53 | 7 | 1 |

The focused Mac comparisons produced identical outcomes for 148 Bash 5.2.37 witnesses and 189 Bash 5.3.20 witnesses. The Linux 5.3.9 comparison matched 186; OS-specific signals and installed locales determine which inputs are comparable. Additional assertions check the Tonari contract directly. All 41 conditional predicates matched both packaged controls. Debian’s 61 POSIX-mode items comprised 44 exact matches, 13 diagnostic-text differences, three version differences and one intentional difference. Ubuntu’s 75 comprised 58 exact matches, 15 diagnostic-text differences and two intentional differences.

Oils ran 2,770 cases on native Linux. Its ordinary expectation and each shell-specific annotation have separate grades. The table covers every missed ordinary or annotated assertion on either executable. For the ordinary expectation, 2,377 cases passed on both, 30 passed on Tonari alone, 27 passed on Bash alone and 336 missed on both. In the 66 bensch terminal cases selected from the earlier 1,014-case comparison, both shells satisfied 64. On the other two, the fixture expected pwd to run after one Tab on the ambiguous prefix pw, or placed the cursor one word farther left than the sent Alt-B sequence. [Case decisions](external-case-decisions.tsv) · [Oils and bensch artifacts](https://github.com/sanohiro/tonari-shell/actions/runs/45871886321) · [Original external comparison](../external-shell/README.md)

GNU の完全一致数は配布試験の期待出力とのバイト単位の比較である。エラー文や関数の整形だけでもドライバー全体が不一致になる。逆に、参照 Bash も失敗した場合には、短い入力で値と副作用を確かめて判定した。POSIX モードは手引きの各項目、対話操作は実際の端末入力、言語機能は Bash の版ごとの比較を参照できる。[GNU ドライバー別の結果と確認箇所](case-review-drivers.tsv) · [個別修正の速度](../../benchmarks.md)

The full native Linux bensch replay covered 1,014 terminal interactions and 875 builtin cases at e05151a. The final runtime change selected the default keymap for forced interactive input; the final release reran the targeted 66 terminal cases and the full Oils suite. The stock builtin runner counted 567 passes, 38 failures and four skips in 609 checks on each shell.

| Full replay | Both passed | Tonari alone | Bash alone | Neither passed |
|---|---:|---:|---:|---:|
| Terminal | 983 | 28 | 0 | 3 |
| Builtins | 830 | 0 | 3 | 42 |

The third shared terminal failure is a history search with history disabled by the stock Bash profile. With history enabled, Right accepts the match for editing on both shells. Two Bash-only builtin rows reopen /dev/tty without a controlling terminal. On a PTY both produced the expected output and status in all eleven trials. The third compares whitespace in jobs output: the command starts 27 columns after the start of Running in Tonari and Bash 5.3.20, and 24 columns after it in Bash 5.2.37. The job is running in all three cases. The per-case archive identifies the original suite and its earlier verdict. [Full replay](bensch-full.tsv.gz) · [Linux workflow](https://github.com/sanohiro/tonari-shell/actions/runs/45871434322)

端末試験と組み込み試験を全件再実行した。jobs の表示幅は Bash 5.3 に合わせており、5.2 を参照すると空白3個の差が出る。/dev/tty の2件は制御端末を用意して別途11回ずつ、出力と終了状態を確認した。

The four yash POSIX shards recorded 9,310 cases passing in both executables and independently verified the POSIX.1-2024 return case. No case passed only in the reference Bash. The gate also checks for new differences from the previous Tonari release. [Four-shard POSIX and shell regression run](https://github.com/sanohiro/tonari-shell/actions/runs/45871878458)
