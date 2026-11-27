# Tonari 利用ガイド

[English](guide.md)

<a id="editing"></a>

## 入力と履歴

| キー | 操作 |
|---|---|
| → / Alt-F | 行末では候補全体／引用符を含む次の引数を取り込みます。行の途中で Alt-F を押すと次の単語に移動します。 |
| ↑ / ↓ | カーソルより左の文字列を接頭辞にして、このセッションの履歴をたどります。空行では時系列順です。最新の一致からさらに ↓ を押すと元の入力とカーソル位置に戻ります。 |
| Ctrl-R | 共有履歴を検索します。2語入力すると一覧になり、1語でも Tab で一覧を開けます。一覧で Enter を押すと選んだ行を編集できます。1語のインライン検索で Enter を押すとその場で実行します。 |
| Tab / Alt-? | パスやコマンド固有の候補を補完します。もう一度 Tab を押して絞り込むか、Alt-? で全件を表示できます。パスはディレクトリごとに前方一致で省略できます。たとえば `s/b/c/ker` で `src/backend/cuda/kernel.cu` を補完できます。 |
| Alt-. | 直前のコマンドの末尾引数を引用符ごと挿入します。続けて押すと一つ古いコマンドの引数に替わります。 たとえば `less 'research notes.txt'` の後、`rg ERROR ` と入力して Alt-. を押すと、そのファイル名を再利用できます。 |
| Ctrl-_ / Ctrl-X Ctrl-U | 標準の Emacs キーマップで編集を一手戻します。履歴候補や補完の採用、複数行の貼り付けは一回で戻せます。 |
| Ctrl-Space, Alt-W, Ctrl-Y | 範囲の始点を置き、選択範囲をコピーして貼り付けます。Insert で上書きモードを切り替えます。ブラケットペースト対応端末では複数行を貼り付けた後も Enter まで編集できます。 |
| Alt-← / Alt-→ | 空のプロンプトで、このシェルで訪れたディレクトリを前後に移動します。 |

既定では、`$XDG_STATE_HOME/tonari` または `~/.local/state/tonari` に保存した履歴をセッション間で共有します。`HISTFILE` に別のファイル名を指定すると、独立したプレーンテキストの履歴を使います。

### Starship

インストール済みの Starship をプロンプトに使うには、対話用の `~/.bashrc` に次の行を追加します。

```bash
eval "$(starship init bash)"
```

Starship 1.27.0 では、`eval "$(starship init bash --print-full-init)"` と書くと同じ Bash 用の設定を読み込みつつ、初期化時の Starship 起動が1回減ります。[Starship の実測値](benchmarks.md#starship-filter)

<a id="build"></a>

## ビルドとテスト

配布用ビルドでは Rust のツールチェーンの版を固定し、標準ライブラリも Tonari と一緒に最適化します。対象の OS・CPU 上で次を実行してください。Linux ではバイナリ検査用に binutils も必要です。スクリプトは Linux では musl を静的リンクした PIE であること、macOS では arm64 実行ファイルのリンク先を確認します。

```sh
case "$(uname -s)" in
  Darwin) target=aarch64-apple-darwin ;;
  Linux) target="$(uname -m)-unknown-linux-musl" ;;
esac
rustup toolchain install nightly-2026-11-20 --profile minimal --component rust-src --target "$target"
binary=$(tools/build-release.sh)
"$binary" --noprofile --norc
```

開発時は `cargo build --locked` でビルドし、`cargo test --locked` で単体テストを実行できます。シェル・端末の回帰試験では GNU Bash 5.2 または 5.3 を比較対象にします。先にリリース版の実行ファイルをビルドしてください。[ソースからのビルド手順](../README.ja.md#インストールして試す)

macOS 標準の Bash は 3.2 です。Homebrew で新しい Bash をインストールしている場合は、次のように `REFERENCE_BASH` で選べます。

```sh
REFERENCE_BASH="$(brew --prefix bash)/bin/bash" python3 tests/shell.py target/release/tonari
```

Linux で `/bin/bash` が対象の版であれば、`python3 tests/shell.py target/release/tonari` で実行できます。[互換性検証](compatibility.md)
