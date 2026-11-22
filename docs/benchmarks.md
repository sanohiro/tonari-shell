# Startup and execution / 起動と実行性能

First prototype, measured 2026-11-22. Each timing is **median / p95 in milliseconds**. The prompt test ends when the first prompt is visible on a pseudo-terminal. The other tests start a fresh shell for each command and wait for its exit.

初回試作を 2026-11-22 に計測しました。時間の単位はミリ秒で、各欄に **中央値 / p95** を示します。プロンプトの計測は、疑似端末に最初のプロンプトが出力されるまでです。ほかの計測では毎回新しいシェルを起動し、その終了までを測っています。

## Debian 13 arm64

Linux VM on Apple M1, Debian userspace, GNU Bash 5.2.37, brush 0.4.0. The executables ran on the VM's local filesystem.

Apple M1 上の Linux 仮想マシンで Debian のユーザーランドを使い、VM 内のファイルシステムに置いた実行ファイルで計測しました。

| Metric / 計測項目 | Tonari | GNU Bash | brush |
|---|---:|---:|---:|
| First prompt / プロンプト表示 | 0.895 / 1.381 | 1.148 / 1.444 | 1.761 / 2.362 |
| `-c ':'` | 0.335 / 0.416 | 0.451 / 0.559 | 1.149 / 1.398 |
| `-c '/usr/bin/true'` | 0.457 / 0.587 | 0.602 / 0.750 | 1.664 / 2.077 |
| `-c '/usr/bin/printf x \| /usr/bin/wc -c'` | 0.722 / 0.885 | 0.902 / 1.339 | 2.009 / 3.101 |
| Executable bytes / 実行ファイルのバイト数 | 397,968 | 1,412,096 | 6,381,288 |

## macOS 27.2.1 arm64

Apple M1, system Bash 3.2.57, brush 0.4.0. The macOS system Bash is older than the project's compatibility baseline. Its universal executable contains three architecture slices. The arm64e slice is 667,584 bytes. Tonari and brush are single-architecture arm64 builds.

Apple M1 で OS 同梱の Bash 3.2.57 と brush 0.4.0 を使用しました。macOS 同梱の Bash は互換性の基準にしている版より古く、実行ファイルには三つのアーキテクチャが含まれます。うち arm64e 部分は 667,584 バイトです。Tonari と brush は arm64 単体のビルドです。

| Metric / 計測項目 | Tonari | GNU Bash | brush |
|---|---:|---:|---:|
| First prompt / プロンプト表示 | 4.905 / 7.973 | 5.810 / 9.159 | 9.129 / 12.498 |
| `-c ':'` | 3.856 / 4.467 | 4.597 / 5.327 | 7.536 / 8.894 |
| `-c '/usr/bin/true'` | 5.183 / 6.447 | 5.975 / 6.877 | 9.960 / 11.260 |
| `-c '/usr/bin/printf x \| /usr/bin/wc -c'` | 5.609 / 6.594 | 7.144 / 8.168 | 10.915 / 13.067 |
| Executable bytes / 実行ファイルのバイト数 | 370,736 | 1,961,872 | 5,621,680 |

## Initial measurement conditions / 初回の計測条件

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
python3 tools/bench.py target/release/tonari --bash /bin/bash
```
