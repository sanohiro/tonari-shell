# Startup and execution / 起動と実行性能

## Performance target / 性能目標

For each workload, divide Tonari's elapsed time by GNU Bash's elapsed time on the same machine and in the same run. A ratio of **0.50** is the goal and **2/3** is the upper target. Ratios between 2/3 and 1.00 identify work that is faster than Bash but still above target. The harness reports both median and p95 ratios, and labels the median result. Direct executable launch time and binary size are reported separately.

各処理で、同じマシン・同じ計測での Tonari の所要時間を GNU Bash の所要時間で割ります。目標は **0.50**、許容する上限は **2/3** です。比が 2/3 より大きく 1.00 未満なら、Bash より速くても目標には未達です。計測器は中央値と p95 の比を表示し、中央値に対する達成状況を示します。実行ファイルの直接起動とバイナリサイズは別項目です。

```sh
python3 tools/bench.py target/release/tonari --bash /bin/bash --runs 300
python3 tools/bench.py target/release/tonari --case session --runs 300
python3 tools/bench.py target/release/tonari --case core --compare-tonari Previous /path/to/previous/tonari --runs 300
```

The default run includes a session workload covering assignment, export, parameter expansion, `cd`, `pwd`, `&&`, redirection and `unset`. It also times 1,000 arithmetic-loop iterations and 16 consecutive external commands inside one shell. For the latter it rotates in a Python parent launching the same 16 executables with `posix_spawn` and waiting for each. The launcher configures standard descriptors before timing; each child inherits them. Use `--case core` for comparisons with binaries predating session support. The primary prompt time spans the complete PTY launch. The harness also timestamps the child immediately before exec to show how much of that same sample follows the launcher's terminal setup and fork.

既定の計測には、代入・export・変数展開・`cd`・`pwd`・`&&`・リダイレクト・`unset` を同じシェルで実行する項目が含まれます。算術ループ1,000回と、一つのシェルから外部プログラム16個を順に起動する項目もあります。後者では同じプログラム16個を Python 親プロセスから直接 `posix_spawn` し、その都度終了を待つ条件も交互に計測します。標準入出力は計時前に用意し、各子プロセスに継承させます。セッション機能の実装前との比較には `--case core` を使います。プロンプト表示の主指標は疑似端末での起動全体です。子プロセスの exec 直前にも時刻を記録し、同じ測定サンプルのうち、端末の準備と fork の後に経過した時間も表示します。

<a id="path-expansion-order"></a>

## Wildcard expansion and collation / ワイルドカード展開と並べ替え

On 2026-11-22 we rotated Tonari `c2bd297`, the updated native release and GNU Bash 5.2.37 on an M1 running macOS 27.2.1 and a Debian 13 arm64 VM on the same computer. Each fresh noninteractive shell signaled readiness before sourcing a script using `set -- pattern`. Times include expansion, setting the positional arguments, printing the checked result, and shell exit. The local filesystem cache was warm. Five warm-ups preceded each set of samples. Mac used 45 samples per multilingual workload and 15 otherwise. Debian used 65 per workload. Values are **median / p95 ms per script**. [Script definitions](../tools/bench_scripts.py)

M1 の Mac と同じマシンの Debian VM で、新版・旧版・GNU Bash の実行順を交替した。起動済みの新しい非対話シェルで展開し、結果を位置パラメーターに設定して件数を確認した。ファイルシステムのキャッシュは温めた。各条件5回のウォームアップ後、Mac の多言語名は45回、他は15回、Debian は各65回を集計。表の回数は1スクリプト内での展開回数で、初回の行は毎回新しいシェルで1回だけ展開した。値はスクリプト全体の中央値 / p95（ms）。

| Expansion / 展開 | Mac previous | Mac Tonari | Mac Bash | Debian previous | Debian Tonari | Debian Bash |
|---|---:|---:|---:|---:|---:|---:|
| `*`, 512 names × 30 | 8.198 / 9.586 | 7.873 / 8.850 | 1096.298 / 1149.446 | 5.581 / 6.116 | 4.076 / 4.544 | 8.335 / 9.003 |
| `*.txt`, 512 matches × 30 | 8.126 / 8.871 | 7.872 / 8.205 | 1096.822 / 1325.614 | 5.585 / 6.389 | 4.125 / 4.595 | 8.754 / 9.155 |
| `item-000*.txt`, one match × 30 | 6.592 / 7.195 | 6.545 / 7.255 | 1085.448 / 1163.648 | 2.172 / 2.453 | 2.002 / 2.204 | 2.704 / 2.986 |
| `*em*07.txt`, six matches × 30 | 8.702 / 9.166 | 6.521 / 7.101 | 1077.425 / 1085.483 | 2.799 / 3.164 | 2.021 / 2.277 | 3.219 / 3.686 |
| `*/*.rs`, eight matches × 30 | 26.757 / 31.335 | 10.387 / 11.076 | 1144.234 / 1296.926 | 11.520 / 12.257 | 2.583 / 3.042 | 11.051 / 11.642 |
| `**/*.rs`, 80 matches × 10 | 44.780 / 46.499 | 41.330 / 44.253 | 1061.469 / 1101.253 | 8.495 / 9.537 | 6.787 / 7.434 | 23.588 / 25.369 |
| UTF-8 collation, 512 matches × 1 | 2.237 / 2.413 | 1.681 / 1.880 | 3.044 / 3.475 | 0.366 / 0.441 | 0.374 / 0.460 | 0.587 / 0.651 |
| UTF-8 collation, 512 matches × 30 | 46.986 / 48.267 | 8.949 / 9.419 | 55.039 / 57.364 | 6.250 / 7.349 | 4.335 / 4.701 | 12.181 / 13.614 |
| UTF-8 collation, 4,096 matches × 1 | 15.104 / 15.733 | 9.806 / 10.958 | 17.664 / 18.478 | 1.789 / 2.195 | 1.703 / 1.899 | 3.846 / 4.211 |

On Mac, the first UTF-8 expansion took 55% of Bash's median at both 512 and 4,096 names. On Debian, dense `*.txt` took 47% of Bash's median, while the one-match prefix still took 74%. In 150 separate Mac startup samples, startup through exit for `-c ':'` took 2.549 / 3.103 ms (previous 2.634 / 3.077). PTY launch through first prompt took 4.350 / 5.000 ms (previous 4.381 / 4.968). [Startup harness](../tools/bench.py)

Mac の UTF-8 名の初回展開は512件・4,096件の両方で Bash の中央値の約55%。Debian の `*.txt` 全件一致は約47%、固定の接頭辞で1件に絞る場合は約74%だった。Mac の起動を別に150回計測し、最初のプロンプトまでの中央値は旧版 4.381 ms、新版 4.350 ms だった。

Flat ASCII fixtures contain 512 files. The two-component case contains 512 ordinary files alongside eight directories, each with a `lib.rs`. The recursive case enables `globstar` and visits a tree with 16 parents and 80 leaf directories, each containing one Rust source and 15 other files. These rows use `LC_ALL=C`. Multilingual names mix case, accents, Japanese text and emoji in `en_US.UTF-8` on Mac or `C.UTF-8` on Debian. The 4,096-name workload exceeds Tonari's retained-order limit. The separate compatibility test checks the complete sorted output after renaming, adding or removing a name. A 4,096-name check compared all 55,478 bytes of NUL-delimited output against GNU Bash on both systems. [Tests](../tests/shell.py)

通常の条件は `LC_ALL=C`。2階層の条件は通常ファイル512件とディレクトリ8件を混在させ、再帰条件は親16個・末端80個の各末端に Rust ファイル1件と他のファイル15件を置いた。多言語名は大小のラテン文字・アクセント付き文字・日本語・絵文字を混在させ、Mac は `en_US.UTF-8`、Debian は `C.UTF-8`。4,096件はソート順を保持する上限を超える。別の4,096件の検証では NUL 区切りの出力55,478バイトが両 OS で GNU Bash と一致した。

A separate Linux `strace -qq -c -e trace=openat,getdents64` run expanded one pattern in a directory of 512 files and one subdirectory holding a Rust source. For `item*.txt` the number of directory-read syscalls fell from 10 to 2 including EOF. For `*/*.rs` the previous build attempted 514 opens, with 512 errors. The new build opened the two directories successfully and read them in four calls including EOF. The Mac binary measured 869,632 bytes, previously 852,992. The Linux static executable measured 1,020,616 bytes, previously 1,009,640. [Design and comparison with Apple's libc](engineering-notes.md)

Linux のシステムコールを別に数えると、512ファイルと1ディレクトリでの `item*.txt` はディレクトリ読み取りが EOF 込みで10回から2回になった。`*/*.rs` は旧版で open が514回、うち512回は通常ファイルへの失敗だった。新版はディレクトリ2件を開き、各2回の読み取りで完了した。単一バイナリの増分は Mac が16,640バイト、Linux が10,976バイト。

```sh
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /path/to/bash --timing started --case glob --case glob-all --case glob-sparse --case glob-multi-star --case glob-subdirs --case glob-recursive --runs 15
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /path/to/bash --timing started --case glob-utf8-once --case glob-utf8-sort --case glob-utf8-large --runs 45
```


<a id="external-descriptor-copies"></a>

## External commands with standard-stream redirection / 標準入出力をつなぎ替える外部コマンド

On 2026-11-22 an M1 ran macOS 27.2.1 natively and Debian 13 arm64 in a VM. We rotated the new release, Tonari `d471f0f` and GNU Bash 5.2.37. Each script ran a foreground command or pipeline 16 times after shell startup with job control off. The driver checked all 38 output bytes, stderr and exit status, with five warm-ups then 65 samples on Mac and 85 on Debian. Times are **median / p95 in ms per 16 iterations**. Both external commands use absolute paths. [Scripts and harness](../tools/bench_scripts.py)

M1 上の macOS と Debian 13 arm64 VM で、新旧 Tonari と Bash 5.2.37 を交互に実行した。起動済みの非対話シェルで各コマンドを16回実行し、毎回スクリプト全体の出力38バイト・標準エラー・終了値を照合した。最初の5回を除き、Mac は65回、Debian は85回の中央値 / p95 を示す。外部の `printf` と `cat` はどちらも絶対パスで起動した。

| Workload / 処理 | Mac previous | Mac Tonari | Mac Bash | Debian previous | Debian Tonari | Debian Bash |
|---|---:|---:|---:|---:|---:|---:|
| External `printf … 2>&1` | 34.714 / 42.517 | 22.760 / 29.488 | 35.731 / 44.748 | 4.451 / 4.919 | 3.917 / 4.544 | 5.141 / 5.771 |
| External `printf … \|& cat 2>&1` | 35.461 / 45.837 | 25.781 / 34.749 | 39.645 / 53.063 | 5.965 / 6.673 | 5.356 / 6.019 | 6.652 / 7.729 |
| External `printf … \|& cat` | 27.142 / 33.982 | 26.974 / 33.238 | 43.303 / 52.806 | 5.446 / 6.217 | 5.512 / 6.220 | 6.729 / 7.549 |
| Builtin `printf … \|& /bin/cat` | 31.430 / 43.813 | 30.712 / 49.884 | 45.136 / 80.124 | — | — | — |
| Builtin `printf …`, direct output | 0.555 / 0.868 | 0.544 / 0.710 | 0.765 / 1.072 | — | — | — |

The Mac median fell 34% for standalone stderr duplication and 27% with the same duplication on the receiving command. The plain merged pipe already used a spawn path. In a separate 24-sample trial, passing its descriptor copies directly to libc gave 26.700 ms versus 27.231 ms using Rust's existing spawn path. For the built-in producer, Tonari still forks to run it as a pipeline stage. Writing the same 16 lines directly from the builtin took 0.544 ms in the final build. [Direct launches](#pipe-process-timing) · [Loader traces](#macos-child-loader)

標準エラーを標準出力にまとめる単体コマンドで旧版比34%、パイプの受信側でもまとめる場合は27%短縮した。通常の `|&` はすでに spawn で起動していた。別の24回の試作比較でも、パイプの複製操作を libc に直接渡すと 26.700 ms、既存経路が 27.231 ms だった。組み込みの `printf` もパイプの一段として実行する場合は fork する。同じ16行を端末や呼び出し側に直接出す用途ではパイプを省ける。

We also rotated 200 Mac launches per shell. From launch through exit, `-c ':'` took 2.735 / 3.211 ms (previous 2.756 / 3.225, Bash 5.866 / 6.669). PTY launch to first prompt took 4.515 / 5.181 ms (previous 4.596 / 5.160, Bash 7.817 / 8.427). The Mac binary grew from 836,416 to 852,992 bytes. Its machine-code section grew by 1,116 bytes, and the Mach-O `__TEXT` segment crossed a 16 KiB boundary. The Linux static PIE grew from 1,008,248 to 1,009,640 bytes.

Mac の起動も各200回測った。新版の `-c ':'` は中央値 2.735 ms、疑似端末の起動からプロンプトまでは 4.515 ms だった。Mac の機械語部分は1,116バイト増え、セグメントの16 KiB境界をまたいだためファイル全体は16,576バイト増えた。Linux の静的リンク版は1,392バイト増えた。

```sh
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /path/to/bash --timing started --case redirect-stderr --case merged-pipeline-redirect --case merged-pipeline --case builtin-merged-pipeline --case builtin-output --runs 65
python3 tools/bench.py /path/to/tonari --compare-shell Previous /path/to/previous --bash /path/to/bash --case startup --case prompt --runs 200
```

<a id="borrowed-glob-names"></a>

## Hyphenated path prefixes and directory scans / ハイフン付きファイル名とディレクトリ走査

On 2026-11-22 we compared revision `3c03267`, the updated release and GNU Bash 5.2.37 on the M1 with macOS 27.2.1 and its Debian 13 arm64 VM. Each shell signaled readiness before sourcing the script. The driver rotated shell order, checked output, stderr and exit status, and ran five warm-ups followed by 35 samples on Mac or 85 on Debian. Each glob script expands its pattern 30 times in a directory with 512 files on the local filesystem. The character-class `case` control runs 3,000 iterations without reading that directory. Values are **median / p95 ms**. The single executables grew by 48 bytes on Mac (836,416 bytes) and 2,160 bytes on Linux (1,008,248 bytes). [Script definitions](../tools/bench_scripts.py)

同じ M1 の macOS と Debian 13 arm64 VM で旧版、新版、Bash 5.2.37 の順番を入れ替えながら測定した。起動済みのシェルにスクリプトを読ませ、出力・標準エラー・終了値を毎回確認した。各5回のウォームアップ後、Mac は35回、Debian は85回を集計。ファイル名展開は512ファイルがあるディレクトリで30回繰り返し、対照の文字クラス付き `case` はディレクトリを読まずに3,000回繰り返す。単位は中央値 / p95（ms）。

| Workload / 処理 | Mac previous | Mac Tonari | Mac Bash | Debian previous | Debian Tonari | Debian Bash |
|---|---:|---:|---:|---:|---:|---:|
| `item-000*.txt`, one match | 10.289 / 13.354 | 6.927 / 8.412 | 1119.627 / 1175.226 | 3.447 / 3.783 | 2.237 / 2.439 | 2.733 / 3.029 |
| `*.txt`, 512 matches | 10.665 / 13.329 | 8.373 / 9.758 | 1128.396 / 1186.428 | 6.310 / 6.829 | 5.721 / 6.300 | 8.865 / 9.568 |
| `case` with `[0-9]` | 5.356 / 6.313 | 5.351 / 7.506 | 7.223 / 9.050 | 4.949 / 5.740 | 4.939 / 5.495 | 8.891 / 9.498 |

The Debian one-match workload fell from 126.1% to 81.8% of Bash's median. All 512 matches took 64.5% of Bash's median and 65.8% of its p95. In a separate Debian ablation, changing only the internal quoting of the initial literal reduced the one-match median from 3.468 to 2.904 ms (65 samples, Bash 2.802). The final build also borrows the names during matching.

Debian で1件だけ一致する条件は Bash の中央値比 126.1% から 81.8% に短縮した。全512件が一致する条件は中央値で 64.5%、p95 で 65.8% だった。別に先頭の固定文字のエスケープだけ直した中間版を65回測ると、1件一致は旧版 3.468 ms、中間版 2.904 ms、Bash 2.802 ms だった。最終版ではディレクトリ内の名前のコピーも省いた。

```sh
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/old-tonari --bash /path/to/bash --timing started --case glob-sparse --case glob --case case-classes --runs 85
```

<a id="completion-and-case"></a>

## Registered completion, case and globs / 登録した補完・case・ファイル名展開

On 2026-11-22 we compared `0a3a293`, the updated native release, and GNU Bash 5.2.37 on an M1 Mac (macOS 27.2.1) and Debian 13 arm64 in a Linux VM on the same machine. Each OS used local binaries and fixtures, warm caches, and rotating shell order. All times below are **median / p95 in milliseconds**. The Mac binary grew from 836,336 to 836,368 bytes. The Linux static PIE grew from 987,496 to 1,006,088 bytes. The Bash binaries measured 1,208,904 and 1,412,096 bytes respectively.

同じ M1 の macOS と Debian 13 arm64 VM で、旧 Tonari・新 Tonari・Bash 5.2.37 を交互に測定した。実行ファイルと入力ファイルは各 OS のローカルディスクに置き、キャッシュを温めた。表は中央値 / p95（ms）。旧版とのサイズ差は Mac が +32 バイト、静的リンクの Linux 版が +18,592 バイト。

| Workload / 処理 | Mac previous | Mac Tonari | Mac Bash | Debian previous | Debian Tonari | Debian Bash |
|---|---:|---:|---:|---:|---:|---:|
| Tab, 10,000 fixed words, all match | 2.370 / 3.378 | 0.321 / 0.592 | 26.589 / 30.556 | 2.692 / 4.407 | 0.373 / 0.421 | 270.014 / 280.553 |
| Tab, 10,000 fixed words, one match | 2.229 / 2.551 | 0.231 / 0.271 | 26.214 / 28.864 | 2.279 / 3.540 | 0.319 / 0.403 | 268.520 / 279.456 |
| Tab, 10,000 words in a variable, one match | 0.964 / 1.259 | 1.005 / 1.490 | 2.563 / 2.859 | 1.235 / 1.726 | 1.237 / 1.588 | 3.779 / 4.911 |
| 3,000 `case` decisions, literal prefix and suffix | 8.599 / 10.305 | 4.673 / 6.365 | 7.821 / 9.563 | 6.853 / 8.746 | 4.243 / 5.434 | 10.350 / 13.731 |
| 3,000 `case` decisions, `[0-9]` and wildcard | 7.029 / 9.938 | 5.167 / 7.492 | 7.240 / 10.112 | 6.985 / 9.759 | 5.302 / 7.684 | 9.548 / 11.058 |
| 30 × `set -- fixtures/*.txt`, 512 files | 14.455 / 18.523 | 11.115 / 12.950 | 1206.226 / 1517.805 | 8.577 / 11.883 | 6.882 / 8.913 | 9.590 / 11.384 |
| Shell launch through exit, `-c ':'` | 2.854 / 4.030 | 2.832 / 5.511 | 6.058 / 10.257 | 0.208 / 0.422 | 0.207 / 0.334 | 0.469 / 1.039 |
| PTY launch through first prompt | 4.581 / 5.362 | 4.505 / 5.189 | 7.714 / 8.688 | 0.608 / 0.990 | 0.585 / 1.771 | 1.034 / 2.245 |

The three Tab rows use a fresh interactive shell and a list named `option-00000-flag` through `option-09999-flag`. After the rc file registers the list, the driver sends the prefix and waits for its echo. It then times Tab plus a following `@` until that character comes back through the PTY. A separate redraw verifies the entire completed line. Each case has three warm-ups and 40 timed samples per shell. The variable control registers the literal expression `$choices`, expanded on Tab. Its median was within 0.05 ms of the previous build on both systems. [Completion driver](../tools/bench_completion.py)

補完は各回、新しい対話シェルの設定ファイルで1万件を登録してから入力した。接頭辞が表示されたところで Tab と続く `@` を送り、PTY に `@` が戻るまでを測った。計時後に行全体を再描画して、補完結果と後続の入力を照合した。ウォームアップ3回、計測40回。変数を使う対照では `$choices` という式を登録して Tab ごとに展開する。この対照の中央値は両 OS とも旧版との差が 0.05 ms 未満だった。

The script rows use `LC_ALL=C`, five warm-ups and 35 timed samples. The driver sources each script after the shell signals readiness and checks stdout, stderr and exit status. The literal-edge `case` workload tries two nonmatching patterns before the matching `src/*.rs`. Its Tonari/Bash median ratio was 0.597 on Mac and 0.410 on Debian. The Mac character-class case (0.714) and Debian pathname expansion (0.718) exceeded the two-thirds target. Startup rows use five warm-ups, 200 runs and `--noprofile --norc` for every participant. The Debian run also timed the same revision linked with GNU ld; its startup and first-prompt times were 0.207 / 0.320 and 0.578 / 1.088 ms respectively. [Script driver](../tools/bench_scripts.py) · [Startup driver](../tools/bench.py)

スクリプトは `LC_ALL=C`、ウォームアップ5回、計測35回。起動済みのシェルにスクリプトを読ませ、毎回出力・診断・終了値を検証した。固定の前後を持つ `case` は、先に2個の不一致を判定してから `src/*.rs` に一致する。Bash に対する中央値の比は Mac で 0.597、Debian で 0.410。Mac の文字クラス付き `case`（0.714）と Debian のファイル名展開（0.718）には 2/3 の目標まで差がある。起動の2項目は全シェルに `--noprofile --norc` を渡し、ウォームアップ5回、計測200回。Debian では同じソースを GNU ld でもリンクし、起動が 0.207 / 0.320 ms、最初のプロンプトまでが 0.578 / 1.088 ms だった。

In a second batch of 300 samples the Mac `-c ':'` p95 was 3.161 ms (previous 3.206), and the Debian first-prompt p95 was 0.815 ms (previous 0.880, same-source GNU ld 0.809). Neither higher p95 from the first batch repeated. The Debian first-prompt medians were 0.588, 0.585 and 0.596 ms respectively.

起動をさらに300回測ると、Mac の `-c ':'` の p95 は 3.161 ms（旧版 3.206）、Debian の最初のプロンプトの p95 は 0.815 ms（旧版 0.880、同じソースの GNU ld 版 0.809）だった。最初の計測で見られた両項目の p95 の増加は再現しなかった。Debian のプロンプトまでの中央値は順に 0.588、0.585、0.596 ms。

```sh
python3 tools/bench_completion.py /path/to/tonari /path/to/previous --bash /path/to/bash --word-list --runs 40
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /path/to/bash --timing started --case case-edges --case case-classes --case glob --runs 35
python3 tools/bench.py /path/to/tonari --compare-shell Previous /path/to/previous --bash /path/to/bash --case startup --case prompt --runs 200
```

<a id="pipe-process-timing"></a>

### Where merged-pipe time is spent / 標準エラーをつなぐパイプの時間内訳

Tonari `98e1614`, GNU Bash 5.2.37, macOS 27.2.1 and Debian 13 arm64 on the same M1, 2026-11-22. The workload prints the numbers 0–15 with 16 separate `/usr/bin/printf` and `/bin/cat` pairs. All 38 output bytes and the exit statuses were checked. Shell timings begin after startup. An already running Python parent also made the pipe, wired stdout and stderr with `POSIX_SPAWN_DUP2`, called `os.posix_spawn` twice, and waited for both children before the next pair. The parent captured the same output. Five warm-ups and 35 measured samples per condition in rotating order. Values are **median / p95 ms**. Each statistic is computed independently.

同じ M1 の2環境で外部の `printf` と `cat` を16組起動し、38バイトの出力と終了値を確認した。シェルの起動は計時外。対照では起動済みの Python 親プロセスから同じパイプを作り、2プロセスを直接起動して両方の終了を待った。各条件を交互に実行し、最初の5回を除く35回の中央値 / p95（ms）を示す。区間ごとに計算しているため、中央値の和は全体の中央値と一致する必要がない。

| 16 pairs / 16組 | macOS | Debian |
|---|---:|---:|
| Tonari, ordinary pipe | 27.498 / 42.688 | 5.302 / 6.289 |
| Tonari, merged pipe | 28.827 / 37.007 | 5.457 / 6.548 |
| Tonari, explicit stderr duplication then pipe | 29.147 / 39.010 | 5.483 / 6.596 |
| Bash, merged pipe | 46.001 / 81.871 | 6.465 / 8.963 |
| Direct merged pipe, total | 27.712 / 35.754 | 5.894 / 7.086 |
| Direct, inside the 32 spawn calls | 5.665 / 7.761 | 3.478 / 4.241 |
| Direct, close pipe ends and wait after launching each pair | 22.160 / 27.790 | 2.429 / 2.782 |

In a separate control, a small native C parent launched the same executable as its child. The first statement of `main` read `CLOCK_MONOTONIC`; the child wrote that timestamp to a pipe and returned. The parent recorded time immediately before and after `posix_spawn` and after `waitpid`. Each OS used its local C compiler with `-O2`, normal dynamic linking, 15 warm-ups, then 200 launches. Most of the cross-host difference in this control accumulated before entry to `main`. Each system runs different executable and loader formats. These measurements do not give separate costs for loading, kernel work or scheduling.

別の対照では、それぞれの OS で同じ小さな C ソースを `-O2` で動的リンクした。親が自分自身を起動し、子は `main` の先頭で時刻を取り、パイプにその値を書いて終了した。親は `posix_spawn` の直前・直後と `waitpid` 完了後に時刻を取った。すべて `CLOCK_MONOTONIC` を使い、最初の15回を除く200回を計測した。この対照の OS 間の差は大部分が子の `main` に入るまでに生じた。ローダー・カーネル・スケジューラごとの寄与は未計測。

| Single native child / 子プロセス1個 | macOS | Debian |
|---|---:|---:|
| Launch request to child entering `main` | 1.470 / 1.920 | 0.158 / 0.206 |
| Entry to parent observing exit | 0.255 / 0.330 | 0.038 / 0.064 |
| Entire launch and wait | 1.728 / 2.242 | 0.197 / 0.274 |
| Inside the parent's spawn call | 0.136 / 0.233 | 0.102 / 0.128 |

The same Debian run compared equivalent `case` patterns on the same input. Replace the case arms in `case-edges` with `src/u*.rs) last=$i ;; *) exit 1 ;;`, then change only `u` to `[u]`. A separate pathname control changed `fixtures/*.txt` to `fixtures/item-000*.txt` in the 512-file fixture and checked the resulting count and first name. In both pathname cases the shell still scans the directory 30 times. [Script and fixture definitions](../tools/bench_scripts.py)

Debian の同じ計測で、同じ入力・分岐数の `case` を用意し、固定の `u` を同じ文字だけに一致する `[u]` に替えた。ファイル名展開では512ファイルを置いたまま、全件に一致する `*.txt` と1件だけ一致する `item-000*.txt` を比較した。どちらもディレクトリを30回走査する。

| Debian workload / 処理 | Tonari | Bash |
|---|---:|---:|
| 3,000 × `case`, `src/u*.rs` | 4.435 / 5.793 | 8.733 / 9.983 |
| 3,000 × `case`, `src/[u]*.rs` | 4.614 / 5.323 | 8.751 / 10.006 |
| 30 globs, 1 of 512 names returned | 3.477 / 4.125 | 2.771 / 3.241 |
| 30 globs, all 512 names returned | 6.427 / 7.478 | 8.764 / 9.932 |

A follow-up used two directories of 512 files, changing the separator in every name and its pattern together. Each glob returned index 000. Each script made 30 expansions; the eight shell/script combinations rotated through five warm-ups and 45 measurements. For literal source spelling, Tonari took 3.853 / 5.434 ms for `item-000*.txt` and 3.134 / 4.430 for `item_000*.txt` (Bash 3.091 / 4.413 and 3.008 / 5.984). With the same patterns supplied via an unquoted variable, Tonari took 3.264 / 7.106 and 3.052 / 5.601 ms. Tonari escapes a hyphen in the accumulated literal prefix when it first encounters the star; the resulting backslash selects the generic matcher for each directory entry. The lower median with an underscore or a variable is consistent with that code path.

追加計測では512個すべてのファイル名と対応するパターンで、区切りを一緒にハイフンまたはアンダースコアに替えた。各スクリプトで30回展開し、どれも番号000の1件に一致する。各条件45回の計測で、直接書いた場合の Tonari の中央値はハイフンで 3.853 ms、アンダースコアで 3.134 ms、同じパターンを非引用の変数に入れた場合は 3.264 ms と 3.052 ms だった。ソースに直接書いたハイフンは最初の星印に達した段階でエスケープされ、各候補の照合が汎用経路に入る。

The later build avoids escaping the initial hyphen and copying every candidate. 更新後は先頭のハイフンをそのまま照合し、一致した候補だけをコピーする。[Updated glob measurements / 更新後の計測](#borrowed-glob-names)

<a id="macos-child-loader"></a>

### Loader activity inside the two external commands / 外部2コマンドのローダー内訳

On the same M1 (macOS 27.2.1 build 26B131), Xcode 27.2 Instruments recorded three separate launches each of `/bin/cat /dev/null` and `/usr/bin/printf x`. The App Launch template reports a dyld interval named *Launch Executable*, with nested *Static Initializer* and *Apply Fixups* events. The initializer row below selects the event for `libSystem.B.dylib`. Values are **median [min–max] milliseconds** from the three traces. These are instrumented standalone launches; the earlier pipeline stopwatch includes overlap between the two children. Both executables list libSystem as their only direct dylib dependency in `otool -L`.

同じ M1 上で Xcode 27.2 の Instruments を使い、実際にパイプにつなぐ2本のコマンドを個別に各3回記録した。表は中央値［最小–最大］、単位は ms。dyld の起動区間の内側に、システムライブラリの初期化やアドレスの修正が含まれる。プロファイラを付けた単独起動と、子プロセスが並行して動くパイプ全体では測定区間が異なる。

| Instruments event / 区間 | `/bin/cat` | `/usr/bin/printf` |
|---|---:|---:|
| dyld Launch Executable | 1.089 [1.080–1.183] | 0.949 [0.923–2.849] |
| Run static initializer, libSystem | 0.572 [0.563–0.668] | 0.492 [0.487–0.533] |
| Apply Fixups | 0.128 [0.113–0.134] | 0.122 [0.115–0.619] |

```sh
xcrun xctrace record --template 'App Launch' --no-prompt --output cat.trace --launch -- /bin/cat /dev/null
xcrun xctrace export --input cat.trace --xpath '/trace-toc/run[@number="1"]/data/table[@schema="dyld-activity-interval"]'
```

<a id="align-experiments"></a>

## Parent memory, pipes and delimiter search / 親シェルのメモリ・パイプ・区切り検索

On 2026-11-22, on an Apple M1 with macOS 27.2.1 and a Debian 13 arm64 VM, we compared Tonari `0e091ca`, the updated native release and GNU Bash 5.2.37. Each case used five warm-ups followed by 30 samples per shell in rotating order. The fixtures and binaries were local to each OS. The timer starts after the ready marker from a fresh, noninteractive shell and includes sourcing the script and collecting the child and its output. The driver checks stdout, stderr and exit status every time. Values are **median / p95 in milliseconds**. The Unicode IFS row uses `en_US.UTF-8` on macOS and `C.UTF-8` on Debian. Other rows use `LC_ALL=C`. [Script driver](../tools/bench_scripts.py)

同じ M1 上の macOS と Debian 13 arm64 VM で、旧 Tonari、新 Tonari、Bash 5.2.37 を順番に入れ替えながら各30回測定した。各条件の最初の5回を除き、起動済みのシェルにスクリプトを読ませて完了するまでを計時している。毎回、標準出力・標準エラー・終了値を照合した。表は中央値 / p95（ms）。絵文字の区切りだけ各 OS の UTF-8 ロケールで測った。

### Fresh shell / 通常の起動状態

| Workload / 処理 | Mac previous | Mac Tonari | Mac Bash | Debian previous | Debian Tonari | Debian Bash |
|---|---:|---:|---:|---:|---:|---:|
| 16 sequential `/usr/bin/true` | 16.540 / 16.975 | 16.449 / 17.043 | 25.659 / 26.740 | 3.172 / 3.977 | 3.179 / 3.502 | 4.380 / 4.732 |
| 16 × `$(/usr/bin/printf …)` | 18.206 / 19.130 | 18.258 / 19.060 | 28.772 / 32.272 | 4.256 / 4.630 | 3.557 / 3.918 | 5.314 / 5.796 |
| 16 × `/usr/bin/printf … |& /bin/cat` | 29.434 / 32.600 | 21.667 / 23.309 | 33.463 / 37.443 | 5.096 / 5.351 | 5.154 / 5.570 | 6.289 / 6.829 |
| 16 × `/usr/bin/printf … | /bin/cat` | 23.364 / 34.105 | 22.066 / 34.865 | 34.326 / 42.694 | 5.006 / 5.611 | 4.992 / 5.435 | 6.117 / 6.798 |
| 400 splits, 32 fields, `IFS=:` | 4.816 / 5.100 | 3.224 / 3.266 | 16.505 / 16.803 | 5.907 / 6.375 | 4.428 / 4.790 | 18.204 / 19.136 |
| 400 splits, 32 fields, `IFS=😀` | 7.524 / 7.880 | 3.192 / 3.381 | 18.707 / 18.946 | 8.457 / 8.802 | 4.283 / 4.508 | 21.912 / 22.381 |
| 2,000 short path trims (basename and extension) | 5.136 / 5.291 | 4.954 / 5.271 | 10.750 / 10.865 | 4.567 / 4.829 | 4.472 / 4.784 | 13.756 / 14.542 |
| 400 basename trims, 2,048-byte final component | 4.863 / 4.929 | 1.529 / 1.631 | 615.173 / 688.967 | 3.519 / 3.677 | 1.445 / 1.510 | 821.893 / 827.673 |
| 2,000 lines via `IFS= read -r` | 6.067 / 6.161 | 5.993 / 6.102 | 9.528 / 9.765 | 4.955 / 5.247 | 4.924 / 5.175 | 10.033 / 10.368 |

### Shell holding a 16 MiB variable / 16 MiB の変数を保持した状態

The driver fills a private, unexported shell variable before the ready marker. Its contents stay out of the child process environment. This deliberately increases parent memory to distinguish process creation and session copying from the work done by the launched program. The pipe workloads start 32 real external processes per sample. [Subshell implementation](../src/subshell.rs)

開始合図の前に未エクスポートのシェル変数を16 MiBのデータで埋める。変数を用意する時間は計測外で、起動する外部コマンドの環境にも含めない。親のメモリ量に依存するコストを見るための条件である。パイプの項目では1サンプルにつき外部プロセスを32個起動する。

| Workload / 処理 | Mac previous | Mac Tonari | Mac Bash | Debian previous | Debian Tonari | Debian Bash |
|---|---:|---:|---:|---:|---:|---:|
| 16 × `$(/usr/bin/printf …)` | 29.121 / 31.489 | 19.100 / 20.138 | 29.923 / 35.012 | 32.208 / 33.367 | 3.903 / 4.349 | 5.899 / 6.490 |
| 16 × `/usr/bin/printf … |& /bin/cat` | 30.251 / 31.605 | 22.500 / 24.389 | 34.495 / 37.449 | 5.647 / 5.903 | 5.404 / 6.223 | 6.855 / 7.652 |
| 64 × `$(printf …)` | 31.379 / 38.031 | 0.954 / 0.991 | 45.333 / 47.300 | 73.942 / 80.318 | 0.422 / 0.547 | 14.949 / 15.185 |
| 16 × `$(printf v; printf …)` | 60.884 / 64.197 | 9.622 / 11.125 | 12.303 / 12.623 | 28.288 / 29.697 | 2.693 / 2.765 | 4.162 / 4.370 |

The ordinary Debian launch and plain pipe controls changed little. For `|&` with a fresh Debian shell, the new median was 5.154 ms versus 5.096 ms previously and 6.289 ms for Bash. With the larger variable the median improved, but its p95 rose from 5.903 to 6.223 ms. On macOS the fresh-shell median for `|&` fell from 29.434 to 21.667 ms. The binaries measured 836,336 bytes on macOS and 987,496 bytes on Linux (static musl with packed relative relocations). [Design and syscall counts](engineering-notes.md)

Debian の通常の連続起動と `|` の差は小さい。変数を追加しない場合の `|&` は旧版 5.096 ms、新版 5.154 ms、Bash 6.289 ms で、2/3 目標に届いていない。大きな変数がある場合は中央値が改善したものの p95 は 5.903 ms から 6.223 ms になった。Mac の通常状態の `|&` は 29.434 ms から 21.667 ms に短縮した。単一実行ファイルは macOS が 836,336 バイト、Linux が 987,496 バイト。

```sh
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /path/to/bash --timing started --runs 30 --case external --case external-substitutions --case merged-pipeline --case pipeline --case field-split --case field-split-unicode --case strings --case long-trim --case read-lines
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /path/to/bash --timing started --runs 30 --state-mib 16 --case external-substitutions --case merged-pipeline --case substitutions --case compound-substitutions
```

<a id="capture-and-spawn"></a>

## Command substitution, process launch and patterns / コマンド置換・プロセス起動・パターン

On 2026-11-22 an Apple M1 ran macOS 27.2.1 natively and Debian 13 arm64 in a VM. We rotated the previous release (`dbde5fe`), the updated release and GNU Bash 5.2.37. Each condition used five warm-ups, then 30 measured runs on macOS or 250 on Debian. The temporary home, scripts, fixtures and executables were on each OS's local filesystem. All shells used `LC_ALL=C`, warm caches and disabled startup files. Every run checked stdout, stderr and exit status. Tables show **median / p95 in milliseconds**. Whole spans launch through exit. Started begins after the ready marker and includes sourcing the script and collecting the result. [Script driver](../tools/bench_scripts.py)

2026-11-22、M1 のネイティブ macOS と同じマシン上の Debian 13 arm64 VM で、旧版・新版・Bash 5.2.37 を交互に実行した。各条件で5回ウォームアップした後、Mac は30回、Debian は250回測定した。各 OS のローカルディスクと温まったキャッシュを使い、出力・標準エラー・終了値を毎回確認している。表は中央値 / p95（ms）。Whole は起動から終了まで、Started は起動済みシェルへ実行許可を送ってからスクリプトを読み込み、終了結果を受け取るまで。

### macOS arm64

| Workload / 処理 | Timing / 区間 | Previous | Tonari | Bash |
|---|---|---:|---:|---:|
| 64 × `$(printf …)` | Whole | 45.225 / 57.215 | 4.391 / 6.480 | 59.866 / 74.905 |
| 64 × `$(printf …)` | Started | 44.217 / 56.791 | 0.695 / 1.542 | 52.581 / 65.089 |
| 16 × `$(/usr/bin/printf …)` | Whole | 35.486 / 51.407 | 25.173 / 34.495 | 43.085 / 53.650 |
| 16 × `$(/usr/bin/printf …)` | Started | 32.277 / 46.950 | 21.006 / 28.854 | 35.450 / 48.887 |
| 1,000 × `[[ =~ ]]` | Whole | 12.054 / 14.025 | 6.570 / 8.419 | 15.664 / 19.997 |
| 1,000 × `[[ =~ ]]` | Started | 7.819 / 9.031 | 2.611 / 3.720 | 8.310 / 9.527 |
| 30 × `set -- fixtures/*.txt` (512 files) | Whole | 31.911 / 48.623 | 17.759 / 21.911 | 1148.808 / 1662.967 |
| 30 × `set -- fixtures/*.txt` (512 files) | Started | 28.573 / 40.128 | 14.095 / 18.297 | 1162.199 / 1529.108 |
| 2,000 basename / extension trims | Whole | 8.890 / 9.811 | 9.094 / 10.132 | 18.058 / 19.634 |
| 2,000 basename / extension trims | Started | 5.227 / 5.812 | 5.436 / 5.821 | 11.286 / 13.308 |

### Debian arm64

| Workload / 処理 | Timing / 区間 | Previous | Tonari | Bash | Direct C |
|---|---|---:|---:|---:|---:|
| 16 × `/usr/bin/true` | Whole | 3.774 / 5.269 | 3.749 / 5.221 | 5.391 / 7.330 | 3.438 / 4.471 |
| 16 × `/usr/bin/true` | Started | 3.659 / 6.302 | 3.621 / 5.071 | 5.031 / 7.347 | 3.503 / 5.106 |
| 32 exports, 16 × `/usr/bin/printenv` | Whole | 5.095 / 7.201 | 4.733 / 8.043 | 6.573 / 11.040 | — |
| 32 exports, 16 × `/usr/bin/printenv` | Started | 4.448 / 7.569 | 4.191 / 7.561 | 5.538 / 8.930 | — |

The native C control was built with Alpine musl and used the same pipes, environment and ready protocol as Tonari. It directly launches `/usr/bin/true` 16 times and waits for each. In the started-shell run the C control also exceeded two-thirds of Bash's median. A loop with a fixed executable bounds what we can save in the shell on this machine. With 32 exports, the started-shell median fell from 4.448 to 4.191 ms. The whole-script p95 rose from 7.201 to 8.043 ms. [Native control](../tools/bench_spawn.c)

C の対照は Alpine の musl でビルドし、同じパイプ・環境・開始合図を使った。起動後の計測では C で直接起動して毎回終了を待っても Bash の中央値の 2/3 を超えた。ここでは実行ファイルと待ち方が同じなので、シェル内部で短縮できる時間は限られる。32変数を export し、子が値を受け取るケースでは起動後の中央値が 4.448 ms から 4.191 ms に短縮した。起動込みの p95 は 7.201 ms から 8.043 ms に増えた。

The macOS release was 836,336 bytes, up from 819,776. The Linux static PIE was 987,496 bytes with packed relative relocations. The ordinary link of the same source was 1,053,032 bytes. Packing produced an ELF `.relr.dyn` section, avoided the extra file padding and retained the single-executable layout. Debian and Ubuntu both ran the packed binary through the shell test suite. [Linux release recipe](#linux-arm64-static-pie)

macOS 版は 819,776 バイトから 836,336 バイトになった。Linux arm64 静的 PIE は相対再配置情報を圧縮して 987,496 バイト。同じソースの通常リンクで観測した 1,053,032 バイトには、セグメント境界のパディングも含まれていた。圧縮後の単一バイナリは Debian と Ubuntu でテストした。

```sh
# Build the native control with the Alpine musl compiler for the Linux run.
cc -Os -static-pie tools/bench_spawn.c -o /tmp/bench-spawn
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /bin/bash --spawn-control /tmp/bench-spawn --case external --case external-env --runs 250
python3 tools/bench_scripts.py /path/to/tonari --previous /path/to/previous --bash /path/to/bash-5.2.37 --case substitutions --case external-substitutions --case regex --case glob --case strings --runs 30
```

<a id="script-execution"></a>

## Running shell scripts / シェルスクリプトの実行

Each named script ran in two modes. Whole-script timing spans the Python launch request through child exit. In the started-shell mode, the driver first waits for a ready marker from a fresh noninteractive shell. It then starts the clock, releases the shell from a builtin read, and sources the same file. That interval includes opening, parsing, executing the script and collecting its result. Both modes verify stdout, stderr and exit status every time. [Reproducible scripts and driver](../tools/bench_scripts.py)

同じスクリプトを、ファイル名を渡して起動する場合と、起動済みのシェルで source する場合で測った。後者は準備完了を受け取ってから計時を始め、ファイルの読み込み・解析・実行・終了の確認までを含む。各回で出力と終了値を照合した。

The Apple M1 ran native macOS and a Debian 13 arm64 VM. Each host ran locally stored release binaries and fixtures, GNU Bash 5.2.37, a temporary home, `LC_ALL=C` and warm filesystem caches. The driver rotated the updated Tonari, Bash and Tonari `903f975` for five warm-ups and 30 measured runs per case and mode. Cells report median / nearest-rank p95 in milliseconds.

M1 のネイティブ macOS と Debian 13 arm64 の VM で、各 OS のローカルファイルシステムにバイナリと入力を置いた。各条件で旧版・新版・Bash 5.2.37 の起動順を入れ替え、5回のウォームアップ後に各30回計測した。表は中央値 / 95 パーセンタイル（ms）。

### Script timings on macOS 27.2.1 arm64

| Script / 処理 | Whole Tonari | Whole Bash | Started Tonari | Started Bash |
|---|---:|---:|---:|---:|
| 10,000 integer additions | 13.838 / 19.157 | 21.354 / 23.313 | 10.122 / 11.643 | 14.787 / 15.288 |
| 4,000 function calls with local variables | 13.034 / 14.418 | 23.487 / 25.129 | 9.593 / 10.248 | 16.820 / 17.338 |
| 2,000 basename and extension trims | 8.996 / 9.423 | 17.680 / 19.701 | 5.137 / 5.654 | 11.168 / 12.381 |
| 2,000 lines via `IFS= read -r` | 10.315 / 15.211 | 17.618 / 34.060 | 6.679 / 9.624 | 10.713 / 13.918 |
| 64 builtin command substitutions | 44.078 / 45.762 | 58.038 / 63.804 | 40.756 / 44.604 | 52.720 / 57.455 |
| 16 sequential `/usr/bin/true` calls | 23.576 / 25.424 | 39.243 / 41.398 | 20.342 / 23.535 | 33.466 / 40.545 |

### Script timings on Debian 13 arm64

| Script / 処理 | Whole Tonari | Whole Bash | Started Tonari | Started Bash |
|---|---:|---:|---:|---:|
| 10,000 integer additions | 10.233 / 11.905 | 23.842 / 24.354 | 9.408 / 10.483 | 23.417 / 25.234 |
| 4,000 function calls with local variables | 12.116 / 13.933 | 24.925 / 25.676 | 10.228 / 12.063 | 24.232 / 27.205 |
| 2,000 basename and extension trims | 5.232 / 6.933 | 14.748 / 15.521 | 4.929 / 5.559 | 14.299 / 15.117 |
| 2,000 lines via `IFS= read -r` | 5.360 / 6.003 | 11.053 / 12.477 | 5.249 / 6.129 | 10.609 / 11.415 |
| 64 builtin command substitutions | 8.562 / 9.853 | 14.691 / 15.467 | 8.228 / 9.209 | 14.096 / 16.344 |
| 16 sequential `/usr/bin/true` calls | 3.628 / 3.966 | 5.176 / 5.939 | 3.302 / 4.151 | 4.604 / 5.069 |

The previous M1 build took 13.837 ms median on the started-shell string workload. The updated build took 5.137 ms, versus 11.168 ms for Bash. The 64 command substitutions on that host still took over two-thirds of Bash's time in both timing modes. The selected fast path kept the release binaries at 819,776 bytes on macOS and 987,496 bytes for static arm64 musl. [Engineering notes](engineering-notes.md)

M1 で起動後にパスを加工する処理は、変更前 13.837 ms、変更後 5.137 ms、Bash 11.168 ms だった。M1 のコマンド置換64回は起動込み・起動後ともに Bash の所要時間の 2/3 を超えている。

A separate 50-run check loaded the real nvm 0.40.5 with `--no-use` from an empty `NVM_DIR` and verified the resulting function definition. Time includes shell startup.

実際の nvm 0.40.5 を空の `NVM_DIR` から読み込み、関数が定義されたことを確認した。起動を含む各50回の値。

| Host | Tonari | Bash |
|---|---:|---:|
| macOS 27.2.1 arm64 | 6.013 / 6.953 | 12.385 / 12.997 |
| Debian 13 arm64 | 4.502 / 5.065 | 7.620 / 8.755 |

```sh
python3 tools/bench_scripts.py target/release/tonari --bash /path/to/bash --runs 30
python3 tools/bench_scripts.py target/release/tonari --bash /path/to/bash --previous /path/to/old/tonari --case strings
python3 tools/bench.py target/release/tonari --bash /path/to/bash --case nvm --nvm-script /path/to/nvm.sh --runs 50
```

## Integrated release build / 統合リリースビルド

Measured on 2026-11-22, after process substitution, shell command hashing, parameter modifiers, and parser allocation changes. The source subtree is `527055de2275e45b0ef7c5a83bcb72d826a96e94`. From a checkout, run `git rev-parse HEAD:src` to compare. The harness is blob `1f7f36916222e276727e0cdd8802381c9e15fdf1`. Build flags, hardware, reference binaries and workloads match the interactive-shell milestone below. Each workload had five warm-ups and 300 rotated samples per shell, with 100 measured for nvm. Time cells are **median / p95 in milliseconds**, ratio cells are the two corresponding Tonari/Bash ratios.

プロセス置換、PATH 検索キャッシュ、追加のパラメータ展開とパーサーのメモリ確保改善を含む統合版を、下記マイルストーンと同一条件で測定しました。各5回のウォームアップ後、起動順を入れ替えて各300回、nvm は各100回計測しました。時間は **中央値 / p95（ミリ秒）**、比はそれぞれの統計量を Bash で割ったものです。

### Debian 13 arm64

| Metric / 項目 | Tonari | GNU Bash 5.2.37 | Tonari/Bash |
|---|---:|---:|---:|
| First prompt / プロンプト | 0.623 / 0.847 | 0.969 / 1.360 | 0.643 / 0.623 |
| Exec to prompt / exec 以降 | 0.334 / 0.462 | 0.681 / 0.967 | 0.490 / 0.477 |
| `-c ':'` | 0.196 / 0.252 | 0.418 / 0.566 | 0.468 / 0.446 |
| One external / 外部1個 | 0.303 / 0.384 | 0.526 / 0.642 | 0.576 / 0.598 |
| Pipeline / パイプ | 0.542 / 0.672 | 0.839 / 1.057 | 0.646 / 0.636 |
| Boolean builtins / 真偽値 | 0.194 / 0.247 | 0.414 / 0.510 | 0.467 / 0.485 |
| 100 redirects / リダイレクト100回 | 0.451 / 0.563 | 0.886 / 1.066 | 0.510 / 0.529 |
| Session builtins / セッション | 0.219 / 0.279 | 0.429 / 0.511 | 0.510 / 0.546 |
| 16 external programs / 外部16個 | 3.396 / 4.778 | 4.930 / 7.573 | 0.689 / 0.631 |
| 1,000 loop iterations / ループ1,000回 | 0.914 / 1.135 | 2.338 / 2.934 | 0.391 / 0.387 |
| nvm 0.40.5 `--no-use` | 4.377 / 5.444 | 7.584 / 9.600 | 0.577 / 0.567 |

### macOS 27.2.1 arm64

| Metric / 項目 | Tonari | GNU Bash 5.2.37 | Apple Bash 3.2.57 | Tonari/5.2.37 | Tonari/3.2.57 |
|---|---:|---:|---:|---:|---:|
| First prompt / プロンプト | 4.806 / 5.926 | 7.906 / 9.752 | 5.573 / 6.818 | 0.608 / 0.608 | 0.862 / 0.869 |
| Exec to prompt / exec 以降 | 3.236 / 4.070 | 6.443 / 8.295 | 4.049 / 5.072 | 0.502 / 0.491 | 0.799 / 0.802 |
| `-c ':'` | 2.587 / 3.195 | 5.654 / 7.148 | 3.248 / 4.356 | 0.458 / 0.447 | 0.796 / 0.733 |
| One external / 外部1個 | 3.892 / 5.351 | 7.165 / 9.058 | 4.640 / 6.127 | 0.543 / 0.591 | 0.839 / 0.873 |
| Pipeline / パイプ | 4.332 / 5.894 | 8.450 / 10.715 | 5.819 / 7.435 | 0.513 / 0.550 | 0.744 / 0.793 |
| Boolean builtins / 真偽値 | 2.693 / 3.928 | 5.701 / 7.212 | 3.293 / 4.339 | 0.472 / 0.545 | 0.818 / 0.905 |
| 100 redirects / リダイレクト100回 | 3.744 / 5.045 | 7.028 / 8.447 | 4.627 / 5.960 | 0.533 / 0.597 | 0.809 / 0.846 |
| Session builtins / セッション | 2.779 / 4.120 | 5.894 / 8.271 | 3.370 / 5.158 | 0.472 / 0.498 | 0.825 / 0.799 |
| 16 external programs / 外部16個 | 22.234 / 31.298 | 37.592 / 46.769 | 34.522 / 43.780 | 0.591 / 0.669 | 0.644 / 0.715 |
| 1,000 loop iterations / ループ1,000回 | 3.619 / 4.600 | 7.303 / 9.829 | 5.559 / 7.454 | 0.496 / 0.468 | 0.651 / 0.617 |
| nvm 0.40.5 `--no-use` | 5.907 / 7.854 | 12.068 / 14.147 | 9.494 / 11.518 | 0.489 / 0.555 | 0.622 / 0.682 |

Against GNU Bash 5.2.37, every macOS median meets the 2/3 target. Debian meets it on every case except 16 consecutive external commands, at 0.689. In this build nvm loads in 4.377 ms on Debian, ratio 0.577. Against Apple’s system Bash 3.2.57, every Tonari median is faster and the 16-external series, arithmetic loop and nvm meet 2/3. Direct single `/usr/bin/true` launch was 0.210 / 0.257 ms on Debian and 1.174 / 1.888 ms on macOS.

GNU Bash 5.2.37 比の中央値は、macOS では全項目が 2/3 以下、Debian では外部16個（0.689）以外が 2/3 以下でした。Debian の nvm 読み込みは 4.377 ms、Bash 比 0.577 です。Apple 同梱の Bash 3.2.57 と比べても中央値は全項目で短く、外部16個・算術ループ・nvm が 2/3 以下でした。`/usr/bin/true` 一つの直接起動は Debian が 0.210 / 0.257 ms、macOS が 1.174 / 1.888 ms です。

| Host / ホスト | Tonari (bytes) | GNU Bash 5.2.37 | Apple Bash 3.2.57 |
|---|---:|---:|---:|
| Debian 13 arm64 | 987,496 | 1,412,096 | — |
| macOS 27.2.1 | 819,760 | 1,119,952 | 1,961,872 universal / 667,584 arm64e slice |

The measured Tonari executables have SHA-256 `09e9f4e0ce62a47cb916a3f476d5abae4c360019a8a894f6e2769858779c7c52` on Linux and `96bc503b5575fac344da3d6cdb17e335d3ef27533dc022a286028723075b86fb` on macOS.

A separate 300-sample rotation timed a persistent C launcher as well as the Python control for 16 child processes. C uses the same libc as Tonari; its own startup is outside the clock. The [control methodology](#starting-16-real-child-processes--外部16プロセスの直接起動) gives the exact timing boundaries.

外部16個の再計測では、同じ libc の常駐 C プロセスも加えて各300回交互に測りました。C 版の計時区間には親自身の起動は含まれません。

| Host / ホスト | Tonari | GNU Bash 5.2.37 | Apple Bash 3.2.57 | Direct C | Direct Python |
|---|---:|---:|---:|---:|---:|
| Debian 13 arm64 | 3.338 / 4.148 | 4.845 / 6.399 | — | 3.062 / 4.091 | 3.454 / 4.693 |
| macOS 27.2.1 | 21.912 / 28.357 | 37.052 / 45.151 | 34.643 / 42.971 | 19.007 / 24.170 | 19.260 / 23.717 |

<a id="first-key-history"></a>

## First keystroke with shared history / 共有履歴と最初のキー入力

On an Apple M1 running macOS 27.2.1, each sample launched the native release build of `76cc5f1` in a fresh PTY and a temporary home with `--noprofile --norc`, `TERM=xterm-256color` and `PS1='tonari$ '`. After reading the initial prompt, the driver timed writing `z` until reading the redrawn `tonari$ z`. Each journal contained 31-byte ASCII rows of the form `1\t1\t0\t/tmp\tprintf entry-000000\n`, with actual tabs and a newline and a six-digit counter starting at zero. No row matched the prefix. Sizes rotated between launches, with three warm-ups then 30 samples per size. p95 uses the nearest rank.

| Journal entries / 履歴件数 | Bytes | Median (ms) | p95 (ms) |
|---:|---:|---:|---:|
| 0 | 0 | 0.063 | 0.091 |
| 1,000 | 31,000 | 0.301 | 0.363 |
| 100,000 | 3,100,000 | 20.635 | 21.060 |

macOS 27.2.1 の M1 で、各回新しい疑似端末と一時ホームを使いました。起動済みのプロンプトに最初の `z` を送ってから、その文字が再描画されるまでを測っています。初回の履歴読み込みと、一致する候補がない場合の検索を含みます。履歴件数を入れ替え、各条件3回のウォームアップ後に30回計測しました。

### Input-priority history refresh / 履歴読み込み中のキー応答

We reran the preceding warm-cache M1 test with the old and updated releases and the same journals. Three warm-ups and 30 measured runs rotated through the builds, entry counts and actions. Immediately after observing the first character, the PTY driver sent Tab to complete the sole command `zcomplete`, or Enter to run `:` and return to the next prompt. A separate run sent `p` and waited for the newest matching history suggestion to become visible. Each cell is **median / p95 in milliseconds**. [Reproducible harness](../tools/bench_history.py)

同じ M1 と履歴で旧版・新版を交互に起動しました。各条件3回のウォームアップ後に30回計測しています。最初の一文字を読み取った直後に Tab でコマンド名を補完するか、Enter で `:` を実行して次のプロンプトを待ちました。別の試行で `p` を入力してから履歴候補そのものが表示されるまでも測りました。

| Entries / 件数 | Interval / 計測区間 | Previous | Updated |
|---:|---|---:|---:|
| 0 | First `z` → redraw | 0.067 / 0.086 | 0.029 / 0.041 |
| 1,000 | First `z` → redraw | 0.300 / 0.345 | 0.024 / 0.038 |
| 100,000 | First `z` → redraw | 20.499 / 21.161 | 0.036 / 0.050 |
| 100,000 | Tab → completed word | 0.355 / 0.399 | 0.090 / 0.167 |
| 100,000 | First `z` → completed word after Tab | 20.858 / 21.512 | 0.132 / 0.207 |
| 100,000 | Enter → next prompt | 0.299 / 0.352 | 0.191 / 0.236 |
| 100,000 | First `:` → next prompt after Enter | 20.953 / 21.982 | 0.218 / 0.271 |
| 100,000 | First `p` → visible history suggestion | 20.270 / 21.711 | 12.448 / 13.258 |

Repeat with `python3 tools/bench_history.py /path/to/previous/tonari target/release/tonari`. Both macOS arm64 executables were 819,760 bytes.

<a id="completion-latency"></a>

## Tab followed by typing / Tab 直後の打鍵

Each fresh PTY received a prefix, then Tab and `@` in a single write. The timer stopped when the PTY delivered `@`. A full redraw outside the timed interval verified the line ended in `item-0@`. One directory contained 10,000 executable files named `item-00000.dat` through `item-09999.dat` and was the only directory on PATH. The completion cases used an empty history and an rc file registering `complete -f demo`. Caches were warm. The builds rotated through three warm-ups and 30 measured sessions per case. All times are **median / p95 milliseconds**, with nearest-rank p95. [Benchmark driver](../tools/bench_completion.py)

各回新しい疑似端末を開き、入力済みの文字列に続けて Tab と `@` を同時に送った。`@` が端末に返るまでを測り、計測後に行を再表示して、補完した文字列の後ろに `@` が入ったことを確認した。一つのディレクトリに実行可能ファイル1万個を用意し、PATH にもそのディレクトリだけを指定した。各条件3回のウォームアップ後、順番を入れ替えながら各30回計測した。表示値は中央値 / 95 パーセンタイル（ms）。

| Host / ホスト | Tab case / 対象 | Tonari `c23b986` | Updated / 変更後 | GNU Bash 5.2.37 |
|---|---|---:|---:|---:|
| macOS M1 | File | 23.541 / 24.566 | 5.306 / 5.775 | 10.466 / 11.204 |
| macOS M1 | Registered `complete -f` | 36.043 / 37.955 | 5.316 / 5.622 | 11.241 / 11.681 |
| macOS M1 | Command on PATH | 22.024 / 23.707 | 5.160 / 5.539 | 159.168 / 162.314 |
| Debian 13 arm64 | File | 15.485 / 17.599 | 2.165 / 2.345 | 4.603 / 4.907 |
| Debian 13 arm64 | Registered `complete -f` | 22.118 / 23.763 | 2.125 / 2.287 | 5.932 / 6.718 |
| Debian 13 arm64 | Command on PATH | 14.892 / 16.178 | 1.919 / 2.115 | 66.534 / 70.168 |

A separate Tonari-only run started after loading 100,000 journal entries. The PTY sent one character per redraw for a prefix matching the newest entry (390 keystrokes) and a prefix with no match (330 keystrokes including UTF-8). The oldest-entry case measured the same keystroke twice, first to the appearance of the typed character and then to its suggestion. These timings end at the PTY and exclude terminal-emulator rendering and display refresh.

履歴候補の計測では10万件の読み込みを終えてから、一文字が再描画されるたびに次の文字を送った。最新履歴に一致する390打鍵と、日本語を含み候補がない330打鍵を集計した。最古の履歴だけに一致する試行は、同じ打鍵から入力文字と履歴候補がそれぞれ返るまでを測定した。計測の終点は疑似端末の出力で、端末アプリの描画やディスプレイの更新時間は含まない。

| Host / ホスト | Interval / 計測区間 | Tonari `c23b986` | Updated / 変更後 |
|---|---|---:|---:|
| macOS M1 | Keystroke, newest match | 0.013 / 0.027 | 0.012 / 0.023 |
| macOS M1 | Keystroke, no match | 0.269 / 0.384 | 0.013 / 0.026 |
| macOS M1 | Oldest match, typed character | 0.378 / 0.410 | 0.015 / 0.047 |
| macOS M1 | Oldest match, visible hint | 0.381 / 0.424 | 0.682 / 0.852 |
| Debian 13 arm64 | Keystroke, newest match | 0.061 / 0.094 | 0.060 / 0.086 |
| Debian 13 arm64 | Keystroke, no match | 0.269 / 0.335 | 0.060 / 0.258 |
| Debian 13 arm64 | Oldest match, typed character | 0.269 / 0.331 | 0.231 / 0.273 |
| Debian 13 arm64 | Oldest match, visible hint | 0.276 / 0.338 | 0.263 / 0.305 |

The M1 ran macOS 27.2.1 natively and Debian 13 arm64 in a Linux VM with the fixture on the VM filesystem. The updated release executables were 819,776 bytes on macOS (16 additional bytes) and 987,496 bytes on Debian (static musl). [Implementation notes](engineering-notes.md)

M1 の macOS 27.2.1 と、その上の Linux VM で動く Debian 13 arm64 で計測した。Debian のテストデータは VM 内に作成した。変更後の実行ファイルは macOS が 819,776 バイト（16 バイト増）、Debian の静的 musl 版が 987,496 バイト（同サイズ）。

```sh
python3 tools/bench_completion.py /path/to/previous/tonari target/release/tonari --bash /path/to/bash
```

<a id="completion-listing"></a>

### Second Tab and full candidate list / 2回目の Tab と候補一覧

In a fresh PTY, the driver typed the full common prefix and pressed Tab once. After the shell acknowledged it, the driver sent a second Tab and `@` together. It timed the interval to the echoed `@`, then verified that every expected filename had arrived and that the editable line still ended in `@`. For full output at 10,000 entries, Bash ran with Readline's `completion-query-items` set to zero and `page-completions` off. Tonari used its default listing. Each shell ran three warm-ups and 30 measured sessions with warm local filesystems, alternating shell order. These are PTY timings, in milliseconds, for Tonari `130ecd8` and GNU Bash 5.2.37.

共通部分をすべて入力して最初の Tab を押し、その応答後に2回目の Tab と `@` を一緒に送った。全候補の後に `@` が返るまでを計り、ファイル名の集合と入力行を検証した。1万件を最後まで出力できるよう、Bash は件数確認とページ送りを無効にし、Tonari は既定設定で測った。各条件は3回のウォームアップ後に各30回。端末アプリの描画時間は含まない。

| Host / ホスト | Files / 件数 | Tonari median / p95 | Bash median / p95 |
|---|---:|---:|---:|
| macOS M1 | 10 | 0.068 / 0.108 | 0.097 / 0.127 |
| macOS M1 | 10,000 | 7.954 / 8.559 | 34.246 / 36.265 |
| Debian 13 arm64 | 10 | 0.088 / 0.101 | 0.101 / 0.141 |
| Debian 13 arm64 | 10,000 | 5.938 / 6.576 | 27.616 / 31.348 |

```sh
python3 tools/bench_completion.py target/release/tonari --bash /path/to/bash --listing
```

## Interactive shell milestone / 対話シェル統合版

Source tree `119e729ae01cef7d8d0883fdd92bb5fbd0bf447f`, measured on 2026-11-22. The harness is Git blob `1f7f36916222e276727e0cdd8802381c9e15fdf1`. Both arm64 release builds use Rust 1.96.1, native CPU tuning, `opt-level=s`, fat LTO, stripped symbols and one codegen unit. The Linux distribution build statically links musl 1.2.6; its host was Debian 13 userspace on a Linux 6.11.11 VM on Apple M1. The macOS run used the same M1 with macOS 27.2.1. All shells received `--noprofile --norc`, a temporary home, a fixed prompt, and used their normal interactive editor. Cases had five warm-ups and 300 measured samples per shell in rotating order, except nvm (100). Each time cell is **median / p95 in milliseconds**. Each ratio cell is **median / p95**, divided by the corresponding Bash statistic.

2026-11-22、上記の同一ソースを Apple M1 の macOS と Linux VM で計測しました。Rust 1.96.1、CPU ネイティブ最適化、`s`、fat LTO、strip、codegen unit 一つを使用し、Linux 版は musl 1.2.6 を静的リンクしています。どのシェルも設定ファイルを無効にし、一時ホーム・同一のプロンプト・通常の対話エディターで起動しました。ウォームアップ5回後、各条件・各シェルを順番を入れ替えながら300回、nvm は100回測定しました。時間は **中央値 / p95（ミリ秒）**、比は各々を同じ統計量の Bash で割った値です。

### Debian 13 arm64 · GNU Bash 5.2.37

| Metric / 項目 | Tonari | GNU Bash | Tonari/Bash |
|---|---:|---:|---:|
| First prompt / プロンプト | 0.685 / 1.034 | 1.108 / 1.492 | 0.618 / 0.693 |
| Exec to prompt / exec 以降 | 0.362 / 0.504 | 0.776 / 1.088 | 0.466 / 0.464 |
| `-c ':'` | 0.188 / 0.229 | 0.416 / 0.500 | 0.451 / 0.458 |
| One external / 外部1個 | 0.308 / 0.403 | 0.550 / 0.720 | 0.560 / 0.561 |
| Pipeline / パイプ | 0.563 / 0.704 | 0.917 / 1.099 | 0.614 / 0.641 |
| Boolean builtins / 真偽値 | 0.194 / 0.243 | 0.431 / 0.515 | 0.449 / 0.472 |
| 100 redirects / リダイレクト100回 | 0.450 / 0.522 | 0.894 / 1.024 | 0.503 / 0.510 |
| Session builtins / セッション | 0.220 / 0.285 | 0.447 / 0.562 | 0.493 / 0.507 |
| 16 external programs / 外部16個 | 3.314 / 3.707 | 4.820 / 5.441 | 0.688 / 0.681 |
| 1,000 loop iterations / ループ1,000回 | 0.981 / 1.090 | 2.295 / 2.555 | 0.427 / 0.427 |
| nvm 0.40.5 `--no-use` | 5.482 / 10.953 | 8.179 / 15.695 | 0.670 / 0.698 |

Direct `/usr/bin/true` measured 0.208 / 0.246 ms. Tonari was 987,496 bytes, Bash was 1,412,096. Two medians exceeded 2/3 of Bash elapsed time, the 16-external-command series at 0.688 and nvm at 0.670. The overall prompt met the median target; its p95 ratio was 0.693.

`/usr/bin/true` 直接起動は 0.208 / 0.246 ms。サイズは Tonari 987,496 バイト、Bash 1,412,096 バイト。中央値で 2/3 を超えたのは外部16個の 0.688 と nvm の 0.670 です。プロンプト表示は中央値が目標範囲で、p95 の比は 0.693 でした。

### macOS 27.2.1 arm64 · GNU Bash 5.2.37 and system Bash 3.2.57

| Metric / 項目 | Tonari | GNU Bash 5.2.37 | System Bash 3.2.57 | Tonari/5.2.37 | Tonari/3.2.57 |
|---|---:|---:|---:|---:|---:|
| First prompt / プロンプト | 4.572 / 5.217 | 7.588 / 8.403 | 5.409 / 6.110 | 0.603 / 0.621 | 0.845 / 0.854 |
| Exec to prompt / exec 以降 | 3.071 / 3.565 | 6.156 / 6.845 | 3.938 / 4.468 | 0.499 / 0.521 | 0.780 / 0.798 |
| `-c ':'` | 2.707 / 3.124 | 5.658 / 6.339 | 3.160 / 3.699 | 0.478 / 0.493 | 0.857 / 0.845 |
| One external / 外部1個 | 3.356 / 3.892 | 6.256 / 7.039 | 3.841 / 4.554 | 0.537 / 0.553 | 0.874 / 0.855 |
| Pipeline / パイプ | 3.872 / 5.596 | 7.719 / 9.175 | 5.166 / 7.047 | 0.502 / 0.610 | 0.750 / 0.794 |
| Boolean builtins / 真偽値 | 2.671 / 6.006 | 5.755 / 10.886 | 3.258 / 7.529 | 0.464 / 0.552 | 0.820 / 0.798 |
| 100 redirects / リダイレクト100回 | 3.968 / 6.639 | 7.560 / 11.709 | 4.869 / 7.432 | 0.525 / 0.567 | 0.815 / 0.893 |
| Session builtins / セッション | 2.561 / 2.985 | 5.467 / 6.313 | 3.022 / 3.708 | 0.468 / 0.473 | 0.847 / 0.805 |
| 16 external programs / 外部16個 | 21.085 / 32.224 | 34.903 / 51.203 | 32.256 / 51.388 | 0.604 / 0.629 | 0.654 / 0.627 |
| 1,000 loop iterations / ループ1,000回 | 3.503 / 5.261 | 7.213 / 10.757 | 5.408 / 8.037 | 0.486 / 0.489 | 0.648 / 0.655 |
| nvm 0.40.5 `--no-use` | 5.616 / 6.010 | 11.787 / 12.582 | 9.327 / 9.944 | 0.476 / 0.478 | 0.602 / 0.604 |

Direct `/usr/bin/true` measured 1.200 / 1.865 ms. Tonari was 770,096 bytes. Native GNU Bash 5.2.37 was 1,119,952 bytes. Apple’s system Bash was 1,961,872 bytes including three architecture slices, with 667,584 bytes in the arm64e slice. Every measured Tonari median fell within 2/3 of native GNU Bash 5.2.37. Against system Bash, the 16-external series, the 1,000-iteration loop and nvm met that threshold. Every other median was faster than system Bash and above the target.

`/usr/bin/true` 直接起動は 1.200 / 1.865 ms。サイズは Tonari 770,096 バイト、ネイティブの GNU Bash 5.2.37 が 1,119,952 バイト、OS 同梱 Bash は3アーキテクチャ合計 1,961,872 バイト（arm64e 部分 667,584 バイト）です。GNU Bash 5.2.37 比では全項目の中央値が 2/3 以下でした。OS 同梱版と比べて 2/3 以下になったのは外部16個・ループ1,000回・nvm で、それ以外の中央値も同梱版より速く、目標には届きませんでした。

### Starting 16 real child processes / 外部16プロセスの直接起動

A separate 300-sample run added a persistent native C parent. It timed 16 calls to `posix_spawn` on `/usr/bin/true`, each followed by a blocking `waitpid`, using `CLOCK_MONOTONIC` inside the parent. The C process inherited the same environment and configured `/dev/null` once before timing. It used the same musl libc as Tonari on Linux and libSystem on macOS. The Python harness rotated the four launch paths, adding system Bash as a fifth on macOS. The C parent’s own startup and communication with the harness were outside the timer. The shell cases include their own startup. These are empirical controls for this workload.

別計測では、常駐する C の親プロセスから `/usr/bin/true` を16回、`posix_spawn` と `waitpid` で一つずつ起動・待機しました。親の起動と計測器との通信は計時外で、シェル側にはシェル自身の起動も含まれます。Linux の C 版は Tonari と同じ musl を使用しました。各条件を交互に300回実行しています。

| Host / ホスト | Tonari | GNU Bash 5.2.37 | System Bash 3.2.57 | Direct C | Direct Python |
|---|---:|---:|---:|---:|---:|
| Debian 13 arm64 | 3.492 / 5.535 | 5.048 / 8.063 | — | 3.257 / 5.663 | 3.576 / 5.349 |
| macOS 27.2.1 | 20.905 / 28.666 | 34.625 / 48.050 | 32.015 / 44.899 | 17.972 / 24.555 | 18.116 / 25.018 |

For the nvm case, source `nvm.sh` v0.40.5 (`cd6f374433a22ec01919a834cf59d03e0ba07c226bed460a50ef5eecc38c39ef` SHA-256) with `--no-use` and an empty `NVM_DIR`. The shell checks that the `nvm` function was defined before returning success. The figures cover sourcing with warm filesystem caches. [nvm v0.40.5](https://github.com/nvm-sh/nvm/tree/v0.40.5)

nvm の項目は、空の `NVM_DIR` を用意して v0.40.5 の `nvm.sh` を `--no-use` で読み込み、最後に `nvm` 関数の定義を確認します。キャッシュが温まった状態での読み込み時間です。

```sh
python3 tools/bench.py target/release/tonari --bash /path/to/bash-5.2.37 --compare-shell System /bin/bash --runs 300
python3 tools/bench.py target/release/tonari --bash /path/to/bash-5.2.37 --compare-shell System /bin/bash --nvm-script /path/to/nvm.sh --case nvm --runs 100
```

## Parser allocation experiment / パーサーのメモリ確保

Starting from `119e729` on Debian 13 arm64, we changed the initial capacity of three parser vectors. A statement list starts with one slot, command words and expansion parts start with two. In the same rotating 250-sample run, sourcing nvm 0.40.5 improved by 12% at the median. Both static musl binaries were 987,496 bytes. The table shows **median / p95 in milliseconds**.

Debian 13 arm64 で `119e729` を基に、文リストの初期確保を1要素、コマンドの単語列と展開部分列を2要素に変えました。各250回の交互計測で nvm 読み込みの中央値が12%短くなりました。静的 musl バイナリはどちらも 987,496 バイトです。各欄は **中央値 / p95（ミリ秒）** です。

| Metric / 項目 | Tonari before / 変更前 | Tonari after / 変更後 | GNU Bash 5.2.37 |
|---|---:|---:|---:|
| Source nvm `--no-use` | 5.044 / 6.755 | 4.436 / 5.247 | 7.716 / 9.471 |
| 1,000 loop iterations / ループ1,000回 | 0.858 / 1.167 | 0.831 / 1.019 | 2.447 / 2.922 |
| `-c ':'` | 0.183 / 0.250 | 0.181 / 0.233 | 0.422 / 0.598 |

## Selecting release optimization on macOS / macOS の最適化レベル

The following Apple M1 run compares three builds of Git tree `dffc90bd24207a49ce4d148aa506f07128ff2c81`. Rust 1.96.1 used native CPU tuning, fat LTO, one codegen unit, symbol stripping and panic abort throughout. Each cell is a median in milliseconds. The basic cases used 200 measured iterations, the last two used 100. All five binaries ran in rotating order on macOS 27.2.1. GNU Bash 5.2.37 and system Bash 3.2.57 are distinct macOS references. Bash 5.2.37 used the bundled Readline and Apple Clang 21.0.0 with `CFLAGS='-O3 -mcpu=native -flto' LDFLAGS='-flto' ./configure --prefix=…`, `make -j4`, and `strip`. Its tarball SHA-256 is `9599b22ecd1d5787ad7d3b7bf0c59f312b3396d1e281175dd1f8a4014da621ff`. [GNU Bash release source](https://ftp.gnu.org/gnu/bash/bash-5.2.37.tar.gz)

Apple M1 の macOS 27.2.1 で同一ソースの最適化レベルを比較しました。Rust 1.96.1、CPU ネイティブ最適化、fat LTO、codegen unit 一つ、strip、panic abort は共通です。各欄はミリ秒の中央値です。通常項目は200回、末尾2項目は100回、5本の実行順を入れ替えて計測しました。上流ソースからビルドした Bash 5.2.37 と OS 同梱の Bash 3.2.57 はそれぞれ独立した比較対象です。

| Metric / 項目 | Tonari `3` | Tonari `s` | Tonari `z` | Bash 5.2.37 | System Bash 3.2.57 |
|---|---:|---:|---:|---:|---:|
| First prompt / プロンプト | 4.836 | 4.880 | 4.816 | 8.113 | 5.711 |
| `-c ':'` | 2.681 | 2.574 | 2.617 | 5.748 | 3.232 |
| One external / 外部1個 | 3.877 | 3.968 | 3.889 | 7.314 | 4.649 |
| Pipeline / パイプ | 4.256 | 4.278 | 4.240 | 8.370 | 5.750 |
| 100 redirects / リダイレクト100回 | 3.676 | 3.658 | 3.702 | 7.004 | 4.541 |
| 1,000 loop iterations / ループ1,000回 | 3.350 | 3.338 | 4.143 | 7.274 | 5.327 |
| 16 external programs / 外部16個 | 26.525 | 26.201 | 26.386 | 44.889 | 41.702 |
| Executable bytes / バイト数 | 769,504 | 687,088 | 605,952 | 1,119,952 | 1,961,872 |

The system executable includes three architectures. Its arm64e slice occupies 667,584 bytes. Level `s` reduced Tonari by 82,416 bytes compared with level 3 and kept the loop at 3.338 ms. Level `z` made the loop 24% slower than `s`. The release profile now uses `s`, which was already the static Linux arm64 distribution setting.

OS 同梱版には3種のアーキテクチャが含まれ、arm64e の部分は 667,584 バイトです。レベル `s` はレベル3より 82,416 バイト小さく、ループは 3.338 ms でした。`z` ではループが `s` より24%遅くなります。この結果を受け、Linux arm64 の静的配布版ですでに使っていた `s` を共通のリリース設定にしました。

## Executor baseline / 実行系の初期計測

The tables compare release builds on an Apple M1, with 5 warm-ups and 300 measured runs per case and executable. Each timing is **median / p95 in milliseconds**. The harness rotates the executables each round. The original Tonari is commit `2874dc8`, rebuilt with the same Rust version (1.96.1) and native CPU settings. Bash skips profile and rc files.

`-c` timing now starts at a native `posix_spawn` call and ends after `waitpid`. The direct `/usr/bin/true` measurement uses the same launcher. Prompt timing includes the Python parent creating a pseudo-terminal, forking and reading the first prompt. Use results from the same run to compare builds.

Apple M1 上でリリースビルドを使い、各項目・実行ファイルにつきウォームアップ5回の後、起動順を入れ替えながら300回計測しました。各欄はミリ秒で **中央値 / p95** を示します。旧版はコミット `2874dc8` を同じ Rust 1.96.1、同じ CPU 向け設定で再ビルドしたものです。Bash は profile と rc を無効にしています。

今回から `-c` は OS の `posix_spawn` 呼び出し直前から `waitpid` の完了までを測ります。`/usr/bin/true` の直接起動も同じ方法です。プロンプト表示には Python の親が疑似端末を作り、fork して最初のプロンプトを読み取る時間を含みます。変更前後は同じ回の測定値で比較してください。

### Debian 13 arm64

| Metric / 計測項目 | Tonari @ 2874dc8 | Tonari (glibc) | Tonari (static musl) | Bash 5.2.37 |
|---|---:|---:|---:|---:|
| First prompt / プロンプト表示 | 0.732 / 0.920 | 0.725 / 0.902 | 0.602 / 0.746 | 1.027 / 1.229 |
| `-c ':'` | 0.300 / 0.372 | 0.303 / 0.379 | 0.170 / 0.212 | 0.430 / 0.519 |
| `/usr/bin/true` | 0.412 / 0.499 | 0.408 / 0.503 | 0.286 / 0.358 | 0.547 / 0.660 |
| `/usr/bin/printf x \| /usr/bin/wc -c` | 0.688 / 0.831 | 0.679 / 0.840 | 0.509 / 0.628 | 0.878 / 1.074 |
| `true; false; true` | 0.863 / 1.024 | 0.300 / 0.376 | 0.170 / 0.214 | 0.439 / 0.550 |
| `: >/dev/null` × 100 | 0.643 / 0.756 | 0.473 / 0.571 | 0.664 / 0.778 | 0.927 / 1.082 |
| Size / バイト数 | 397,968 | 397,968 | 397,560 | 1,412,096 |

Direct `/usr/bin/true` / 同じランチャーからの直接起動 **0.219 / 0.262 ms**。

Both new builds use the same source. The GNU build uses the release profile and glibc. The static build uses Alpine's compiler, musl and size optimization. It reached the prompt sooner and finished short commands faster. The GNU build completed the 100 file redirections faster.

新版2本は同じソースです。通常版は GNU ツールチェーンと glibc、静的リンク版は Alpine のコンパイラと musl、サイズ最適化を使用しました。今回、静的リンク版はプロンプト表示と短いコマンドの完了が速く、ファイルへのリダイレクト100回は通常版の方が速い結果でした。

### macOS 27.2.1 arm64

| Metric / 計測項目 | Tonari @ 2874dc8 | Tonari | System Bash 3.2.57 |
|---|---:|---:|---:|
| First prompt / プロンプト表示 | 4.478 / 5.217 | 4.422 / 5.071 | 5.501 / 6.392 |
| `-c ':'` | 2.489 / 2.887 | 2.412 / 2.891 | 3.149 / 3.635 |
| `/usr/bin/true` | 3.591 / 4.375 | 3.575 / 4.300 | 4.445 / 5.098 |
| `/usr/bin/printf x \| /usr/bin/wc -c` | 4.089 / 4.843 | 4.107 / 4.864 | 5.781 / 6.479 |
| `true; false; true` | 6.221 / 7.313 | 2.379 / 2.894 | 3.111 / 3.610 |
| `: >/dev/null` × 100 | 3.772 / 4.384 | 3.587 / 4.268 | 4.734 / 5.743 |
| Size / バイト数 | 370,736 | 354,048 | 1,961,872 |

Direct `/usr/bin/true` / 同じランチャーからの直接起動 **1.172 / 1.493 ms**。

The shell now executes `true` and `false` internally. File redirections on builtins save fewer descriptors. The release profile uses `panic = "abort"`; the macOS executable is smaller. The shell also copies single-quoted arguments in one span, which reduces parsing time for long literal arguments.

`true` と `false` は組み込みで実行し、組み込みのファイルリダイレクトでは記述子の退避を減らしました。リリースビルドには `panic = "abort"` を使い、macOS の実行ファイルも小さくなりました。単一引用符の中身はまとめてコピーするため、長いリテラル引数の解析時間も短縮されています。

## Linux arm64 static PIE

The static build links musl into a position-independent executable. It runs on the Debian and Ubuntu arm64 test systems without a separate musl installation. The tested build uses Alpine's native Rust 1.96.1 package (`aarch64-alpine-linux-musl`), musl development files, a C compiler driver and LLD 22.1.3. Build on the target CPU when using `target-cpu=native`. LLD packs the segments with 64 KiB load alignment. [File sizes and startup timings](#completion-and-case)

静的リンク版は musl を実行ファイルに組み込んだ PIE（位置独立実行形式）です。検証した Debian と Ubuntu の arm64 環境では musl の追加インストールなしで動きます。下記は Alpine のネイティブ Rust パッケージ 1.96.1（`aarch64-alpine-linux-musl`）、musl の開発ファイル、C コンパイラドライバ、LLD 22.1.3 を使ったビルド手順です。`target-cpu=native` はビルドした CPU 向けの設定です。LLD はロード時に 64 KiB 境界で整列できるよう、ファイル内に各セグメントを配置します。

```sh
CARGO_PROFILE_RELEASE_OPT_LEVEL=s \
RUSTFLAGS='-C target-cpu=native -C target-feature=+crt-static' \
cargo rustc --locked --release --target aarch64-alpine-linux-musl -- \
  -C link-arg=-fuse-ld=lld -C link-arg=-Wl,-z,pack-relative-relocs,-z,max-page-size=65536
```

## First prototype / 初回試作

First prototype, measured 2026-11-22. Each timing is **median / p95 in milliseconds**. The prompt test ends when the first prompt is visible on a pseudo-terminal. The other tests start a fresh shell for each command and wait for its exit.

初回試作を 2026-11-22 に計測しました。時間の単位はミリ秒で、各欄に **中央値 / p95** を示します。プロンプトの計測は、疑似端末に最初のプロンプトが出力されるまでです。ほかの計測では毎回新しいシェルを起動し、その終了までを測っています。

### Debian 13 arm64

Linux VM on Apple M1, Debian userspace, GNU Bash 5.2.37, brush 0.4.0. The executables ran on the VM's local filesystem.

Apple M1 上の Linux 仮想マシンで Debian のユーザーランドを使い、VM 内のファイルシステムに置いた実行ファイルで計測しました。

| Metric / 計測項目 | Tonari | GNU Bash | brush |
|---|---:|---:|---:|
| First prompt / プロンプト表示 | 0.895 / 1.381 | 1.148 / 1.444 | 1.761 / 2.362 |
| `-c ':'` | 0.335 / 0.416 | 0.451 / 0.559 | 1.149 / 1.398 |
| `-c '/usr/bin/true'` | 0.457 / 0.587 | 0.602 / 0.750 | 1.664 / 2.077 |
| `-c '/usr/bin/printf x \| /usr/bin/wc -c'` | 0.722 / 0.885 | 0.902 / 1.339 | 2.009 / 3.101 |
| Executable bytes / 実行ファイルのバイト数 | 397,968 | 1,412,096 | 6,381,288 |

### macOS 27.2.1 arm64

Apple M1, system Bash 3.2.57, brush 0.4.0. The macOS system Bash is older than the project's compatibility baseline. Its universal executable contains three architecture slices. The arm64e slice is 667,584 bytes. Tonari and brush are single-architecture arm64 builds.

Apple M1 で OS 同梱の Bash 3.2.57 と brush 0.4.0 を使用しました。macOS 同梱の Bash は互換性の基準にしている版より古く、実行ファイルには三つのアーキテクチャが含まれます。うち arm64e 部分は 667,584 バイトです。Tonari と brush は arm64 単体のビルドです。

| Metric / 計測項目 | Tonari | GNU Bash | brush |
|---|---:|---:|---:|
| First prompt / プロンプト表示 | 4.905 / 7.973 | 5.810 / 9.159 | 9.129 / 12.498 |
| `-c ':'` | 3.856 / 4.467 | 4.597 / 5.327 | 7.536 / 8.894 |
| `-c '/usr/bin/true'` | 5.183 / 6.447 | 5.975 / 6.877 | 9.960 / 11.260 |
| `-c '/usr/bin/printf x \| /usr/bin/wc -c'` | 5.609 / 6.594 | 7.144 / 8.168 | 10.915 / 13.067 |
| Executable bytes / 実行ファイルのバイト数 | 370,736 | 1,961,872 | 5,621,680 |

### Initial measurement conditions / 初回の計測条件

- Tonari was built with Rust 1.96.1, fat LTO, one codegen unit and symbol stripping. The reference shells were the OS-provided Bash and the upstream brush release binary. File sizes exclude shared libraries. [brush 0.4.0 binaries](https://github.com/reubeno/brush/releases/tag/brush-shell-v0.4.0)
- The harness uses five warm-ups and 100 measured launches per shell per case, rotating shell order. All shells receive the same minimal environment, temporary home, prompt and `PATH`. Bash and brush skip profile and rc files. brush also skips its own config. Interactive editors retain their defaults.
- Timings include the Python parent's process-launch overhead. First-prompt timing starts before the PTY fork. The harness answers cursor-position queries and stops timing when it reads the prompt. This run covers warm caches, an empty home and one arm64 machine. Real rc files, history, completion, sustained workloads and agent end-to-end latency need separate measurements.

Tonari は Rust 1.96.1 で CPU ネイティブ向けにビルドし、fat LTO、codegen unit 一つ、シンボル除去を使用しました。Bash は各 OS の配布物、brush は上流のリリースバイナリです。サイズは実行ファイル本体の値です。

計測項目ごとに各シェルを5回ウォームアップし、起動順を入れ替えながら各100回計測しました。最小限の環境変数、一時ホーム、プロンプト、`PATH` は共通です。Bash と brush は profile と rc の読み込みを無効にし、brush 固有の設定も無効にしました。対話エディタはそれぞれの既定値です。

時間には Python の親プロセスが起動要求を出すコストも含みます。プロンプト表示の計時は疑似端末の fork 直前に始まり、カーソル位置の問い合わせにはハーネスが応答し、プロンプトを受信した時点で終了します。今回は arm64 の1台で、キャッシュが温まりホームが空の条件です。実際の rc、履歴、補完、連続実行、エージェント全体の待ち時間は別途計測する対象です。

## Benchmark against GNU Bash / GNU Bash との比較

GNU Bash is the ongoing reference for speed and executable size. The initial three-shell run used the benchmark script at commit `33204d5`.

今後は速度と実行ファイルのサイズを GNU Bash と比較します。初回に3種類のシェルを計測したスクリプトはコミット `33204d5` にあります。

```sh
RUSTFLAGS='-C target-cpu=native' cargo build --release --locked
python3 tools/bench.py target/release/tonari --bash /bin/bash --runs 300
```

Use `--compare-tonari Original /path/to/previous/tonari` and, on Linux arm64, `--compare-tonari Static /path/to/static/tonari` to interleave the builds in the same run.

旧版は `--compare-tonari Original /path/to/previous/tonari`、Linux arm64 の静的リンク版は `--compare-tonari Static /path/to/static/tonari` で同じ計測に加えられます。

## Repeated PATH lookup / PATH 検索の繰り返し

On macOS 27.2.1 arm64 (Apple M1), the same Tonari release executable ran 40 commands named `helper` followed by `:`. The last directory on PATH contained a symlink named `helper` to `/usr/bin/true`. Every preceding directory was empty. The benchmark set PATH in the parent environment and ran `tonari --noprofile --norc -c` with either `set +h` or `set -h` at the start of the command string. Each cell is the median wall time for a whole session over 24 paired runs. Launch order alternated and the first pair was discarded. Rust 1.96.1, release profile, `-C target-cpu=native`.

| PATH directories / ディレクトリ数 | Cache disabled / `set +h` | Cache enabled / `set -h` |
| ---: | ---: | ---: |
| 1 | 49.503 ms | 50.392 ms |
| 17 | 94.143 ms | 49.873 ms |

macOS 27.2.1 arm64（Apple M1）で、同じリリース版を使い、`helper` を40回呼び出して最後に `:` を実行しました。親から渡す PATH の末尾に `/usr/bin/true` へのリンクを置き、それより前のディレクトリは空にしています。シェル起動から終了までを計り、キャッシュ有効・無効の実行順を毎回交互に入れ替えました。最初の1組を除いた24組の中央値です。1ディレクトリでは差は約0.9 ms、17ディレクトリではキャッシュにより約44.3 ms 短縮しました。

<a id="feature-audit"></a>

## 2026-11-22 — Timings across the Bash feature inventory / Bash 機能一覧の速度

The shell code was `39b0e6e`. We used native ARM64 release binaries on an Apple M1 with macOS 27.2.1 and a Docker Desktop Linux 6.11.11 VM. Debian 13 used GNU Bash 5.2.37, Ubuntu 26.04 used 5.3.9 and macOS used 5.2.37. The Linux Tonari was a 1,053,592-byte static musl executable. The macOS executable was 886,400 bytes. Bash was 1,412,096, 1,609,176 and 1,208,904 bytes respectively. This is a warm-cache run. The two shell executables were staged on the same filesystem, and shell order alternated between samples. [Environment and binary hashes, Bash then Tonari](compatibility/environment.json)

Each row below counts names with a verified behavioral match, including shared witnesses and GNU golden matches. The denominators for the two timings differ because only canonical command strings support the readiness handshake. Interface-only checks are available in the detailed matrix. The comparison uses the median per executable, with two warmups and 15 measured samples per executable (seven for a GNU driver). The TSVs retain absolute medians and p95 values for every measured feature. [Full matrix](compatibility/matrix.md) · [Method and release gates](compatibility.md)

各セルは「達成した名前の数 / 計測できた名前の数」です。左側は起動込み、右側は起動済みシェルに処理を渡してから終了までです。名前には同じテストを参照する予約語等が含まれます。シェル起動後に再度同期できる形式だけが右側の母数に入ります。中央値で 1/2 が目標、2/3 が上限です。

| Platform | Whole ≤ 1/2 | Whole ≤ 2/3 | Whole ≤ Bash | Started ≤ 1/2 | Started ≤ 2/3 | Started ≤ Bash |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Debian 13 | 7/481 | 324/481 | 471/481 | 6/429 | 23/429 | 360/429 |
| Ubuntu 26.04 | 6/481 | 312/481 | 473/481 | 6/429 | 14/429 | 358/429 |
| macOS | 4/473 | 434/473 | 471/473 | 9/432 | 62/432 | 424/432 |

The checked command substitution `v=$(printf 'a\n\n')` took 0.34× Bash's started time on both Linux systems. The three-stage `printf | tr | wc` pipeline took 0.85× on Debian, 0.88× on Ubuntu and 0.72× on macOS. Waiting for a background subshell’s exit status took 1.05×, 1.07× and 1.17× respectively. These are elapsed times for small, complete programs, so the scheduler, external commands and shell exit contribute to each result. A script that repeats work inside one shell answers a different performance question.

起動済みの Bash と比べると、末尾改行を除去するコマンド置換は両 Linux で 0.34 倍でした。3段のパイプは Debian で 0.85 倍、Ubuntu で 0.88 倍、macOS で 0.72 倍です。バックグラウンドのサブシェルの終了値を `wait` で取得する例はそれぞれ 1.05、1.07、1.17 倍でした。この単発計測には、OS による起床や終了処理も含まれます。

The companion workload run covers repeated scripts (15 samples per shell), startup and command execution (65), and completion, candidate listing and keyboard latency (12 sessions; individual keystroke counts are recorded). The shared editor cases verify the completed text or the set of displayed candidates. Tonari-only history suggestions have absolute latency measurements. [All script, startup and editor measurements](compatibility/workloads.tsv)

反復スクリプト、起動と外部実行、入力・補完・候補一覧の数値も別表に収録しました。キーボード操作では入力が画面に返るまで測り、補完結果の文字列または候補集合を照合しました。履歴からの入力候補については Tonari 自身の遅延を記録しています。

All 30 repeated scripts finished faster than Bash after startup on each platform. At the 2/3 target, Debian and Ubuntu passed 21 of 30 and macOS passed 25 of 30. For an interactive example, listing 10,000 candidates took a median of 4.24 ms in Tonari and 21.52 ms in Bash on Ubuntu. Ordinary filename Tab completion in the 10,000-file fixture took 2.32 ms versus 3.06 ms. The first operation clears the upper target and the second exceeds it.

起動済みシェルで繰り返し処理を行うスクリプト30本は、3環境ともすべて Bash より速く完了しました。上限の 2/3 まで達したのは Debian と Ubuntu で各21本、macOS で25本です。Ubuntu での候補1万件の一覧表示は Tonari が中央値 4.24 ms、Bash が 21.52 ms でした。同じ規模のディレクトリで通常のファイル名補完を行うと 2.32 ms と 3.06 ms で、こちらは上限に届いていません。

<a id="2026-11-23-runtime"></a>

## 2026-11-23 · Corrected operations / 修正した経路

Each cell reports median / p95 in milliseconds. Each script runs inside an already-started shell. Five warmups preceded 25 measured runs per executable, with rotating execution order and verified stdout and status. The previous release is the executable used for the November 22 inventory. All three binaries in each comparison were copied to the same local filesystem. macOS arm64 uses Bash 5.2.37. The arm64 Linux guests use Debian 13 Bash 5.2.37 and Ubuntu 26.04 Bash 5.3.9. Tonari uses the native release configuration on macOS and the static size-optimized musl release on Linux.

修正した経路を、起動済みシェルで同じスクリプトを実行して比較しました。表の値は中央値 / p95（ms）、比は Tonari の中央値を Bash の中央値で割った値です。以前の実装では結果が一致しない新機能の欄は空欄です。パイプには外部の `/usr/bin/printf` と `/bin/cat` の起動・実行・終了待ちが含まれます。

| Platform | Workload | Bash | Previous Tonari | Current Tonari | Current / Bash |
|---|---|---:|---:|---:|---:|
| macOS | `echo-substitutions` | 43.824 / 45.487 | 35.080 / 35.719 | 0.507 / 0.525 | 0.012 |
| macOS | `compound-substitutions` | 11.418 / 11.691 | 9.150 / 9.989 | 0.452 / 0.501 | 0.040 |
| macOS | `pipeline` | 33.457 / 36.377 | 21.711 / 22.222 | 21.605 / 22.175 | 0.646 |
| macOS | `merged-pipeline` | 33.558 / 34.964 | 21.693 / 23.210 | 21.731 / 22.781 | 0.648 |
| macOS | `external-substitutions` | 28.928 / 29.421 | 18.358 / 18.622 | 18.335 / 19.232 | 0.634 |
| macOS | `extended-patterns` | 1.659 / 1.719 | — | 1.021 / 1.065 | 0.615 |
| macOS | `resource-limits` | 4.636 / 4.664 | — | 3.730 / 3.940 | 0.805 |
| macOS | `brace-range` | 4.743 / 5.055 | — | 1.481 / 1.607 | 0.312 |
| macOS | `getopts-clusters` | 1.762 / 1.803 | — | 1.016 / 1.048 | 0.576 |
| macOS | `mapfile-lines` | 3.154 / 3.252 | — | 1.877 / 1.911 | 0.595 |
| macOS | `case-fallthrough` | 2.992 / 3.033 | — | 1.864 / 1.903 | 0.623 |
| macOS | `directory-stack` | 6.154 / 6.302 | — | 5.219 / 5.348 | 0.848 |
| macOS | `descriptor-moves` | 3.628 / 3.677 | — | 3.139 / 3.187 | 0.865 |
| macOS | `integer-assignments` | 3.274 / 3.308 | — | 2.191 / 2.221 | 0.669 |
| Debian | `echo-substitutions` | 14.606 / 24.716 | 8.652 / 9.760 | 0.253 / 0.330 | 0.017 |
| Debian | `compound-substitutions` | 3.865 / 4.856 | 2.540 / 3.350 | 0.230 / 0.365 | 0.060 |
| Debian | `pipeline` | 6.815 / 8.640 | 5.666 / 8.273 | 5.892 / 7.541 | 0.865 |
| Debian | `merged-pipeline` | 6.362 / 7.511 | 5.300 / 6.132 | 5.142 / 6.074 | 0.808 |
| Debian | `external-substitutions` | 5.462 / 6.322 | 3.655 / 4.341 | 3.606 / 4.799 | 0.660 |
| Debian | `extended-patterns` | 1.705 / 1.782 | — | 0.732 / 0.798 | 0.429 |
| Debian | `resource-limits` | 2.090 / 2.185 | — | 1.097 / 1.204 | 0.525 |
| Debian | `brace-range` | 6.336 / 7.006 | — | 1.457 / 1.581 | 0.230 |
| Debian | `getopts-clusters` | 1.757 / 1.781 | — | 0.762 / 0.784 | 0.433 |
| Debian | `mapfile-lines` | 4.261 / 4.472 | — | 1.858 / 1.948 | 0.436 |
| Debian | `case-fallthrough` | 3.564 / 3.947 | — | 1.572 / 1.727 | 0.441 |
| Debian | `directory-stack` | 2.210 / 2.260 | — | 1.216 / 1.248 | 0.550 |
| Debian | `descriptor-moves` | 1.140 / 1.186 | — | 0.987 / 1.002 | 0.865 |
| Debian | `integer-assignments` | 4.259 / 4.758 | — | 1.819 / 2.074 | 0.427 |
| Ubuntu | `echo-substitutions` | 12.487 / 13.228 | 7.732 / 8.187 | 0.220 / 0.260 | 0.018 |
| Ubuntu | `compound-substitutions` | 3.591 / 3.702 | 2.381 / 2.489 | 0.203 / 0.236 | 0.057 |
| Ubuntu | `pipeline` | 18.821 / 19.333 | 16.795 / 17.235 | 16.869 / 19.291 | 0.896 |
| Ubuntu | `merged-pipeline` | 18.879 / 19.387 | 16.895 / 17.295 | 16.833 / 17.307 | 0.892 |
| Ubuntu | `external-substitutions` | 16.051 / 16.495 | 14.347 / 14.595 | 14.265 / 14.604 | 0.889 |
| Ubuntu | `extended-patterns` | 1.442 / 1.466 | — | 0.701 / 0.734 | 0.486 |
| Ubuntu | `resource-limits` | 1.956 / 2.119 | — | 1.078 / 1.236 | 0.551 |
| Ubuntu | `brace-range` | 6.607 / 6.775 | — | 1.399 / 1.424 | 0.212 |
| Ubuntu | `getopts-clusters` | 1.506 / 1.548 | — | 0.758 / 0.795 | 0.503 |
| Ubuntu | `mapfile-lines` | 4.853 / 4.909 | — | 1.835 / 1.891 | 0.378 |
| Ubuntu | `case-fallthrough` | 2.963 / 3.000 | — | 1.559 / 1.608 | 0.526 |
| Ubuntu | `directory-stack` | 2.095 / 2.162 | — | 1.210 / 1.239 | 0.577 |
| Ubuntu | `descriptor-moves` | 1.073 / 1.121 | — | 0.988 / 1.001 | 0.921 |
| Ubuntu | `integer-assignments` | 3.730 / 3.832 | — | 1.820 / 1.944 | 0.488 |

The two builtin substitutions run 16 compound or 64 single-echo captures. The pipeline and external substitution scripts run 16 times each. The range workload expands 1,000 zero-padded numbers 16 times. The option workload parses a cluster and a value across 128 loops. The array workload reads 2,000 lines into an indexed array 16 times. The extended-pattern workload tests 500 case arms containing a repeated character class and a choice of suffixes. The resource-limit workload performs 256 soft-limit updates and queries. The full inventory’s earlier ratios remain attached to its original binary. [Workload definitions](../tools/bench_scripts.py) · [Compatibility changes](compatibility/changes.tsv)
### Startup and executable sizes / 起動時間とサイズ

The tested shell is commit `7144562`, source tree `d3eb7e7529fb8cc293923cc281408bacb2208a2f`. The macOS executable SHA-256 is `1079993cd86465306eacf1b69dcb62f45ec99e15fa4558ffb41a49624606bf35` and the static Linux executable SHA-256 is `03882e6c7cfceff66607382451a02d540b8c6abab3e2ceb99000a622c4e4e52e`. The host was an Apple M1 running macOS 27.2.1. The Linux guests ran native arm64 code. The empty-command and PTY benchmarks rotated each binary through five warmups and 51 measured launches. The Linux PTY fixture provided an empty system bashrc so the prompt marker was stable. The Bash column below is the executable size. Dynamic system libraries are outside that figure.

起動は各実行ファイルを51回測定しました。表は中央値 / p95（ms）です。最初のプロンプトには PTY 子プロセスの起動を含みます。同じ試行から exec の直前から表示までの区間も計測しました。

| Platform | Interval | Bash | Previous Tonari | Current Tonari | Current / Bash |
|---|---|---:|---:|---:|---:|
| macOS | empty command | 4.781 / 5.220 | 2.327 / 2.613 | 2.234 / 2.544 | 0.467 |
| macOS | first prompt | 6.356 / 6.772 | 3.791 / 4.057 | 3.571 / 3.969 | 0.562 |
| macOS | exec to first prompt | 5.268 / 5.615 | 2.571 / 2.727 | 2.470 / 2.692 | 0.469 |
| Debian | empty command | 0.420 / 0.635 | 0.195 / 0.277 | 0.196 / 0.279 | 0.468 |
| Debian | first prompt | 0.903 / 1.181 | 0.548 / 0.685 | 0.524 / 0.760 | 0.580 |
| Debian | exec to first prompt | 0.647 / 0.833 | 0.277 / 0.335 | 0.264 / 0.418 | 0.408 |
| Ubuntu | empty command | 0.414 / 0.488 | 0.190 / 0.232 | 0.194 / 0.226 | 0.468 |
| Ubuntu | first prompt | 0.924 / 1.298 | 0.567 / 0.839 | 0.553 / 1.151 | 0.598 |
| Ubuntu | exec to first prompt | 0.652 / 0.835 | 0.284 / 0.401 | 0.280 / 0.673 | 0.429 |

| Platform | Bash bytes | Previous Tonari bytes | Current Tonari bytes |
|---|---:|---:|---:|
| macOS | 1,208,904 | 886,400 | 969,360 |
| Debian | 1,412,096 | 1,053,592 | 1,146,920 |
| Ubuntu | 1,609,176 | 1,053,592 | 1,146,920 |

Across the 42 script and platform pairs, 17 reached half of Bash's elapsed time and 31 met the two-thirds upper target. These counts apply to these 14 checked workloads on three platforms. The following medians exceeded the upper target.

- macOS — `resource-limits` (0.805), `directory-stack` (0.848), `descriptor-moves` (0.865), `integer-assignments` (0.669).
- Debian — `pipeline` (0.865), `merged-pipeline` (0.808), `descriptor-moves` (0.865).
- Ubuntu — `pipeline` (0.896), `merged-pipeline` (0.892), `external-substitutions` (0.889), `descriptor-moves` (0.921).

The intermediate macOS directory-stack run took 10.902 ms (1.687× Bash) before caching the logical directory. The first implementation of the extended-pattern workload took 1.669 ms (1.008× Bash). An additional 41-run comparison of the compiled matcher measured 1.157 ms against 1.853 ms for Bash. The tables use a fresh 25-run series on each platform. Each external-pipeline workload includes the child programs, their startup and the final wait.

macOS のディレクトリスタックは論理パスをキャッシュする前は Bash の1.687倍、拡張パターンの反復は構文と照合結果を再利用する前は1.008倍かかっていました。上の表には修正後の同じ実行ファイルで計測した結果を掲載しています。全1,552項目の速度表は11月22日のバイナリの記録です。

```sh
python3 tools/bench_scripts.py "$tonari" --bash "$bash" --timing started --runs 25 \
  --case echo-substitutions --case compound-substitutions --case pipeline --case merged-pipeline \
  --case external-substitutions --case extended-patterns --case resource-limits --case brace-range \
  --case getopts-clusters --case mapfile-lines --case case-fallthrough --case directory-stack \
  --case descriptor-moves --case integer-assignments
python3 tools/bench.py "$tonari" --bash "$bash" --runs 51 --case startup --case prompt
```
