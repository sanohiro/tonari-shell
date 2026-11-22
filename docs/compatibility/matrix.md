# Bash feature matrix / Bash 機能別の結果

[Method, scope and reproduction / 検証方法と対象範囲](../compatibility.md)

This table records the pre-fix timing baseline. The current direct checks and corrected-workload timings are available separately. この表は修正前の全件計測です。[Current results / 現在の全件判定](current.tsv) · [Changed observations / 修正差分](changes.tsv) · [Full matrix after fixes / 修正後の全件計測](../compatibility-2026-11-23/matrix.md)

Each cell gives the compatibility result and the median elapsed-time ratio (Tonari/Bash), whole run followed by already-started where applicable. Exact nanoseconds, p95, sample counts and witnesses are in each platform TSV. Bold ratios exceed 2/3 before rounding. An upward arrow marks Bash time or slower. Shared witnesses appear under each public name. Manual sections are index entries.

各セルは互換判定と所要時間の中央値の比（Tonari/Bash）です。左が起動込み、右が起動後です。数値が一つなら起動込みのみです。太字は上限 2/3 を超えた値、↑ は Bash と同じか遅い値です。TSV にナノ秒単位の中央値・p95、サンプル数、検証内容を収録しました。同じ処理を参照する項目があり、マニュアルの節は索引として数えています。

✓ match / 一致 · G GNU golden match · Δ difference / 動作差 · E stderr difference · M missing interface · V missing default · I interface only · P POSIX mode unavailable · F fixture unavailable · ? inconclusive · N absent from reference · T timeout · ↗ separate function-export probe · § manual index · × observation changed during timing · — outside this lane.

## Arithmetic

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>!=</code> | ✓ 0.62 / **1.04↑** | ✓ 0.60 / **1.02↑** | ✓ 0.56 / **0.74** |
| <code>!a</code> | ✓ 0.59 / **0.99** | ✓ 0.60 / **1.02↑** | ✓ 0.58 / **0.78** |
| <code>%</code> | ✓ 0.62 / **1.01↑** | ✓ 0.61 / **1.04↑** | ✓ 0.58 / **0.75** |
| <code>%=</code> | ✓ 0.59 / **0.89** | ✓ 0.60 / **1.01↑** | ✓ 0.55 / **0.73** |
| <code>&amp;</code> | ✓ 0.62 / **1.02↑** | ✓ 0.61 / **1.00** | ✓ 0.58 / **0.74** |
| <code>&amp;&amp;</code> | ✓ 0.62 / **0.99** | ✓ 0.60 / **0.99** | ✓ 0.58 / **0.69** |
| <code>&amp;=</code> | ✓ 0.56 / **0.79** | ✓ 0.59 / **1.02↑** | ✓ 0.55 / **0.82** |
| <code>(a+1)*2</code> | ✓ 0.58 / **1.02↑** | ✓ 0.62 / **0.85** | ✓ 0.53 / **0.76** |
| <code>*</code> | ✓ 0.61 / **0.85** | ✓ 0.61 / **1.00** | ✓ 0.59 / **0.73** |
| <code>**</code> | ✓ 0.63 / **0.98** | ✓ 0.61 / **1.04↑** | ✓ 0.57 / **0.72** |
| <code>*=</code> | ✓ 0.58 / **0.78** | ✓ 0.60 / **0.94** | ✓ 0.55 / **0.69** |
| <code>+</code> | ✓ 0.60 / **1.02↑** | ✓ 0.60 / **1.04↑** | ✓ 0.56 / **0.75** |
| <code>++a</code> | ✓ 0.61 / **1.01↑** | ✓ 0.61 / **0.86** | ✓ 0.56 / **0.76** |
| <code>+=</code> | ✓ 0.60 / **0.87** | ✓ 0.61 / **1.02↑** | ✓ 0.58 / **0.71** |
| <code>+a</code> | ✓ 0.61 / **0.76** | ✓ 0.61 / **0.99** | ✓ 0.55 / **0.70** |
| <code>,</code> | ✓ 0.61 / **0.84** | ✓ 0.61 / **1.05↑** | ✓ 0.59 / **0.70** |
| <code>-</code> | ✓ 0.61 / **0.98** | ✓ 0.62 / **1.09↑** | ✓ 0.58 / **0.75** |
| <code>--a</code> | ✓ 0.58 / **0.82** | ✓ 0.59 / **0.99** | ✓ 0.57 / 0.66 |
| <code>-=</code> | ✓ 0.62 / **0.88** | ✓ 0.60 / **0.87** | ✓ 0.60 / **0.76** |
| <code>-a</code> | ✓ 0.59 / **0.81** | ✓ 0.61 / **1.02↑** | ✓ 0.59 / **0.78** |
| <code>/</code> | ✓ 0.60 / **0.89** | ✓ 0.61 / **1.03↑** | ✓ 0.55 / **0.75** |
| <code>/=</code> | ✓ 0.58 / **0.84** | ✓ 0.59 / **1.05↑** | ✓ 0.57 / **0.68** |
| <code>017</code> | ✓ 0.62 / **1.02↑** | ✓ 0.60 / **1.00↑** | ✓ 0.56 / **0.71** |
| <code>0xff</code> | ✓ 0.62 / **1.01↑** | ✓ 0.60 / **0.87** | ✓ 0.56 / **0.73** |
| <code>16#ff</code> | ✓ 0.62 / **0.98** | ✓ 0.55 / **0.99** | ✓ 0.57 / **0.75** |
| <code>&lt;</code> | ✓ 0.62 / **0.98** | ✓ 0.62 / **1.04↑** | ✓ 0.59 / **0.74** |
| <code>&lt;&lt;</code> | ✓ 0.61 / **1.05↑** | ✓ 0.61 / **1.07↑** | ✓ 0.55 / **0.70** |
| <code>&lt;&lt;=</code> | ✓ 0.58 / **0.85** | ✓ 0.60 / **0.99** | ✓ 0.59 / **0.67** |
| <code>&lt;=</code> | ✓ 0.63 / **1.01↑** | ✓ 0.62 / **1.04↑** | ✓ 0.56 / **0.74** |
| <code>=</code> | ✓ 0.61 / **0.86** | ✓ 0.61 / **0.99** | ✓ 0.56 / **0.71** |
| <code>==</code> | ✓ 0.61 / **1.01↑** | ✓ 0.61 / **0.95** | ✓ 0.58 / **0.67** |
| <code>&gt;</code> | ✓ 0.62 / **1.00↑** | ✓ 0.61 / **1.02↑** | ✓ 0.57 / **0.69** |
| <code>&gt;=</code> | ✓ 0.62 / **1.03↑** | ✓ 0.60 / **1.02↑** | ✓ 0.59 / **0.67** |
| <code>&gt;&gt;</code> | ✓ 0.62 / **1.01↑** | ✓ 0.62 / **1.03↑** | ✓ 0.58 / **0.73** |
| <code>&gt;&gt;=</code> | ✓ 0.56 / **0.83** | ✓ 0.61 / **0.99** | ✓ 0.56 / **0.71** |
| <code>^</code> | ✓ 0.62 / **1.03↑** | ✓ 0.60 / **0.87** | ✓ 0.56 / **0.67** |
| <code>^=</code> | ✓ 0.58 / **0.88** | ✓ 0.61 / **1.01↑** | ✓ 0.58 / **0.73** |
| <code>a++</code> | ✓ 0.61 / **0.97** | ✓ 0.61 / **1.07↑** | ✓ 0.57 / **0.68** |
| <code>a--</code> | ✓ 0.60 / **0.91** | ✓ 0.61 / **1.01↑** | ✓ 0.59 / **0.73** |
| <code>a?3:4</code> | ✓ 0.62 / **1.01↑** | ✓ 0.58 / **1.04↑** | ✓ 0.55 / **0.72** |
| <code>&#124;</code> | ✓ 0.62 / **0.98** | ✓ 0.59 / **0.99** | ✓ 0.57 / **0.76** |
| <code>&#124;=</code> | ✓ 0.61 / **0.86** | ✓ 0.60 / **0.99** | ✓ 0.58 / **0.72** |
| <code>&#124;&#124;</code> | ✓ 0.62 / **1.01↑** | ✓ 0.60 / **1.00↑** | ✓ 0.60 / **0.76** |
| <code>~a</code> | ✓ 0.60 / **0.99** | ✓ 0.61 / **0.83** | ✓ 0.57 / **0.68** |

## Builtin

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>.</code> | ✓ 0.64 / **0.83** | ✓ 0.61 / **0.80** | ✓ 0.57 / **0.71** |
| <code>:</code> | ✓ 0.60 / **0.83** | ✓ 0.61 / **0.81** | ✓ 0.56 / **0.76** |
| <code>[</code> | ✓ 0.61 / **0.85** | ✓ 0.60 / **0.83** | ✓ 0.57 / **0.77** |
| <code>alias</code> | ✓ 0.60 / **0.94** | ✓ 0.60 / **0.96** | ✓ 0.55 / **0.73** |
| <code>bg</code> | ✓ **1.02↑** | ✓ **0.93** | ✓ **0.87** |
| <code>bind</code> | M | M | M |
| <code>break</code> | ✓ 0.61 / **1.02↑** | ✓ 0.63 / **0.97** | ✓ 0.58 / **0.68** |
| <code>builtin</code> | ✓ 0.62 / **0.99** | ✓ 0.62 / **0.96** | ✓ 0.56 / **0.73** |
| <code>caller</code> | M | M | M |
| <code>cd</code> | ✓ **0.69** / **0.82** | ✓ **0.83** / **0.92** | ✓ 0.58 / 0.65 |
| <code>command</code> | ✓ 0.59 / **1.05↑** | ✓ 0.59 / **1.03↑** | ✓ 0.56 / **0.77** |
| <code>compgen</code> | ✓ 0.61 / **1.07↑** | ✓ 0.59 / **1.03↑** | ✓ 0.54 / **0.72** |
| <code>complete</code> | ✓ 0.62 / **1.04↑** | ✓ 0.62 / **1.03↑** | ✓ 0.57 / **0.77** |
| <code>compopt</code> | Δ | Δ | Δ |
| <code>continue</code> | ✓ 0.59 / **0.83** | ✓ 0.59 / **0.79** | ✓ 0.56 / **0.73** |
| <code>declare</code> | Δ | Δ | Δ |
| <code>dirs</code> | M | M | M |
| <code>disown</code> | M | M | M |
| <code>echo</code> | ✓ 0.61 / **0.76** | ✓ 0.58 / **0.92** | ✓ 0.56 / 0.66 |
| <code>enable</code> | M | M | M |
| <code>eval</code> | ✓ 0.58 / **0.86** | ✓ 0.60 / **0.90** | ✓ 0.57 / **0.70** |
| <code>exec</code> | ✓ **0.69** / **0.92** | ✓ **0.68** / **0.91** | ✓ **0.69** / **0.88** |
| <code>exit</code> | ✓ 0.60 / **0.89** | ✓ 0.60 / **0.88** | ✓ 0.57 / **0.74** |
| <code>export</code> | ✓ **0.69** / **0.91** | ✓ **0.69** / **0.93** | ✓ **0.72** / **0.91** |
| <code>false</code> | ✓ 0.60 / **0.89** | ✓ 0.60 / **0.88** | ✓ 0.54 / **0.74** |
| <code>fc</code> | M | M | M |
| <code>fg</code> | ✓ **1.02↑** | ✓ **0.93** | ✓ **0.87** |
| <code>getopts</code> | M | M | M |
| <code>hash</code> | ✓ **0.68** / **0.88** | ✓ **0.86** / **0.97** | ✓ **0.69** / **0.83** |
| <code>help</code> | M | M | M |
| <code>history</code> | M | M | M |
| <code>jobs</code> | Δ | Δ | Δ |
| <code>kill</code> | ✓ 0.64 / **0.76** | ✓ 0.64 / **0.81** | ✓ 0.57 / **0.69** |
| <code>let</code> | M | M | M |
| <code>local</code> | ✓ 0.61 / **0.78** | ✓ 0.61 / **0.81** | ✓ 0.54 / **0.68** |
| <code>logout</code> | M | M | M |
| <code>mapfile</code> | M | M | M |
| <code>popd</code> | M | M | M |
| <code>printf</code> | ✓ 0.58 / **0.90** | ✓ 0.60 / **0.85** | ✓ 0.56 / 0.67 |
| <code>pushd</code> | M | M | M |
| <code>pwd</code> | ✓ **0.72** / **0.80** | ✓ **0.88** / **0.92** | ✓ 0.58 / 0.67 |
| <code>read</code> | ✓ 0.63 / **0.81** | ✓ 0.61 / **0.82** | ✓ 0.57 / **0.73** |
| <code>readarray</code> | M | M | M |
| <code>readonly</code> | M | M | M |
| <code>return</code> | ✓ 0.61 / **0.91** | ✓ 0.62 / **0.94** | ✓ 0.58 / **0.69** |
| <code>set</code> | ✓ 0.59 / 0.62 | ✓ 0.59 / 0.62 | ✓ 0.61 / **0.81** |
| <code>shift</code> | ✓ 0.62 / **0.91** | ✓ 0.61 / **0.96** | ✓ 0.57 / **0.69** |
| <code>shopt</code> | ✓ 0.59 / **0.75** | ✓ 0.60 / **0.87** | ✓ 0.57 / **0.78** |
| <code>source</code> | ✓ 0.64 / **0.80** | ✓ 0.62 / **0.78** | ✓ 0.57 / **0.74** |
| <code>suspend</code> | M | M | M |
| <code>test</code> | ✓ 0.58 / **0.82** | ✓ 0.58 / **0.82** | ✓ 0.56 / **0.70** |
| <code>times</code> | M | M | M |
| <code>trap</code> | ✓ 0.58 / **0.73** | ✓ 0.58 / **0.74** | ✓ 0.55 / **0.67** |
| <code>true</code> | ✓ 0.58 / **0.89** | ✓ 0.59 / **0.92** | ✓ 0.56 / 0.66 |
| <code>type</code> | ✓ 0.58 / **0.79** | ✓ 0.60 / **0.79** | ✓ 0.55 / **0.71** |
| <code>typeset</code> | ✓ 0.62 / **0.89** | ✓ 0.61 / **0.85** | ✓ 0.56 / **0.67** |
| <code>ulimit</code> | M | M | M |
| <code>umask</code> | M | M | M |
| <code>unalias</code> | ✓ 0.57 / **0.78** | ✓ 0.58 / **0.84** | ✓ 0.58 / **0.74** |
| <code>unset</code> | ✓ 0.59 / **0.73** | ✓ 0.59 / **0.76** | ✓ 0.56 / **0.71** |
| <code>wait</code> | ✓ **0.78** / **1.05↑** | ✓ **0.75** / **1.07↑** | ✓ 0.64 / **1.17↑** |

## Builtin option

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>. -p</code> | — | Δ | — |
| <code>alias -p</code> | ✓ 0.61 / **1.03↑** | ✓ 0.62 / **1.04↑** | ✓ 0.57 / **0.69** |
| <code>bind -P</code> | M | M | M |
| <code>bind -S</code> | M | M | M |
| <code>bind -V</code> | M | M | M |
| <code>bind -X</code> | M | M | M |
| <code>bind -f</code> | M | M | M |
| <code>bind -l</code> | M | M | M |
| <code>bind -m</code> | M | M | M |
| <code>bind -p</code> | M | M | M |
| <code>bind -q</code> | M | M | M |
| <code>bind -r</code> | M | M | M |
| <code>bind -s</code> | M | M | M |
| <code>bind -u</code> | M | M | M |
| <code>bind -v</code> | M | M | M |
| <code>bind -x</code> | M | M | M |
| <code>cd -@</code> | N | N | N |
| <code>cd -L</code> | ✓ **0.74** / **0.82** | ✓ **0.89** / **0.93** | ✓ 0.64 / **0.73** |
| <code>cd -P</code> | ✓ **0.74** / **0.82** | ✓ **0.90** / **0.94** | ✓ 0.66 / **0.73** |
| <code>cd -e</code> | Δ | Δ | Δ |
| <code>command -V</code> | ✓ 0.62 / **0.95** | ✓ 0.59 / **0.93** | ✓ 0.56 / **0.77** |
| <code>command -p</code> | ✓ **0.69** / **0.90** | ✓ **0.69** / **0.93** | ✓ **0.73** / **0.97** |
| <code>command -v</code> | ✓ 0.60 / **0.97** | ✓ 0.60 / **0.93** | ✓ 0.57 / **0.73** |
| <code>compgen -A</code> | ✓ **0.71** / **0.77** | ✓ **0.92** / **0.93** | ✓ 0.61 / **0.69** |
| <code>compgen -C</code> | ✓ 0.66 / **0.70** | ✓ **0.90** / **0.95** | ✓ 0.60 / 0.66 |
| <code>compgen -F</code> | ✓ **0.72** / **0.78** | ✓ **0.90** / **0.92** | ✓ 0.63 / **0.67** |
| <code>compgen -G</code> | ✓ **0.71** / **0.78** | ✓ **0.91** / **0.95** | ✓ 0.62 / 0.66 |
| <code>compgen -P</code> | ✓ **0.73** / **0.78** | ✓ **0.91** / **0.93** | ✓ 0.62 / 0.63 |
| <code>compgen -S</code> | ✓ **0.72** / **0.78** | ✓ **0.89** / **0.92** | ✓ 0.63 / 0.65 |
| <code>compgen -V</code> | — | Δ | — |
| <code>compgen -W</code> | ✓ **0.71** / **0.78** | ✓ **0.89** / **0.93** | ✓ 0.61 / 0.63 |
| <code>compgen -X</code> | ✓ **0.72** / **0.78** | ✓ **0.91** / **0.95** | ✓ 0.62 / **0.68** |
| <code>compgen -a</code> | ✓ **0.72** / **0.76** | ✓ **0.88** / **0.94** | ✓ 0.64 / **0.67** |
| <code>compgen -b</code> | ✓ **0.72** / **0.78** | ✓ **0.91** / **0.90** | ✓ 0.62 / 0.66 |
| <code>compgen -c</code> | ✓ **0.76** / **0.83** | ✓ **0.89** / **0.94** | ✓ 0.62 / **0.71** |
| <code>compgen -d</code> | ✓ **0.71** / **0.77** | ✓ **0.92** / **0.93** | ✓ 0.62 / **0.69** |
| <code>compgen -e</code> | ✓ **0.72** / **0.79** | ✓ **0.87** / **0.95** | ✓ 0.64 / **0.69** |
| <code>compgen -f</code> | ✓ **0.73** / **0.78** | ✓ **0.89** / **0.84** | ✓ 0.63 / **0.69** |
| <code>compgen -g</code> | Δ | Δ | ✓ 0.41 / 0.34 |
| <code>compgen -j</code> | ✓ **0.72** / **0.78** | ✓ **1.05↑** / **1.00** | ✓ 0.63 / **0.68** |
| <code>compgen -k</code> | ✓ **0.71** / **0.78** | ✓ **0.87** / **0.84** | ✓ 0.64 / **0.68** |
| <code>compgen -o</code> | ✓ **0.72** / **0.78** | ✓ **0.91** / **0.92** | ✓ 0.66 / 0.58 |
| <code>compgen -s</code> | ✓ **0.71** / **0.77** | ✓ **0.91** / **0.94** | ✓ 0.45 / 0.43 |
| <code>compgen -u</code> | Δ | Δ | Δ |
| <code>compgen -v</code> | ✓ **0.72** / **0.77** | ✓ **0.89** / **0.94** | ✓ 0.61 / **0.68** |
| <code>complete -A</code> | Δ | Δ | Δ |
| <code>complete -C</code> | ✓ **0.73** / **0.82** | ✓ **0.88** / **0.92** | ✓ 0.59 / 0.63 |
| <code>complete -D</code> | Δ | Δ | Δ |
| <code>complete -E</code> | Δ | Δ | Δ |
| <code>complete -F</code> | ✓ **0.73** / **0.82** | ✓ **0.85** / **0.91** | ✓ 0.61 / 0.66 |
| <code>complete -G</code> | ✓ **0.72** / **0.82** | ✓ **0.85** / **0.90** | ✓ 0.57 / **0.69** |
| <code>complete -I</code> | Δ | Δ | Δ |
| <code>complete -P</code> | Δ | Δ | Δ |
| <code>complete -S</code> | Δ | Δ | Δ |
| <code>complete -W</code> | ✓ **0.72** / **0.82** | ✓ **0.87** / **0.91** | ✓ 0.60 / **0.71** |
| <code>complete -X</code> | ✓ **0.72** / **0.82** | ✓ **0.85** / **0.92** | ✓ 0.62 / **0.68** |
| <code>complete -a</code> | Δ | Δ | Δ |
| <code>complete -b</code> | Δ | Δ | Δ |
| <code>complete -c</code> | Δ | Δ | Δ |
| <code>complete -d</code> | Δ | Δ | Δ |
| <code>complete -e</code> | Δ | Δ | Δ |
| <code>complete -f</code> | Δ | Δ | Δ |
| <code>complete -g</code> | Δ | Δ | Δ |
| <code>complete -j</code> | Δ | Δ | Δ |
| <code>complete -k</code> | Δ | Δ | Δ |
| <code>complete -o</code> | Δ | Δ | Δ |
| <code>complete -p</code> | ✓ 0.62 / **1.04↑** | ✓ 0.62 / **1.03↑** | ✓ 0.57 / **0.77** |
| <code>complete -r</code> | ✓ 0.63 / **0.83** | ✓ 0.63 / **0.85** | ✓ 0.56 / **0.78** |
| <code>complete -s</code> | Δ | Δ | Δ |
| <code>complete -u</code> | Δ | Δ | Δ |
| <code>complete -v</code> | Δ | Δ | Δ |
| <code>compopt -D</code> | Δ | Δ | Δ |
| <code>compopt -E</code> | Δ | Δ | Δ |
| <code>compopt -I</code> | Δ | Δ | Δ |
| <code>compopt -o</code> | Δ | Δ | Δ |
| <code>declare -A</code> | Δ | Δ | Δ |
| <code>declare -F</code> | ✓ 0.62 / **1.04↑** | ✓ 0.61 / **1.04↑** | ✓ 0.56 / **0.83** |
| <code>declare -I</code> | E | E | E |
| <code>declare -a</code> | ✓ 0.61 / **0.86** | ✓ 0.60 / **0.78** | ✓ 0.56 / **0.71** |
| <code>declare -f</code> | Δ | Δ | Δ |
| <code>declare -g</code> | ✓ 0.60 / **0.79** | ✓ 0.58 / **0.90** | ✓ 0.58 / **0.70** |
| <code>declare -i</code> | Δ | Δ | Δ |
| <code>declare -l</code> | Δ | Δ | Δ |
| <code>declare -n</code> | Δ | Δ | Δ |
| <code>declare -p</code> | ✓ 0.61 / **1.08↑** | ✓ 0.58 / **0.81** | ✓ 0.54 / **0.79** |
| <code>declare -r</code> | Δ | Δ | Δ |
| <code>declare -t</code> | Δ | Δ | Δ |
| <code>declare -u</code> | Δ | Δ | Δ |
| <code>declare -x</code> | ✓ **0.70** / **0.95** | ✓ **0.69** / **0.94** | ✓ **0.71** / **0.93** |
| <code>dirs -N</code> | M | M | M |
| <code>dirs -c</code> | M | M | M |
| <code>dirs -l</code> | M | M | M |
| <code>dirs -p</code> | M | M | M |
| <code>dirs -v</code> | M | M | M |
| <code>disown -a</code> | M | M | M |
| <code>disown -h</code> | M | M | M |
| <code>disown -r</code> | M | M | M |
| <code>echo -E</code> | ✓ 0.56 / **0.92** | ✓ 0.60 / **0.91** | ✓ 0.54 / **0.67** |
| <code>echo -e</code> | ✓ 0.61 / **0.94** | ✓ 0.58 / **0.93** | ✓ 0.56 / **0.68** |
| <code>echo -n</code> | ✓ 0.60 / **0.94** | ✓ 0.58 / **0.84** | ✓ 0.53 / **0.71** |
| <code>enable -a</code> | M | M | M |
| <code>enable -d</code> | M | M | M |
| <code>enable -f</code> | M | M | M |
| <code>enable -n</code> | M | M | M |
| <code>enable -p</code> | M | M | M |
| <code>enable -s</code> | M | M | M |
| <code>exec -a</code> | ✓ **0.69** / **0.94** | ✓ **0.69** / **0.93** | ✓ **0.74** / **1.04↑** |
| <code>exec -c</code> | ✓ **0.69** / **0.92** | ✓ **0.68** / **0.92** | ✓ **0.74** / **0.93** |
| <code>exec -l</code> | ✓ **0.88** / **0.99** | ✓ **0.91** / **0.99** | ✓ **0.73** / **1.00** |
| <code>export -f</code> | Δ | Δ | Δ |
| <code>export -n</code> | ✓ **0.68** / **0.90** | ✓ **0.70** / **0.92** | ✓ **0.71** / **0.91** |
| <code>export -p</code> | ✓ **0.76** / **0.87** | ✓ **0.75** / **0.84** | ✓ 0.60 / 0.65 |
| <code>fc -e</code> | M | M | M |
| <code>fc -l</code> | M | M | M |
| <code>fc -n</code> | M | M | M |
| <code>fc -r</code> | M | M | M |
| <code>fc -s</code> | M | M | M |
| <code>hash -d</code> | ✓ 0.62 / **0.81** | ✓ 0.63 / **0.83** | ✓ 0.57 / **0.74** |
| <code>hash -l</code> | ✓ 0.62 / **0.99** | ✓ 0.59 / **0.83** | ✓ 0.57 / **0.73** |
| <code>hash -p</code> | ✓ **0.68** / **0.88** | ✓ **0.86** / **0.97** | ✓ **0.69** / **0.83** |
| <code>hash -r</code> | ✓ 0.62 / **0.77** | ✓ 0.62 / **0.75** | ✓ 0.53 / **0.83** |
| <code>hash -t</code> | ✓ **0.68** / **0.88** | ✓ **0.86** / **0.97** | ✓ **0.69** / **0.83** |
| <code>help -d</code> | M | M | M |
| <code>help -m</code> | M | M | M |
| <code>help -s</code> | M | M | M |
| <code>history -a</code> | M | M | M |
| <code>history -c</code> | M | M | M |
| <code>history -d</code> | M | M | M |
| <code>history -n</code> | M | M | M |
| <code>history -p</code> | M | M | M |
| <code>history -r</code> | M | M | M |
| <code>history -s</code> | M | M | M |
| <code>history -w</code> | M | M | M |
| <code>jobs -l</code> | Δ | Δ | Δ |
| <code>jobs -n</code> | Δ | Δ | Δ |
| <code>jobs -p</code> | Δ | Δ | Δ |
| <code>jobs -r</code> | Δ | Δ | Δ |
| <code>jobs -s</code> | Δ | Δ | Δ |
| <code>jobs -x</code> | ✓ **1.00** / **1.00** | ✓ **1.00** / **1.00↑** | ✓ **0.90** / **0.97** |
| <code>kill -l</code> | ✓ 0.60 / **0.91** | ✓ 0.61 / **0.81** | ✓ 0.57 / **0.75** |
| <code>kill -n</code> | ✓ 0.57 / **0.88** | ✓ 0.61 / **0.87** | ✓ 0.54 / **0.73** |
| <code>kill -s</code> | ✓ 0.59 / **0.85** | ✓ 0.59 / **0.81** | ✓ 0.57 / **0.70** |
| <code>mapfile -C</code> | M | M | M |
| <code>mapfile -O</code> | M | M | M |
| <code>mapfile -c</code> | M | M | M |
| <code>mapfile -d</code> | M | M | M |
| <code>mapfile -n</code> | M | M | M |
| <code>mapfile -s</code> | M | M | M |
| <code>mapfile -t</code> | M | M | M |
| <code>mapfile -u</code> | M | M | M |
| <code>popd -N</code> | M | M | M |
| <code>popd -n</code> | M | M | M |
| <code>printf -v</code> | ✓ 0.58 / **0.90** | ✓ 0.60 / **0.85** | ✓ 0.56 / 0.67 |
| <code>pushd -N</code> | M | M | M |
| <code>pushd -n</code> | M | M | M |
| <code>pwd -L</code> | ✓ **0.72** / **0.83** | ✓ **0.85** / **0.92** | ✓ 0.62 / 0.65 |
| <code>pwd -P</code> | ✓ **0.72** / **0.82** | ✓ **0.86** / **0.91** | ✓ 0.59 / 0.65 |
| <code>read -E</code> | N | Δ | N |
| <code>read -N</code> | ✓ 0.62 / **0.86** | ✓ 0.58 / **0.79** | ✓ 0.56 / **0.68** |
| <code>read -a</code> | ✓ 0.63 / **0.81** | ✓ 0.59 / **0.75** | ✓ 0.55 / **0.70** |
| <code>read -d</code> | ✓ 0.63 / **0.86** | ✓ 0.60 / **0.82** | ✓ 0.55 / **0.69** |
| <code>read -e</code> | Δ | Δ | Δ |
| <code>read -i</code> | Δ | Δ | Δ |
| <code>read -n</code> | ✓ 0.63 / **0.87** | ✓ 0.61 / **0.84** | ✓ 0.55 / **0.73** |
| <code>read -p</code> | ✓ **0.81** | ✓ **0.80** | ✓ 0.63 |
| <code>read -r</code> | ✓ 0.61 / **0.84** | ✓ 0.60 / **0.80** | ✓ 0.57 / **0.69** |
| <code>read -s</code> | Δ | Δ | Δ |
| <code>read -t</code> | ✓ **1.01↑** / **1.03↑** | ✓ **1.01↑** / **0.99** | ✓ **0.93** / **1.03↑** |
| <code>read -u</code> | ✓ 0.59 / **0.91** | ✓ 0.61 / **0.89** | ✓ 0.60 / **0.68** |
| <code>readarray -C</code> | M | M | M |
| <code>readarray -O</code> | M | M | M |
| <code>readarray -c</code> | M | M | M |
| <code>readarray -d</code> | M | M | M |
| <code>readarray -n</code> | M | M | M |
| <code>readarray -s</code> | M | M | M |
| <code>readarray -t</code> | M | M | M |
| <code>readarray -u</code> | M | M | M |
| <code>readonly -A</code> | M | M | M |
| <code>readonly -a</code> | M | M | M |
| <code>readonly -f</code> | M | M | M |
| <code>readonly -p</code> | M | M | M |
| <code>set -B</code> | Δ | Δ | Δ |
| <code>set -C</code> | Δ | Δ | Δ |
| <code>set -E</code> | Δ | Δ | Δ |
| <code>set -H</code> | Δ | Δ | Δ |
| <code>set -P</code> | Δ | Δ | Δ |
| <code>set -T</code> | Δ | Δ | Δ |
| <code>set -a</code> | Δ | Δ | Δ |
| <code>set -b</code> | Δ | Δ | Δ |
| <code>set -e</code> | I 0.63 / **0.91** | I 0.61 / **0.88** | I 0.56 / **0.71** |
| <code>set -f</code> | I 0.60 / **0.81** | I 0.61 / **0.87** | I 0.55 / **0.74** |
| <code>set -h</code> | I 0.60 / **0.87** | I 0.60 / **0.87** | I 0.58 / **0.75** |
| <code>set -k</code> | Δ | Δ | Δ |
| <code>set -m</code> | Δ | Δ | Δ |
| <code>set -n</code> | Δ | Δ | Δ |
| <code>set -o</code> | ✓ 0.59 / 0.62 | ✓ 0.59 / 0.62 | ✓ 0.61 / **0.81** |
| <code>set -p</code> | Δ | Δ | Δ |
| <code>set -t</code> | Δ | Δ | Δ |
| <code>set -u</code> | I 0.60 / **0.86** | I 0.61 / **0.84** | I 0.56 / **0.71** |
| <code>set -v</code> | Δ | Δ | Δ |
| <code>set -x</code> | Δ | Δ | Δ |
| <code>shopt -o</code> | ✓ 0.63 / **1.04↑** | ✓ 0.63 / **0.90** | ✓ 0.56 / **0.72** |
| <code>shopt -p</code> | ✓ 0.62 / **0.97** | ✓ 0.61 / **0.96** | ✓ 0.57 / **0.73** |
| <code>shopt -q</code> | ✓ 0.61 / **0.94** | ✓ 0.62 / **0.96** | ✓ 0.58 / **0.72** |
| <code>shopt -s</code> | ✓ 0.59 / **0.75** | ✓ 0.60 / **0.87** | ✓ 0.57 / **0.78** |
| <code>shopt -u</code> | ✓ 0.59 / **0.75** | ✓ 0.60 / **0.87** | ✓ 0.57 / **0.78** |
| <code>source -p</code> | — | Δ | — |
| <code>suspend -f</code> | M | M | M |
| <code>trap -P</code> | — | Δ | — |
| <code>trap -l</code> | ✓ **0.68** / **0.77** | ✓ 0.66 / **0.76** | ✓ 0.61 / **0.70** |
| <code>trap -p</code> | ✓ 0.59 / **0.70** | ✓ 0.57 / **0.73** | ✓ 0.53 / **0.71** |
| <code>type -P</code> | ✓ 0.65 / **0.80** | ✓ **0.68** / **0.78** | ✓ 0.59 / **0.67** |
| <code>type -a</code> | ✓ 0.66 / **0.75** | ✓ **0.90** / **0.90** | ✓ 0.61 / 0.64 |
| <code>type -f</code> | ✓ 0.62 / **0.88** | ✓ 0.60 / **0.86** | ✓ 0.56 / **0.69** |
| <code>type -p</code> | ✓ 0.61 / **0.88** | ✓ 0.60 / **0.84** | ✓ 0.57 / 0.65 |
| <code>type -t</code> | ✓ 0.61 / **0.97** | ✓ 0.61 / **0.94** | ✓ 0.56 / **0.72** |
| <code>typeset -A</code> | Δ | Δ | Δ |
| <code>typeset -F</code> | ✓ 0.62 / **1.06↑** | ✓ 0.62 / **1.09↑** | ✓ 0.55 / **0.75** |
| <code>typeset -I</code> | E | E | E |
| <code>typeset -a</code> | ✓ 0.61 / **0.85** | ✓ 0.61 / **0.83** | ✓ 0.59 / **0.70** |
| <code>typeset -f</code> | Δ | Δ | Δ |
| <code>typeset -g</code> | ✓ 0.60 / **0.79** | ✓ 0.54 / **0.95** | ✓ 0.56 / **0.72** |
| <code>typeset -i</code> | Δ | Δ | Δ |
| <code>typeset -l</code> | Δ | Δ | Δ |
| <code>typeset -n</code> | Δ | Δ | Δ |
| <code>typeset -p</code> | ✓ 0.62 / **1.04↑** | ✓ 0.60 / **0.95** | ✓ 0.54 / **0.78** |
| <code>typeset -r</code> | Δ | Δ | Δ |
| <code>typeset -t</code> | Δ | Δ | Δ |
| <code>typeset -u</code> | Δ | Δ | Δ |
| <code>typeset -x</code> | ✓ **0.70** / **0.95** | ✓ **0.70** / **0.94** | ✓ **0.68** / **0.99** |
| <code>ulimit -H</code> | M | M | M |
| <code>ulimit -P</code> | M | M | M |
| <code>ulimit -R</code> | M | M | M |
| <code>ulimit -S</code> | M | M | M |
| <code>ulimit -T</code> | M | M | M |
| <code>ulimit -a</code> | M | M | M |
| <code>ulimit -b</code> | M | M | M |
| <code>ulimit -c</code> | M | M | M |
| <code>ulimit -d</code> | M | M | M |
| <code>ulimit -e</code> | M | M | M |
| <code>ulimit -f</code> | M | M | M |
| <code>ulimit -i</code> | M | M | M |
| <code>ulimit -k</code> | M | M | M |
| <code>ulimit -l</code> | M | M | M |
| <code>ulimit -m</code> | M | M | M |
| <code>ulimit -n</code> | M | M | M |
| <code>ulimit -p</code> | M | M | M |
| <code>ulimit -q</code> | M | M | M |
| <code>ulimit -r</code> | M | M | M |
| <code>ulimit -s</code> | M | M | M |
| <code>ulimit -t</code> | M | M | M |
| <code>ulimit -u</code> | M | M | M |
| <code>ulimit -v</code> | M | M | M |
| <code>ulimit -x</code> | M | M | M |
| <code>umask -S</code> | M | M | M |
| <code>umask -p</code> | M | M | M |
| <code>unalias -a</code> | ✓ 0.61 / **0.88** | ✓ 0.59 / **0.86** | ✓ 0.54 / **0.70** |
| <code>unset -f</code> | ✓ 0.59 / **0.73** | ✓ 0.59 / **0.76** | ✓ 0.56 / **0.71** |
| <code>unset -n</code> | E | E | E |
| <code>unset -v</code> | ✓ 0.62 / **0.89** | ✓ 0.62 / **0.98** | ✓ 0.55 / **0.70** |
| <code>wait -f</code> | E | E | E |
| <code>wait -n</code> | Δ | Δ | Δ |
| <code>wait -p</code> | Δ | Δ | Δ |

## Compatibility level

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>31</code> | Δ | Δ | Δ |
| <code>32</code> | Δ | — | Δ |
| <code>40</code> | Δ | Δ | Δ |
| <code>41</code> | Δ | Δ | Δ |
| <code>42</code> | Δ | Δ | Δ |
| <code>43</code> | Δ | Δ | Δ |
| <code>44</code> | Δ | Δ | Δ |
| <code>50</code> | Δ | Δ | Δ |
| <code>51</code> | ✓ 0.61 / **0.86** | ✓ 0.58 / **0.91** | ✓ 0.57 / **0.72** |
| <code>52</code> | — | Δ | — |

## Completion action

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>alias</code> | ✓ **0.73** / **0.79** | ✓ **0.90** / **0.93** | ✓ 0.59 / 0.67 |
| <code>arrayvar</code> | ✓ **0.72** / **0.78** | ✓ **0.90** / **0.92** | ✓ 0.63 / 0.66 |
| <code>binding</code> | Δ | Δ | Δ |
| <code>builtin</code> | ✓ **0.72** / **0.78** | ✓ **0.89** / **0.94** | ✓ 0.62 / **0.67** |
| <code>command</code> | ✓ **0.74** / **0.80** | ✓ **0.91** / **0.93** | ✓ 0.63 / 0.66 |
| <code>directory</code> | ✓ **0.73** / **0.77** | ✓ **0.91** / **0.92** | ✓ 0.63 / **0.68** |
| <code>disabled</code> | Δ | Δ | Δ |
| <code>enabled</code> | Δ | Δ | Δ |
| <code>export</code> | ✓ **0.73** / **0.78** | ✓ **0.91** / **0.95** | ✓ 0.64 / 0.64 |
| <code>file</code> | ✓ **0.71** / **0.77** | ✓ **0.90** / **0.94** | ✓ 0.62 / 0.66 |
| <code>function</code> | ✓ **0.73** / **0.78** | ✓ **0.88** / **0.92** | ✓ 0.62 / **0.68** |
| <code>group</code> | Δ | Δ | E |
| <code>helptopic</code> | Δ | Δ | Δ |
| <code>hostname</code> | E | E | E |
| <code>job</code> | Δ | Δ | Δ |
| <code>keyword</code> | ✓ **0.74** / **0.78** | ✓ **0.90** / **0.93** | ✓ 0.61 / 0.66 |
| <code>running</code> | Δ | Δ | Δ |
| <code>service</code> | Δ | E | Δ |
| <code>setopt</code> | Δ | Δ | Δ |
| <code>shopt</code> | Δ | Δ | Δ |
| <code>signal</code> | E | E | E |
| <code>stopped</code> | E | E | E |
| <code>user</code> | Δ | Δ | Δ |
| <code>variable</code> | ✓ **0.73** / **0.78** | ✓ **0.90** / **0.92** | ✓ 0.64 / **0.68** |

## Completion option

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>bashdefault</code> | Δ | Δ | Δ |
| <code>default</code> | Δ | Δ | Δ |
| <code>dirnames</code> | Δ | Δ | Δ |
| <code>filenames</code> | Δ | Δ | Δ |
| <code>fullquote</code> | Δ | Δ | Δ |
| <code>noquote</code> | Δ | Δ | Δ |
| <code>nosort</code> | Δ | Δ | Δ |
| <code>nospace</code> | Δ | Δ | Δ |
| <code>plusdirs</code> | Δ | Δ | Δ |

## Conditional

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>!=</code> | ✓ 0.60 / **0.79** | ✓ 0.60 / **0.86** | ✓ 0.56 / **0.78** |
| <code>-G</code> | ✓ 0.61 / **0.76** | ✓ 0.61 / **0.77** | ✓ 0.58 / **0.78** |
| <code>-L</code> | ✓ **0.67** / **0.77** | ✓ **0.81** / **0.91** | ✓ 0.61 / 0.64 |
| <code>-N</code> | Δ | Δ | Δ |
| <code>-O</code> | ✓ 0.61 / **0.75** | ✓ 0.60 / **0.77** | ✓ 0.58 / **0.73** |
| <code>-R</code> | Δ | Δ | Δ |
| <code>-S</code> | ✓ 0.60 / **0.76** | ✓ 0.61 / **0.77** | ✓ 0.59 / **0.77** |
| <code>-a</code> | ✓ 0.61 / **0.74** | ✓ 0.60 / **0.77** | ✓ 0.58 / **0.77** |
| <code>-b</code> | F | F | ✓ 0.59 / **0.74** |
| <code>-c</code> | ✓ 0.59 / **0.77** | ✓ 0.60 / **0.77** | ✓ 0.60 / **0.76** |
| <code>-d</code> | ✓ 0.61 / **0.75** | ✓ 0.60 / **0.76** | ✓ 0.59 / **0.82** |
| <code>-e</code> | ✓ 0.60 / **0.75** | ✓ 0.60 / **0.76** | ✓ 0.59 / **0.78** |
| <code>-ef</code> | ✓ **0.68** / **0.78** | ✓ **0.84** / **0.92** | ✓ 0.59 / 0.65 |
| <code>-eq</code> | ✓ 0.60 / **0.78** | ✓ 0.60 / **0.81** | ✓ 0.54 / **0.76** |
| <code>-f</code> | ✓ 0.60 / **0.74** | ✓ 0.61 / **0.77** | ✓ 0.59 / **0.74** |
| <code>-g</code> | ✓ **0.69** / **0.74** | ✓ **0.87** / **0.92** | ✓ 0.59 / 0.65 |
| <code>-ge</code> | ✓ 0.60 / **0.79** | ✓ 0.60 / **0.83** | ✓ 0.57 / **0.79** |
| <code>-gt</code> | ✓ 0.58 / **0.81** | ✓ 0.60 / **0.88** | ✓ 0.57 / **0.75** |
| <code>-h</code> | ✓ **0.67** / **0.77** | ✓ **0.81** / **0.89** | ✓ 0.57 / 0.64 |
| <code>-k</code> | ✓ **0.70** / **0.79** | ✓ **0.86** / **0.92** | ✓ 0.62 / 0.62 |
| <code>-le</code> | ✓ 0.60 / **0.79** | ✓ 0.59 / **0.80** | ✓ 0.56 / **0.71** |
| <code>-lt</code> | ✓ 0.59 / **0.77** | ✓ 0.60 / **0.76** | ✓ 0.55 / **0.78** |
| <code>-n</code> | ✓ 0.60 / **0.92** | ✓ 0.62 / **0.89** | ✓ 0.60 / **0.70** |
| <code>-ne</code> | ✓ 0.59 / **0.81** | ✓ 0.60 / **0.77** | ✓ 0.56 / **0.71** |
| <code>-nt</code> | ✓ **0.69** / **0.77** | ✓ **0.87** / **0.93** | ✓ 0.62 / **0.69** |
| <code>-o</code> | Δ | Δ | Δ |
| <code>-ot</code> | ✓ **0.69** / **0.77** | ✓ **0.86** / **0.90** | ✓ 0.62 / **0.77** |
| <code>-p</code> | ✓ **0.70** / **0.81** | ✓ **0.78** / **0.90** | ✓ 0.61 / **0.74** |
| <code>-r</code> | ✓ 0.61 / **0.75** | ✓ 0.59 / **0.74** | ✓ 0.60 / **0.77** |
| <code>-s</code> | ✓ 0.60 / **0.74** | ✓ 0.60 / **0.76** | ✓ 0.58 / **0.71** |
| <code>-t</code> | ✓ **0.84** | ✓ **0.82** | ✓ **0.67** |
| <code>-u</code> | ✓ **0.69** / **0.77** | ✓ **0.84** / **0.92** | ✓ 0.61 / 0.62 |
| <code>-v</code> | ✓ 0.60 / **0.87** | ✓ 0.59 / **0.86** | ✓ 0.59 / **0.67** |
| <code>-w</code> | ✓ 0.61 / **0.76** | ✓ 0.57 / **0.76** | ✓ 0.58 / **0.80** |
| <code>-x</code> | ✓ **0.68** / **0.76** | ✓ **0.86** / **0.91** | ✓ 0.60 / 0.63 |
| <code>-z</code> | ✓ 0.60 / **0.96** | ✓ 0.61 / **0.91** | ✓ 0.58 / **0.71** |
| <code>&lt;</code> | ✓ 0.60 / **0.81** | ✓ 0.60 / **0.85** | ✓ 0.55 / **0.67** |
| <code>=</code> | ✓ 0.60 / **0.81** | ✓ 0.60 / **0.85** | ✓ 0.57 / **0.73** |
| <code>==</code> | ✓ 0.59 / **0.80** | ✓ 0.61 / **0.81** | ✓ 0.56 / **0.69** |
| <code>=~</code> | ✓ 0.62 / **0.84** | ✓ 0.61 / **0.78** | ✓ 0.56 / **0.78** |
| <code>&gt;</code> | ✓ 0.61 / **0.80** | ✓ 0.60 / **0.79** | ✓ 0.56 / **0.73** |

## GNU regression group

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>run-alias</code> | Δ | Δ | — |
| <code>run-appendop</code> | Δ | Δ | — |
| <code>run-arith</code> | Δ | Δ | — |
| <code>run-arith-for</code> | Δ | Δ | — |
| <code>run-array</code> | Δ | Δ | — |
| <code>run-array2</code> | G **0.76** | G **0.76** | — |
| <code>run-assoc</code> | Δ | Δ | — |
| <code>run-attr</code> | Δ | Δ | — |
| <code>run-braces</code> | Δ | Δ | — |
| <code>run-builtins</code> | Δ | ? | — |
| <code>run-case</code> | Δ | Δ | — |
| <code>run-casemod</code> | Δ | Δ | — |
| <code>run-complete</code> | Δ | Δ | — |
| <code>run-comsub</code> | Δ | Δ | — |
| <code>run-comsub-eof</code> | Δ | Δ | — |
| <code>run-comsub-posix</code> | Δ | Δ | — |
| <code>run-comsub2</code> | — | Δ | — |
| <code>run-cond</code> | Δ | Δ | — |
| <code>run-coproc</code> | Δ | Δ | — |
| <code>run-cprint</code> | Δ | Δ | — |
| <code>run-dbg-support</code> | Δ | Δ | — |
| <code>run-dbg-support2</code> | Δ | Δ | — |
| <code>run-dirstack</code> | Δ | Δ | — |
| <code>run-dollars</code> | Δ | Δ | — |
| <code>run-dynvar</code> | Δ | Δ | — |
| <code>run-errors</code> | Δ | Δ | — |
| <code>run-execscript</code> | ? | ? | — |
| <code>run-exp-tests</code> | Δ | Δ | — |
| <code>run-exportfunc</code> | ↗ | ↗ | — |
| <code>run-extglob</code> | Δ | Δ | — |
| <code>run-extglob2</code> | Δ | Δ | — |
| <code>run-extglob3</code> | Δ | Δ | — |
| <code>run-func</code> | Δ | Δ | — |
| <code>run-getopts</code> | Δ | Δ | — |
| <code>run-glob-bracket</code> | — | Δ | — |
| <code>run-glob-test</code> | Δ | Δ | — |
| <code>run-globstar</code> | Δ | Δ | — |
| <code>run-heredoc</code> | Δ | Δ | — |
| <code>run-herestr</code> | Δ | Δ | — |
| <code>run-histexpand</code> | Δ | Δ | — |
| <code>run-history</code> | Δ | Δ | — |
| <code>run-ifs</code> | Δ | Δ | — |
| <code>run-ifs-posix</code> | G **0.88** | G **0.87** | — |
| <code>run-input-test</code> | G **0.82** | G **0.81** | — |
| <code>run-intl</code> | ? | ? | — |
| <code>run-invert</code> | G **0.79** | G **0.79** | — |
| <code>run-invocation</code> | — | Δ | — |
| <code>run-iquote</code> | G **0.71** | G **0.73** | — |
| <code>run-jobs</code> | Δ | T | — |
| <code>run-lastpipe</code> | Δ | Δ | — |
| <code>run-mapfile</code> | Δ | Δ | — |
| <code>run-more-exp</code> | Δ | Δ | — |
| <code>run-nameref</code> | Δ | Δ | — |
| <code>run-new-exp</code> | Δ | Δ | — |
| <code>run-nquote</code> | Δ | Δ | — |
| <code>run-nquote1</code> | Δ | Δ | — |
| <code>run-nquote2</code> | G **0.76** | G **0.76** | — |
| <code>run-nquote3</code> | G **0.76** | G **0.76** | — |
| <code>run-nquote4</code> | Δ | Δ | — |
| <code>run-nquote5</code> | G **0.77** | G **0.76** | — |
| <code>run-parser</code> | Δ | Δ | — |
| <code>run-posix2</code> | Δ | Δ | — |
| <code>run-posixexp</code> | Δ | Δ | — |
| <code>run-posixexp2</code> | Δ | Δ | — |
| <code>run-posixpat</code> | Δ | Δ | — |
| <code>run-posixpipe</code> | Δ | Δ | — |
| <code>run-precedence</code> | G **0.79** | G **0.78** | — |
| <code>run-printf</code> | Δ | ? | — |
| <code>run-procsub</code> | Δ | Δ | — |
| <code>run-quote</code> | Δ | Δ | — |
| <code>run-quotearray</code> | Δ | ? | — |
| <code>run-read</code> | ? | ? | — |
| <code>run-redir</code> | Δ | Δ | — |
| <code>run-rhs-exp</code> | Δ | Δ | — |
| <code>run-rsh</code> | Δ | Δ | — |
| <code>run-set-e</code> | Δ | Δ | — |
| <code>run-set-x</code> | Δ | Δ | — |
| <code>run-shopt</code> | Δ | Δ | — |
| <code>run-strip</code> | G **0.75** | G **0.75** | — |
| <code>run-test</code> | Δ | ? | — |
| <code>run-tilde</code> | Δ | Δ | — |
| <code>run-tilde2</code> | Δ | Δ | — |
| <code>run-trap</code> | Δ | Δ | — |
| <code>run-type</code> | Δ | Δ | — |
| <code>run-varenv</code> | Δ | Δ | — |
| <code>run-vredir</code> | Δ | Δ | — |

## History expansion

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>!!</code> | Δ | Δ | Δ |
| <code>!!:$</code> | Δ | Δ | Δ |
| <code>!!:$:e</code> | Δ | Δ | Δ |
| <code>!!:$:h</code> | Δ | Δ | Δ |
| <code>!!:$:r</code> | Δ | Δ | Δ |
| <code>!!:$:t</code> | Δ | Δ | Δ |
| <code>!!:*</code> | Δ | Δ | Δ |
| <code>!!:0</code> | Δ | Δ | Δ |
| <code>!!:1-2</code> | Δ | Δ | Δ |
| <code>!!:2*</code> | Δ | Δ | Δ |
| <code>!!:^</code> | Δ | Δ | Δ |
| <code>!!:p</code> | Δ | Δ | Δ |
| <code>!!:q</code> | Δ | Δ | Δ |
| <code>!!:s/file/name/</code> | Δ | Δ | Δ |
| <code>!!:x</code> | Δ | Δ | Δ |
| <code>!-1</code> | Δ | Δ | Δ |
| <code>!1</code> | Δ | Δ | Δ |
| <code>!?file?</code> | Δ | Δ | Δ |
| <code>!echo</code> | Δ | Δ | Δ |

## Invocation

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>+O</code> | Δ | Δ | Δ |
| <code>+o</code> | ✓ 0.59 | ✓ 0.58 | ✓ 0.64 |
| <code>--</code> | ✓ 0.60 | ✓ 0.59 | ✓ 0.58 |
| <code>--debugger</code> | ? | ? | ? |
| <code>--dump-po-strings</code> | Δ | Δ | Δ |
| <code>--dump-strings</code> | Δ | Δ | Δ |
| <code>--help</code> | Δ | Δ | Δ |
| <code>--init-file</code> | E | E | E |
| <code>--login</code> | Δ | Δ | Δ |
| <code>--noediting</code> | I 0.62 | I 0.60 | I 0.56 |
| <code>--noprofile</code> | ✓ 0.62 | ✓ 0.61 | ✓ 0.56 |
| <code>--norc</code> | I 0.61 | I 0.59 | I 0.58 |
| <code>--posix</code> | Δ | Δ | Δ |
| <code>--pretty-print</code> | Δ | Δ | Δ |
| <code>--rcfile</code> | E | E | E |
| <code>--restricted</code> | Δ | Δ | Δ |
| <code>--verbose</code> | Δ | Δ | Δ |
| <code>--version</code> | Δ | Δ | Δ |
| <code>-B</code> | Δ | Δ | Δ |
| <code>-C</code> | Δ | Δ | Δ |
| <code>-D</code> | Δ | Δ | Δ |
| <code>-E</code> | Δ | Δ | Δ |
| <code>-H</code> | Δ | Δ | Δ |
| <code>-O</code> | Δ | Δ | Δ |
| <code>-P</code> | Δ | Δ | Δ |
| <code>-T</code> | Δ | Δ | Δ |
| <code>-a (export assignments)</code> | Δ | Δ | Δ |
| <code>-b</code> | Δ | Δ | Δ |
| <code>-c</code> | ✓ 0.59 | ✓ 0.61 | ✓ 0.58 |
| <code>-e</code> | ✓ 0.58 | ✓ 0.59 | ✓ 0.60 |
| <code>-f</code> | ✓ **0.67** | ✓ **0.83** | ✓ 0.61 |
| <code>-h</code> | Δ | Δ | Δ |
| <code>-i</code> | E | E | E |
| <code>-k</code> | Δ | Δ | Δ |
| <code>-l</code> | Δ | Δ | Δ |
| <code>-m</code> | Δ | Δ | Δ |
| <code>-n</code> | ✓ 0.58 | ✓ 0.58 | ✓ 0.57 |
| <code>-o</code> | ✓ 0.60 | ✓ 0.59 | ✓ 0.62 |
| <code>-p</code> | Δ | Δ | Δ |
| <code>-r</code> | Δ | Δ | Δ |
| <code>-s</code> | ✓ 0.61 | ✓ 0.61 | ✓ 0.56 |
| <code>-t</code> | Δ | Δ | Δ |
| <code>-u</code> | E | E | E |
| <code>-v</code> | Δ | Δ | Δ |
| <code>-x</code> | Δ | Δ | Δ |
| <code>login logout</code> | Δ | Δ | Δ |
| <code>read script file</code> | ✓ 0.61 | ✓ 0.59 | ✓ 0.57 |
| <code>stdin script</code> | ✓ 0.63 | ✓ 0.61 | ✓ 0.58 |

## Manual section

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>A Programmable Completion Example</code> | § | § | § |
| <code>ANSI-C Quoting</code> | § | § | § |
| <code>Aliases</code> | § | § | § |
| <code>Arithmetic Expansion</code> | § | § | § |
| <code>Arrays</code> | § | § | § |
| <code>Bash Builtins</code> | § | § | § |
| <code>Bash Conditional Expressions</code> | § | § | § |
| <code>Bash Features</code> | § | § | § |
| <code>Bash History Builtins</code> | § | § | § |
| <code>Bash History Facilities</code> | § | § | § |
| <code>Bash POSIX Mode</code> | § | § | § |
| <code>Bash Startup Files</code> | § | § | § |
| <code>Bash Variables</code> | § | § | § |
| <code>Basic Installation</code> | § | § | § |
| <code>Basic Shell Features</code> | § | § | § |
| <code>Bindable Readline Commands</code> | § | § | § |
| <code>Bourne Shell Builtins</code> | § | § | § |
| <code>Bourne Shell Variables</code> | § | § | § |
| <code>Brace Expansion</code> | § | § | § |
| <code>Builtin Index</code> | § | § | § |
| <code>Command Execution Environment</code> | § | § | § |
| <code>Command Grouping</code> | § | § | § |
| <code>Command Line Editing</code> | § | § | § |
| <code>Command Search and Execution</code> | § | § | § |
| <code>Command Substitution</code> | § | § | § |
| <code>Commands For Completion</code> | § | § | § |
| <code>Commands For History</code> | § | § | § |
| <code>Commands For Killing</code> | § | § | § |
| <code>Commands For Moving</code> | § | § | § |
| <code>Commands For Text</code> | § | § | § |
| <code>Comments</code> | § | § | § |
| <code>Compilers and Options</code> | § | § | § |
| <code>Compiling For Multiple Architectures</code> | § | § | § |
| <code>Compound Commands</code> | § | § | § |
| <code>Concept Index</code> | § | § | § |
| <code>Conditional Constructs</code> | § | § | § |
| <code>Conditional Init Constructs</code> | § | § | § |
| <code>Controlling the Prompt</code> | § | § | § |
| <code>Coprocesses</code> | § | § | § |
| <code>Creating Internationalized Scripts</code> | § | § | § |
| <code>Definitions</code> | § | § | § |
| <code>Directory Stack Builtins</code> | § | § | § |
| <code>Double Quotes</code> | § | § | § |
| <code>Environment</code> | § | § | § |
| <code>Escape Character</code> | § | § | § |
| <code>Event Designators</code> | § | § | § |
| <code>Executing Commands</code> | § | § | § |
| <code>Exit Status</code> | § | § | § |
| <code>Filename Expansion</code> | § | § | § |
| <code>Function Index</code> | § | § | § |
| <code>GNU Free Documentation License</code> | § | § | § |
| <code>GNU Parallel</code> | § | § | § |
| <code>History Interaction</code> | § | § | § |
| <code>Indexes</code> | § | § | § |
| <code>Installation Names</code> | § | § | § |
| <code>Installing Bash</code> | § | § | § |
| <code>Interactive Shell Behavior</code> | § | § | § |
| <code>Interactive Shells</code> | § | § | § |
| <code>Introduction</code> | § | § | § |
| <code>Introduction and Notation</code> | § | § | § |
| <code>Invoking Bash</code> | § | § | § |
| <code>Is this Shell Interactive?</code> | § | § | § |
| <code>Job Control</code> | § | § | § |
| <code>Job Control Basics</code> | § | § | § |
| <code>Job Control Builtins</code> | § | § | § |
| <code>Job Control Variables</code> | § | § | § |
| <code>Keyboard Macros</code> | § | § | § |
| <code>Lists</code> | § | § | § |
| <code>Locale Translation</code> | § | § | § |
| <code>Looping Constructs</code> | § | § | § |
| <code>Major Differences From The Bourne Shell</code> | § | § | § |
| <code>Miscellaneous Commands</code> | § | § | § |
| <code>Modifiers</code> | § | § | § |
| <code>Modifying Shell Behavior</code> | § | § | § |
| <code>Numeric Arguments</code> | § | § | § |
| <code>Operation Controls</code> | § | § | § |
| <code>Optional Features</code> | § | § | § |
| <code>Pattern Matching</code> | § | § | § |
| <code>Pipelines</code> | § | § | § |
| <code>Positional Parameters</code> | § | § | § |
| <code>Process Substitution</code> | § | § | § |
| <code>Programmable Completion</code> | § | § | § |
| <code>Programmable Completion Builtins</code> | § | § | § |
| <code>Quote Removal</code> | § | § | § |
| <code>Quoting</code> | § | § | § |
| <code>Readline Arguments</code> | § | § | § |
| <code>Readline Bare Essentials</code> | § | § | § |
| <code>Readline Init File</code> | § | § | § |
| <code>Readline Init File Syntax</code> | § | § | § |
| <code>Readline Interaction</code> | § | § | § |
| <code>Readline Killing Commands</code> | § | § | § |
| <code>Readline Movement Commands</code> | § | § | § |
| <code>Readline vi Mode</code> | § | § | § |
| <code>Redirections</code> | § | § | § |
| <code>Reporting Bugs</code> | § | § | § |
| <code>Reserved Word Index</code> | § | § | § |
| <code>Reserved Words</code> | § | § | § |
| <code>Sample Init File</code> | § | § | § |
| <code>Searching</code> | § | § | § |
| <code>Sharing Defaults</code> | § | § | § |
| <code>Shell Arithmetic</code> | § | § | § |
| <code>Shell Builtin Commands</code> | § | § | § |
| <code>Shell Commands</code> | § | § | § |
| <code>Shell Compatibility Mode</code> | § | § | § |
| <code>Shell Expansions</code> | § | § | § |
| <code>Shell Functions</code> | § | § | § |
| <code>Shell Operation</code> | § | § | § |
| <code>Shell Parameter Expansion</code> | § | § | § |
| <code>Shell Parameters</code> | § | § | § |
| <code>Shell Scripts</code> | § | § | § |
| <code>Shell Syntax</code> | § | § | § |
| <code>Shell Variables</code> | § | § | § |
| <code>Signals</code> | § | § | § |
| <code>Simple Command Expansion</code> | § | § | § |
| <code>Simple Commands</code> | § | § | § |
| <code>Single Quotes</code> | § | § | § |
| <code>Special Builtins</code> | § | § | § |
| <code>Special Parameters</code> | § | § | § |
| <code>Specifying the System Type</code> | § | § | § |
| <code>The Directory Stack</code> | § | § | § |
| <code>The Restricted Shell</code> | § | § | § |
| <code>The Set Builtin</code> | § | § | § |
| <code>The Shopt Builtin</code> | § | § | § |
| <code>Tilde Expansion</code> | § | § | § |
| <code>Top</code> | § | § | § |
| <code>Using History Interactively</code> | § | § | § |
| <code>Variable Index</code> | § | § | § |
| <code>What is Bash?</code> | § | § | § |
| <code>What is a shell?</code> | § | § | § |
| <code>What is an Interactive Shell?</code> | § | § | § |
| <code>Word Designators</code> | § | § | § |
| <code>Word Splitting</code> | § | § | § |

## POSIX mode item

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>01</code> | P | P | P |
| <code>02</code> | P | P | P |
| <code>03</code> | P | P | P |
| <code>04</code> | P | P | P |
| <code>05</code> | P | P | P |
| <code>06</code> | P | P | P |
| <code>07</code> | P | P | P |
| <code>08</code> | P | P | P |
| <code>09</code> | P | P | P |
| <code>10</code> | P | P | P |
| <code>11</code> | P | P | P |
| <code>12</code> | P | P | P |
| <code>13</code> | P | P | P |
| <code>14</code> | P | P | P |
| <code>15</code> | P | P | P |
| <code>16</code> | P | P | P |
| <code>17</code> | P | P | P |
| <code>18</code> | P | P | P |
| <code>19</code> | P | P | P |
| <code>20</code> | P | P | P |
| <code>21</code> | P | P | P |
| <code>22</code> | P | P | P |
| <code>23</code> | P | P | P |
| <code>24</code> | P | P | P |
| <code>25</code> | P | P | P |
| <code>26</code> | P | P | P |
| <code>27</code> | P | P | P |
| <code>28</code> | P | P | P |
| <code>29</code> | P | P | P |
| <code>30</code> | P | P | P |
| <code>31</code> | P | P | P |
| <code>32</code> | P | P | P |
| <code>33</code> | P | P | P |
| <code>34</code> | P | P | P |
| <code>35</code> | P | P | P |
| <code>36</code> | P | P | P |
| <code>37</code> | P | P | P |
| <code>38</code> | P | P | P |
| <code>39</code> | P | P | P |
| <code>40</code> | P | P | P |
| <code>41</code> | P | P | P |
| <code>42</code> | P | P | P |
| <code>43</code> | P | P | P |
| <code>44</code> | P | P | P |
| <code>45</code> | P | P | P |
| <code>46</code> | P | P | P |
| <code>47</code> | P | P | P |
| <code>48</code> | P | P | P |
| <code>49</code> | P | P | P |
| <code>50</code> | P | P | P |
| <code>51</code> | P | P | P |
| <code>52</code> | P | P | P |
| <code>53</code> | P | P | P |
| <code>54</code> | P | P | P |
| <code>55</code> | P | P | P |
| <code>56</code> | P | P | P |
| <code>57</code> | P | P | P |
| <code>58</code> | P | P | P |
| <code>59</code> | P | P | P |
| <code>60</code> | P | P | P |
| <code>61</code> | P | P | P |
| <code>62</code> | P | P | P |
| <code>63</code> | — | P | — |
| <code>64</code> | — | P | — |
| <code>65</code> | — | P | — |
| <code>66</code> | — | P | — |
| <code>67</code> | — | P | — |
| <code>68</code> | — | P | — |
| <code>69</code> | — | P | — |
| <code>70</code> | — | P | — |
| <code>71</code> | — | P | — |
| <code>72</code> | — | P | — |
| <code>73</code> | — | P | — |
| <code>74</code> | — | P | — |
| <code>75</code> | — | P | — |
| <code>76</code> | — | P | — |

## Parameter

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>$!</code> | ✓ **0.78** / **1.01↑** | ✓ **0.79** / **1.05↑** | ✓ **0.67** / **1.18↑** |
| <code>$#</code> | ✓ 0.62 / **1.02↑** | ✓ 0.62 / **1.09↑** | ✓ 0.56 / **0.69** |
| <code>$$</code> | ✓ 0.64 / **0.75** | ✓ 0.64 / **0.74** | ✓ 0.59 / **0.76** |
| <code>$*</code> | ✓ 0.61 / **1.05↑** | ✓ 0.62 / **1.04↑** | ✓ 0.56 / **0.72** |
| <code>$-</code> | ✓ 0.62 / **1.01↑** | ✓ 0.62 / **1.00↑** | ✓ 0.58 / **0.69** |
| <code>$0</code> | ✓ 0.62 / **0.96** | ✓ 0.60 / **0.98** | ✓ 0.57 / **0.76** |
| <code>$1 … ${10}</code> | ✓ 0.62 / **0.87** | ✓ 0.60 / **0.90** | ✓ 0.67 / **0.70** |
| <code>$?</code> | ✓ 0.62 / **1.09↑** | ✓ 0.62 / **1.02↑** | ✓ 0.60 / **0.76** |
| <code>$@</code> | ✓ 0.62 / **0.87** | ✓ 0.61 / **1.01↑** | ✓ 0.59 / **0.71** |
| <code>$_</code> | Δ | Δ | Δ |
| <code>${!a[@]}</code> | ✓ 0.61 / **0.85** | ✓ 0.60 / **0.87** | ✓ 0.59 / **0.71** |
| <code>${!name}</code> | ✓ 0.63 / **1.01↑** | ✓ 0.60 / **1.03↑** | ✓ 0.57 / **0.72** |
| <code>${!prefix*}</code> | Δ | Δ | Δ |
| <code>${!prefix@}</code> | Δ | Δ | Δ |
| <code>${#a[@]}</code> | ✓ 0.61 / **1.02↑** | ✓ 0.60 / **0.87** | ✓ 0.60 / **0.72** |
| <code>${#v}</code> | ✓ **0.69** / **0.78** | ✓ **0.68** / **0.76** | ✓ 0.62 / **0.86** |
| <code>${a[*]}</code> | ✓ 0.62 / **0.88** | ✓ 0.61 / **0.89** | ✓ 0.57 / **0.74** |
| <code>${a[@]}</code> | ✓ 0.62 / **0.87** | ✓ 0.61 / **0.88** | ✓ 0.55 / **0.73** |
| <code>${v##a*}</code> | ✓ **0.69** / **0.79** | ✓ **0.69** / **0.76** | ✓ 0.63 / **0.82** |
| <code>${v#a*}</code> | ✓ **0.69** / **0.79** | ✓ **0.68** / **0.76** | ✓ 0.64 / **0.77** |
| <code>${v%%*a}</code> | ✓ **0.69** / **0.80** | ✓ **0.69** / **0.78** | ✓ 0.65 / **0.77** |
| <code>${v%*a}</code> | ✓ **0.69** / **0.78** | ✓ 0.62 / **0.74** | ✓ 0.64 / **0.82** |
| <code>${v+a}</code> | ✓ **0.70** / **0.78** | ✓ **0.68** / **0.77** | ✓ 0.64 / **0.71** |
| <code>${v,,}</code> | Δ | Δ | Δ |
| <code>${v,}</code> | Δ | Δ | Δ |
| <code>${v-a}</code> | ✓ **0.67** / **0.78** | ✓ **0.67** / **0.76** | ✓ 0.62 / **0.80** |
| <code>${v/#a/X}</code> | ✓ **0.69** / **0.76** | ✓ **0.68** / **0.78** | ✓ 0.61 / **0.74** |
| <code>${v/%a/X}</code> | ✓ **0.69** / **0.78** | ✓ **0.68** / **0.77** | ✓ 0.63 / **0.87** |
| <code>${v//a/X}</code> | ✓ **0.68** / **0.78** | ✓ **0.69** / **0.78** | ✓ 0.65 / **0.71** |
| <code>${v/a/X}</code> | ✓ **0.70** / **0.75** | ✓ **0.69** / **0.78** | ✓ **0.67** / **0.75** |
| <code>${v: -2:1}</code> | ✓ **0.69** / **0.78** | ✓ **0.68** / **0.74** | ✓ 0.65 / **0.76** |
| <code>${v:+a}</code> | ✓ **0.69** / **0.76** | ✓ **0.68** / **0.77** | ✓ 0.61 / **0.79** |
| <code>${v:-a}</code> | ✓ **0.70** / **0.77** | ✓ **0.69** / **0.76** | ✓ 0.65 / **0.79** |
| <code>${v:1}</code> | ✓ **0.69** / **0.78** | ✓ **0.67** / **0.77** | ✓ 0.65 / **0.79** |
| <code>${v:=a}</code> | ✓ **0.68** / **0.78** | ✓ **0.67** / **0.78** | ✓ **0.68** / **0.79** |
| <code>${v:?a}</code> | ✓ 0.64 / **0.74** | ✓ 0.65 / **0.72** | ✓ 0.63 / **0.76** |
| <code>${v=a}</code> | ✓ **0.68** / **0.77** | ✓ **0.68** / **0.77** | ✓ 0.63 / **0.85** |
| <code>${v?a}</code> | ✓ 0.67 / **0.74** | ✓ 0.66 / **0.75** | ✓ 0.60 / **0.82** |
| <code>${v@A}</code> | Δ | Δ | Δ |
| <code>${v@E}</code> | Δ | Δ | Δ |
| <code>${v@K}</code> | Δ | Δ | Δ |
| <code>${v@L}</code> | Δ | Δ | Δ |
| <code>${v@P}</code> | ✓ **0.70** / **0.79** | ✓ **0.67** / **0.76** | ✓ 0.63 / **0.82** |
| <code>${v@Q}</code> | Δ | Δ | Δ |
| <code>${v@U}</code> | Δ | Δ | Δ |
| <code>${v@a}</code> | Δ | Δ | Δ |
| <code>${v@k}</code> | Δ | Δ | Δ |
| <code>${v@u}</code> | Δ | Δ | Δ |
| <code>${v^^}</code> | Δ | Δ | Δ |
| <code>${v^}</code> | Δ | Δ | Δ |
| <code>${v}</code> | ✓ **0.68** / **0.77** | ✓ **0.69** / **0.75** | ✓ 0.65 / **0.74** |

## Printf format

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>%</code> | ✓ 0.60 / **0.95** | ✓ 0.58 / **0.92** | ✓ 0.59 / **0.71** |
| <code>(fmt)T</code> | Δ | Δ | Δ |
| <code>A</code> | ✓ 0.60 / **1.00↑** | ✓ 0.60 / **0.97** | ✓ 0.58 / **0.73** |
| <code>E</code> | ✓ 0.55 / **0.83** | ✓ 0.60 / **0.95** | ✓ 0.56 / **0.79** |
| <code>G</code> | ✓ 0.62 / **1.05↑** | ✓ 0.58 / **1.02↑** | ✓ 0.58 / **0.75** |
| <code>Q</code> | ✓ 0.61 / **0.93** | ✓ 0.58 / **0.95** | ✓ 0.58 / **0.73** |
| <code>X</code> | ✓ 0.59 / **0.92** | ✓ 0.59 / **0.92** | ✓ 0.59 / **0.78** |
| <code>a</code> | ✓ 0.61 / **0.97** | ✓ 0.60 / **0.95** | ✓ 0.59 / **0.74** |
| <code>b</code> | ✓ 0.61 / **0.95** | ✓ 0.58 / **0.84** | ✓ 0.57 / **0.76** |
| <code>c</code> | ✓ 0.59 / **0.94** | ✓ 0.58 / **0.94** | ✓ 0.57 / **0.68** |
| <code>d</code> | ✓ 0.58 / **0.98** | ✓ 0.58 / **0.94** | ✓ 0.56 / **0.74** |
| <code>e</code> | ✓ 0.56 / **0.87** | ✓ 0.60 / **0.97** | ✓ 0.58 / **0.73** |
| <code>f</code> | ✓ 0.59 / **0.99** | ✓ 0.59 / **0.97** | ✓ 0.55 / **0.69** |
| <code>g</code> | ✓ 0.60 / **1.02↑** | ✓ 0.60 / **0.94** | ✓ 0.56 / **0.73** |
| <code>i</code> | ✓ 0.60 / **0.96** | ✓ 0.59 / **0.84** | ✓ 0.59 / **0.78** |
| <code>n</code> | ✓ 0.59 / **1.11↑** | ✓ 0.60 / **1.07↑** | ✓ 0.58 / **0.74** |
| <code>o</code> | ✓ 0.61 / **0.95** | ✓ 0.59 / **0.94** | ✓ 0.56 / **0.70** |
| <code>q</code> | ✓ 0.61 / **0.95** | ✓ 0.58 / **0.92** | ✓ 0.58 / **0.73** |
| <code>reuse format</code> | ✓ 0.62 / **1.01↑** | ✓ 0.61 / **1.06↑** | ✓ 0.57 / **0.70** |
| <code>s</code> | ✓ 0.61 / **0.95** | ✓ 0.58 / **0.91** | ✓ 0.58 / 0.63 |
| <code>u</code> | ✓ 0.60 / **0.98** | ✓ 0.59 / **0.97** | ✓ 0.57 / **0.80** |
| <code>width / precision</code> | ✓ 0.62 / **1.09↑** | ✓ 0.59 / **1.02↑** | ✓ 0.57 / **0.79** |
| <code>x</code> | ✓ 0.59 / **0.97** | ✓ 0.59 / **0.92** | ✓ 0.57 / **0.73** |

## Prompt escape

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>\!</code> | Δ | Δ | Δ |
| <code>\#</code> | Δ | Δ | Δ |
| <code>\$</code> | ✓ **0.83** | ✓ **0.78** | ✓ 0.64 |
| <code>\101</code> | Δ | Δ | Δ |
| <code>\@</code> | Δ | Δ | Δ |
| <code>\A</code> | Δ | Δ | Δ |
| <code>\D{%Y}</code> | Δ | Δ | Δ |
| <code>\H</code> | ✓ **0.80** | ✓ **0.81** | ✓ 0.61 |
| <code>\T</code> | Δ | Δ | Δ |
| <code>\V</code> | Δ | Δ | Δ |
| <code>\W</code> | ✓ **0.78** | ✓ **0.74** | ✓ 0.60 |
| <code>\[</code> | ✓ **0.80** | ✓ **0.81** | ✓ 0.61 |
| <code>\\</code> | ✓ **0.79** | ✓ **0.82** | ✓ 0.63 |
| <code>\]</code> | ✓ **0.79** | ✓ **0.82** | ✓ 0.63 |
| <code>\a</code> | ✓ **0.79** | ✓ **0.79** | ✓ 0.62 |
| <code>\d</code> | Δ | Δ | Δ |
| <code>\e</code> | ✓ **0.80** | ✓ **0.81** | ✓ 0.65 |
| <code>\h</code> | ✓ **0.81** | ✓ **0.79** | ✓ 0.63 |
| <code>\j</code> | Δ | Δ | Δ |
| <code>\l</code> | Δ | Δ | Δ |
| <code>\n</code> | ✓ **0.82** | ✓ **0.83** | ✓ 0.57 |
| <code>\r</code> | ✓ **0.73** | ✓ **0.79** | ✓ 0.62 |
| <code>\s</code> | Δ | Δ | Δ |
| <code>\t</code> | Δ | Δ | Δ |
| <code>\u</code> | ✓ **0.79** | ✓ **0.82** | ✓ **0.80** |
| <code>\v</code> | Δ | Δ | Δ |
| <code>\w</code> | ✓ **0.82** | ✓ **0.81** | ✓ 0.60 |

## Readline command

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>abort</code> | M | M | M |
| <code>accept-line</code> | M | M | M |
| <code>alias-expand-line</code> | M | M | M |
| <code>arrow-key-prefix</code> | M | M | M |
| <code>backward-byte</code> | M | M | M |
| <code>backward-char</code> | M | M | M |
| <code>backward-delete-char</code> | M | M | M |
| <code>backward-kill-line</code> | M | M | M |
| <code>backward-kill-word</code> | M | M | M |
| <code>backward-word</code> | M | M | M |
| <code>bash-vi-complete</code> | — | M | — |
| <code>beginning-of-history</code> | M | M | M |
| <code>beginning-of-line</code> | M | M | M |
| <code>bracketed-paste-begin</code> | M | M | M |
| <code>call-last-kbd-macro</code> | M | M | M |
| <code>capitalize-word</code> | M | M | M |
| <code>character-search</code> | M | M | M |
| <code>character-search-backward</code> | M | M | M |
| <code>clear-display</code> | M | M | M |
| <code>clear-screen</code> | M | M | M |
| <code>complete</code> | M | M | M |
| <code>complete-command</code> | M | M | M |
| <code>complete-filename</code> | M | M | M |
| <code>complete-hostname</code> | M | M | M |
| <code>complete-into-braces</code> | M | M | M |
| <code>complete-username</code> | M | M | M |
| <code>complete-variable</code> | M | M | M |
| <code>copy-backward-word</code> | M | M | M |
| <code>copy-forward-word</code> | M | M | M |
| <code>copy-region-as-kill</code> | M | M | M |
| <code>dabbrev-expand</code> | M | M | M |
| <code>delete-char</code> | M | M | M |
| <code>delete-char-or-list</code> | M | M | M |
| <code>delete-horizontal-space</code> | M | M | M |
| <code>digit-argument</code> | M | M | M |
| <code>display-shell-version</code> | M | M | M |
| <code>do-lowercase-version</code> | M | M | M |
| <code>downcase-word</code> | M | M | M |
| <code>dump-functions</code> | M | M | M |
| <code>dump-macros</code> | M | M | M |
| <code>dump-variables</code> | M | M | M |
| <code>dynamic-complete-history</code> | M | M | M |
| <code>edit-and-execute-command</code> | M | M | M |
| <code>emacs-editing-mode</code> | M | M | M |
| <code>end-kbd-macro</code> | M | M | M |
| <code>end-of-history</code> | M | M | M |
| <code>end-of-line</code> | M | M | M |
| <code>exchange-point-and-mark</code> | M | M | M |
| <code>execute-named-command</code> | — | M | — |
| <code>export-completions</code> | — | M | — |
| <code>fetch-history</code> | M | M | M |
| <code>forward-backward-delete-char</code> | M | M | M |
| <code>forward-byte</code> | M | M | M |
| <code>forward-char</code> | M | M | M |
| <code>forward-search-history</code> | M | M | M |
| <code>forward-word</code> | M | M | M |
| <code>glob-complete-word</code> | M | M | M |
| <code>glob-expand-word</code> | M | M | M |
| <code>glob-list-expansions</code> | M | M | M |
| <code>history-and-alias-expand-line</code> | M | M | M |
| <code>history-expand-line</code> | M | M | M |
| <code>history-search-backward</code> | M | M | M |
| <code>history-search-forward</code> | M | M | M |
| <code>history-substring-search-backward</code> | M | M | M |
| <code>history-substring-search-forward</code> | M | M | M |
| <code>insert-comment</code> | M | M | M |
| <code>insert-completions</code> | M | M | M |
| <code>insert-last-argument</code> | M | M | M |
| <code>kill-line</code> | M | M | M |
| <code>kill-region</code> | M | M | M |
| <code>kill-whole-line</code> | M | M | M |
| <code>kill-word</code> | M | M | M |
| <code>magic-space</code> | M | M | M |
| <code>menu-complete</code> | M | M | M |
| <code>menu-complete-backward</code> | M | M | M |
| <code>next-history</code> | M | M | M |
| <code>next-screen-line</code> | M | M | M |
| <code>non-incremental-forward-search-history</code> | M | M | M |
| <code>non-incremental-forward-search-history-again</code> | M | M | M |
| <code>non-incremental-reverse-search-history</code> | M | M | M |
| <code>non-incremental-reverse-search-history-again</code> | M | M | M |
| <code>old-menu-complete</code> | M | M | M |
| <code>operate-and-get-next</code> | M | M | M |
| <code>overwrite-mode</code> | M | M | M |
| <code>possible-command-completions</code> | M | M | M |
| <code>possible-completions</code> | M | M | M |
| <code>possible-filename-completions</code> | M | M | M |
| <code>possible-hostname-completions</code> | M | M | M |
| <code>possible-username-completions</code> | M | M | M |
| <code>possible-variable-completions</code> | M | M | M |
| <code>previous-history</code> | M | M | M |
| <code>previous-screen-line</code> | M | M | M |
| <code>print-last-kbd-macro</code> | M | M | M |
| <code>quoted-insert</code> | M | M | M |
| <code>re-read-init-file</code> | M | M | M |
| <code>redraw-current-line</code> | M | M | M |
| <code>reverse-search-history</code> | M | M | M |
| <code>revert-line</code> | M | M | M |
| <code>self-insert</code> | M | M | M |
| <code>set-mark</code> | M | M | M |
| <code>shell-backward-kill-word</code> | M | M | M |
| <code>shell-backward-word</code> | M | M | M |
| <code>shell-expand-line</code> | M | M | M |
| <code>shell-forward-word</code> | M | M | M |
| <code>shell-kill-word</code> | M | M | M |
| <code>shell-transpose-words</code> | M | M | M |
| <code>skip-csi-sequence</code> | M | M | M |
| <code>spell-correct-word</code> | M | M | M |
| <code>start-kbd-macro</code> | M | M | M |
| <code>tab-insert</code> | M | M | M |
| <code>tilde-expand</code> | M | M | M |
| <code>transpose-chars</code> | M | M | M |
| <code>transpose-words</code> | M | M | M |
| <code>tty-status</code> | M | M | M |
| <code>undo</code> | M | M | M |
| <code>universal-argument</code> | M | M | M |
| <code>unix-filename-rubout</code> | M | M | M |
| <code>unix-line-discard</code> | M | M | M |
| <code>unix-word-rubout</code> | M | M | M |
| <code>upcase-word</code> | M | M | M |
| <code>vi-append-eol</code> | M | M | M |
| <code>vi-append-mode</code> | M | M | M |
| <code>vi-arg-digit</code> | M | M | M |
| <code>vi-bWord</code> | M | M | M |
| <code>vi-back-to-indent</code> | M | M | M |
| <code>vi-backward-bigword</code> | M | M | M |
| <code>vi-backward-word</code> | M | M | M |
| <code>vi-bword</code> | M | M | M |
| <code>vi-change-case</code> | M | M | M |
| <code>vi-change-char</code> | M | M | M |
| <code>vi-change-to</code> | M | M | M |
| <code>vi-char-search</code> | M | M | M |
| <code>vi-column</code> | M | M | M |
| <code>vi-complete</code> | M | M | M |
| <code>vi-delete</code> | M | M | M |
| <code>vi-delete-to</code> | M | M | M |
| <code>vi-eWord</code> | M | M | M |
| <code>vi-edit-and-execute-command</code> | M | M | M |
| <code>vi-editing-mode</code> | M | M | M |
| <code>vi-end-bigword</code> | M | M | M |
| <code>vi-end-word</code> | M | M | M |
| <code>vi-eof-maybe</code> | M | M | M |
| <code>vi-eword</code> | M | M | M |
| <code>vi-fWord</code> | M | M | M |
| <code>vi-fetch-history</code> | M | M | M |
| <code>vi-first-print</code> | M | M | M |
| <code>vi-forward-bigword</code> | M | M | M |
| <code>vi-forward-word</code> | M | M | M |
| <code>vi-fword</code> | M | M | M |
| <code>vi-goto-mark</code> | M | M | M |
| <code>vi-insert-beg</code> | M | M | M |
| <code>vi-insertion-mode</code> | M | M | M |
| <code>vi-match</code> | M | M | M |
| <code>vi-movement-mode</code> | M | M | M |
| <code>vi-next-word</code> | M | M | M |
| <code>vi-overstrike</code> | M | M | M |
| <code>vi-overstrike-delete</code> | M | M | M |
| <code>vi-prev-word</code> | M | M | M |
| <code>vi-put</code> | M | M | M |
| <code>vi-redo</code> | M | M | M |
| <code>vi-replace</code> | M | M | M |
| <code>vi-rubout</code> | M | M | M |
| <code>vi-search</code> | M | M | M |
| <code>vi-search-again</code> | M | M | M |
| <code>vi-set-mark</code> | M | M | M |
| <code>vi-subst</code> | M | M | M |
| <code>vi-tilde-expand</code> | M | M | M |
| <code>vi-undo</code> | M | M | M |
| <code>vi-unix-word-rubout</code> | M | M | M |
| <code>vi-yank-arg</code> | M | M | M |
| <code>vi-yank-pop</code> | M | M | M |
| <code>vi-yank-to</code> | M | M | M |
| <code>yank</code> | M | M | M |
| <code>yank-last-arg</code> | M | M | M |
| <code>yank-nth-arg</code> | M | M | M |
| <code>yank-pop</code> | M | M | M |

## Readline variable

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>bell-style</code> | M | M | M |
| <code>bind-tty-special-chars</code> | M | M | M |
| <code>blink-matching-paren</code> | M | M | M |
| <code>byte-oriented</code> | M | M | M |
| <code>colored-completion-prefix</code> | M | M | M |
| <code>colored-stats</code> | M | M | M |
| <code>comment-begin</code> | M | M | M |
| <code>completion-display-width</code> | M | M | M |
| <code>completion-ignore-case</code> | M | M | M |
| <code>completion-map-case</code> | M | M | M |
| <code>completion-prefix-display-length</code> | M | M | M |
| <code>completion-query-items</code> | M | M | M |
| <code>convert-meta</code> | M | M | M |
| <code>disable-completion</code> | M | M | M |
| <code>echo-control-characters</code> | M | M | M |
| <code>editing-mode</code> | M | M | M |
| <code>emacs-mode-string</code> | M | M | M |
| <code>enable-active-region</code> | M | M | M |
| <code>enable-bracketed-paste</code> | M | M | M |
| <code>enable-keypad</code> | M | M | M |
| <code>enable-meta-key</code> | M | M | M |
| <code>expand-tilde</code> | M | M | M |
| <code>force-meta-prefix</code> | — | M | — |
| <code>history-preserve-point</code> | M | M | M |
| <code>history-size</code> | M | M | M |
| <code>horizontal-scroll-mode</code> | M | M | M |
| <code>input-meta</code> | M | M | M |
| <code>keymap</code> | M | M | M |
| <code>keyseq-timeout</code> | M | M | M |
| <code>mark-directories</code> | M | M | M |
| <code>mark-modified-lines</code> | M | M | M |
| <code>mark-symlinked-directories</code> | M | M | M |
| <code>match-hidden-files</code> | M | M | M |
| <code>menu-complete-display-prefix</code> | M | M | M |
| <code>meta-flag</code> | M | M | M |
| <code>output-meta</code> | M | M | M |
| <code>page-completions</code> | M | M | M |
| <code>prefer-visible-bell</code> | M | M | M |
| <code>print-completions-horizontally</code> | M | M | M |
| <code>revert-all-at-newline</code> | M | M | M |
| <code>search-ignore-case</code> | — | M | — |
| <code>show-all-if-ambiguous</code> | M | M | M |
| <code>show-all-if-unmodified</code> | M | M | M |
| <code>show-mode-in-prompt</code> | M | M | M |
| <code>skip-completed-text</code> | M | M | M |
| <code>vi-cmd-mode-string</code> | M | M | M |
| <code>vi-ins-mode-string</code> | M | M | M |
| <code>visible-stats</code> | M | M | M |

## Redirection

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>&amp;&gt;</code> | ✓ **0.72** / **0.85** | ✓ **0.84** / **0.93** | ✓ **0.74** / **0.92** |
| <code>&amp;&gt;&gt;</code> | ✓ **0.73** / **0.85** | ✓ **0.85** / **0.92** | ✓ **0.77** / **0.89** |
| <code>/dev/fd/N</code> | ✓ **0.68** / **0.93** | ✓ **0.85** / **0.98** | ✓ 0.60 / **0.89** |
| <code>/dev/tcp/host/port</code> | Δ | Δ | Δ |
| <code>/dev/udp/host/port</code> | Δ | Δ | Δ |
| <code>&lt;</code> | ✓ 0.50 / 0.52 | ✓ **0.74** / **0.80** | ✓ 0.56 / 0.62 |
| <code>&lt;&lt;-</code> | ✓ 0.50 / 0.53 | ✓ **0.72** / **0.81** | ✓ 0.58 / 0.61 |
| <code>&lt;&lt;&lt;</code> | ✓ 0.50 / 0.52 | ✓ **0.73** / **0.81** | ✓ 0.57 / **0.70** |
| <code>&lt;&gt;</code> | Δ | Δ | Δ |
| <code>&gt;</code> | ✓ **0.70** / **0.90** | ✓ **0.88** / **0.98** | ✓ 0.66 / **0.85** |
| <code>&gt;&gt;</code> | ✓ **0.69** / **0.90** | ✓ **0.87** / **0.99** | ✓ 0.63 / **0.82** |
| <code>&gt;&#124;</code> | Δ | Δ | Δ |
| <code>here-document</code> | ✓ 0.51 / 0.54 | ✓ **0.75** / **0.81** | ✓ 0.61 / **0.68** |
| <code>n&lt;&amp;-</code> | ✓ 0.61 / **0.84** | ✓ 0.61 / **0.87** | ✓ 0.57 / **0.71** |
| <code>n&lt;&amp;m</code> | ✓ 0.52 / 0.55 | ✓ **0.74** / **0.82** | ✓ 0.57 / **0.68** |
| <code>n&lt;&amp;m-</code> | Δ | Δ | Δ |
| <code>n&gt;&amp;-</code> | ✓ 0.63 / **0.80** | ✓ 0.64 / **0.77** | ✓ 0.57 / **0.70** |
| <code>n&gt;&amp;m</code> | ✓ **0.70** / **0.95** | ✓ **0.87** / **0.97** | ✓ 0.62 / **0.91** |
| <code>n&gt;&amp;m-</code> | Δ | Δ | Δ |
| <code>quoted here-document</code> | ✓ 0.53 / 0.56 | ✓ **0.75** / **0.83** | ✓ 0.58 / 0.62 |
| <code>{varname} descriptor</code> | Δ | Δ | Δ |

## Reserved word

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>!</code> | ✓ 0.59 / **1.03↑** | ✓ 0.60 / **1.01↑** | ✓ 0.57 / **0.71** |
| <code>[[</code> | ✓ 0.61 / **1.01↑** | ✓ 0.62 / **1.02↑** | ✓ 0.57 / **0.71** |
| <code>]]</code> | ✓ 0.61 / **1.01↑** | ✓ 0.62 / **1.02↑** | ✓ 0.57 / **0.71** |
| <code>case</code> | ✓ 0.62 / **1.03↑** | ✓ 0.62 / **1.10↑** | ✓ 0.57 / **0.72** |
| <code>coproc</code> | Δ | Δ | Δ |
| <code>do</code> | ✓ 0.60 / **0.89** | ✓ 0.60 / **0.90** | ✓ 0.58 / **0.67** |
| <code>done</code> | ✓ 0.60 / **0.89** | ✓ 0.60 / **0.90** | ✓ 0.58 / **0.67** |
| <code>elif</code> | ✓ 0.62 / **1.06↑** | ✓ 0.61 / **1.06↑** | ✓ 0.56 / **0.70** |
| <code>else</code> | ✓ 0.62 / **1.06↑** | ✓ 0.61 / **1.06↑** | ✓ 0.56 / **0.70** |
| <code>esac</code> | ✓ 0.62 / **1.03↑** | ✓ 0.62 / **1.10↑** | ✓ 0.57 / **0.72** |
| <code>fi</code> | ✓ 0.62 / **1.06↑** | ✓ 0.61 / **1.06↑** | ✓ 0.56 / **0.70** |
| <code>for</code> | ✓ 0.60 / **0.89** | ✓ 0.60 / **0.90** | ✓ 0.58 / **0.67** |
| <code>function</code> | ✓ 0.60 / **0.85** | ✓ 0.61 / **0.85** | ✓ 0.56 / **0.70** |
| <code>if</code> | ✓ 0.62 / **1.06↑** | ✓ 0.61 / **1.06↑** | ✓ 0.56 / **0.70** |
| <code>in</code> | ✓ 0.60 / **0.89** | ✓ 0.60 / **0.90** | ✓ 0.58 / **0.67** |
| <code>select</code> | Δ | Δ | Δ |
| <code>then</code> | ✓ 0.62 / **1.06↑** | ✓ 0.61 / **1.06↑** | ✓ 0.56 / **0.70** |
| <code>time</code> | Δ | Δ | Δ |
| <code>until</code> | ✓ 0.59 / **0.86** | ✓ 0.60 / **0.89** | ✓ 0.56 / **0.68** |
| <code>while</code> | ✓ 0.59 / **0.83** | ✓ 0.59 / **0.88** | ✓ 0.56 / **0.70** |
| <code>{</code> | ✓ 0.62 / **1.07↑** | ✓ 0.62 / **1.04↑** | ✓ 0.55 / **0.71** |
| <code>}</code> | ✓ 0.62 / **1.07↑** | ✓ 0.62 / **1.04↑** | ✓ 0.55 / **0.71** |

## Shell language

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>ANSI-C quoting</code> | ✓ 0.61 / **0.95** | ✓ 0.59 / **0.92** | ✓ 0.55 / **0.72** |
| <code>Bash startup via BASH_ENV</code> | ✓ 0.54 / 0.57 | ✓ 0.54 / 0.55 | ✓ 0.55 / 0.50 |
| <code>DEBUG trap</code> | Δ | Δ | Δ |
| <code>ERR trap</code> | Δ | Δ | Δ |
| <code>EXIT trap</code> | ✓ 0.62 / **0.86** | ✓ 0.60 / **0.83** | ✓ 0.57 / **0.71** |
| <code>IFS word splitting</code> | ✓ 0.60 / **0.90** | ✓ 0.60 / **0.92** | ✓ 0.56 / **0.68** |
| <code>PATH command search</code> | ✓ **0.75** / **0.82** | ✓ **0.87** / **0.92** | ✓ **1.01↑** / **1.42↑** |
| <code>POSIX mode</code> | Δ | Δ | Δ |
| <code>POSIX special builtin failure</code> | Δ | Δ | Δ |
| <code>REPLY substitution (5.3)</code> | N | Δ | N |
| <code>RETURN trap</code> | Δ | Δ | Δ |
| <code>alias parse time</code> | ✓ 0.60 / **0.81** | ✓ 0.58 / **0.81** | ✓ 0.54 / 0.66 |
| <code>arithmetic base notation</code> | ✓ 0.63 / **1.10↑** | ✓ 0.61 / **1.07↑** | ✓ 0.59 / **0.70** |
| <code>arithmetic command status</code> | ✓ 0.59 / **0.87** | ✓ 0.59 / **0.91** | ✓ 0.58 / **0.80** |
| <code>arithmetic expansion</code> | ✓ 0.60 / **1.03↑** | ✓ 0.61 / **1.09↑** | ✓ 0.57 / **0.69** |
| <code>arithmetic for</code> | ✓ 0.58 / **0.89** | ✓ 0.58 / **0.90** | ✓ 0.59 / **0.77** |
| <code>arithmetic integer division and remainder</code> | ✓ 0.61 / **1.04↑** | ✓ 0.61 / **1.06↑** | ✓ 0.56 / **0.71** |
| <code>arithmetic short circuit</code> | ✓ 0.62 / **0.86** | ✓ 0.62 / **0.91** | ✓ 0.57 / **0.78** |
| <code>array append</code> | ✓ 0.61 / **0.81** | ✓ 0.59 / **0.83** | ✓ 0.60 / **0.72** |
| <code>array slice and negative index</code> | ✓ 0.61 / **0.83** | ✓ 0.60 / **0.80** | ✓ 0.59 / **0.73** |
| <code>associative arrays</code> | Δ | Δ | Δ |
| <code>background and wait</code> | ✓ **0.78** / **1.05↑** | ✓ **0.75** / **1.07↑** | ✓ 0.64 / **1.17↑** |
| <code>backquote substitution</code> | ✓ 0.41 / 0.36 | ✓ 0.40 / 0.36 | ✓ 0.52 / 0.30 |
| <code>brace alternation</code> | Δ | Δ | Δ |
| <code>brace group</code> | ✓ 0.62 / **1.07↑** | ✓ 0.62 / **1.04↑** | ✓ 0.55 / **0.71** |
| <code>brace sequence</code> | Δ | Δ | Δ |
| <code>bracket expressions and classes</code> | ✓ **0.68** / **0.81** | ✓ **0.83** / **0.90** | ✓ 0.52 / 0.55 |
| <code>case ;&amp; fallthrough</code> | Δ | Δ | Δ |
| <code>case ;;&amp; retest</code> | Δ | Δ | Δ |
| <code>case in esac</code> | ✓ 0.62 / **1.03↑** | ✓ 0.62 / **1.10↑** | ✓ 0.57 / **0.72** |
| <code>case patterns from variables</code> | ✓ 0.63 / **1.04↑** | ✓ 0.63 / **1.04↑** | ✓ 0.55 / **0.69** |
| <code>command substitution</code> | ✓ 0.40 / 0.34 | ✓ 0.41 / 0.34 | ✓ 0.51 / 0.32 |
| <code>comments</code> | ✓ 0.59 / **0.97** | ✓ 0.59 / **0.85** | ✓ 0.55 / **0.77** |
| <code>completion filters</code> | ✓ 0.61 / **0.85** | ✓ 0.62 / **0.84** | ✓ 0.56 / **0.69** |
| <code>conditional command</code> | ✓ 0.61 / **1.01↑** | ✓ 0.62 / **1.02↑** | ✓ 0.57 / **0.71** |
| <code>coproc</code> | Δ | Δ | Δ |
| <code>coprocess input/output descriptors</code> | Δ | Δ | Δ |
| <code>current environment substitution (5.3)</code> | N | Δ | N |
| <code>directory cd -</code> | ✓ **0.71** / **0.81** | ✓ **0.82** / **0.91** | ✓ 0.62 / **0.68** |
| <code>directory stack rotation</code> | Δ | Δ | Δ |
| <code>double quotes</code> | ✓ 0.61 / **0.99** | ✓ 0.60 / **1.04↑** | ✓ 0.55 / **0.73** |
| <code>dynamic function scope</code> | ✓ 0.60 / **0.76** | ✓ 0.60 / **0.77** | ✓ 0.58 / **0.72** |
| <code>escape character</code> | ✓ 0.56 / **1.01↑** | ✓ 0.60 / **1.08↑** | ✓ 0.56 / **0.75** |
| <code>escape sequences in printf %b</code> | ✓ 0.60 / **0.93** | ✓ 0.58 / **0.92** | ✓ 0.57 / **0.72** |
| <code>escaped backslash filenames</code> | ✓ **0.67** / **0.82** | ✓ **0.81** / **0.90** | ✓ 0.55 / 0.65 |
| <code>exit status 126</code> | ✓ 0.52 / 0.55 | ✓ **0.72** / **0.77** | ✓ 0.58 / 0.51 |
| <code>exit status 127</code> | ✓ 0.40 / 0.35 | ✓ 0.41 / 0.36 | ✓ 0.52 / 0.40 |
| <code>extended globs</code> | Δ | Δ | Δ |
| <code>extglob !(...)</code> | Δ | Δ | Δ |
| <code>extglob *(...)</code> | Δ | Δ | Δ |
| <code>extglob +(...)</code> | Δ | Δ | Δ |
| <code>extglob ?(...)</code> | Δ | Δ | Δ |
| <code>extglob @(...)</code> | Δ | Δ | Δ |
| <code>filename wildcards</code> | ✓ **0.67** / **0.84** | ✓ **0.81** / **0.88** | ✓ 0.58 / 0.56 |
| <code>for do done</code> | ✓ 0.60 / **0.89** | ✓ 0.60 / **0.90** | ✓ 0.58 / **0.67** |
| <code>function and return</code> | ✓ 0.60 / **0.85** | ✓ 0.61 / **0.85** | ✓ 0.56 / **0.70** |
| <code>function argument restoration</code> | ✓ 0.60 / **0.81** | ✓ 0.61 / **0.78** | ✓ 0.56 / **0.67** |
| <code>function definition redirections</code> | ✓ **0.69** / **0.94** | ✓ **0.85** / **0.96** | ✓ 0.62 / **0.80** |
| <code>function export</code> | Δ | Δ | Δ |
| <code>group redirections</code> | ✓ **0.70** / **0.89** | ✓ **0.85** / **1.00** | ✓ 0.66 / **0.88** |
| <code>grouped pipeline status</code> | ✓ 0.64 / **0.73** | ✓ **0.79** / **0.87** | ✓ 0.59 / **0.74** |
| <code>history event expansion</code> | Δ | Δ | Δ |
| <code>history modifiers</code> | Δ | Δ | Δ |
| <code>history search</code> | Δ | Δ | Δ |
| <code>history word designator</code> | Δ | Δ | Δ |
| <code>if then elif else fi</code> | ✓ 0.62 / **1.06↑** | ✓ 0.61 / **1.06↑** | ✓ 0.56 / **0.70** |
| <code>indexed arrays</code> | ✓ 0.62 / **0.77** | ✓ 0.61 / **0.79** | ✓ 0.57 / **0.71** |
| <code>line continuation</code> | ✓ 0.62 / **0.98** | ✓ 0.57 / **0.93** | ✓ 0.59 / **0.72** |
| <code>lists and short-circuit</code> | ✓ 0.60 / **0.84** | ✓ 0.60 / **0.92** | ✓ 0.57 / **0.69** |
| <code>local variable inheritance</code> | Δ | Δ | Δ |
| <code>locale translation syntax</code> | ✓ 0.61 / **0.94** | ✓ 0.59 / **0.87** | ✓ 0.56 / **0.72** |
| <code>merged stderr pipeline</code> | ✓ 0.66 / **0.84** | ✓ **0.82** / **0.93** | ✓ 0.65 / **0.81** |
| <code>nameref</code> | Δ | Δ | Δ |
| <code>negation</code> | ✓ 0.59 / **1.03↑** | ✓ 0.60 / **1.01↑** | ✓ 0.57 / **0.71** |
| <code>pathname expansion of leading dots</code> | ✓ **0.68** / **0.82** | ✓ **0.81** / **0.93** | ✓ 0.58 / 0.58 |
| <code>pipeline</code> | ✓ **0.71** / **0.85** | ✓ **0.80** / **0.88** | ✓ 0.57 / **0.72** |
| <code>pipeline stage statuses</code> | ✓ 0.58 / 0.65 | ✓ 0.58 / 0.65 | ✓ 0.60 / **0.78** |
| <code>primary prompt parameter expansion</code> | ✓ 0.60 / **1.04↑** | ✓ 0.60 / **0.97** | ✓ 0.57 / **0.76** |
| <code>printf %q round trip</code> | ✓ 0.61 / **0.79** | ✓ 0.62 / **0.75** | ✓ 0.58 / **0.67** |
| <code>printf reusable quoting</code> | ✓ 0.61 / **0.79** | ✓ 0.62 / **0.75** | ✓ 0.58 / **0.67** |
| <code>printf timestamp</code> | Δ | Δ | Δ |
| <code>priorities of lookup</code> | ✓ 0.58 / **0.79** | ✓ 0.59 / **0.85** | ✓ 0.61 / **0.73** |
| <code>process substitution input</code> | ✓ 0.66 / **0.86** | ✓ **0.83** / **0.94** | ✓ 0.63 / **0.81** |
| <code>process substitution output</code> | ✓ **0.72** / **0.84** | ✓ **0.85** / **0.91** | ✓ **0.70** / **0.85** |
| <code>programmable completion callback</code> | ✓ 0.62 / **0.73** | ✓ 0.61 / **0.74** | ✓ 0.56 / **0.68** |
| <code>quote removal</code> | ✓ 0.58 / **0.97** | ✓ 0.59 / **0.94** | ✓ 0.58 / **0.75** |
| <code>quoted and unquoted here-documents</code> | ✓ 0.53 / 0.56 | ✓ **0.75** / **0.83** | ✓ 0.58 / 0.62 |
| <code>restricted shell</code> | E | E | E |
| <code>script fallback without shebang</code> | Δ | Δ | Δ |
| <code>select</code> | Δ | Δ | Δ |
| <code>shell environment isolation</code> | ✓ 0.64 / **0.78** | ✓ 0.65 / **0.77** | ✓ 0.62 / **0.77** |
| <code>shell script source</code> | ✓ 0.64 / **0.80** | ✓ 0.62 / **0.78** | ✓ 0.57 / **0.74** |
| <code>signal exit status</code> | E | E | E |
| <code>signal trap</code> | ✓ 0.59 / **0.79** | ✓ 0.59 / **0.82** | ✓ 0.58 / **0.73** |
| <code>simple commands and assignments</code> | ✓ **0.67** / **0.82** | ✓ **0.69** / **0.81** | ✓ 0.66 / **0.80** |
| <code>single quotes</code> | ✓ 0.60 / **0.94** | ✓ 0.59 / **0.91** | ✓ 0.54 / **0.67** |
| <code>subshell exit isolation</code> | ✓ 0.58 / **0.76** | ✓ 0.61 / **0.71** | ✓ 0.58 / **0.82** |
| <code>subshell scope</code> | ✓ 0.65 / **0.80** | ✓ 0.66 / **0.81** | ✓ 0.62 / **0.78** |
| <code>terminal signal ignore and reset</code> | ✓ 0.62 / **0.93** | ✓ 0.61 / **0.88** | ✓ 0.58 / **0.69** |
| <code>tilde current and previous directories</code> | ✓ **0.70** / **0.82** | ✓ **0.81** / **0.91** | ✓ 0.60 / **0.72** |
| <code>tilde expansion</code> | ✓ 0.61 / **1.03↑** | ✓ 0.59 / **1.03↑** | ✓ 0.56 / **0.75** |
| <code>tilde named user</code> | Δ | Δ | Δ |
| <code>time -p</code> | Δ | Δ | Δ |
| <code>time keyword</code> | Δ | Δ | Δ |
| <code>until</code> | ✓ 0.59 / **0.86** | ✓ 0.60 / **0.89** | ✓ 0.56 / **0.68** |
| <code>wait -n</code> | Δ | Δ | Δ |
| <code>wait -p</code> | Δ | Δ | Δ |
| <code>while</code> | ✓ 0.59 / **0.83** | ✓ 0.59 / **0.88** | ✓ 0.56 / **0.70** |

## Signal

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>ABRT</code> | ✓ 0.61 / **0.80** | ✓ 0.59 / **0.80** | ✓ 0.58 / **0.71** |
| <code>ALRM</code> | ✓ 0.60 / **0.81** | ✓ 0.59 / **0.80** | ✓ 0.57 / **0.68** |
| <code>BUS</code> | ✓ 0.60 / **0.79** | ✓ 0.59 / **0.78** | ✓ 0.55 / **0.76** |
| <code>CHLD</code> | Δ | Δ | Δ |
| <code>CONT</code> | ✓ 0.59 / **0.78** | ✓ 0.58 / **0.80** | ✓ 0.57 / **0.71** |
| <code>EMT</code> | — | — | Δ |
| <code>FPE</code> | ✓ 0.59 / **0.77** | ✓ 0.60 / **0.79** | ✓ 0.56 / **0.71** |
| <code>HUP</code> | ✓ 0.60 / **0.79** | ✓ 0.58 / **0.78** | ✓ 0.56 / **0.73** |
| <code>ILL</code> | ✓ 0.59 / **0.80** | ✓ 0.58 / **0.79** | ✓ 0.59 / **0.72** |
| <code>INFO</code> | — | — | Δ |
| <code>INT</code> | ✓ 0.60 / **0.80** | ✓ 0.59 / **0.81** | ✓ 0.57 / **0.70** |
| <code>IO</code> | ✓ 0.60 / **0.76** | ✓ 0.59 / **0.77** | ✓ 0.56 / **0.71** |
| <code>KILL</code> | I 0.60 / **0.77** | I 0.59 / **0.71** | I 0.55 / **0.73** |
| <code>PIPE</code> | ✓ 0.59 / **0.75** | ✓ 0.60 / **0.79** | ✓ 0.57 / **0.67** |
| <code>PROF</code> | ✓ 0.60 / **0.79** | ✓ 0.59 / **0.78** | ✓ 0.54 / **0.69** |
| <code>PWR</code> | Δ | Δ | — |
| <code>QUIT</code> | ✓ 0.59 / **0.76** | ✓ 0.59 / **0.78** | ✓ 0.56 / **0.76** |
| <code>RTMAX</code> | Δ | Δ | — |
| <code>RTMAX-1</code> | Δ | Δ | — |
| <code>RTMAX-10</code> | Δ | Δ | — |
| <code>RTMAX-11</code> | Δ | Δ | — |
| <code>RTMAX-12</code> | Δ | Δ | — |
| <code>RTMAX-13</code> | Δ | Δ | — |
| <code>RTMAX-14</code> | Δ | Δ | — |
| <code>RTMAX-2</code> | Δ | Δ | — |
| <code>RTMAX-3</code> | Δ | Δ | — |
| <code>RTMAX-4</code> | Δ | Δ | — |
| <code>RTMAX-5</code> | Δ | Δ | — |
| <code>RTMAX-6</code> | Δ | Δ | — |
| <code>RTMAX-7</code> | Δ | Δ | — |
| <code>RTMAX-8</code> | Δ | Δ | — |
| <code>RTMAX-9</code> | Δ | Δ | — |
| <code>RTMIN</code> | Δ | Δ | — |
| <code>RTMIN+1</code> | Δ | Δ | — |
| <code>RTMIN+10</code> | Δ | Δ | — |
| <code>RTMIN+11</code> | Δ | Δ | — |
| <code>RTMIN+12</code> | Δ | Δ | — |
| <code>RTMIN+13</code> | Δ | Δ | — |
| <code>RTMIN+14</code> | Δ | Δ | — |
| <code>RTMIN+15</code> | Δ | Δ | — |
| <code>RTMIN+2</code> | Δ | Δ | — |
| <code>RTMIN+3</code> | Δ | Δ | — |
| <code>RTMIN+4</code> | Δ | Δ | — |
| <code>RTMIN+5</code> | Δ | Δ | — |
| <code>RTMIN+6</code> | Δ | Δ | — |
| <code>RTMIN+7</code> | Δ | Δ | — |
| <code>RTMIN+8</code> | Δ | Δ | — |
| <code>RTMIN+9</code> | Δ | Δ | — |
| <code>SEGV</code> | ✓ 0.60 / **0.77** | ✓ 0.59 / **0.78** | ✓ 0.57 / **0.69** |
| <code>STKFLT</code> | Δ | Δ | — |
| <code>STOP</code> | I 0.58 / **0.75** | I 0.59 / **0.73** | I 0.55 / **0.69** |
| <code>SYS</code> | ✓ 0.59 / **0.77** | ✓ 0.58 / **0.80** | ✓ 0.58 / **0.71** |
| <code>TERM</code> | ✓ 0.59 / **0.81** | ✓ 0.60 / **0.79** | ✓ 0.57 / **0.74** |
| <code>TRAP</code> | ✓ 0.59 / **0.80** | ✓ 0.60 / **0.79** | ✓ 0.56 / **0.67** |
| <code>TSTP</code> | ✓ 0.59 / **0.79** | ✓ 0.59 / **0.79** | ✓ 0.55 / **0.71** |
| <code>TTIN</code> | ✓ 0.59 / **0.77** | ✓ 0.59 / **0.81** | ✓ 0.57 / **0.69** |
| <code>TTOU</code> | ✓ 0.57 / **0.84** | ✓ 0.59 / **0.79** | ✓ 0.58 / **0.71** |
| <code>URG</code> | ✓ 0.57 / **0.87** | ✓ 0.60 / **0.77** | ✓ 0.52 / **0.70** |
| <code>USR1</code> | ✓ 0.60 / **0.78** | ✓ 0.59 / **0.77** | ✓ 0.56 / **0.74** |
| <code>USR2</code> | ✓ 0.60 / **0.80** | ✓ 0.60 / **0.78** | ✓ 0.58 / **0.67** |
| <code>VTALRM</code> | ✓ 0.58 / **0.76** | ✓ 0.58 / **0.78** | ✓ 0.55 / **0.75** |
| <code>WINCH</code> | ✓ 0.60 / **0.78** | ✓ 0.60 / **0.80** | ✓ 0.57 / **0.71** |
| <code>XCPU</code> | ✓ 0.58 / **0.77** | ✓ 0.61 / **0.78** | ✓ 0.56 / 0.64 |
| <code>XFSZ</code> | ✓ 0.59 / **0.75** | ✓ 0.60 / **0.78** | ✓ 0.58 / **0.74** |

## Terminal

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>foreground pipeline</code> | ✓ **1.02↑** | ✓ **0.93** | ✓ **0.87** |
| <code>interactive alias and signal</code> | ✓ **0.91** | ✓ **0.89** | ✓ **0.75** |

## Variable

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>BASH</code> | ✓ 0.52 / 0.53 | ✓ 0.51 / 0.51 | ✓ **0.74** / **1.06↑** |
| <code>BASHOPTS</code> | Δ | Δ | Δ |
| <code>BASHPID</code> | ✓ 0.64 / **0.77** | ✓ 0.64 / **0.76** | ✓ 0.56 / **0.76** |
| <code>BASH_ALIASES</code> | Δ | Δ | Δ |
| <code>BASH_ARGC</code> | Δ | Δ | Δ |
| <code>BASH_ARGV</code> | Δ | Δ | Δ |
| <code>BASH_ARGV0</code> | Δ | Δ | Δ |
| <code>BASH_CMDS</code> | Δ | Δ | Δ |
| <code>BASH_COMMAND</code> | Δ | Δ | Δ |
| <code>BASH_COMPAT</code> | Δ | Δ | Δ |
| <code>BASH_ENV</code> | ✓ 0.54 / 0.57 | ✓ 0.54 / 0.55 | ✓ 0.55 / 0.50 |
| <code>BASH_EXECUTION_STRING</code> | Δ | Δ | Δ |
| <code>BASH_LINENO</code> | Δ | Δ | Δ |
| <code>BASH_LOADABLES_PATH</code> | V | V | V |
| <code>BASH_MONOSECONDS</code> | — | Δ | — |
| <code>BASH_REMATCH</code> | ✓ 0.62 / **0.84** | ✓ 0.61 / **0.78** | ✓ 0.56 / **0.78** |
| <code>BASH_SOURCE</code> | ✓ 0.59 / **0.74** | ✓ 0.59 / **0.75** | ✓ 0.57 / **0.82** |
| <code>BASH_SUBSHELL</code> | Δ | Δ | Δ |
| <code>BASH_TRAPSIG</code> | — | Δ | — |
| <code>BASH_VERSINFO</code> | Δ | Δ | Δ |
| <code>BASH_VERSION</code> | Δ | Δ | Δ |
| <code>BASH_XTRACEFD</code> | Δ | Δ | Δ |
| <code>CDPATH</code> | ✓ **0.75** / **0.85** | ✓ **0.88** / **0.95** | ✓ 0.62 / **0.78** |
| <code>CHILD_MAX</code> | ✓ **0.84** / **0.85** | ✓ **0.87** / **0.88** | ✓ **0.82** / **0.93** |
| <code>COLUMNS</code> | ✓ **0.86** | ✓ **0.86** | ✓ **0.67** |
| <code>COMPREPLY</code> | ✓ 0.62 / **0.73** | ✓ 0.61 / **0.74** | ✓ 0.56 / **0.68** |
| <code>COMP_CWORD</code> | ✓ **1.18↑** | ✓ **1.16↑** | ✓ **0.70** |
| <code>COMP_KEY</code> | ✓ **1.14↑** | ✓ **1.14↑** | ✓ **0.69** |
| <code>COMP_LINE</code> | ✓ **1.14↑** | ✓ **1.16↑** | ✓ **0.70** |
| <code>COMP_POINT</code> | ✓ **1.15↑** | ✓ **1.17↑** | ✓ **0.69** |
| <code>COMP_TYPE</code> | ✓ **1.14↑** | ✓ **1.16↑** | ✓ **0.70** |
| <code>COMP_WORDBREAKS</code> | V | V | V |
| <code>COMP_WORDS</code> | ✓ **1.13↑** | ✓ **1.17↑** | ✓ **0.70** |
| <code>COPROC</code> | Δ | Δ | Δ |
| <code>DIRSTACK</code> | Δ | Δ | Δ |
| <code>EMACS</code> | Δ | Δ | Δ |
| <code>ENV</code> | Δ | Δ | Δ |
| <code>EPOCHREALTIME</code> | Δ | Δ | Δ |
| <code>EPOCHSECONDS</code> | Δ | Δ | Δ |
| <code>EUID</code> | Δ | Δ | Δ |
| <code>EXECIGNORE</code> | Δ | Δ | Δ |
| <code>FCEDIT</code> | Δ | Δ | Δ |
| <code>FIGNORE</code> | Δ | Δ | Δ |
| <code>FUNCNAME</code> | ✓ 0.59 / **0.74** | ✓ 0.61 / **0.79** | ✓ 0.57 / **0.71** |
| <code>FUNCNEST</code> | Δ | Δ | Δ |
| <code>GLOBIGNORE</code> | Δ | Δ | Δ |
| <code>GLOBSORT</code> | — | Δ | — |
| <code>GROUPS</code> | Δ | Δ | Δ |
| <code>HISTCMD</code> | V | V | V |
| <code>HISTCONTROL</code> | Δ | Δ | Δ |
| <code>HISTFILE</code> | Δ | Δ | Δ |
| <code>HISTFILESIZE</code> | Δ | Δ | Δ |
| <code>HISTIGNORE</code> | Δ | Δ | Δ |
| <code>HISTSIZE</code> | Δ | Δ | Δ |
| <code>HISTTIMEFORMAT</code> | Δ | Δ | Δ |
| <code>HOME</code> | ✓ **0.74** / **0.87** | ✓ **0.89** / **0.95** | ✓ 0.62 / **0.77** |
| <code>HOSTFILE</code> | Δ | Δ | Δ |
| <code>HOSTNAME</code> | V | V | V |
| <code>HOSTTYPE</code> | V | V | V |
| <code>IFS</code> | ✓ 0.60 / **0.90** | ✓ 0.60 / **0.92** | ✓ 0.56 / **0.68** |
| <code>IGNOREEOF</code> | Δ | Δ | Δ |
| <code>INPUTRC</code> | I | I | I |
| <code>INSIDE_EMACS</code> | Δ | Δ | Δ |
| <code>LANG</code> | ✓ 0.12 / 0.08 | ✓ 0.12 / 0.08 | ✓ 0.50 / 0.46 |
| <code>LC_ALL</code> | ✓ 0.12 / 0.08 | ✓ 0.12 / 0.07 | ✓ 0.52 / 0.47 |
| <code>LC_COLLATE</code> | Δ | Δ | ✓ 0.53 / 0.52 |
| <code>LC_CTYPE</code> | ✓ 0.25 / 0.20 | ✓ 0.26 / 0.19 | ✓ 0.49 / 0.46 |
| <code>LC_MESSAGES</code> | Δ | Δ | Δ |
| <code>LC_NUMERIC</code> | Δ | Δ | Δ |
| <code>LC_TIME</code> | Δ | Δ | Δ |
| <code>LINENO</code> | Δ | Δ | Δ |
| <code>LINES</code> | ✓ **0.84** | ✓ **0.83** | ✓ **0.67** |
| <code>MACHTYPE</code> | V | V | V |
| <code>MAIL</code> | Δ | Δ | Δ |
| <code>MAILCHECK</code> | Δ | Δ | Δ |
| <code>MAILPATH</code> | Δ | Δ | Δ |
| <code>MAPFILE</code> | Δ | Δ | Δ |
| <code>OLDPWD</code> | ✓ **0.71** / **0.87** | ✓ **0.82** / **0.93** | ✓ 0.57 / **0.71** |
| <code>OPTARG</code> | Δ | Δ | Δ |
| <code>OPTERR</code> | Δ | Δ | Δ |
| <code>OPTIND</code> | Δ | Δ | Δ |
| <code>OSTYPE</code> | V | V | V |
| <code>PATH</code> | ✓ **0.75** / **0.82** | ✓ **0.87** / **0.92** | ✓ **1.01↑** / **1.42↑** |
| <code>PIPESTATUS</code> | ✓ 0.58 / 0.65 | ✓ 0.58 / 0.65 | ✓ 0.60 / **0.78** |
| <code>POSIXLY_CORRECT</code> | Δ | Δ | Δ |
| <code>PPID</code> | Δ | Δ | Δ |
| <code>PROMPT_COMMAND</code> | ✓ **0.84** | ✓ **0.83** | ✓ **0.68** |
| <code>PROMPT_DIRTRIM</code> | ✓ **0.84** | ✓ **0.88** | Δ |
| <code>PS0</code> | Δ | Δ | Δ |
| <code>PS1</code> | ✓ **0.81** | ✓ **0.80** | ✓ 0.63 |
| <code>PS2</code> | ✓ **0.81** | ✓ **0.83** | ✓ 0.62 |
| <code>PS3</code> | Δ | Δ | Δ |
| <code>PS4</code> | Δ | Δ | Δ |
| <code>PWD</code> | ✓ **0.71** / **0.80** | ✓ **0.83** / **0.93** | ✓ 0.59 / **0.70** |
| <code>RANDOM</code> | ✓ 0.63 / **0.77** | ✓ 0.61 / **0.78** | ✓ 0.54 / **0.77** |
| <code>READLINE_ARGUMENT</code> | Δ | Δ | Δ |
| <code>READLINE_LINE</code> | Δ | Δ | Δ |
| <code>READLINE_MARK</code> | Δ | Δ | Δ |
| <code>READLINE_POINT</code> | Δ | Δ | Δ |
| <code>REPLY</code> | ✓ 0.60 / **0.82** | ✓ 0.61 / **0.82** | ✓ 0.58 / **0.69** |
| <code>SECONDS</code> | ✓ 0.62 / **1.08↑** | ✓ 0.61 / **1.01↑** | ✓ 0.58 / **0.73** |
| <code>SHELL</code> | ✓ 0.51 / 0.51 | ✓ 0.50 / 0.51 | ✓ 0.52 / 0.50 |
| <code>SHELLOPTS</code> | Δ | Δ | Δ |
| <code>SHLVL</code> | Δ | Δ | Δ |
| <code>SRANDOM</code> | Δ | Δ | Δ |
| <code>TIMEFORMAT</code> | Δ | Δ | Δ |
| <code>TMOUT</code> | Δ | Δ | ? |
| <code>TMPDIR</code> | I | I | I |
| <code>UID</code> | Δ | Δ | Δ |
| <code>_</code> | V | V | V |
| <code>histchars</code> | Δ | Δ | Δ |

## set -o

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>allexport</code> | Δ | Δ | Δ |
| <code>braceexpand</code> | Δ | Δ | Δ |
| <code>emacs</code> | Δ | Δ | Δ |
| <code>errexit</code> | ✓ 0.59 / **0.88** | ✓ 0.61 / **0.89** | ✓ 0.55 / **0.72** |
| <code>errtrace</code> | Δ | Δ | Δ |
| <code>functrace</code> | Δ | Δ | Δ |
| <code>hashall</code> | I 0.60 / **0.94** | I 0.60 / **1.01↑** | I 0.57 / **0.72** |
| <code>histexpand</code> | Δ | Δ | Δ |
| <code>history</code> | Δ | Δ | Δ |
| <code>ignoreeof</code> | Δ | Δ | Δ |
| <code>interactive-comments</code> | Δ | Δ | Δ |
| <code>keyword</code> | Δ | Δ | Δ |
| <code>monitor</code> | E | Δ | Δ |
| <code>noclobber</code> | Δ | Δ | Δ |
| <code>noexec</code> | Δ | Δ | Δ |
| <code>noglob</code> | ✓ **0.69** / **0.78** | ✓ **0.83** / **0.92** | ✓ 0.55 / 0.58 |
| <code>nolog</code> | Δ | Δ | Δ |
| <code>notify</code> | Δ | Δ | Δ |
| <code>nounset</code> | E | E | E |
| <code>onecmd</code> | E | E | E |
| <code>physical</code> | Δ | Δ | Δ |
| <code>pipefail</code> | ✓ 0.59 / 0.66 | ✓ 0.61 / **0.68** | ✓ 0.59 / **0.86** |
| <code>posix</code> | Δ | Δ | Δ |
| <code>privileged</code> | Δ | Δ | Δ |
| <code>verbose</code> | Δ | Δ | Δ |
| <code>vi</code> | Δ | Δ | Δ |
| <code>xtrace</code> | Δ | Δ | Δ |

## shopt

| Feature / 項目 | Debian | Ubuntu | macOS |
| --- | --- | --- | --- |
| <code>array_expand_once</code> | — | M | — |
| <code>assoc_expand_once</code> | M | M | M |
| <code>autocd</code> | M | M | M |
| <code>bash_source_fullpath</code> | — | M | — |
| <code>cdable_vars</code> | M | M | M |
| <code>cdspell</code> | M | M | M |
| <code>checkhash</code> | M | M | M |
| <code>checkjobs</code> | M | M | M |
| <code>checkwinsize</code> | I 0.59 / **0.88** | I 0.60 / **0.86** | I 0.54 / **0.72** |
| <code>cmdhist</code> | M | M | M |
| <code>compat31</code> | M | M | M |
| <code>compat32</code> | M | M | M |
| <code>compat40</code> | M | M | M |
| <code>compat41</code> | M | M | M |
| <code>compat42</code> | M | M | M |
| <code>compat43</code> | M | M | M |
| <code>compat44</code> | M | M | M |
| <code>complete_fullquote</code> | M | M | M |
| <code>direxpand</code> | M | M | M |
| <code>dirspell</code> | M | M | M |
| <code>dotglob</code> | ✓ **0.69** / **0.79** | ✓ **0.81** / **0.91** | ✓ 0.56 / 0.57 |
| <code>execfail</code> | M | M | M |
| <code>expand_aliases</code> | ✓ 0.60 / **0.81** | ✓ 0.58 / **0.81** | ✓ 0.54 / 0.66 |
| <code>extdebug</code> | M | M | M |
| <code>extglob</code> | M | M | M |
| <code>extquote</code> | M | M | M |
| <code>failglob</code> | M | M | M |
| <code>force_fignore</code> | M | M | M |
| <code>globasciiranges</code> | M | M | M |
| <code>globskipdots</code> | M | M | M |
| <code>globstar</code> | ✓ **0.71** / **0.78** | ✓ **0.85** / **0.92** | ✓ 0.61 / **0.69** |
| <code>gnu_errfmt</code> | M | M | M |
| <code>histappend</code> | M | M | M |
| <code>histreedit</code> | M | M | M |
| <code>histverify</code> | M | M | M |
| <code>hostcomplete</code> | M | M | M |
| <code>huponexit</code> | M | M | M |
| <code>inherit_errexit</code> | M | M | M |
| <code>interactive_comments</code> | M | M | M |
| <code>lastpipe</code> | M | M | M |
| <code>lithist</code> | M | M | M |
| <code>localvar_inherit</code> | M | M | M |
| <code>localvar_unset</code> | M | M | M |
| <code>login_shell</code> | M | M | M |
| <code>mailwarn</code> | M | M | M |
| <code>no_empty_cmd_completion</code> | M | M | M |
| <code>nocaseglob</code> | M | M | M |
| <code>nocasematch</code> | M | M | M |
| <code>noexpand_translation</code> | M | M | M |
| <code>nullglob</code> | ✓ 0.59 / **0.75** | ✓ 0.60 / **0.87** | ✓ 0.57 / **0.78** |
| <code>patsub_replacement</code> | ✓ 0.62 / **0.89** | ✓ 0.60 / **0.81** | ✓ 0.57 / **0.70** |
| <code>progcomp</code> | I 0.59 / **0.82** | I 0.60 / **0.95** | I 0.57 / **0.73** |
| <code>progcomp_alias</code> | M | M | M |
| <code>promptvars</code> | M | M | M |
| <code>restricted_shell</code> | M | M | M |
| <code>shift_verbose</code> | M | M | M |
| <code>sourcepath</code> | ✓ **0.68** / **0.78** | ✓ **0.74** / **0.89** | ✓ 0.58 / 0.58 |
| <code>varredir_close</code> | M | M | M |
| <code>xpg_echo</code> | M | M | M |
