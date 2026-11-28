# Specialist sources

From the repository root, `python3 tools/review-skill.py NAME` prints the pinned SKILL.md. Pass a second argument such as `references/flow-a.md` to read a chapter at the same revision. `--list` prints the source, revision, path and license of every candidate. Redirect output into a review packet for offline reading. Only requested text is downloaded. The OpenAI reader first tries the locally installed system skill if its digest matches. A missing download must be reported before claiming a specialist review.

| When | Read | How it applies |
| --- | --- | --- |
| Every PR | `review-agent` | Fresh read-only reviewer, merge-base diff, changed lines and affected callers. Reserve a separate context for the existing-code pass. |
| Establish a performance cause | `rust-performance`, then `references/benchmarking.md` | Check build identity, coverage of the costly path, workload setup and variance. Use Tonari's existing PTY or running-shell harness. |
| Remove a confirmed cost | `rust-hot-path` | Follow `references/allocation-reduction.md` for growth/ownership, `references/hashing-and-io.md` for tables and I/O. Recheck semantics and binary size. |
| Review Rust design | `rust-best-practices` | Start with `references/chapter_01.md` (ownership) and `references/chapter_04.md` (errors). Apply where the change removes a responsibility, intermediate value or inconsistent state. |
| Linux x86-64 CPU attribution | `linux-perf` | Use `references/flow-a.md` for counts and `references/flow-b.md` for stacks. On each Intel **or** AMD host record `uname -m`, `lscpu`, `perf version`, `perf list`, and `cat /proc/sys/kernel/perf_event_paranoid`. Test each candidate with `perf stat -e EVENT -- COMMAND`. Check the count and running percentage. Use supported cycles/instructions, branch or cache events for the hypothesis. Confirm the exact PMU alias on this CPU before requesting vendor-specific events. A denied, multiplexed or unavailable counter is a measurement limit. |
| macOS arm64 CPU or waits | `instruments-profiling` | Use Xcode's `xcrun xctrace list templates`. For CPU use Time Profiler. For child launch, pipelines, I/O and scheduling use System Trace. Launch the exact native executable. Repeat a short operation long enough to appear in samples. Export the TOC, select a schema actually recorded, verify it has target rows. Pair trace attribution with uninstrumented release timings. |
| Bash test design | `bash-ops` | Its `references/footguns.md` and `references/strict-mode-and-traps.md` suggest quoting, pipelines and failure cases. |
| POSIX-mode test design | `posix-shell-pro` | Use the portable parameter, input, and exit-status examples for sh-mode witnesses. Compare the correct invocation and reference version. |

Both shell-writing guides are optional stimulus. Their script-template, installation and lint conventions do not set Tonari's semantics. Use [GNU Bash releases and source](https://ftp.gnu.org/gnu/bash/), the [POSIX.1-2024 shell language](https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html) and the [compatibility audit](../../../../docs/compatibility.md).

We also inspected `profiler-guide` (reference only). Its `profiler_top_functions`, `profiler_cpu_samples` and signpost-query tools require its companion MCP plugin. Xcode supplies recording, table discovery and XPath export directly. The CLI-only guide above covers Tonari's workloads.

```sh
xcrun xctrace record --template 'Time Profiler' --output cpu.trace --launch -- /absolute/path/to/tonari script.sh
xcrun xctrace record --template 'System Trace' --output wait.trace --launch -- /absolute/path/to/tonari pipeline.sh
xcrun xctrace export --input cpu.trace --toc
xcrun xctrace export --input wait.trace --toc
```

For symbol attribution, keep a separate matching optimized build with debug symbols. For Linux, inspect a supported event with `perf record -e EVENT -g -- COMMAND` and `perf report --stdio`. Keep all analysis tools on the developer host. A useful summary reports on-CPU work separately from blocked time.

Intel's catalog repository carries Apache-2.0 at its root. Its linux-perf skill, reference files and upstream provenance identify MIT. Apple's plugin guide declares MIT in its upstream README. The catalog records the license that applies to each text.
