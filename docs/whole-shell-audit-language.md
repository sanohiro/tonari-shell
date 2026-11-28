# Language execution and pathname expansion / 言語実行とパス名展開

## Source paths and witnesses / 実装経路と観測

The review started with the source at `1754509`. Input size and repetition count varied independently. The checks run complete programs and verify their output, diagnostic stream and status. The numerical filename check computes its expected order from the values of the decimal names. [Reproduction driver](../tools/ci/whole-language.py)

| Shell operation | Source path and observation | Control |
| --- | --- | --- |
| File, piped stdin and `eval` | `input::run` and `input::text` retried the accumulated source after each physical line containing a dollar sign in a multiline double quote. Ordinary parameters keep that quote open. For a nested expansion, the lexer tests whether it can consume that physical line up to a supplied closing quote. The complete input unit still supplies the AST for execution. | Equal-width literal text, nested quoted commands, braced parameters and arithmetic expansions. Aliases, backslash-sensitive input and here-documents take the existing parser path. |
| Repeated scalar append and length | `execute::assign_existing_word` reaches `Session::assign_existing_text` for a literal append. Expanded appends can reach `Session::append_plain_scalar`. Both now add the suffix's character count to an already cached length. The original full scan remains lazy for an unmeasured variable. | Append without a length query, direct repeated length, empty local binding, UTF-8 text and assignment replacing the whole scalar. |
| Scalar nameref and function scope | `word::modifier::value` and `execute::assign_existing_word` can read the current target's cached count. The name is resolved on each access. `eval::call_with_bindings` saves and restores the active variable scope, and the eval cache retains syntax between calls. | Change the target value, shadow it in a function, retarget twice-linked references, then convert the scalar to an indexed array. A separate array-element reference uses the general expansion path. |
| Conditional equality and case | `test_expression::condition` compares two stored strings directly for a quoted right operand. In `eval::run_case`, a stored arm parameter containing plain text can also be compared by reference. Each visit consults its current value and case-folding option. | Equal-length late mismatch, unequal lengths, quoted and unquoted wildcards, positional operands, a left operand that sets the right variable, first/last/missing arm, and `;;&` retesting a changed pattern. |
| Pattern, replacement and regular expression | The fixed glob can reject a differing required suffix. `test_expression::regex_search` consults the session cache by expression, case and locale, then asks the compiled regex to search. `word::replace::literal` reuses its byte finder while constructing the output. | Compare a near-end mismatch with a literal, glob, and anchored regex. Hold input at 4,096 characters and vary the number of replacements. Then hold the match count at 128 and vary the replacement length. Verify the marker count, final length and both ends of the result. |
| Glob listing and sorting | `pattern::paths` passes a literal component prefix to `directory::visit_names`, filters the live entries, then appends sorted fields. `order::append` reuses the name permutation only when the current listing matches it. Numeric sorting uses stable cached keys on this name-ordered list. Oversized positive numbers compare by significant digit length and bytes. | Ascending and descending order, leading zeroes, long names ending in a letter, signs, a new matching file after a cached listing and changing `GLOBIGNORE`. |
| Arithmetic and arrays | `arithmetic::Parser::atom` validates number tokens before applying arithmetic short-circuit rules. The inactive branch suppresses division and variable writes. Sparse indexed slices consult occupied indices and associative writes use the current binding. | A skipped invalid octal numeral, skipped division by zero, a skipped increment, a sparse slice and an associative write through a local nameref. |

行数・文字列長・分岐数と同一操作の反復回数を区別した。複数行の引用では、単純な変数展開のたびに先頭から読み直していた。ネストした展開も、その物理行だけが開いた二重引用の内側に収まるかを字句解析で確認できる。文字列長は追記分だけを加算し、名前参照ではアクセス時点の実体が持つ文字数を使用する。比較と case は参照してよい保存済み文字列を直接読み、展開に副作用がある式では通常の経路を通る。glob は毎回ディレクトリを読み、数値順の桁あふれは全桁で比較する。

## Reference decisions / GNU Bash との照合

* In Bash 5.3, a function can declare a local `BASH_REMATCH`. Matching and clearing the local capture leave the outer array intact. Two nested functions keep their captures separate. The 5.3 change record explicitly permits this and GNU's `lib/sh/shmatch.c` uses `builtin_find_indexed_array`. Tonari already produced the specified outputs. [GNU 5.3 release announcement](https://lists.gnu.org/archive/html/bug-bash/2025-04/msg00023.html)
* Both shells diagnose the invalid octal numeral in `$((0 ? 08 : 6))`. Both produce 1 for `$((1 || (2 / 0)))`. GNU's arithmetic reader validates the token before deciding whether operations in the branch execute. [Shell arithmetic](https://www.gnu.org/software/bash/manual/html_node/Shell-Arithmetic.html)
* GNU's numerical filename comparator uses `strtoimax` without distinguishing different overflows. On this filesystem it produced the same relative order for the 20- and 21-digit witness in both sort directions. The numeric values determine the order in Tonari. GNU's manual also specifies filename collation when numeric values are equal. [Compatibility witness](compatibility.md#numeric-filename-values)
* Ubuntu 24.04 Bash 5.2.21 reports 0 for `w=(雪); declare -n elt='w[0]'; echo "${#elt}"` under C.UTF-8. Bash 5.3.20 and Tonari return 1. The driver checks this result separately. Bash 5.2 also lacks `GLOBSORT`, so the driver excludes its numeric-sort times after checking the feature through `*` expansion.

GNU Bash 5.3 の変更履歴と実装では `BASH_REMATCH` をローカルにできる。関数を二重にした照合でも外側の配列を保持した。未評価の算術分岐にある無効な 8 進定数は構文エラーになる。ゼロ除算とインクリメントの副作用は実行しない。数値ファイル名の昇順と降順は実際の値で判定した。配列要素を指す名前参照の文字数と `GLOBSORT` の有無は Bash 5.2 と 5.3 を区別した。

## Native measurements / 実機測定

Apple M1, macOS 27.2.1, APFS, `en_US.UTF-8`. Both Tonari executables used the project's distribution profile, with the candidate built from the baseline and the language changes. Each signed executable was 2,129,664 bytes. GNU Bash 5.3.20 was 873,872 bytes. Each table entry is the median wall time in milliseconds from five interleaved trials after one warmup, including shell startup. The driver checks every timed sample.

```sh
python3 tools/ci/whole-language.py /path/to/before /path/to/after /path/to/bash --runs 5
```

各サイズを実行するたびに、比較するシェルの開始順をずらした。各数値は起動から終了までの時間の中央値で、行中のサイズは小・中・大の順。初回の正しい数値順実装では比較のたびに桁を調べた。数値キーを一度だけ作る実装との別の17回測定で、128ファイルを60回展開すると 9.209 → 8.932 ms、512ファイルでは 22.366 → 20.235 ms になった。GNU Bash はそれぞれ 24.002、73.225 ms。

The first correct numerical comparator inspected both names on every comparison. Computing the key once per pathname reduced 60 complete expansions from 9.209 to 8.932 ms at 128 files and from 22.366 to 20.235 ms at 512 files in a separate 17-trial comparison. Bash took 24.002 and 73.225 ms.

| Workload | Sizes | Before (ms) | After (ms) | Bash (ms) |
| --- | --- | --- | --- | --- |
| `double quote $v/file` | 192 / 384 / 768 | 7.127 / 10.047 / 21.959 | 4.552 / 5.505 / 5.717 | 11.642 / 11.158 / 11.333 |
| `double quote $v/pipe` | 192 / 384 / 768 | 6.845 / 14.391 / 25.582 | 6.248 / 7.131 / 8.650 | 12.805 / 13.228 / 14.485 |
| `double quote $v/eval` | 192 / 384 / 768 | 6.889 / 12.191 / 20.868 | 5.608 / 5.637 / 5.039 | 11.887 / 12.570 / 11.157 |
| `double quote ${v:-none}/file` | 192 / 384 / 768 | 8.300 / 18.879 / 55.656 | 5.421 / 5.415 / 5.399 | 12.485 / 12.095 / 12.869 |
| `double quote word/file` | 192 / 384 / 768 | 5.774 / 5.404 / 5.144 | 4.934 / 4.963 / 5.598 | 10.932 / 13.431 / 10.896 |
| `grow plus n=${#s}` | 1024 / 2048 / 4096 | 5.814 / 7.088 / 17.260 | 5.337 / 6.219 / 8.111 | 28.350 / 74.788 / 426.214 |
| `grow plus :` | 1024 / 2048 / 4096 | 5.765 / 6.376 / 9.562 | 5.070 / 7.565 / 7.063 | 16.621 / 20.050 / 27.971 |
| `length of s` | 2048 / 4096 / 8192 | 7.474 / 5.390 / 4.981 | 6.851 / 5.622 / 5.254 | 86.597 / 126.789 / 224.596 |
| `length of ref` | 2048 / 4096 / 8192 | 8.402 / 10.331 / 9.524 | 6.893 / 5.418 / 4.730 | 72.002 / 130.548 / 214.447 |
| `equality "$b"` | 16384 / 32768 / 65536 | 5.806 / 6.883 / 8.131 | 4.767 / 4.699 / 4.875 | 132.081 / 245.568 / 477.833 |
| `equality z` | 16384 / 32768 / 65536 | 4.442 / 5.697 / 4.885 | 4.676 / 4.603 / 4.648 | 128.483 / 244.357 / 476.014 |
| `numeric pathname sort` | 128 / 256 / 512 | 9.020 / 12.562 / 18.837 | 9.566 / 12.947 / 19.250 | 24.852 / 38.267 / 63.275 |
| `case static first` | 32 / 64 / 128 | 4.161 / 4.315 / 4.386 | 3.952 / 4.127 / 4.318 | 10.109 / 9.595 / 10.053 |
| `case static last` | 32 / 64 / 128 | 4.488 / 4.384 / 5.088 | 4.595 / 4.379 / 5.089 | 10.352 / 11.214 / 12.205 |
| `case static miss` | 32 / 64 / 128 | 4.349 / 4.346 / 4.646 | 4.725 / 4.053 / 4.956 | 10.609 / 11.301 / 12.573 |
| `case variable first` | 32 / 64 / 128 | 4.415 / 4.523 / 4.795 | 4.486 / 4.451 / 4.537 | 10.436 / 9.843 / 9.975 |
| `case variable last` | 32 / 64 / 128 | 5.256 / 6.095 / 7.721 | 4.716 / 4.432 / 4.918 | 10.270 / 11.124 / 12.507 |
| `case variable miss` | 32 / 64 / 128 | 5.093 / 6.068 / 7.713 | 4.505 / 4.333 / 4.826 | 10.966 / 10.704 / 12.500 |
| `near-tail miss literal` | 512 / 1024 / 2048 | 4.241 / 4.223 / 4.199 | 4.232 / 4.108 / 4.538 | 11.268 / 12.654 / 14.658 |
| `near-tail miss glob` | 512 / 1024 / 2048 | 4.475 / 4.632 / 4.368 | 4.356 / 5.030 / 4.039 | 9.754 / 11.366 / 11.530 |
| `near-tail miss regex` | 512 / 1024 / 2048 | 6.190 / 7.880 / 10.556 | 6.154 / 7.685 / 10.549 | 12.266 / 13.857 / 17.646 |
| `replace matches` | 128 / 256 / 512 | 4.772 / 4.768 / 5.173 | 4.516 / 4.705 / 4.902 | 22.694 / 28.341 / 37.972 |
| `replace replacement bytes` | 8 / 16 / 32 | 4.890 / 4.828 / 4.734 | 4.761 / 5.061 / 5.035 | 25.259 / 29.489 / 39.949 |

The two largest multiline and borrowed-string comparisons separate the avoided work from launch time. In the ordinary numeric pathname controls, the corrected sorter took 9.566, 12.947 and 19.250 ms against 9.020, 12.562 and 18.837 ms for the original sorter. Its added cost buys the verified ordering of oversized numbers. For a missing dynamic case pattern at 128 arms, direct comparison reduced the measured time from 7.713 to 4.826 ms. The fixed-pattern and regex controls continue to cover the already faster paths.

大きな複数行入力と長い文字列の反復で、不要な再解析とコピーの影響が見える。通常桁数の数値ファイル名では正しいソートの追加費用も表に示した。128分岐の変数パターンで不一致の場合は 7.713 ms から 4.826 ms になった。静的な分岐・glob・正規表現・置換も同じ手順で測定した。

## Regex misses and shared stdin, 2026-11-29 / 正規表現の不一致と標準入力

On the native Linux x86-64 runner, the original 2,048-character regex miss took 3.931 ms for the candidate and 3.606 ms for Bash. At 512 characters the same measurements were 1.287 and 1.957 ms. This reversal prompted a separate measurement of `regexec` and of the shell code surrounding it.

The static conditional borrows the stored UTF-8 subject. Up to 255 bytes, the regex adapter adds a terminator in a stack buffer. Larger subjects go into a new `CString`. A regex with up to 15 capture groups uses stack storage for the offsets. A failed match creates no captured strings and clears `BASH_REMATCH`. The raw C fixture compiles `^f+hit$` once, then runs 1,000 validated misses on the same string, with or without copying that string before each call. On the M1 the two curves nearly overlapped. These are medians from seven alternating measurements in milliseconds. [C fixture](../tools/ci/regex-engine.c)

| Leading `f` characters | 0 | 512 | 1,024 | 2,048 | 8,192 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Precompiled libc, caller's string | 0.303 | 10.424 | 21.487 | 42.300 | 166.563 |
| Precompiled libc, fresh copy | 0.389 | 10.815 | 21.367 | 41.944 | 168.746 |

The candidate checks a required trailing literal after successful compilation. The optimization accepts case-sensitive EREs ending in `$` whose preceding bytes are ASCII letters, digits, space, underscore, period, slash, hyphen, caret, dollar sign, plus, question mark or asterisk. Concatenation makes the final uninterrupted run of letters and digits mandatory, and the final anchor fixes its position. An absent suffix gives the ordinary miss result and clears the match array. Inputs passing that test still execute the compiled POSIX expression. Alternation, quoting escapes, bracket expressions, parentheses, non-ASCII patterns, case folding and invalid expressions follow the native path. The helper preserves the previous error for a subject containing NUL.

On the M1, nine interleaved end-to-end measurements used the preceding language candidate, the suffix check, and GNU Bash 5.3.20. Each process evaluated the regex 150 times. The capture control verified the matched length and the subgroup bytes. The native library still determines both control results. Both isolated Tonari executables occupied 2,129,664 bytes. [Public workloads and capture checks](../tools/ci/whole-language.py)

| Workload | Leading characters | Before (ms) | After (ms) | Bash (ms) |
| --- | --- | --- | --- | --- |
| Missing suffix | 512 / 1,024 / 2,048 | 6.908 / 8.186 / 11.298 | 5.224 / 4.833 / 5.159 | 13.191 / 14.443 / 18.897 |
| Correct suffix, mismatch before it | 512 / 1,024 / 2,048 | 6.922 / 8.480 / 11.542 | 6.481 / 8.545 / 11.743 | 13.240 / 14.773 / 18.715 |
| Successful grouped match | 512 / 1,024 / 2,048 | 6.192 / 7.506 / 9.871 | 5.963 / 7.685 / 10.015 | 12.710 / 14.569 / 16.939 |

The 768-line pipe case supplied 13,090 bytes. The same Linux runner measured 3.345 ms for Tonari and 3.794 ms for Bash. Tonari's `input::stdin_reader` and GNU's `fd_to_buffered_stream` both use one-byte reads on a nonseekable command input. An external command inheriting the pipe must be able to consume the bytes following its command line. The native job compares read counts under `strace` separately from wall timings when the tracer is available. [Native job](../tools/ci/whole-audit.sh)

Linux x86-64 で長い文字列では当初の正規表現照合が Bash の時間を超えた。C の単体測定では、文字列のコピー以外の `regexec` 自体の時間が長さに応じて増えていた。コンパイルが成功した単純な式だけ、末尾のアンカーと必須リテラルを先に調べる。候補が通った場合や、選択・引用・文字クラス・キャプチャ・大小文字を無視する式では POSIX 実装が照合を行う。接尾辞が正しく途中だけ異なる例、キャプチャ内容、無効な式の終了値 2、直前の照合結果の保持、不一致後の配列クリアを個別に検証した。パイプ入力の1バイト読み取りは、子プロセスが後続入力を同じ記述子から読めるようにするためで、GNU Bash も同じ方式を使う。
