# Tonari

[English](README.md)

Tonari は現代の macOS と Linux（WSL2 を含む）向けの Rust 製シェルです。普段使うログインシェルとして Bash を置き換えることを目指しています。Bash 形式のコマンド、起動設定、ツール連携を Tonari 自身が解釈します。対話編集、履歴候補、補完は標準で使えます。

**現在は端末で試せる試作版です。** 参照する3環境すべてで、Bash の組み込み61個がそれぞれの代表テストに一致しました。ログイン端末で Bash のプロファイル、エクスポートした関数、fzf の Ctrl-T を使えます。Readline の一部の編集操作、シェルオプション、スクリプトの実行結果には差があり、各項目の結果を公開しています。

Debian・Ubuntu・macOS で各項目がどこまで動作するかを一覧に掲載しています。検証ガイドには、実行したコマンド、動作差、実測値もあります。[現在の機能別判定](docs/compatibility/current.tsv) · [検証ガイド](docs/compatibility.md) · [Tonari が意図して選んだ動作](docs/compatibility.md#intentional-differences)

## 試す

ビルドには Rust 1.88 以降を使用します。macOS で最小対応版の 1.88.0 でも単体テストを通し、配布用バイナリは 1.96.1 でビルドしました。この例ではビルドするマシンの CPU 向けに最適化します。起動引数を付けると、自分の Bash 設定を読み込む前に標準のエディターを試せます。

```sh
binary=$(tools/build-release.sh)
"$binary" --noprofile --norc
```

プロンプトではパイプや関数を試せます。↑ で現在の端末の履歴をたどり、Ctrl-R で他のセッションを含めて検索します。行末で → を押すと薄く表示された候補を採用します。Tab は空白を含むパスも補完します。ブラケットペースト対応端末では、複数行を貼り付けた後に編集でき、Enter で実行します。

```bash
count_lines() { wc -l < "$1"; }
for file in README*.md; do printf '%s ' "$file"; count_lines "$file"; done
```

次のコマンドで Bash 向けの起動パスを一時ディレクトリに配置できます。どちらの名前でも同じ実行ファイルが直接起動します。別のシェルから起動した Tonari は `SHELL` を継承します。

```sh
prefix=$(mktemp -d)
tools/stage.sh "$binary" "$prefix"
"$prefix/libexec/tonari/bash" --noprofile --norc -c 'printf "%s\n" one two | wc -l'
"$prefix/bin/tonari" --noprofile --norc
```

## いま使える機能

- **入力と状態。** 対話端末、`-c`、ファイル、標準入力、`source`、`eval` が共通の実行系を使います。読み取り専用変数、エクスポートした環境、関数のローカル変数、位置引数、`cd` の結果が Bash のスコープに従って引き継がれます。
- **言語。** 条件リスト、グループ、サブシェル、関数、ループ、`case`、`[[ ]]`、算術演算、コマンド置換、グロブ、引用、添字配列、連想配列を実装しています。拡張パターンは条件式、`case`、ファイル名展開、変数の置換で使えます。パス展開は `dotglob`・`failglob`・`nullglob`・再帰グロブの `globstar` に対応し、間接展開では `${!name}` や `${!#}` を使えます。エイリアスは定義後に読み込んだコマンドに適用されます。コマンド入力は UTF-8 です。
- **プロセス制御。** 外部コマンドと並行動作するパイプで、順序を保つリダイレクト、ヒアドキュメント、ヒアストリングを使えます。ストリームには任意のバイト列を流せます。フォアグラウンド制御、Ctrl-C、Ctrl-Z、`jobs`、`bg`、`fg`、バックグラウンドパイプ、`disown`、`wait` を実プロセスで検証しています。各段の終了値は `PIPESTATUS` で取得できます。`kill` でジョブにシグナルを送り、`trap` でシグナルや `EXIT`・`DEBUG`・`ERR`・`RETURN` の処理を登録できます。
- **起動設定。** ログイン時は `/etc/profile`、続いて `~/.bash_profile`、`~/.bash_login`、`~/.profile` の順で最初に読める一つを実行します。対話ログインシェルを Ctrl-D で閉じたときやログインシェルで明示的に終了したときは `~/.bash_logout` を実行します。対話・非ログイン時はユーザーの Bash rc、非対話時は `BASH_ENV` に対応します。エディターは `PS1`、`PS2`、文字列・配列の `PROMPT_COMMAND` を扱います。
- **履歴と補完。** 独自の履歴は `$XDG_STATE_HOME/tonari` または `~/.local/state/tonari` に保存します。Bash でよく使う記録除外・永続化停止設定に対応します。Tab では `complete` に登録した単語リストや Bash 形式の補完関数を使えます。

nvm 0.40.3 と 0.40.5 をそれぞれ Tonari で読み込み、同じセッションで Node 24.1.0 と 18.13.0 を切り替え、それぞれ `node --version` で確認しました。macOS arm64 のログイン端末で Debian の bash-completion 2.16.0 と GitHub CLI 2.96.0 の補完スクリプトを読み込み、`gh iss` に Tab を押すと `gh issue ` が入力されることを確認しました。

Git では起動後に別プロセスが作ったブランチを補完して切り替え、Debian の direnv 2.32.1 ではディレクトリ移動時の環境変数の設定と解除を確認しました。fzf 0.74.0 の設定を読み込み、Emacs・vi の両モードで Ctrl-T から空白と日本語を含むファイル名を選択できました。エディターでは `bind -x`、キーマクロ、`INPUTRC`、初期値を編集できる `read -e -i` が使えます。スクリプトでは `select`、`time`、`coproc` に対応しました。`exec {log}>file` で確保した記述子は変数を介して使用・解放できます。`shopt -s nocasematch` は `case`、`[[ ]]`、変数の文字列置換、補完フィルターに適用されます。複雑な Readline 操作、ファイル名展開の `nocaseglob`、Bash 5.3 の展開構文には差が残ります。Debian の x86-64-v3 版は QEMU 上で統合テスト124件を通過しました。x86-64 実機での性能、WSL2 での検証、起動設定の解析結果を永続化するキャッシュが残っています。Debian・Ubuntu の arm64 では、一時アカウントに登録した Tonari でログインからログアウトまで確認しました。[機能ごとの結果](docs/compatibility-2026-11-23-response/matrix.md)

## 速度とサイズ

機能互換の基準は Debian 13 の GNU Bash 5.2.37 と Ubuntu 26.04 の 5.3.9 です。シェル自身の処理は Bash の所要時間の半分、遅くても 3 分の 2 を目指します。ごく短い単発処理や、共通の OS 処理・待ち時間が大半を占める処理は、Bash と同程度なら許容します。起動・外部コマンド・パイプ・反復実行を個別に計測し、macOS 標準の Bash 3.2 とも比較します。[計測結果・バイナリサイズ・ビルド条件](docs/benchmarks.md)

どのビルドも、インストールする実行ファイルは1本です。macOS arm64 版と Linux glibc 版は OS の照合順序を使い、シェル内のロケール変更にも従います。Linux は arm64 と x86-64-v3 を対象とします。arm64 の静的 musl 版も用意しました。Debian の実測ではこちらの起動が速く、ファイル名はバイト順になります。ビルドスクリプトはホストのツールチェーンに合う設定を選びます。[配布版の比較と実測](docs/benchmarks.md#interactive-response)

## 開発

入力編集・履歴・補完・展開は Rust で実装し、機能の追加で実行時の共有ライブラリを増やさないことを原則とします。Rust クレートは、必要な動作、その処理にかかる時間、配布バイナリのサイズを確認して採用します。ファイル名展開でコマンドに渡す引数の順序は、標準の macOS 版と glibc 版では現在のロケールの照合順序に従います。オプションの静的 musl 版はバイト順を使うため、名前付きロケールでは配布元の Bash と順序が異なる場合があります。[実行時依存とファイル名の順序](docs/engineering-notes.md#runtime-dependencies)

```sh
cargo test --locked
cargo build --locked
python3 tests/shell.py target/debug/tonari
binary=$(tools/build-release.sh)
python3 tools/bench.py "$binary" --bash /bin/bash
```

結合テストでは、出力、ファイル、終了ステータスを Bash と比較します。エディターとジョブ制御は疑似端末で検証します。テストの比較対象は `REFERENCE_BASH`、ベンチの比較対象は `--bash` で指定できます。`--compare-tonari Previous /path/to/previous` を付けると、旧版と新版を同じ計測内で交互に起動します。

英語と日本語のドキュメントは一緒に更新します。コミットメッセージ、PR のタイトルと本文は英語で書きます。判断の理由と再確認の手順は[開発知見](docs/engineering-notes.md)に記録します。
