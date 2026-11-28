# Tonari

[日本語](README.ja.md)

Tonari is a fast, Bash-compatible login shell for macOS and Linux. It runs your Bash scripts and startup files, with fish-style autosuggestions, searchable history, and tab completion built in. It is designed to be your everyday shell in place of Bash or zsh.

Tonari runs Bash syntax in its own Rust interpreter. You can try it from your current shell, keep your existing `~/.bashrc`, and call the system Bash at `/bin/bash` whenever you want.

## At the prompt

| Try this | What happens |
|---|---|
| Type `git sw` | When a history suggestion appears, press → to accept it or Alt-F to take just the next argument. ↑ and ↓ browse commands from this session that start with `git sw`. |
| Press Tab twice | When several completions match, the second Tab opens a list. Type to filter it, use ↑ or ↓ to choose, and press Enter to insert your choice. |
| Press Ctrl-R and type `ssh 15432` | Find commands across sessions containing both fragments, in either order. Select a command to edit, or press Ctrl-G to return to your draft. |

These features work with the default settings. Tab also uses existing Bash completion definitions from tools such as Git and GitHub CLI. [Editing shortcuts and prompt setup](docs/guide.md#editing)

## Install and try it

Build from source with a current stable Rust toolchain and a native linker. Packages for **Homebrew** on macOS, **APT** on Debian/Ubuntu, and installation from crates.io with `cargo install` are planned.

```sh
git clone https://github.com/sanohiro/tonari-shell.git
cd tonari-shell
cargo build --release --locked
./target/release/tonari
```

Type `exit` to return to your previous shell. Tonari targets Apple Silicon Macs and Linux on x86-64 and arm64. WSL2 is an intended target. The optimized Linux distribution build is a single static executable and targets x86-64-v3 on x86-64. [Distribution build instructions](docs/guide.md#build)

## Use Tonari as your login shell

From the checkout above, install the binary under a stable prefix.

```sh
sudo ./tools/stage.sh target/release/tonari /usr/local
```

This installs one executable at `/usr/local/libexec/tonari/bash` and a `/usr/local/bin/tonari` symlink to it. Add the full `/usr/local/libexec/tonari/bash` path on its own line in `/etc/shells`, then select it as your login shell.

```sh
/usr/local/libexec/tonari/bash --noprofile --norc -c : && chsh -s /usr/local/libexec/tonari/bash
```

Log out and back in. Tools that check the basename of `$SHELL` now see `bash`, and scripts can use `BASH_VERSION` to select their Bash integration. You can load `nvm.sh` in your usual Bash rc and run `nvm use`. With GitHub CLI’s Bash completion loaded, Tab after `gh iss` completes `gh issue`. Scripts with a `#!/bin/bash` shebang still run the system Bash. Starting `tonari` inside another shell inherits that session’s `$SHELL`. To switch your login shell back, pass its original path to `chsh -s` and log in again. [Tested tool integrations](docs/benchmarks.md#tool-hooks)

### Your Bash configuration

An interactive non-login shell reads `~/.bashrc`. A login shell reads `/etc/profile`, then the first readable file among `~/.bash_profile`, `~/.bash_login`, and `~/.profile`, in that order. If you keep aliases, completions, or nvm in `~/.bashrc`, you can source it from your login profile.

```bash
if [ -n "${BASH_VERSION-}" ] && [ -r "$HOME/.bashrc" ]; then
    . "$HOME/.bashrc"
fi
```

## Performance and compatibility

| Task (median elapsed time) | Tonari | GNU Bash |
|---|---:|---:|
| First interactive prompt, Apple M1 | 4.02 ms | 8.58 ms |
| Launch 48 external commands with output to `/dev/null`, Linux x86-64 | 14.0 ms | 22.4 ms |
| Write `pwd` to a regular file 1,000 times, Linux x86-64 | 68.7 ms | 65.0 ms |

The M1 comparison used 61 starts per shell and GNU Bash 5.3.20. Both Linux tests used the same native runner. The optimized binaries measured 1.78 MB on macOS arm64 and 2.47 MB on Linux x86-64 in CI. [Measurements and conditions](docs/benchmarks.md#runtime-audit-v2) · [M1 startup results](https://github.com/sanohiro/tonari-shell/pull/18)

Tonari is tested against GNU Bash on Debian, Ubuntu, and macOS. It accepts UTF-8 shell text and passes arbitrary bytes through pipes and redirections. The compatibility guide records differences in scripting and interactive behavior. [Bash compatibility](docs/compatibility.md) · [Intentional differences](docs/compatibility.md#intentional-differences)

## License

Tonari’s original code and documentation are available under the [MIT License](LICENSE). Third-party dependencies and upstream test material retain their respective licenses.
