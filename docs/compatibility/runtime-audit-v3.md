# Third runtime audit / 第3回監査

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
| 3.1. Builtin redirection | `pwd` writes a short path and newline together. A spare descriptor is reserved before truncating a file. Even when the opened descriptor is below the soft limit, all higher slots can be occupied. Opening first can destroy the old contents before reporting a redirection failure. |
| 3.2–3.3. Spawn, environment and here-strings | `/dev/null` already avoids a private duplicate when its fd is above stderr. Opening an arbitrary file in the parent as a nonblocking probe can wake FIFO peers or open a device twice. Keep general opens in the child and keep signal reset before a blocking open. Memory-backed reads and callbacks still need an observable input fd. Cached environments compare live changes including temporary assignments and restored scopes. |
| 4. Script execution | Numeric printf, live bounds and explicit steps in arithmetic loops, function-local variable lookup and silent shell option changes do less work. Namerefs and arithmetic expressions use the general evaluator when needed. Lexer whitespace skipping and caching of inherited FUNCNEST were already present. |
| 6. Size | Six builtin entry points use an explicit inlining boundary. Cold listing code shares writers and compatible sorts share a comparison. Tiny dynamic-variable and implicit-signal collections use vectors. Trap listing order matters and process-substitution statuses can reach 4,096 entries. Numeric and time locale data share names, with compact per-field IDs. Decoding all 16,081 field strings reproduced the old locale data. A sorted portable-symbol table had the same M1 file size and took longer in the measured lookup. |
| 7–8. Maintenance | Option parsing is shared where grammars match. `dirs -1` is a valid operand, and bind keymaps and completion actions have distinct option rules. Streaming input also owns command history across lookahead. The broad proposals to restructure Session, change visibility everywhere, replace all error types and split the editor and parsers remain separate maintenance work. Existing export paths handle both ordinary exported values and array declarations. |

端末の先読み、通常ファイルや FIFO の事前 open、環境キャッシュの無条件再利用には観測可能な副作用がある。監査の予測値は採用の根拠にせず、現行コード、再現試験、配布バイナリを照合した。とくに通常ファイルへの `pwd` と短いキー入力では、目標比に届かなかった測定も記録している。[測定条件、採用版と試作の比較](../benchmarks.md#runtime-audit-v3)
