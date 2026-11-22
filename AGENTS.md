# Recording project knowledge

When an investigation produces a reusable finding or changes a design decision, update [the engineering notes](docs/engineering-notes.md) in English and Japanese. Record what was observed, why it matters to Tonari, and the test, command or measurement that can verify it. Include the environment when it affects the result.

Keep benchmark tables and run conditions in [the benchmark record](docs/benchmarks.md). Link to the relevant evidence from the notes. When new evidence changes a conclusion, date the update and explain what changed.

# Checking Bash compatibility and speed

Use the reference-derived [feature audit](docs/compatibility.md) when changing shell behavior. Add a direct Bash witness for new functionality, including its observable result. Compare release timing on the same machine and filesystem after the behavior matches. Keep the 1/2 goal and 2/3 upper target per workload. Record outstanding differences and target misses in the matrix. Before updating the performance baseline, run the previous-snapshot comparison and inspect newly failing names and modes.
