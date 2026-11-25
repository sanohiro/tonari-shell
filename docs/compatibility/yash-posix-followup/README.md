# POSIX compatibility and execution time / POSIX 互換性と実行時間

Tonari `7cad956` and GNU Bash ran as `sh` against yash 2.60, commit `1cdb509a617894ed96a12c33b43bcd7adb6f1ff5`. The Linux x86-64 reference was Ubuntu 24.04’s Bash 5.2.21 and the macOS arm64 reference was GNU Bash 5.2.37. Each native CI host ran the same 122 files three times per executable. Each comparison uses the upstream case identity and its requested stdout, stderr and exit-status assertions. The earlier Tonari executable comes from `72c2500`. [Native run](https://github.com/sanohiro/tonari-shell/actions/runs/45663406314) · [macOS shard 1](https://github.com/sanohiro/tonari-shell/actions/runs/45669899204) · [Upstream tests](https://github.com/magicant/yash/tree/2.60/tests)

## Bash-passing cases / Bash が通るケース

| Host | Cases | Tonari passes | Both pass | Only Tonari passes | Only Bash passes | Other Tonari verdicts |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Linux x86-64 | 11,079 | 11,057 | 9,311 | 1,746 | 0 | 22 |
| macOS arm64 | 11,079 | 11,057 | 9,310 | 1,747 | 0 | 22 |

各ホストで11,079件を3回ずつ照合し、以前は Bash だけが通っていた55件がすべて合格しました。各ホストで残る22件の内訳は、両シェルとも失敗7件、スキップ14件、上流が失敗を想定した試験での成功1件です。[Linux](ubuntu-24.04.tsv.gz) · [macOS](macos-15.tsv.gz) · [Fixture counts](fixtures.tsv)

A supplementary run covered the full suite once on native Linux arm64 in both distribution environments. The distribution Bash executable was Debian 13's 5.2.37 or Ubuntu 26.04's 5.3.9. The independently built static Tonari used Alpine Rust 1.96.1 and the same source. [Distribution verdicts for all 11,079 cases](distro-full.tsv.gz)

| Distribution | Cases | Both pass | Only Tonari passes | Only Bash passes | Neither has an ordinary pass |
|---|---:|---:|---:|---:|---:|
| Debian 13 arm64 | 11,079 | 9,303 | 1,746 | 0 | 30 |
| Ubuntu 26.04 arm64 | 11,079 | 9,336 | 1,711 | 0 | 32 |

上流のコントローラーを変更せず、両ディストリビューションで通常57ファイル・1,965件を Make 経由でも照合しました。ジョブ制御の3ファイルは全122ファイルの計測で検証しています。Bash だけが通ったケースはどちらも0件で、Tonari は Debian で1,935件、Ubuntu で1,933件に合格しました。上の全件比較には残りのファイル、停止した子のタイムアウト処理、シグナル組み合わせも含まれます。[Unmodified Make results](distro-arm64.tsv.gz) · [Previous comparison](../yash-posix/README.md)

On a physical M1, the earlier release intermittently returned 1 from a foreground subshell that should print `ok` with SIGQUIT ignored. In a diagnostic build Darwin returned EPERM from the child’s `setpgid(0, 0)` while its actual process group already equalled its PID. The parent also assigns that group after fork. An immediate membership check reduced failures on the same 15,000-case input from 13 to 1. A direct C reproducer caught 19 errors in 150,000 forks where the old membership was still visible, including immediately repeated EPERM. The final release retries concurrent creation of the leader’s own group on this error path. All 15,000 repeated upstream bodies passed at `04c7a98` and at `b2bb185`. The C control with retry also recovered every group assignment in 280,000 forks. An 11,079-case physical M1 pass also had zero Bash-only cases after the fix. [Case verdicts before and after](physical-m1-verdicts.tsv.gz) · [Apple kernel implementation](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_prot.c) · [Deterministic regression](../../../src/jobs.rs)

実機でまれに起きたジョブ起動の競合も直しました。親がすでに目的のプロセスグループを設定した子が、同じグループを設定し直したときのエラーで終了していました。競合時には所属グループの反映にも時間差があり、直後の確認だけでは15,000回のうち1回失敗しました。最終版は EPERM が出て、自分のグループを作成中の場合だけ、所属を確認して短く再試行します。通常の成功経路はシステムコール1回です。テストではセッションリーダーを作って同じエラーを必ず発生させ、希望のグループに所属している場合と別グループの場合を個別に検証します。

The shell now handles continuations across parameter names and operators, aliases that introduce grammar tokens, nested declaration commands, readonly errors and special builtin exit rules. Background commands preserve ignored signals, jobs can be selected by a substring of a compound command, and foreground job control restores the inherited terminal process group. The regression suite also checks editable input during a background completion notice. [Direct witnesses](../../../tests/shell.py) · [PR regression gate](../../../.github/workflows/posix-shell.yml)

残る上流の判定には、コマンド置換内のエイリアスをいつ展開するか、`PATH=` のときに組み込みの `echo` を実行するか、代入を含む単純コマンドでリダイレクトを適用する順序などがあります。上流が失敗を期待する `return-p.tst:159` は成功したため、上流の分類どおり「予期しない成功」として残しました。Debian arm64 の補助環境では `ppid-p.tst:5` が使う外部の `ps` コマンドが見つからず、双方で失敗しました。[Linux case table](ubuntu-24.04.tsv.gz) · [macOS case table](macos-15.tsv.gz)

The regular suite passed all 59 Rust tests and 234 of the 236 end-to-end tests, with two platform skips. Against GNU Bash 5.2.37, the separate 1,411-entry ordinary-shell audit retained every previous match. Three previously different probes now matched and three more exposed the expected option interface. [Audit driver](../../../tests/compat/audit.py) · [Compatibility guide](../../compatibility.md)

## Timings / 速度

| Workload set | Joint passes | Tonari faster | At most 2/3 | Median case ratio |
| --- | ---: | ---: | ---: | ---: |
| Linux x86-64 · All | 9,311 | 9,268 | 9,037 | 0.430 |
| Linux x86-64 · 60 ordinary files | 1,841 | 1,824 | 1,698 | 0.561 |
| macOS arm64 · All | 9,310 | 9,272 | 8,640 | 0.478 |
| macOS arm64 · 60 ordinary files | 1,840 | 1,808 | 1,537 | 0.537 |

Each case contributes one ratio of median elapsed times. The 62 generated signal fixtures contain 9,088 cases. The ordinary-file rows show the other 1,991 separately. A suite-wide ratio below 2/3 does not imply a per-case guarantee. [Three-trial results](summary.json) · [Initial above-target measurements](above-target.tsv)

The nine original pipeline cases ran 17 times per shell with the shipping release. A further 14 files ran 11 times at `e4a68b8`, before the interactive job notification change. They exercise read, redirection, command substitution, quoting, umask, aliases, shift, shell options, PATH lookup, signals and waits. [Final pipeline and throughput run](https://github.com/sanohiro/tonari-shell/actions/runs/45663405999) · [I/O fixtures](https://github.com/sanohiro/tonari-shell/actions/runs/45638830305) · [Signals and waits](https://github.com/sanohiro/tonari-shell/actions/runs/45638845137) · [Case medians](focused.tsv)

| Host | Retested | At most 2/3 | Above 2/3 and faster than Bash | At least Bash time |
| --- | ---: | ---: | ---: | ---: |
| Linux x86-64 | 274 | 158 | 111 | 5 |
| macOS arm64 | 670 | 608 | 62 | 0 |

The five cases that took longer than Bash on replay all terminate a process with a fatal signal on Linux. Every replayed macOS case finished ahead of Bash. [All replay timings](repeated-above-target.tsv)

| Upstream case | Signal | Tonari ms | Bash ms | Ratio |
| --- | --- | ---: | ---: | ---: |
| sigquit1-p.tst:7022 | SIGQUIT | 49.233 | 41.642 | 1.1823 |
| kill2-p.tst:23 | SIGSEGV | 88.285 | 79.198 | 1.11475 |
| kill2-p.tst:18 | SIGILL | 97.828 | 91.035 | 1.07462 |
| kill2-p.tst:13 | SIGABRT | 86.886 | 85.961 | 1.01076 |
| kill2-p.tst:15 | SIGBUS | 87.980 | 87.067 | 1.01048 |

目標の2/3を超えたのは、Linux で116件、macOS で62件です。内訳で最も多いのは Linux のファイル作成マスク（49件）、macOS のシグナルトラップ（11件）とファイル作成マスク（10件）でした。上流のケースには外部コマンドの実行や実際のファイル作成も含まれるため、組み込み単体の測定値も掲載しています。[組み込み単体](builtin-costs.tsv) · [ケース別の再測定](repeated-above-target.tsv)

3回で2/3を超えた比較可能な全ケースを元のテスト名・行番号から選び、同じ実行ファイルで11回ずつ再計測しました。先行テストが作るファイルに依存し、選別すると元の合格を再現できないケースは、そのファイル全体を再実行しました。[Above-target case replays](repeated-above-target.tsv) · [Native replay samples](https://github.com/sanohiro/tonari-shell/actions/runs/45663406314) · [Replay driver](../../../tools/ci/yash-repeats.py)

On each host, the control calls `posix_spawn` and `waitpid` directly for the same 100 sequential external programs. All four executables rotate through the launch order. Timing includes starting the outer executable. Times below are medians in milliseconds. [Control source](../../../tools/bench_spawn.c) · [Complete repeated workloads](throughput.tsv) · [Physical M1 samples](physical-m1.jsonl.gz)

| Host and workload | Previous | Tonari | Bash | Direct C | Tonari/Bash |
| --- | ---: | ---: | ---: | ---: | ---: |
| Linux x86-64 · startup | 0.333 | 0.329 | 0.824 | — | 0.39927 |
| Linux x86-64 · stdin | 3.31 | 0.511 | 1.997 | — | 0.25588 |
| Linux x86-64 · spawn (81 trials) | 33.797 | 33.725 | 46.659 | 33.839 | 0.7228 |
| macOS arm64 · startup | 3.132 | 3.191 | 5.762 | — | 0.5538 |
| macOS arm64 · stdin | 6.523 | 4.051 | 8.451 | — | 0.47935 |
| macOS arm64 · spawn (81 trials) | 193.572 | 190.631 | 280.807 | 188.052 | 0.67887 |
| Apple M1 · startup | 3.332 | 3.261 | 5.834 | — | 0.55896 |
| Apple M1 · stdin | 5.442 | 3.487 | 6.994 | — | 0.49857 |
| Apple M1 · spawn (81 trials) | 102.878 | 102.97 | 165.856 | 101.542 | 0.62084 |

`sh` reads these assignments from a seekable fd 0. For an existing ordinary scalar, Tonari can evaluate a whole-line literal or a reference to another stored scalar without parsing a new command or performing a seek on each line. A program, builtin or trap that can consume input sees the correct file offset. The separate input-size experiment uses 600-byte and 16,000-byte values and interleaves assignments with a real `read`. [Shared-input measurements](shared-input.tsv) · [Buffer-size experiment](https://github.com/sanohiro/tonari-shell/actions/runs/45638851792) · [Repeated workloads](throughput.tsv)

パイプラインの各段が終了状態を返すだけの場合は、ジョブやシグナル、リダイレクトに子が必要かを判定してからプロセス生成を省きます。外部コマンドを含む段では親の検索キャッシュを変更せずに実行パスを解決でき、リダイレクトはファイルアクションとして子に渡せます。[Pipeline implementation](../../../src/execute.rs) · [Observed functions and cost review](sites.tsv)

At `e024141`, with `set -b`, the editor blocks until keyboard input, a child status change, or the next preview deadline. The PTY check times a keystroke, releases a child through a FIFO and verifies that the notification preserves the command being edited. この測定では通知を待っている間の入力と、その後に実行したコマンドの内容も確認しています。[PTY samples](https://github.com/sanohiro/tonari-shell/actions/runs/45646272752) · [Medians and p95](notify.tsv)

| Host | Previous notice ms | Release notice ms | Bash notice ms | Release key p95 ms |
| --- | ---: | ---: | ---: | ---: |
| Linux x86-64 | 50.27794 | 0.385844 | 0.4114 | 0.036007 |
| macOS arm64 | 52.355708 | 1.997834 | 2.267916 | 0.101458 |
| Apple M1 | 52.137959 | 3.608084 | 3.259292 | 0.074708 |

Linux の致命シグナルは、シェルに到達するまでと、シグナル送信直前に書いた1バイトを親が観測してから終了を回収するまでの2区間を計測しました。`core_pattern` には systemd-coredump が指定されています。生成テストの所要時間は、この OS 側の終了処理も含みます。上流の `umask` の一部は実ファイルを作って `ls | cut` を起動し、`set-p.tst:128` は5段の外部 `grep` を実行します。対応する組み込みだけを5,000回実行する比較も記録しました。[Signal intervals](signal-stages.tsv) · [Builtin-only workloads](builtin-costs.tsv) · [Diagnostic run](https://github.com/sanohiro/tonari-shell/actions/runs/45641370257)

An executable text file without a shebang makes the kernel return `ENOEXEC`. When a noninteractive parent was reading its script on stdin, its private reader forced a fresh Tonari executable to load again. The forked child now drops its own inherited private readers before starting a fresh shell session. User descriptors survive and the parent continues from the correct input offset. A third prototype that remembered the last such pathname gave no further improvement on the physical M1. [Exact release comparison](executable-text.tsv) · [Native samples](https://github.com/sanohiro/tonari-shell/actions/runs/45645909959)

| Host and plain script workload | Previous ms | Release ms | Bash ms | Tonari/Bash |
| --- | ---: | ---: | ---: | ---: |
| Linux x86-64 · plain-script | 0.737 | 0.633 | 1.097 | 0.57703 |
| Linux x86-64 · plain-script-30 | 11.248 | 8.052 | 10.094 | 0.7977 |
| macOS arm64 · plain-script | 14.922 | 9.117 | 13.19 | 0.69121 |
| macOS arm64 · plain-script-30 | 163.538 | 36.455 | 48.932 | 0.74501 |
| Apple M1 · plain-script | 6.356 | 4.117 | 6.87 | 0.59927 |
| Apple M1 · plain-script-30 | 90.736 | 24.655 | 30.342 | 0.81257 |

The original `simple-p.tst:201` executes a text file with a slash in its pathname. All 17 runs on the physical M1 matched the upstream assertions. Before and after the fix it took 6.511 and 4.357 ms respectively, against 6.862 ms for Bash. [Original upstream fixture medians](focused.tsv)

A 31-trial Linux signal check with the shipping binary marked the instant before self-delivery. For SIGILL, Tonari reached the marker in 0.972 ms and its parent observed exit 168.875 ms later. Bash took 246.832 ms after the marker. A native C executable took 199.402 ms after the marker for SIGILL and 270.918 ms for SIGQUIT. For SIGBUS the start-to-marker / marker-to-wait medians were 1.789 / 254.243 ms for Tonari, 1.845 / 262.141 ms for Bash and 1.615 / 189.21 ms for C. The original SIGQUIT case also starts an interactive monitored shell and a background job. The direct control isolates the signal termination portion. [Signal timings](fatal-signal-control.tsv) · [Raw samples](https://github.com/sanohiro/tonari-shell/actions/runs/45663405999)

The stripped native release is 1,992,152 bytes (+33,344 versus the previous release) on Linux x86-64, 1,499,760 bytes (+33,248 versus the previous release) on macOS arm64. Linux ships as a static musl executable and the arm64 Mach-O links libSystem. [Sizes and hashes](binary-sizes.tsv)

The earlier macOS sweep found two slow cases that both set PATH before launching executable text without a shebang. Tonari previously had to load its executable again on every call. At `b2bb185`, the forked interpreter could apply exported PATH and SHLVL, including their removal, before starting a fresh script session. The table uses the original complete quote and unset fixture files on CI. The physical M1 reproduced the input, arguments, fixture files and exit/output checks, with balanced invocation order. Times are medians in milliseconds. [Full sample timings](script-search-path.tsv) · [Native upstream rerun](https://github.com/sanohiro/tonari-shell/actions/runs/45654408197)

| Host | Case | Trials | Before | After | Bash |
| --- | ---: | ---: | ---: | ---: | ---: |
| Linux x86-64 | quote-p.tst:724 | 17 | 1.845 | 1.946 | 2.644 |
| Linux x86-64 | unset-p.tst:84 | 17 | 1.633 | 1.721 | 2.436 |
| macOS arm64 | quote-p.tst:724 | 17 | 12.619 | 7.253 | 10.602 |
| macOS arm64 | unset-p.tst:84 | 17 | 10.839 | 4.545 | 8.749 |
| Apple M1 | unset-p.tst:84 | 81 | 12.973 | 6.353 | 9.589 |
| Apple M1 | quote-p.tst:724 | 81 | 11.827 | 7.450 | 11.289 |
| Apple M1 | unset-only | 81 | 6.164 | 6.146 | 15.913 |

Macで遅かった関数削除と引用符のケースは、どちらもPATHを更新してからシバンのない外部ファイルを3～4回実行しています。子シェルに渡す検索パスの変更と削除を保持し、読み直していた実行ファイルを省きました。関数の登録・削除だけを2,000組繰り返した実測も併記しています。

At `2959a51`, Tonari began applying the prepared PWD and OLDPWD export changes. The three cases below change directory, create real executable files and invoke them through a relative or reassigned PATH. Each CI and physical M1 comparison ran the entire original simple-command fixture 17 times per executable and used `b2bb185` for the previous column. The caller retains its own environment. [Per-case samples](working-directory-scripts.tsv) · [Native rerun](https://github.com/sanohiro/tonari-shell/actions/runs/45658029247)

| Host | Case | Trials | Before ms | After ms | Bash ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Linux x86-64 | simple-p.tst:223 | 17 | 3.059 | 3.166 | 4.200 |
| Linux x86-64 | simple-p.tst:237 | 17 | 4.945 | 5.143 | 6.904 |
| Linux x86-64 | simple-p.tst:258 | 17 | 4.613 | 4.800 | 6.441 |
| macOS arm64 | simple-p.tst:223 | 17 | 16.925 | 14.175 | 19.700 |
| macOS arm64 | simple-p.tst:237 | 17 | 32.870 | 25.017 | 33.015 |
| macOS arm64 | simple-p.tst:258 | 17 | 27.871 | 22.605 | 31.345 |
| Apple M1 | simple-p.tst:223 | 17 | 14.194 | 11.240 | 17.418 |
| Apple M1 | simple-p.tst:237 | 17 | 24.994 | 18.934 | 28.660 |
| Apple M1 | simple-p.tst:258 | 17 | 23.768 | 17.478 | 26.103 |

On native Linux x86-64 these three medians increased by 0.107–0.198 ms versus the preceding build. All three finished ahead of Bash. Linux ではこの変更に小さなコストがあり、OS ごとの分岐は追加していません。

`cd` による作業ディレクトリの更新も新しいスクリプトに反映します。シンボリックリンクを経由した論理パス、エクスポートの解除、親に残る状態を検証しました。

The set builtin used to send its option table to line-buffered stdout one line at a time. The release groups the small table in one explicit buffer and checks the final flush. The original case also runs command substitution, eval, a redirection and an external diff. The focused comparison measures the complete case independently from rendering and replaying the option list in a loop. [Native samples](https://github.com/sanohiro/tonari-shell/actions/runs/45663406421) · [Repeated timings](option-reports.tsv)

| Host | Workload | Trials | Before ms | After ms | Bash ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Linux x86-64 | set-p.tst:150 | 31 | 1.304 | 1.288 | 2.042 |
| Linux x86-64 | print-options | 21 | 25.756 | 15.457 | 15.089 |
| Linux x86-64 | replay-options | 21 | 15.277 | 15.544 | 45.554 |
| macOS arm64 | set-p.tst:150 | 31 | 6.867 | 6.988 | 11.105 |
| macOS arm64 | print-options | 21 | 35.292 | 10.010 | 47.513 |
| macOS arm64 | replay-options | 21 | 24.466 | 22.247 | 70.684 |
| Apple M1 | set-p.tst:150 | 31 | 6.773 | 6.437 | 10.307 |
| Apple M1 | print-options | 21 | 53.611 | 9.459 | 70.739 |
| Apple M1 | replay-options | 21 | 20.213 | 20.111 | 54.538 |

`set -o` と `set +o` の一覧出力をまとめ、書き込み失敗の終了ステータスを保持しています。出力1,000組、保存したオプションの復元1,000回、上流の単発試験はそれぞれ別に計測しました。

## Reproduction / 再現

Use the pinned upstream checkout, a release built with `tools/build-release.sh` and the same reference Bash version. The first command prepares the upstream controller once in a disposable checkout. The original shell under test remains uninstrumented during timing. The full CI run also uses a separately built observer to map each test to Rust functions. [Host and executable identities](builds.tsv) · [Controller and timer](../../../tools/ci/yash-posix.py) · [Function probe definitions](../../../tools/ci/yash-paths.py)

```sh
git clone --branch 2.60 https://github.com/magicant/yash.git /tmp/tonari-yash
python3 tools/ci/yash-posix.py /tmp/tonari-yash /tmp/tonari-results --prepare \
  --tonari /absolute/path/to/tonari --previous /absolute/path/to/previous-tonari \
  --bash /absolute/path/to/bash --runs 3 --require-bash-passes \
  --inventory docs/compatibility/yash-posix/ubuntu-24.04.tsv.gz
cc -O2 tools/bench_spawn.c -o /tmp/tonari-spawn-control
python3 tools/ci/yash-throughput.py --tonari /absolute/path/to/tonari \
  --first /absolute/path/to/previous-tonari --bash /absolute/path/to/bash \
  --spawn-control /tmp/tonari-spawn-control --case spawn --runs 81
```

For the complete CI measurement, dispatch the native benchmark workflow using suites `external-yash-0` through `external-yash-7`, three trials and previous revision `72c2500` on both native runners. Each shard repeats its above-target inputs on its own runner with the identical binary. Every pull request also runs the 122 pinned files once on Linux, checking the expected case identities and enforcing each Bash pass. 各シャードでは当初の比較と同じマシンで2/3を超えた項目を追試します。[Native workflow](../../../.github/workflows/native-benchmarks.yml) · [Pull-request workflow](../../../.github/workflows/posix-shell.yml)

GitHub reported that a runner could not be acquired for macOS shard 1 and recorded no test steps. The separately dispatched shard used the same source, reference Bash, release and baseline binary hashes. Its initial three trials and all 11-trial replays ran together on its own native runner. macOS の1シャードはランナー割り当て前に取り消されたため、同じバイナリで独立に完走させました。[Runner annotation](https://github.com/sanohiro/tonari-shell/actions/runs/45663406314/job/139378997266) · [Completed measurements](https://github.com/sanohiro/tonari-shell/actions/runs/45669899204)
