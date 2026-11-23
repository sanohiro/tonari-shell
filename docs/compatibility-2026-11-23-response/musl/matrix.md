# Bash feature matrix / Bash 機能別の結果

[Method, scope and reproduction / 検証方法と対象範囲](../../compatibility.md)

[Debian TSV](Debian.tsv) · [Ubuntu TSV](Ubuntu.tsv) · [Environment](environment.json)

Each cell gives the compatibility result and the median elapsed-time ratio (Tonari/Bash), whole run followed by already-started where applicable. Exact nanoseconds, p95, sample counts and witnesses are in each platform TSV. Bold ratios exceed 2/3 before rounding. An upward arrow marks Bash time or slower. Shared witnesses appear under each public name. Manual sections are index entries.

各セルは互換判定と所要時間の中央値の比（Tonari/Bash）です。左が起動込み、右が起動後です。数値が一つなら起動込みのみです。太字は上限 2/3 を超えた値、↑ は Bash と同じか遅い値です。TSV にナノ秒単位の中央値・p95、サンプル数、検証内容を収録しました。同じ処理を参照する項目があり、マニュアルの節は索引として数えています。

✓ match / 一致 · ◎ verified Tonari contract / 意図した差 · ! Tonari contract violation / 仕様違反 · G GNU golden match · Δ difference / 動作差 · E stderr difference · M missing interface · V missing default · I interface only · P POSIX mode unavailable · F fixture unavailable · ? inconclusive · N absent from reference · T timeout · ↗ separate function-export probe · § manual index · × observation changed during timing · — outside this lane.

## Arithmetic

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>!=</code> | ✓ 0.42 / **1.03↑** | ✓ 0.54 / **0.86** |
| <code>!a</code> | ✓ 0.66 / **1.07↑** | ✓ 0.52 / **0.88** |
| <code>%</code> | ✓ 0.64 / **0.90** | ✓ 0.55 / 0.67 |
| <code>%=</code> | ✓ **0.81** / **0.92** | ✓ 0.65 / **0.88** |
| <code>&amp;</code> | ✓ 0.59 / **0.80** | ✓ 0.62 / **0.79** |
| <code>&amp;&amp;</code> | ✓ 0.62 / **0.99** | ✓ 0.66 / **0.84** |
| <code>&amp;=</code> | ✓ **0.67** / **1.04↑** | ✓ 0.49 / **0.90** |
| <code>(a+1)*2</code> | ✓ **0.68** / **0.82** | ✓ 0.62 / **0.85** |
| <code>*</code> | ✓ **0.72** / **0.90** | ✓ 0.65 / **0.71** |
| <code>**</code> | ✓ 0.57 / **0.81** | ✓ 0.67 / **0.75** |
| <code>*=</code> | ✓ 0.43 / **0.79** | ✓ 0.60 / **0.84** |
| <code>+</code> | ✓ 0.62 / 0.63 | ✓ 0.63 / **0.89** |
| <code>++a</code> | ✓ 0.59 / **0.93** | ✓ 0.59 / **0.88** |
| <code>+=</code> | ✓ **0.77** / **0.80** | ✓ 0.62 / **0.87** |
| <code>+a</code> | ✓ 0.60 / **0.95** | ✓ 0.58 / **0.93** |
| <code>,</code> | ✓ 0.60 / **0.71** | ✓ 0.62 / **1.05↑** |
| <code>-</code> | ✓ 0.58 / **1.03↑** | ✓ 0.62 / **0.87** |
| <code>--a</code> | ✓ 0.65 / **0.95** | ✓ 0.62 / **1.07↑** |
| <code>-=</code> | ✓ 0.62 / **0.75** | ✓ 0.58 / **0.92** |
| <code>-a</code> | ✓ 0.62 / **0.98** | ✓ 0.61 / **0.79** |
| <code>/</code> | ✓ 0.60 / **0.99** | ✓ **0.70** / **1.24↑** |
| <code>/=</code> | ✓ 0.60 / **0.79** | ✓ 0.62 / **0.86** |
| <code>017</code> | ✓ **0.73** / **1.03↑** | ✓ **0.70** / **1.01↑** |
| <code>0xff</code> | ✓ 0.46 / **0.90** | ✓ 0.61 / **0.83** |
| <code>16#ff</code> | ✓ 0.54 / **0.75** | ✓ 0.61 / **0.92** |
| <code>&lt;</code> | ✓ 0.62 / **0.86** | ✓ **0.73** / 0.64 |
| <code>&lt;&lt;</code> | ✓ 0.54 / **0.87** | ✓ 0.54 / **0.67** |
| <code>&lt;&lt;=</code> | ✓ 0.61 / **0.90** | ✓ 0.59 / **0.84** |
| <code>&lt;=</code> | ✓ **0.74** / **1.09↑** | ✓ **0.76** / **1.08↑** |
| <code>=</code> | ✓ 0.55 / **0.89** | ✓ 0.61 / **0.90** |
| <code>==</code> | ✓ 0.48 / 0.64 | ✓ 0.57 / **1.23↑** |
| <code>&gt;</code> | ✓ 0.54 / **0.87** | ✓ 0.62 / **0.94** |
| <code>&gt;=</code> | ✓ 0.63 / **1.29↑** | ✓ 0.64 / **1.09↑** |
| <code>&gt;&gt;</code> | ✓ 0.63 / **0.92** | ✓ 0.56 / **1.03↑** |
| <code>&gt;&gt;=</code> | ✓ 0.55 / **1.12↑** | ✓ 0.56 / **1.25↑** |
| <code>^</code> | ✓ 0.55 / **1.14↑** | ✓ 0.61 / **0.85** |
| <code>^=</code> | ✓ 0.56 / **0.80** | ✓ 0.61 / **0.90** |
| <code>a++</code> | ✓ **0.73** / **0.86** | ✓ 0.62 / **0.88** |
| <code>a--</code> | ✓ 0.63 / **0.88** | ✓ 0.63 / **0.93** |
| <code>a?3:4</code> | ✓ **0.77** / **0.84** | ✓ 0.60 / **1.02↑** |
| <code>&#124;</code> | ✓ 0.59 / **0.85** | ✓ 0.63 / **0.81** |
| <code>&#124;=</code> | ✓ 0.55 / **0.84** | ✓ 0.59 / **0.87** |
| <code>&#124;&#124;</code> | ✓ 0.66 / **0.98** | ✓ 0.62 / **0.96** |
| <code>~a</code> | ✓ 0.57 / **0.88** | ✓ 0.58 / **0.91** |

## Builtin

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>.</code> | ✓ 0.49 / **0.81** | ✓ 0.48 / 0.50 |
| <code>:</code> | ✓ 0.63 / 0.57 | ✓ 0.51 / 0.58 |
| <code>[</code> | ✓ 0.49 / **1.01↑** | ✓ **0.67** / **1.31↑** |
| <code>alias</code> | ✓ 0.47 / **0.70** | ✓ 0.63 / **1.04↑** |
| <code>bg</code> | ✓ **0.99** | ✓ **0.94** |
| <code>bind</code> | ✓ 0.43 / **0.67** | ✓ 0.50 / **0.75** |
| <code>break</code> | ✓ **0.73** / **0.85** | ✓ **0.88** / **0.77** |
| <code>builtin</code> | ✓ **0.87** / **0.76** | ✓ 0.49 / **0.98** |
| <code>caller</code> | ✓ **0.67** / 0.36 | ✓ **0.83** / **1.16↑** |
| <code>cd</code> | ✓ 0.63 / **1.22↑** | ✓ **0.73** / **0.96** |
| <code>command</code> | ✓ **0.71** / **1.03↑** | ✓ **0.75** / 0.62 |
| <code>compgen</code> | ✓ 0.60 / **0.91** | ✓ 0.61 / **0.72** |
| <code>complete</code> | ✓ 0.66 / **0.81** | ✓ 0.65 / **0.97** |
| <code>compopt</code> | ✓ **0.74** / 0.66 | ✓ 0.52 / **0.71** |
| <code>continue</code> | ✓ 0.53 / 0.66 | ✓ **0.78** / **1.25↑** |
| <code>declare</code> | ✓ **0.72** / **0.75** | ✓ **0.76** / **1.08↑** |
| <code>dirs</code> | ✓ **0.76** / **0.83** | ✓ **0.86** / **0.97** |
| <code>disown</code> | ✓ **1.00** / **1.00↑** | ✓ **0.98** / **1.00** |
| <code>echo</code> | ✓ 0.37 / **1.11↑** | ✓ **0.99** / **0.81** |
| <code>enable</code> | ✓ 0.54 / **0.80** | ✓ **0.77** / **1.10↑** |
| <code>eval</code> | ✓ 0.55 / **0.73** | ✓ **0.78** / **0.91** |
| <code>exec</code> | ✓ **0.83** / **1.08↑** | ✓ 0.52 / **0.84** |
| <code>exit</code> | ✓ 0.60 / **1.09↑** | ✓ 0.59 / **0.74** |
| <code>export</code> | ✓ **0.68** / **1.12↑** | ✓ **0.92** / **0.96** |
| <code>false</code> | ✓ **0.69** / **0.67** | ✓ 0.52 / **0.67** |
| <code>fc</code> | ✓ 0.65 / **0.97** | ✓ **0.71** / **1.26↑** |
| <code>fg</code> | ✓ **0.99** | ✓ **0.94** |
| <code>getopts</code> | ✓ **0.71** / 0.54 | ✓ **0.82** / **0.73** |
| <code>hash</code> | ✓ **0.70** / **1.06↑** | ✓ **0.96** / **1.24↑** |
| <code>help</code> | ✓ 0.49 / **0.92** | ✓ 0.63 / **0.74** |
| <code>history</code> | ✓ 0.52 / **1.10↑** | ✓ **0.71** / 0.65 |
| <code>jobs</code> | ✓ **0.99** / **1.00↑** | ✓ **0.99** / **1.00↑** |
| <code>kill</code> | ✓ **0.73** / **0.80** | ✓ **0.74** / **0.88** |
| <code>let</code> | ✓ 0.64 / **1.01↑** | ✓ **0.77** / **0.98** |
| <code>local</code> | ✓ **0.84** / **0.89** | ✓ 0.66 / **0.86** |
| <code>logout</code> | ✓ 0.61 | ✓ 0.56 |
| <code>mapfile</code> | ✓ 0.58 / **0.91** | ✓ 0.65 / **1.00↑** |
| <code>popd</code> | ✓ **0.81** / **0.89** | ✓ **1.06↑** / **0.89** |
| <code>printf</code> | ✓ 0.59 / **1.03↑** | ✓ **0.69** / **0.98** |
| <code>pushd</code> | ✓ **0.77** / **0.82** | ✓ **0.81** / **0.85** |
| <code>pwd</code> | ✓ **0.69** / **0.78** | ✓ **0.98** / **0.79** |
| <code>read</code> | ✓ 0.65 / **0.88** | ✓ 0.63 / **0.75** |
| <code>readarray</code> | ✓ **0.73** / **1.11↑** | ✓ 0.54 / 0.64 |
| <code>readonly</code> | ✓ **0.75** / **0.73** | ✓ 0.52 / **0.82** |
| <code>return</code> | ✓ **0.93** / **0.92** | ✓ **0.70** / **1.62↑** |
| <code>set</code> | ✓ 0.63 / **0.70** | ✓ **0.69** / **0.73** |
| <code>shift</code> | ✓ 0.65 / **0.97** | ✓ 0.60 / **1.00↑** |
| <code>shopt</code> | ✓ **0.75** / **0.74** | ✓ **0.74** / **0.93** |
| <code>source</code> | ✓ **0.68** / **1.36↑** | ✓ **0.95** / **1.09↑** |
| <code>suspend</code> | ✓ **0.91** | ✓ **0.69** |
| <code>test</code> | ✓ 0.63 / **1.18↑** | ✓ 0.56 / 0.55 |
| <code>times</code> | ✓ 0.55 / **0.69** | ✓ 0.56 / 0.60 |
| <code>trap</code> | ✓ **0.68** / **0.72** | ✓ 0.58 / **0.73** |
| <code>true</code> | ✓ 0.62 / **0.89** | ✓ 0.63 / **0.68** |
| <code>type</code> | ✓ **0.73** / **1.05↑** | ✓ 0.63 / **0.96** |
| <code>typeset</code> | ✓ 0.60 / 0.65 | ✓ 0.59 / **0.80** |
| <code>ulimit</code> | ✓ **0.73** / **0.77** | ✓ 0.49 / **0.86** |
| <code>umask</code> | ✓ **0.72** / **0.74** | ✓ **0.88** / **0.87** |
| <code>unalias</code> | ✓ 0.64 / 0.52 | ✓ **0.70** / **0.83** |
| <code>unset</code> | ✓ **0.92** / **0.95** | ✓ 0.66 / **0.80** |
| <code>wait</code> | ✓ 0.65 / **0.98** | ✓ **0.79** / **1.24↑** |

## Builtin option

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>. -p</code> | — | Δ |
| <code>alias -p</code> | ✓ 0.59 / **0.81** | ✓ 0.63 / **0.89** |
| <code>bind -P</code> | ? | ? |
| <code>bind -S</code> | ? | ? |
| <code>bind -V</code> | ? | ? |
| <code>bind -X</code> | ? | ? |
| <code>bind -f</code> | ? | ? |
| <code>bind -l</code> | ? | ? |
| <code>bind -m</code> | ? | ? |
| <code>bind -p</code> | ? | ? |
| <code>bind -q</code> | ? | ? |
| <code>bind -r</code> | ? | ? |
| <code>bind -s</code> | ? | ? |
| <code>bind -u</code> | ? | ? |
| <code>bind -v</code> | ? | ? |
| <code>bind -x</code> | ? | ? |
| <code>cd -@</code> | N | N |
| <code>cd -L</code> | ✓ **0.75** / **0.78** | ✓ **0.93** / **0.91** |
| <code>cd -P</code> | ✓ **0.73** / **0.77** | ✓ **1.02↑** / **0.84** |
| <code>cd -e</code> | Δ | Δ |
| <code>command -V</code> | ✓ 0.59 / **0.96** | ✓ 0.64 / **1.01↑** |
| <code>command -p</code> | ✓ **0.68** / **0.95** | ✓ **0.74** / **0.71** |
| <code>command -v</code> | ✓ 0.59 / **0.77** | ✓ **0.84** / **0.76** |
| <code>compgen -A</code> | ✓ **0.72** / **0.77** | ✓ **0.86** / **1.00** |
| <code>compgen -C</code> | ✓ 0.66 / **0.72** | ✓ **0.93** / **0.91** |
| <code>compgen -F</code> | ✓ **0.77** / **0.79** | ✓ **0.89** / **0.94** |
| <code>compgen -G</code> | ✓ **0.72** / **0.81** | ✓ **0.87** / **0.92** |
| <code>compgen -P</code> | ✓ **0.73** / **0.79** | ✓ **0.90** / **0.92** |
| <code>compgen -S</code> | ✓ **0.73** / **0.89** | ✓ **0.83** / **0.95** |
| <code>compgen -V</code> | — | Δ |
| <code>compgen -W</code> | ✓ **0.79** / **0.87** | ✓ **0.91** / **0.94** |
| <code>compgen -X</code> | ✓ **0.81** / **0.70** | ✓ **0.86** / **0.87** |
| <code>compgen -a</code> | ✓ **0.74** / **0.77** | ✓ **0.88** / **0.99** |
| <code>compgen -b</code> | ✓ **0.69** / **0.68** | ✓ **0.83** / **0.87** |
| <code>compgen -c</code> | ✓ **0.85** / **0.81** | ✓ **0.93** / **1.02↑** |
| <code>compgen -d</code> | ✓ **0.72** / **0.79** | ✓ **0.93** / **0.94** |
| <code>compgen -e</code> | ✓ **0.75** / **0.76** | ✓ **0.92** / **0.91** |
| <code>compgen -f</code> | ✓ **0.70** / **0.76** | ✓ **0.91** / **0.94** |
| <code>compgen -g</code> | Δ | Δ |
| <code>compgen -j</code> | ✓ **0.74** / **0.77** | ✓ **0.95** / **0.94** |
| <code>compgen -k</code> | ✓ **0.69** / **0.77** | ✓ **0.90** / **0.97** |
| <code>compgen -o</code> | ✓ **0.74** / **0.94** | ✓ **0.95** / **0.96** |
| <code>compgen -s</code> | ✓ 0.60 / **0.73** | ✓ **0.94** / **0.95** |
| <code>compgen -u</code> | ✓ **0.75** / **0.75** | ✓ **0.93** / **0.97** |
| <code>compgen -v</code> | ✓ **0.71** / **0.79** | ✓ **0.90** / **0.92** |
| <code>complete -A</code> | ✓ **0.70** / **0.68** | ✓ **0.84** / **0.95** |
| <code>complete -C</code> | ✓ **0.68** / **0.86** | ✓ **0.84** / **0.95** |
| <code>complete -D</code> | ✓ 0.65 / **0.83** | ✓ 0.59 / **0.94** |
| <code>complete -E</code> | ✓ **0.72** / **1.02↑** | ✓ 0.66 / **0.95** |
| <code>complete -F</code> | ✓ **0.83** / **0.85** | ✓ **0.84** / **0.89** |
| <code>complete -G</code> | ✓ **0.73** / **0.86** | ✓ **0.82** / **0.92** |
| <code>complete -I</code> | ✓ 0.58 / **0.81** | ✓ 0.65 / **1.32↑** |
| <code>complete -P</code> | ✓ **0.75** / **0.90** | ✓ **0.91** / **0.93** |
| <code>complete -S</code> | ✓ **0.89** / **0.71** | ✓ **0.89** / **0.95** |
| <code>complete -W</code> | ✓ **0.85** / **0.81** | ✓ **0.85** / **0.79** |
| <code>complete -X</code> | ✓ **0.74** / **0.98** | ✓ **0.87** / **0.90** |
| <code>complete -a</code> | ✓ **0.78** / **0.74** | ✓ **0.85** / **0.91** |
| <code>complete -b</code> | ✓ 0.63 / **0.74** | ✓ **0.83** / **0.90** |
| <code>complete -c</code> | ✓ **0.99** / **0.73** | ✓ **0.88** / **0.90** |
| <code>complete -d</code> | ✓ **0.82** / **0.71** | ✓ **0.87** / **0.91** |
| <code>complete -e</code> | ✓ **0.81** / **0.89** | ✓ **0.85** / **0.89** |
| <code>complete -f</code> | ✓ **0.71** / **1.00** | ✓ **0.87** / **0.90** |
| <code>complete -g</code> | Δ | Δ |
| <code>complete -j</code> | ✓ **0.76** / **0.97** | ✓ **0.85** / **0.94** |
| <code>complete -k</code> | ✓ **0.71** / **0.73** | ✓ **0.88** / **0.93** |
| <code>complete -o</code> | ✓ 0.62 / **0.83** | ✓ **0.88** / **0.90** |
| <code>complete -p</code> | ✓ 0.66 / **0.81** | ✓ 0.65 / **0.97** |
| <code>complete -r</code> | ✓ **0.77** / **0.95** | ✓ 0.63 / **0.96** |
| <code>complete -s</code> | Δ | Δ |
| <code>complete -u</code> | ✓ **0.86** / **0.76** | ✓ **0.88** / **0.92** |
| <code>complete -v</code> | ✓ **0.71** / **0.98** | ✓ **0.85** / **0.94** |
| <code>compopt -D</code> | ✓ 0.65 / **0.78** | ✓ 0.63 / **0.94** |
| <code>compopt -E</code> | ✓ **0.72** / **0.74** | ✓ 0.60 / **0.91** |
| <code>compopt -I</code> | ✓ 0.61 / **1.35↑** | ✓ 0.62 / **0.94** |
| <code>compopt -o</code> | ✓ **0.74** / 0.66 | ✓ 0.52 / **0.71** |
| <code>declare -A</code> | ✓ **0.67** / **0.93** | ✓ 0.64 / **0.93** |
| <code>declare -F</code> | ✓ 0.60 / **0.91** | ✓ 0.63 / **0.89** |
| <code>declare -I</code> | ✓ **0.78** / 0.59 | ✓ 0.66 / **0.95** |
| <code>declare -a</code> | ✓ 0.57 / **0.94** | ✓ 0.63 / **0.85** |
| <code>declare -f</code> | ✓ 0.61 / **0.81** | ✓ **0.74** / **0.77** |
| <code>declare -g</code> | ✓ **0.75** / **0.92** | ✓ 0.64 / **0.96** |
| <code>declare -i</code> | ✓ 0.64 / **0.76** | ✓ 0.60 / **0.94** |
| <code>declare -l</code> | Δ | Δ |
| <code>declare -n</code> | ✓ 0.63 / **0.89** | ✓ 0.60 / **0.98** |
| <code>declare -p</code> | ✓ 0.60 / 0.64 | ✓ 0.58 / **1.01↑** |
| <code>declare -r</code> | ✓ 0.63 / **0.74** | ✓ 0.62 / **0.72** |
| <code>declare -t</code> | ✓ 0.48 / 0.65 | ✓ 0.60 / **0.89** |
| <code>declare -u</code> | Δ | Δ |
| <code>declare -x</code> | ✓ **0.73** / **1.01↑** | ✓ **0.70** / **0.88** |
| <code>dirs -N</code> | ✓ **0.79** / **0.87** | ✓ **0.90** / **0.90** |
| <code>dirs -c</code> | ✓ 0.62 / **0.81** | ✓ **0.83** / **0.90** |
| <code>dirs -l</code> | ✓ **0.74** / **0.72** | ✓ **0.81** / **0.87** |
| <code>dirs -p</code> | ✓ 0.62 / **1.03↑** | ✓ **0.83** / **0.87** |
| <code>dirs -v</code> | ✓ **0.71** / **0.79** | ✓ **0.88** / **0.90** |
| <code>disown -a</code> | ✓ **0.98** / **1.00** | ✓ **0.98** / **0.99** |
| <code>disown -h</code> | ✓ **0.99** / **0.99** | ✓ **0.99** / **1.00** |
| <code>disown -r</code> | ✓ **0.99** / **1.00↑** | ✓ **1.00** / **1.00↑** |
| <code>echo -E</code> | ✓ 0.66 / **0.87** | ✓ 0.58 / **0.83** |
| <code>echo -e</code> | ✓ **0.81** / **1.39↑** | ✓ 0.57 / **0.80** |
| <code>echo -n</code> | ✓ 0.63 / **0.81** | ✓ 0.56 / **0.79** |
| <code>enable -a</code> | ? | ? |
| <code>enable -d</code> | ? | ? |
| <code>enable -f</code> | ? | ? |
| <code>enable -n</code> | ? | ? |
| <code>enable -p</code> | ? | ? |
| <code>enable -s</code> | ? | ? |
| <code>exec -a</code> | ✓ **0.93** / **1.69↑** | ✓ **0.70** / **0.94** |
| <code>exec -c</code> | ✓ **0.86** / **1.13↑** | ✓ **0.75** / **1.03↑** |
| <code>exec -l</code> | ✓ **0.78** / **1.40↑** | ✓ **0.95** / **0.99** |
| <code>export -f</code> | ✓ 0.57 / 0.34 | ✓ 0.56 / 0.58 |
| <code>export -n</code> | ✓ 0.66 / **0.92** | ✓ **0.67** / **0.95** |
| <code>export -p</code> | ✓ **0.77** / **1.04↑** | ✓ **0.69** / **0.84** |
| <code>fc -e</code> | ? | ? |
| <code>fc -l</code> | ? | ? |
| <code>fc -n</code> | ? | ? |
| <code>fc -r</code> | ? | ? |
| <code>fc -s</code> | ? | ? |
| <code>hash -d</code> | ✓ **0.76** / **1.29↑** | ✓ 0.62 / **0.85** |
| <code>hash -l</code> | ✓ 0.33 / **1.49↑** | ✓ 0.61 / **0.75** |
| <code>hash -p</code> | ✓ **0.70** / **1.06↑** | ✓ **0.96** / **1.24↑** |
| <code>hash -r</code> | ✓ 0.39 / **0.70** | ✓ 0.61 / **0.75** |
| <code>hash -t</code> | ✓ **0.70** / **1.06↑** | ✓ **0.96** / **1.24↑** |
| <code>help -d</code> | ? | ? |
| <code>help -m</code> | ? | ? |
| <code>help -s</code> | ? | ? |
| <code>history -a</code> | ? | ? |
| <code>history -c</code> | ? | ? |
| <code>history -d</code> | ? | ? |
| <code>history -n</code> | ? | ? |
| <code>history -p</code> | ? | ? |
| <code>history -r</code> | ? | ? |
| <code>history -s</code> | ? | ? |
| <code>history -w</code> | ? | ? |
| <code>jobs -l</code> | ✓ **0.98** / **0.99** | ✓ **1.01↑** / **1.00↑** |
| <code>jobs -n</code> | ✓ **0.97** / **1.00↑** | Δ |
| <code>jobs -p</code> | ✓ **0.99** / **1.00↑** | ✓ **0.99** / **1.00↑** |
| <code>jobs -r</code> | ✓ **0.99** / **0.99** | ✓ **0.99** / **1.00** |
| <code>jobs -s</code> | ✓ **0.99** / **0.99** | ✓ **0.99** / **1.00** |
| <code>jobs -x</code> | ✓ **0.99** / **0.99** | ✓ **1.00** / **1.01↑** |
| <code>kill -l</code> | ✓ 0.53 / **0.78** | ✓ 0.61 / **0.86** |
| <code>kill -n</code> | ✓ 0.60 / **0.83** | ✓ **0.71** / **1.09↑** |
| <code>kill -s</code> | ✓ **0.72** / **0.88** | ✓ **0.68** / **0.90** |
| <code>mapfile -C</code> | ✓ **0.67** / **0.77** | ✓ **0.84** / **0.90** |
| <code>mapfile -O</code> | ✓ 0.60 / **0.68** | ✓ 0.57 / **1.16↑** |
| <code>mapfile -c</code> | ✓ 0.53 / **0.90** | ✓ 0.61 / **1.24↑** |
| <code>mapfile -d</code> | ✓ **0.73** / **0.95** | ✓ 0.59 / **0.96** |
| <code>mapfile -n</code> | ✓ 0.65 / **0.91** | ✓ 0.62 / **0.96** |
| <code>mapfile -s</code> | ✓ 0.64 / **0.89** | ✓ **0.79** / **1.04↑** |
| <code>mapfile -t</code> | ✓ **0.70** / **1.06↑** | ✓ **0.75** / **0.96** |
| <code>mapfile -u</code> | ✓ **0.70** / **0.99** | ✓ 0.62 / **1.11↑** |
| <code>popd -N</code> | ✓ **0.69** / **0.79** | ✓ **0.83** / **0.87** |
| <code>popd -n</code> | ✓ **0.79** / **0.81** | ✓ **0.89** / **0.93** |
| <code>printf -v</code> | ✓ 0.59 / **1.03↑** | ✓ **0.69** / **0.98** |
| <code>pushd -N</code> | ✓ **0.75** / **0.81** | ✓ **0.83** / **1.02↑** |
| <code>pushd -n</code> | ✓ **0.73** / **0.86** | ✓ **0.88** / **0.92** |
| <code>pwd -L</code> | ✓ **0.71** / **0.85** | ✓ **0.83** / **0.92** |
| <code>pwd -P</code> | ✓ **0.69** / **0.92** | ✓ **0.89** / **0.88** |
| <code>read -E</code> | N | ✓ **0.81** |
| <code>read -N</code> | ✓ 0.64 / **0.87** | ✓ **0.71** / **0.87** |
| <code>read -a</code> | ✓ 0.61 / **0.99** | ✓ **0.69** / **1.25↑** |
| <code>read -d</code> | ✓ **0.67** / **0.92** | ✓ **0.68** / **0.79** |
| <code>read -e</code> | ✓ **0.78** | ✓ **1.01↑** |
| <code>read -i</code> | ✓ **0.78** | ✓ **0.80** |
| <code>read -n</code> | ✓ 0.66 / **0.95** | ✓ 0.49 / **1.02↑** |
| <code>read -p</code> | ✓ **0.81** | ✓ **0.76** |
| <code>read -r</code> | ✓ 0.63 / **0.88** | ✓ 0.61 / **1.00↑** |
| <code>read -s</code> | ✓ **0.88** | ✓ **0.80** |
| <code>read -t</code> | ✓ **1.00** / **1.00↑** | ✓ **1.00** / **1.00↑** |
| <code>read -u</code> | ✓ 0.60 / **0.76** | ✓ 0.65 / **1.00** |
| <code>readarray -C</code> | ✓ 0.63 / **0.92** | ✓ **0.68** / **1.01↑** |
| <code>readarray -O</code> | ✓ 0.62 / **0.82** | ✓ 0.63 / **1.09↑** |
| <code>readarray -c</code> | ✓ 0.63 / **0.88** | ✓ **0.69** / **1.04↑** |
| <code>readarray -d</code> | ✓ 0.62 / **0.80** | ✓ **0.71** / **0.81** |
| <code>readarray -n</code> | ✓ 0.62 / **0.85** | ✓ 0.60 / **0.94** |
| <code>readarray -s</code> | ✓ 0.63 / **0.96** | ✓ 0.63 / **0.91** |
| <code>readarray -t</code> | ✓ 0.63 / **0.83** | ✓ **0.79** / **0.74** |
| <code>readarray -u</code> | ✓ 0.63 / **0.84** | ✓ 0.66 / **0.82** |
| <code>readonly -A</code> | ✓ 0.61 / **0.72** | ✓ **0.69** / **0.72** |
| <code>readonly -a</code> | ✓ 0.61 / **0.71** | ✓ **0.71** / **0.89** |
| <code>readonly -f</code> | ✓ 0.61 / **0.89** | ✓ 0.66 / **0.84** |
| <code>readonly -p</code> | ✓ **0.67** / **0.78** | ✓ **0.74** / **0.80** |
| <code>set -B</code> | I 0.59 / **0.96** | I 0.65 / **1.10↑** |
| <code>set -C</code> | I 0.61 / **0.89** | I 0.57 / **1.20↑** |
| <code>set -E</code> | I 0.60 / **0.93** | I 0.66 / **1.04↑** |
| <code>set -H</code> | I 0.60 / **0.81** | I **0.67** / **1.02↑** |
| <code>set -P</code> | Δ | Δ |
| <code>set -T</code> | I 0.60 / **0.81** | I 0.65 / **1.01↑** |
| <code>set -a</code> | I 0.60 / **0.91** | I **0.68** / **0.99** |
| <code>set -b</code> | Δ | Δ |
| <code>set -e</code> | I 0.60 / **0.84** | I 0.66 / **1.08↑** |
| <code>set -f</code> | I 0.59 / **0.86** | I 0.60 / **1.37↑** |
| <code>set -h</code> | I 0.60 / **0.89** | I 0.66 / **0.93** |
| <code>set -k</code> | Δ | Δ |
| <code>set -m</code> | I 0.62 / **0.90** | I 0.66 / **0.85** |
| <code>set -n</code> | Δ | Δ |
| <code>set -o</code> | ✓ 0.63 / **0.70** | ✓ **0.69** / **0.73** |
| <code>set -p</code> | Δ | Δ |
| <code>set -t</code> | I 0.58 / **0.89** | I 0.65 / **0.74** |
| <code>set -u</code> | I 0.62 / **0.88** | I 0.58 / **1.03↑** |
| <code>set -v</code> | I 0.62 / **1.03↑** | I 0.59 / **0.75** |
| <code>set -x</code> | E | E |
| <code>shopt -o</code> | ✓ 0.65 / **0.90** | ✓ 0.64 / **1.02↑** |
| <code>shopt -p</code> | ✓ 0.62 / **0.88** | ✓ 0.64 / **0.84** |
| <code>shopt -q</code> | ✓ **0.67** / **0.96** | ✓ 0.63 / **0.90** |
| <code>shopt -s</code> | ✓ **0.75** / **0.74** | ✓ **0.74** / **0.93** |
| <code>shopt -u</code> | ✓ **0.75** / **0.74** | ✓ **0.74** / **0.93** |
| <code>source -p</code> | — | Δ |
| <code>suspend -f</code> | ? | ? |
| <code>trap -P</code> | — | Δ |
| <code>trap -l</code> | ✓ **0.69** / **0.77** | ✓ **0.75** / **0.76** |
| <code>trap -p</code> | ✓ 0.57 / **0.77** | ✓ 0.57 / **1.08↑** |
| <code>type -P</code> | ✓ **0.70** / **0.81** | ✓ **0.71** / **0.89** |
| <code>type -a</code> | ✓ 0.66 / **0.81** | ✓ **0.85** / **0.99** |
| <code>type -f</code> | ✓ 0.61 / **0.89** | ✓ 0.63 / **1.08↑** |
| <code>type -p</code> | ✓ 0.61 / **0.86** | ✓ 0.65 / **0.95** |
| <code>type -t</code> | ✓ 0.59 / **0.88** | ✓ 0.61 / **0.91** |
| <code>typeset -A</code> | ✓ 0.58 / **0.83** | ✓ 0.64 / **0.92** |
| <code>typeset -F</code> | ✓ 0.57 / **0.94** | ✓ 0.64 / **0.84** |
| <code>typeset -I</code> | ✓ 0.59 / **0.81** | ✓ 0.66 / **1.17↑** |
| <code>typeset -a</code> | ✓ 0.57 / **0.90** | ✓ 0.58 / **0.89** |
| <code>typeset -f</code> | ✓ 0.66 / **0.79** | ✓ **0.72** / **0.86** |
| <code>typeset -g</code> | ✓ 0.59 / **0.89** | ✓ 0.65 / **0.98** |
| <code>typeset -i</code> | ✓ 0.59 / **0.88** | ✓ 0.61 / **0.98** |
| <code>typeset -l</code> | Δ | Δ |
| <code>typeset -n</code> | ✓ 0.60 / **0.84** | ✓ 0.59 / **0.87** |
| <code>typeset -p</code> | ✓ 0.56 / **0.85** | ✓ 0.61 / **0.98** |
| <code>typeset -r</code> | ✓ 0.63 / **0.78** | ✓ 0.66 / **0.77** |
| <code>typeset -t</code> | ✓ 0.60 / **0.85** | ✓ 0.57 / **0.92** |
| <code>typeset -u</code> | Δ | Δ |
| <code>typeset -x</code> | ✓ **0.71** / **0.96** | ✓ **0.69** / **1.08↑** |
| <code>ulimit -H</code> | ✓ 0.60 / **0.77** | ✓ 0.60 / **1.04↑** |
| <code>ulimit -P</code> | N | N |
| <code>ulimit -R</code> | ✓ 0.59 / **0.90** | ✓ 0.58 / **0.84** |
| <code>ulimit -S</code> | ✓ 0.57 / **0.87** | ✓ 0.63 / **0.96** |
| <code>ulimit -T</code> | N | N |
| <code>ulimit -a</code> | ✓ 0.59 / **0.91** | ✓ 0.63 / **0.93** |
| <code>ulimit -b</code> | N | N |
| <code>ulimit -c</code> | ✓ 0.56 / **1.01↑** | ✓ 0.64 / **1.02↑** |
| <code>ulimit -d</code> | ✓ 0.60 / **0.83** | ✓ 0.56 / **0.88** |
| <code>ulimit -e</code> | ✓ 0.58 / **1.00** | ✓ 0.59 / **0.75** |
| <code>ulimit -f</code> | ✓ 0.57 / **0.99** | ✓ 0.64 / **1.00** |
| <code>ulimit -i</code> | ✓ 0.60 / **0.94** | ✓ 0.61 / **0.77** |
| <code>ulimit -k</code> | N | N |
| <code>ulimit -l</code> | ✓ 0.59 / **0.82** | ✓ 0.64 / **0.99** |
| <code>ulimit -m</code> | ✓ 0.58 / **0.98** | ✓ 0.56 / **1.05↑** |
| <code>ulimit -n</code> | ✓ 0.60 / **0.97** | ✓ 0.57 / **0.86** |
| <code>ulimit -p</code> | ✓ 0.58 / **0.97** | ✓ 0.61 / **1.05↑** |
| <code>ulimit -q</code> | ✓ 0.58 / **1.01↑** | ✓ 0.58 / **0.94** |
| <code>ulimit -r</code> | ✓ 0.59 / **1.00** | ✓ 0.59 / **0.76** |
| <code>ulimit -s</code> | ✓ 0.60 / **0.97** | ✓ 0.59 / **0.83** |
| <code>ulimit -t</code> | ✓ 0.59 / **0.93** | ✓ 0.58 / **1.04↑** |
| <code>ulimit -u</code> | ✓ 0.56 / **0.83** | ✓ 0.60 / **0.86** |
| <code>ulimit -v</code> | ✓ 0.56 / **0.95** | ✓ 0.59 / **0.78** |
| <code>ulimit -x</code> | ✓ 0.58 / **1.04↑** | ✓ 0.60 / **0.85** |
| <code>umask -S</code> | ✓ 0.58 / **0.88** | ✓ 0.57 / **0.95** |
| <code>umask -p</code> | ✓ 0.57 / **0.85** | ✓ 0.59 / **0.85** |
| <code>unalias -a</code> | ✓ 0.60 / **0.93** | ✓ **0.68** / **1.00** |
| <code>unset -f</code> | ✓ **0.92** / **0.95** | ✓ 0.66 / **0.80** |
| <code>unset -n</code> | ✓ 0.61 / **0.80** | ✓ 0.60 / **0.83** |
| <code>unset -v</code> | ✓ 0.59 / **0.84** | ✓ 0.59 / **0.93** |
| <code>wait -f</code> | ✓ **0.78** / **1.15↑** | ✓ **0.75** / **1.15↑** |
| <code>wait -n</code> | ✓ **0.97** / **0.99** | ✓ **0.97** / **1.02↑** |
| <code>wait -p</code> | ✓ **0.98** / **1.00** | ✓ **0.98** / **0.98** |

## Compatibility level

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>31</code> | Δ | Δ |
| <code>32</code> | Δ | — |
| <code>40</code> | Δ | Δ |
| <code>41</code> | Δ | Δ |
| <code>42</code> | Δ | Δ |
| <code>43</code> | Δ | Δ |
| <code>44</code> | Δ | Δ |
| <code>50</code> | Δ | Δ |
| <code>51</code> | ✓ 0.59 / **1.03↑** | ✓ 0.61 / **0.81** |
| <code>52</code> | — | Δ |

## Completion action

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>alias</code> | ✓ **0.74** / **0.80** | ✓ **0.94** / **0.97** |
| <code>arrayvar</code> | ✓ **0.75** / **0.79** | ✓ **0.91** / **0.99** |
| <code>binding</code> | ✓ **0.73** / **0.78** | ✓ **0.96** / **0.92** |
| <code>builtin</code> | ✓ **0.74** / **0.79** | ✓ **0.91** / **0.95** |
| <code>command</code> | ✓ **0.75** / **0.82** | ✓ **0.90** / **1.00** |
| <code>directory</code> | ✓ **0.72** / **0.80** | ✓ **0.90** / **0.90** |
| <code>disabled</code> | ✓ **0.72** / **0.77** | ✓ **0.92** / **0.93** |
| <code>enabled</code> | ✓ **0.73** / **0.78** | ✓ **0.93** / **0.94** |
| <code>export</code> | ✓ **0.73** / **0.82** | ✓ **0.93** / **0.93** |
| <code>file</code> | ✓ **0.72** / **0.80** | ✓ **0.90** / **0.97** |
| <code>function</code> | ✓ **0.73** / **0.77** | ✓ **0.91** / **0.98** |
| <code>group</code> | Δ | Δ |
| <code>helptopic</code> | ✓ **0.72** / **0.82** | ✓ **0.91** / **0.92** |
| <code>hostname</code> | I **0.72** / **0.80** | I **0.92** / **0.93** |
| <code>job</code> | ✓ **0.99** / **1.00** | ✓ **0.98** / **1.00** |
| <code>keyword</code> | ✓ **0.73** / **0.81** | ✓ **0.98** / **0.99** |
| <code>running</code> | ✓ **0.99** / **1.00** | ✓ **0.98** / **0.98** |
| <code>service</code> | Δ | Δ |
| <code>setopt</code> | ✓ **0.77** / **0.83** | ✓ **0.93** / **0.89** |
| <code>shopt</code> | ✓ **0.74** / **0.80** | ✓ **0.88** / **0.95** |
| <code>signal</code> | E | E |
| <code>stopped</code> | I **0.98** / **1.00↑** | I **0.99** / **0.99** |
| <code>user</code> | ✓ **0.75** / **0.80** | ✓ **0.91** / **0.94** |
| <code>variable</code> | ✓ **0.73** / **0.82** | ✓ **0.93** / **1.01↑** |

## Completion option

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>bashdefault</code> | I 0.61 / **0.96** | I **0.84** / **1.08↑** |
| <code>default</code> | I 0.58 / **0.89** | I **0.98** / 0.60 |
| <code>dirnames</code> | I 0.63 / **0.80** | I **0.97** / **0.83** |
| <code>filenames</code> | I 0.64 / **0.82** | I **0.84** / **1.14↑** |
| <code>fullquote</code> | Δ | I **0.73** / **1.32↑** |
| <code>noquote</code> | I 0.65 / **0.80** | I **0.69** / **0.98** |
| <code>nosort</code> | I **0.72** / **0.92** | I **0.81** / **0.82** |
| <code>nospace</code> | I 0.66 / **1.01↑** | I **0.91** / **0.77** |
| <code>plusdirs</code> | I **0.70** / **0.69** | I 0.61 / **1.11↑** |

## Conditional

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>!=</code> | ✓ 0.48 / 0.58 | ✓ 0.54 / **1.02↑** |
| <code>-G</code> | ✓ 0.59 / **0.74** | ✓ **0.83** / **0.72** |
| <code>-L</code> | ✓ 0.59 / **0.70** | ✓ **0.77** / **0.83** |
| <code>-N</code> | Δ | Δ |
| <code>-O</code> | ✓ 0.63 / **0.79** | ✓ **0.75** / **1.13↑** |
| <code>-R</code> | ✓ 0.56 / **1.04↑** | ✓ 0.48 / **1.14↑** |
| <code>-S</code> | ✓ **0.71** / **1.18↑** | ✓ 0.59 / **0.81** |
| <code>-a</code> | ✓ 0.58 / **1.19↑** | ✓ 0.63 / **0.97** |
| <code>-b</code> | F | F |
| <code>-c</code> | ✓ **0.84** / **0.98** | ✓ 0.54 / **0.77** |
| <code>-d</code> | ✓ 0.48 / **0.88** | ✓ **0.76** / **1.41↑** |
| <code>-e</code> | ✓ 0.63 / **0.80** | ✓ 0.60 / **0.80** |
| <code>-ef</code> | ✓ 0.65 / **0.86** | ✓ **0.80** / **0.87** |
| <code>-eq</code> | ✓ 0.59 / **1.03↑** | ✓ 0.56 / 0.57 |
| <code>-f</code> | ✓ 0.66 / **0.96** | ✓ 0.63 / **0.76** |
| <code>-g</code> | ✓ **0.86** / **0.78** | ✓ **0.83** / **0.97** |
| <code>-ge</code> | ✓ **0.83** / **1.15↑** | ✓ 0.55 / **0.88** |
| <code>-gt</code> | ✓ 0.67 / **0.82** | ✓ 0.65 / **1.03↑** |
| <code>-h</code> | ✓ **0.69** / **0.83** | ✓ **0.92** / **0.80** |
| <code>-k</code> | ✓ **0.72** / **0.85** | ✓ **0.91** / **1.03↑** |
| <code>-le</code> | ✓ 0.60 / **0.78** | ✓ **0.74** / **1.38↑** |
| <code>-lt</code> | ✓ 0.61 / **0.77** | ✓ **0.67** / **0.70** |
| <code>-n</code> | ✓ **0.67** / **0.99** | ✓ 0.61 / **0.96** |
| <code>-ne</code> | ✓ 0.64 / 0.65 | ✓ 0.57 / **0.96** |
| <code>-nt</code> | ✓ **0.85** / 0.66 | ✓ **0.87** / **0.97** |
| <code>-o</code> | Δ | Δ |
| <code>-ot</code> | ✓ 0.67 / **0.90** | ✓ **0.93** / **1.00↑** |
| <code>-p</code> | ✓ **0.71** / **0.95** | ✓ **0.72** / **0.92** |
| <code>-r</code> | ✓ **0.73** / **0.87** | ✓ 0.61 / **1.27↑** |
| <code>-s</code> | ✓ **0.93** / **1.17↑** | ✓ 0.56 / **1.05↑** |
| <code>-t</code> | ✓ **0.88** | ✓ **0.88** |
| <code>-u</code> | ✓ **0.86** / **0.82** | ✓ **0.81** / **0.89** |
| <code>-v</code> | ✓ 0.62 / **1.04↑** | ✓ 0.57 / **1.12↑** |
| <code>-w</code> | ✓ **0.68** / **0.78** | ✓ **0.70** / **0.81** |
| <code>-x</code> | ✓ **0.79** / **0.74** | ✓ **0.96** / **0.97** |
| <code>-z</code> | ✓ **0.76** / **0.93** | ✓ **0.80** / **1.32↑** |
| <code>&lt;</code> | ✓ 0.55 / **0.70** | ✓ 0.61 / **1.21↑** |
| <code>=</code> | ✓ 0.66 / **0.98** | ✓ 0.60 / **1.10↑** |
| <code>==</code> | ✓ 0.46 / 0.56 | ✓ 0.65 / **0.94** |
| <code>=~</code> | ✓ 0.63 / **1.15↑** | ✓ **0.68** / **0.89** |
| <code>&gt;</code> | ✓ 0.50 / **1.25↑** | ✓ 0.62 / **0.86** |

## GNU regression group

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>run-alias</code> | Δ | Δ |
| <code>run-appendop</code> | Δ | Δ |
| <code>run-arith</code> | Δ | Δ |
| <code>run-arith-for</code> | Δ | Δ |
| <code>run-array</code> | Δ | Δ |
| <code>run-array2</code> | G **0.74** | G **0.72** |
| <code>run-assoc</code> | Δ | Δ |
| <code>run-attr</code> | Δ | Δ |
| <code>run-braces</code> | G **0.86** | Δ |
| <code>run-builtins</code> | Δ | ? |
| <code>run-case</code> | Δ | Δ |
| <code>run-casemod</code> | Δ | Δ |
| <code>run-complete</code> | Δ | Δ |
| <code>run-comsub</code> | Δ | Δ |
| <code>run-comsub-eof</code> | Δ | Δ |
| <code>run-comsub-posix</code> | Δ | Δ |
| <code>run-comsub2</code> | — | Δ |
| <code>run-cond</code> | Δ | Δ |
| <code>run-coproc</code> | Δ | Δ |
| <code>run-cprint</code> | Δ | Δ |
| <code>run-dbg-support</code> | Δ | Δ |
| <code>run-dbg-support2</code> | Δ | Δ |
| <code>run-dirstack</code> | Δ | Δ |
| <code>run-dollars</code> | Δ | Δ |
| <code>run-dynvar</code> | Δ | Δ |
| <code>run-errors</code> | Δ | Δ |
| <code>run-execscript</code> | ? | ? |
| <code>run-exp-tests</code> | Δ | Δ |
| <code>run-exportfunc</code> | ↗ | ↗ |
| <code>run-extglob</code> | G **0.80** | Δ |
| <code>run-extglob2</code> | G **0.84** | G **0.90** |
| <code>run-extglob3</code> | G **1.03↑** | G **0.93** |
| <code>run-func</code> | Δ | Δ |
| <code>run-getopts</code> | Δ | Δ |
| <code>run-glob-bracket</code> | — | Δ |
| <code>run-glob-test</code> | Δ | Δ |
| <code>run-globstar</code> | Δ | Δ |
| <code>run-heredoc</code> | Δ | Δ |
| <code>run-herestr</code> | Δ | Δ |
| <code>run-histexpand</code> | Δ | Δ |
| <code>run-history</code> | Δ | Δ |
| <code>run-ifs</code> | Δ | Δ |
| <code>run-ifs-posix</code> | G **0.74** | G **0.76** |
| <code>run-input-test</code> | G **0.80** | G **0.92** |
| <code>run-intl</code> | ? | ? |
| <code>run-invert</code> | G **0.84** | G **0.84** |
| <code>run-invocation</code> | — | Δ |
| <code>run-iquote</code> | G **0.76** | G **0.71** |
| <code>run-jobs</code> | Δ | Δ |
| <code>run-lastpipe</code> | Δ | Δ |
| <code>run-mapfile</code> | Δ | Δ |
| <code>run-more-exp</code> | Δ | Δ |
| <code>run-nameref</code> | Δ | Δ |
| <code>run-new-exp</code> | Δ | Δ |
| <code>run-nquote</code> | Δ | Δ |
| <code>run-nquote1</code> | G **0.74** | G **0.79** |
| <code>run-nquote2</code> | G **0.81** | G **0.77** |
| <code>run-nquote3</code> | G **0.87** | G **0.80** |
| <code>run-nquote4</code> | Δ | Δ |
| <code>run-nquote5</code> | G **0.77** | G **0.76** |
| <code>run-parser</code> | Δ | Δ |
| <code>run-posix2</code> | Δ | Δ |
| <code>run-posixexp</code> | Δ | Δ |
| <code>run-posixexp2</code> | Δ | Δ |
| <code>run-posixpat</code> | Δ | Δ |
| <code>run-posixpipe</code> | Δ | Δ |
| <code>run-precedence</code> | G **0.89** | G **0.81** |
| <code>run-printf</code> | Δ | ? |
| <code>run-procsub</code> | G **0.98** | G **0.98** |
| <code>run-quote</code> | Δ | Δ |
| <code>run-quotearray</code> | Δ | ? |
| <code>run-read</code> | ? | ? |
| <code>run-redir</code> | Δ | Δ |
| <code>run-rhs-exp</code> | Δ | Δ |
| <code>run-rsh</code> | Δ | Δ |
| <code>run-set-e</code> | Δ | Δ |
| <code>run-set-x</code> | Δ | Δ |
| <code>run-shopt</code> | Δ | Δ |
| <code>run-strip</code> | G 0.50 | G 0.49 |
| <code>run-test</code> | Δ | ? |
| <code>run-tilde</code> | Δ | Δ |
| <code>run-tilde2</code> | Δ | Δ |
| <code>run-trap</code> | Δ | Δ |
| <code>run-type</code> | Δ | Δ |
| <code>run-varenv</code> | Δ | Δ |
| <code>run-vredir</code> | Δ | Δ |

## History expansion

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>!!</code> | ✓ 0.60 / **1.01↑** | ✓ **0.82** / **0.68** |
| <code>!!:$</code> | ✓ 0.60 / **1.04↑** | ✓ **0.67** / **0.83** |
| <code>!!:$:e</code> | Δ | Δ |
| <code>!!:$:h</code> | ✓ 0.58 / **0.89** | ✓ 0.66 / **1.05↑** |
| <code>!!:$:r</code> | ✓ 0.60 / **0.83** | ✓ **0.67** / **1.07↑** |
| <code>!!:$:t</code> | ✓ 0.57 / **0.86** | ✓ 0.54 / **1.41↑** |
| <code>!!:*</code> | ✓ 0.64 / **0.88** | ✓ 0.60 / **0.85** |
| <code>!!:0</code> | ✓ 0.57 / **0.85** | ✓ 0.65 / **0.75** |
| <code>!!:1-2</code> | ✓ **0.70** / **0.70** | ✓ 0.61 / **1.33↑** |
| <code>!!:2*</code> | ✓ 0.61 / **0.90** | ✓ 0.58 / **1.33↑** |
| <code>!!:^</code> | ✓ 0.61 / **0.87** | ✓ **0.77** / **0.89** |
| <code>!!:p</code> | ✓ 0.56 / **0.85** | ✓ 0.62 / **0.77** |
| <code>!!:q</code> | ✓ **0.67** / **1.01↑** | ✓ 0.59 / **0.75** |
| <code>!!:s/file/name/</code> | ✓ 0.63 / **0.74** | ✓ 0.52 / **0.96** |
| <code>!!:x</code> | ✓ 0.59 / **0.97** | ✓ 0.61 / **0.68** |
| <code>!-1</code> | ✓ 0.59 / **0.92** | ✓ 0.61 / **1.00↑** |
| <code>!1</code> | ✓ 0.63 / **0.87** | ✓ 0.66 / **0.98** |
| <code>!?file?</code> | ✓ 0.58 / **0.90** | ✓ 0.50 / 0.57 |
| <code>!echo</code> | ✓ **0.68** / **0.94** | ✓ **0.71** / **0.91** |

## Invocation

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>+O</code> | ✓ 0.63 | ✓ 0.55 |
| <code>+o</code> | ✓ **0.73** | ✓ **0.71** |
| <code>--</code> | ✓ **0.84** | ✓ 0.57 |
| <code>--debugger</code> | ? | ? |
| <code>--dump-po-strings</code> | Δ | Δ |
| <code>--dump-strings</code> | Δ | Δ |
| <code>--help</code> | Δ | Δ |
| <code>--init-file</code> | E | E |
| <code>--login</code> | ✓ 0.52 | ✓ 0.44 |
| <code>--noediting</code> | I 0.66 | I 0.56 |
| <code>--noprofile</code> | ✓ **0.73** | ✓ 0.48 |
| <code>--norc</code> | I 0.65 | I 0.56 |
| <code>--posix</code> | Δ | Δ |
| <code>--pretty-print</code> | Δ | Δ |
| <code>--rcfile</code> | E | E |
| <code>--restricted</code> | Δ | Δ |
| <code>--verbose</code> | Δ | Δ |
| <code>--version</code> | Δ | Δ |
| <code>-B</code> | ✓ 0.62 | ✓ 0.54 |
| <code>-C</code> | ✓ 0.46 | ✓ 0.58 |
| <code>-D</code> | Δ | Δ |
| <code>-E</code> | ✓ **0.73** | ✓ 0.57 |
| <code>-H</code> | ✓ **0.68** | ✓ **0.81** |
| <code>-O</code> | ✓ 0.62 | ✓ 0.53 |
| <code>-P</code> | Δ | Δ |
| <code>-T</code> | ✓ 0.64 | ✓ **0.92** |
| <code>-a (export assignments)</code> | ✓ **1.30↑** | ✓ **0.96** |
| <code>-b</code> | Δ | Δ |
| <code>-c</code> | ✓ 0.57 | ✓ 0.65 |
| <code>-e</code> | ✓ 0.55 | ✓ 0.62 |
| <code>-f</code> | ✓ 0.63 | ✓ **0.83** |
| <code>-h</code> | ✓ 0.57 | ✓ **0.71** |
| <code>-i</code> | E | E |
| <code>-k</code> | Δ | Δ |
| <code>-l</code> | ✓ 0.46 | ✓ **0.67** |
| <code>-m</code> | Δ | Δ |
| <code>-n</code> | ✓ 0.45 | ✓ **0.94** |
| <code>-o</code> | ✓ **0.97** | ✓ 0.61 |
| <code>-p</code> | Δ | Δ |
| <code>-r</code> | Δ | Δ |
| <code>-s</code> | ✓ **0.81** | ✓ **0.72** |
| <code>-t</code> | ✓ **1.01↑** | ✓ **0.72** |
| <code>-u</code> | E | E |
| <code>-v</code> | E | E |
| <code>-x</code> | ✓ 0.51 | ✓ 0.58 |
| <code>login logout</code> | ✓ 0.61 | ✓ 0.56 |
| <code>read script file</code> | ✓ 0.57 | ✓ 0.56 |
| <code>stdin script</code> | ✓ **0.87** | ✓ **0.70** |

## Manual section

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>A Programmable Completion Example</code> | § | § |
| <code>ANSI-C Quoting</code> | § | § |
| <code>Aliases</code> | § | § |
| <code>Arithmetic Expansion</code> | § | § |
| <code>Arrays</code> | § | § |
| <code>Bash Builtins</code> | § | § |
| <code>Bash Conditional Expressions</code> | § | § |
| <code>Bash Features</code> | § | § |
| <code>Bash History Builtins</code> | § | § |
| <code>Bash History Facilities</code> | § | § |
| <code>Bash POSIX Mode</code> | § | § |
| <code>Bash Startup Files</code> | § | § |
| <code>Bash Variables</code> | § | § |
| <code>Basic Installation</code> | § | § |
| <code>Basic Shell Features</code> | § | § |
| <code>Bindable Readline Commands</code> | § | § |
| <code>Bourne Shell Builtins</code> | § | § |
| <code>Bourne Shell Variables</code> | § | § |
| <code>Brace Expansion</code> | § | § |
| <code>Builtin Index</code> | § | § |
| <code>Command Execution Environment</code> | § | § |
| <code>Command Grouping</code> | § | § |
| <code>Command Line Editing</code> | § | § |
| <code>Command Search and Execution</code> | § | § |
| <code>Command Substitution</code> | § | § |
| <code>Commands For Completion</code> | § | § |
| <code>Commands For History</code> | § | § |
| <code>Commands For Killing</code> | § | § |
| <code>Commands For Moving</code> | § | § |
| <code>Commands For Text</code> | § | § |
| <code>Comments</code> | § | § |
| <code>Compilers and Options</code> | § | § |
| <code>Compiling For Multiple Architectures</code> | § | § |
| <code>Compound Commands</code> | § | § |
| <code>Concept Index</code> | § | § |
| <code>Conditional Constructs</code> | § | § |
| <code>Conditional Init Constructs</code> | § | § |
| <code>Controlling the Prompt</code> | § | § |
| <code>Coprocesses</code> | § | § |
| <code>Creating Internationalized Scripts</code> | § | § |
| <code>Definitions</code> | § | § |
| <code>Directory Stack Builtins</code> | § | § |
| <code>Double Quotes</code> | § | § |
| <code>Environment</code> | § | § |
| <code>Escape Character</code> | § | § |
| <code>Event Designators</code> | § | § |
| <code>Executing Commands</code> | § | § |
| <code>Exit Status</code> | § | § |
| <code>Filename Expansion</code> | § | § |
| <code>Function Index</code> | § | § |
| <code>GNU Free Documentation License</code> | § | § |
| <code>GNU Parallel</code> | § | § |
| <code>History Interaction</code> | § | § |
| <code>Indexes</code> | § | § |
| <code>Installation Names</code> | § | § |
| <code>Installing Bash</code> | § | § |
| <code>Interactive Shell Behavior</code> | § | § |
| <code>Interactive Shells</code> | § | § |
| <code>Introduction</code> | § | § |
| <code>Introduction and Notation</code> | § | § |
| <code>Invoking Bash</code> | § | § |
| <code>Is this Shell Interactive?</code> | § | § |
| <code>Job Control</code> | § | § |
| <code>Job Control Basics</code> | § | § |
| <code>Job Control Builtins</code> | § | § |
| <code>Job Control Variables</code> | § | § |
| <code>Keyboard Macros</code> | § | § |
| <code>Lists</code> | § | § |
| <code>Locale Translation</code> | § | § |
| <code>Looping Constructs</code> | § | § |
| <code>Major Differences From The Bourne Shell</code> | § | § |
| <code>Miscellaneous Commands</code> | § | § |
| <code>Modifiers</code> | § | § |
| <code>Modifying Shell Behavior</code> | § | § |
| <code>Numeric Arguments</code> | § | § |
| <code>Operation Controls</code> | § | § |
| <code>Optional Features</code> | § | § |
| <code>Pattern Matching</code> | § | § |
| <code>Pipelines</code> | § | § |
| <code>Positional Parameters</code> | § | § |
| <code>Process Substitution</code> | § | § |
| <code>Programmable Completion</code> | § | § |
| <code>Programmable Completion Builtins</code> | § | § |
| <code>Quote Removal</code> | § | § |
| <code>Quoting</code> | § | § |
| <code>Readline Arguments</code> | § | § |
| <code>Readline Bare Essentials</code> | § | § |
| <code>Readline Init File</code> | § | § |
| <code>Readline Init File Syntax</code> | § | § |
| <code>Readline Interaction</code> | § | § |
| <code>Readline Killing Commands</code> | § | § |
| <code>Readline Movement Commands</code> | § | § |
| <code>Readline vi Mode</code> | § | § |
| <code>Redirections</code> | § | § |
| <code>Reporting Bugs</code> | § | § |
| <code>Reserved Word Index</code> | § | § |
| <code>Reserved Words</code> | § | § |
| <code>Sample Init File</code> | § | § |
| <code>Searching</code> | § | § |
| <code>Sharing Defaults</code> | § | § |
| <code>Shell Arithmetic</code> | § | § |
| <code>Shell Builtin Commands</code> | § | § |
| <code>Shell Commands</code> | § | § |
| <code>Shell Compatibility Mode</code> | § | § |
| <code>Shell Expansions</code> | § | § |
| <code>Shell Functions</code> | § | § |
| <code>Shell Operation</code> | § | § |
| <code>Shell Parameter Expansion</code> | § | § |
| <code>Shell Parameters</code> | § | § |
| <code>Shell Scripts</code> | § | § |
| <code>Shell Syntax</code> | § | § |
| <code>Shell Variables</code> | § | § |
| <code>Signals</code> | § | § |
| <code>Simple Command Expansion</code> | § | § |
| <code>Simple Commands</code> | § | § |
| <code>Single Quotes</code> | § | § |
| <code>Special Builtins</code> | § | § |
| <code>Special Parameters</code> | § | § |
| <code>Specifying the System Type</code> | § | § |
| <code>The Directory Stack</code> | § | § |
| <code>The Restricted Shell</code> | § | § |
| <code>The Set Builtin</code> | § | § |
| <code>The Shopt Builtin</code> | § | § |
| <code>Tilde Expansion</code> | § | § |
| <code>Top</code> | § | § |
| <code>Using History Interactively</code> | § | § |
| <code>Variable Index</code> | § | § |
| <code>What is Bash?</code> | § | § |
| <code>What is a shell?</code> | § | § |
| <code>What is an Interactive Shell?</code> | § | § |
| <code>Word Designators</code> | § | § |
| <code>Word Splitting</code> | § | § |

## POSIX mode item

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>01</code> | P | P |
| <code>02</code> | P | P |
| <code>03</code> | P | P |
| <code>04</code> | P | P |
| <code>05</code> | P | P |
| <code>06</code> | P | P |
| <code>07</code> | P | P |
| <code>08</code> | P | P |
| <code>09</code> | P | P |
| <code>10</code> | P | P |
| <code>11</code> | P | P |
| <code>12</code> | P | P |
| <code>13</code> | P | P |
| <code>14</code> | P | P |
| <code>15</code> | P | P |
| <code>16</code> | P | P |
| <code>17</code> | P | P |
| <code>18</code> | P | P |
| <code>19</code> | P | P |
| <code>20</code> | P | P |
| <code>21</code> | P | P |
| <code>22</code> | P | P |
| <code>23</code> | P | P |
| <code>24</code> | P | P |
| <code>25</code> | P | P |
| <code>26</code> | P | P |
| <code>27</code> | P | P |
| <code>28</code> | P | P |
| <code>29</code> | P | P |
| <code>30</code> | P | P |
| <code>31</code> | P | P |
| <code>32</code> | P | P |
| <code>33</code> | P | P |
| <code>34</code> | P | P |
| <code>35</code> | P | P |
| <code>36</code> | P | P |
| <code>37</code> | P | P |
| <code>38</code> | P | P |
| <code>39</code> | P | P |
| <code>40</code> | P | P |
| <code>41</code> | P | P |
| <code>42</code> | P | P |
| <code>43</code> | P | P |
| <code>44</code> | P | P |
| <code>45</code> | P | P |
| <code>46</code> | P | P |
| <code>47</code> | P | P |
| <code>48</code> | P | P |
| <code>49</code> | P | P |
| <code>50</code> | P | P |
| <code>51</code> | P | P |
| <code>52</code> | P | P |
| <code>53</code> | P | P |
| <code>54</code> | P | P |
| <code>55</code> | P | P |
| <code>56</code> | P | P |
| <code>57</code> | P | P |
| <code>58</code> | P | P |
| <code>59</code> | P | P |
| <code>60</code> | P | P |
| <code>61</code> | P | P |
| <code>62</code> | P | P |
| <code>63</code> | — | P |
| <code>64</code> | — | P |
| <code>65</code> | — | P |
| <code>66</code> | — | P |
| <code>67</code> | — | P |
| <code>68</code> | — | P |
| <code>69</code> | — | P |
| <code>70</code> | — | P |
| <code>71</code> | — | P |
| <code>72</code> | — | P |
| <code>73</code> | — | P |
| <code>74</code> | — | P |
| <code>75</code> | — | P |
| <code>76</code> | — | P |

## Parameter

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>$!</code> | ✓ **0.79** / **0.85** | ✓ **1.00↑** / **1.41↑** |
| <code>$#</code> | ✓ **0.74** / **1.11↑** | ✓ 0.58 / **1.26↑** |
| <code>$$</code> | ✓ 0.62 / 0.62 | ✓ **0.82** / **0.71** |
| <code>$*</code> | ✓ 0.47 / **0.75** | ✓ **0.68** / **1.34↑** |
| <code>$-</code> | ✓ 0.52 / **0.94** | ✓ 0.47 / 0.62 |
| <code>$0</code> | ✓ **0.73** / **0.84** | ✓ **0.68** / 0.65 |
| <code>$1 … ${10}</code> | ✓ 0.64 / 0.52 | ✓ 0.57 / **0.78** |
| <code>$?</code> | ✓ 0.48 / **0.74** | ✓ 0.55 / 0.59 |
| <code>$@</code> | ✓ **0.75** / 0.55 | ✓ **0.88** / **1.37↑** |
| <code>$_</code> | Δ | Δ |
| <code>${!a[@]}</code> | ✓ **0.71** / **0.85** | ✓ **0.67** / **1.02↑** |
| <code>${!name}</code> | ✓ 0.58 / 0.66 | ✓ **0.75** / 0.65 |
| <code>${!prefix*}</code> | Δ | Δ |
| <code>${!prefix@}</code> | Δ | Δ |
| <code>${#a[@]}</code> | ✓ 0.60 / **0.75** | ✓ 0.57 / 0.63 |
| <code>${#v}</code> | ✓ **0.80** / 0.60 | ✓ **0.81** / **0.71** |
| <code>${a[*]}</code> | ✓ 0.57 / **0.99** | ✓ 0.62 / **1.04↑** |
| <code>${a[@]}</code> | ✓ 0.64 / **1.10↑** | ✓ 0.60 / **0.89** |
| <code>${v##a*}</code> | ✓ 0.58 / **0.76** | ✓ 0.65 / **0.88** |
| <code>${v#a*}</code> | ✓ **0.67** / **0.88** | ✓ **0.77** / **0.72** |
| <code>${v%%*a}</code> | ✓ 0.63 / **0.77** | ✓ **0.70** / **0.89** |
| <code>${v%*a}</code> | ✓ **0.73** / **0.73** | ✓ 0.65 / **0.86** |
| <code>${v+a}</code> | ✓ 0.58 / 0.66 | ✓ **0.70** / **0.77** |
| <code>${v,,}</code> | ✓ 0.63 / **0.77** | ✓ **0.71** / **0.86** |
| <code>${v,}</code> | ✓ **0.72** / **0.68** | ✓ **0.73** / **0.75** |
| <code>${v-a}</code> | ✓ **0.79** / **0.72** | ✓ **0.78** / **0.87** |
| <code>${v/#a/X}</code> | ✓ **0.68** / **0.78** | ✓ **0.71** / **0.83** |
| <code>${v/%a/X}</code> | ✓ 0.56 / **0.74** | ✓ **0.69** / 0.55 |
| <code>${v//a/X}</code> | ✓ **0.72** / **0.79** | ✓ **0.71** / **0.72** |
| <code>${v/a/X}</code> | ✓ 0.66 / **0.80** | ✓ **0.75** / **0.80** |
| <code>${v: -2:1}</code> | ✓ 0.58 / **0.68** | ✓ **0.72** / 0.64 |
| <code>${v:+a}</code> | ✓ **0.70** / 0.66 | ✓ 0.66 / **0.72** |
| <code>${v:-a}</code> | ✓ 0.56 / **0.81** | ✓ **0.67** / **0.78** |
| <code>${v:1}</code> | ✓ **0.74** / 0.62 | ✓ **0.73** / 0.66 |
| <code>${v:=a}</code> | ✓ **0.76** / **0.72** | ✓ **0.67** / **0.79** |
| <code>${v:?a}</code> | ✓ 0.56 / **0.73** | ✓ 0.60 / **0.68** |
| <code>${v=a}</code> | ✓ 0.62 / **0.74** | ✓ **0.73** / **0.80** |
| <code>${v?a}</code> | ✓ 0.57 / **0.77** | ✓ 0.55 / **0.70** |
| <code>${v@A}</code> | Δ | Δ |
| <code>${v@E}</code> | ✓ 0.64 / **0.71** | ✓ **0.67** / **0.74** |
| <code>${v@K}</code> | Δ | Δ |
| <code>${v@L}</code> | ✓ **0.72** / **0.78** | ✓ **0.79** / **0.82** |
| <code>${v@P}</code> | ✓ **0.73** / **0.76** | ✓ 0.63 / **0.89** |
| <code>${v@Q}</code> | ✓ **0.68** / **0.73** | ✓ **0.69** / **0.83** |
| <code>${v@U}</code> | ✓ 0.66 / **0.78** | ✓ 0.58 / **0.69** |
| <code>${v@a}</code> | Δ | Δ |
| <code>${v@k}</code> | Δ | Δ |
| <code>${v@u}</code> | ✓ 0.63 / **0.78** | ✓ **0.75** / **0.91** |
| <code>${v^^}</code> | ✓ **0.71** / **0.78** | ✓ 0.58 / **0.83** |
| <code>${v^}</code> | ✓ **0.74** / **0.75** | ✓ **0.68** / **0.69** |
| <code>${v}</code> | ✓ **0.68** / 0.50 | ✓ 0.62 / **0.82** |

## Printf format

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>%</code> | ✓ **0.86** / **0.84** | ✓ 0.54 / **1.04↑** |
| <code>(fmt)T</code> | Δ | Δ |
| <code>A</code> | ✓ **0.93** / **0.73** | ✓ **1.00** / **0.83** |
| <code>E</code> | ✓ 0.61 / 0.61 | ✓ 0.54 / **0.87** |
| <code>G</code> | ✓ **0.90** / **0.95** | ✓ 0.58 / **0.97** |
| <code>Q</code> | ✓ 0.62 / **0.85** | ✓ 0.61 / **1.00** |
| <code>X</code> | ✓ **0.82** / **1.07↑** | ✓ 0.56 / **0.81** |
| <code>a</code> | ✓ 0.59 / **1.30↑** | ✓ **0.91** / **0.88** |
| <code>b</code> | ✓ 0.54 / 0.65 | ✓ **0.71** / **1.21↑** |
| <code>c</code> | ✓ **0.75** / **1.08↑** | ✓ **0.71** / **0.86** |
| <code>d</code> | ✓ 0.65 / **1.05↑** | ✓ **0.84** / **1.17↑** |
| <code>e</code> | ✓ **0.69** / **0.79** | ✓ 0.52 / **0.93** |
| <code>f</code> | ✓ 0.44 / **0.88** | ✓ 0.62 / **1.14↑** |
| <code>g</code> | ✓ 0.58 / **0.85** | ✓ **0.67** / **0.93** |
| <code>i</code> | ✓ 0.65 / **1.21↑** | ✓ 0.60 / **0.85** |
| <code>n</code> | ✓ **0.86** / **1.23↑** | ✓ **0.78** / **0.73** |
| <code>o</code> | ✓ 0.50 / **0.82** | ✓ 0.55 / **1.16↑** |
| <code>q</code> | ✓ 0.58 / **1.35↑** | ✓ 0.54 / **1.34↑** |
| <code>reuse format</code> | ✓ 0.61 / **1.55↑** | ✓ 0.65 / **0.97** |
| <code>s</code> | ✓ 0.61 / **0.71** | ✓ **0.67** / 0.65 |
| <code>u</code> | ✓ **0.78** / **0.80** | ✓ **0.80** / **0.75** |
| <code>width / precision</code> | ✓ 0.50 / **0.74** | ✓ **0.81** / **0.97** |
| <code>x</code> | ✓ **0.94** / **0.71** | ✓ **0.79** / **1.07↑** |

## Prompt escape

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>\!</code> | ✓ **0.77** | ✓ **0.89** |
| <code>\#</code> | ✓ **0.80** | ✓ **0.88** |
| <code>\$</code> | ✓ **0.79** | ✓ **1.00↑** |
| <code>\101</code> | ✓ **0.82** | ✓ 0.62 |
| <code>\@</code> | ✓ **0.80** | ✓ **0.75** |
| <code>\A</code> | ✓ **0.76** | ✓ **0.75** |
| <code>\D{%Y}</code> | ✓ **0.81** | ✓ **0.68** |
| <code>\H</code> | ✓ **0.79** | ✓ **0.75** |
| <code>\T</code> | ✓ **0.81** | ✓ **0.86** |
| <code>\V</code> | Δ | Δ |
| <code>\W</code> | ✓ **0.77** | ✓ **0.71** |
| <code>\[</code> | ✓ **0.78** | ✓ 0.66 |
| <code>\\</code> | ✓ **0.75** | ✓ **0.81** |
| <code>\]</code> | ✓ **0.75** | ✓ **0.82** |
| <code>\a</code> | ✓ **0.76** | ✓ **0.93** |
| <code>\d</code> | ✓ **0.81** | ✓ **1.10↑** |
| <code>\e</code> | ✓ **0.78** | ✓ **1.15↑** |
| <code>\h</code> | ✓ **0.80** | ✓ **0.84** |
| <code>\j</code> | ✓ **0.83** | ✓ **0.97** |
| <code>\l</code> | ✓ **0.81** | ✓ **0.79** |
| <code>\n</code> | ✓ **0.77** | ✓ **0.77** |
| <code>\r</code> | ✓ **0.82** | ✓ **1.09↑** |
| <code>\s</code> | ✓ **0.78** | ✓ **1.02↑** |
| <code>\t</code> | ✓ **0.78** | ✓ **0.82** |
| <code>\u</code> | ✓ **0.78** | ✓ **0.93** |
| <code>\v</code> | ✓ **0.81** | Δ |
| <code>\w</code> | ✓ **0.79** | ✓ **0.68** |

## Readline command

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>abort</code> | I 0.53 / 0.54 | Δ |
| <code>accept-line</code> | I 0.50 / 0.56 | I 0.62 / 0.67 |
| <code>alias-expand-line</code> | M | M |
| <code>arrow-key-prefix</code> | M | M |
| <code>backward-byte</code> | M | M |
| <code>backward-char</code> | I 0.54 / 0.51 | Δ |
| <code>backward-delete-char</code> | I 0.53 / 0.56 | I 0.54 / 0.61 |
| <code>backward-kill-line</code> | I 0.50 / 0.53 | I 0.52 / 0.48 |
| <code>backward-kill-word</code> | I 0.46 / 0.66 | Δ |
| <code>backward-word</code> | I 0.51 / 0.55 | Δ |
| <code>bash-vi-complete</code> | — | M |
| <code>beginning-of-history</code> | I 0.51 / 0.62 | Δ |
| <code>beginning-of-line</code> | I 0.51 / 0.56 | Δ |
| <code>bracketed-paste-begin</code> | I 0.55 / 0.56 | Δ |
| <code>call-last-kbd-macro</code> | M | M |
| <code>capitalize-word</code> | M | M |
| <code>character-search</code> | M | M |
| <code>character-search-backward</code> | M | M |
| <code>clear-display</code> | M | M |
| <code>clear-screen</code> | I 0.49 / 0.53 | I 0.53 / 0.51 |
| <code>complete</code> | I 0.52 / **0.75** | Δ |
| <code>complete-command</code> | M | M |
| <code>complete-filename</code> | M | M |
| <code>complete-hostname</code> | M | M |
| <code>complete-into-braces</code> | M | M |
| <code>complete-username</code> | M | M |
| <code>complete-variable</code> | M | M |
| <code>copy-backward-word</code> | M | M |
| <code>copy-forward-word</code> | M | M |
| <code>copy-region-as-kill</code> | I 0.52 / 0.52 | I 0.58 / 0.48 |
| <code>dabbrev-expand</code> | M | M |
| <code>delete-char</code> | I 0.53 / 0.51 | I **0.72** / **0.82** |
| <code>delete-char-or-list</code> | M | M |
| <code>delete-horizontal-space</code> | M | M |
| <code>digit-argument</code> | M | M |
| <code>display-shell-version</code> | M | M |
| <code>do-lowercase-version</code> | M | M |
| <code>downcase-word</code> | M | M |
| <code>dump-functions</code> | M | M |
| <code>dump-macros</code> | M | M |
| <code>dump-variables</code> | M | M |
| <code>dynamic-complete-history</code> | M | M |
| <code>edit-and-execute-command</code> | M | M |
| <code>emacs-editing-mode</code> | I 0.53 / 0.47 | I 0.56 / **0.81** |
| <code>end-kbd-macro</code> | M | M |
| <code>end-of-history</code> | I 0.52 / **0.69** | Δ |
| <code>end-of-line</code> | I 0.54 / 0.48 | Δ |
| <code>exchange-point-and-mark</code> | I 0.50 / **0.77** | I 0.54 / 0.59 |
| <code>execute-named-command</code> | — | M |
| <code>export-completions</code> | — | M |
| <code>fetch-history</code> | M | M |
| <code>forward-backward-delete-char</code> | M | M |
| <code>forward-byte</code> | M | M |
| <code>forward-char</code> | I 0.47 / 0.56 | Δ |
| <code>forward-search-history</code> | M | M |
| <code>forward-word</code> | I 0.57 / 0.47 | Δ |
| <code>glob-complete-word</code> | M | M |
| <code>glob-expand-word</code> | M | M |
| <code>glob-list-expansions</code> | M | M |
| <code>history-and-alias-expand-line</code> | M | M |
| <code>history-expand-line</code> | M | M |
| <code>history-search-backward</code> | M | M |
| <code>history-search-forward</code> | M | M |
| <code>history-substring-search-backward</code> | M | M |
| <code>history-substring-search-forward</code> | M | M |
| <code>insert-comment</code> | I 0.56 / 0.59 | Δ |
| <code>insert-completions</code> | M | M |
| <code>insert-last-argument</code> | M | M |
| <code>kill-line</code> | I 0.57 / 0.60 | I 0.53 / 0.56 |
| <code>kill-region</code> | I 0.56 / 0.49 | I 0.55 / 0.51 |
| <code>kill-whole-line</code> | M | M |
| <code>kill-word</code> | I 0.48 / 0.51 | Δ |
| <code>magic-space</code> | M | M |
| <code>menu-complete</code> | M | M |
| <code>menu-complete-backward</code> | M | M |
| <code>next-history</code> | I 0.51 / 0.61 | Δ |
| <code>next-screen-line</code> | M | M |
| <code>non-incremental-forward-search-history</code> | M | M |
| <code>non-incremental-forward-search-history-again</code> | M | M |
| <code>non-incremental-reverse-search-history</code> | M | M |
| <code>non-incremental-reverse-search-history-again</code> | M | M |
| <code>old-menu-complete</code> | M | M |
| <code>operate-and-get-next</code> | M | M |
| <code>overwrite-mode</code> | M | M |
| <code>possible-command-completions</code> | M | M |
| <code>possible-completions</code> | I 0.52 / 0.56 | Δ |
| <code>possible-filename-completions</code> | M | M |
| <code>possible-hostname-completions</code> | M | M |
| <code>possible-username-completions</code> | M | M |
| <code>possible-variable-completions</code> | M | M |
| <code>previous-history</code> | I 0.51 / 0.61 | Δ |
| <code>previous-screen-line</code> | M | M |
| <code>print-last-kbd-macro</code> | M | M |
| <code>quoted-insert</code> | I 0.50 / 0.55 | I 0.51 / **0.69** |
| <code>re-read-init-file</code> | M | M |
| <code>redraw-current-line</code> | I 0.51 / 0.50 | I 0.62 / 0.58 |
| <code>reverse-search-history</code> | I 0.55 / 0.47 | I **0.67** / **0.78** |
| <code>revert-line</code> | M | M |
| <code>self-insert</code> | I 0.56 / 0.52 | I 0.60 / 0.50 |
| <code>set-mark</code> | I 0.64 / 0.63 | Δ |
| <code>shell-backward-kill-word</code> | M | M |
| <code>shell-backward-word</code> | M | M |
| <code>shell-expand-line</code> | M | M |
| <code>shell-forward-word</code> | M | M |
| <code>shell-kill-word</code> | M | M |
| <code>shell-transpose-words</code> | M | M |
| <code>skip-csi-sequence</code> | M | M |
| <code>spell-correct-word</code> | M | M |
| <code>start-kbd-macro</code> | M | M |
| <code>tab-insert</code> | M | M |
| <code>tilde-expand</code> | M | M |
| <code>transpose-chars</code> | I 0.49 / **0.74** | I 0.55 / 0.48 |
| <code>transpose-words</code> | M | M |
| <code>tty-status</code> | M | M |
| <code>undo</code> | M | M |
| <code>universal-argument</code> | M | M |
| <code>unix-filename-rubout</code> | M | M |
| <code>unix-line-discard</code> | I 0.51 / 0.58 | I 0.64 / **0.73** |
| <code>unix-word-rubout</code> | I 0.55 / 0.53 | I 0.49 / **0.70** |
| <code>upcase-word</code> | M | M |
| <code>vi-append-eol</code> | I 0.52 / 0.60 | I **0.70** / **0.72** |
| <code>vi-append-mode</code> | I 0.51 / 0.57 | I 0.42 / **0.87** |
| <code>vi-arg-digit</code> | M | M |
| <code>vi-bWord</code> | M | M |
| <code>vi-back-to-indent</code> | M | M |
| <code>vi-backward-bigword</code> | M | M |
| <code>vi-backward-word</code> | M | M |
| <code>vi-bword</code> | M | M |
| <code>vi-change-case</code> | M | M |
| <code>vi-change-char</code> | M | M |
| <code>vi-change-to</code> | M | M |
| <code>vi-char-search</code> | M | M |
| <code>vi-column</code> | M | M |
| <code>vi-complete</code> | M | M |
| <code>vi-delete</code> | M | M |
| <code>vi-delete-to</code> | M | M |
| <code>vi-eWord</code> | M | M |
| <code>vi-edit-and-execute-command</code> | M | M |
| <code>vi-editing-mode</code> | I 0.52 / 0.66 | I 0.47 / 0.62 |
| <code>vi-end-bigword</code> | M | M |
| <code>vi-end-word</code> | M | M |
| <code>vi-eof-maybe</code> | M | M |
| <code>vi-eword</code> | M | M |
| <code>vi-fWord</code> | M | M |
| <code>vi-fetch-history</code> | M | M |
| <code>vi-first-print</code> | M | M |
| <code>vi-forward-bigword</code> | M | M |
| <code>vi-forward-word</code> | M | M |
| <code>vi-fword</code> | M | M |
| <code>vi-goto-mark</code> | M | M |
| <code>vi-insert-beg</code> | I 0.40 / 0.62 | I 0.46 / **0.97** |
| <code>vi-insertion-mode</code> | I 0.56 / 0.62 | I 0.52 / 0.54 |
| <code>vi-match</code> | M | M |
| <code>vi-movement-mode</code> | I 0.56 / 0.44 | I 0.61 / 0.53 |
| <code>vi-next-word</code> | M | M |
| <code>vi-overstrike</code> | M | M |
| <code>vi-overstrike-delete</code> | M | M |
| <code>vi-prev-word</code> | M | M |
| <code>vi-put</code> | M | M |
| <code>vi-redo</code> | M | M |
| <code>vi-replace</code> | M | M |
| <code>vi-rubout</code> | M | M |
| <code>vi-search</code> | M | M |
| <code>vi-search-again</code> | M | M |
| <code>vi-set-mark</code> | M | M |
| <code>vi-subst</code> | M | M |
| <code>vi-tilde-expand</code> | M | M |
| <code>vi-undo</code> | M | M |
| <code>vi-unix-word-rubout</code> | M | M |
| <code>vi-yank-arg</code> | M | M |
| <code>vi-yank-pop</code> | M | M |
| <code>vi-yank-to</code> | M | M |
| <code>yank</code> | I 0.49 / 0.66 | I **0.82** / 0.54 |
| <code>yank-last-arg</code> | M | M |
| <code>yank-nth-arg</code> | M | M |
| <code>yank-pop</code> | M | M |

## Readline variable

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>bell-style</code> | M | M |
| <code>bind-tty-special-chars</code> | M | M |
| <code>blink-matching-paren</code> | M | M |
| <code>byte-oriented</code> | M | M |
| <code>colored-completion-prefix</code> | M | M |
| <code>colored-stats</code> | M | M |
| <code>comment-begin</code> | M | M |
| <code>completion-display-width</code> | M | M |
| <code>completion-ignore-case</code> | M | M |
| <code>completion-map-case</code> | M | M |
| <code>completion-prefix-display-length</code> | M | M |
| <code>completion-query-items</code> | M | M |
| <code>convert-meta</code> | M | M |
| <code>disable-completion</code> | I 0.53 / 0.65 | I 0.62 / 0.65 |
| <code>echo-control-characters</code> | M | M |
| <code>editing-mode</code> | I 0.57 / 0.66 | I **0.73** / **0.67** |
| <code>emacs-mode-string</code> | M | M |
| <code>enable-active-region</code> | M | M |
| <code>enable-bracketed-paste</code> | I 0.65 / **0.84** | I 0.59 / **0.84** |
| <code>enable-keypad</code> | M | M |
| <code>enable-meta-key</code> | M | M |
| <code>expand-tilde</code> | M | M |
| <code>force-meta-prefix</code> | — | M |
| <code>history-preserve-point</code> | M | M |
| <code>history-size</code> | M | M |
| <code>horizontal-scroll-mode</code> | M | M |
| <code>input-meta</code> | M | M |
| <code>keymap</code> | I 0.53 / **0.69** | I 0.66 / 0.60 |
| <code>keyseq-timeout</code> | I 0.58 / 0.57 | I 0.65 / **0.75** |
| <code>mark-directories</code> | M | M |
| <code>mark-modified-lines</code> | M | M |
| <code>mark-symlinked-directories</code> | M | M |
| <code>match-hidden-files</code> | M | M |
| <code>menu-complete-display-prefix</code> | M | M |
| <code>meta-flag</code> | M | M |
| <code>output-meta</code> | M | M |
| <code>page-completions</code> | M | M |
| <code>prefer-visible-bell</code> | M | M |
| <code>print-completions-horizontally</code> | M | M |
| <code>revert-all-at-newline</code> | M | M |
| <code>search-ignore-case</code> | — | M |
| <code>show-all-if-ambiguous</code> | M | M |
| <code>show-all-if-unmodified</code> | M | M |
| <code>show-mode-in-prompt</code> | M | M |
| <code>skip-completed-text</code> | M | M |
| <code>vi-cmd-mode-string</code> | M | M |
| <code>vi-ins-mode-string</code> | M | M |
| <code>visible-stats</code> | M | M |

## Redirection

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>&amp;&gt;</code> | ✓ **0.71** / **0.68** | ✓ **0.85** / **0.91** |
| <code>&amp;&gt;&gt;</code> | ✓ 0.63 / **0.85** | ✓ **0.77** / **0.81** |
| <code>/dev/fd/N</code> | ✓ **0.72** / **1.27↑** | ✓ **0.88** / **1.01↑** |
| <code>/dev/tcp/host/port</code> | Δ | Δ |
| <code>/dev/udp/host/port</code> | Δ | Δ |
| <code>&lt;</code> | ✓ 0.55 / 0.55 | ✓ **0.83** / 0.62 |
| <code>&lt;&lt;-</code> | ✓ 0.48 / 0.66 | ✓ **0.72** / **0.70** |
| <code>&lt;&lt;&lt;</code> | ✓ 0.44 / **0.82** | ✓ **0.78** / **0.79** |
| <code>&lt;&gt;</code> | ✓ 0.63 / **1.27↑** | ✓ **0.82** / **1.00↑** |
| <code>&gt;</code> | ✓ **0.90** / **0.85** | ✓ **0.77** / **1.44↑** |
| <code>&gt;&gt;</code> | ✓ **0.71** / **0.82** | ✓ **0.82** / **0.90** |
| <code>&gt;&#124;</code> | ✓ **0.75** / **0.94** | ✓ **0.84** / **0.86** |
| <code>here-document</code> | ✓ 0.50 / 0.63 | ✓ **0.69** / 0.64 |
| <code>n&lt;&amp;-</code> | ✓ 0.63 / **1.48↑** | ✓ 0.60 / **0.80** |
| <code>n&lt;&amp;m</code> | ✓ 0.35 / 0.63 | ✓ **0.80** / **0.80** |
| <code>n&lt;&amp;m-</code> | ✓ 0.39 / 0.47 | ✓ **0.69** / **0.73** |
| <code>n&gt;&amp;-</code> | ✓ 0.65 / **0.77** | ✓ **0.88** / **0.77** |
| <code>n&gt;&amp;m</code> | ✓ **1.05↑** / **0.96** | ✓ 0.66 / **1.09↑** |
| <code>n&gt;&amp;m-</code> | ✓ 0.58 / **0.91** | ✓ **0.80** / **0.98** |
| <code>quoted here-document</code> | ✓ 0.54 / 0.65 | ✓ **0.78** / **0.87** |
| <code>{varname} descriptor</code> | ✓ **0.68** / **1.26↑** | ✓ **0.93** / **1.05↑** |

## Reserved word

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>!</code> | ✓ **0.69** / **0.79** | ✓ 0.62 / **0.73** |
| <code>[[</code> | ✓ 0.44 / **1.07↑** | ✓ **0.77** / **0.94** |
| <code>]]</code> | ✓ 0.44 / **1.07↑** | ✓ **0.77** / **0.94** |
| <code>case</code> | ✓ 0.54 / **0.81** | ✓ 0.52 / **1.21↑** |
| <code>coproc</code> | ✓ 0.61 / 0.64 | ✓ 0.64 / **0.73** |
| <code>do</code> | ✓ 0.41 / **0.93** | ✓ 0.58 / **0.92** |
| <code>done</code> | ✓ 0.41 / **0.93** | ✓ 0.58 / **0.92** |
| <code>elif</code> | ✓ 0.59 / 0.65 | ✓ 0.58 / **1.00** |
| <code>else</code> | ✓ 0.59 / 0.65 | ✓ 0.58 / **1.00** |
| <code>esac</code> | ✓ 0.54 / **0.81** | ✓ 0.52 / **1.21↑** |
| <code>fi</code> | ✓ 0.59 / 0.65 | ✓ 0.58 / **1.00** |
| <code>for</code> | ✓ 0.41 / **0.93** | ✓ 0.58 / **0.92** |
| <code>function</code> | ✓ 0.42 / **1.27↑** | ✓ **0.77** / **0.79** |
| <code>if</code> | ✓ 0.59 / 0.65 | ✓ 0.58 / **1.00** |
| <code>in</code> | ✓ 0.41 / **0.93** | ✓ 0.58 / **0.92** |
| <code>select</code> | ✓ **0.77** / **0.71** | ✓ **0.73** / **0.80** |
| <code>then</code> | ✓ 0.59 / 0.65 | ✓ 0.58 / **1.00** |
| <code>time</code> | ✓ **0.71** / **1.12↑** | ✓ **0.88** / **1.05↑** |
| <code>until</code> | ✓ 0.62 / **0.84** | ✓ 0.55 / **0.72** |
| <code>while</code> | ✓ 0.47 / **1.39↑** | ✓ **0.70** / **0.90** |
| <code>{</code> | ✓ **0.81** / **1.18↑** | ✓ 0.61 / **0.85** |
| <code>}</code> | ✓ **0.81** / **1.18↑** | ✓ 0.61 / **0.85** |

## Shell language

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>ANSI-C quoting</code> | ✓ **0.80** / **1.16↑** | ✓ 0.61 / 0.54 |
| <code>Bash startup via BASH_ENV</code> | ✓ **0.87** / **0.73** | ✓ **0.75** / 0.65 |
| <code>DEBUG trap</code> | ✓ 0.52 / **0.86** | ✓ 0.60 / **0.75** |
| <code>ERR trap</code> | ✓ **0.68** / **1.27↑** | ✓ 0.67 / **0.95** |
| <code>EXIT trap</code> | ✓ **0.75** / **0.92** | ✓ 0.53 / **0.99** |
| <code>IFS word splitting</code> | ✓ **0.80** / **0.99** | ✓ **0.71** / **1.00** |
| <code>PATH command search</code> | ✓ **0.81** / **0.94** | ✓ **1.01↑** / **0.97** |
| <code>POSIX mode</code> | Δ | Δ |
| <code>POSIX special builtin failure</code> | Δ | Δ |
| <code>REPLY substitution (5.3)</code> | N | Δ |
| <code>RETURN trap</code> | ✓ 0.63 / **0.68** | ✓ 0.58 / **0.84** |
| <code>alias parse time</code> | ✓ 0.58 / **0.87** | ✓ 0.57 / **1.35↑** |
| <code>arithmetic base notation</code> | ✓ **0.94** / **0.87** | ✓ **0.95** / **0.96** |
| <code>arithmetic command status</code> | ✓ 0.51 / **1.16↑** | ✓ **0.76** / **1.11↑** |
| <code>arithmetic expansion</code> | ✓ **0.74** / **0.85** | ✓ 0.62 / 0.66 |
| <code>arithmetic for</code> | ✓ **0.89** / **0.83** | ✓ **0.69** / **0.94** |
| <code>arithmetic integer division and remainder</code> | ✓ **0.71** / **1.06↑** | ✓ 0.59 / 0.52 |
| <code>arithmetic short circuit</code> | ✓ **0.81** / **0.89** | ✓ 0.62 / **0.88** |
| <code>array append</code> | ✓ 0.59 / 0.61 | ✓ **0.68** / 0.54 |
| <code>array slice and negative index</code> | ✓ 0.49 / **0.99** | ✓ 0.65 / **1.49↑** |
| <code>associative arrays</code> | ✓ 0.60 / **1.76↑** | ✓ 0.64 / **0.97** |
| <code>background and wait</code> | ✓ 0.65 / **0.98** | ✓ **0.79** / **1.24↑** |
| <code>backquote substitution</code> | ✓ 0.42 / 0.31 | ✓ 0.43 / 0.38 |
| <code>brace alternation</code> | ✓ 0.63 / **1.24↑** | ✓ **0.68** / **0.99** |
| <code>brace group</code> | ✓ **0.81** / **1.18↑** | ✓ 0.61 / **0.85** |
| <code>brace sequence</code> | ✓ 0.62 / 0.65 | ✓ 0.56 / **1.45↑** |
| <code>bracket expressions and classes</code> | ✓ **0.78** / 0.62 | ✓ **0.83** / **0.93** |
| <code>case ;&amp; fallthrough</code> | ✓ 0.58 / 0.64 | ✓ 0.65 / **0.70** |
| <code>case ;;&amp; retest</code> | ✓ 0.52 / **0.91** | ✓ 0.53 / **0.86** |
| <code>case in esac</code> | ✓ 0.54 / **0.81** | ✓ 0.52 / **1.21↑** |
| <code>case patterns from variables</code> | ✓ 0.58 / **1.29↑** | ✓ 0.55 / **0.80** |
| <code>command substitution</code> | ✓ 0.39 / 0.39 | ✓ 0.37 / 0.33 |
| <code>comments</code> | ✓ 0.43 / **1.29↑** | ✓ 0.61 / **0.90** |
| <code>completion filters</code> | ✓ 0.54 / **0.99** | ✓ **0.85** / **0.80** |
| <code>conditional command</code> | ✓ 0.44 / **1.07↑** | ✓ **0.77** / **0.94** |
| <code>coproc</code> | ✓ 0.61 / 0.64 | ✓ 0.64 / **0.73** |
| <code>coprocess input/output descriptors</code> | ✓ 0.61 / 0.64 | ✓ 0.64 / **0.73** |
| <code>current environment substitution (5.3)</code> | N | Δ |
| <code>deferred bad substitution</code> | ✓ **0.93** / **0.75** | ✓ 0.60 / **0.80** |
| <code>directory cd -</code> | ✓ **0.84** / **0.84** | ✓ **0.94** / **0.88** |
| <code>directory stack rotation</code> | ✓ **0.72** / **0.98** | ✓ **0.96** / **0.95** |
| <code>double quotes</code> | ✓ **0.69** / 0.58 | ✓ **0.72** / **1.25↑** |
| <code>dynamic function scope</code> | ✓ **0.79** / **1.10↑** | ✓ 0.65 / **0.76** |
| <code>escape character</code> | ✓ 0.51 / 0.31 | ✓ 0.62 / **0.83** |
| <code>escape sequences in printf %b</code> | ✓ 0.64 / **1.10↑** | ✓ 0.65 / **0.71** |
| <code>escaped backslash filenames</code> | ✓ **0.69** / **0.93** | ✓ **0.72** / **0.96** |
| <code>exit status 126</code> | ✓ 0.50 / 0.57 | ✓ **0.75** / **0.77** |
| <code>exit status 127</code> | ✓ 0.34 / 0.24 | ✓ 0.33 / 0.40 |
| <code>extended globs</code> | ✓ 0.54 / **0.94** | ✓ 0.59 / **1.22↑** |
| <code>extglob !(...)</code> | ✓ 0.62 / **0.98** | ✓ **0.74** / **1.08↑** |
| <code>extglob *(...)</code> | ✓ **1.04↑** / **0.73** | ✓ **0.67** / 0.65 |
| <code>extglob +(...)</code> | ✓ **0.71** / **0.84** | ✓ **0.81** / **0.87** |
| <code>extglob ?(...)</code> | ✓ 0.60 / **1.21↑** | ✓ 0.66 / 0.59 |
| <code>extglob @(...)</code> | ✓ 0.58 / **0.81** | ✓ 0.65 / **1.22↑** |
| <code>filename wildcards</code> | ✓ 0.52 / 0.67 | ✓ **0.68** / **0.97** |
| <code>for do done</code> | ✓ 0.41 / **0.93** | ✓ 0.58 / **0.92** |
| <code>function and return</code> | ✓ 0.42 / **1.27↑** | ✓ **0.77** / **0.79** |
| <code>function argument restoration</code> | ✓ **0.75** / **0.67** | ✓ 0.49 / **0.69** |
| <code>function definition redirections</code> | ✓ **0.77** / **1.48↑** | ✓ **0.79** / **0.97** |
| <code>function export</code> | ✓ 0.57 / 0.34 | ✓ 0.56 / 0.58 |
| <code>group redirections</code> | ✓ **0.76** / **1.38↑** | ✓ **0.92** / **0.95** |
| <code>grouped pipeline status</code> | ✓ 0.63 / **0.89** | ✓ **0.70** / **0.81** |
| <code>history event expansion</code> | ✓ **0.69** / **0.79** | ✓ **0.69** / **0.95** |
| <code>history modifiers</code> | ✓ 0.63 / 0.56 | ✓ 0.54 / **1.32↑** |
| <code>history search</code> | ✓ 0.66 / **1.38↑** | ✓ **0.84** / **1.03↑** |
| <code>history word designator</code> | ✓ **0.80** / **1.15↑** | ✓ **0.78** / **1.34↑** |
| <code>if then elif else fi</code> | ✓ 0.59 / 0.65 | ✓ 0.58 / **1.00** |
| <code>indexed arrays</code> | ✓ **0.73** / **0.74** | ✓ 0.66 / **0.88** |
| <code>line continuation</code> | ✓ 0.63 / **0.82** | ✓ **0.77** / **0.90** |
| <code>lists and short-circuit</code> | ✓ **0.84** / **1.21↑** | ✓ 0.66 / **0.93** |
| <code>local variable inheritance</code> | ✓ 0.56 / **1.02↑** | ✓ **0.91** / 0.57 |
| <code>locale translation syntax</code> | ✓ 0.55 / **0.89** | ✓ **0.67** / 0.65 |
| <code>merged stderr pipeline</code> | ✓ 0.61 / **0.95** | ✓ **0.76** / **0.95** |
| <code>nameref</code> | ✓ 0.32 / **1.34↑** | ✓ **0.75** / **0.96** |
| <code>negation</code> | ✓ **0.69** / **0.79** | ✓ 0.62 / **0.73** |
| <code>pathname expansion of leading dots</code> | ✓ **0.78** / 0.66 | ✓ **0.81** / **1.09↑** |
| <code>pipeline</code> | ✓ **0.73** / **0.80** | ✓ **1.07↑** / **0.84** |
| <code>pipeline stage statuses</code> | ✓ 0.47 / **0.67** | ✓ 0.64 / **0.71** |
| <code>primary prompt parameter expansion</code> | ✓ 0.39 / **1.03↑** | ✓ 0.64 / **1.15↑** |
| <code>printf %q round trip</code> | ✓ 0.64 / **1.16↑** | ✓ 0.60 / **0.79** |
| <code>printf reusable quoting</code> | ✓ 0.64 / **1.16↑** | ✓ 0.60 / **0.79** |
| <code>printf timestamp</code> | Δ | Δ |
| <code>priorities of lookup</code> | ✓ **0.89** / **1.05↑** | ✓ **0.95** / **0.87** |
| <code>process substitution input</code> | ✓ **0.80** / **0.88** | ✓ **0.84** / **0.89** |
| <code>process substitution output</code> | ✓ 0.54 / **0.80** | ✓ **0.83** / **0.92** |
| <code>programmable completion callback</code> | ✓ **0.74** / **0.88** | ✓ 0.58 / **0.67** |
| <code>quote removal</code> | ✓ **0.85** / **1.47↑** | ✓ 0.58 / **0.69** |
| <code>quoted and unquoted here-documents</code> | ✓ 0.54 / 0.65 | ✓ **0.78** / **0.87** |
| <code>restricted shell</code> | E | E |
| <code>script fallback without shebang</code> | ✓ **0.88** / **1.03↑** | ✓ **0.71** / **1.06↑** |
| <code>select</code> | ✓ **0.77** / **0.71** | ✓ **0.73** / **0.80** |
| <code>shell environment isolation</code> | ✓ 0.57 / **0.75** | ✓ 0.58 / **1.00** |
| <code>shell script source</code> | ✓ **0.68** / **1.36↑** | ✓ **0.95** / **1.09↑** |
| <code>signal exit status</code> | E | E |
| <code>signal trap</code> | ✓ **0.69** / **1.13↑** | ✓ 0.58 / **0.92** |
| <code>simple commands and assignments</code> | ✓ 0.63 / **0.72** | ✓ **0.83** / **0.89** |
| <code>single quotes</code> | ✓ 0.41 / **1.21↑** | ✓ **0.80** / **0.92** |
| <code>subshell exit isolation</code> | ✓ 0.62 / **0.88** | ✓ **0.70** / **0.93** |
| <code>subshell scope</code> | ✓ 0.55 / **1.03↑** | ✓ 0.61 / **0.94** |
| <code>terminal signal ignore and reset</code> | ✓ 0.63 / **0.69** | ✓ **0.94** / **1.01↑** |
| <code>tilde current and previous directories</code> | ✓ 0.61 / **0.93** | ✓ **0.80** / **0.89** |
| <code>tilde expansion</code> | ✓ 0.49 / **0.87** | ✓ **0.77** / **0.75** |
| <code>tilde named user</code> | Δ | Δ |
| <code>time -p</code> | ✓ **0.86** / **0.69** | ✓ **0.91** / **0.79** |
| <code>time keyword</code> | ✓ **0.71** / **1.12↑** | ✓ **0.88** / **1.05↑** |
| <code>until</code> | ✓ 0.62 / **0.84** | ✓ 0.55 / **0.72** |
| <code>wait -n</code> | ✓ **0.97** / **0.99** | ✓ **0.97** / **1.02↑** |
| <code>wait -p</code> | ✓ **0.98** / **1.00** | ✓ **0.98** / **0.98** |
| <code>while</code> | ✓ 0.47 / **1.39↑** | ✓ **0.70** / **0.90** |

## Signal

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>ABRT</code> | ✓ 0.59 / **0.71** | ✓ 0.59 / **0.89** |
| <code>ALRM</code> | ✓ 0.58 / **0.75** | ✓ 0.62 / **0.93** |
| <code>BUS</code> | ✓ 0.59 / **0.73** | ✓ **0.69** / **0.84** |
| <code>CHLD</code> | Δ | Δ |
| <code>CONT</code> | ✓ 0.59 / **0.75** | ✓ 0.59 / **0.92** |
| <code>FPE</code> | ✓ 0.57 / **0.75** | ✓ 0.66 / **0.96** |
| <code>HUP</code> | ✓ 0.60 / **0.76** | ✓ 0.61 / **0.81** |
| <code>ILL</code> | ✓ 0.59 / **0.78** | ✓ 0.65 / **0.85** |
| <code>INT</code> | ✓ 0.60 / **0.73** | ✓ 0.59 / **0.89** |
| <code>IO</code> | ✓ 0.60 / **0.78** | ✓ 0.59 / **0.92** |
| <code>KILL</code> | I 0.55 / 0.67 | I 0.56 / **0.76** |
| <code>PIPE</code> | ✓ 0.61 / **0.73** | ✓ 0.62 / **0.97** |
| <code>PROF</code> | ✓ 0.61 / **0.77** | ✓ 0.62 / **0.93** |
| <code>PWR</code> | Δ | Δ |
| <code>QUIT</code> | ✓ 0.59 / **0.77** | ✓ 0.62 / **0.94** |
| <code>RTMAX</code> | Δ | Δ |
| <code>RTMAX-1</code> | Δ | Δ |
| <code>RTMAX-10</code> | Δ | Δ |
| <code>RTMAX-11</code> | Δ | Δ |
| <code>RTMAX-12</code> | Δ | Δ |
| <code>RTMAX-13</code> | Δ | Δ |
| <code>RTMAX-14</code> | Δ | Δ |
| <code>RTMAX-2</code> | Δ | Δ |
| <code>RTMAX-3</code> | Δ | Δ |
| <code>RTMAX-4</code> | Δ | Δ |
| <code>RTMAX-5</code> | Δ | Δ |
| <code>RTMAX-6</code> | Δ | Δ |
| <code>RTMAX-7</code> | Δ | Δ |
| <code>RTMAX-8</code> | Δ | Δ |
| <code>RTMAX-9</code> | Δ | Δ |
| <code>RTMIN</code> | Δ | Δ |
| <code>RTMIN+1</code> | Δ | Δ |
| <code>RTMIN+10</code> | Δ | Δ |
| <code>RTMIN+11</code> | Δ | Δ |
| <code>RTMIN+12</code> | Δ | Δ |
| <code>RTMIN+13</code> | Δ | Δ |
| <code>RTMIN+14</code> | Δ | Δ |
| <code>RTMIN+15</code> | Δ | Δ |
| <code>RTMIN+2</code> | Δ | Δ |
| <code>RTMIN+3</code> | Δ | Δ |
| <code>RTMIN+4</code> | Δ | Δ |
| <code>RTMIN+5</code> | Δ | Δ |
| <code>RTMIN+6</code> | Δ | Δ |
| <code>RTMIN+7</code> | Δ | Δ |
| <code>RTMIN+8</code> | Δ | Δ |
| <code>RTMIN+9</code> | Δ | Δ |
| <code>SEGV</code> | ✓ 0.59 / **0.74** | ✓ 0.62 / **0.89** |
| <code>STKFLT</code> | Δ | Δ |
| <code>STOP</code> | I 0.56 / 0.66 | I 0.58 / **0.73** |
| <code>SYS</code> | ✓ 0.60 / **0.84** | ✓ 0.62 / **0.83** |
| <code>TERM</code> | ✓ 0.59 / **0.77** | ✓ 0.62 / **0.85** |
| <code>TRAP</code> | ✓ 0.59 / **0.73** | ✓ 0.61 / **1.00↑** |
| <code>TSTP</code> | ✓ 0.58 / **0.74** | ✓ 0.57 / **0.80** |
| <code>TTIN</code> | ✓ 0.60 / **0.75** | ✓ 0.59 / **0.97** |
| <code>TTOU</code> | ✓ 0.59 / **0.74** | ✓ **0.67** / **0.79** |
| <code>URG</code> | ✓ 0.60 / **0.74** | ✓ **0.71** / **0.91** |
| <code>USR1</code> | ✓ 0.60 / **0.72** | ✓ 0.65 / **0.88** |
| <code>USR2</code> | ✓ 0.58 / **0.73** | ✓ 0.59 / **1.13↑** |
| <code>VTALRM</code> | ✓ 0.59 / **0.84** | ✓ 0.62 / **0.90** |
| <code>WINCH</code> | ✓ 0.55 / **0.73** | ✓ 0.64 / **0.73** |
| <code>XCPU</code> | ✓ 0.62 / **0.76** | ✓ 0.60 / **0.84** |
| <code>XFSZ</code> | ✓ 0.57 / **0.91** | ✓ 0.59 / **0.85** |

## Terminal

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>foreground pipeline</code> | ✓ **0.99** | ✓ **0.94** |
| <code>interactive alias and signal</code> | ✓ **1.07↑** | ✓ **0.92** |
| <code>suspend and resume</code> | ✓ **0.91** | ✓ **0.69** |

## UTF-8 text

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>character length and slice in C locale</code> | ◎ | ◎ |
| <code>multibyte IFS after whitespace</code> | ◎ | ◎ |

## Variable

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>BASH</code> | ✓ 0.59 / **0.69** | ✓ 0.53 / 0.48 |
| <code>BASHOPTS</code> | ✓ 0.53 / **0.76** | ✓ 0.61 / **0.72** |
| <code>BASHPID</code> | ✓ 0.57 / **0.85** | ✓ 0.66 / **0.70** |
| <code>BASH_ALIASES</code> | Δ | Δ |
| <code>BASH_ARGC</code> | ✓ **0.84** / **0.85** | ✓ 0.64 / **0.91** |
| <code>BASH_ARGV</code> | ✓ **0.69** / **1.10↑** | ✓ **0.68** / **0.93** |
| <code>BASH_ARGV0</code> | ✓ 0.60 / **1.32↑** | ✓ 0.66 / **0.99** |
| <code>BASH_CMDS</code> | Δ | Δ |
| <code>BASH_COMMAND</code> | ✓ **0.68** / 0.64 | ✓ 0.64 / **0.80** |
| <code>BASH_COMPAT</code> | Δ | Δ |
| <code>BASH_ENV</code> | ✓ **0.87** / **0.73** | ✓ **0.75** / 0.65 |
| <code>BASH_EXECUTION_STRING</code> | Δ | Δ |
| <code>BASH_LINENO</code> | ✓ 0.53 / **1.23↑** | ✓ 0.63 / **1.05↑** |
| <code>BASH_LOADABLES_PATH</code> | V | V |
| <code>BASH_MONOSECONDS</code> | — | Δ |
| <code>BASH_REMATCH</code> | ✓ 0.63 / **1.15↑** | ✓ **0.68** / **0.89** |
| <code>BASH_SOURCE</code> | ✓ 0.59 / **0.84** | ✓ 0.61 / **0.95** |
| <code>BASH_SUBSHELL</code> | ✓ 0.63 / **0.74** | ✓ 0.64 / **0.71** |
| <code>BASH_TRAPSIG</code> | — | Δ |
| <code>BASH_VERSINFO</code> | ✓ **0.80** / **0.85** | ✓ 0.62 / **0.83** |
| <code>BASH_VERSION</code> | ✓ 0.61 / 0.59 | ✓ 0.64 / **0.95** |
| <code>BASH_XTRACEFD</code> | ✓ **0.74** / **1.07↑** | ✓ **0.84** / **1.04↑** |
| <code>CDPATH</code> | ✓ 0.67 / **0.89** | ✓ **0.95** / **0.96** |
| <code>CHILD_MAX</code> | ✓ **0.87** / **0.86** | ✓ **0.91** / **0.94** |
| <code>COLUMNS</code> | ✓ **0.90** | ✓ **0.94** |
| <code>COMPREPLY</code> | ✓ **0.74** / **0.88** | ✓ 0.58 / **0.67** |
| <code>COMP_CWORD</code> | ✓ **0.84** | ✓ **0.73** |
| <code>COMP_KEY</code> | ✓ **0.83** | ✓ 0.67 |
| <code>COMP_LINE</code> | ✓ **0.86** | ✓ **0.71** |
| <code>COMP_POINT</code> | ✓ **0.69** | ✓ **0.71** |
| <code>COMP_TYPE</code> | ✓ **0.92** | ✓ **0.70** |
| <code>COMP_WORDBREAKS</code> | V | V |
| <code>COMP_WORDS</code> | ✓ 0.65 | ✓ **0.72** |
| <code>COPROC</code> | ✓ 0.61 / 0.64 | ✓ 0.64 / **0.73** |
| <code>DIRSTACK</code> | ✓ 0.62 / **1.00↑** | ✓ **0.84** / **0.86** |
| <code>EMACS</code> | ✓ **0.72** | ✓ **0.86** |
| <code>ENV</code> | Δ | Δ |
| <code>EPOCHREALTIME</code> | ✓ 0.58 / 0.54 | ✓ **0.72** / **1.14↑** |
| <code>EPOCHSECONDS</code> | ✓ 0.61 / **0.86** | ✓ **0.70** / **1.05↑** |
| <code>EUID</code> | ✓ **0.86** / **0.81** | ✓ **0.78** / **0.90** |
| <code>EXECIGNORE</code> | Δ | Δ |
| <code>FCEDIT</code> | ✓ **0.84** / **0.68** | ✓ **0.85** / **0.98** |
| <code>FIGNORE</code> | Δ | Δ |
| <code>FUNCNAME</code> | ✓ 0.61 / **1.06↑** | ✓ 0.63 / **0.91** |
| <code>FUNCNEST</code> | Δ | Δ |
| <code>GLOBIGNORE</code> | ✓ 0.65 / **0.75** | ✓ **0.84** / **0.85** |
| <code>GLOBSORT</code> | — | Δ |
| <code>GROUPS</code> | Δ | Δ |
| <code>HISTCMD</code> | V | V |
| <code>HISTCONTROL</code> | ✓ **0.87** / **1.08↑** | ✓ **0.76** / **1.13↑** |
| <code>HISTFILE</code> | ✓ **0.74** / **0.93** | ✓ **0.82** / **1.05↑** |
| <code>HISTFILESIZE</code> | ✓ 0.54 / 0.61 | ✓ **0.71** / **0.81** |
| <code>HISTIGNORE</code> | ✓ **0.73** / **1.10↑** | ✓ **0.81** / **1.02↑** |
| <code>HISTSIZE</code> | ✓ **0.75** / **0.90** | ✓ **0.72** / **0.82** |
| <code>HISTTIMEFORMAT</code> | ✓ 0.53 / **0.69** | ✓ **0.72** / **0.79** |
| <code>HOME</code> | ✓ **0.75** / **0.70** | ✓ **0.91** / **0.92** |
| <code>HOSTFILE</code> | ✓ 0.64 / **1.43↑** | ✓ 0.59 / **0.83** |
| <code>HOSTNAME</code> | V | V |
| <code>HOSTTYPE</code> | I | I |
| <code>IFS</code> | ✓ **0.80** / **0.99** | ✓ **0.71** / **1.00** |
| <code>IGNOREEOF</code> | ✓ **0.93** | ✓ **0.72** |
| <code>INPUTRC</code> | I | I |
| <code>INSIDE_EMACS</code> | ✓ 0.67 | ✓ **0.74** |
| <code>LANG</code> | ✓ **0.74** / 0.52 | ✓ 0.47 / 0.58 |
| <code>LC_ALL</code> | ✓ 0.35 / 0.36 | ✓ 0.58 / 0.38 |
| <code>LC_COLLATE</code> | Δ | Δ |
| <code>LC_CTYPE</code> | ✓ 0.48 / 0.52 | ✓ 0.61 / 0.54 |
| <code>LC_MESSAGES</code> | Δ | Δ |
| <code>LC_NUMERIC</code> | Δ | Δ |
| <code>LC_TIME</code> | Δ | Δ |
| <code>LINENO</code> | ✓ 0.59 / **1.04↑** | ✓ 0.60 / **0.67** |
| <code>LINES</code> | ✓ **0.87** | ✓ **0.92** |
| <code>MACHTYPE</code> | I | I |
| <code>MAIL</code> | Δ | Δ |
| <code>MAILCHECK</code> | Δ | Δ |
| <code>MAILPATH</code> | Δ | Δ |
| <code>MAPFILE</code> | ✓ 0.55 / **0.73** | ✓ 0.55 / **0.81** |
| <code>OLDPWD</code> | ✓ **0.88** / **0.81** | ✓ **0.86** / **0.94** |
| <code>OPTARG</code> | ✓ **0.71** / 0.54 | ✓ **0.82** / **0.73** |
| <code>OPTERR</code> | ✓ 0.63 / **0.78** | ✓ **0.69** / **0.99** |
| <code>OPTIND</code> | ✓ **0.71** / 0.54 | ✓ **0.82** / **0.73** |
| <code>OSTYPE</code> | I | I |
| <code>PATH</code> | ✓ **0.81** / **0.94** | ✓ **1.01↑** / **0.97** |
| <code>PIPESTATUS</code> | ✓ 0.47 / **0.67** | ✓ 0.64 / **0.71** |
| <code>POSIXLY_CORRECT</code> | Δ | Δ |
| <code>PPID</code> | ✓ 0.67 / **0.96** | ✓ 0.66 / **0.82** |
| <code>PROMPT_COMMAND</code> | ✓ **0.89** | ✓ **0.91** |
| <code>PROMPT_DIRTRIM</code> | ✓ **0.88** | ✓ **0.98** |
| <code>PS0</code> | ✓ **1.06↑** | ✓ **0.96** |
| <code>PS1</code> | ✓ **0.88** | ✓ **0.86** |
| <code>PS2</code> | ✓ **0.83** | ✓ **0.91** |
| <code>PS3</code> | ✓ 0.60 / **0.80** | ✓ **0.77** / **1.14↑** |
| <code>PS4</code> | ✓ 0.60 / **1.04↑** | ✓ **0.71** / **0.86** |
| <code>PWD</code> | ✓ **0.72** / **1.02↑** | ✓ **0.78** / **0.95** |
| <code>RANDOM</code> | ✓ 0.66 / **0.97** | ✓ 0.63 / **0.88** |
| <code>READLINE_ARGUMENT</code> | ✓ **0.83** | ✓ **0.79** |
| <code>READLINE_LINE</code> | ✓ **0.67** | ✓ **0.70** |
| <code>READLINE_MARK</code> | ✓ **0.81** | ✓ **0.77** |
| <code>READLINE_POINT</code> | ✓ **0.81** | ✓ **0.92** |
| <code>REPLY</code> | ✓ 0.61 / **0.82** | ✓ 0.61 / **0.89** |
| <code>SECONDS</code> | ✓ 0.61 / **0.97** | ✓ **0.72** / **0.83** |
| <code>SHELL</code> | ✓ 0.49 / 0.62 | ✓ 0.59 / 0.61 |
| <code>SHELLOPTS</code> | ✓ 0.63 / **0.93** | ✓ 0.65 / **0.85** |
| <code>SHLVL</code> | ✓ 0.56 / 0.58 | ✓ 0.62 / 0.63 |
| <code>SRANDOM</code> | Δ | Δ |
| <code>TIMEFORMAT</code> | ✓ **0.71** / **1.12↑** | ✓ **0.88** / **1.05↑** |
| <code>TMOUT</code> | ✓ **1.00↑** | ✓ **1.00** |
| <code>TMPDIR</code> | I | I |
| <code>UID</code> | ✓ **0.71** / **0.84** | ✓ **0.86** / **0.86** |
| <code>_</code> | V | V |
| <code>histchars</code> | Δ | Δ |

## set -o

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>allexport</code> | ✓ 0.57 / **0.94** | ✓ **0.70** / **0.89** |
| <code>braceexpand</code> | ✓ 0.64 / **1.05↑** | ✓ **0.75** / **0.94** |
| <code>emacs</code> | I 0.64 / **0.97** | I **0.79** / **0.91** |
| <code>errexit</code> | ✓ 0.57 / **0.67** | ✓ **0.67** / **0.84** |
| <code>errtrace</code> | ✓ **0.69** / **0.81** | ✓ **0.75** / **0.87** |
| <code>functrace</code> | ✓ 0.62 / **0.80** | ✓ 0.61 / **1.05↑** |
| <code>hashall</code> | I 0.55 / **0.83** | I 0.60 / **0.88** |
| <code>histexpand</code> | I **0.70** / **1.07↑** | I 0.64 / **0.90** |
| <code>history</code> | I 0.61 / 0.61 | I 0.64 / **0.86** |
| <code>ignoreeof</code> | Δ | Δ |
| <code>interactive-comments</code> | Δ | Δ |
| <code>keyword</code> | Δ | Δ |
| <code>monitor</code> | ✓ **0.87** / **0.99** | ✓ **0.93** / **0.95** |
| <code>noclobber</code> | ✓ 0.58 / **0.68** | ✓ **0.92** / **0.91** |
| <code>noexec</code> | Δ | Δ |
| <code>noglob</code> | ✓ **1.09↑** / **0.92** | ✓ **0.80** / **0.94** |
| <code>nolog</code> | Δ | Δ |
| <code>notify</code> | Δ | Δ |
| <code>nounset</code> | E | E |
| <code>onecmd</code> | Δ | Δ |
| <code>physical</code> | Δ | Δ |
| <code>pipefail</code> | ✓ 0.62 / 0.63 | ✓ 0.62 / **0.75** |
| <code>posix</code> | Δ | Δ |
| <code>privileged</code> | Δ | Δ |
| <code>verbose</code> | E | E |
| <code>vi</code> | I 0.61 / **1.05↑** | I 0.56 / **0.75** |
| <code>xtrace</code> | ✓ 0.62 / **1.05↑** | ✓ 0.60 / **0.80** |

## shopt

| Feature / 項目 | Debian | Ubuntu |
| --- | --- | --- |
| <code>array_expand_once</code> | — | M |
| <code>assoc_expand_once</code> | M | M |
| <code>autocd</code> | M | M |
| <code>bash_source_fullpath</code> | — | M |
| <code>cdable_vars</code> | M | M |
| <code>cdspell</code> | M | M |
| <code>checkhash</code> | ✓ **0.74** / **0.85** | ✓ **0.82** / **0.95** |
| <code>checkjobs</code> | M | M |
| <code>checkwinsize</code> | I 0.60 / **0.90** | I 0.52 / **0.94** |
| <code>cmdhist</code> | M | M |
| <code>compat31</code> | M | M |
| <code>compat32</code> | M | M |
| <code>compat40</code> | M | M |
| <code>compat41</code> | M | M |
| <code>compat42</code> | M | M |
| <code>compat43</code> | M | M |
| <code>compat44</code> | M | M |
| <code>complete_fullquote</code> | M | M |
| <code>direxpand</code> | M | M |
| <code>dirspell</code> | M | M |
| <code>dotglob</code> | ✓ **0.77** / **0.70** | ✓ **0.90** / **1.03↑** |
| <code>execfail</code> | M | M |
| <code>expand_aliases</code> | ✓ 0.58 / **0.87** | ✓ 0.57 / **1.35↑** |
| <code>extdebug</code> | ✓ **0.67** / **0.90** | Δ |
| <code>extglob</code> | ✓ 0.54 / **0.94** | ✓ 0.59 / **1.22↑** |
| <code>extquote</code> | M | M |
| <code>failglob</code> | ✓ **0.81** / **0.98** | ✓ 0.59 / **0.77** |
| <code>force_fignore</code> | M | M |
| <code>globasciiranges</code> | M | M |
| <code>globskipdots</code> | ✓ 0.59 / **0.81** | ✓ **0.78** / **1.01↑** |
| <code>globstar</code> | ✓ **0.74** / **0.67** | ✓ **0.87** / **0.73** |
| <code>gnu_errfmt</code> | M | M |
| <code>histappend</code> | I **0.69** / **1.07↑** | I 0.58 / **0.88** |
| <code>histreedit</code> | M | M |
| <code>histverify</code> | M | M |
| <code>hostcomplete</code> | I 0.53 / **1.11↑** | I **0.73** / **1.01↑** |
| <code>huponexit</code> | I 0.62 / **0.98** | I 0.63 / **0.88** |
| <code>inherit_errexit</code> | ✓ 0.50 / 0.64 | ✓ 0.57 / **0.71** |
| <code>interactive_comments</code> | M | M |
| <code>lastpipe</code> | M | M |
| <code>lithist</code> | M | M |
| <code>localvar_inherit</code> | ✓ 0.56 / **1.02↑** | ✓ **0.91** / 0.57 |
| <code>localvar_unset</code> | I **0.95** / **0.80** | I 0.60 / **0.82** |
| <code>login_shell</code> | I **0.98** / 0.65 | I 0.59 / **0.81** |
| <code>mailwarn</code> | M | M |
| <code>no_empty_cmd_completion</code> | M | M |
| <code>nocaseglob</code> | M | M |
| <code>nocasematch</code> | ✓ 0.67 / **0.76** | ✓ **0.81** / **0.96** |
| <code>noexpand_translation</code> | M | M |
| <code>nullglob</code> | ✓ **0.75** / **0.74** | ✓ **0.74** / **0.93** |
| <code>patsub_replacement</code> | ✓ 0.65 / **1.01↑** | ✓ **0.69** / **0.86** |
| <code>progcomp</code> | I 0.59 / 0.65 | I 0.57 / **0.78** |
| <code>progcomp_alias</code> | M | M |
| <code>promptvars</code> | I 0.61 / **1.13↑** | I 0.63 / **0.93** |
| <code>restricted_shell</code> | M | M |
| <code>shift_verbose</code> | M | M |
| <code>sourcepath</code> | ✓ **1.10↑** / **1.06↑** | ✓ **0.80** / **0.97** |
| <code>varredir_close</code> | E | E |
| <code>xpg_echo</code> | M | M |
