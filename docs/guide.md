# Using Tonari / 利用ガイド

<a id="editing"></a>

## Editing and history / 入力と履歴

| Keys | English | 日本語 |
|---|---|---|
| → / Alt-F | At the end of the line, accept the full suggestion or its next shell argument, including quoting. Inside the line, Alt-F moves forward a word. | 行末では候補全体／引用符を含む次の引数を取り込みます。行の途中で Alt-F を押すと次の単語に移動します。 |
| ↑ / ↓ | Recall commands from this session using the text before the cursor. On an empty line, browse chronologically. Moving past the newest match restores the draft and cursor. | カーソルより左の文字列を接頭辞にして、このセッションの履歴をたどります。空行では時系列順です。最新の一致からさらに ↓ を押すと元の入力とカーソル位置に戻ります。 |
| Ctrl-R | Search shared history. Type two fragments in either order to open the list, or press Tab for a single fragment. Enter in the list returns a command to the editor. Enter during single-fragment inline search executes it. | 共有履歴を検索します。2語入力すると一覧になり、1語でも Tab で一覧を開けます。一覧で Enter を押すと選んだ行を編集できます。1語のインライン検索で Enter を押すとその場で実行します。 |
| Tab / Alt-? | Tab completes paths and programmable matches. A second Tab opens a filterable list. Alt-? prints all matches. Try abbreviated directories such as `s/b/c/ker` for `src/backend/cuda/kernel.cu`. | パスやコマンド固有の候補を補完します。もう一度 Tab を押して絞り込むか、Alt-? で全件を表示できます。パスはディレクトリごとに前方一致で省略できます。 |
| Alt-. | Insert the preceding command’s last argument with its quoting. Press again for an older argument. After `less 'research notes.txt'`, type `rg ERROR ` and press Alt-. to reuse the filename. | 直前のコマンドの末尾引数を引用符ごと挿入します。続けて押すと一つ古いコマンドの引数に替わります。 |
| Ctrl-_ / Ctrl-X Ctrl-U | Undo one edit in the default Emacs keymap. An accepted suggestion, completion, or bracketed paste is one edit. | 標準の Emacs キーマップで編集を一手戻します。履歴候補や補完の採用、複数行の貼り付けは一回で戻せます。 |
| Ctrl-Space, Alt-W, Ctrl-Y | Set a mark, copy the region, then paste it. Insert toggles overwrite mode. Bracketed paste leaves multiline input editable until Enter. | 範囲の始点を置き、選択範囲をコピーして貼り付けます。Insert で上書きモードを切り替えます。ブラケットペースト対応端末では複数行を貼り付けた後も Enter まで編集できます。 |
| Alt-← / Alt-→ | At an empty prompt, move back or forward through directories visited in this shell. | 空のプロンプトで、このシェルで訪れたディレクトリを前後に移動します。 |

With the default history setting, Tonari shares its journal across sessions under `$XDG_STATE_HOME/tonari` or `~/.local/state/tonari`. An explicit alternative `HISTFILE` selects a separate plain history file. 既定ではこのジャーナルをセッション間で共有します。`HISTFILE` に別のファイル名を指定すると、独立したプレーンテキストの履歴を使います。

### Starship

To use an installed Starship prompt, add the following line to your interactive `~/.bashrc`. Starship を使う場合は、対話用の `~/.bashrc` に次の行を追加します。

```bash
eval "$(starship init bash)"
```

With Starship 1.27.0, `eval "$(starship init bash --print-full-init)"` loads the same Bash hook with one fewer Starship launch. この形式では初期化時の Starship 起動が1回減ります。[Starship measurements / 実測値](benchmarks.md#starship-filter)

<a id="build"></a>

## Build and test / ビルドとテスト

For a distribution build, install the pinned toolchain and build on the target OS and architecture. On Linux, install binutils for the binary checks. The script verifies the static musl PIE on Linux and the arm64 executable’s linkage on macOS. 配布用ビルドでは標準ライブラリも同じ設定で最適化します。対象の OS・CPU 上で次を実行してください。Linux のバイナリ検査には binutils を使います。

```sh
case "$(uname -s)" in
  Darwin) target=aarch64-apple-darwin ;;
  Linux) target="$(uname -m)-unknown-linux-musl" ;;
esac
rustup toolchain install nightly-2026-11-20 --profile minimal --component rust-src --target "$target"
binary=$(tools/build-release.sh)
"$binary" --noprofile --norc
```

For development, use `cargo build --locked` and `cargo test --locked`. To run the shell and PTY regression suite, build the executable first and run `python3 tests/shell.py target/debug/tonari`. Use `REFERENCE_BASH` to select the Bash reference. 開発時は通常の Cargo ビルドを使えます。比較対象の Bash は `REFERENCE_BASH` で選べます。[Regression guide / 互換性検証](compatibility.md)
