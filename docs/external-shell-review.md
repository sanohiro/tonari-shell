# External shell benchmark implementation review / 外部シェルベンチの項目別レビュー

All 147 previously published workloads are listed below. Each row identifies the code inspected. The two numbers in each platform column are elapsed time relative to the previous Tonari and to Bash. Smaller values are faster. The CSV contains original units, separate physical M1 measurements, compiler-profile controls and the source functions confirmed by runtime coverage. [Measurements and methodology](benchmarks.md#external-case-review) · [Machine-readable data](external-shell-review.csv)

各 OS の欄は **採用版の所要時間 / 旧 Tonari、採用版の所要時間 / Bash** の順です。たとえば 0.750 · 0.500 は旧版より25%短く、Bash の半分の時間です。「—」は比較できなかった値です。項目名を元のスクリプトと対応づけ、実装を確認した理由を最後の欄に記載しました。「該当経路を変更」は今回手を入れた実装を表します。個別の改善幅は測定値で確認できます。

### Reproducing the M1 controls / 対照試験の再現

Use a fresh shell for each snippet. The scalar trials used `LC_ALL=en_US.UTF-8` and `PATH=/usr/bin:/bin`. Run the previous binary, candidate and reference Bash in rotating order and compare the final output as well as elapsed time. These are controls in addition to the upstream workloads.

```sh
# Slice three characters from a long scalar
printf -v s '%65536s' x
s=猫😀abc$s
for ((i=0;i<1800;i++)); do v=${s:1:3}; done
echo "$v"
```

```sh
# Register the same function repeatedly
for ((i=0;i<100000;i++)); do f() { :; }; done
type -t f
```

```sh
# Resolve and capture an external program with redirected input
for ((i=0;i<100;i++)); do v=$(cut -c2- <<<abc); done
echo "$v"
```

The original timing driver accepts one or more exact `--case` filters for unixwin, fumiyas, luish and GNU Hello. The trace driver takes a checkout of the same pinned workload, a build made with `RUSTFLAGS='-C instrument-coverage'`, and a matching pair of `LLVM_PROFDATA` and `LLVM_COV` executables. Source functions recorded during setup and checkpoints can appear in the diagnostic. [Pinned versions and fixtures](../tools/ci/external-shell.sh) · [Timing driver](../tools/bench_external.py) · [Coverage driver](../tools/ci/trace-external-paths.py)

```sh
python3 tools/ci/trace-external-paths.py unixwin /path/to/shellbench ./target/debug/tonari --case funcs.redefine
python3 tools/bench_external.py unixwin /path/to/shellbench ./release/tonari /path/to/bash --case funcs.redefine --runs 15
```



<details>
<summary>shellspec (70)</summary>

| Case | macOS Tonari / previous · Bash | Linux Tonari / previous · Bash | 実際に確認した実装 |
|---|---:|---:|---|
| `assign.sh: positional params` | 0.998 · 0.388 | 0.986 · 0.572 | **既存の経路を確認・維持**。set -- で位置引数を一つに置き換えるコスト。 set -- で1個の位置引数を設定する。準備済みの引数ベクターを引き渡す。 [set](../src/session.rs#L337) |
| `assign.sh: variable` | 0.989 · 0.501 | 1.001 · 0.711 | **既存の経路を確認・維持**。関数内で同じスカラー変数に定数を繰り返し代入する。 既存値へのリテラル代入は文字列バッファを再利用する。local/typeset の宣言は計時ループ外で、反復中は同じ変数を書き換える。空文字も同じ経路。 [assign_existing_literal](../src/session.rs#L752) / [assign_value](../src/session.rs#L662) |
| `assign.sh: local var` | 1.026 · 0.513 | 0.999 · 0.690 | **既存の経路を確認・維持**。local の宣言後にローカル変数へ書き込む。宣言時間は計測外。 既存値へのリテラル代入は文字列バッファを再利用する。local/typeset の宣言は計時ループ外で、反復中は同じ変数を書き換える。空文字も同じ経路。 [assign_existing_literal](../src/session.rs#L752) / [assign_value](../src/session.rs#L662) |
| `assign.sh: local var (typeset)` | 0.901 · 0.490 | 0.995 · 0.682 | **既存の経路を確認・維持**。typeset の宣言後にローカル変数へ書き込む。宣言時間は計測外。 既存値へのリテラル代入は文字列バッファを再利用する。local/typeset の宣言は計時ループ外で、反復中は同じ変数を書き換える。空文字も同じ経路。 [assign_existing_literal](../src/session.rs#L752) / [assign_value](../src/session.rs#L662) |
| `cmp.sh: [ ]` | 0.843 · 0.325 | 0.913 · 0.528 | **該当経路を変更**。[ ] が偽になる if の条件と本体を、反復のたびに複製する処理を削減。 条件と本体を借用し、角括弧の引数個数に応じて比較する。失敗時のステータスを保存する。 [run_if](../src/eval.rs#L205) / [test_args](../src/builtins.rs#L84) |
| `cmp.sh: [[ ]]` | 0.698 · 0.442 | 0.726 · 0.596 | **該当経路を変更**。[[ ]] の固定文字列を直接比較し、if の構文木も再利用。 リテラル右辺はバイト列を直接比較する。!= は結果を反転し、展開されたパターンと nocasematch は専用照合を通す。 [condition](../src/test_expression.rs#L21) / [run_if](../src/eval.rs#L205) |
| `cmp.sh: case` | 1.050 · 0.580 | 1.003 · 0.731 | **既存の経路を確認・維持**。case のリテラルの分岐が一致しない場合のコスト。 case の枝は共有し、静的パターンは読み込み時に用意する。この例は選択語を展開して1個の枝と照合する。 [run_command](../src/eval.rs#L231) / [matches](../src/pattern.rs#L151) |
| `count.sh: posix` | 1.070 · 0.589 | 0.975 · 0.735 | **既存の経路を確認・維持**。算術展開で 1 を加え、通常の変数に書き戻す。 カウンタは保存値から数値を読み、加算した値を通常のスカラーへ代入する。 [calculate](../src/arithmetic.rs#L421) / [expression](../src/arithmetic.rs#L257) |
| `count.sh: typeset -i` | 0.931 · 0.548 | 1.000 · 0.766 | **既存の経路を確認・維持**。同じ算術展開の結果を、整数属性の変数に書き戻す。 typeset -i を付けた代入先は整数属性を維持する。8進や式として解釈すべき値は通常の算術評価に戻す。 [calculate](../src/arithmetic.rs#L421) / [expression](../src/arithmetic.rs#L257) |
| `count.sh: increment` | 1.052 · 0.502 | 0.959 · 0.630 | **該当経路を変更**。整数属性の変数を算術コマンドでインクリメントする。 10進スカラーの後置更新では優先順位の再解析を省く。元の値を返し、readonly・参照・8進値には通常の代入規則を適用する。 [calculate](../src/arithmetic.rs#L421) / [write_scalar_decimal](../src/session.rs#L724) |
| `eval.sh: direct assign` | 0.798 · 0.317 | 0.975 · 0.479 | **該当経路を変更**。関数を再定義して呼び、その中で通常の代入を行う。 同一の定義とソース位置を再登録する場合は関数表を書き換えない。異なる本体・readonly・allexport は再判定する。 [dispatch](../src/eval.rs#L139) / [define_function](../src/session/functions.rs#L45) |
| `eval.sh: eval assign` | 1.074 · 0.292 | 0.969 · 0.358 | **該当経路を変更**。関数を再定義して呼び、その中で固定の eval を実行する。 eval はソースと読取条件が一致する場合だけ解析結果を再利用する。外側で同じ関数を再登録する処理も省いた。 [builtin](../src/input/eval.rs#L6) / [dispatch](../src/eval.rs#L139) |
| `eval.sh: command subs` | 0.0030 · 0.0026 | 0.011 · 0.0059 | **該当経路を変更**。echo だけの関数を再定義し、結果を取得する。親にトラップがあっても不要な子を省ける。 引数なし関数の本体が固定 echo の場合は出力を直接取得する。子で観測できるトレースや変数変更があれば通常の実行に戻す。 [wrapped_capture](../src/subshell.rs#L241) / [capture_simple](../src/subshell.rs#L350) |
| `func.sh: no func` | 0.943 · 0.480 | 1.008 · 0.681 | **既存の経路を確認・維持**。コロンを直接呼ぶ。関数呼び出しと比較する基準。 リテラルの : と true は出力も子生成も不要。反復ではトラップ等がなければ引数準備を再利用する。ShellSpec はトラップ付きで、制御ループも計時する。 [run_literal](../src/execute.rs#L1660) / [run_statements](../src/eval.rs#L24) |
| `func.sh: func` | 1.014 · 0.414 | 0.996 · 0.494 | **該当経路を変更**。コロンだけを含む関数を呼ぶ。定義は計測外。 argv の先頭削除による全引数移動を除去した。残りを位置引数に移し、extdebug の場合だけデバッグ用に複製する。 [call_function](../src/eval.rs#L566) / [enter_function](../src/session.rs#L981) |
| `null.sh: blank` | — · — | — · — | **上流の試験が両OSで不成立**。本体が空で上流の計測器が両シェルとも計時できない。 上流の空の計時本体は両シェルで構文エラー。計時できる処理がない。  |
| `null.sh: assign variable` | 0.957 · 0.540 | 0.992 · 0.697 | **既存の経路を確認・維持**。空のスカラー値を繰り返し代入する。 既存値へのリテラル代入は文字列バッファを再利用する。local/typeset の宣言は計時ループ外で、反復中は同じ変数を書き換える。空文字も同じ経路。 [assign_existing_literal](../src/session.rs#L752) / [assign_value](../src/session.rs#L662) |
| `null.sh: define function` | 0.952 · 0.484 | 0.947 · 0.672 | **該当経路を変更**。同じ名前に関数定義を繰り返し登録する。本体は実行しない。 同一の定義とソース位置を再登録する場合は関数表を書き換えない。異なる本体・readonly・allexport は再判定する。 [dispatch](../src/eval.rs#L139) / [define_function](../src/session/functions.rs#L45) |
| `null.sh: undefined variable` | 0.888 · 0.558 | 1.009 · 0.684 | **既存の経路を確認・維持**。未定義変数に空の既定値を展開し、コマンドの引数が消える場合。 未定義変数の空の代替値は非引用時に引数から消える。代替語は条件が成立したときだけ評価する。 [resolve](../src/word.rs#L143) / [append_fields](../src/word.rs#L351) |
| `null.sh: : command` | 1.030 · 0.488 | 0.999 · 0.689 | **既存の経路を確認・維持**。引数なしのコロンを測る。上流の反復ループも計時に含む。 リテラルの : と true は出力も子生成も不要。反復ではトラップ等がなければ引数準備を再利用する。ShellSpec はトラップ付きで、制御ループも計時する。 [run_literal](../src/execute.rs#L1660) / [run_statements](../src/eval.rs#L24) |
| `output.sh: echo` | 0.838 · 0.470 | 1.004 · 0.669 | **既存の経路を確認・維持**。echo が固定の一行を書き出す。 組み込みで一行を組み立て、fd の復元・子の終了前に出力を完了する。bulk はループ全体に一つのリダイレクト。 [echo](../src/builtins.rs#L16) / [echo_into](../src/builtins.rs#L28) |
| `output.sh: printf` | 1.076 · 0.488 | 1.014 · 0.657 | **既存の経路を確認・維持**。printf が固定の書式と改行を書き出す。 改行エスケープは printf のデコーダで処理する。標準出力は書込失敗を返し、単純な捕捉出力は共通バッファへ書く。 [run](../src/session/printf.rs#L31) / [render](../src/session/printf.rs#L409) |
| `output.sh: print` | — · — | — · — | **上流の試験が両OSで不成立**。両シェルとも上流が呼ぶ print 組み込みに対応していない。 上流は print を要求する。両シェルともコマンドが見つからず、比較値を作れない。  |
| `stringop1.sh: string length` | 0.949 · 0.672 | 0.963 · 0.785 | **該当経路を変更**。ASCII の短い文字列の文字数を求める。 スカラー長は保存値を借用し UTF-8 の文字数を数える。数える前の全文コピーを除いた。配列・特殊変数・参照は別経路。 [value](../src/word/modifier.rs#L75) / [borrowed_scalar](../src/expansion.rs#L101) |
| `stringop2.sh: substr 1 builtin` | 0.841 · 0.472 | 0.962 · 0.692 | **該当経路を変更**。先頭2文字を除く。シェル内蔵の文字列展開を計測。 正の位置は必要な文字まで、負の位置は末尾からたどる。保存値を借用し、抽出する範囲だけをコピーする。 [substring](../src/word/modifier.rs#L332) / [borrowed_scalar](../src/expansion.rs#L101) |
| `stringop2.sh: substr 1 echo \| cut` | 0.467 · 0.512 | 0.627 · 0.369 | **該当経路を変更**。先頭2文字を除く。短い組み込み出力をパイプで cut に渡し、中間シェルを省く。 小さい出力は上限を確認して実パイプへ書き、cut/sed に直接渡す。段間の余分なシェルを除き、EOF と読取側の終了状態を維持する。 [capture_with_writer](../src/execute.rs#L1481) / [capture_external](../src/execute.rs#L1509) |
| `stringop2.sh: substr 1 cut here doc` | 0.756 · 0.751 | 0.751 · 0.420 | **該当経路を変更**。先頭2文字を除く。ヒアドキュメントを cut に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop2.sh: substr 1 cut here str` | 0.581 · 0.506 | 0.757 · 0.423 | **該当経路を変更**。先頭2文字を除く。ヒアストリングを cut に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop2.sh: substr 2 builtin` | 0.870 · 0.587 | 0.952 · 0.674 | **該当経路を変更**。3文字目から3文字を取り出す。シェル内蔵の文字列展開を計測。 正の位置は必要な文字まで、負の位置は末尾からたどる。保存値を借用し、抽出する範囲だけをコピーする。 [substring](../src/word/modifier.rs#L332) / [borrowed_scalar](../src/expansion.rs#L101) |
| `stringop2.sh: substr 2 echo \| cut` | 0.554 · 0.482 | 0.654 · 0.373 | **該当経路を変更**。3文字目から3文字を取り出す。短い組み込み出力をパイプで cut に渡し、中間シェルを省く。 小さい出力は上限を確認して実パイプへ書き、cut/sed に直接渡す。段間の余分なシェルを除き、EOF と読取側の終了状態を維持する。 [capture_with_writer](../src/execute.rs#L1481) / [capture_external](../src/execute.rs#L1509) |
| `stringop2.sh: substr 2 cut here doc` | 0.695 · 0.704 | 0.733 · 0.416 | **該当経路を変更**。3文字目から3文字を取り出す。ヒアドキュメントを cut に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop2.sh: substr 2 cut here str` | 0.495 · 0.465 | 0.744 · 0.411 | **該当経路を変更**。3文字目から3文字を取り出す。ヒアストリングを cut に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop2.sh: substr 2 expr` | — · — | 1.004 · 0.630 | **該当経路を変更**。3文字目から3文字を取り出す。macOS 標準の expr はこの構文に未対応。 部分文字列の抽出は外部 expr が担当する。シェル側は探索・fd 接続・終了待ちを確認。macOS 標準 expr はこの構文に未対応。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop3.sh: str remove ^ shortest builtin` | 0.956 · 0.603 | 0.994 · 0.701 | **既存の経路を確認・維持**。最初のコロンまでを除く。シェル内蔵の文字列展開を計測。 星とリテラル区切りの組は先頭／末尾から一度だけ検索する。最短・最長で向きを切り替え、一般パターンは照合器を使う。 この入力は find を通る。 [trim_boundary](../src/pattern.rs#L310) / [trim](../src/pattern.rs#L293) |
| `stringop3.sh: str remove ^ shortest echo \| cut` | 0.557 · 0.519 | 0.640 · 0.375 | **該当経路を変更**。最初のコロンまでを除く。短い組み込み出力をパイプで cut に渡し、中間シェルを省く。 小さい出力は上限を確認して実パイプへ書き、cut/sed に直接渡す。段間の余分なシェルを除き、EOF と読取側の終了状態を維持する。 [capture_with_writer](../src/execute.rs#L1481) / [capture_external](../src/execute.rs#L1509) |
| `stringop3.sh: str remove ^ shortest cut here doc` | 0.536 · 0.610 | 0.745 · 0.426 | **該当経路を変更**。最初のコロンまでを除く。ヒアドキュメントを cut に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop3.sh: str remove ^ shortest cut here str` | 0.531 · 0.547 | 0.740 · 0.413 | **該当経路を変更**。最初のコロンまでを除く。ヒアストリングを cut に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop3.sh: str remove ^ longest builtin` | 0.995 · 0.536 | 0.997 · 0.668 | **既存の経路を確認・維持**。最後のコロンまでを除く。シェル内蔵の文字列展開を計測。 星とリテラル区切りの組は先頭／末尾から一度だけ検索する。最短・最長で向きを切り替え、一般パターンは照合器を使う。 この入力は rfind を通る。 [trim_boundary](../src/pattern.rs#L310) / [trim](../src/pattern.rs#L293) |
| `stringop3.sh: str remove ^ longest echo \| cut` | 0.498 · 0.547 | 0.634 · 0.370 | **該当経路を変更**。最後のコロンまでを除く。短い組み込み出力をパイプで cut に渡し、中間シェルを省く。 小さい出力は上限を確認して実パイプへ書き、cut/sed に直接渡す。段間の余分なシェルを除き、EOF と読取側の終了状態を維持する。 [capture_with_writer](../src/execute.rs#L1481) / [capture_external](../src/execute.rs#L1509) |
| `stringop3.sh: str remove ^ longest cut here doc` | 0.636 · 0.631 | 0.729 · 0.416 | **該当経路を変更**。最後のコロンまでを除く。ヒアドキュメントを cut に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop3.sh: str remove ^ longest cut here str` | 0.499 · 0.582 | 0.743 · 0.417 | **該当経路を変更**。最後のコロンまでを除く。ヒアストリングを cut に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop3.sh: str remove $ shortest builtin` | 1.177 · 0.659 | 1.009 · 0.696 | **既存の経路を確認・維持**。最後のコロン以降を除く。シェル内蔵の文字列展開を計測。 星とリテラル区切りの組は先頭／末尾から一度だけ検索する。最短・最長で向きを切り替え、一般パターンは照合器を使う。 この入力は rfind を通る。 [trim_boundary](../src/pattern.rs#L310) / [trim](../src/pattern.rs#L293) |
| `stringop3.sh: str remove $ shortest echo \| cut` | 0.631 · 0.574 | 0.647 · 0.375 | **該当経路を変更**。最後のコロン以降を除く。短い組み込み出力をパイプで cut に渡し、中間シェルを省く。 小さい出力は上限を確認して実パイプへ書き、cut/sed に直接渡す。段間の余分なシェルを除き、EOF と読取側の終了状態を維持する。 [capture_with_writer](../src/execute.rs#L1481) / [capture_external](../src/execute.rs#L1509) |
| `stringop3.sh: str remove $ shortest cut here doc` | 0.631 · 0.636 | 0.745 · 0.418 | **該当経路を変更**。最後のコロン以降を除く。ヒアドキュメントを cut に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop3.sh: str remove $ shortest cut here str` | 0.599 · 0.588 | 0.741 · 0.405 | **該当経路を変更**。最後のコロン以降を除く。ヒアストリングを cut に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop3.sh: str remove $ longest builtin` | 1.018 · 0.582 | 1.007 · 0.698 | **既存の経路を確認・維持**。最初のコロン以降を除く。シェル内蔵の文字列展開を計測。 星とリテラル区切りの組は先頭／末尾から一度だけ検索する。最短・最長で向きを切り替え、一般パターンは照合器を使う。 この入力は find を通る。 [trim_boundary](../src/pattern.rs#L310) / [trim](../src/pattern.rs#L293) |
| `stringop3.sh: str remove $ longest echo \| cut` | 0.577 · 0.524 | 0.637 · 0.364 | **該当経路を変更**。最初のコロン以降を除く。短い組み込み出力をパイプで cut に渡し、中間シェルを省く。 小さい出力は上限を確認して実パイプへ書き、cut/sed に直接渡す。段間の余分なシェルを除き、EOF と読取側の終了状態を維持する。 [capture_with_writer](../src/execute.rs#L1481) / [capture_external](../src/execute.rs#L1509) |
| `stringop3.sh: str remove $ longest cut here doc` | 0.577 · 0.603 | 0.745 · 0.418 | **該当経路を変更**。最初のコロン以降を除く。ヒアドキュメントを cut に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop3.sh: str remove $ longest cut here str` | 0.728 · 0.681 | 0.747 · 0.409 | **該当経路を変更**。最初のコロン以降を除く。ヒアストリングを cut に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop4.sh: str subst one builtin` | 1.006 · 0.608 | 0.932 · 0.684 | **該当経路を変更**。最初のコロンを置換。シェル内蔵の文字列展開を計測。 パターンと置換語にエスケープがなければ借用する。初回・先頭・末尾は所有中の文字列を書き換え、全置換では一致箇所をまとめて処理する。 [substitute](../src/word/replace.rs#L16) / [unescape](../src/pattern.rs#L509) |
| `stringop4.sh: str subst one echo \| sed` | 0.560 · 0.512 | 0.731 · 0.480 | **該当経路を変更**。最初のコロンを置換。短い組み込み出力をパイプで sed に渡し、中間シェルを省く。 小さい出力は上限を確認して実パイプへ書き、cut/sed に直接渡す。段間の余分なシェルを除き、EOF と読取側の終了状態を維持する。 [capture_with_writer](../src/execute.rs#L1481) / [capture_external](../src/execute.rs#L1509) |
| `stringop4.sh: str subst one sed here doc` | 0.603 · 0.695 | 0.817 · 0.535 | **該当経路を変更**。最初のコロンを置換。ヒアドキュメントを sed に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop4.sh: str subst one sed here str` | 0.729 · 0.635 | 0.810 · 0.532 | **該当経路を変更**。最初のコロンを置換。ヒアストリングを sed に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop4.sh: str subst all builtin` | 1.046 · 0.569 | 0.995 · 0.636 | **該当経路を変更**。すべてのコロンを置換。シェル内蔵の文字列展開を計測。 パターンと置換語にエスケープがなければ借用する。初回・先頭・末尾は所有中の文字列を書き換え、全置換では一致箇所をまとめて処理する。 [substitute](../src/word/replace.rs#L16) / [unescape](../src/pattern.rs#L509) |
| `stringop4.sh: str subst all echo \| sed` | 0.482 · 0.551 | 0.733 · 0.482 | **該当経路を変更**。すべてのコロンを置換。短い組み込み出力をパイプで sed に渡し、中間シェルを省く。 小さい出力は上限を確認して実パイプへ書き、cut/sed に直接渡す。段間の余分なシェルを除き、EOF と読取側の終了状態を維持する。 [capture_with_writer](../src/execute.rs#L1481) / [capture_external](../src/execute.rs#L1509) |
| `stringop4.sh: str subst all sed here doc` | 0.700 · 0.664 | 0.814 · 0.535 | **該当経路を変更**。すべてのコロンを置換。ヒアドキュメントを sed に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop4.sh: str subst all sed here str` | 0.624 · 0.648 | 0.811 · 0.532 | **該当経路を変更**。すべてのコロンを置換。ヒアストリングを sed に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop4.sh: str subst front builtin` | 1.009 · 0.559 | 0.988 · 0.675 | **該当経路を変更**。先頭の abc: を置換。シェル内蔵の文字列展開を計測。 パターンと置換語にエスケープがなければ借用する。初回・先頭・末尾は所有中の文字列を書き換え、全置換では一致箇所をまとめて処理する。 [substitute](../src/word/replace.rs#L16) / [unescape](../src/pattern.rs#L509) |
| `stringop4.sh: str subst front echo \| sed` | 0.506 · 0.527 | 0.710 · 0.500 | **該当経路を変更**。先頭の abc: を置換。短い組み込み出力をパイプで sed に渡し、中間シェルを省く。 小さい出力は上限を確認して実パイプへ書き、cut/sed に直接渡す。段間の余分なシェルを除き、EOF と読取側の終了状態を維持する。 [capture_with_writer](../src/execute.rs#L1481) / [capture_external](../src/execute.rs#L1509) |
| `stringop4.sh: str subst front here doc` | 0.554 · 0.623 | 0.809 · 0.532 | **該当経路を変更**。先頭の abc: を置換。ヒアドキュメントを sed に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop4.sh: str subst front sed here str` | 0.600 · 0.613 | 0.812 · 0.529 | **該当経路を変更**。先頭の abc: を置換。ヒアストリングを sed に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop4.sh: str subst back builtin` | 1.048 · 0.591 | 0.982 · 0.666 | **該当経路を変更**。末尾の :ghi を置換。シェル内蔵の文字列展開を計測。 パターンと置換語にエスケープがなければ借用する。初回・先頭・末尾は所有中の文字列を書き換え、全置換では一致箇所をまとめて処理する。 [substitute](../src/word/replace.rs#L16) / [unescape](../src/pattern.rs#L509) |
| `stringop4.sh: str subst back echo \| sed` | 0.611 · 0.565 | 0.743 · 0.490 | **該当経路を変更**。末尾の :ghi を置換。短い組み込み出力をパイプで sed に渡し、中間シェルを省く。 小さい出力は上限を確認して実パイプへ書き、cut/sed に直接渡す。段間の余分なシェルを除き、EOF と読取側の終了状態を維持する。 [capture_with_writer](../src/execute.rs#L1481) / [capture_external](../src/execute.rs#L1509) |
| `stringop4.sh: str subst back here doc` | 0.598 · 0.610 | 0.821 · 0.543 | **該当経路を変更**。末尾の :ghi を置換。ヒアドキュメントを sed に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `stringop4.sh: str subst back sed here str` | 0.647 · 0.523 | 0.823 · 0.537 | **該当経路を変更**。末尾の :ghi を置換。ヒアストリングを sed に直接渡す経路を試した。 外部ツールの探索結果は置換内で使い捨てる。短い入力は実 fd で直接起動し、親の hash 表・入力の改行・大入力の処理を維持する。 [capture_external](../src/execute.rs#L1509) / [start](../src/execute.rs#L1297) |
| `subshell.sh: no subshell` | 0.963 · 0.548 | 0.997 · 0.675 | **既存の経路を確認・維持**。組み込み true の単独実行。グループ化の比較基準。 リテラルの : と true は出力も子生成も不要。反復ではトラップ等がなければ引数準備を再利用する。ShellSpec はトラップ付きで、制御ループも計時する。 [run_literal](../src/execute.rs#L1660) / [run_statements](../src/eval.rs#L24) |
| `subshell.sh: brace` | 0.888 · 0.497 | 1.001 · 0.689 | **既存の経路を確認・維持**。波括弧内で true を現在のシェル環境で実行する。 波括弧は現在のシェルで構文木を借用して実行する。子生成とスコープ複製は生じない。 [borrowed_program](../src/eval.rs#L113) / [dispatch](../src/eval.rs#L139) |
| `subshell.sh: subshell` | 0.997 · 0.661 | 0.974 · 0.524 | **既存の経路を確認・維持**。丸括弧内の true は独立した子の環境で実行する。 丸括弧は独立した子で実行し、構文木は借用する。親子のシグナルと終了状態を分ける処理を保つ。 [launch_borrowed](../src/subshell.rs#L591) / [launch_group](../src/subshell.rs#L580) |
| `subshell.sh: command subs` | 0.862 · 0.0020 | 1.016 · 0.0054 | **既存の経路を確認・維持**。true の空出力と終了ステータスをコマンド置換で取得する。 true の捕捉は空出力と成功状態だけで済む。トラップなどの条件を確認してパイプと子生成を省く。 [capture_simple](../src/subshell.rs#L350) / [captured_output](../src/subshell.rs#L206) |
| `subshell.sh: external command` | 1.030 · 0.682 | 0.986 · 0.652 | **既存の経路を確認・維持**。事前に解決した true の実行ファイルを起動し、終了まで待つ。 絶対パスなので PATH 探索はない。実行ファイルの起動と終了待ちは残り、可能なら親の環境をそのまま渡す。 [start](../src/execute.rs#L1297) / [spawn_resolved](../src/execute/spawn.rs#L72) |

</details>

<details>
<summary>fumiyas (17)</summary>

| Case | macOS Tonari / previous · Bash | Linux Tonari / previous · Bash | 実際に確認した実装 |
|---|---:|---:|---|
| `Parameter Expansion 1: "$PARAMETER"` | 0.500 · 0.014 | 1.000 · 0.105 | **既存の経路を確認・維持**。大きな文字列を引用付きの一引数として20回展開する。 引用済みの単独展開は1引数。IFS とファイル探索を通らず、実行用の引数が値を所有する。 [resolve](../src/word.rs#L143) / [append_fields](../src/word.rs#L351) |
| `Parameter Expansion 2: $PARAMETER` | 0.848 · 0.217 | 0.880 · 0.501 | **該当経路を変更**。10万語を20回展開する。グロブがない単独の展開は、既定 IFS で区切り、引数を直接作る。 長い単独展開は空白の位置から最終引数を直接作る。独自 IFS・グロブ・隣接する展開は条件を確認して通常経路へ戻す。 [append_fields](../src/word.rs#L351) / [separators](../src/word.rs#L759) |
| `Parameter Expansion 3: "${PARAMETER##*/}" (modifier)` | 1.042 · 0.299 | 1.014 · 0.335 | **既存の経路を確認・維持**。100万回の basename 相当の展開。最後のスラッシュ検索が効く。 星とリテラル区切りの組は先頭／末尾から一度だけ検索する。最短・最長で向きを切り替え、一般パターンは照合器を使う。 この入力は rfind を通る。 [trim_boundary](../src/pattern.rs#L310) / [trim](../src/pattern.rs#L293) |
| `Array Parameter Expansion 1: "${ARRAY[1]}" (one element)` | 0.973 · 0.040 | 0.952 · 0.013 | **該当経路を変更**。100万要素の配列から異なる5個の添字を繰り返し読む。 通常の非負10進添字は式の解析を省く。8進・式・負の添字は従来の評価。unixwin の添字42は10要素配列の範囲外。 [resolve_index](../src/session/array.rs#L504) / [element](../src/session/array.rs#L242) |
| `Array Parameter Expansion 2: "${ARRAY[@]}" (all elements)` | 1.024 · 0.048 | 0.992 · 0.137 | **既存の経路を確認・維持**。100万要素の配列全体を、引用付きの個別引数として5回展開する。 引用された配列の各要素を最終引数に直接追加する。要素番号の文字列を作り直さず、空要素と疎配列の順序を保つ。 [append_resolved_fields](../src/expansion.rs#L320) / [append_indexed_fields](../src/session/array.rs#L348) |
| `Arithmetic Evaluation 1: let EXPRESSION` | 0.839 · 0.066 | 0.836 · 0.163 | **該当経路を変更**。let 組み込みで500万回インクリメントする。 10進スカラーの後置更新では優先順位の再解析を省く。元の値を返し、readonly・参照・8進値には通常の代入規則を適用する。 [calculate](../src/arithmetic.rs#L421) / [write_scalar_decimal](../src/session.rs#L724) |
| `Arithmetic Evaluation 2: ((EXPRESSION))` | 0.974 · 0.107 | 0.876 · 0.237 | **該当経路を変更**。算術コマンドで500万回インクリメントする。 10進スカラーの後置更新では優先順位の再解析を省く。元の値を返し、readonly・参照・8進値には通常の代入規則を適用する。 [calculate](../src/arithmetic.rs#L421) / [write_scalar_decimal](../src/session.rs#L724) |
| `Arithmetic Expansion 1: $((EXPRESSION))` | 1.010 · 0.425 | 0.992 · 0.602 | **既存の経路を確認・維持**。算術式から変数名を直接参照し、加算結果を書き戻す。 通常の10進変数は文字列を複製せず数値を得る。式の優先順位を評価し、代入式では変数属性と返す値を維持する。 [calculate](../src/arithmetic.rs#L421) / [expression](../src/arithmetic.rs#L257) |
| `Arithmetic Expansion 2: $(($PARAMETER+EXPRESSION))` | 1.015 · 0.517 | 0.995 · 0.754 | **既存の経路を確認・維持**。算術式の中で先に $j を展開し、加算結果を書き戻す。 式内の $j は数値解析前にシェル展開される。空値や式を含む値の扱いが変わるので、単なる変数トークンには置き換えない。 [arithmetic](../src/expansion.rs#L267) / [calculate](../src/arithmetic.rs#L421) |
| `Test 1: [[ EXPRESSION ]]` | 1.009 · 0.502 | 1.004 · 0.596 | **既存の経路を確認・維持**。[[ -d ]] でカレントディレクトリと親を500万回確認する。 どちらの -d も対象ごとの metadata は1回。[[ はリテラルを借用し、[ は終端の ] を検証する。ディレクトリ変更後も正しい結果を得るため stat 結果は使い回さない。 [unary](../src/builtins.rs#L211) / [condition](../src/test_expression.rs#L21) |
| `Test 2: [ EXPRESSION ]` | 1.001 · 0.274 | 0.994 · 0.363 | **既存の経路を確認・維持**。同じディレクトリ検査を [ -d ] 組み込み経由で行う。 どちらの -d も対象ごとの metadata は1回。[[ はリテラルを借用し、[ は終端の ] を検証する。ディレクトリ変更後も正しい結果を得るため stat 結果は使い回さない。 [unary](../src/builtins.rs#L211) / [condition](../src/test_expression.rs#L21) |
| `Fork` | 0.959 · 0.248 | 0.959 · 0.155 | **既存の経路を確認・維持**。空出力のプロセス置換を5万回実行する。生成した fd と子の終了が観測可能。 プロセス置換の fd と PID は観測可能。コマンド終了時に fd を閉じ、wait 用に子の状態を回収する。 [launch](../src/execute/process_substitution.rs#L104) / [finish](../src/execute/process_substitution.rs#L55) |
| `Fork & Exec` | — · — | 1.002 · 0.502 | **既存の経路を確認・維持**。上流の /bin/true を4万回起動し、1回ずつ待つ。 絶対パスなので PATH 探索はない。実行ファイルの起動と終了待ちは残り、可能なら親の環境をそのまま渡す。 [start](../src/execute.rs#L1297) / [spawn_resolved](../src/execute/spawn.rs#L72) |
| `Fork & Exec (/usr/bin/true)` | 1.056 · 0.745 | — · — | **既存の経路を確認・維持**。macOS に実在する /usr/bin/true で4万回起動する。 絶対パスなので PATH 探索はない。実行ファイルの起動と終了待ちは残り、可能なら親の環境をそのまま渡す。 [start](../src/execute.rs#L1297) / [spawn_resolved](../src/execute/spawn.rs#L72) |
| `Iterate Parameters 1; for` | 0.991 · 0.105 | 0.983 · 0.185 | **既存の経路を確認・維持**。位置引数100万個を for で順に処理する。 暗黙の位置引数はループ開始時に一度だけ固定し、本文の shift や set の影響を受けない。明示した語の展開には一時ベクターを作らない。 [run_command](../src/eval.rs#L231) / [repeat](../src/eval.rs#L511) |
| `Iterate Parameters 2: while shift` | 0.976 · 0.185 | 1.000 · 0.263 | **既存の経路を確認・維持**。位置引数10万個を shift で順に消費する。 先頭位置を進め、末尾で shift の失敗状態を返す。毎回残りの引数を詰め直す処理はない。 [shift](../src/session.rs#L314) / [repeat](../src/eval.rs#L511) |
| `Iterate Parameters 3: while ((n++<$#))` | 1.090 · 0.345 | 1.000 · 0.626 | **既存の経路を確認・維持**。位置引数10万個の個数と算術カウンタを while で比較する。 位置引数の個数は O(1)。比較前に $# を展開し、最後に偽になる回も n++ の副作用を保つ。 [expanded](../src/arithmetic.rs#L74) / [arithmetic](../src/expansion.rs#L267) |

</details>

<details>
<summary>unixwin (50)</summary>

| Case | macOS Tonari / previous · Bash | Linux Tonari / previous · Bash | 実際に確認した実装 |
|---|---:|---:|---|
| `startup.empty_c` | 0.859 · 0.532 | 0.868 · 0.355 | **既存の経路を確認・維持**。空の -c で新しいシェルを起動し、終了まで測る。 非対話の -c は端末検出を遅延する。BASH_ENV やエクスポート関数がある場合は処理が必要で、true では組み込み1回も含む。 [invoke](../src/main.rs#L359) / [run_invocation](../src/main.rs#L382) |
| `startup.true_c` | 0.951 · 0.758 | 0.890 · 0.349 | **既存の経路を確認・維持**。-c で新しいシェルを起動し、true を一度実行する。 非対話の -c は端末検出を遅延する。BASH_ENV やエクスポート関数がある場合は処理が必要で、true では組み込み1回も含む。 [invoke](../src/main.rs#L359) / [run_invocation](../src/main.rs#L382) |
| `startup.biglib_source` | 1.147 · 0.724 | 1.025 · 0.893 | **既存の経路を確認・維持**。関数ライブラリを読み込み、定義を登録する。呼び出しは含まない。 ライブラリをまとめて読み、単純な関数は軽い定義表現を使う。別名の新規関数は一つずつ関数表に登録する。 [file_script](../src/input.rs#L75) / [parse](../src/parser/wrapper.rs#L37) |
| `parse.n_simple@3000` | 0.958 · 0.639 | 0.981 · 0.614 | **該当経路を変更**。3000個の単純コマンドとリダイレクトの構文を検査する。 -n は完了した文を順次破棄する。末尾の演算子、未完の複合文を検出する状態は維持し、リダイレクトや関数本体を実行しない。 [file_script](../src/input.rs#L75) / [session_source_at](../src/parser.rs#L452) |
| `parse.n_funcs@1500` | 0.981 · 0.610 | 1.001 · 0.695 | **該当経路を変更**。1500個の関数の定義と本体の構文を検査する。 -n は完了した文を順次破棄する。末尾の演算子、未完の複合文を検出する状態は維持し、リダイレクトや関数本体を実行しない。 [file_script](../src/input.rs#L75) / [session_source_at](../src/parser.rs#L452) |
| `parse.n_mixed@5000` | 0.879 · 0.666 | 1.019 · 0.787 | **該当経路を変更**。5000規模の混合スクリプトを -n で検査する。 -n は完了した文を順次破棄する。末尾の演算子、未完の複合文を検出する状態は維持し、リダイレクトや関数本体を実行しない。 [file_script](../src/input.rs#L75) / [session_source_at](../src/parser.rs#L452) |
| `expand.tilde_home` | 1.160 · 0.431 | 1.011 · 0.449 | **既存の経路を確認・維持**。各シェルに割り当てた HOME の下へ展開できたことを別に照合。 単独の ~ は現在の HOME を読む。ユーザー情報の検索は行わず、次の展開には変更後の HOME を使う。 [tilde](../src/word.rs#L703) / [get](../src/session.rs#L462) |
| `expand.tilde_user` | — · 0.586 | — · 0.701 | **該当経路を変更**。~root/bin の実際の展開先を照合。旧版は文字列をそのまま返していた。 名前付き ~ は OS のアカウント情報を引く。通常はスタック上の領域で足り、ERANGE の場合だけ拡張する。旧版の誤った展開も検出した。 [user_home](../src/word.rs#L715) / [tilde](../src/word.rs#L703) |
| `expand.nested_cmdsubst` | 0.013 · 0.0063 | 0.0086 · 0.0028 | **該当経路を変更**。echo を3重に入れ子にして500回展開。副作用がない場合に fork を省く。 安全な組み込みの引数は同じベクターへ直接展開する。不要な一時ベクターを除き、末尾改行・終了状態・同名関数の挙動を保つ。 [capture_simple](../src/subshell.rs#L350) / [captured_output](../src/subshell.rs#L206) |
| `strings.append_assign` | 1.138 · 0.372 | 0.996 · 0.252 | **該当経路を変更**。+= で6バイトずつ追加し、終了後に文字列全体を照合。 通常のスカラーはバッファを伸ばして追記する。自分自身で始まる代入は、残りの展開が変数を変えない場合だけ適用する。 [take_append_suffix](../src/word.rs#L114) / [append_plain_scalar](../src/session.rs#L643) |
| `strings.concat_subst` | 0.804 · 0.116 | 0.667 · 0.085 | **該当経路を変更**。s=$s$x で、展開中に s が変更されない場合に確保済み領域を再利用。 通常のスカラーはバッファを伸ばして追記する。自分自身で始まる代入は、残りの展開が変数を変えない場合だけ適用する。 [take_append_suffix](../src/word.rs#L114) / [append_plain_scalar](../src/session.rs#L643) |
| `arrays.append_loop` | 1.113 · 0.464 | 0.999 · 0.303 | **既存の経路を確認・維持**。配列へ3000回追加し、全要素を照合。 連続した添字への追加は密な格納を維持する。疎配列や共有状態で必要な複製と区別し、計時外に全3000要素を検証する。 [assign_array](../src/session/array.rs#L383) / [insert](../src/session/array.rs#L108) |
| `arrays.assign_idx_mod` | 1.035 · 0.442 | 1.007 · 0.375 | **既存の経路を確認・維持**。64個の添字へ8000回書き込み、添字と値を照合。 添字式を現在のカウンタで評価して既存スロットを書き換える。ループごとに変わる添字の結果は保存しない。 [assign_element](../src/session/array.rs#L534) / [calculate](../src/arithmetic.rs#L421) |
| `arrays.read_elem` | 0.994 · 0.434 | 0.953 · 0.321 | **該当経路を変更**。要素が10個の配列で42番を読む。存在しない添字の検索を測る。 通常の非負10進添字は式の解析を省く。8進・式・負の添字は従来の評価。unixwin の添字42は10要素配列の範囲外。 [resolve_index](../src/session/array.rs#L504) / [element](../src/session/array.rs#L242) |
| `control.if_true` | 1.035 · 0.489 | 0.888 · 0.291 | **該当経路を変更**。if の真側を繰り返す。解析済みの条件と本体を借用。 条件と両方の枝を借用する。枝の選択前に構文木全体を複製する処理を除き、条件の終了状態と短絡を保つ。 [run_if](../src/eval.rs#L205) / [borrowed_program](../src/eval.rs#L113) |
| `control.if_false_else` | 0.874 · 0.417 | 0.733 · 0.292 | **該当経路を変更**。if の偽側から else へ進む。両方の分岐の複製を省く。 条件と両方の枝を借用する。枝の選択前に構文木全体を複製する処理を除き、条件の終了状態と短絡を保つ。 [run_if](../src/eval.rs#L205) / [borrowed_program](../src/eval.rs#L113) |
| `funcs.call_empty` | 0.994 · 0.271 | 0.937 · 0.209 | **該当経路を変更**。処理がコロンだけの関数を8000回呼び出す。 argv の先頭削除による全引数移動を除去した。残りを位置引数に移し、extdebug の場合だけデバッグ用に複製する。 [call_function](../src/eval.rs#L566) / [enter_function](../src/session.rs#L981) |
| `funcs.call_args` | 1.061 · 0.273 | 0.962 · 0.218 | **該当経路を変更**。固定の4引数を渡して関数を5000回呼び出す。 argv の先頭削除による全引数移動を除去した。残りを位置引数に移し、extdebug の場合だけデバッグ用に複製する。 [call_function](../src/eval.rs#L566) / [enter_function](../src/session.rs#L981) |
| `funcs.redefine` | 0.915 · 0.421 | 0.926 · 0.186 | **該当経路を変更**。同じ関数を3000回再定義し、最後に関数として存在することを確認。 同一の定義とソース位置を再登録する場合は関数表を書き換えない。異なる本体・readonly・allexport は再判定する。 [dispatch](../src/eval.rs#L139) / [define_function](../src/session/functions.rs#L45) |
| `subst.cmd_builtin_echo` | 0.958 · 0.014 | 0.727 · 0.0077 | **該当経路を変更**。echo の出力をコマンド置換で取り出し、最終値を確認。 安全な組み込みの引数は同じベクターへ直接展開する。不要な一時ベクターを除き、末尾改行・終了状態・同名関数の挙動を保つ。 [capture_simple](../src/subshell.rs#L350) / [captured_output](../src/subshell.rs#L206) |
| `subst.cmd_true` | 0.605 · 0.0079 | 0.723 · 0.0049 | **既存の経路を確認・維持**。true の空の出力を500回取得する。 true の捕捉は空出力と成功状態だけで済む。トラップなどの条件を確認してパイプと子生成を省く。 [capture_simple](../src/subshell.rs#L350) / [captured_output](../src/subshell.rs#L206) |
| `subst.cmd_ext_date` | 0.691 · 0.839 | 0.993 · 0.615 | **該当経路を変更**。外部 date を100回起動し、時刻の出力形式を別に検証。 date は実際に起動する。置換内で探索した実行ファイルにパイプを渡し、出力を読み切って終了状態を回収する。親の hash 表は更新しない。 [capture_external](../src/execute.rs#L1509) / [spawn_with_dups](../src/execute/spawn.rs#L37) |
| `redir.devnull_write` | 0.883 · 0.588 | 0.993 · 0.398 | **既存の経路を確認・維持**。毎回 /dev/null を開いて echo で書き込む。 単一出力の echo は開いた fd へ直接書く。毎回の open はファイル・権限・追記位置・fd 上限に影響するため残す。変数版は語の展開も行う。 [run_builtin](../src/execute.rs#L1023) / [open_redirect](../src/execute.rs#L104) |
| `redir.file_append` | 0.955 · 0.884 | 0.978 · 0.428 | **既存の経路を確認・維持**。ファイルに2000回追記し、終了後に内容をすべて照合。 単一出力の echo は開いた fd へ直接書く。毎回の open はファイル・権限・追記位置・fd 上限に影響するため残す。変数版は語の展開も行う。 [run_builtin](../src/execute.rs#L1023) / [open_redirect](../src/execute.rs#L104) |
| `redir.pipe_data_10k` | 1.023 · 0.847 | 0.993 · 0.682 | **既存の経路を確認・維持**。1万行のファイルを cat、cat、wc の3段で流す。全段が外部プロセス。 外部3段はカーネルのパイプで直接転送する。シェルが1万行を読み直してコピーする処理はなく、fd とプロセスの状態を管理する。 [run_pipeline](../src/execute.rs#L2071) / [start](../src/execute.rs#L1297) |
| `glob.star_txt@200` | 1.054 · 0.191 | 1.020 · 0.537 | **該当経路を変更**。tree/*.txt の展開と各一致のカウントを併せて測る。 名前を照合してから一致パスを作り、リテラルのディレクトリ部分を借用する。列挙結果が同じ場合のみ以前の整列順を使う。展開後の件数カウントも計時範囲。 [paths](../src/pattern.rs#L527) / [unescape](../src/pattern.rs#L509) |
| `glob.star_all@300` | 1.485 · 0.226 | 0.957 · 0.507 | **該当経路を変更**。tree/* の展開、整列、全項目のカウントを併せて測る。 星1個の照合でも隠し名を除外し、UTF-8 を確かめて一致パスを作る。新しく列挙した結果が同じ場合のみ順序を再利用する。 [paths](../src/pattern.rs#L527) / [unescape](../src/pattern.rs#L509) |
| `cond.dbracket_str_eq` | 0.999 · 0.317 | 0.753 · 0.223 | **該当経路を変更**。[[ == ]] の右辺がリテラルのときは、パターンを組み立てず直接比較。 リテラル右辺はバイト列を直接比較する。!= は結果を反転し、展開されたパターンと nocasematch は専用照合を通す。 [condition](../src/test_expression.rs#L21) |
| `cond.dbracket_str_ne` | 0.819 · 0.306 | 0.753 · 0.220 | **該当経路を変更**。[[ != ]] の右辺がリテラルのときは、直接比較の結果を反転。 リテラル右辺はバイト列を直接比較する。!= は結果を反転し、展開されたパターンと nocasematch は専用照合を通す。 [condition](../src/test_expression.rs#L21) |
| `jobs.bg_wait_one` | 0.804 · 0.972 | 1.001 · 0.981 | **既存の経路を確認・維持**。子の sleep 50 ms を待つため、この時間が下限になる。 単発は50 ms、20個並行は各20 ms の sleep を実行する。実 PID を登録し、終了待ちとジョブ表の後始末を確認する。 [wait_builtin](../src/jobs.rs#L768) / [wait_for_ids](../src/jobs.rs#L835) |
| `jobs.bg_many@20` | 0.989 · 0.965 | 1.004 · 0.942 | **既存の経路を確認・維持**。20 ms の sleep を20個並行起動して最後の子まで待つ。 単発は50 ms、20個並行は各20 ms の sleep を実行する。実 PID を登録し、終了待ちとジョブ表の後始末を確認する。 [wait_builtin](../src/jobs.rs#L768) / [wait_for_ids](../src/jobs.rs#L835) |
| `data.printf_bulk@10000` | 1.015 · 0.499 | 1.013 · 0.832 | **既存の経路を確認・維持**。整数・桁埋め・小数を含む書式を1万回処理する。 整数の桁揃えと浮動小数の変換は数値書式を通る。%s の高速経路はこの行には当たらない。外側のリダイレクトは1回。 [render](../src/session/printf.rs#L409) / [run](../src/session/printf.rs#L31) |
| `data.echo_bulk@10000` | 1.069 · 0.468 | 0.994 · 0.488 | **既存の経路を確認・維持**。外側で一度リダイレクトし、カウンタを展開して1万行出力する。 組み込みで一行を組み立て、fd の復元・子の終了前に出力を完了する。bulk はループ全体に一つのリダイレクト。 [echo](../src/builtins.rs#L16) / [echo_into](../src/builtins.rs#L28) |
| `mixed.log_tally@20000` | 0.967 · 0.500 | 1.000 · 0.619 | **該当経路を変更**。ログ2万行を分割して連想配列へ集計し、全キーと件数を照合。 read は入力中の範囲でフィールドを分け、連想配列はキーで更新する。整列は列挙時に限り、計時外では全キーの集計値を照合する。 [field_ranges](../src/session/read.rs#L458) / [assign_key](../src/session/assoc.rs#L99) |
| `mixed.config_parse@2000` | 1.038 · 0.698 | 1.025 · 0.818 | **既存の経路を確認・維持**。設定2000行を読み、case と eval で代入する。設定値を照合。 行ごとに eval の代入テキストが変わる。case の固定パターンを再利用し、入力行と eval はその時点の設定で評価する。 [run](../src/session/read.rs#L208) / [run_command](../src/eval.rs#L231) |
| `mixed.arg_parse@1000` | 1.000 · 0.651 | 0.894 · 0.380 | **既存の経路を確認・維持**。固定の6引数を一度解析する。名前の1000は実行回数を表さない。 case の枝は解析済み、shift は位置を進める。上流の名前にある1000に対し、実際は6引数の1回の解析。 [run_command](../src/eval.rs#L231) / [shift](../src/session.rs#L314) |
| `coproc.cat_roundtrip` | 0.972 · 0.691 | 1.026 · 0.530 | **既存の経路を確認・維持**。cat のコプロセスと500往復し、最後の返信を照合。 コプロセスは読み書きの2つの fd を保持する。パイプは巻き戻せず、read は次の利用者のデータまで先読みできない。tr のフィクスチャはバッファなしで動かす。 [launch](../src/execute/coprocess.rs#L60) / [run](../src/session/read.rs#L208) |
| `coproc.tr_roundtrip` | 1.097 · 0.715 | 0.966 · 0.542 | **既存の経路を確認・維持**。tr のコプロセスと300往復し、変換後の返信を照合。 コプロセスは読み書きの2つの fd を保持する。パイプは巻き戻せず、read は次の利用者のデータまで先読みできない。tr のフィクスチャはバッファなしで動かす。 [launch](../src/execute/coprocess.rs#L60) / [run](../src/session/read.rs#L208) |
| `errors.cmd_not_found` | 0.971 · 0.056 | 0.981 · 0.045 | **既存の経路を確認・維持**。見つからないコマンドを500回実行し、診断の行数も照合。 全候補が存在しない場合、同じコマンド内で子が再探索する処理を除く。次の反復までに実行ファイルが作られる可能性があるため、失敗は持ち越さない。 [search](../src/execute/hash.rs#L57) / [resolve](../src/execute/hash.rs#L84) |
| `errors.cmd_not_found_path` | 0.973 · 0.058 | 0.987 · 0.065 | **既存の経路を確認・維持**。実在しないディレクトリを含む PATH で500回検索し、診断を照合。 全候補が存在しない場合、同じコマンド内で子が再探索する処理を除く。次の反復までに実行ファイルが作られる可能性があるため、失敗は持ち越さない。 [search](../src/execute/hash.rs#L57) / [resolve](../src/execute/hash.rs#L84) |
| `builtin.echo_plain` | 1.119 · 0.664 | 0.987 · 0.394 | **既存の経路を確認・維持**。リテラルを echo して毎回 /dev/null にリダイレクト。 単一出力の echo は開いた fd へ直接書く。毎回の open はファイル・権限・追記位置・fd 上限に影響するため残す。変数版は語の展開も行う。 [run_builtin](../src/execute.rs#L1023) / [open_redirect](../src/execute.rs#L104) |
| `builtin.echo_var` | 0.979 · 0.692 | 1.006 · 0.476 | **既存の経路を確認・維持**。変数を展開して echo し、毎回 /dev/null にリダイレクト。 単一出力の echo は開いた fd へ直接書く。毎回の open はファイル・権限・追記位置・fd 上限に影響するため残す。変数版は語の展開も行う。 [run_builtin](../src/execute.rs#L1023) / [open_redirect](../src/execute.rs#L104) |
| `builtin.shift_one` | 1.149 · 0.362 | 0.982 · 0.231 | **既存の経路を確認・維持**。一度 shift した後で4個の引数を再設定する。 set -- は引数ベクターの所有権を移し、shift は先頭位置を進める。unixwin の反復には4引数の再設定も含む。 [set](../src/session.rs#L337) / [shift](../src/session.rs#L314) |
| `arith.expand_add` | 1.105 · 0.419 | 0.975 · 0.407 | **既存の経路を確認・維持**。算術展開 a+b の結果を通常の変数に代入する。 a と b の通常の10進値はコピーせず読み、算術展開を終えた後に x へ代入する。 [calculate](../src/arithmetic.rs#L421) / [expression](../src/arithmetic.rs#L257) |
| `arith.cmd_add` | 0.979 · 0.298 | 1.003 · 0.242 | **既存の経路を確認・維持**。算術コマンドの内部で a+b を代入し、終了ステータスも求める。 算術式内で和を x に書き込む。スカラーの保存領域を再利用し、結果の数値からコマンドの終了状態を決める。 [calculate](../src/arithmetic.rs#L421) / [expression](../src/arithmetic.rs#L257) |
| `data.read_mapfile@1000` | 1.130 · 0.590 | 0.765 · 0.423 | **該当経路を変更**。1000行読み込んだ配列の長さを保存済み件数から取得。両端の値も確認。 制限なしの mapfile はまとめて読み、レコード境界から件数を得る。コールバック・件数上限・途中までのパイプ読みは別の入力経路。 [read_all](../src/session/mapfile.rs#L88) / [stored_array_len](../src/session/array.rs#L336) |
| `glob.lad_star@1000` | 0.937 · 0.064 | 0.965 · 0.579 | **該当経路を変更**。1000ファイル規模で *.txt の展開と一致数のカウントを測る。 名前を照合してから一致パスを作り、リテラルのディレクトリ部分を借用する。列挙結果が同じ場合のみ以前の整列順を使う。展開後の件数カウントも計時範囲。 [paths](../src/pattern.rs#L527) / [unescape](../src/pattern.rs#L509) |
| `hotloop.floor_colon_0` | 0.847 · 0.220 | 0.913 · 0.136 | **既存の経路を確認・維持**。算術ループ内でコロンを実行する基準値。 リテラルの : と true は出力も子生成も不要。反復ではトラップ等がなければ引数準備を再利用する。ShellSpec はトラップ付きで、制御ループも計時する。 [run_literal](../src/execute.rs#L1660) / [run_statements](../src/eval.rs#L24) |
| `hotloop.floor_arith_incr_1` | 1.136 · 0.326 | 0.980 · 0.196 | **既存の経路を確認・維持**。ループ変数に1を足した式を評価する。式自体は代入しない。 カウンタに1を加えた結果で終了状態を決める。本文は変数を書き換えず、ループ制御が別にカウンタを増やす。 [calculate](../src/arithmetic.rs#L421) / [expression](../src/arithmetic.rs#L257) |
| `hotloop.composite_reported_2` | 0.965 · 0.365 | 0.957 · 0.368 | **該当経路を変更**。整数を書式出力してからインクリメントし、最後の値を確認。 現在値を整数表示してから後置更新する。通常のスカラーなら更新時の式解析を減らせる。表示と更新の順序を保つ。 [run](../src/session/printf.rs#L31) / [calculate](../src/arithmetic.rs#L421) |

</details>

<details>
<summary>luish (7)</summary>

| Case | macOS Tonari / previous · Bash | Linux Tonari / previous · Bash | 実際に確認した実装 |
|---|---:|---:|---|
| `arith` | 0.978 · 0.503 | 0.996 · 0.567 | **複合処理を経路別に確認**。マンデルブロ集合・素数のふるい・コラッツ数列の結果を照合する。 Mandelbrot・ふるい・Collatz 中の算術、条件、eval を確認。単純な後置更新を短縮し、複雑な式の優先順位や短絡は維持する。 [calculate](../src/arithmetic.rs#L421) / [run_if](../src/eval.rs#L205) |
| `arrays` | 0.970 · 0.481 | 0.820 · 0.580 | **複合処理を経路別に確認**。配列スタック・キュー・ソート・ふるい等を実行。要素数取得と分岐の複製を改善。 スタック・キュー・ソート・ふるいで、配列長を格納件数から得る。通常の10進添字では算術解析を通さず、計算添字は評価する。 [stored_array_len](../src/session/array.rs#L336) / [resolve_index](../src/session/array.rs#L504) |
| `build` | 0.826 · 0.995 | 0.982 · 0.699 | **複合処理を経路別に確認**。ソースツリーを生成し、個別のシェルでレシピを実行。差分ビルドまで測る。 レシピ用シェルの起動、環境の継承・変更、パイプと待ちを確認する。実際の生成物を照合し、子ツールの時間も含めて比較する。 [start](../src/execute.rs#L1297) / [spawn_resolved](../src/execute/spawn.rs#L72) |
| `configure` | 0.989 · 0.814 | 0.995 · 0.693 | **複合処理を経路別に確認**。疑似 configure を実行。sed、eval、外部コマンドを含み、コンパイラの代役は grep。 模擬 configure の sed・grep・シェルヘルパーを確認。捕捉する外部コマンドの探索と if の複製を削減。コンパイラ試験は grep を使う。 [capture_external](../src/execute.rs#L1509) / [builtin](../src/input/eval.rs#L6) |
| `functions` | 0.942 · 0.322 | 0.925 · 0.370 | **複合処理を経路別に確認**。再帰・位置引数の転送・shift・getopts を含む。引用付き引数の転送と分岐を改善。 再帰関数は引数列を移し、単独の "$@" は最終引数へ直接追加する。呼び出し、shift、引数転送を一緒に通す。 [call_function](../src/eval.rs#L566) / [shift](../src/session.rs#L314) |
| `strings` | 1.016 · 0.374 | 0.877 · 0.505 | **複合処理を経路別に確認**。文字列反転やエスケープを含む。1文字の除去、連結、分割の経路を改善。 反転処理の端の1文字除去、トリム、IFS による分割を個別に確認。ASCII の1文字と UTF-8 の文字境界を区別する。 [trim_boundary](../src/pattern.rs#L310) / [substring](../src/word/modifier.rs#L332) |
| `textproc` | 0.921 · 0.447 | 0.971 · 0.516 | **複合処理を経路別に確認**。ログと INI を生成して解析。分割、パターン除去、集計をまとめて測る。 生成したログと INI の read・区切り削除・動的代入を確認する。フィールド探索とパターン処理が必要な箇所を区別し、最終レポートを比較する。 [field_ranges](../src/session/read.rs#L458) / [trim_boundary](../src/pattern.rs#L310) |

</details>

<details>
<summary>gnu-hello (3)</summary>

| Case | macOS Tonari / previous · Bash | Linux Tonari / previous · Bash | 実際に確認した実装 |
|---|---:|---:|---|
| `configure syntax check` | 1.003 · 0.830 | 0.732 · 0.987 | **該当経路を変更**。配布版 configure を -n で構文検査する。長い入力の解析時のメモリも実機で比較。 -n は完了した文を順次破棄する。末尾の演算子、未完の複合文を検出する状態は維持し、リダイレクトや関数本体を実行しない。 [file_script](../src/input.rs#L75) / [session_source_at](../src/parser.rs#L452) |
| `configure --help` | 0.832 · 0.819 | 1.019 · 0.821 | **複合処理を経路別に確認**。配布版 configure のヘルプ起動から出力完了までを測る。 実際の configure が --help で終了するところまでを確認する。複合文の読取・条件分岐・ヘルプ全体の出力を含む。 [file_script](../src/input.rs#L75) / [buffered_compound](../src/input.rs#L191) |
| `GNU Hello configure` | 1.016 · 1.051 | 0.997 · 0.947 | **複合処理を経路別に確認**。実際の cc を使い、毎回空のディレクトリで configure を完走。生成ファイルを照合しビルドも確認。 未変更の configure、新規ビルド先、実コンパイラで解析・継承 fd・環境・起動・待機を確認する。config.h、config.status、ビルドした実行ファイルを照合する。 [buffered_compound](../src/input.rs#L191) / [start](../src/execute.rs#L1297) |

</details>
