# Tonari

[English](README.md)

Tonari は macOS と Linux 向けの高速な Bash 互換ログインシェルです。Bash のスクリプトや設定ファイルを引き継ぎながら、fish のような入力候補、履歴検索、Tab 補完を標準で使えます。Bash や zsh の代わりに、普段のログインシェルとして使えるよう設計しました。

Bash 構文は Rust 製のインタープリターで実行します。今のシェルから気軽に試せて、使い慣れた `~/.bashrc` を引き継げます。必要なときはいつでも `/bin/bash` でシステムの Bash を起動できます。

## プロンプトで試す

| 操作 | できること |
|---|---|
| `git sw` と入力する | 履歴候補が出たら → で候補全体、Alt-F で次の引数だけを取り込めます。↑↓ では、このセッションで `git sw` から始まるコマンドをたどれます。 |
| Tab を2回押す | 補完候補が複数あるときは、2回目の Tab で一覧が開きます。そのまま入力して絞り込み、↑↓ で選び、Enter で編集中の行に挿入できます。 |
| Ctrl-R を押して `ssh 15432` と入力する | どのセッションで実行したかにかかわらず、両方の語を含むコマンドを探せます。語順は自由です。選んだコマンドを編集でき、Ctrl-G で書きかけの行に戻れます。 |

どれも標準設定で使えます。Git や GitHub CLI などの Bash 用補完定義を読み込むと、その候補も Tab で選べます。[編集キーとプロンプトの設定](docs/guide.ja.md#editing)

## インストールして試す

現在はソースからビルドできます。安定版の Rust とネイティブリンカーを用意してください。macOS 向けの **Homebrew**、Debian・Ubuntu 向けの **APT**、crates.io からの `cargo install` による導入も予定しています。

```sh
git clone https://github.com/sanohiro/tonari-shell.git
cd tonari-shell
cargo build --release --locked
./target/release/tonari
```

`exit` で元のシェルに戻れます。対象は Apple Silicon の Mac と、x86-64・arm64 の Linux です。WSL2 での利用も想定しています。Linux の配布用ビルドは静的リンクした実行ファイル1本で、x86-64 版は x86-64-v3 向けです。[配布用ビルドの手順](docs/guide.ja.md#build)

## ログインシェルにする

上の手順でビルドしたら、ソースディレクトリで次を実行します。

```sh
sudo ./tools/stage.sh target/release/tonari /usr/local
```

実体は `/usr/local/libexec/tonari/bash` に入り、`/usr/local/bin/tonari` はそこへのシンボリックリンクになります。`/etc/shells` に `/usr/local/libexec/tonari/bash` を1行追加してから、ログインシェルを変更します。

```sh
/usr/local/libexec/tonari/bash --noprofile --norc -c : && chsh -s /usr/local/libexec/tonari/bash
```

ログインし直すと、`$SHELL` の末尾で判定するツールには `bash` と見えます。スクリプトは `BASH_VERSION` も参照できます。いつもの Bash 設定から `nvm.sh` を読み込めば `nvm use` が使えます。GitHub CLI の Bash 用補完を読み込むと、`gh iss` で Tab を押して `gh issue` を補完できます。先頭が `#!/bin/bash` のスクリプトは従来どおりシステムの Bash が実行します。別のシェルで `tonari` と打って起動した場合、`$SHELL` は親の設定を引き継ぎます。ログインシェルを元に戻すには、元のパスを `chsh -s` に指定してログインし直してください。[ツール連携の検証結果](docs/benchmarks.md#tool-hooks)

### Bash の設定を引き継ぐ

ログインシェル以外で対話起動すると `~/.bashrc` を読みます。ログイン時は `/etc/profile` を読んだ後、`~/.bash_profile`、`~/.bash_login`、`~/.profile` の順に最初に読めるファイルを一つ読みます。エイリアスや補完、nvm の設定を `~/.bashrc` にまとめているなら、Bash と同じようにログインプロファイルから読み込めます。

```bash
if [ -n "${BASH_VERSION-}" ] && [ -r "$HOME/.bashrc" ]; then
    . "$HOME/.bashrc"
fi
```

## 速度と互換性

| 操作（所要時間の中央値） | Tonari | GNU Bash |
|---|---:|---:|
| 最初の対話プロンプトを表示、Apple M1 | 4.02 ms | 8.58 ms |
| 出力を `/dev/null` に向けて外部コマンドを48回起動、Linux x86-64 | 14.0 ms | 22.4 ms |
| `pwd` の結果を通常ファイルに1,000回書き込み、Linux x86-64 | 68.7 ms | 65.0 ms |

M1 では GNU Bash 5.3.20 と各61回比較し、Linux の2項目は同じネイティブランナーで測定しました。CI での配布用バイナリのサイズは macOS arm64 で 1.78 MB、Linux x86-64 で 2.47 MB です。[測定条件と結果](docs/benchmarks.md#runtime-audit-v2) · [M1 での起動計測](https://github.com/sanohiro/tonari-shell/pull/18)

Debian・Ubuntu・macOS 上で GNU Bash と比較しています。シェル構文の文字コードは UTF-8 です。パイプやリダイレクトにはバイナリデータも流せます。スクリプトや対話操作で Bash と挙動が違う箇所は、入力例と一緒に公開しています。[Bash 互換性の検証](docs/compatibility.md) · [意図して違いを残した挙動](docs/compatibility.md#intentional-differences)

## ライセンス

Tonari 独自のコードとドキュメントには [MIT License](LICENSE) を適用します。依存ライブラリや外部由来のテスト資料には、それぞれのライセンスが適用されます。
