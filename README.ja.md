# Tonari

[English](README.md)

Tonari は現代の macOS と Linux（WSL2 を含む）向けの Rust 製シェルです。普段使うログインシェルとして Bash を置き換えることを目指しています。Bash 形式のコマンド、起動設定、ツール連携を Tonari 自身が解釈します。対話編集、履歴候補、補完は標準で使えます。

**現在は端末で試せる試作版です。** ログインシェルに設定する前に、今のシェルから起動して試せます。実際の Bash 設定一式や外部ツールとの互換性は検証中です。

## 試す

Rust 1.87 以降でビルドします。この例ではビルドするマシンの CPU 向けに最適化します。起動引数を付けると、自分の Bash 設定を読み込む前に標準のエディターを試せます。

```sh
RUSTFLAGS='-C target-cpu=native' cargo build --release --locked
./target/release/tonari --noprofile --norc
```

プロンプトではパイプや関数を試せます。↑ で現在の端末の履歴をたどり、Ctrl-R で他のセッションを含めて検索します。行末で → を押すと薄く表示された候補を採用します。Tab は空白を含むパスも補完します。ブラケットペースト対応端末では、複数行を貼り付けた後に編集でき、Enter で実行します。

```bash
count_lines() { wc -l < "$1"; }
for file in README*.md; do printf '%s ' "$file"; count_lines "$file"; done
```

次のコマンドで Bash 向けの起動パスを一時ディレクトリに配置できます。どちらの名前でも同じ実行ファイルが直接起動します。別のシェルから起動した Tonari は `SHELL` を継承します。

```sh
prefix=$(mktemp -d)
tools/stage.sh target/release/tonari "$prefix"
"$prefix/libexec/tonari/bash" --noprofile --norc -c 'printf "%s\n" one two | wc -l'
"$prefix/bin/tonari" --noprofile --norc
```

## いま使える機能

- **入力と状態。** 対話端末、`-c`、ファイル、標準入力、`source`、`eval` が共通の実行系を使います。変数、エクスポートした環境、関数のローカル変数、位置引数、`cd` の結果が Bash のスコープに従って引き継がれます。
- **言語。** 条件リスト、グループ、サブシェル、関数、ループ、`case`、`[[ ]]`、算術演算、コマンド置換、グロブ、引用、添字配列を実装しています。パス展開は `dotglob`・`nullglob`・再帰グロブの `globstar` に対応し、間接展開では `${!name}` や `${!#}` を使えます。エイリアスは定義後に読み込んだコマンドに適用されます。コマンド入力は UTF-8 です。
- **プロセス制御。** 外部コマンドと並行動作するパイプで、順序を保つリダイレクト、ヒアドキュメント、ヒアストリングを使えます。ストリームには任意のバイト列を流せます。フォアグラウンド制御、Ctrl-C、Ctrl-Z、`jobs`、`bg`、`fg`、バックグラウンドパイプ、`wait` を実プロセスで検証しています。各段の終了値は `PIPESTATUS` で取得できます。`kill` でジョブにシグナルを送り、`trap` でシグナルや `EXIT` の処理を登録できます。
- **起動設定。** ログイン時はシステムの profile に続いて、読み取り可能な最初の Bash ユーザープロファイルを実行します。対話・非ログイン時はユーザーの Bash rc、非対話時は `BASH_ENV` に対応します。エディターは `PS1`、`PS2`、文字列の `PROMPT_COMMAND` を扱います。
- **履歴と補完。** 独自の履歴は `$XDG_STATE_HOME/tonari` または `~/.local/state/tonari` に保存します。Bash でよく使う記録除外・永続化停止設定に対応します。Tab では `complete` に登録した単語リストや Bash 形式の補完関数を使えます。

実際の nvm 0.40.5 を Tonari で読み込み、同じセッションで Node 24.1.0 と 18.13.0 を切り替え、それぞれ `node --version` で確認しました。GitHub CLI 2.96.0 が `gh completion -s bash` で生成する補完定義も登録できます。`gh iss` から `gh issue` への補完には現状 bash-completion の補助関数が必要です。Debian の bash-completion 一式の読み込みには未対応です。

互換性の残作業には、ブレース展開、拡張グロブ、変数の変換演算子の拡充、連想配列、Bash の `DEBUG`・`ERR`・`RETURN` トラップ、起動時の解析結果の永続キャッシュ、実ツールでの結合検証があります。WSL2 と x86-64 は実機での確認が残っています。

## 速度とサイズ

機能互換の基準は Debian 13 の GNU Bash 5.2.37 と Ubuntu 26.04 の 5.3.9 です。速度は同じ処理で Bash の所要時間の半分、遅くても 3 分の 2 を目指します。起動・外部コマンド・パイプ・反復実行を個別に計測し、macOS 標準の Bash 3.2 とも比較します。[計測結果・バイナリサイズ・ビルド条件](docs/benchmarks.md)

OS・CPU アーキテクチャごとに実行ファイルを1本配布する設計です。macOS 版は OS 標準のライブラリを使います。Linux arm64 版は musl を静的リンクし、ELF の相対再配置情報を圧縮します。[Linux 配布版のビルド方法](docs/benchmarks.md#linux-arm64-static-pie)

## 開発

```sh
cargo test --locked
cargo build --locked
python3 tests/shell.py target/debug/tonari
python3 tools/bench.py target/release/tonari --bash /bin/bash
```

結合テストでは、出力、ファイル、終了ステータスを Bash と比較します。エディターとジョブ制御は疑似端末で検証します。テストの比較対象は `REFERENCE_BASH`、ベンチの比較対象は `--bash` で指定できます。`--compare-tonari Previous /path/to/previous` を付けると、旧版と新版を同じ計測内で交互に起動します。

英語と日本語のドキュメントは一緒に更新します。コミットメッセージ、PR のタイトルと本文は英語で書きます。判断の理由と再確認の手順は[開発知見](docs/engineering-notes.md)に記録します。
