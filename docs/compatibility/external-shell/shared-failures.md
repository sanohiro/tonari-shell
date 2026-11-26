# Shared failures in bensch and Oils / 両シェルが失敗した試験

The original full comparison had 356 joint failures across both operating systems, covering 272 distinct cases. Linux had 3 interactive, 42 builtin and 41 Oils failures. macOS had 56 interactive, 42 builtin and 172 Oils failures. Each row in the review records its original and replayed output excerpts, status, assertion results and the available control. A “PASS” under an ordinary assertion describes what that suite expects. It can also encode an Oils-specific policy. An Oils Bash annotation can override a more general expectation. The table preserves both grades and identifies which ordinary assertions failed. PTY evidence records the original matcher's error. [356 case findings](shared-failures.tsv.gz) · [Original full comparison](README.md)

両シェルが同じ結果でも、それだけで正しいとは判定しません。表には上流の採点をそのまま残し、実行したコマンド、標準出力・標準エラー・終了値、入力後に確定した内容を照合しました。履歴設定や外部コマンドの差で説明できるケースと、実際に Tonari の修正が必要だったケースが含まれます。[互換性の判断基準](../../compatibility.md#intentional-differences)

<a id="shell-behavior"></a>

## Execution, expansion and interactive behavior / 実行・展開・対話操作

The release at **42f53ff** ran the complete pinned Oils and bensch inventories on Ubuntu 24.04 x86-64 with Bash 5.2.21 and macOS 15 arm64 with Bash 5.3.20. Each host ran 2,770 language scripts, 875 builtin cases and 1,014 PTY sequences. The file of remaining failures includes per-host findings for both Bash-only and joint failures, the actual process statuses and output excerpts. The native CI artifacts retain the complete raw streams for scripts. [9,318 case results and implementation paths](shell-behavior-replay.tsv.gz) · [Reviewed failures](shell-behavior-findings.tsv.gz) · [Full native audit](https://github.com/sanohiro/tonari-shell/actions/runs/45766999152)

| Host | Suite | Both pass | Only Bash | Only Tonari | Neither | Timed | Tonari ≤ ⅔ Bash | Tonari > Bash |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Linux x86-64 | Oils | 2661 | 67 | 1 | 41 | 2119 | 1658 | 16 |
| Linux x86-64 | bensch builtins | 831 | 2 | 0 | 42 | 818 | 796 | 0 |
| Linux x86-64 | bensch PTY | 983 | 0 | 28 | 3 | 983 | 231 | 237 |
| macOS arm64 | Oils | 2515 | 49 | 39 | 167 | 1993 | 1591 | 51 |
| macOS arm64 | bensch builtins | 830 | 2 | 0 | 43 | 817 | 801 | 3 |
| macOS arm64 | bensch PTY | 933 | 0 | 25 | 56 | 933 | 353 | 269 |

Compared with the previous published replay, 99 Oils cases on Linux and 96 on macOS moved from a Tonari failure to an accepted grade. Among the rows graded as Bash-only in this replay, Tonari meets the ordinary Oils assertions in 31 on Linux and 21 on macOS. Oils assigns the remaining failures their original upstream grade. A passing ordinary assertion checks the complete ordinary expectation for that case.

Linux と macOS の件数には参照 Bash の版、GNU と BSD の外部コマンド、ファイルシステム、TTY の有無、上流の採点方法も反映されます。例えば macOS の ps は sid 列を受け付けず、~root は /var/root に展開されます。組み込み exec の /dev/tty へのリダイレクトは、両 OS とも制御端末を与えた追加試験で出力と終了状態を照合しています。

- Functions and prefix assignments now restore the correct scope across eval and unset. RANDOM and SECONDS have dynamic integer semantics. Quoted array transformations, non-whitespace IFS separators, declaration indices and case attributes have direct argument and state checks.
- Redirection errors restore the descriptors before invoking ERR. The trap reports the failing operator's source line. SIGPIPE and child exit codes have controls that establish whether a pipe reader remains open. A child cannot signal success for a continue that targets its parent's loop.
- Tracing includes conditional and arithmetic commands, assignment source and prompt expansion. Tests capture trace output on its own file descriptor and verify that the underlying operand executes once. Interactive tests cover explicit history files, prompt escapes, optional autocd, Vi editing and bracket-matching feedback.
- The GNU Bash manual describes a letter range in C order. Tonari preserves generated backslashes and backticks as characters. Bash 5.2 can parse a generated backtick as syntax. For an indirect array subscript, Tonari supports numeric, variable, default-value and arithmetic forms using a read-only evaluator. Shell code introduced through variable contents is outside that interface. [GNU brace expansion](https://www.gnu.org/software/bash/manual/html_node/Brace-Expansion.html) · [Regression tests](../../../tests/shell.py)

通常期待値を満たした例には、複合コマンドのリダイレクト失敗を ERR が正しい行で報告するケースがあります。UTF-8 の文字境界、再利用できるトレース表現、連想配列の順序、終了順に依存するパイプは、それぞれ実際の値と副作用で確認しました。片方だけが通るケースも、双方が落ちるケースも一覧に残しています。

The macOS replay of Oils background/7 timed three independent child writes with sleeps of 10, 20 and 30 ms. Output order varied in both shells. Waiting on the explicit PIDs still returned 3, 2 and 1. A physical M1 control gave each child a release FIFO and an acknowledgement FIFO, then exercised all six output permutations in each shell. Every sequence and every wait status matched. For background/17, an independent controller released one child, read the result of wait -n, and then released the other. All 52 sequences returned the only available status first, covering both child orders in Bash and Tonari. [Original parallel jobs](https://github.com/oils-for-unix/oils/blob/d114978ca467143f171a835cbcbe8ab6ccc009a7/spec/background.test.sh#L85-L111)

The script time columns compare the median of five alternating executions when both grades are accepted and every recorded status, stdout and stderr agrees. The PTY time is the median delay to the first byte over paired input steps from one complete upstream session. A redraw can acknowledge one key with several writes; rendering, terminal scheduling and running the submitted command are also measured by the separate repeated response workload. Each row includes the observed implementation sites from an independent instrumented process. [Implementation-site review](sites.tsv) · [43 verified workloads](../../benchmarks.md#shipped-shell-timings) · [Native helper and response controls](https://github.com/sanohiro/tonari-shell/actions/runs/45762981435) · [History and editing controls](https://github.com/sanohiro/tonari-shell/actions/runs/45763138891)

Linux had 237 stock PTY cases with a later median first byte in Tonari. Among those cases the median difference was 65.5 µs and the 95th percentile was 279.9 µs. The repeated full-response controls below measure a separate observation.

macOS had 269 stock PTY cases with a later median first byte in Tonari. Among those cases the median difference was 60.3 µs and the 95th percentile was 459.1 µs. The repeated full-response controls below measure a separate observation.

### Controls for the external test environment / 上流の試験環境を揃えた追試

Three independent controls ran at 4ae605d. For the 66 editing cases, each test got its own PTY, history enabled and no status-overwriting marker commands. The response test timed eight verified command results through a terminal, 41 times per shell. In Oils, compiled argv and environment helpers replaced their interpreter-starting equivalents. Helper timing counts use only cases whose outputs and status agreed before and after substitution in both shells. [Helper samples](shell-behavior-helper-controls.tsv.gz) · [Native helper and response run](https://github.com/sanohiro/tonari-shell/actions/runs/45762981435) · [Editing controls](https://github.com/sanohiro/tonari-shell/actions/runs/45763138891)

| Native host | Fresh PTY both pass | Fresh PTY neither passes | Complete PTY response faster than Bash | Native argv helper ≤ ⅔ Bash | Native environment helper ≤ ⅔ Bash |
| --- | ---: | ---: | ---: | ---: | ---: |
| Linux x86-64 | 64/66 | 2/66 | 8/8 | 265/292 | 36/36 |
| macOS arm64 | 64/66 | 2/66 | 8/8 | 259/290 | 30/35 |

The two residual fresh-PTY assertions expect a single Tab to select `pwd` from multiple `pw` matches, and three backward-word motions to move across four words. Both shells retain the ambiguous prefix and perform the requested three motions. The source-level alias and subscript regression added after these controls also checks stderr, the selected element and the alias visible when a function runs. [Key sequences and fixtures](https://github.com/tenseleyFlow/bensch/tree/c8e2a1ea493fc0dceb9af645dc445e619cb449e7/suites/interactive) · [Source-level regression](../../../tests/shell.py)

履歴を無効にする上流設定と、前の終了値を書き換えるマーカーを外した対話試験も別に記録しました。補完が一意でない入力と、移動回数が期待位置に足りない入力は、実際の候補とカーソル位置で確認しています。

<a id="date-and-expansion-replay"></a>

## Date, expansion and completion replay / 日時・展開・補完の修正後

Each host ran the same 2,770 Oils cases, 875 bensch builtin cases and 1,014 bensch interactive cases. The full PTY suite ran at 05aeade. The final Oils and builtin replays ran at 3c9cdcb, after the date and timestamped-history changes. Interactive journal persistence and the prompt have additional direct PTY checks on the final implementation. The table keeps upstream verdicts. Oils accepts its Bash-specific annotations. The archive also retains grades against the ordinary expectations. [9,318 case results](date-and-expansion-replay.tsv.gz) · [Oils and builtin runs](https://github.com/sanohiro/tonari-shell/actions/runs/45752854550) · [Interactive runs](https://github.com/sanohiro/tonari-shell/actions/runs/45747754708)

| Host | Suite | Both | Bash only | Tonari only | Neither | Timed | Tonari ≤ ⅔ Bash | Tonari > Bash |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Linux x86-64 | Oils | 2,567 | 161 | 1 | 41 | 2,062 | 1,599 | 12 |
| Linux x86-64 | bensch builtins | 831 | 2 | 0 | 42 | 818 | 791 | 1 |
| Linux x86-64 | bensch PTY | 983 | 0 | 28 | 3 | 983 | 267 | 324 |
| macOS arm64 | Oils | 2,425 | 139 | 40 | 166 | 1,938 | 1,519 | 61 |
| macOS arm64 | bensch builtins | 830 | 2 | 0 | 43 | 817 | 777 | 4 |
| macOS arm64 | bensch PTY | 933 | 0 | 24 | 57 | 933 | 320 | 291 |

For scripts, the speed columns require five alternating trials per shell, stable identical output and status, and an accepted verdict in both shells. For PTYs, the speed columns use the median first-byte delay over matched input steps in one upstream run where both shells passed. PTY measurements include terminal scheduling. The separate 13-trial targeted benchmarks compare the time to verified results. [Native targeted timings](../../benchmarks.md#calendar-and-expansion)

Compared with the preceding script replay, 86 Oils cases on Linux and 85 on macOS moved from failing to accepted for Tonari. No previously accepted Oils case moved to failing. Open cases include genuine scripting differences, such as the dynamic RANDOM and SECONDS variables, alongside reference bugs and intentional UTF-8 behavior. The historical review below describes the controls for shell-specific annotations and for explicit expectations that differ from GNU Bash.

前回の Oils 全件との比較で、Tonari が新たに通ったのは Linux 86 件、macOS 85 件でした。以前通っていた Oils の項目に新たな失敗はありません。Bash だけが通るもの、双方が失敗するものも表と個別データに残しました。スクリプトは同じ結果が安定したケースに限って時間を比較し、PTY はキー入力から最初の表示までを別に扱います。

One macOS PTY verdict changed from Tonari-only to neither. The bensch history/36 case submits a one-line loop, presses Up and Enter, and looks for the digit 1. Its Bash profile sets HISTSIZE=0. A local M1 control used the original loop and logged each call to echo, waited for the loop to finish, then sent Up and Enter. In 11 fresh sessions per shell and setting, both Tonari and GNU Bash 5.2.37 executed the loop once with HISTSIZE=0 and twice with HISTSIZE=500. This independently checks recall and avoids mistaking the original echoed input for a second execution. 新たに両者失敗となった履歴の 1 件は、画面に数字が見えるかに加えてループの実行回数を確認しました。履歴を有効にした場合、両シェルとも全回で再実行できました。[Upstream steps](https://github.com/tenseleyFlow/bensch/blob/c8e2a1ea493fc0dceb9af645dc445e619cb449e7/suites/interactive/history/history.yaml#L360) · [Runner settings](../../../tools/ci/bensch-pty.py)

## Preceding native results / 前回の実機結果

| Host | Suite | Both | Bash only | Tonari only | Neither | Timed | Tonari ≤ ⅔ Bash | Tonari > Bash |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Ubuntu x86-64 | Oils | 2481 | 247 | 1 | 41 | 2007 | 1581 | 16 |
| Ubuntu x86-64 | bensch builtins | 831 | 2 | 0 | 42 | 818 | 799 | 0 |
| Ubuntu x86-64 | bensch PTY, stock conditions | 63 | 0 | 0 | 3 | — | — | — |
| Ubuntu x86-64 | bensch PTY, history enabled and no markers | 64 | 0 | 0 | 2 | — | — | — |
| macOS arm64 | Oils | 2342 | 221 | 38 | 169 | 1885 | 1485 | 53 |
| macOS arm64 | bensch builtins | 830 | 2 | 0 | 43 | 817 | 780 | 3 |
| macOS arm64 | bensch PTY, stock conditions | 10 | 0 | 0 | 56 | — | — | — |
| macOS arm64 | bensch PTY, history enabled and no markers | 64 | 0 | 0 | 2 | — | — | — |

Each host ran all 2,770 eligible Oils cases and 875 builtin cases. The interactive follow-up ran the union of the 66 previously Bash-only or jointly failing PTY cases. The earlier snapshot covered all 1,014 PTY cases per host. The script table contains each case’s source revision, raw grades, ordinary Oils grades and median timings. A ratio requires stable byte-identical outcomes accepted by both. [7,290 script results](script-replay.tsv.gz) Script timings use five interleaved trials per shell. See the raw artifacts for every trial, emission and status. 試験ごとの出力差、両者成功・片方のみ・両者失敗を区別しています。[Interactive replay](https://github.com/sanohiro/tonari-shell/actions/runs/45735592985) · [Builtins and common-environment controls](https://github.com/sanohiro/tonari-shell/actions/runs/45736947179) · [Full Oils replay](https://github.com/sanohiro/tonari-shell/actions/runs/45738742622) · [Targeted replay](https://github.com/sanohiro/tonari-shell/actions/runs/45739226345) · [Final native replay](https://github.com/sanohiro/tonari-shell/actions/runs/45740024248)

Pull requests run the complete end-to-end shell and terminal regression file once alongside the four yash shards. It exercises Tonari's specified results even where Bash or an upstream annotation differs. PR では POSIX 比較に加え、意図した差を含むシェルと端末の回帰試験を自動実行します。[Pull request workflow](../../../.github/workflows/posix-shell.yml)

## Confirmed fixes / 再現して修正したもの

- The ten previously Bash-only bensch PTY cases passed on both hosts. They exercised vi word motions and edit operators, and diagnostics for duplicating a closed descriptor. In a separate history-enabled PTY, Ctrl-R followed by Right now accepts a result for further editing. The regression checks the actual file written after the edit. vi の編集と履歴検索からの編集再開は、入力確定後のコマンドの効果を確認しています。[PTY regressions](../../../tests/shell.py)
- With job control disabled and `shopt -s lastpipe`, `printf 'ok\n' | read -r v` sets `v=ok` in the parent. The status of each stage remains in `PIPESTATUS`. Oils `vars-special/34` still expects an initially empty `$_` after passing the argument `lastpipe`. Compare the preceding argument separately from the pipeline's effect. このケースには期待値の問題と、親シェルで値を受け取れない実装不備が共存していました。[Regression](../../../tests/shell.py) · [Oils case](https://github.com/oils-for-unix/oils/blob/d114978ca467143f171a835cbcbe8ab6ccc009a7/spec/vars-special.test.sh)
- Compound array initializers now expand right-hand sides and then evaluate arithmetic indices against the developing array. Quoted compound declarations preserve field boundaries. Associative arithmetic preserves keys containing a backslash, bracket, quote, dollar sign or Japanese text. An old Bash `BUG` expectation in `array-assoc/34` rewards losing the element. The ordinary expectation and the direct regression verify the key maps to 42. 通常の期待値と実際の配列要素を確認し、古い Bash と同じデータ消失は修正しました。[Oils case](https://github.com/oils-for-unix/oils/blob/d114978ca467143f171a835cbcbe8ab6ccc009a7/spec/array-assoc.test.sh#L567)
- History import after copying a file recognizes its common prefix. Importing a different file whose prefix has different contents reads that file from the beginning. Explicit `HISTSIZE` and `HISTFILESIZE` limits take effect immediately. Tonari keeps its private interactive journal separate from explicitly selected plain Bash history files. 履歴の重複と取りこぼしをそれぞれテストします。[History regressions](../../../tests/shell.py)
- `kill -L` now reports a bad signal, continues through later arguments and returns failure. `[[ "$HOME" =~ ~ ]]` expands the home directory as literal regex text even when it contains `^` or `$`. The Homebrew Bash 5.3.20 reference continued expanding `~` to the initial HOME after reassignment. Both shells matched when the same metacharacter-containing HOME was set at launch. A separate regression protects Tonari’s live HOME lookup. ホームディレクトリをプロセス起動時に指定した場合と、シェル内で更新した場合を個別に確認しました。[Regression tests](../../../tests/shell.py)

For symbolic permissions, `umask 0124; umask a=,a=u && umask` prints `0777` in Tonari. GNU Bash 5.2.37 rejects the operand and the Homebrew 5.3.20 build prints `0111`. POSIX specifies applying successive symbolic clauses in order. Starting with permission bits `0653`, both GNU and Darwin `chmod a=,a=u` leave mode `000`. The inverse mask is `0777`. 上流の期待値に合わなくても、順番に適用した権限と最終結果を直接検証します。[POSIX chmod](https://pubs.opengroup.org/onlinepubs/9799919799/utilities/chmod.html) · [POSIX umask](https://pubs.opengroup.org/onlinepubs/9799919799/utilities/umask.html)

In POSIX mode, `x=42; x=tmp unset x` leaves `x` unset in Tonari, Bash 5.3.20, yash and dash. Bash 5.2.37 restores `42`. The same split occurs inside a function and when the special builtin is `eval 'unset x'`. With `x=tmp :`, all four references and Tonari retain `tmp`. Keep the effect of the actual special builtin on its current binding. Bash 5.2 だけを参照すると、削除した変数が復活する方に合わせてしまいます。この差は独立した回帰試験で保護しています。[Simple command assignments](https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html#tag_19_09_01) · [Regression](../../../tests/shell.py)

## What changed across platforms / OS 間の差

At revision `7f01725`, 172 of the 2,770 Oils cases had a different pair of pass/fail results across the two native hosts. Tonari's individual result differed in 103. Rebuilding both Bash references from the same GNU Bash 5.2.37 source and providing GNU coreutils and GNU sed reduced those numbers to 36 and 26. The original references were Bash 5.2.21 on Ubuntu x86-64 and Bash 5.3.20 on macOS arm64. This experiment varied several inputs together. It establishes configuration sensitivity. The following witnesses isolate individual causes. [Both native controls](https://github.com/sanohiro/tonari-shell/actions/runs/45736947179)

同じコードでも、OS が提供するコマンドやファイルシステム、参照 Bash の版、試験ランナーによって観測値は変わります。GNU Bash と GNU ツールを揃えると Tonari 自体の合否が OS 間で異なるケースは 103 件から 26 件に減りました。残る差にも個別の確認が必要です。

- Darwin `wc -l` adds leading spaces. GNU `wc -l` on a pipe prints the count without them. Several fixtures hard-code paths such as `/usr/bin/cat` or restrict PATH to `/bin` and then call `which`. The Darwin paths differ. `~root` is normally `/var/root` on these Macs and `/root` on these Linux hosts. The macOS test volume also merges names differing only in case. [Per-case evidence](shared-failures.tsv.gz)
- The bensch Bash profile sets `HISTSIZE=0`. On Darwin the runner can insert an `echo` between commands, replacing the status being tested. Fresh PTYs with history enabled and command markers disabled passed 64 of the 66 cases in **both shells on both hosts**. The two remaining assertions expect `pw<Tab>` to choose `pwd` on an ambiguous PATH, and three backward-word motions to reach the first of four words. In the stock replay the shared failures remained 3 on Linux and 56 on macOS. [Runner source](https://github.com/tenseleyFlow/bensch/blob/c8e2a1ea493fc0dceb9af645dc445e619cb449e7/framework/runner.py) · [Native replay](https://github.com/sanohiro/tonari-shell/actions/runs/45735592985)
- On the physical Mac the inherited PATH resolved both `x` and `X` under `/opt/X11/bin` to XQuartz. With a harmless logging stand-in, Bash and Tonari both executed `x` in Oils `alias/18` and `X` in `exit-status/6` and `/7`. The fixtures expect command lookup to fail. Both shell subprocesses now receive the same explicit test PATH. XQuartz の起動経路を再現しました。実際に画面に現れた各プロセスの親までは記録されていません。[Test environment](../../../tools/ci/spec-pairs.py)
- Of the 42 previous builtin joint failures per host, 36 exercise fortsh-specific commands. Two try named coprocess syntax without a compound command. Two need pre-existing history state. One compares a literal fortsh diagnostic and one defines and calls an alias on the same parsed line. With corrected inputs both shells exchanged data with a live named coprocess, deleted a seeded history entry, imported two lines, and expanded the alias. Both printed zero and returned 1 for an invalid numeric `printf` operand. The two additional `exec` fixtures which reopen `/dev/tty` return the intended output with a controlling terminal on both platforms. [Direct terminal samples](https://github.com/sanohiro/tonari-shell/actions/runs/45736947179)

The final script comparison also has newly joint failures. On both hosts `array-assoc/34` now fails the old Bash annotation because both shells keep the data. On macOS the output order varied in `background/7`. In builtin `portable/test_exec/10`, both shells timed out without a controlling terminal. The failed reopen of `/dev/tty` left stdout pointing at the file subsequently read by `cat`. Giving the original command a controlling PTY produced `captured` and status 0 in all 11 trials for each shell. 新たに両者失敗となった行も、配列の実データ、完了順、端末の有無まで確認しました。[Script transitions](script-replay.tsv.gz) · [PTY samples](https://github.com/sanohiro/tonari-shell/actions/runs/45736947179)

The follow-up implemented the four open differences from this review. Tests cover default-value assignment tildes, named homes formed across brace alternatives, indexed-looking ordinary arguments, and the builtin date formatter. GNU Bash 5.2 drops a 128-byte result in Oils `builtin-printf/51`. Tonari returns all 128 bytes, and its own regression checks all 4,000 bytes of a longer format. この調査で特定した4件を実装し、日時・引数・チルダの実際の値を Bash と照合しました。日時書式を切り捨てる旧版のバグは期待値から除外します。[Behavior and examples](../../compatibility.md#calendar-and-arguments) · [Performance](../../benchmarks.md#calendar-and-expansion)

When `background/7` runs on a loaded macOS runner, delays of 10, 20 and 30 milliseconds can produce different orders of output. Both shells varied across the five trials of the full replay at `d34efd8`. The three explicit PID waits still returned 3, 2 and 1. Keep the observed verdict and exclude unstable times from speed comparisons. macOS の並行出力は順序が変動した回があり、終了値を個別に確認しました。[Full replay](https://github.com/sanohiro/tonari-shell/actions/runs/45738742622)
