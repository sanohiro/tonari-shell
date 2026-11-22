# Bash compatibility audit / Bash 互換性の検証

Tonari remains a prototype for use from an existing shell. The inventory starts with the reference Bash installations and the GNU manual. The matrix gives the observed behavior and the elapsed time for each matching workload. [Current feature inventory](compatibility/current.tsv) · [Current timing matrix](compatibility-2026-11-23-login/matrix.md) · [Debian evidence](compatibility-2026-11-23-login/Debian.tsv) · [Ubuntu evidence](compatibility-2026-11-23-login/Ubuntu.tsv) · [macOS evidence](compatibility-2026-11-23-login/macOS.tsv)

Tonari は現在、既存のシェルから起動して試せる段階です。参照先の Bash と GNU マニュアルを起点に項目を列挙しました。一覧には機能名ごとの判定と速度を載せ、環境別 TSV には実際のテスト内容と差が出た出力例を残しています。[現在の機能別一覧](compatibility/current.tsv) · [各項目の最新計測](compatibility-2026-11-23-login/matrix.md)

## Login sessions on 2026-11-23 / ログインシェルの検証

The primary witnesses for `logout`, `history` and `fc` now match on Debian 13, Ubuntu 26.04 and native macOS arm64. Of Ubuntu's 61 builtins, 56 match their primary check. The remaining five absent interfaces are `bind`, `caller`, `enable`, `help` and `suspend`. Direct comparisons cover the distro login path, local variable restoration during completion, arithmetic array subscripts, and shell nesting. The history journal stays under the XDG state directory. The `history` and `fc` checks cover selected file operations, listing and replay. Interactive history event expansion and all Readline bindings still lack full compatibility. [Integration tests](../tests/shell.py) · [Login timings](benchmarks.md#login-profiles)

ログイン時の `/etc/profile` とユーザープロファイルの順序、通常の `-c` での `BASH_ENV`、Ctrl-D による終了処理を検証しました。Debian と Ubuntu は各103件の統合テストを通過し、macOS はディストリビューション固有の1件を除く102件を通過しました。macOS での初回の並列実行では、参照 Bash の Ctrl-C トラップと、前景パイプラインの再開後に Ctrl-C を送る二つのテストがタイムアウトしました。全体を再実行して通過した結果を記録しています。`history` と `fc` の代表ケースは一致し、`bind` など五つの組み込み、`DEBUG` トラップや関数のエクスポートには差が残ります。[互換性一覧](compatibility/current.tsv)

The current direct audit adds 43 matching names on each platform against `f3f5539`. Every previously matching direct check still passes. GNU golden matches remain 16 of 83 groups on Debian and 14 of 86 on Ubuntu. Both `run-jobs` drivers finished. [Current inventory](compatibility/current.tsv) · [GNU observations](compatibility/gnu.tsv)

直前の版と比べて、各環境で43項目の直接比較が新たに一致しました。GNU の回帰試験は両環境とも Tonari のタイムアウトが0件でした。GNU のグループごとの出力差や判定不能の件数は下の表に記載しています。

## Earlier checks on 2026-11-23 / 同日のログイン対応前の結果

The primary checks for 53 of Ubuntu's 61 Bash builtins match on stdout, stderr and exit status. The eight missing builtins are `bind`, `caller`, `enable`, `fc`, `help`, `history`, `logout` and `suspend`. Of the 59 Ubuntu `shopt` names, 11 match their behavior check, three accept the interface and 45 are missing. The `DEBUG` trap, exported shell functions and loading the full Debian bash-completion setup remain open compatibility issues. [Current feature inventory](compatibility/current.tsv)

In each of the three environments, 36 direct checks that failed or lacked an interface at commit `835304b` now match. All previously matching direct checks still pass. Changes include associative keys, read-only bindings, parameter case conversions, `ERR` and `RETURN` traps, and background job ownership. The integration suite exercises 94 cases on each platform, including a hangup on a controlling terminal. Against the November 22 snapshot, the cumulative gains are 162 direct checks on Debian, 161 on Ubuntu and 156 on macOS. The inventory covers 1,554 distinct names, including two explicit UTF-8 contracts. Some names exercise the same behavior. [Changed observations](compatibility/changes.tsv) · [Release measurements](benchmarks.md#2026-11-23-compatibility)

Ubuntu の Bash の組み込み61個のうち、53個の代表ケースで出力と終了値が一致しました。`history`、`fc`、`bind` など8個が未実装です。直前の版で差があった直接比較のうち、各環境で36項目が新たに一致し、以前通ったものはすべて通っています。94件の統合テストでは、空白や絵文字を含む連想配列のキー、読み取り専用変数への書き込み失敗後の再開位置、端末終了時に `disown -h` で保護されたジョブの生存も確認しました。`DEBUG`、関数のエクスポート、Debian の bash-completion 一式の読み込みには引き続き差があります。[機能別結果](compatibility/current.tsv) · [修正後の速度](benchmarks.md#2026-11-23-compatibility)

## GNU regression results on 2026-11-23 / 修正後の GNU 回帰試験

| Platform | Groups | Golden match | Different | Bash control inconclusive | Tonari timeout | Separate exported-function probe |
|---|---:|---:|---:|---:|---:|---:|
| Debian 13 | 83 | 16 | 63 | 3 | 0 | 1 |
| Ubuntu 26.04 | 86 | 14 | 64 | 7 | 0 | 1 |

Both distributions gained `run-nquote1`, which exercises quoted and unquoted control characters. All three extended-glob drivers match on Debian. Two match on Ubuntu, where `run-extglob` reports a difference in the width of `shopt` output. A separate path filter also distinguishes the references. With `extglob` enabled, `GLOBIGNORE='+([^[:alnum:]])'` excludes `./++` from `./*` on Bash 5.2 and retains it on Bash 5.3. Tonari follows 5.2 in this case. The job-control driver finishes on both distributions and still reports output differences. [GNU driver results and first differences](compatibility/gnu.tsv)

引用の有無と制御文字の扱いを調べる `run-nquote1` が両環境で新たに一致しました。ジョブ制御のグループは両環境で最後まで走りましたが、出力差が残っています。個別テストで確認した `GLOBIGNORE` の挙動は Debian と Ubuntu の Bash でも異なり、句読点だけのファイル `./++` を対象とする例では Debian の Bash 5.2 に合わせました。表の「判定不能」は配布版 Bash 自身の結果を正解ファイルと照合できなかったグループです。[GNU の実測結果](compatibility/gnu.tsv)

<a id="intentional-differences"></a>

## Intentional differences / Tonari が採用する動作

Bash defines the expected syntax and tool-facing behavior for supported scripts. Tonari defines its UTF-8 text contract and can support inputs beyond Bash's capabilities. Record a concrete input, the reference outputs and the reason for a deliberate difference. Test extensions against Tonari's promised result. The matrix marks a verified intentional difference with ◎ and a violated Tonari contract with !. A new reference result is flagged for review. The release gate checks the Tonari result and keeps parity and speed counts specific to comparable behavior. [Shell tests](../tests/shell.py)

Bash 互換の基準は、対応するスクリプトとツールが期待する構文・動作です。Tonari の仕様を壊す Bash の不具合は再現せず、Bash が扱えない入力でも Tonari の仕様で正しく扱えるものはサポートします。意図した差には再現例と採用理由を残します。機能の未実装やバージョン間の仕様差は別途記録します。一覧の ◎ は Tonari の期待値を実測して確認した差、! は Tonari 自身の仕様違反です。参照先が未知の結果を返した場合も再確認できるように記録します。

| Input / 入力 | Reference Bash / 参照結果 | Tonari / 採用する動作 |
|---|---|---|
| `IFS=' 木'; v='a 木b'; set -- $v` in UTF-8 | macOS 5.2.37 yields `a`, two empty arguments, then `b`. Debian 5.2.37 and Ubuntu 5.3.9 prefix `b` with bytes `9c a8`. | Two arguments, `a` and `b`. Consume the entire UTF-8 separator. 区切り文字を途中で分断しません。 |
| `v=雪🌸` under `LC_ALL=C` | `${#v}` is 7 bytes. `${v:1:1}` yields byte `9b`. | Length 2 and substring `🌸`. Positions in shell text count Unicode scalar values across these locales. 文字単位の長さと切り出しを維持します。 |
| A `printf` command with raw byte `ff` inside single quotes, supplied through `-c`, a file or stdin in the C locale | The command runs and writes `ff`. | Reject the invalid command text with status 2 and a UTF-8 diagnostic. 不正な入力単位を実行しません。 |
| `cat <input.bin \| cat >output.bin` with all 256 byte values | The output file equals the input. | The output file equals the input. UTF-8 validation applies to shell text. パイプやリダイレクトでバイナリをそのまま転送できます。 |

The C locale byte result is expected Bash behavior. The UTF-8 `IFS` example exhibits different outcomes even among the reference builds. Tonari keeps valid text intact in both cases. [Encoding observations](engineering-notes.md)

`LC_ALL=C` でバイトを数えるのは Bash の仕様です。一方、UTF-8 の `IFS` の例では参照実装同士でも結果が分かれます。どちらも Tonari では文字の境界を守る理由があり、その期待値を回帰テストにしています。[開発知見](engineering-notes.md)

## How to read the evidence / 判定の読み方

- Debian 13 arm64 uses Bash 5.2.37. Ubuntu 26.04 arm64 uses Bash 5.3.9 with the GNU 5.3 release test files. macOS arm64 uses Bash 5.2.37 for additional native comparisons. The two Linux runs include the upstream regression groups. The macOS run covers the direct feature and terminal probes.
- Builtins, their short-option usage, reserved words, shell options, signal names, Readline commands and Readline variables come from the reference executable. Manual sections, documented variables, compatibility levels and numbered POSIX-mode rules come from the matching GNU sources. The other rows name concrete language operations, invocation forms, prompt escapes, and terminal workflows. [GNU Bash manual](https://www.gnu.org/software/bash/manual/bash.html) · [GNU Bash releases](https://ftp.gnu.org/gnu/bash/)
- A direct script match requires equal stdout bytes, stderr bytes and exit status after replacing the test directory and executable path with fixed labels. PTY tests check the displayed prompt, captured input/completion fields, terminal process group or returned status. Date and time prompts use the documented display format. Network redirection probes check received bytes over loopback.
- Each GNU driver runs once against the distribution shell and once against Tonari using the same executable pathname, fixtures, locale and controlling terminal. A golden match requires a successful driver, a nonempty sequence of successful direct golden-file comparisons, and no reported skips on either side. A failing, skipped or unobservable control remains inconclusive. The ordinary exported-function behavior has its own direct probe. The aggregate, profiling and exported-function drivers are outside the golden totals.
- A matching observation applies to the named case. A working `trap EXIT` can coexist with a failing `trap DEBUG`. An absent `bind` marks every Readline binding interface as missing. A rejected `--posix` leaves the manual’s numbered POSIX rules blocked at entry. A counted name can share a witness with other names. The 132 manual sections form an index. These dependencies make a single percentage a poor measure of how much of Bash works. Unicode tests cover the project's UTF-8 contract. Combinations of features, external plugins, other CPUs and WSL2 require their own evidence.
- 直接の比較では標準出力、標準エラー、終了値を照合します。対話機能は疑似端末から実際にキーを送り、補完コールバックの値や前景プロセスグループも調べます。GNU の回帰テストは配布版 Bash 自身でも実行します。正解ファイルと照合できなかったグループには「判定不能」と明記しました。
- 組み込みやオプションの一致はその検証例が通ったという意味です。たとえば `trap EXIT` が通っても `trap DEBUG` には動作差があります。Readline の多数の行が一つの `bind` の欠落に結び付きます。件数には別名や同じ操作を参照する項目も含まれ、全組み合わせを網羅するものではありません。

## What the timing measures / 速度の測り方

Each candidate measurement checks its result again. Changed observations have a visible error and no timing credit. Each shell uses a newly prepared directory and clean environment. In the default run the harness alternates shell order, discards two warmups and uses 15 samples per executable, or seven for GNU drivers. Median and nearest-rank p95 are stored in nanoseconds. Both binaries are staged on the same filesystem. [Feature benchmark](../tests/compat/bench.py)

- **Whole** measures launching the shell, running the case, consuming its output and waiting for termination. PTY and GNU rows time a complete checked dialogue or upstream driver, including its helpers. Fixture creation and copying the GNU tree happen outside the driver timer.
- **Started** measures canonical `-c` cases from a readiness handshake to exit. This includes waking the shell, running the case, reading its output and reaping it. Each sample must still produce the original bytes and status. A numeric ratio of 0.50 means Tonari took half the Bash time. The upper target uses the exact fraction 2/3 before rounding.
- Short one-shot measurements include fixed launcher and scheduler costs. Probes containing `sleep`, a PTY exchange, file creation or an external utility include that wall time. For per-operation throughput, the script suite repeats work in one shell and verifies the result. The editor suite measures keys to visible PTY output and checks the completed line or displayed candidates. [Script workloads](../tools/bench_scripts.py) · [Keyboard workloads](../tools/bench_completion.py)
- 所要時間は Tonari÷Bash です。毎回結果を照合し、起動込みと起動済みを分けます。後者にも同期・出力回収・終了待ちの時間が入ります。入力1文字の遅延は専用の端末ベンチ、展開やループは反復スクリプトでも確認します。外部プログラムや意図的な待ち時間を含む項目も実測値を残します。未実装・動作不一致の項目は互換動作の速度を算出できるまで一覧にその状態を表示します。

## Reproduce on Linux / Linux での再現

Build a Linux Tonari release for the machine running Docker. The image below supplies the distribution's Bash, locales, GNU files and its four test helpers. Set `linux_binary` to the absolute path of the Linux executable. Build each fixture once and run the two distributions sequentially. The container init process reaps children from background and disown tests throughout the timing run. Without it the cgroup can exhaust its PID allowance and even the reference shell can fail to fork. The benchmark record identifies the exact release binaries used for the published measurements. [Release build instructions](benchmarks.md)

```sh
linux_binary=/absolute/path/to/linux/tonari
run_lane() {
    label=$1 base=$2 version=$3 digest=$4
    fixture=$(mktemp -d)
    curl -fL "https://ftp.gnu.org/gnu/bash/bash-$version.tar.gz" -o "$fixture/bash-src.tar.gz" &&
    docker build -f tests/compat/Dockerfile --build-arg BASE="$base" \
        --build-arg GNU_SHA256="$digest" -t "tonari-audit-$label" "$fixture" &&
    docker run --rm --init --network none --pids-limit 128 --tmpfs /tmp:exec,mode=1777,size=384m \
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

バックグラウンドジョブを繰り返し実行するため、コンテナは `--init` で起動して親を離れた子プロセスを回収します。修正中は上の `--correctness-only` で機能を再確認します。速度を計測する段階で同じ実行ファイルと結果ファイルを指定します。計測器がどちらかの実行ファイルの変更を検出すると、速度の集計を始める前に終了します。

```sh
label=debian
docker run --rm --init --network none --pids-limit 128 --tmpfs /tmp:exec,mode=1777,size=384m \
    -v "$PWD:/repo:ro" -v "$linux_binary:/tonari:ro" -v "$PWD/$label.audit.json:/checked.json:ro" \
    "tonari-audit-$label" --reuse-audit /checked.json > "$label.json"
```

## Check a release / リリース時の検証

```sh
python3 tests/compat/changes.py --baseline docs/compatibility --inventory current.tsv --gnu gnu.tsv \
    Debian=debian.audit.json Ubuntu=ubuntu.audit.json macOS=audit-mac.json > changes.tsv
python3 tests/compat/report.py --compare docs/compatibility-2026-11-23 \
    Debian=debian.json Ubuntu=ubuntu.json macOS=mac.json
python3 tests/compat/report.py --require-target --require-complete \
    Debian=debian.json Ubuntu=ubuntu.json macOS=mac.json
python3 tests/compat/report.py --output new-report \
    Debian=debian.json Ubuntu=ubuntu.json macOS=mac.json
python3 tools/bench_scripts.py target/release/tonari --bash "$reference_bash" --require-target
python3 tools/bench.py target/release/tonari --bash "$reference_bash" --require-target
python3 tools/bench_completion.py target/release/tonari --bash "$reference_bash" --listing --require-target
```

The comparison exits nonzero if a name disappears, a matching observation or timing is lost, or a previously qualifying median crosses 2/3. The optional `--require-target` check reports the strict 2/3 ratio for every matching feature. For short operations and common OS waits, apply the parity criterion to the reported times. [Performance target](benchmarks.md#performance-target) The completeness check also fails on missing or unverified behavior. Review disputed timings with more runs on the same hardware before updating a snapshot. The `--output` operation writes a new matrix and platform TSVs.

比較ゲートは、一覧から消えた項目、以前通った動作の不一致、計測の消失、以前達成していた上限の超過を終了コードで通知します。`--require-target` は 2/3 を超えた項目をすべて報告します。短い処理や共通の OS 待ちは、実測時間から同等かどうかを判断します。`--require-complete` は未確認・未実装・動作差をすべて検出します。更新時には結果を見比べ、境界付近では同じ機械で回数を増やして確認します。
