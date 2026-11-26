"""Instrument a disposable Tonari checkout for an untimed per-case path replay."""
import json
import re
import sys
from pathlib import Path

# Each annotation describes the work performed at a measured call site and the
# allocation, parse, lookup or system call inspected in the optimization review.
SITES = """
cli parse | Decode argv once and borrow its basename before -c replaces $0.
startup initialize | Select the startup file for this invocation before opening a file.
startup source_file | Search PATH for dot operands and open the selected script once.
input stdin_script | Share the seekable input position with commands that read stdin.
input file_script | Reuse file metadata and the buffered script reader.
input text | Parse complete commands in order so earlier aliases take effect.
input/eval parse | Reuse the parsed eval program only while parser options allow it.
parser/lexer lex | Reuse token buffers and scan plain ASCII words in chunks.
parser/aliases expand_alias | Check the live alias table before copying replacement tokens.
alias builtin | Share alias definitions until mutation and quote only printed definitions.
parser/here_doc redirect | Save the delimiter and defer reading the body until newline.
eval run_if | Evaluate each condition once and skip unselected branches.
eval repeat | Reuse the loop body and propagate break, continue and exit.
eval call_function | Share the function definition and restore positional arguments on return.
session/functions define_function | Register an Rc definition without reparsing its body.
arithmetic with_variables | Parse the evaluated arithmetic text with the active variable lookup.
expansion fields_with_glob | Split only eligible fields and expand active patterns.
expansion scalar | Expand a single scalar without pathname enumeration.
pattern pathnames | Enumerate matching entries and sort the resulting paths.
pattern trim | Find the matching parameter boundary before copying the retained text.
word/tilde tilde | Use HOME for a bare tilde and consult the OS account database only for named users.
session assign_value | Reuse existing scalar capacity and preserve special-variable side effects.
session persist_prefix | Move prepared exported prefixes directly into the session.
session bind | Save temporary bindings for restoration after the builtin.
execute/prepare prepare | Expand command words and prefixes in their specified order.
execute/prepare prepare_redirects | Expand redirect operands once, preserving left-to-right order.
execute/builtin run_builtin | Save overwritten descriptors only for the duration of a returning builtin.
execute/redirect apply_protected | Apply opens, duplicates and closes in order and propagate OS failure.
execute/here_input open | Use the bounded pipe path when capacity permits and a file otherwise.
execute/hash resolve | Reuse a cached executable only while current search rules allow it.
execute/spawn spawn_resolved | Omit spawn attributes for ordinary foreground external commands.
execute/external exec | Replace the process image for eligible tail commands without an extra fork.
execute start_raw | Evaluate pipeline-stage expansions in the appropriate process.
execute start | Select spawn, fork or in-process builtin dispatch for this pipeline stage.
subshell capture_substitution | Try direct capture before creating a pipe and child.
jobs finish | Retain per-stage exit status and restore the controlling process group.
jobs/builtins kill | Resolve signal and PID or job specification before sending.
jobs/builtins wait_for_ids | Consume saved child status or wait for the requested job.
jobs/builtins wait_next | Poll and block atomically to avoid losing an exit notification.
traps set | Save the inherited disposition when first installing a signal trap.
traps after_fork | Reset caught traps while retaining ignores in shell children.
traps dispatch | Drain pending signal bits at shell command boundaries.
traps finish | Apply exit traps and preserve the signal or status seen by the parent.
session/printf run | Format directly from shell arguments and respect assignment escapes.
builtins echo | Write the prepared bytes and account for descriptor failure.
builtins test | Evaluate test operands, performing metadata queries only when needed.
builtins/umask run | Read or set the process mask and format the requested representation.
session read | Avoid consuming bytes belonging to the next stdin reader.
session get | Read special and ordinary variables through the current binding.
session shift | Advance the existing argument iterator instead of copying the tail.
session/options set | Change only the selected options or positional arguments.
session/getopts getopts | Reuse the option cursor while checking variable attributes.
session/directories directory_builtin | Resolve the requested directory while preserving PWD and OLDPWD.
execute/lookup describe | Check reserved words, builtins, functions and path lookup in shell order.
session/options shopt | Query and set the option used by the interpreter.
interactive interactive | Redraw at command boundaries and dispatch traps before blocking for input.
interactive read_terminal_line | Leave later lines available to children sharing redirected stdin.
""".strip()

PROBE = """
use std::{io::Write, sync::atomic::{AtomicU64, Ordering::Relaxed}};
static SEEN: AtomicU64 = AtomicU64::new(0);
pub(crate) fn hit(index: u8) {
    let bit = 1u64 << index;
    if SEEN.load(Relaxed) & bit == 0 && SEEN.fetch_or(bit, Relaxed) & bit == 0 {
        if let Some(path) = std::env::var_os("TONARI_CASE_TRACE") {
            if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = out.write_all(&[index]);
            }
        }
    }
}
"""

EDITOR_SITES = """
editor binding | Check custom prefixes and the constant single-byte action table before building a sequence.
editor insert_burst | Collect adjacent committed UTF-8 text without reading beyond the first command delimiter.
editor motion_burst | Apply completed unmodified motions together up to a bounded repaint interval.
editor/recall navigate | Use the history position and unchanged search prefix for adjacent navigation.
history find_command | Reuse history search state and yield to pending terminal input.
complete at | Identify the token, consult programmable completion and enumerate eligible candidates.
session variable_names | Start the ordered variable lookup at the requested prefix and stop after the last matching name.
""".strip()


def instrument(root, output, editor=False):
    sites = []
    for i, entry in enumerate((SITES + ("\n" + EDITOR_SITES if editor else "")).splitlines()):
        location, review = entry.split(" | ")
        module, function = location.split()
        path = root / "src" / (module + ".rs")
        source = path.read_text()
        indent = r"[ \t]*"
        matches = list(re.finditer("^" + indent + r"(?:pub(?:\([^)]*\))? )?fn " + function + r"\s*(?:<|\()", source, re.M))
        if len(matches) != 1 or i >= 64:
            raise ValueError(f"ambiguous probe {location}")
        brace = source.index("{", matches[0].end())
        path.write_text(source[:brace+1] + f"\ncrate::case_trace::hit({i});" + source[brace+1:])
        sites.append({"module": module, "function": function, "review": review})
    (root / "src/case_trace.rs").write_text(PROBE)
    main = root / "src/main.rs"
    main.write_text(main.read_text() + "\nmod case_trace;\n")
    output.write_text(json.dumps(sites, indent=2) + "\n")

if __name__ == "__main__":
    instrument(Path(sys.argv[1]), Path(sys.argv[2]), "--editor" in sys.argv[3:])
