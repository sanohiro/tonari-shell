# Third runtime audit / 第3回監査

<a id="final-shortlist"></a>

## November 27 follow-up / 最終候補10項目の再評価

The shortlist describes `8e7b4f7`. We checked the ten proposals against the merged source through PR #33, exercised their public entry points and built release variants for the remaining scope and layout proposals.

| # | Candidate | Evaluation on the current source |
|---:|---|---|
| 1 | printf options and numeric formats | Attached and repeated `-v` already have Bash regression cases. The flags fit in a bit set. The C conversion format fits a checked 32-byte stack buffer. |
| 2 | Brace limits, umask and C strings | The word and byte limits, conditional `X`, and checked strings for user-supplied OS arguments and paths all landed in PR #21. The limits apply before materializing large ranges. |
| 3 | Startup environment and tty checks | Bash flags and imported functions share the initial environment scan. PS1, HOME, TERM, HISTFILE and editing descriptors may change in startup scripts. Read live values when opening the editor. |
| 4 | Editing and key sequences | Short lines already append when the old screen tail is valid. Multi-key bindings reuse a growable buffer, including custom sequences longer than 32 bytes. Coalescing still needs to stop where an accepted command can inherit queued input. |
| 5 | Completion | Registry lookup borrows command names through `Cow<str>`. Literal and native candidates bypass terminal-mode changes. A scripted callback can read the tty, change foreground group or redirect descriptors. PR #32 measures 10,000 literal words through a PTY. |
| 6 | File redirection | Short paths and logical pwd already use stack buffers. A free fd below the soft limit does not imply there is another free fd for the saved destination. Reserving the descriptor before a truncating open preserves the file on failure. A fast route for a builtin must also preserve its diagnostics and shell side effects. |
| 7 | Spawn and here-input | `private_fd` directly returns descriptors above 2. Opening an unknown path just to test its type can touch a FIFO or device before the child exists. The input descriptor remains observable by callbacks, traps and fd pressure. Packed argv and sparse environment comparison landed in PRs #29–30. An environment-generation counter would also have to track scope restoration, command-local assignments, exported functions and inherited masking. |
| 8 | Scripts and collections | Inherited variable names, FUNCNEST and simple arithmetic loops already have dedicated paths. In a sorted-Vec scope prototype the M1 function median fell 7.153 → 6.945 ms. Declaring 1,024 reverse-ordered locals rose 5.252 → 11.118 ms. The scope map also supports ordered listing, removal and restoration. Keep the map. PR #33 caches Bash's associative ordering key for the duration of a sort. A blanket deterministic hash introduces predictable user-key collisions and does not remove this ordering work. |
| 9 | Outlining and tiny sets | Forcing eleven more builtin boundaries left the M1 executable at 1,765,472 bytes and grew the Linux static PIE by 2,176 bytes. Linux functions improved slightly and file-redirection medians rose slightly in that run. Keep the previously measured boundaries. Trap output requires order and process substitution can retain thousands of child statuses. |
| 10 | Sorting, writers and locale data | Path sorting already shares comparison code and caches file metadata per key. A numeric filename and a file attribute have different keys. Multiple cold paths already use `dyn Write`. Packing shared substrings in the generated time tables preserved every string and reduced Linux by 5,504 bytes. The estimate of hundreds of kilobytes did not materialize in the measured layouts. |

`Bindings` の試作では、大きなスコープでも元の表示順・復元順を保って比較した。端末の所有権、FIFO の open、切り詰め前の FD 予約は、速度とともに利用者が観測する挙動を照合した。起動時に保存できる値と rc ファイルの実行後に変化する値は読み取る時点を区別する。 [Measurements / 測定](../benchmarks.md#shortlist-and-references) · [Prior audit / 前回の判定](#engineering-decisions--実装判断)

The audit described `8e7b4f7`. We checked its findings against `8349230` and compared the changes in `2ae4a52` with GNU Bash 5.3.20 on Apple M1 and the Ubuntu 24.04 Bash on native Linux x86-64. The scripts use their normal command, script or PTY entry points. [Regression cases](../../tests/shell.py) · [Release measurements](../benchmarks.md#runtime-audit-v3)

## Behavior / 挙動

| Input or boundary | Decision and observable result |
|---|---|
| `printf -vx '%s' one` and `printf -va -vb '%s' two` | The first assigns `x=one`. The last specified target in the second receives `two`. A separate operand and the `--` terminator also work. Flags use a bit set and the numeric formatting path builds its bounded C format on the stack. |
| `umask 077; umask a+X; umask` | Prints `0066`. Starting at 777 leaves 0777. In `umask u+x,g+X` the test for X uses permissions at the start of the command. Special mode bits have no effect on the creation mask. The interpreter remembers the mask it installed and a fork inherits it. |
| `f() { break; }; for i in 1 2; do f; echo "$i"; done` | Modern Bash and Tonari both run two iterations and diagnose the out-of-loop break. In POSIX mode this use returns success quietly. Under `BASH_COMPAT=4.3`, the function can break the caller's loop. The regression covers `continue` too. |
| `{1..5000000000}` or `{a,b}` repeated 21 times in one word | Tonari rejects more than 1,048,576 brace alternatives in one source word. Nested alternatives add, adjacent expansions multiply, and counting happens before evaluating the alternatives. The command does not run on error. A separate 64 MiB limit applies to the accumulated field text of one brace-expanded word. Large padded ranges are checked before allocation. Smaller compound cases retain Bash expansion order. |
| External command, `exec -a`, environment or redirection passed to the OS | Constructing a C string can return an error to the shell. A short redirection path uses a checked stack buffer. A filename containing an interior NUL cannot be silently shortened at the OS boundary. |

ブレース展開には Tonari 独自の上限がある。巨大な範囲や直積は非ゼロで終了し、そのコマンドを起動しない。条件付きの実行権 `X` と、関数内の `break`・`continue` は Bash の版・モードを明示して照合した。回帰試験は終了値、出力、ファイル作成の有無を確かめる。

## Engineering decisions / 実装判断

| Audit area | Changes, existing behavior and trade-offs |
|---|---|
| 1. Startup | One pass reads inherited Bash settings and functions. Keep owned settings across initialization. rc files may change TERM, HOME, HISTFILE or descriptors. The editor reuses its stdin tty result. PPID is captured at initialization to keep the recorded value stable after fork or reparenting. |
| 2.1–2.3. Editing | Short unstyled lines and first last-argument insertions can append when the rendered tail is valid. Replacing a previous argument redraws it. The editor reuses the key sequence buffer, writes plain prompt spans together and combines the final frame and newline. Existing motion and paste batching leaves subsequent typeahead available to the foreground command. |
| 2.4–2.5. Completion and history | Completion borrows registration keys on lookup. Small compgen requests already allocate a smaller buffer. Preview callbacks can execute arbitrary commands with inherited descriptors, so terminal ownership and restoration still matter. History drops expired strings promptly, batches compaction, and recycles token and span buffers after event expansion. |
| 3.1. Builtin redirection | `pwd` writes a short path and newline together. A spare descriptor is reserved before truncating a file. Even when the opened descriptor is below the soft limit, all higher slots can be occupied. With a descriptor limit of four and only stdin/stdout/stderr open, `pwd >file` returns 1 in both shells. Tonari preserves the old contents and Bash 5.3.20 truncates them. At five both succeed. |
| 3.2–3.3. Spawn, environment and here-strings | `/dev/null` already avoids a private duplicate when its fd is above stderr. Opening an arbitrary file in the parent as a nonblocking probe can wake FIFO peers or open a device twice. Keep general opens in the child and keep signal reset before a blocking open. Memory-backed reads and callbacks still need an observable input fd. Cached environments compare live changes including temporary assignments and restored scopes. |
| 4. Script execution | Numeric printf, live bounds and explicit steps in arithmetic loops, function-local variable lookup and silent shell option changes do less work. Namerefs and arithmetic expressions use the general evaluator when needed. Lexer whitespace skipping and caching of inherited FUNCNEST were already present. |
| 6. Size | Six builtin entry points use an explicit inlining boundary. Cold listing code shares writers and compatible sorts share a comparison. Tiny dynamic-variable and implicit-signal collections use vectors. Trap listing order matters and process-substitution statuses can reach 4,096 entries. Numeric and time locale data share names, with compact per-field IDs. Decoding all 16,081 field strings reproduced the old locale data. A sorted portable-symbol table had the same M1 file size and took longer in the measured lookup. |
| 7–8. Maintenance | Option parsing is shared where grammars match. `dirs -1` is a valid operand, and bind keymaps and completion actions have distinct option rules. Streaming input also owns command history across lookahead. The broad proposals to restructure Session, change visibility everywhere, replace all error types and split the editor and parsers remain separate maintenance work. Existing export paths handle both ordinary exported values and array declarations. |

端末の先読み、通常ファイルや FIFO の事前 open、環境キャッシュの無条件再利用には観測可能な副作用がある。監査の予測値は採用の根拠にせず、現行コード、再現試験、配布バイナリを照合した。とくに通常ファイルへの `pwd` と短いキー入力では、目標比に届かなかった測定も記録している。[測定条件、採用版と試作の比較](../benchmarks.md#runtime-audit-v3)
