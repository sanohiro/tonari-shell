# Bash 5.2 and 5.3 compatibility follow-up / GNU ソース照合の追試

The earlier review at `40fd68f` recorded several options that could change the result of a script and several GNU tests that stopped before reaching their later inputs. This review compared the implementations, reduced differences to separate inputs, and reran the original distribution drivers. The focused tests check output bytes, exit status and the presence of diagnostics. Additional assertions inspect files, process groups, terminal updates, accepted arguments and socket data. [Focused tests](../../../tests/compat/source_review.py) · [GNU Bash releases](https://ftp.gnu.org/gnu/bash/)

前回残った項目を、実際に Bash と Tonari へ同じ入力を渡して調べ直した。GNU テストが途中で止まった場合は、後続の入力も単独で実行した。履歴と補完ではキーを疑似端末に送り、表示の変化に加え、確定後にコマンドが受け取る引数も確認した。

## Execution and expansion / 実行と展開

| GNU implementation | Tonari behavior verified / 確認した動作 |
|---|---|
| `execute_cmd.c`, `subst.c`, `builtins/set.def` | `set -k` applies to assignments after the command name, including calls to functions defined before the option changed. Quoted assignment-like arguments stay arguments. Prefix assignments, substitutions and pipelines retain their scopes. |
| `input.c`, `redir.c`, `shell.c` | Replacing fd 0 with `exec` changes where subsequent commands come from when stdin itself supplies the script. A named script continues reading its own file. Checks cover files, pipes, read-ahead and child process input. Executable images presented as shell source produce a failure. |
| `execute_cmd.c`, `findcmd.c` | A bare missing command calls `command_not_found_handle` with its original arguments and a separate execution environment. Status propagates through pipelines, captures and background jobs. Explicit paths bypass the hook. The ordinary lookup result is reused on the miss path. |
| `parse.y`, `alias.c` | Alias replacements can open quotes, command substitutions or arithmetic expressions completed in the input. Tests cover comment boundaries, redirection operands, POSIX case patterns and nested subshells. Dollar-single-quoted strings accept braced hexadecimal bytes, including adjacent bytes making a valid UTF-8 scalar. An incomplete length operator reports an expansion error when evaluated. The backquote reader preserves escaped trailing backslashes and scans unescaped spans without allocating a second source string. |
| `subst.c`, `arrayfunc.c`, `variables.c`, `builtins/declare.def` | Scalar `[@]` slices count characters. Removing a nameref attribute and assigning in the same declaration updates the correct target. Positional indirection handles missing parameters. Clearing all indexed elements preserves the declaration at modern compatibility levels. Dynamic function and debugger variables follow the active frame. |
| `subst.c` | A failed assignment such as `${6=x}` discards the current input in the default mode. The next input runs with failure status, unless `errexit` terminates the script. Quoted `"$*"` and `"${a[*]}"` join elements using the separator in effect at that point in the word. The UTF-8 test uses 雪 as the separator even with `LC_ALL=C`. |
| `trap.c`, `builtins/common.c`, `builtins/exit.def` | A bare return from a signal action in an interrupted function uses the status at trap entry. A nested shell function uses its own status. Explicit operands win. A bare exit inside the EXIT handler preserves the original exit status. |
| `variables.c`, `subst.c`, `test.c`, `execute_cmd.c` | Focused tests cover legacy quoted-regex handling, seeded random sequences, loop control across a function boundary, array tests and unsets, and 5.2/5.3 conditional grammar. A function-local compatibility setting restores the caller's setting on return. |
| `builtins/read.def`, `builtins/printf.def`, `sig.c` | Named read destinations, readonly variables, escaped final whitespace, localized decimal input, field width, grouping and timezone formatting have direct tests. The signal tables use native signal names and numbers. Numeric locale data comes from the existing build dependency and adds no glibc runtime dependency. |

`set -k` は既に定義された関数にも反映する。`exec 0<file` は標準入力からスクリプトを読んでいる場合と、引数のファイルから読んでいる場合の両方を試した。`command_not_found_handle` では空引数、空白入り引数、環境、リダイレクト、終了値を確認した。GNU の版で挙動が変わる `read` や配列は、比較対象の Bash の版も記録している。

## The editor and history / 入力・補完・履歴

Readline word-motion and editing bindings, filename completion with case or dash/underscore folding, `bind -p` filters, and suppression of empty-command completion were reviewed in `bashline.c`, `builtins/bind.def` and the bundled Readline sources. History tests cover quick substitution at a physical newline, repeated and global substitutions, recalling the last matching word, continuing after a failed argument to `history -p`, custom event characters, error re-editing, verification before execution, loading an explicit history file and multiline entries. Quotes and here-documents can require literal newlines. Ordinary compound commands can be stored on one line.

`histverify` は履歴展開の結果を編集行へ戻し、確定するまで実行しない。`histreedit` は履歴展開が失敗した入力を編集し直せる。大文字小文字を無視したファイル補完は、候補名のバイト列を調べてから必要な文字列処理を行う。行末に置いた `^old^new` が次の行まで書き換えに巻き込む不具合を修正した。前回の置換を繰り返す `:&` や、直近で検索した語の `:%` も実データで試験した。デバッガ用引数の保存とシグナル名の一覧構築も必要時まで遅らせた。[Editor and history implementation](../../../src/history.rs) · [PTY and timing cases](../../../tests/compat/source_review.py)

## Decisions and limits / 差を許容する場合

- Bash and Tonari both expand a POSIX-mode alias inside a command substitution with a closing parenthesis in the source. GNU Bash also accepts an alias that supplies the outer closing parenthesis by itself. For example, after `alias short='echo ok )'`, GNU accepts a physical line ending in `echo $( short`. Tonari diagnoses an incomplete substitution. POSIX.1-2024 §2.6.3 explicitly leaves this case unspecified. [POSIX Shell Command Language, Command Substitution](https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html#tag_19_06_03)
- The pinned yash case `return-p.tst:148` calls a shell function from a signal trap. The function runs `true; return`. Its actual output is `trapped 0` under Tonari and Bash 5.3. The fixture and Bash 5.2 use `trapped 19`. Austin Group interpretation 1602 applies the saved status when the return ends the trap action. Here the return ends only the nested function. CI keeps the upstream failure and requires exit 0, exactly `trapped 0` on stdout and empty stderr on every trial. A return of 19, extra output, a timeout or a different diagnostic fails this independent check. [Austin Group issue 1602](https://www.austingroupbugs.net/view.php?id=1602) · [Upstream fixture](https://github.com/magicant/yash/blob/1cdb509a617894ed96a12c33b43bcd7adb6f1ff5/tests/return-p.tst#L148)
- Tonari keeps its UTF-8 text contract, bytewise Linux filename ordering and single evaluation of subscripts. It preserves the escaped final space of a `read` operand. The hexadecimal floating spellings `0x1.8p+0` and `0xcp-3` denote the same number. The generated inventory retains their different bytes.
- A trapped SIGCHLD wakes the interpreter. It reaps available children and calls the handler for the observed child statuses, even if the kernel coalesced several signals. Sending SIGCHLD without a child event produces no callback, as in the GNU control. A foreground child reaped by the handler does not trigger the same handler again. A 24-child burst and an action that starts an external command both have independent expected counts.
- Headless job control was tested with a child in a separate session. Enabling job control must still put a child in the right process group. An attempt to suppress it solely because stdin lacked a terminal broke the yash signal tests and was reverted.
- Restricted or privileged shell operation, loadable C builtins, mailbox polling, gettext message tooling and the full GNU Readline configuration surface remain outside the verified default-shell feature set. Several GNU drivers still have mixed results. One example is splitting unquoted positional parameters when the same word changes IFS twice. The full distribution records below retain the observed differences and the coverage limits.

エイリアスの末尾にしか閉じ括弧がない例については、POSIX に規定がなく、Tonari は入力の不足を報告する。UTF-8 専用、添字の一度だけの評価、端末がない場合のジョブ制御は意図して維持した。GNU の比較には診断文や書式の差と、処理結果を変える差の両方が残る。単に両シェルが失敗したことを合格の根拠にはしていない。

## Distribution results / ディストリビューション別の結果

The before column uses `ca9850b`, which added GNU output retention without changing the interpreter. The tested release is `0b223cc`. The raw audits contain the observed stdout, diagnostics, exit statuses and per-driver diff output (up to 128 KiB per driver run). A complete GNU driver can retain its original classification after many of its constituent commands are fixed. An inconclusive control means the packaged Bash did not establish the upstream expected result. [Native Linux run](https://github.com/sanohiro/tonari-shell/actions/runs/45819952396) · [Per-driver comparisons](follow-up-drivers.tsv) · [All changed inventory classifications](follow-up-changes.tsv)

| Distribution | Reference Bash | Inventory entries | Golden match | Different | Inconclusive control | Separate export probe |
|---|---|---:|---:|---:|---:|---:|
| Debian 13 | 5.2.37 | 1525 | 24 (was 22) | 55 (was 57) | 3 | 1 |
| Ubuntu 26.04 | 5.3.9 | 1556 | 22 (was 20) | 56 (was 58) | 7 | 1 |

GNU のドライバーには多数の入力があり、エラーメッセージの接頭辞ひとつでも全体が不一致になる。逆に、構文解析が途中で停止すると後続の正当な入力が検証されない。ドライバー別一覧には旧版・修正版の出力行数と両シェルの終了値を併記した。見出しだけの項目、単独の再現例がない項目、シェルの識別文字列、GNU の比較側で確定できなかった項目も元の判定のまま保存した。[Debian full audit](follow-up-debian.json.gz) · [Ubuntu full audit](follow-up-ubuntu.json.gz)

At `0b223cc`, the Apple M1 passed all 77 focused Bash 5.3.20 comparisons and all 58 applicable Bash 5.2.37 comparisons. The native Linux run passed all 76 applicable Bash 5.3.9 comparisons. The ordinary shell and terminal suite ran 297 tests on both platforms (three platform skips on the Mac). Across the four yash CI jobs, 9,310 comparison entries were scored comparable and the nested trap return passed its independent POSIX.1-2024 check. [POSIX and integration run](https://github.com/sanohiro/tonari-shell/actions/runs/45819933515)

Ubuntu の履歴展開ドライバーで出力された差分は 675 行から 154 行、パラメーター展開の `run-new-exp` は 553 行から 189 行になった。これは diff の行数で、テスト件数を表す数字ではない。

## Native timings / 実測

Each release workload was warmed up, then run in shuffled shell order. The Apple M1 ran macOS 27.2.1 with Bash 5.3.20 (15 samples per shell). The Linux x86-64 job ran a stripped, static musl PIE against Ubuntu's Bash 5.3.9 (9 samples per shell). Times for scripts include shell startup, parsing and all repetitions. The terminal rows time from sending the editing action to observing its result. CI CPU scheduling and the terminal's PTY transport contribute to those values. Ratios use elapsed time, so smaller values are faster. [All 44 workloads](follow-up-timings.tsv) · [Raw samples and comparison revisions](follow-up-samples.jsonl.gz)

The measured M1 executable occupies 1,749,024 bytes and links only libSystem. The native Linux x86-64-v3 executable occupies 2,486,872 bytes and is a stripped static PIE. Linux の単語編集は 51 µs（Bash 47 µs）、履歴を編集行へ戻す操作は 44 µs（Bash 39 µs）が中央値だった。

| Workload | Repetitions | M1 Bash ms | M1 Tonari ms | x86-64 Bash ms | x86-64 Tonari ms |
|---|---:|---:|---:|---:|---:|
| Word editing / 単語編集 | 1 | 0.083 | 0.057 | 0.047 | 0.051 |
| First case-folded file completion / 初回ファイル補完 | 1 | 111.39 | 0.684 | 0.319 | 0.317 |
| History expansion shown for verification / 展開した履歴を編集行に表示 | 1 | 0.07 | 0.039 | 0.039 | 0.044 |
| Keyword assignments / コマンド名以降の代入 | 900 | 13.702 | 6.103 | 3.445 | 1.694 |
| POSIX case alias in command substitution / エイリアス入りのコマンド置換 | 450 | 426.857 | 368.775 | 109.717 | 67.137 |
| Missing-command handler / 未検出コマンドのフック | 90 | 79.153 | 64.64 | 20.336 | 13.172 |
| External background command, wait and trap / 外部起動・待機・トラップ | 90 | 213.355 | 192.223 | 42.904 | 33.388 |
| External background command and wait / トラップなしの外部起動・待機 | 90 | 203.642 | 189.376 | 42.349 | 32.776 |
| Braced byte escapes / 波括弧付きバイトエスケープ | 1200 | 11.635 | 5.512 | 1.81 | 1.522 |

- macOS-arm64 は44項目中 44 項目で Tonari の中央値が短く、39 項目で Bash の所要時間の 2/3 以下になった。
- Linux-x86_64 は44項目中 42 項目で Tonari の中央値が短く、35 項目で Bash の所要時間の 2/3 以下になった。

The version before the final history optimization is also timed in the TSV. The sample archive retains earlier first-working implementations and follow-up passes for the editor, expansions, locale handling, command lookup and traps. The 90-process wait includes an actual operating-system process for each iteration. The POSIX case-alias workload sources and executes 450 command substitutions. Compare those elapsed times separately from pure shell operations. シェル全般の速度や全 GNU ドライバーの速度をこの44項目だけから推定していない。

Reproduce the behavioral checks on a native release with the commands below. Omit `--runs 0` to time the supported workloads. [Harness and selection options](../../../tests/compat/source_review.py)

```sh
python3 tests/compat/source_review.py "$BASH_REFERENCE" "$TONARI" --runs 0
python3 tests/compat/source_review.py "$BASH_REFERENCE" "$TONARI" --case child-trap-wait --case untrapped-wait
```
