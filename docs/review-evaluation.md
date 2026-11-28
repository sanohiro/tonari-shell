# Independent review trial · 2026-11-28

Four fresh Codex contexts reviewed historical native arm64 releases against GNU Bash 5.3.20. The coordinator fixed one target per packet before dispatch. Each packet contained the old Rust source, build inputs, a Bash executable, the review instructions and relevant specialist texts. Reviewers had no inherited conversation and were instructed to read only their packet. This was procedural blinding on a shared host. No Git metadata, subsequent fixes, report, benchmark inventory or answer key was supplied. Reviewers sent their source hypotheses to the coordinator before running probes. The directory reviewer was also asked to survey the ordinary public command forms. The timing includes build waits.

A full hit required the affected site, triggering condition, distinguishing input and cause. An observable difference alone earned no full credit.

| Pre-fix revision and scope | Sealed target and independent evidence | Result |
| --- | --- | --- |
| `ff7e9b4`, arrays | For N compound appends of the form `a+=([k$i]=x)`, another live owner of the array caused the existing entries to be cloned. At 512/1,024/2,048 keys Tonari took 10.18/44.07/166.68 ms. At 2,048 that is 2,096,128 previous entries copied. Direct element assignment was the control. | Full |
| `d9da54c`, directories | After `cd real`, `pushd -` treated the dash as a filename. The reviewer traced the direct directory-change call, checked the status, cwd, OLDPWD and stack, and compared `pushd "$OLDPWD"`. | Full |
| `06a998a`, interactive completion | The reviewer identified repeated PATH enumeration in `visit_commands` and measured rare-prefix completion with one versus four copies of the same directory. The proposed remedy only removed duplicates within one Tab. The sealed case was repeated Tabs with an ordinary list of **distinct** directories. Caching names across Tabs also has to preserve live execute permissions. | Partial, counted as a miss |
| Fresh holdout `0bf6aa0`, inspecting shell state | After the protocol change, the reviewer traced `shopt` output to a line-buffered writer, counted N writes for N named options and 2N when repeated, and compared `shopt -op` with byte-identical `set +o`. A coordinator rerun of the full `shopt -p` counted 56 writes for 56 lines on the old release, one for 59 lines on the current release. | Full |

Initial result **2/3 full**, with one partial. The revised procedure requires an ordinary configuration and counting work both inside a request and across two requests. Its first new holdout was **1/1 full**. These are four selected problems with known answers. There was no unaided control group or estimate of defect prevalence. The partial case received no credit as a full hit. False-positive adjudication also covered the extra diagnoses below. The broader hypothesis lists contain genuine rejections. Bash itself rejects `cd ''` in the measured configuration and agrees with Tonari on integer associative initialization against the previous binding. Neither was filed as a defect.

### Findings beyond the answer key

The coordinator replayed these independent witnesses against `c70e173` and the same GNU Bash. The table records the observed failures on that snapshot. [PR #57](https://github.com/sanohiro/tonari-shell/pull/57) fixes all four in the shell and adds regressions.

| Witness | Observable result before the fix |
| --- | --- |
| `declare -A m=(first [bracket]=word)`, print `"${m[first]}"` | Tonari prints `word`; Bash prints `[bracket]=word`. Quoting the value makes the outputs agree. |
| Start in directory `real` with inherited `PWD` naming a different, missing directory. Print `"$PWD"`, run `pwd` and `/usr/bin/printenv PWD`. | Tonari repairs the shell's view but passes the stale inherited path to the external command. Bash reports the actual directory for all three. |
| `RANDOM=23; declare -pr >/dev/null; echo "$RANDOM"` | Tonari prints 18581. With the listing omitted both shells print 26116. Bash preserves 26116 with the listing too. Filtering realizes a dynamic value that is absent from the output. |
| Export `SHELLOPTS BASHOPTS`, then `set -o pipefail; shopt -s nullglob`. Inspect `set`, direct expansion and `declare -p`. | Tonari's `set` listing omits the two newly enabled options. Direct expansion and named declarations observe them. Bash lists the enabled options. |

The review also caught historical defects already fixed in the current shell. One altered the number of arguments in `"${a[@]}$((a[2]=7))"`. Another offered an owner-owned mode 0601 command on macOS although executing it returned 126. Both current witnesses now match Bash. A failed rotation into a missing directory produced different subsequent stack contents. A Bash match alone would not justify discarding the previous Tonari stack on failure.

### Disposition of additional reports on the historical snapshots

The coordinator checked the release witnesses, raw sample series and corresponding source paths. The directory and completion behavior probes reproduced the archived results on both old Tonari and Bash. The following inventory includes findings outside the four preselected targets. A confirmed observation about an old release does not imply a defect remains on current main.

| Domain | Confirmed observations beyond the target |
| --- | --- |
| Arrays | Earlier quoted array fields changed after later arithmetic in the same word. An associative pair lost its literal prefix. Counting via a nameref serialized every index. Appending text to an array element copied the existing string each time. Ordinary bulk append cloned its already-owned new strings, unlike replacement. The last two were avoidable work even where Tonari beat Bash. |
| Directories | Physical stack changes ignored `set -P` for a symlink followed by `/..`. Startup exposed a stale inherited PWD to a child. A failed status write after `cdable_vars` allowed the process cwd and PWD to disagree. After rebinding DIRSTACK, numeric tilde expansion read the user array. Silent `pushd -n +0` copied the entire stack. In a removed cwd `cd -P .` returned different statuses, and on failed rotation the shells retained different stacks. We recorded the last two as observable differences. |
| Completion | Confirmed the mode-0601 suggestion, eager HOSTFILE scan for a registered static generator, full filename list on a folded first Tab and repeated visits to duplicate PATH components. At 50k/100k/200k hosts the unused lookup took 20.23/45.85/77.56 ms; a prefix without `@` stayed below 0.15 ms at the median. PTY drafts and `compgen` output also confirmed differences for slash-prefixed command paths, duplicate rows, command position after an assignment, editing inside a word, and marking directories after abbreviated completion. Bash's optional duplicate-PATH timing at 8k exceeded its observation window and supplied no timing result. |
| Listings | The read-only filter advanced RANDOM and materialized an alias table absent from the output. Increasing from 0 to 512 aliases took a batch of 256 unchanged filtered listings from 9.3 to 33.2 ms. Exported option strings could be stale in `set`. |

**False positives.** No promoted diagnosis in this inventory was contradicted on its reported historical input. Two additional source-only array ideas (a temporary field vector per literal and conversion of packed mapfile records) lack performance witnesses and were kept out of the confirmed results. Rejected hypotheses included an OLDPWD difference on ordinary `pushd +0`, an empty-stack difference for `pushd -n`, parent-state pollution from POSIX `$(pwd)`, and attribute conversion by `compgen -V`. This evaluation cannot estimate a false-positive rate for unreviewed paths.

The coordinator also checked GNU Bash 5.3 with patches through 020. The directory stack delegates its directory change to `cd_builtin` in `builtins/pushd.def`; startup rebinds the validated directory in `set_pwd` in `variables.c`. POSIX specifies how the shell initializes PWD from an inherited value. Arrays, shopt and pushd are Bash interfaces. [GNU Bash patches](https://ftp.gnu.org/gnu/bash/bash-5.3-patches/) · [POSIX shell variables](https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html)

### Runtime follow-up

The release checks now assert the complete associative value, child-visible PWD, next seeded random value and option listings after enabling **and** disabling each option. An additional source pass found that printing with `declare -gp` selected a hidden outer binding, that completion exposed declared-but-unassigned variables, and that a later assignment could change earlier quoted fields from a live associative table. The witnesses compare argument counts, visible bindings, exit status and the subsequent shell state. The directory tests check that a failed change preserves the stack. The completion tests inspect the draft at the insertion point and distinguish characters supplied by the user or a callback from separators added by the editor. [Compatibility decisions](compatibility.md#independent-review-follow-up) · [Native measurements](benchmarks.md#independent-review-follow-up)

当時の表に記録した4件をシェル本体で修正した。再レビューで見つけた変数の可視性、動的配列の展開順、カーソル途中の補完にも回帰試験を加えた。履歴上の指摘のうち、すでに修正された経路は再現入力で確認した。[修正 PR](https://github.com/sanohiro/tonari-shell/pull/57)

### Profiler and source checks

All nine pinned skill texts and 31 specialist reference files matched the reviewed upstream downloads. The OpenAI Codex 0.171.0 system skill also matched its tagged public source. No profiler plugin was needed on the M1. With Xcode 27.2, Time Profiler exported 1,132 sample rows from a running arithmetic loop. System Trace exported 1,812 thread-state rows during repeated sleep/wc pipelines, including blocked and runnable intervals. We checked Tonari's process identity in each export. These counts validate collection and table selection.

On native hosted Linux, the initial Xeon 6975P-C job with perf policy 4 denied even task-clock. On an EPYC 9V45, the opt-in job temporarily permitted user-mode events and measured 38.07 ms task-clock (three runs, ±1.55%) for 50,000 checked associative appends. The five hardware events including cycles were unavailable on that guest. With the same user-mode policy on another Xeon 6975P-C run, hardware cycles, instructions, branches and branch misses were all available. Its task-clock was 46.80 ms. Counter availability depends on the exposed PMU and the host policy. A separate native EPYC 7763 run compared release wall times against Bash. [Initial probes and release timings](https://github.com/sanohiro/tonari-shell/actions/runs/46143318949) · [AMD counters](https://github.com/sanohiro/tonari-shell/actions/runs/46143751726) · [Intel counters](https://github.com/sanohiro/tonari-shell/actions/runs/46143908032)

### Cost and replay

| Reviewer | Elapsed seconds | Input tokens | Cached input (included) | Output tokens |
| --- | ---: | ---: | ---: | ---: |
| Arrays | 405 | 3,330,076 | 3,244,672 | 11,835 |
| Directories | 361 | 1,850,254 | 1,798,400 | 10,642 |
| Completion | 510 | 3,073,936 | 3,012,992 | 15,697 |
| Holdout | 284 | 1,318,267 | 1,268,864 | 9,596 |
| Total | 1,560 reviewer-seconds | 9,572,533 | 9,324,928 | 47,770 |

The first three ran concurrently. About 12 minutes elapsed from dispatch of the first reviewer through completion of the holdout. Usage is the per-thread final total from the four review sessions, including repeated cached input. Setup, coordinator work, PR review and CI are outside that total. Billing amounts were unavailable.

To replay a case, export `Cargo.toml Cargo.lock src crates tools/build-release.sh` with `git archive REV` into an empty directory and run `tools/build-release.sh` there. Give a fresh reviewer that source, the corresponding release, the instructions and a versioned reference Bash. Keep this document and existing regression tests with the coordinator. Use new hidden cases for the next assessment of discovery. The [benchmark record](benchmarks.md#independent-review-trial) contains the native controls and commands.

**日本語の結果**　初回は3件中2件を自力で特定し、通常の PATH に対する Tab 間の再走査を見逃しました。確認手順を改めた後、別の修正前コードで一覧表示の行ごとの書き込みを特定しました。副次的に現行版でも、連想配列の値の欠落、子に渡る PWD の不整合、参照専用の一覧表示で乱数列が進む問題、`set` の古いオプション表示を確認しました。上の表に観測結果と判別用の入力を残しています。4件は既知の題材を選んだ評価で、全コードでの検出率は未推定です。[日本語の運用手順](review.ja.md)
