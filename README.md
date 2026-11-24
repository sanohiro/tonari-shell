# Tonari

[日本語](README.ja.md)

Tonari is a Rust shell for modern macOS and Linux, including WSL2. It aims to replace Bash as an everyday login shell. Bash-style commands, startup files and tool integrations run on Tonari’s own interpreter. Interactive editing, history suggestions and completion are built in.

**Tonari is a working prototype.** All 61 Bash builtins pass their primary check on the three reference systems. A login session can load Bash profiles, import exported functions, and run fzf’s Ctrl-T binding. Some Readline commands, shell options and script behaviors still differ. The feature matrix lists the individual results.

The current inventory gives the outcome for every named check on Debian, Ubuntu and macOS. The audit guide links the commands, observed differences and measured timings. [Current inventory](docs/compatibility/current.tsv) · [Audit guide](docs/compatibility.md) · [Intentional differences](docs/compatibility.md#intentional-differences)

## Try it

Development builds require Rust 1.88 or later. Distribution builds use a pinned rustup toolchain to optimize the standard library with the shell. The arm64 release targets the build CPU. Linux x86-64 targets x86-64-v3. Build on the target OS and architecture. Linux also requires binutils for the linker checks.

```sh
case "$(uname -s)" in
  Darwin) target=aarch64-apple-darwin ;;
  Linux) target="$(uname -m)-unknown-linux-musl" ;;
esac
rustup toolchain install nightly-2026-11-20 --profile minimal --component rust-src --target "$target"
binary=$(tools/build-release.sh)
"$binary" --noprofile --norc
```

To build with the installed compiler and its prebuilt standard library, pass `--host-toolchain`. On GNU/Linux install the matching musl target for that compiler first. A plain `cargo build` targets the development host. All release entry points use O2, fat LTO and symbol stripping.

At the prompt, try a pipeline or a shell function. Use ↑ for this session’s history, Ctrl-R to search across sessions, and → at the end of the line to accept a suggestion. Tab completes paths with spaces. If the exact directory is missing, you can abbreviate each level, for example `s/b/c/ker<Tab>` for `src/backend/cuda/kernel.cu`. Insert toggles overwrite mode. Ctrl-Space sets a mark, Alt-W copies the region and Ctrl-Y pastes it. Bracketed paste keeps multiline commands editable until you press Enter. Long commands show a moving window around the cursor.

```bash
count_lines() { wc -l < "$1"; }
for file in README*.md; do printf '%s ' "$file"; count_lines "$file"; done
```

To stage the Bash-compatible entry point under a temporary prefix, run these commands. Both names start the same executable. A nested session inherits `SHELL`.

```sh
prefix=$(mktemp -d)
tools/stage.sh "$binary" "$prefix"
"$prefix/libexec/tonari/bash" --noprofile --norc -c 'printf "%s\n" one two | wc -l'
"$prefix/bin/tonari" --noprofile --norc
```

## Current capabilities

- **Input and state.** Interactive terminals, `-c`, script files, stdin, `source`, and `eval` share the evaluator. Read-only variables, exported environment values, function-local scope, positional parameters, and `cd` persist within their Bash scopes.
- **Language.** Conditional lists, groups, subshells, functions, loops, `case`, `[[ ]]`, arithmetic, command substitution, globs, quoted words, indexed arrays, and associative arrays are implemented. Extended patterns work in conditionals, case arms, filename expansion and parameter operations. Pathname expansion supports `dotglob`, `failglob`, `nullglob`, and recursive `globstar`. Parameter expansion supports indirection such as `${!name}` and `${!#}`. Alias definitions affect subsequently read commands. The parser accepts UTF-8 shell text.
- **Processes.** External commands and concurrent pipelines support ordered redirections, here-documents and here-strings. Streams can carry arbitrary bytes. Foreground job control, Ctrl-C, Ctrl-Z, `jobs`, `bg`, `fg`, background pipelines, `disown`, and `wait` are exercised with real processes. Scripts can inspect `PIPESTATUS`, signal jobs with `kill`, and register signal, `EXIT`, `DEBUG`, `ERR`, and `RETURN` handlers with `trap`.
- **Startup.** Login sessions read `/etc/profile` and the first readable file among `~/.bash_profile`, `~/.bash_login`, and `~/.profile`, in order. A login session runs `~/.bash_logout` on interactive EOF or explicit exit. Interactive non-login sessions read the user’s Bash rc. Noninteractive sessions support `BASH_ENV`. The editor honors `PS1`, `PS2`, and scalar or array `PROMPT_COMMAND`.
- **History and completion.** Tonari keeps its own journal under `$XDG_STATE_HOME/tonari` or `~/.local/state/tonari`. History supports common Bash exclusion and persistence settings. Tab uses Bash definitions registered with `complete`. Interactive shells enable `progcomp_alias` so an alias inherits its command’s completion. When ordinary candidates run out, Tab can recall a path used by another command in the same project, or a previous value for the same command and option. Ctrl-G leaves reverse search and restores the draft and its cursor.

We sourced nvm 0.40.3 and 0.40.5 in Tonari, switched between Node 24.1.0 and 18.13.0 in the same session, and verified each switch with `node --version`. On macOS arm64, a staged login session loaded Debian bash-completion 2.16.0 and the script from GitHub CLI 2.96.0. Pressing Tab after `gh iss` inserted `gh issue `.

Git also completed a branch created by another process after startup, and direnv 2.32.1 applied and removed a directory environment on Debian. An fzf 0.74.0 integration test selected a filename containing a space and Japanese text with Ctrl-T in both Emacs and vi editing modes. The editor supports `bind -x`, key macros, `INPUTRC` and editable `read -e -i` input. Scripts can use `select`, `time` and `coproc`. Named redirections such as `exec {log}>file` allocate a descriptor that the caller can use and close. The `nocasematch` option works in `case`, `[[ ]]`, parameter replacement and completion filters. `shopt -s nocaseglob` makes pathname expansion and `GLOBIGNORE` filters case-insensitive. Advanced Readline commands and Bash 5.3 expansion forms still have differences. Native CI measures Linux x86-64 on Intel and AMD, Linux arm64 and macOS arm64. WSL2 validation and a persistent startup parsing cache remain open. Debian and Ubuntu arm64 also passed actual account login and logout using Tonari’s private `bash` path. [Per-feature results](docs/compatibility.md)

## Speed and size

The feature reference is GNU Bash 5.2.37 on Debian 13 and 5.3.9 on Ubuntu 26.04. For work done by the shell we aim for half of Bash’s elapsed time, with two-thirds as the upper target. Bash-equivalent time is acceptable for very short operations and workloads dominated by the same OS work. The benchmark records startup, external commands, pipelines, and repeated operations separately. The macOS system Bash 3.2 is an additional comparison. [Measurements, binary sizes and build conditions](docs/benchmarks.md)

Linux releases for arm64 and x86-64-v3 are single, static musl executables. They run without glibc, a dynamic loader or an installed musl package. The macOS arm64 executable links to libSystem. [Release sizes and startup measurements](docs/native-2026-11-24/README.md)

## Development

Prefer the Rust standard library or a pure Rust crate for a self-contained operation. Terminal cell width uses `unicode-width`, history searches use `memchr`, and the Linux musl build uses `dlmalloc` for small allocations and its static system allocator for large buffers. File descriptors, terminals and process control use native APIs through the `libc` crate. Evaluate additional native libraries against the required behavior, latency and shipped size. Linux orders pathname expansions by UTF-8 bytes in every locale. macOS follows the active locale. [Dependency policy and filename order](docs/engineering-notes.md#runtime-dependencies)

```sh
cargo test --locked
cargo build --locked
python3 tests/shell.py target/debug/tonari
binary=$(tools/build-release.sh)
python3 tools/bench.py "$binary" --bash /bin/bash
```

The end-to-end tests compare output, files, and exit status with Bash. They also exercise the editor and job control through pseudo-terminals. Use `REFERENCE_BASH` to select a test reference, or `--bash` for benchmarks. Use `--compare-tonari Previous /path/to/previous` to interleave builds in the same run.

Keep English and Japanese documentation in sync. Write commit messages and PR titles and descriptions in English. Record findings and their verification steps in the [engineering notes](docs/engineering-notes.md).
