# Recording project knowledge

When an investigation produces a reusable finding or changes a design decision, update [the engineering notes](docs/engineering-notes.md) in English and Japanese. Record what was observed, why it matters to Tonari, and the test, command or measurement that can verify it. Include the environment when it affects the result.

Keep benchmark tables and run conditions in [the benchmark record](docs/benchmarks.md). Link to the relevant evidence from the notes. When new evidence changes a conclusion, date the update and explain what changed.

# Runtime dependencies

Use the Rust standard library or a pure Rust crate for individual algorithms, including editing, Unicode processing, history, completion and expansion. Use native APIs for OS facilities such as process control and terminal I/O. Additional native dependencies need a concrete functional or measured performance reason. Check behavior, latency, binary size and transitive native dependencies when adding a crate. Ship one executable per target. The Linux releases use static musl and must pass the build script’s static PIE and shared dependency checks. Inspect macOS linkage with `otool -L`. Sort Linux pathname expansions by UTF-8 byte order in all locales. Preserve macOS locale collation and explicit completion order such as `complete -o nosort`.

# Checking Bash compatibility and speed

Prefer responsive typing, history and completion, fast startup and lower execution times when selecting a release profile. Use the same implementation across CPU families where possible. Base performance exceptions on an interleaved native comparison with the common implementation on the relevant targets. Use emulation and binary translation for functional checks. Record the executable size alongside those measurements. A modest size increase is acceptable for a meaningful speed improvement. At comparable performance, choose the smaller binary.

Use the reference-derived [feature audit](docs/compatibility.md) when changing shell behavior. Add a direct Bash witness for new functionality, including its observable result. Compare release timing on the same machine and filesystem after the behavior matches. Aim for 1/2 of Bash time, with 2/3 as the upper target for shell work. Accept parity for very short operations and workloads whose elapsed time is dominated by the same OS operation or wait. Use absolute times and run-to-run variation to judge close results. Where a short operation repeats in a script, also measure its throughput in a running shell. Record outstanding differences and target misses in the matrix. Before updating the performance baseline, run the previous-snapshot comparison and inspect newly failing names and modes.

When measuring a tool that ships Bash scripts, run its unmodified integration with the same settings in both shells. Time the operation someone actually waits for and verify its result. If initialization can be deferred, report startup and first use separately so the cost of the first command stays visible.

Bash スクリプトを使うツールは、配布されたスクリプトをそのまま両方のシェルで同じ設定で動かす。ユーザーが待つ操作を計測し、期待した結果が得られることも確認する。遅延読み込みを測る場合は、起動時間と初回操作の待ち時間をそれぞれ記録する。

## Required optimization pass for new or extended features / 機能追加・拡張時の高速化

Before merging a new or extended feature, implement and measure at least one concrete optimization of its first working version, even if that version already meets the Bash target.

1. Verify the result and time the first implementation on a representative workload through the feature’s normal entry point. Include Bash on the same workload when it provides equivalent behavior. For a Tonari-only feature, use the first working implementation as the baseline.
2. Identify a cost in that workload, implement a change to reduce it, then repeat the same correctness check and native release benchmark. For interactive features, time the relevant input through the visible response.
3. Keep the change if it preserves behavior and gives a repeatable improvement. If it does not, keep the better implementation. Record the workload, reproduction command, before/after timings, applicable Bash timing and the reason for the decision in the benchmark record. If Tonari still misses the target, continue tuning where the measurements identify avoidable shell work.

機能を追加・拡張したら、マージ前に必ず、最初に正しく動いた版を基準に少なくとも一度は具体的な高速化を実装して測る。初回から Bash 比の目標を達成した場合も対象とする。実際の使い方で結果を確認し、同じ入力・実機・リリース条件で初回版、改善案、同等機能のある Bash を比較する。対話機能では入力から画面に結果が出るまでを測る。再現する改善を採用し、採否にかかわらず再現コマンド・実測値・判断理由を記録する。未達でシェル側に削れるコストが見えている場合は改善を続ける。

When reviewing a benchmark suite, inspect the implementation reached by every published case, including cases already below the performance target. Record the relevant function, the work the input actually triggers, and whether copying, parsing, lookup, allocation or system calls can be removed. Verify changes through the original workload and a Bash behavior witness. Record unavailable tools and sample failures explicitly.

ベンチマークの見直しでは、目標を達成済みの項目も含め、公開した各ケースが通る実装を確認する。該当関数、入力が実際に行わせる処理、コピー・解析・探索・割り当て・システムコールを省けるかを記録する。変更は元のベンチと Bash の動作照合で検証し、実行できない試験やツールも明記する。

## Independent review / 独立レビュー

Before merging every PR, use the [Tonari review skill](.agents/skills/tonari-review/SKILL.md) and delegate the merge-base diff and its impact to a fresh OpenAI `review-agent`. For critical parser, expansion, process/FD/signal, input, history or completion changes, and before closing any subsystem audit, also delegate related **unchanged** source to a separate reviewer without inherited conversation. Obtain source-derived hypotheses before disclosing prior findings. Verify actionable findings and record their disposition and evidence before closing.

各 PR のマージ前に差分と影響先を独立レビューする。構文解析・展開・プロセス・FD・シグナル・入力・履歴・補完の重要な変更時と、分野別監査を閉じる前には、未変更の関連コードも別のレビュアーに渡す。仮説を受け取ってから既知の所見と照合し、対応と検証結果を記録する。[日本語の運用手順](docs/review.ja.md)

# Intentional differences from Bash

Use Bash as the compatibility reference for supported scripts and tool interfaces. When a Bash behavior breaks a documented Tonari guarantee, keep the Tonari behavior. Record the input, observed Bash and Tonari outputs, and rationale in the compatibility guide and add a regression test. Validate Tonari extensions against their own documented result. Keep observed Bash version differences and outstanding implementation gaps identifiable in the audit.

Bash の不具合や対象外の制約を再現すると Tonari の仕様を壊す場合は、Tonari の仕様を優先します。実際の入力、両者の出力、採用理由を互換性ガイドとテストに残します。Tonari 独自の機能は自身の仕様で検証し、未実装や Bash の版による違いも区別できるようにします。

When Bash and Tonari return the same result, still check whether the behavior is
correct. If both reproduce a bug or the behavior is demonstrably wrong, fix
Tonari and accept the resulting difference from Bash. Keep a minimal witness of
the observable problem, the reason for the expected behavior, and a regression
test for Tonari's corrected result. Apply this rule to jointly failing cases and
to upstream assertions that explicitly expect a Bash bug. Check the ordinary
expectation separately from any shell-specific annotation, including the script's
actual arguments, output, status and side effects. Account for the reference
version, runner and external commands when they affect the result.

Bash と Tonari が同じ結果を返す場合も、挙動の正しさを確認します。双方がバグを
再現する場合や、挙動が明らかにおかしい場合は Tonari を修正し、修正によって
生じる Bash との差を許容します。問題を再現する最小の入力、期待する挙動の根拠、
修正結果を守る回帰テストを残します。両シェルが失敗した試験と、Bash の既知の
バグを期待値にしている試験にも適用します。上流の通常期待値とシェル固有の
注記を個別に照合し、実際の引数、出力、終了状態、副作用を確認します。参照した
Bash の版、ランナー、外部コマンドが結果に影響した場合は、その条件も記録します。

When closing a compatibility audit, follow each discrepancy through the relevant GNU and Tonari code. Fix defects in the supported behavior, including defects shared with Bash. If an upstream driver stops early, exercise its later supported cases independently. Rerun the affected suites on the resulting release, complete the optimization pass above, and record the witness and rationale for each retained difference. Ask the user when the outcome requires a product decision.

互換性監査では差分を両方の実装まで追い、対象範囲の不具合は Bash と共通のものも含めて直す。上流の試験が途中で止まったら、後続の対象ケースを単独で実行する。修正版で再試験と上記の速度検証を行い、残す差には入力・結果・理由を記録する。仕様を選び直す必要がある場合はユーザーに判断を求める。
