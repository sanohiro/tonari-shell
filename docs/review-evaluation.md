# Independent review trial · 2026-11-28

Four fresh Codex contexts reviewed historical native arm64 releases against GNU Bash 5.3.20. The coordinator fixed one target per packet before dispatch. Each packet contained the old Rust source, build inputs, a Bash executable, the review instructions and relevant specialist texts. Reviewers had no inherited conversation and were instructed to read only their packet. This was procedural blinding on a shared host. No Git metadata, subsequent fixes, report, benchmark inventory or answer key was supplied. Reviewers sent their source hypotheses to the coordinator before running probes. The directory reviewer was also asked to survey the ordinary public command forms. The timing includes build waits.

A full hit required the affected site, triggering condition, distinguishing input and cause. An observable difference alone earned no full credit.

| Pre-fix revision and scope | Sealed target and independent evidence | Result |
| --- | --- | --- |
| `ff7e9b4`, arrays | For N compound appends of the form `a+=([k$i]=x)`, another live owner of the array caused the existing entries to be cloned. At 512/1,024/2,048 keys Tonari took 10.18/44.07/166.68 ms. At 2,048 that is 2,096,128 previous entries copied. Direct element assignment was the control. | Full |
| `d9da54c`, directories | After `cd real`, `pushd -` treated the dash as a filename. The reviewer traced the direct directory-change call, checked the status, cwd, OLDPWD and stack, and compared `pushd "$OLDPWD"`. | Full |
| `06a998a`, interactive completion | The reviewer identified repeated PATH enumeration in `visit_commands` and measured rare-prefix completion with one versus four copies of the same directory. The proposed remedy only removed duplicates within one Tab. The sealed case was repeated Tabs with an ordinary list of **distinct** directories. Caching names across Tabs also has to preserve live execute permissions. | Partial, counted as a miss |
| Fresh holdout `0bf6aa0`, inspecting shell state | After the protocol change, the reviewer traced `shopt` output to a line-buffered writer, counted N writes for N named options and 2N when repeated, and compared `shopt -op` with byte-identical `set +o`. A coordinator rerun of the full `shopt -p` counted 56 writes for 56 lines on the old release, one for 59 lines on the current release. | Full |

Initial result **2/3 full**, with one partial. The revised procedure requires an ordinary configuration and counting work both inside a request and across two requests. Its first new holdout was **1/1 full**. These are four selected problems with known answers. There was no unaided control group or estimate of defect prevalence. Among the four scored targets there were zero unsupported diagnoses reported as full hits. The broader hypothesis lists contain genuine rejections. Bash itself rejects `cd ''` in the measured configuration and agrees with Tonari on integer associative initialization against the previous binding. Neither was filed as a defect.

### Findings beyond the answer key

The coordinator replayed these independent witnesses against `c70e173` and the same GNU Bash. They are open shell findings for follow-up. The review integration changes development tooling.

| Witness | Observable result on the current release |
| --- | --- |
| `declare -A m=(first [bracket]=word)`, print `"${m[first]}"` | Tonari prints `word`; Bash prints `[bracket]=word`. Quoting the value makes the outputs agree. |
| Start in directory `real` with inherited `PWD` naming a different, missing directory. Print `"$PWD"`, run `pwd` and `/usr/bin/printenv PWD`. | Tonari repairs the shell's view but passes the stale inherited path to the external command. Bash reports the actual directory for all three. |
| `RANDOM=23; declare -pr >/dev/null; echo "$RANDOM"` | Tonari prints 18581. With the listing omitted both shells print 26116. Bash preserves 26116 with the listing too. Filtering realizes a dynamic value that is absent from the output. |
| Export `SHELLOPTS BASHOPTS`, then `set -o pipefail; shopt -s nullglob`. Inspect `set`, direct expansion and `declare -p`. | Tonari's `set` listing omits the two newly enabled options. Direct expansion and named declarations observe them. Bash lists the enabled options. |

The review also caught historical defects already fixed in the current shell. One altered the number of arguments in `"${a[@]}$((a[2]=7))"`. Another offered an owner-owned mode 0601 command on macOS although executing it returned 126. Both current witnesses now match Bash. A failed rotation into a missing directory produced different subsequent stack contents. A Bash match alone would not justify discarding the previous Tonari stack on failure.

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
