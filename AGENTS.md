# Recording project knowledge

When an investigation produces a reusable finding or changes a design decision, update [the engineering notes](docs/engineering-notes.md) in English and Japanese. Record what was observed, why it matters to Tonari, and the test, command or measurement that can verify it. Include the environment when it affects the result.

Keep benchmark tables and run conditions in [the benchmark record](docs/benchmarks.md). Link to the relevant evidence from the notes. When new evidence changes a conclusion, date the update and explain what changed.

# Runtime dependencies

Use the Rust standard library or a pure Rust crate for individual algorithms, including editing, Unicode processing, history, completion and expansion. Use native APIs for OS facilities such as process control and terminal I/O. Additional native dependencies need a concrete functional or measured performance reason. Check behavior, latency, binary size and transitive native dependencies when adding a crate. Ship one executable per target. The Linux releases use static musl and must pass the build script’s static PIE and shared dependency checks. Inspect macOS linkage with `otool -L`. Sort Linux pathname expansions by UTF-8 byte order in all locales. Preserve macOS locale collation and explicit completion order such as `complete -o nosort`.

# Checking Bash compatibility and speed

Prefer responsive typing, history and completion, fast startup and lower execution times when selecting a release profile. Use the same implementation across CPU families where possible. Base performance exceptions on an interleaved native comparison with the common implementation on the relevant targets. Use emulation and binary translation for functional checks. Record the executable size alongside those measurements. A modest size increase is acceptable for a meaningful speed improvement. At comparable performance, choose the smaller binary.

Use the reference-derived [feature audit](docs/compatibility.md) when changing shell behavior. Add a direct Bash witness for new functionality, including its observable result. Compare release timing on the same machine and filesystem after the behavior matches. Aim for 1/2 of Bash time, with 2/3 as the upper target for shell work. Accept parity for very short operations and workloads whose elapsed time is dominated by the same OS operation or wait. Use absolute times and run-to-run variation to judge close results. Where a short operation repeats in a script, also measure its throughput in a running shell. Record outstanding differences and target misses in the matrix. Before updating the performance baseline, run the previous-snapshot comparison and inspect newly failing names and modes.

## Required optimization pass for new or extended features / 機能追加・拡張時の高速化

Before merging a new or extended feature, implement and measure at least one concrete optimization of its first working version, even if that version already meets the Bash target.

1. Verify the result and time the first implementation on a representative workload through the feature’s normal entry point. Include Bash on the same workload when it provides equivalent behavior. For a Tonari-only feature, use the first working implementation as the baseline.
2. Identify a cost in that workload, implement a change to reduce it, then repeat the same correctness check and native release benchmark. For interactive features, time the relevant input through the visible response.
3. Keep the change if it preserves behavior and gives a repeatable improvement. If it does not, keep the better implementation. Record the workload, reproduction command, before/after timings, applicable Bash timing and the reason for the decision in the benchmark record. If Tonari still misses the target, continue tuning where the measurements identify avoidable shell work.

機能を追加・拡張したら、マージ前に必ず、最初に正しく動いた版を基準に少なくとも一度は具体的な高速化を実装して測る。初回から Bash 比の目標を達成した場合も対象とする。実際の使い方で結果を確認し、同じ入力・実機・リリース条件で初回版、改善案、同等機能のある Bash を比較する。対話機能では入力から画面に結果が出るまでを測る。再現する改善を採用し、採否にかかわらず再現コマンド・実測値・判断理由を記録する。未達でシェル側に削れるコストが見えている場合は改善を続ける。

# Intentional differences from Bash

Use Bash as the compatibility reference for supported scripts and tool interfaces. When a Bash behavior breaks a documented Tonari guarantee, keep the Tonari behavior. Record the input, observed Bash and Tonari outputs, and rationale in the compatibility guide and add a regression test. Validate Tonari extensions against their own documented result. Keep observed Bash version differences and outstanding implementation gaps identifiable in the audit.

Bash の不具合や対象外の制約を再現すると Tonari の仕様を壊す場合は、Tonari の仕様を優先します。実際の入力、両者の出力、採用理由を互換性ガイドとテストに残します。Tonari 独自の機能は自身の仕様で検証し、未実装や Bash の版による違いも区別できるようにします。
