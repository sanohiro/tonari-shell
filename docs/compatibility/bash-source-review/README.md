# GNU Bash source review / GNU Bash ソース照合

This review started from the GNU Bash 5.2.37 and 5.3 source distributions. The packaged Bash was the behavioral reference on Debian 13 and Ubuntu 26.04. On macOS arm64 the controls were Bash 5.2.37 and 5.3.20. The 5.3 release archive has SHA-256 `0d5cd86965f869a26cf64f4b71be7b96f90a3ba8b3d74e27e8e9d9d5550f31ba`. [GNU Bash source distributions](https://ftp.gnu.org/gnu/bash/) · [Test programs](../../../tests/compat/source_review.py)

GNU の実装で、値がどこから来るか、どのスコープで変わるか、失敗時に何が残るかを確認した。既存の一覧でオプション名が受理されていた場合も、実際の引数、変数、出力、終了値、ファイル、受信データまで調べた。導入前の Mac の比較対象は `9dd5c8c`、最終測定に使ったソースは `40fd68f`。同じ入力を Bash の版ごとに実行している。

The PR regression workflow runs the focused suite against the runner's installed Bash. The native source-review suite additionally exercises Bash 5.3 and retains the timing samples. The Rust paths below are relative to src/. [PR regression workflow](../../../.github/workflows/posix-shell.yml) · [Native comparison](../../../tools/ci/native-bench.sh)

The follow-up implements keyword assignments and scoped compatibility behavior, exercises editor and history controls in a PTY and records updated full-driver results. 前回の残件に対する実装、許容する差の根拠、最新の試験結果は追試にまとめた。[Follow-up / 追試と判断](follow-up.md)

## Source map / 照合した実装

| GNU Bash source | Tonari source | Observable behavior checked / 確認した動作 |
|---|---|---|
| `parse.y`, `subst.c`, `execute_cmd.c` | `parser/lexer.rs`, `subshell.rs`, `expansion.rs` | Bash 5.3 `${ command; }` and `${\| command; }`. Captured output, trailing newlines, REPLY, local variables, nested grammar, function depth, return, break, exit and 256 KiB of output. |
| `builtins/source.def`, `builtins/common.c`, `builtins/set.def` | `startup.rs`, `session/functions.rs`, `session.rs` | Explicit source search path, symlink resolution with `bash_source_fullpath`, and the caller's positional parameters after a sourced file runs `set --` or `shift`. |
| `pathexp.c`, `test.c` | `pattern/order.rs`, `test_expression.rs` | Name, numeric, time, size, blocks and unsorted globs. A dangling symlink, equal metadata and nanosecond timestamp differences for `-N`. |
| `pcomplete.c`, `bashline.c`, `builtins/complete.def` | `completion_spec.rs`, `complete.rs`, `session/array.rs` | `compgen -V`, readonly and nameref destinations, empty results, group/service/signal sources, COMP_WORDBREAKS, callback arguments and actual Tab insertion. FIGNORE also applies to abbreviated directories. |
| `trap.c`, `builtins/trap.def`, `variables.c` | `traps.rs`, `subshell.rs`, `session/dynamic.rs` | `trap -P`, nested `BASH_TRAPSIG`, `BASH_MONOSECONDS`, `BASH_EXECUTION_STRING`, `HISTCMD` and `FUNCNEST`. |
| `variables.c`, `alias.c`, `hashcmd.c`, `findcmd.c` | `session/assoc.rs`, `session/array.rs`, `execute/lookup.rs`, `execute/hash.rs` | `BASH_ALIASES` and `BASH_CMDS` reflect mutations by builtins and subscripts. `GROUPS` exposes the real group IDs. `EXECIGNORE` affects PATH lookup and completion. |
| `builtins/cd.def`, `builtins/exec.def`, `builtins/shift.def`, `variables.c` | `session/options.rs`, `session.rs`, `execute.rs` | Physical directories with `set -P`, exec failure in a subshell and the parent, shift diagnostics and restoration of scoped `POSIXLY_CORRECT`. |
| `lib/readline/histexpand.c`, `builtins/declare.def`, `lib/sh/casemod.c` | `history/expand.rs`, `session/declare.rs`, `pattern.rs` | History `:e` includes the dot. `declare -c` capitalizes the first character and lowercases the remainder on assignment, including individual array elements. The ~ and ~~ expansions toggle the first character or all matching characters. |
| `redir.c`, `lib/sh/netopen.c` | `execute.rs`, `execute/named_fd.rs` | TCP and UDP redirection for builtins, external commands and a persistent descriptor. The receiver checks every payload. Numeric IPv4 and TCP hostname resolution exercise distinct paths. |
| `parse.y`, `builtins/shopt.def`, `builtins/set.def` | `parser/lexer.rs`, `input/eval.rs`, `session/options.rs` | Both comment option names control interactive parsing. A sourced script still accepts comments. Cached eval text respects the active setting. |

## Results / 結果

The inventory columns count entries and changes in their classification relative to `9dd5c8c`. Changes include newly observable interfaces. Runtime matches, shell-specific behavior and interface-only results remain separate in the downloadable table. The GNU columns count complete upstream regression drivers. [Baseline Linux run](https://github.com/sanohiro/tonari-shell/actions/runs/45789789698) · [Final Linux run](https://github.com/sanohiro/tonari-shell/actions/runs/45794524355) · [Every GNU driver and first observed difference](gnu-drivers.tsv)

| Reference | Inventory rows | Changed rows | GNU golden match | GNU output differs | GNU control inconclusive | Separate export test |
|---|---:|---:|---:|---:|---:|---:|
| macOS arm64 / 5.2.37 | 1411 | 27 | — | — | — | — |
| macOS arm64 / 5.3.20 | 1439 | 38 | — | — | — | — |
| Debian x86-64 / 5.2.37 | 1525 | 28 | 22 | 57 | 3 | 1 |
| Ubuntu x86-64 / 5.3.9 | 1556 | 39 | 20 | 58 | 7 | 1 |

今回の変更で、修正前に一致していた項目が不一致へ変わった例は4環境とも0件。GNU の大小文字変換テストでは、修正後の Tonari の出力が Debian・Ubuntu ともに GNU 配布の期待値と一致した。Ubuntu のテストには、単一の変数属性に加えて文字列中の大小文字を反転させる展開も含まれる。対象機能の短い試験は Bash 5.3 で 34 件、5.2 で 19 件を照合し、補完の実入力と TCP・UDP の受信結果を別途確認した。

## Reading the inventory / 一覧の読み方

[Complete per-reference inventory](inventory.tsv.gz) · [Observed differences and GNU driver excerpts](differences.jsonl.gz) · [Timings and reproduction](../../benchmarks.md#gnu-source-review)

The inventory includes independent runtime comparisons, checks that only exercise an interface, manual section headings and POSIX items that still lack independent probes. A GNU driver can fail its output comparison on a diagnostic prefix, whitespace, or a behavior difference. A driver may contain hundreds of assertions and stop at its first syntax error. Counting each driver as one missing feature would overstate some differences and hide others. Use the recorded output and the smaller scripts to investigate.

一覧には、動作の一致、インターフェースだけの確認、未検証、意図した差、GNU の期待出力との差を個別に記録した。例えば `jobs -n` の記録ではジョブ番号・状態・コマンドが一致し、表示に使う空白数が違う。一方、旧版の動作を選ぶ `BASH_COMPAT` や `set -k` の不足は実行結果を変える。例えば `set -k; set -- keep NAME=value; echo "$#"` は Bash で 1 を出し、当時の Tonari では 2 だった。追試後は両方とも 1 になる。これらを同じ「合格」にまとめることはできない。

### Differences at `40fd68f` / 追試前の差

- GNU Readline key functions and inputrc variables extend beyond Tonari's editor. The new PTY tests cover programmable word boundaries for punctuation, consecutive delimiters, custom settings, a trailing delimiter, quotes, escaping and hostname prefixes. They check the callback arguments and the command subsequently executed.
- `set -k` / `set -o keyword` and legacy compatibility levels had observable gaps in this snapshot. Restricted and privileged shell invocation remain unsupported. Tonari should not be used to confine users or to run privileged scripts.
- The GNU array driver deliberately includes invalid array syntax. In the ordinary noninteractive mode Bash can print a diagnostic, skip the line and resume. At this revision Tonari stopped at that syntax error. This also prevents the remainder of that driver from proving coverage of later valid array cases. The separate array and associative-array witnesses still run.
- The shell identity in help, version output and the version prompt escapes differs. Tonari's UTF-8 text contract and Linux bytewise pathname collation are documented design decisions. These entries have different implications from a missing script interface. [Compatibility policy](../../compatibility.md)

この時点で残っていた代入引数や Readline 個別設定への対応は追試で更新した。制限シェルや特権モードも用途として保証していない。構文エラーからの復帰や GNU テストの制御側の失敗も記録しており、ソース全体についての完全互換や全テスト合格は宣言していない。

```sh
python3 tests/compat/audit.py --source "$GNU_BASH_SOURCE" --bash "$BASH_REFERENCE" --tonari "$TONARI"
python3 tests/compat/source_review.py "$BASH_REFERENCE" "$TONARI" --runs 9
python3 tests/compat/source_review.py "$BASH_REFERENCE" "$TONARI" --first "$FIRST_WORKING" --case declared-case
```
