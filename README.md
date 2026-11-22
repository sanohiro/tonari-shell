# Tonari

[日本語](README.ja.md)

Tonari is a Rust shell for modern macOS and Linux, including WSL2. It aims to replace Bash as an everyday login shell. Bash-style commands, startup files and tool integrations run on Tonari’s own interpreter. Interactive editing, history suggestions and completion are built in.

**Tonari is a working prototype.** Try it in a nested terminal before using it for your login session. Compatibility with complete Bash configurations and third-party tools is still being tested.

## Try it

Build with Rust 1.87 or later. This example tunes the binary for the CPU of the build machine. The startup flags let you try the editor before loading your Bash configuration.

```sh
RUSTFLAGS='-C target-cpu=native' cargo build --release --locked
./target/release/tonari --noprofile --norc
```

At the prompt, try a pipeline or a shell function. Use ↑ for this session’s history, Ctrl-R to search across sessions, and → at the end of the line to accept a suggestion. Tab completes paths with spaces. Bracketed paste keeps multiline commands editable until you press Enter.

```bash
count_lines() { wc -l < "$1"; }
for file in README*.md; do printf '%s ' "$file"; count_lines "$file"; done
```

To stage the Bash-compatible entry point under a temporary prefix, run these commands. Both names start the same executable. A nested session inherits `SHELL`.

```sh
prefix=$(mktemp -d)
tools/stage.sh target/release/tonari "$prefix"
"$prefix/libexec/tonari/bash" --noprofile --norc -c 'printf "%s\n" one two | wc -l'
"$prefix/bin/tonari" --noprofile --norc
```

## Current capabilities

- **Input and state.** Interactive terminals, `-c`, script files, stdin, `source`, and `eval` share the evaluator. Variables, exported environment values, function-local scope, positional parameters, and `cd` persist within their Bash scopes.
- **Language.** Conditional lists, groups, subshells, functions, loops, `case`, `[[ ]]`, arithmetic, command substitution, globs, quoted words, and indexed arrays are implemented. Alias definitions affect subsequently read commands. The parser accepts UTF-8 shell text.
- **Processes.** External commands and concurrent pipelines support ordered redirections, here-documents and here-strings. Streams can carry arbitrary bytes. Foreground job control, Ctrl-C, Ctrl-Z, `jobs`, `bg`, `fg`, background pipelines, and `wait` are exercised with real processes.
- **Startup.** Login sessions read the system profile and the first readable Bash user profile. Interactive non-login sessions read the user’s Bash rc. Noninteractive sessions support `BASH_ENV`. The editor honors `PS1`, `PS2`, and a scalar `PROMPT_COMMAND`.
- **History and completion.** Tonari keeps its own journal under `$XDG_STATE_HOME/tonari` or `~/.local/state/tonari`. History supports common Bash exclusion and persistence settings. Tab can use Bash word lists and shell completion functions registered with `complete`.

We sourced nvm 0.40.5 in Tonari, switched between Node 24.1.0 and 18.13.0 in the same session, and verified each switch with `node --version`. The script from `gh completion -s bash` (GitHub CLI 2.96.0) registers successfully. Completing `gh iss` to `gh issue` currently needs a bash-completion helper. Loading the full Debian bash-completion library is still pending.

Compatibility work includes more shell options and expansions, associative arrays, signal traps, persistent startup parsing caches, and end-to-end coverage of real tool integrations. WSL2 and x86-64 still need native validation.

## Speed and size

The feature reference is GNU Bash 5.2.37 on Debian 13 and 5.3.9 on Ubuntu 26.04. Our speed target is half the elapsed time of Bash on the same workload, with two-thirds as the upper target. The benchmark records startup, external commands, pipelines, and repeated operations separately. The macOS system Bash 3.2 is an additional comparison. [Measurements, binary sizes and build conditions](docs/benchmarks.md)

Tonari ships as one executable per OS and CPU architecture. Linux arm64 uses static musl. The macOS executable links to OS libraries.

## Development

```sh
cargo test --locked
cargo build --locked
python3 tests/shell.py target/debug/tonari
python3 tools/bench.py target/release/tonari --bash /bin/bash
```

The end-to-end tests compare output, files, and exit status with Bash. They also exercise the editor and job control through pseudo-terminals. Use `REFERENCE_BASH` to select a test reference, or `--bash` for benchmarks. Use `--compare-tonari Previous /path/to/previous` to interleave builds in the same run.

Keep English and Japanese documentation in sync. Write commit messages and PR titles and descriptions in English. Record findings and their verification steps in the [engineering notes](docs/engineering-notes.md).
