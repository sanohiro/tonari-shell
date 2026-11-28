---
name: tonari-review
description: Independently review Tonari PRs and existing shell subsystems for observable bugs, avoidable work and scaling regressions. Use before merging a PR, after critical changes and before closing a subsystem audit.
---

# Reviewing Tonari

For every PR, delegate the complete merge-base diff and its callers to a fresh, read-only reviewer using OpenAI's `review-agent`. Resolve the specialist texts with the pinned [source catalog](references/sources.md). Use the upstream defect and location criteria for findings introduced by the diff.

For changes to parsing, expansion, processes, file descriptors, signals, editing, history or completion, and at the close of **any subsystem audit**, also review the related existing code. Determine scope from affected behavior and callers, including unchanged fast and fallback paths. Start a new context with no inherited conversation. Supply the source, supported spec and priorities. Keep prior findings, PR narrative, benchmark case names, before/after diffs, implementation conclusions and audit logs with the coordinator. For a blind historical trial export the old source and build inputs to a directory without Git metadata, fixes, regression fixtures or reports. Give the reviewer a matching executable and a versioned reference Bash. Do not let the diff reviewer stand in for this pass.

## Instructions for the independent source reviewer

1. Read the source first. Trace a public shell operation across its callers to storage, output or OS work. Record hypotheses **with the coordinator before** opening existing benchmarks, reading prior findings or measuring. For each, name the function, growing input, work per iteration, expected symptom, and a second input that distinguishes competing explanations. Include an ordinary state/behavior hypothesis and a simplification where relevant. Read specialist guidance as needed.
2. Design the witness before choosing an existing harness. Vary input size at N, 2N and 4N and vary repetitions separately. Count visits, copies, allocations, sorts, parses or system calls from the code or instrumentation. Time ratios alone cannot establish complexity. Start with these independent axes for the affected operation.

   | Operation | Inputs to distinguish |
   | --- | --- |
   | `case` | Branch count, first/last/no match, literal and dynamic patterns |
   | `[[ ]]` | Length, literal/glob/regex, mismatch only at the end |
   | Expansion | Input and output length, match count, ASCII, Japanese and emoji |
   | History, completion, glob | Population, skew, first call, repeated call and first call after mutation |
   | Spawn, pipeline, redirection | Small/large transfers, parent setup, child launch, I/O wait and early reader exit |

3. Exercise the real entry point. Check argv boundaries, status, stderr and the relevant observable state after completion or failure (parent variables, cwd, FDs, files, terminal). Contrast fast/fallback, repeat use, interruption or partial failure where applicable. Give an input that would fail under the alternative interpretation. Treat Bash and POSIX shell-writing recipes as stimulus only. Decide behavior from the supported GNU Bash source and executable, the applicable POSIX provision and Tonari's documented contract. Describe observable intentional differences such as UTF-8, OS or Bash version. Apply the repository's shared-Bash-bug rule.
4. Follow ownership and state lifetime. Look for an entire operation, duplicate rule, conversion, copy or invalidation that can be eliminated. Justify a refactor by the simpler code, inconsistency removed or measured savings. For performance use `rust-performance` for attribution and `rust-hot-path` for changing the proven cost. For structure use Apollo's ownership and error-handling chapters. Choose the platform profiler using the [source catalog](references/sources.md).
5. Return the site, trigger, minimal input, expected and observed result and causal chain for each supported finding. Include falsified hypotheses separately and list any unmeasured dimension. Do not edit the implementation. The coordinator can now disclose other evidence, verify unexpected findings independently, and resolve or correct them.

## Closing the review

Reuse `tests/shell.py`, `tests/review_*.py` through their actual runner, `tools/ci/{shell-scaling,yash-posix,oils-cases}.py`, and the relevant existing `tools/bench_*.py` as applicable. For UI measure key-to-visible response via PTY. Compare native release before/after/Bash interleaved on the same machine, filesystem, environment and input. Collect both running-shell throughput and user wait where startup masks short operations. Prioritize typing stalls, history/completion, startup, large pathname expansion, spawn/pipes/redirection, variables/functions/substitution, then whole scripts. Use the timing target and short/OS-bound exceptions in AGENTS.md. Primary targets are macOS arm64 and Linux x86-64. Rosetta supplies functional evidence only.

Record the reviewed revision, routes, hypotheses, witnesses, successful and rejected changes, timing spread and outstanding limits in existing audit/benchmark records or PR evidence. Recheck the affected routes after fixes. Close when actionable supported defects have dispositions and fixes are verified. For a skill trial the coordinator seals the answer key before dispatch, scores location/trigger/input/cause against it, verifies extra findings, and records hits, misses, false positives, elapsed time and available usage. A revised discovery protocol needs a fresh undisclosed holdout.
