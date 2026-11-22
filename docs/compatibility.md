# Bash compatibility audit / Bash 互換性の検証

Tonari remains a prototype for use from an existing shell. The inventory starts with the reference Bash installations and the GNU manual. You can look up a feature, see the exact observation that differed, and find its timing when that observation matched. [Current feature inventory](compatibility/current.tsv) · [Previous timing matrix](compatibility/matrix.md) · [Debian evidence](compatibility/Debian.tsv) · [Ubuntu evidence](compatibility/Ubuntu.tsv) · [macOS evidence](compatibility/macOS.tsv)

Tonari は現在、既存のシェルから起動して試せる段階です。参照先の Bash と GNU マニュアルを起点に項目を列挙しました。一覧には機能名ごとの判定と速度を載せ、環境別 TSV には実際のテスト内容と差が出た出力例を残しています。[現在の機能別一覧](compatibility/current.tsv) · [速度の集計](benchmarks.md#feature-audit)

## Direct checks on 2026-11-23 / 直接比較の更新

The Ubuntu 26.04 control exposes 61 builtins. The primary behavior witnesses now match for 51. The remaining 10 have no builtin entry, including `bind`, `history` and `readonly`. The three previously differing builtin witnesses now match (`jobs`, `declare`, `compopt`). The 59 shopt names still include 48 missing interfaces. Associative arrays and the `ERR`, `DEBUG` and `RETURN` traps have failing witnesses. The changes file compares each row with the previous snapshot and gives its witness. The current inventory lists unchanged rows as well. [Changed observations by platform](compatibility/changes.tsv) · [Timing after the fixes](benchmarks.md#2026-11-23-runtime)

Ubuntu の組み込み61個のうち51個は代表ケースで標準出力・標準エラー・終了値が Bash と一致しました。未実装は10個です。新たに通ったケースには、整数変数への加算、波括弧の列挙と連番、先に終わった子の取得、行の配列への一括読み込み、ディレクトリスタックの回転、ファイル記述子の移動、リソース上限の変更、拡張パターン照合があります。各組み込みの全オプションや組み合わせには、引き続き個別の確認が必要です。従来の全件の計測値は変更前の記録です。[更新された検証例](compatibility/changes.tsv) · [修正後の速度](benchmarks.md#2026-11-23-runtime)

Against the November 22 snapshot, 126 direct checks on Debian, 125 on Ubuntu and 120 on macOS newly matched. Every previously matching direct observation remained a match. A feature and its option can refer to the same workload. The combined inventory has 1,552 distinct names. [Current inventory](compatibility/current.tsv) · [Changed witnesses with outputs](compatibility/changes.tsv)

## GNU regression results on 2026-11-23 / 修正後の GNU 回帰試験

| Platform | Groups | Golden match | Different | Bash control inconclusive | Tonari timeout | Separate exported-function probe |
|---|---:|---:|---:|---:|---:|---:|
| Debian 13 | 83 | 15 | 64 | 3 | 0 | 1 |
| Ubuntu 26.04 | 86 | 13 | 65 | 7 | 0 | 1 |

Both shells now finish the job-control driver. Tonari took 72.1 s on Debian and 74.2 s on Ubuntu, including the driver's explicit waits. The two Bash controls took 62.2 s and 64.3 s respectively. The named job group still differs from its golden output. All three GNU extended-glob groups pass on Debian. On Ubuntu, the remaining output difference in `run-extglob` is the column width in `shopt extglob`. Bash 5.2 uses a 15-character name field and Bash 5.3 uses 20. Tonari currently follows 5.2. [GNU driver results and first differences](compatibility/gnu.tsv)

前回は Ubuntu のジョブ制御が110秒で打ち切られました。今回のバイナリは両環境で最後まで実行し、残った出力差も記録しています。拡張パターンの GNU 3グループは Debian ですべて一致し、Ubuntu では2グループが一致しました。残り1グループには `shopt` の表示幅の差があります。表の判定不能は、参照先の Bash 自身で正解出力を確認できなかったグループです。意図的な待機を含む動作確認の所要時間は速度目標の達成件数には含めていません。

## How to read the evidence / 判定の読み方

- Debian 13 arm64 uses Bash 5.2.37. Ubuntu 26.04 arm64 uses Bash 5.3.9 with the GNU 5.3 release test files. macOS arm64 uses Bash 5.2.37 for additional native comparisons. The two Linux runs include the upstream regression groups. The macOS run covers the direct feature and terminal probes.
- Builtins, their short-option usage, reserved words, shell options, signal names, Readline commands and Readline variables come from the reference executable. Manual sections, documented variables, compatibility levels and numbered POSIX-mode rules come from the matching GNU sources. The other rows name concrete language operations, invocation forms, prompt escapes, and terminal workflows. [GNU Bash manual](https://www.gnu.org/software/bash/manual/bash.html) · [GNU Bash releases](https://ftp.gnu.org/gnu/bash/)
- A direct script match requires equal stdout bytes, stderr bytes and exit status after replacing the test directory and executable path with fixed labels. PTY tests check the displayed prompt, captured input/completion fields, terminal process group or returned status. Date and time prompts use the documented display format. Network redirection probes check received bytes over loopback.
- Each GNU driver runs once against the distribution shell and once against Tonari using the same executable pathname, fixtures, locale and controlling terminal. A golden match requires a successful driver, a nonempty sequence of successful direct golden-file comparisons, and no reported skips on either side. A failing, skipped or unobservable control remains inconclusive. The ordinary exported-function behavior has its own direct probe. The aggregate, profiling and exported-function drivers are outside the golden totals.
- A matching observation applies to the named case. A working `trap EXIT` can coexist with a failing `trap ERR`. An absent `bind` marks every Readline binding interface as missing. A rejected `--posix` leaves the manual’s numbered POSIX rules blocked at entry. A counted name can share a witness with other names. The 132 manual sections form an index. These dependencies make a single percentage a poor measure of how much of Bash works. Unicode tests cover the project's UTF-8 contract. Combinations of features, external plugins, other CPUs and WSL2 require their own evidence.
- 直接の比較では標準出力、標準エラー、終了値を照合します。対話機能は疑似端末から実際にキーを送り、補完コールバックの値や前景プロセスグループも調べます。GNU の回帰テストは配布版 Bash 自身でも実行します。正解ファイルと照合できなかったグループには「判定不能」と明記しました。
- 組み込みやオプションの一致はその検証例が通ったという意味です。たとえば `trap EXIT` が通っても `trap ERR` には動作差があります。Readline の多数の行が一つの `bind` の欠落に結び付きます。件数には別名や同じ操作を参照する項目も含まれ、全組み合わせを網羅するものではありません。

## What the timing measures / 速度の測り方

Each candidate measurement checks its result again. Changed observations have a visible error and no timing credit. Each shell uses a newly prepared directory and clean environment. In the default run the harness alternates shell order, discards two warmups and uses 15 samples per executable, or seven for GNU drivers. Median and nearest-rank p95 are stored in nanoseconds. Both binaries are staged on the same filesystem. [Feature benchmark](../tests/compat/bench.py)

- **Whole** measures launching the shell, running the case, consuming its output and waiting for termination. PTY and GNU rows time a complete checked dialogue or upstream driver, including its helpers. Fixture creation and copying the GNU tree happen outside the driver timer.
- **Started** measures canonical `-c` cases from a readiness handshake to exit. This includes waking the shell, running the case, reading its output and reaping it. Each sample must still produce the original bytes and status. A numeric ratio of 0.50 means Tonari took half the Bash time. The upper target uses the exact fraction 2/3 before rounding.
- Short one-shot measurements include fixed launcher and scheduler costs. Probes containing `sleep`, a PTY exchange, file creation or an external utility include that wall time. For per-operation throughput, the script suite repeats work in one shell and verifies the result. The editor suite measures keys to visible PTY output and checks the completed line or displayed candidates. [Script workloads](../tools/bench_scripts.py) · [Keyboard workloads](../tools/bench_completion.py)
- 所要時間は Tonari÷Bash です。毎回結果を照合し、起動込みと起動済みを分けます。後者にも同期・出力回収・終了待ちの時間が入ります。入力1文字の遅延は専用の端末ベンチ、展開やループは反復スクリプトでも確認します。外部プログラムや意図的な待ち時間を含む項目も実測値を残します。未実装・動作不一致の項目は互換動作の速度を算出できるまで一覧にその状態を表示します。

## Reproduce on Linux / Linux での再現

Build a Linux Tonari release for the machine running Docker. The image below supplies the distribution's Bash, locales, GNU files and its four test helpers. Set `linux_binary` to the absolute path of the Linux executable. Build each fixture once and run the two distributions sequentially. The benchmark record identifies the exact release binaries used for the published measurements. [Release build instructions](benchmarks.md)

```sh
linux_binary=/absolute/path/to/linux/tonari
run_lane() {
    label=$1 base=$2 version=$3 digest=$4
    fixture=$(mktemp -d)
    curl -fL "https://ftp.gnu.org/gnu/bash/bash-$version.tar.gz" -o "$fixture/bash-src.tar.gz" &&
    docker build -f tests/compat/Dockerfile --build-arg BASE="$base" \
        --build-arg GNU_SHA256="$digest" -t "tonari-audit-$label" "$fixture" &&
    docker run --rm --network none --pids-limit 128 --tmpfs /tmp:exec,mode=1777,size=384m \
        -v "$PWD:/repo:ro" -v "$linux_binary:/tonari:ro" "tonari-audit-$label" --correctness-only > "$label.audit.json"
}
run_lane debian debian:trixie-slim 5.2.37 9599b22ecd1d5787ad7d3b7bf0c59f312b3396d1e281175dd1f8a4014da621ff
run_lane ubuntu ubuntu:26.04 5.3 0d5cd86965f869a26cf64f4b71be7b96f90a3ba8b3d74e27e8e9d9d5550f31ba
```

On macOS use a native modern Bash and the unpacked GNU source tree matching its feature release.

```sh
python3 tests/compat/audit.py --bash "$reference_bash" --tonari target/release/tonari \
    --source "$gnu_source" > audit-mac.json
python3 tests/compat/bench.py --bash "$reference_bash" --tonari target/release/tonari \
    --audit audit-mac.json > mac.json
```

To time a verified build, mount its saved JSON into the same fixture and pass `--reuse-audit`. The benchmark compares the SHA-256 of both staged executables with the audit and checks each sample against its recorded result.

修正中は上の `--correctness-only` で機能を再確認します。速度を計測する段階で同じ実行ファイルと結果ファイルを指定します。計測器がどちらかの実行ファイルの変更を検出すると、速度の集計を始める前に終了します。

```sh
label=debian
docker run --rm --network none --pids-limit 128 --tmpfs /tmp:exec,mode=1777,size=384m \
    -v "$PWD:/repo:ro" -v "$linux_binary:/tonari:ro" -v "$PWD/$label.audit.json:/checked.json:ro" \
    "tonari-audit-$label" --reuse-audit /checked.json > "$label.json"
```

## Check a release / リリース時の検証

```sh
python3 tests/compat/changes.py --baseline docs/compatibility --inventory current.tsv --gnu gnu.tsv \
    Debian=debian.audit.json Ubuntu=ubuntu.audit.json macOS=audit-mac.json > changes.tsv
python3 tests/compat/report.py --compare docs/compatibility \
    Debian=debian.json Ubuntu=ubuntu.json macOS=mac.json
python3 tests/compat/report.py --require-target --require-complete \
    Debian=debian.json Ubuntu=ubuntu.json macOS=mac.json
python3 tests/compat/report.py --output new-report \
    Debian=debian.json Ubuntu=ubuntu.json macOS=mac.json
python3 tools/bench_scripts.py target/release/tonari --bash "$reference_bash" --require-target
python3 tools/bench.py target/release/tonari --bash "$reference_bash" --require-target
python3 tools/bench_completion.py target/release/tonari --bash "$reference_bash" --listing --require-target
```

The comparison exits nonzero if a name disappears, a matching observation or timing is lost, or a previously qualifying median crosses 2/3. The absolute target checks every matching feature's measured median. The completeness check also fails on missing or unverified behavior. Review disputed timings with more runs on the same hardware before updating a snapshot. The `--output` operation writes a new matrix and platform TSVs.

比較ゲートは、一覧から消えた項目、以前通った動作の不一致、計測の消失、以前達成していた上限の超過を終了コードで通知します。`--require-target` は既知の未達も含めて判定します。`--require-complete` は未確認・未実装・動作差をすべて検出します。更新時には結果を見比べ、境界付近では同じ機械で回数を増やして確認します。
