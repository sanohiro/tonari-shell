# Recording project knowledge

When an investigation produces a reusable finding or changes a design decision, update [the engineering notes](docs/engineering-notes.md) in English and Japanese. Record what was observed, why it matters to Tonari, and the test, command or measurement that can verify it. Include the environment when it affects the result.

Keep benchmark tables and run conditions in [the benchmark record](docs/benchmarks.md). Link to the relevant evidence from the notes. When new evidence changes a conclusion, date the update and explain what changed.

# Checking Bash compatibility and speed

Use the reference-derived [feature audit](docs/compatibility.md) when changing shell behavior. Add a direct Bash witness for new functionality, including its observable result. Compare release timing on the same machine and filesystem after the behavior matches. Aim for 1/2 of Bash time, with 2/3 as the upper target for shell work. Accept parity for very short operations and workloads whose elapsed time is dominated by the same OS operation or wait. Use absolute times and run-to-run variation to judge close results. Where a short operation repeats in a script, also measure its throughput in a running shell. Record outstanding differences and target misses in the matrix. Before updating the performance baseline, run the previous-snapshot comparison and inspect newly failing names and modes.

# Intentional differences from Bash

Use Bash as the compatibility reference for supported scripts and tool interfaces. When a Bash behavior breaks a documented Tonari guarantee, keep the Tonari behavior. Record the input, observed Bash and Tonari outputs, and rationale in the compatibility guide and add a regression test. Validate Tonari extensions against their own documented result. Keep observed Bash version differences and outstanding implementation gaps identifiable in the audit.

Bash の不具合や対象外の制約を再現すると Tonari の仕様を壊す場合は、Tonari の仕様を優先します。実際の入力、両者の出力、採用理由を互換性ガイドとテストに残します。Tonari 独自の機能は自身の仕様で検証し、未実装や Bash の版による違いも区別できるようにします。
