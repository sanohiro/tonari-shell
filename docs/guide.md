# Using Tonari

[日本語](guide.ja.md)

<a id="editing"></a>

## Editing and history

| Keys | Action |
|---|---|
| → / Alt-F | At the end of the line, accept the full suggestion or its next shell argument, including quoting. Inside the line, Alt-F moves forward a word. |
| ↑ / ↓ | Recall commands from this session using the text before the cursor. On an empty line, browse chronologically. Moving past the newest match restores the draft and cursor. |
| Ctrl-R | Search shared history. Type two fragments in either order to open the list, or press Tab for a single fragment. Enter in the list returns a command to the editor. Enter during single-fragment inline search executes it. |
| Tab / Alt-? | Tab completes paths and programmable matches. A second Tab opens a filterable list. Alt-? prints all matches. Try abbreviated directories such as `s/b/c/ker` for `src/backend/cuda/kernel.cu`. |
| Alt-. | Insert the preceding command’s last argument with its quoting. Press again for an older argument. After `less 'research notes.txt'`, type `rg ERROR ` and press Alt-. to reuse the filename. |
| Ctrl-_ / Ctrl-X Ctrl-U | Undo one edit in the default Emacs keymap. An accepted suggestion, completion, or bracketed paste is one edit. |
| Ctrl-Space, Alt-W, Ctrl-Y | Set a mark, copy the region, then paste it. Insert toggles overwrite mode. Bracketed paste leaves multiline input editable until Enter. |
| Alt-← / Alt-→ | At an empty prompt, move back or forward through directories visited in this shell. |

With the default history setting, Tonari shares its journal across sessions under `$XDG_STATE_HOME/tonari` or `~/.local/state/tonari`. An explicit alternative `HISTFILE` selects a separate plain history file.

### Starship

To use an installed Starship prompt, add the following line to your interactive `~/.bashrc`.

```bash
eval "$(starship init bash)"
```

With Starship 1.27.0, `eval "$(starship init bash --print-full-init)"` loads the same Bash hook with one fewer Starship launch. [Starship measurements](benchmarks.md#starship-filter)

<a id="build"></a>

## Build and test

For a distribution build, install the pinned toolchain and build on the target OS and architecture. The build optimizes the Rust standard library along with Tonari. On Linux, install binutils for the binary checks. The script verifies the static musl PIE on Linux and the arm64 executable’s linkage on macOS.

```sh
case "$(uname -s)" in
  Darwin) target=aarch64-apple-darwin ;;
  Linux) target="$(uname -m)-unknown-linux-musl" ;;
esac
rustup toolchain install nightly-2026-11-20 --profile minimal --component rust-src --target "$target"
binary=$(tools/build-release.sh)
"$binary" --noprofile --norc
```

For development, use `cargo build --locked` and `cargo test --locked`. The shell and terminal regression suite compares against GNU Bash 5.2 or 5.3. Build the release executable first. [Source build instructions](../README.md#install-and-try-it)

On macOS the system ships Bash 3.2. If you installed a newer Bash through Homebrew, select it with `REFERENCE_BASH`.

```sh
REFERENCE_BASH="$(brew --prefix bash)/bin/bash" python3 tests/shell.py target/release/tonari
```

On Linux, if `/bin/bash` is a supported reference version, run `python3 tests/shell.py target/release/tonari`. [Regression guide](compatibility.md)
