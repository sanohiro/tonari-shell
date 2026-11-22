# Tonari

[日本語](README.ja.md)

Tonari is a shell written in Rust for modern macOS and Linux, including WSL2. Its goal is to replace Bash as a user's everyday shell. Existing tools should be able to use their Bash integration, and an interactive user should get built-in history suggestions, completion and multiline editing on the first launch, with no plugins to install. Fast startup, fast command execution and a small executable are design requirements.

**The first prototype runs foreground commands and pipelines.** It is ready to try in a terminal. Login-shell use, Bash startup files and tool integration are future milestones.

## Try it

Build with Rust 1.87 or later. This build tunes the executable for the machine running the compiler.

```sh
RUSTFLAGS='-C target-cpu=native' cargo build --release --locked
./target/release/tonari -c "printf '%s\n' one two three | wc -l"
./target/release/tonari
```

The first command after the build prints `3`. At the `tonari$ ` prompt, try `ls -1 | wc -l > count`, then `cat count`. This writes a file named `count` in the current directory. Ctrl-C interrupts a foreground pipeline and returns to the prompt. `exit` or Ctrl-D at an empty prompt ends the session.

## What works today

- The terminal and `-c` share a parser and executor. Both accept UTF-8 command text, literal words, single and double quotes, backslash escapes, comments, and commands separated by semicolons or newlines.
- External commands use `PATH` or an explicit path. Pipeline stages run concurrently. Pipes and redirected files carry arbitrary bytes.
- File redirections support `<`, `>` and `>>`. Standard descriptors 0, 1 and 2 can be duplicated or closed. Tonari connects the pipe first, then applies the command's redirections from left to right.
- A pipeline returns the last stage's status. The builtins are `:` and `exit`. A final standalone external command in `-c` can replace the Tonari process directly.
- Interactive input uses the terminal's line discipline. An open quote or trailing pipe continues on the next line.

Variables and expansions, shell functions, `cd`, `source`, `&&` and `||`, file or stdin scripts, background jobs, suspend/resume, history and completion are still ahead. The current CLI has no login mode or Bash-named entry point.

## Compatibility and performance targets

The reference shells for development are GNU Bash 5.2.37 from Debian 13 and 5.3.9 from Ubuntu 26.04. The shared Bash syntax, startup-file rules, shell variables and observable process behaviour form the compatibility target. Tonari implements its own interpreter and process control.

The planned install layout gives tools a runnable path ending in `libexec/tonari/bash`. The user-facing `tonari` command will link to that executable. A nested Tonari session will inherit `SHELL`. Choosing the dedicated Bash path as a login shell will let tools select it through `SHELL`. Integration tests with real tools are still pending.

Tonari aims to start faster, run commands faster and ship a smaller executable than GNU Bash on the same machine. In the first warm-cache Debian arm64 run, median time to the prompt was **0.90 ms** for Tonari and **1.15 ms** for Bash. The executables were **397,968 bytes** and **1,412,096 bytes**, respectively. This baseline covers a shell with two builtins and a minimal prompt. [Full results, p95 and methodology](docs/benchmarks.md)

## Development

```sh
cargo test --locked
cargo build --locked
python3 tests/shell.py target/debug/tonari
python3 tools/bench.py target/release/tonari --bash /bin/bash
```

The end-to-end suite compares output, file contents and exit status with Bash, and exercises Ctrl-C on a pseudo-terminal. To select a Bash binary, set `REFERENCE_BASH` for the tests or pass `--bash` to the benchmark.

Keep the English and Japanese documentation in sync. Write commit messages, PR titles and PR descriptions in English.
