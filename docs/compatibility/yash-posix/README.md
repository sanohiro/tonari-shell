# POSIX sh comparison / POSIX sh の検証

The release under test is `10e63fd`. The previous release is `d858d41`. Yash 2.60 at `1cdb509a617894ed96a12c33b43bcd7adb6f1ff5` supplies the scripts, helpers, controlling terminal and expectations. Its runner invokes each candidate via the same name, `sh`. Linux uses the runner's GNU Bash 5.2.21 and macOS uses a native build of GNU Bash 5.2.37. Each host exercises all 122 POSIX fixture files, including 62 generated signal fixtures. [Upstream instructions](https://github.com/magicant/yash/blob/2.60/tests/README.md) · [Full native run](https://github.com/sanohiro/tonari-shell/actions/runs/45615699784)

各行は上流のファイル名、行番号、試験名で識別します。同じ行の Bash と Tonari に対して、上流が指定した終了状態・標準出力・標準エラーを個別に照合します。同じ判定が示す一致の範囲は、その行で上流が指定した検査です。通常の Bash としての動作は別の1,411項目の監査で照合しました。[通常モードの監査](../../compatibility.md)

The regular Rust suite passed 58 tests. The Bash 5.2.37 end-to-end run executed 230 tests with two platform skips. The separate check with Apple's system Bash 3.2.57 produced the same 26 failing subtests before and after the change. The new POSIX witness uses the project's modern Bash reference because the old Bash exits on a missing file even under `command .`.

## Results / 全件の結果

| Host | Named cases | Both pass | Only Tonari passes | Only Bash passes | Both skip | Other outcomes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Linux x86-64 | 11,079 | 9,256 | 1,676 | 55 | 14 | 78 |
| macOS arm64 | 11,079 | 9,255 | 1,677 | 55 | 14 | 78 |

| Host | Comparable | Faster than Bash | At most 2/3 of Bash | At or above Bash | Median case ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| Linux x86-64 | 9,256 | 9,198 | 8,932 | 58 | 0.428 |
| macOS arm64 | 9,255 | 9,231 | 8,753 | 24 | 0.483 |

The 60 ordinary fixture files cover syntax, expansions, builtins, pipelines and job control, among other operations. Their timings can be read independently of the 62 generated signal files.

| Ordinary fixtures only / 自動生成のシグナル試験を除く | Named cases | Comparable | Faster than Bash | At most 2/3 of Bash | Median case ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| Linux x86-64 | 1,991 | 1,790 | 1,776 | 1,659 | 0.558 |
| macOS arm64 | 1,991 | 1,789 | 1,775 | 1,576 | 0.546 |

Both hosts recorded generated input for all 11,079 cases and Rust path markers for all 11,065 executed cases. Each gained 819 stable joint passes over the previous Tonari revision and retained every previously stable joint pass. On each platform, 1,590 Tonari-only passes are generated signal combinations. The single verdict comparison that differs between hosts is `trap-p.tst:60`, which Bash passes on Linux. The title of `test-p.tst:99` includes the block device the upstream script finds locally.

「その他」の78件は、両者が上流の期待値を満たさなかった74件と、両者とも合格せずタイムアウトなどの判定が異なった4件です。各OSで14件は両方ともスキップされ、残る全件で3回の合否が一致しました。生成されたシグナル試験の数が多いため、通常60ファイルだけの速度も上に示しています。[機能別集計](fixtures.tsv) · [判定が異なる試験](differences.tsv) · [目標を超えた時間](above-target.tsv) · [全判定と件数](summary.json)

Every case and each observed Rust function are available in the downloadable tables. [Linux x86-64](ubuntu-24.04.tsv.gz) · [macOS arm64](macos-15.tsv.gz) · [Function IDs and cost review](sites.tsv)

## Distribution references / Debian・Ubuntu の Bash

We also ran the unmodified upstream Make rules against the distribution Bash packages and a native Linux arm64 Tonari built from the same source with Alpine Rust 1.96.1. These are supplementary correctness observations. Rows below count ordinary passes. Twenty-one cases were skipped in each run. [All four verdicts for each case](distro-arm64.tsv.gz)

| Distribution | Bash | Compared cases | Both pass | Only Tonari passes | Only Bash passes | Neither has an ordinary pass |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Debian 13 arm64 | 5.2.37 | 1,965 | 1,778 | 71 | 51 | 65 |
| Ubuntu 26.04 arm64 | 5.3.9 | 1,965 | 1,781 | 66 | 65 | 53 |

この追加比較は生成されたシグナル組み合わせを除く60ファイルのうち、両方が最後まで到達した57ファイルです。生の Make 実行では両ディストリビューションの Bash が `bg-p` と `fg-p` で停止した子を待ち続け、`kill4-p` は上流ランナーまで SIGHUP で終了しました。この3ファイルは上表の分母から外し、タイムアウトと再開を扱える本体の全件比較で検証しました。

## Pipes, redirections and waits / パイプ・リダイレクト・終了待ち

As a check on the timed controller we also ran the unmodified `make test-posix` in separate clean fixture builds on the physical M1. Neither executable was wrapped, and no test was skipped in these four files. All nine pipeline cases passed on both shells.

| Upstream file | Cases | Both pass | Only Tonari passes | Only Bash passes | Both fail |
| --- | ---: | ---: | ---: | ---: | ---: |
| Redirection | 61 | 59 | 1 | 0 | 1 |
| Pipeline | 9 | 9 | 0 | 0 | 0 |
| Command substitution | 21 | 20 | 1 | 0 | 0 |
| Wait | 16 | 4 | 5 | 0 | 7 |
| Total | 107 | 92 | 7 | 0 | 8 |

計時用の変更を加えない上流版でも、Tonari は99件、Bash 5.2.37 は92件成功しました。両方失敗した8件のうち7件は `wait` で、例として未知の子に対する診断出力があります。

For the four original fixtures, the next table gives the median Tonari/Bash time ratio across comparable cases and the number that met the 2/3 target. Each case includes shell startup. The repeated external-launch and shared-input measurements exercise a running shell. [Repeated workloads](throughput-native.tsv)

| Fixture | Linux median ratio | Linux within 2/3 | macOS median ratio | macOS within 2/3 |
| --- | ---: | ---: | ---: | ---: |
| Redirection | 0.629 | 50 / 59 | 0.666 | 30 / 59 |
| Pipeline | 0.708 | 4 / 9 | 0.622 | 9 / 9 |
| Command substitution | 0.526 | 20 / 20 | 0.541 | 16 / 20 |
| Wait | 0.488 | 4 / 4 | 0.549 | 4 / 4 |

## Reading the results / 結果の読み方

* The two compressed TSV files retain every named case, the verdicts for all three shells, per-shell medians and individual samples in microseconds, the first trial's failed assertions and the untimed path replay. A vertical bar in a verdict means that trials disagreed. A single run's assertions and the stderr-presence check are diagnostic aids. The original upstream reports in the artifacts contain the actual differences.
* A speed ratio requires three ordinary upstream passes by **both** Tonari and Bash. A value of 0.60 means Tonari took 60% of Bash's elapsed time. Ratios use per-shell medians for the same case. The overall median weights each case equally. Signal fixtures contribute many cases. The fixture table also reports each file separately. The above-target table keeps every case above 2/3, ordered by ratio within each native host. [Above-target cases](above-target.tsv)
* Both upstream expected-failure verdicts, skips, timeouts, controller losses and missing observations retain distinct labels. A known failure of another shell does not excuse a failure in Tonari. An upstream pass is an observation about the assertions and environment of that case.
* `sites.tsv` maps probe IDs to the Rust function and the parsing, copying, lookup or OS operation inspected there. A separate diagnostic binary records sites reached by the upstream input, including already-fast cases. Each hit shows that the function executed at least once. The independent release timings measure elapsed time. Skipped tests have no shell path or timing.
* The raw artifacts preserve generated scripts and arguments, when available, first-trial upstream reports for all three shells, per-run observations, host and binary details. A frozen digest in each table connects a case to its captured input.

全試験の順位付けには、両方のシェルが全3回成功したケースだけを使いました。外部コマンドや子の終了待ちも、実際のスクリプトが待つ時間として含みます。上流の期待値生成、出力比較、診断用バイナリの記録は計時区間の外です。0.67前後の短い試験は3回の中央値だけで最適化の採否を決めず、該当するファイルを11回繰り返す検証と、シェル内で同じ処理を繰り返す検証も行います。[追加計測](https://github.com/sanohiro/tonari-shell/actions/runs/45616001148)

## Implementation review / 実装の確認

The fixture names below omit the `-p.tst` suffix. Each row in the case tables links the actual replay to the individual functions listed in `sites.tsv`. Grouping cases here describes the shared work inspected. A probe hit establishes execution of a function. The paired changes and repeated workloads supply evidence about its cost.

| Upstream fixtures | Work checked and outcome |
| --- | --- |
| `alias, comment, input, lineno, quote, startup` | Check when a complete input unit becomes executable, whether the next alias definition is visible, what bytes remain for a child reader and which positions diagnostic text retains. Changed interactive input boundaries and the alias representation. Unquoted backslash-newline inside a multi-character operator or assignment still has known parsing gaps. |
| `andor, break, case, continue, for, function, grouping, if, until, while` | Inspect the parsed compound body, branch dispatch and loop transfer. A nested condition used to be mistaken for its enclosing terminator. Functions reuse their parsed body on invocation. The controller times the complete short script including process startup. |
| `arith, fsplit, fnmatch, param, path, tilde` | Check which expansions split fields, enumerate directories, scan a pattern or query the user database. Fixed assignment tilde expansion across colons. A tilde inside a parameter-expansion operand still differs. In the Linux bracket fixture the missing outputs correspond to collating elements and equivalence classes. The ordinary POSIX character class checks in that fixture produced the expected output. |
| `builtins, command, declutil, export, getopts, readonly, set, shift, unset` | Inspect prefix scope, scalar ownership, argument advancement and builtin lookup. Special builtin assignments now take ownership of the prepared strings. Recursive `command command export`, readonly reassignment errors and the notify flag still have named differences. |
| `cd, dot, errexit, error, eval, exit, nop, option, return, simple` | Check startup and sourced-file lookup, physical directory lookup, `eval` parsing, nested error propagation and command-cache invalidation. Fixed `cd -P -e`, `exit -- N`, `return -- N` and missing-file handling for the special dot builtin. Remaining examples include default statuses inside traps and a removed executable in the command hash. |
| `async, bg, cmdsub, exec, fg, job, kill1–4, pipeline, ppid, read, redir, test, testtty, trap, umask, wait` | Inspect parent versus child execution, ordered fd operations, captured output, status collection, terminal ownership and process launch attributes. Direct generic launches can omit a spawn attribute object. Builtin writes now check write errors. A returning `fg` blocks SIGTTOU through the terminal handoff and mode restoration. The recheck also splits fatal-signal execution at a marker immediately before `kill`. |
| 62 generated `sig*` fixtures | Check inherited, explicitly ignored, trapped and reset dispositions across the shell, subshell, substitution, asynchronous list and external exec. Explicit inherited ignores and the default SIGPIPE behavior were checked at the new process boundary. Review all generated combinations individually in the compressed tables. Case names specify who sent the signal, the target and the initial disposition. |

同じ合格数でも中身は異なります。たとえば Bash だけが通る行のうち、`readonly a=1; readonly a=2; echo not-reached` を Tonari が最後まで実行するケースと、`set -b` の非同期通知オプションが使えないケースは残課題です。UTF-8 やロケールの方針でこれらを許容扱いにはしていません。[全ファイルの集計](fixtures.tsv) · [判定が異なる試験](differences.tsv)

On Linux, 16 of the 55 Bash-only passes come from the quoting fixture. Ten split a word or operator across a backslash and newline. Six check single quotes or tilde in the operand of a parameter expansion. Ten other Bash-only passes come from aliases in grammar positions, including a here-document delimiter, a function definition and pieces of operators. The option and `set` fixtures account for another ten. The individual rows retain the missed exit, stdout and stderr checks.

## Interpreting differences / 差異の判定

| Upstream case | Observed behavior and decision |
| --- | --- |
| `redir-p.tst:90` | A here-document with no command expands `${v=inner}`. A following `echo "${v-unset}"` prints `unset` in Tonari and Ubuntu 26.04 Bash 5.3.9, and `inner` in Bash 5.2.37 on both the M1 and Debian 13. Keep the assignment inside the redirection environment. An independent check confirms that both shells actually execute the next command. [POSIX.1-2024 §2.9.1.3](https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html#tag_19_09_01_03) |
| `redir-p.tst:290` | Tonari, Debian Bash 5.2.37 and Ubuntu Bash 5.3.9 accept `3</dev/null >&3`. Yash expects an immediate failure. POSIX permits an error when the source is open but lacks write access. Keep the successful descriptor duplication. A subsequent write to that descriptor still fails. [POSIX.1-2024 §2.7.6](https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html#tag_19_07_06) |
| `wait-p.tst:28` | Both shells return 127 for the unknown child and print a diagnostic. The fixture also requires empty stderr. The ledger preserves the failing stderr assertion. |

ヒアドキュメントだけのコマンドで展開した変数は、Tonari では親シェルに残しません。Debian 13 と Ubuntu 26.04 の arm64 コンテナでも明示的に Bash を `sh` 名で起動し、Ubuntu の Bash 5.3.9 では Tonari と同じ結果になることを確認しました。一方、出力先に読み取り専用 fd を複製する例では、規格上はその時点でエラーにすることも許されています。他の不一致は、根拠のある個別判断が付くまで差異として扱います。[回帰テスト](../../../tests/shell.py)

## Reproduction / 再現手順

Use independent clean yash builds when running the upstream Make target against two executables. Its saved `.trs` files can otherwise reuse the first executable's result. The example uses Homebrew GNU Make (`gmake`) on macOS. On Debian and Ubuntu use `make`. Each checkout writes its own `tests/summary.log`.

```sh
git clone --branch 2.60 --depth 1 https://github.com/magicant/yash.git yash-tonari
git -C yash-tonari worktree add --detach ../yash-bash HEAD
for tree in yash-tonari yash-bash; do
    (cd "$tree" && ./configure --disable-nls && gmake yash) || exit 1
done
gmake -C yash-tonari/tests TESTEE=/absolute/path/to/tonari test-posix
gmake -C yash-bash/tests   TESTEE=/absolute/path/to/bash   test-posix
```

For an individual native shard, run the project driver against a fresh checkout. It adds a monotonic clock to the controlling yash process immediately around the existing testee fork, redirections, execution and wait. The upstream controller still creates the sh symlink, opens the expected streams and judges the output. The candidate executable has no timing wrapper. File preparation, expectation checks, diff programs and polling by the outer Python runner are outside the measured interval.

```sh
python3 tools/ci/yash-posix.py /tmp/clean-yash /tmp/posix-0 \
  --prepare --tonari /absolute/path/to/tonari --bash /absolute/path/to/bash \
  --previous /absolute/path/to/previous-tonari --shard 0/8 --runs 3
gh run download 45615699784 -D /tmp/posix-native
gh run download 45617891045 -D /tmp/posix-resumed
gh run download 45616001148 -D /tmp/posix-repeated
python3 tools/ci/yash-report.py /tmp/posix-native docs/compatibility/yash-posix \
  --replacement /tmp/posix-resumed --recheck /tmp/posix-repeated
```

Bash can signal the upstream controller just as its subprocess writes a verdict. Shard 6 was repeated to resume beyond that case and collect the remaining generated inputs. Whole shards replace the earlier measurements with identical timed executables. [Resumed native run](https://github.com/sanohiro/tonari-shell/actions/runs/45617891045)

The collector creates private argument logs and preopens path-recording sinks before restrictive umask tests. The report verifies the Tonari hashes, GNU Bash version and trial counts across all eight shards on each host. It also checks that the higher-repeat run used the same Tonari release and Bash version. The native Linux x86-64 binary was a 1,958,808-byte stripped static PIE (previously 1,952,344 bytes). The macOS arm64 CI binary was 1,466,512 bytes (previously 1,466,496). [Build identities](builds.tsv)

The pull-request workflow runs eight upstream fixture files and checks for newly lost Bash passes against the PR base. The native benchmark workflow runs the full list on demand. It rotates the three release executables per fixture and trial. A timeout is recorded against the failing case. If a job-control case terminates its own upstream controller, the driver retains the partial report, resumes the fixture after that case and records the loss. [PR workflow](../../../.github/workflows/posix-shell.yml) · [Runner](../../../tools/ci/yash-posix.py) · [Instrumentation](../../../tools/ci/yash-paths.py)
