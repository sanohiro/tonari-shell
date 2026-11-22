# Bash compatibility audit / Bash 互換性の検証

Tonari remains a prototype for use from an existing shell. The inventory starts with the reference Bash installations and the GNU manual. You can look up a feature, see the exact observation that differed, and find its timing when that observation matched. [Feature matrix](compatibility/matrix.md) · [Debian evidence](compatibility/Debian.tsv) · [Ubuntu evidence](compatibility/Ubuntu.tsv) · [macOS evidence](compatibility/macOS.tsv)

Tonari は現在、既存のシェルから起動して試せる段階です。参照先の Bash と GNU マニュアルを起点に項目を列挙しました。一覧には機能名ごとの判定と速度を載せ、環境別 TSV には実際のテスト内容と差が出た出力例を残しています。[機能別一覧](compatibility/matrix.md) · [速度の集計](benchmarks.md#feature-audit)

## Current gaps / 確認できた差分

On Ubuntu, Bash exposes 61 builtins. Tonari matches the primary witness for 38. Three produce different results (`compopt`, `declare`, `jobs`). The other 20 have no same-name builtin, including `bind`, `getopts`, `mapfile`, `readonly` and `umask`. Of the 59 `shopt` names, six match their behavior probe, two pass an interface-only check and 51 lack the interface. Brace alternation and ranges, extended globs, associative arrays, the `ERR`, `DEBUG` and `RETURN` traps and `wait -n/-p` all have concrete counterexamples. [Ubuntu witnesses and observed output](compatibility/Ubuntu.tsv)

Ubuntu の組み込み61個を引くと、代表ケースが一致したのは38個、動作差が3個、同名の組み込みがないものが20個でした。単純な関数定義・戻り値、EXIT・USR1 のトラップ、前景と背景の切り替えは実際に通っています。一方で関数のエクスポート、拡張グロブ、連想配列、特殊トラップ、複数の子のうち先に終わったものを待つ操作などに差があります。[再現例を含む一覧](compatibility/matrix.md)

The GNU test groups on Debian produced 10 golden matches, 69 differences and three inconclusive Bash controls. On Ubuntu there were 10 golden matches, 67 differences, seven inconclusive controls and one Tonari timeout. The exported-function group is recorded separately in both totals of 83 and 86 respectively. For the Ubuntu job-control group, Bash completed the control while Tonari exceeded the 110-second watchdog. The TSV includes the first golden-file difference or skip for each affected control.

GNU のグループ回帰試験では Debian・Ubuntu ともに10グループが正解ファイルと一致しました。違いが確認できたのは Debian で69、Ubuntu で67グループです。配布版 Bash 自身で正解ファイルを再現できなかったグループも残り、Ubuntu のジョブ制御では Tonari が110秒でタイムアウトしました。関数エクスポートは両環境とも専用の直接テストで判定しています。

## How to read the evidence / 判定の読み方

- Debian 13 arm64 uses Bash 5.2.37. Ubuntu 26.04 arm64 uses Bash 5.3.9 with the GNU 5.3 release test files. macOS arm64 uses Bash 5.2.37 for additional native comparisons. The two Linux runs include the upstream regression groups. The macOS run covers the direct feature and terminal probes.
- Builtins, their short-option usage, reserved words, shell options, signal names, Readline commands and Readline variables come from the reference executable. Manual sections, documented variables, compatibility levels and numbered POSIX-mode rules come from the matching GNU sources. The other rows name concrete language operations, invocation forms, prompt escapes, and terminal workflows. [GNU Bash manual](https://www.gnu.org/software/bash/manual/bash.html) · [GNU Bash releases](https://ftp.gnu.org/gnu/bash/)
- A direct script match requires equal stdout bytes, stderr bytes and exit status after replacing the test directory and executable path with fixed labels. PTY tests check the displayed prompt, captured input/completion fields, terminal process group or returned status. Date and time prompts use the documented display format. Network redirection probes check received bytes over loopback.
- Each GNU driver runs once against the distribution shell and once against Tonari using the same executable pathname, fixtures, locale and controlling terminal. A golden match requires a successful driver, a nonempty sequence of successful direct golden-file comparisons, and no reported skips on either side. A failing, skipped or unobservable control remains inconclusive. The ordinary exported-function behavior has its own direct probe. The aggregate, profiling and exported-function drivers are outside the golden totals.
- A matching observation applies to the named case. A working `trap EXIT` can coexist with a failing `trap ERR`. An absent `bind` marks every Readline binding interface as missing. A rejected `--posix` leaves the manual’s numbered POSIX rules blocked at entry. A counted name can share a witness with other names. The 132 manual sections form an index. These dependencies make a single percentage a poor measure of how much of Bash works. Unicode tests cover the project's UTF-8 contract. Combinations of features, external plugins, other CPUs and WSL2 require their own evidence.
- 直接の比較では標準出力、標準エラー、終了値を照合します。対話機能は疑似端末から実際にキーを送り、補完コールバックの値や前景プロセスグループも調べます。GNU の回帰テストは配布版 Bash 自身でも実行します。正解ファイルと照合できなかったグループには「判定不能」と明記しました。
- 組み込みやオプションの一致はその検証例が通ったという意味です。たとえば `fg` で前景に戻せても、`jobs -p` をコマンド置換の中から呼ぶと結果が異なります。Readline の多数の行が一つの `bind` の欠落に結び付きます。件数には別名や同じ操作を参照する項目も含まれ、全組み合わせを網羅するものではありません。

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
        -v "$PWD:/repo:ro" -v "$linux_binary:/tonari:ro" "tonari-audit-$label" > "$label.json"
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

## Check a release / リリース時の検証

```sh
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
