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

