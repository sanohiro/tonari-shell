mod eval;
mod lookahead;
pub(crate) use eval::{EvalCache, builtin as eval_builtin, trap_text};
pub(crate) use lookahead::buffered_compound;
use lookahead::{file_group, whole_group};

use crate::execute::fd::PrivateFile;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::fs::FileExt;

use crate::{Flow, Session, execute_program, parser};

// Append at most one read to an already reserved buffer.
pub(crate) fn read_spare(file: &File, output: &mut Vec<u8>, limit: usize) -> io::Result<usize> {
    let tail = output.spare_capacity_mut();
    let count = tail.len().min(limit);
    debug_assert_ne!(count, 0);
    loop {
        // The kernel initializes only the returned prefix of this writable allocation.
        let n = unsafe { libc::read(file.as_raw_fd(), tail.as_mut_ptr().cast(), count) };
        if n >= 0 {
            unsafe { output.set_len(output.len() + n as usize) };
            return Ok(n as usize);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

// External commands inherit fd 0. Any unread text buffered by the shell must
// still be available to a child which reads stdin (for example, "cat\n...").
pub fn stdin_script(session: &mut Session, check: bool, status: u8) -> Flow {
    match stdin_reader() {
        Ok((reader, shared_offset)) => run(
            session,
            reader,
            shared_offset,
            true,
            None,
            (status, 1),
            check,
        ),
        Err(e) => error(e, 1),
    }
}

fn stdin_reader() -> io::Result<(BufReader<PrivateFile>, bool)> {
    let mut file = PrivateFile::duplicate(io::stdin().as_fd())?;
    let seekable = file.stream_position().is_ok();
    let reader = if seekable {
        BufReader::new(file)
    } else {
        BufReader::with_capacity(1, file)
    };
    Ok((reader, seekable))
}

pub(crate) fn executable_header(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some(
            b"\x7fELF"
                | b"\xcf\xfa\xed\xfe"
                | b"\xfe\xed\xfa\xcf"
                | b"\xce\xfa\xed\xfe"
                | b"\xfe\xed\xfa\xce"
                | b"\xca\xfe\xba\xbe"
                | b"\xca\xfe\xba\xbf"
                | b"\xbe\xba\xfe\xca"
                | b"\xbf\xba\xfe\xca"
        )
    )
}

pub fn file_script(
    session: &mut Session,
    file: File,
    metadata: Option<std::fs::Metadata>,
    status: u8,
    check: bool,
) -> Flow {
    let metadata = metadata.or_else(|| file.metadata().ok());
    if metadata.as_ref().is_some_and(|m| m.is_dir()) {
        return error(
            "cannot run a directory as a script",
            if session.source_depth == 0 { 126 } else { 1 },
        );
    }
    let metadata = metadata.filter(|_| !session.verbose);
    if let Some(meta) = &metadata
        && meta.is_file()
        && (session.source_depth > 0 && meta.len() < 128 * 1024
            || check && (8192..=1024 * 1024).contains(&meta.len()))
    {
        // This regular script starts at offset zero. Read one byte beyond
        // the advertised length so a changed file still takes the stream path.
        let len = meta.len() as usize;
        let mut small = [0; 4096];
        let mut large;
        let bytes = if len < small.len() {
            &mut small[..=len]
        } else {
            large = vec![0; len + 1];
            &mut large[..]
        };
        if let Ok(n) = file.read_at(bytes, 0)
            && n == len
            && len != 0
        {
            if executable_header(&bytes[..n]) {
                return error("cannot execute binary file", 126);
            }
            if let Ok(source) = std::str::from_utf8(&bytes[..n]) {
                drop(file);
                return text(session, source, status, false, check, None);
            }
        }
    }
    if let Some(meta) = &metadata
        && (session.source_depth != 0 || !session.history_enabled)
        && let Some((program, next_line)) = file_group(session, &file, meta)
    {
        let mut file = match PrivateFile::new(file) {
            Ok(file) => file,
            Err(e) => return error(e, 1),
        };
        let flow = if check {
            Flow::Continue(0)
        } else {
            match execute_program(session, program, status, false) {
                Flow::Unwind(code) if session.source_depth == 0 => Flow::Continue(code),
                Flow::Discard(code) => Flow::Continue(code),
                flow => flow,
            }
        };
        if let Flow::Continue(code) = flow
            && !session.onecmd
            && file
                .metadata()
                .is_ok_and(|current| current.len() > meta.len())
        {
            if let Err(e) = file.seek(io::SeekFrom::Start(meta.len())) {
                return error(e, 1);
            }
            return run(
                session,
                BufReader::new(file),
                false,
                false,
                None,
                (code, next_line),
                check,
            );
        }
        return flow;
    }
    // Keep large compound commands in view across the usual 8 KiB boundary.
    // Verbose mode must still report each line as it is read.
    let file_len = metadata.filter(|m| m.is_file()).map(|m| m.len());
    let capacity = file_len.unwrap_or(8192).clamp(8192, 128 * 1024) as usize;
    match PrivateFile::new(file) {
        Ok(file) => {
            let mut reader = BufReader::with_capacity(capacity, file);
            // Reuse the parser's first read. Binary images need no line scan.
            if reader.fill_buf().is_ok_and(executable_header) {
                return error("cannot execute binary file", 126);
            }
            run(session, reader, false, false, file_len, (status, 1), check)
        }
        Err(e) => error(e, 1),
    }
}

fn error(message: impl std::fmt::Display, status: u8) -> Flow {
    diagnostic!("{message}");
    Flow::Continue(status)
}

fn syntax_error(session: &Session, problem: parser::Error, first_line: usize) -> Flow {
    let flow = if first_line == 1
        && !session.command_string
        && !session.stdin_script
        && session.source_depth == 0
        && matches!(problem, parser::Error::NulByte)
    {
        Flow::Exit(126)
    } else if let parser::Error::ArrayToken(status) = problem
        && !session.errexit
    {
        if session.posix && !session.interactive {
            Flow::Exit(status)
        } else {
            Flow::Continue(status)
        }
    } else if session.posix && !session.interactive {
        Flow::Exit(2)
    } else {
        Flow::Continue(2)
    };
    if let Some(prefix) = session.diagnostic_prefix(|| first_line) {
        stderr_line!("{prefix}{}", problem.message());
    } else if session.source_depth > 0
        && let Some(path) = session.source_names.last()
    {
        diagnostic!(
            "{path}: command starting at line {first_line}: {}",
            problem.message()
        );
    } else if !session.command_string && !session.stdin_script && !session.interactive {
        stderr_line!("{}: line {first_line}: {}", session.arg0, problem.message());
    } else {
        diagnostic!("{}", problem.message());
    }
    flow
}

fn blank_or_comment(line: &str, session: &Session) -> bool {
    let first = line.trim_start_matches([' ', '\t']);
    first.is_empty()
        || first == "\n"
        || first.starts_with('#')
            && session.read_comments()
            && !first.trim_end_matches('\n').contains(['\n', '\0'])
}

fn history_input<'a>(
    session: &mut Session,
    text: &'a str,
    quote: Option<char>,
) -> Option<std::borrow::Cow<'a, str>> {
    let event = match crate::history::expand::expand_line(session, text, quote) {
        Ok(event) => event,
        Err(error) => {
            diagnostic!("{error}");
            return None;
        }
    };
    if matches!(event.text, std::borrow::Cow::Owned(_)) {
        stderr_text!("{}", event.text);
        if !event.text.ends_with('\n') {
            stderr_line!();
        }
    }
    if event.print {
        session.begin_history_line(&event.text);
        session.finish_history_line();
        return None;
    }
    Some(event.text)
}

// Only use a stored scalar as the source. Dynamic parameters, namerefs,
// arrays, unset variables and arithmetic run through ordinary expansion.
fn assign_buffered_scalar(session: &mut Session, command: &str, scratch: &mut String) -> bool {
    if let Some((name, value)) = parser::plain_assignment(command) {
        return session.assign_existing_literal(name, value);
    }
    let Some((target, source)) = command.split_once("=$") else {
        return false;
    };
    if !crate::word::is_identifier(target) || !crate::word::is_identifier(source) {
        return false;
    }
    let Some(value) = session.stored_scalar(source) else {
        return false;
    };
    value.clone_into(scratch);
    session.assign_existing_literal(target, scratch)
}

// After the parser identifies an open delimiter, ordinary physical lines
// cannot finish it. Retry on syntax, NUL, heredocs and history expansion.
pub(crate) fn still_open(
    reason: Option<parser::Incomplete>,
    text: &str,
    session: &Session,
) -> bool {
    use parser::Incomplete::*;
    if !matches!(reason, Some(SingleQuote | DoubleQuote)) && session.reads_aliases() {
        return false;
    }
    let line = text.as_bytes();
    let plain = |b: &u8| !b.is_ascii() || b.is_ascii_alphanumeric() || b" _-+:./=,\t\n".contains(b);
    match reason {
        Some(CompoundEnd(close)) => {
            // A complete, independent command with no outer terminator can only
            // extend the pending body. Never defer quotes, here-docs or syntax errors.
            (!text.contains(close)
                || text
                    .trim_ascii_start()
                    .split_ascii_whitespace()
                    .next()
                    .is_some_and(|keyword| {
                        matches!(
                            (keyword, close),
                            ("if", "fi")
                                | ("case", "esac")
                                | ("while" | "until" | "select" | "for", "done")
                        )
                    }))
                && !text.contains("<<")
                && (parser::validate_tail(text, session, 1)
                    || close == "esac"
                        && text.trim_end().ends_with(";;")
                        && parser::validate_tail(&format!("case x in\n{text}\nesac"), session, 1))
        }
        Some(SingleQuote) => memchr::memchr2(b'\'', 0, line).is_none(),
        Some(DoubleQuote) => !line.iter().any(|b| b"\"\0$`\\".contains(b)),
        Some(Message("line continuation")) => line
            .strip_suffix(b"\\\n")
            .is_some_and(|s| s.iter().all(plain)),
        Some(
            Message(
                "unfinished array assignment"
                | "unterminated arithmetic expression"
                | "unterminated command substitution"
                | "unterminated parameter expansion",
            )
            | Subshell,
        ) => {
            line.iter().all(plain)
                && !matches!(
                    line.trim_ascii().split(|b| b.is_ascii_whitespace()).next(),
                    Some(b"then" | b"else" | b"elif" | b"fi" | b"do" | b"done" | b"esac" | b"in")
                )
        }
        _ => false,
    }
}

// An unaliased function header cannot execute before its compound body. Both
// readers can skip the first parse when they have buffered text to probe.
fn parse_prefix<P>(
    session: &mut Session,
    text: &str,
    probe: bool,
    parse: impl FnOnce(&mut Session) -> Result<P, parser::Error>,
) -> Result<P, parser::Error> {
    if probe && parser::opens_function(text) && !session.reads_aliases() {
        Err(parser::Error::unfinished_function())
    } else {
        parse(session)
    }
}

enum Parsed<P> {
    Execute(P),
    ReadMore,
    Skip,
    Failed(Flow),
}

// Both file and string inputs execute one complete unit at a time. The initial
// status is visible to the first command, but empty input returns success.
struct ScriptState {
    status: u8,
    ran: bool,
    history_quote: Option<char>,
    incomplete: Option<parser::Incomplete>,
    here_doc: Option<parser::HereDocHint>,
}

impl ScriptState {
    fn new(status: u8) -> Self {
        Self {
            status,
            ran: false,
            history_quote: None,
            incomplete: None,
            here_doc: None,
        }
    }

    fn record(&mut self, status: u8) {
        self.status = status;
        self.ran = true;
    }

    fn finish(self) -> Flow {
        Flow::Continue(if self.ran { self.status } else { 0 })
    }

    fn parsed<P: AsRef<[parser::Statement]>>(
        &mut self,
        result: Result<P, parser::Error>,
        more: bool,
        session: &mut Session,
        check: bool,
        line: usize,
    ) -> Parsed<P> {
        self.history_quote = result.as_ref().err().and_then(parser::Error::pending_quote);
        self.incomplete = match &result {
            Err(parser::Error::Incomplete(reason)) => Some(*reason),
            _ => None,
        };
        match result {
            Ok(p) if !check && !p.as_ref().is_empty() => Parsed::Execute(p),
            Ok(_) => Parsed::Skip,
            Err(parser::Error::HereDoc(hint)) if more => {
                self.here_doc = Some(hint);
                Parsed::ReadMore
            }
            Err(parser::Error::Incomplete(_)) if more => Parsed::ReadMore,
            Err(e @ parser::Error::ArrayToken(status))
                if !check && !session.posix && !session.errexit && !session.noexec =>
            {
                syntax_error(session, e, line);
                self.record(status);
                session.pipe_status.single(status);
                Parsed::Skip
            }
            Err(e) => Parsed::Failed(syntax_error(session, e, line)),
        }
    }
}

fn run(
    session: &mut Session,
    mut reader: BufReader<PrivateFile>,
    mut shared_offset: bool,
    from_stdin: bool,
    file_len: Option<u64>,
    start: (u8, usize),
    check: bool,
) -> Flow {
    let mut assignment_value = String::new();
    let (status, mut base_line) = start;
    let mut input = Vec::new();
    let mut state = ScriptState::new(status);
    if base_line > 1 {
        state.record(status);
    }
    loop {
        let line_start = input.len();
        let n = match reader.read_until(b'\n', &mut input) {
            Ok(n) => n,
            Err(e) => return error(e, 1),
        };
        if session.verbose && n != 0 {
            use std::io::Write;
            let _ = io::stderr().write_all(&input[line_start..]);
        }
        if n == 0 && input.is_empty() {
            break;
        }
        if n != 0
            && state.here_doc.is_none()
            && session.histexpand
            && session.history_enabled
            && session.source_depth == 0
        {
            let Ok(line) = std::str::from_utf8(&input[line_start..]) else {
                return error("expected UTF-8 command text", 2);
            };
            match history_input(session, line, state.history_quote) {
                Some(std::borrow::Cow::Owned(replacement)) => {
                    input.truncate(line_start);
                    input.extend_from_slice(replacement.as_bytes());
                }
                None => {
                    input.truncate(line_start);
                    input.push(b'\n');
                }
                _ => {}
            }
        }
        if let Some(incomplete) = state.incomplete
            && n != 0
        {
            let Ok(line) = std::str::from_utf8(&input[line_start..]) else {
                return error("expected UTF-8 command text", 2);
            };
            if still_open(Some(incomplete), line, session)
                || matches!(incomplete, parser::Incomplete::Subshell)
                    && line.contains(')')
                    && !session.reads_aliases()
                    && parser::word_line(line, session)
            {
                continue;
            }
        }
        if let Some(hint) = &mut state.here_doc {
            let Ok(line) = std::str::from_utf8(&input[line_start..]) else {
                return error("expected UTF-8 command text", 2);
            };
            if n != 0 && !hint.may_finish(line) {
                continue;
            }
            state.here_doc = None;
        }
        let Ok(text) = std::str::from_utf8(&input) else {
            return error("expected UTF-8 command text", 2);
        };
        // At a new physical line these tokens cannot open a command. Keep
        // accumulated compound commands, history expansion and NUL diagnostics.
        if line_start == 0 && n != 0 && blank_or_comment(text, session) {
            if !check && session.source_depth == 0 && session.begin_history_line(text) {
                session.finish_history_line();
            }
            base_line += text.bytes().filter(|&b| b == b'\n').count();
            input.clear();
            continue;
        }
        // A write to an existing plain scalar cannot run shell code or consume stdin.
        // Preserve read-ahead across those lines. The next observable command still
        // synchronizes the shared offset before any builtin, trap or child can read.
        if shared_offset
            && line_start == 0
            && !check
            && !session.noexec
            && !session.onecmd
            && !session.xtrace
            && !session.history_enabled
            && !session.traps.observes_commands()
            && crate::traps::pending().is_none()
            && session.coprocesses.is_empty()
            && session.jobs.is_empty()
            && let Some(command) = text.strip_suffix('\n')
            && (assign_buffered_scalar(session, command, &mut assignment_value)
                || matches!(command.trim_ascii(), ":" | "true")
                    && !session.reads_aliases()
                    && !session.functions.contains_key(command.trim_ascii())
                    && !session.disabled_builtins.contains(command.trim_ascii()))
        {
            let command_name = command.trim_ascii();
            session.remember_argument(if matches!(command_name, ":" | "true") {
                command_name
            } else {
                ""
            });
            session.current_command = Some(parser::Location::label(command.into(), base_line));
            session.pipe_status.single(0);
            session.command_number += usize::from(session.source_depth == 0);
            session.substitutions.finish(session.substitutions.scope());
            state.record(0);
            base_line += 1;
            input.clear();
            continue;
        }
        let buffered = line_start == 0 && n != 0 && !shared_offset && !session.verbose;
        let mut result = parse_prefix(session, text, buffered, |s| {
            parser::session_source_at(text, n == 0, s, base_line)
        });
        // A large compound can start near the end of a read buffer. After
        // refilling, retry lookahead at geometrically spaced prefix sizes.
        let retry = line_start >= 1024
            && line_start.ilog2() != input.len().ilog2()
            && !shared_offset
            && !session.verbose
            && n != 0;
        if (buffered || retry)
            && (session.source_depth > 0 || !session.history_enabled)
            && matches!(result, Err(parser::Error::Incomplete(_)))
        {
            // Refill a short window only when the regular file still has data.
            // This avoids reparsing the entire old window one line at a time.
            if retry
                && reader.buffer().len() < reader.capacity() / 2
                && let Some(len) = file_len
                && let Ok(pos) = reader.stream_position()
                && pos.saturating_add(reader.buffer().len() as u64) < len
                && reader.seek(io::SeekFrom::Start(pos)).is_ok()
                && let Err(e) = reader.fill_buf()
            {
                return error(e, 1);
            }
            let parse = |size| {
                let rest = std::str::from_utf8(&reader.buffer()[..size])
                    .map_err(|_| parser::Error::Invalid("expected UTF-8 command text"))?;
                let mut unit = String::with_capacity(text.len() + size);
                unit.push_str(text);
                unit.push_str(rest);
                parser::session_source_at(&unit, false, session, base_line)
            };
            let batch = if !from_stdin
                && !session.onecmd
                && !session.history_enabled
                && session.readonly_functions.is_empty()
                && !session.posix
            {
                lookahead::function_run(text, reader.buffer(), session, parse)
            } else {
                None
            };
            let extra_commands = batch
                .as_ref()
                .map_or(0, |(program, _, _)| program.len() - 1);
            if let Ok((program, consumed, lines)) = batch.map(Ok).unwrap_or_else(|| {
                buffered_compound(text, reader.buffer(), false, session.reads_aliases(), parse)
            }) {
                if !check && session.source_depth == 0 {
                    session.command_number += extra_commands;
                }
                reader.consume(consumed);
                base_line += lines;
                result = Ok(program);
            }
        }
        match state.parsed(result, n != 0, session, check, base_line) {
            Parsed::ReadMore => continue,
            Parsed::Skip => {}
            Parsed::Failed(flow) => return flow,
            Parsed::Execute(program) => {
                if shared_offset && !reader.buffer().is_empty() {
                    // Unlike stream_position(), this discards the read-ahead buffer.
                    #[allow(clippy::seek_from_current)]
                    if let Err(e) = reader.seek(io::SeekFrom::Current(0)) {
                        return error(e, 1);
                    }
                }
                let remember = session.source_depth == 0 && session.begin_history_line(text);
                let flow = execute_program(session, program, state.status, false);
                // A permanent redirect can change fd 0 from a regular file to a
                // pipe or the reverse. Named scripts keep their private reader.
                if from_stdin && std::mem::take(&mut session.stdin_replaced) {
                    match stdin_reader() {
                        Ok(next) => (reader, shared_offset) = next,
                        Err(e) if e.raw_os_error() == Some(libc::EBADF) => return flow,
                        Err(e) => return error(e, 1),
                    }
                }
                session.command_number += usize::from(session.source_depth == 0);
                if remember {
                    session.finish_history_line();
                }
                if session.onecmd && session.source_depth == 0 && session.function_depth == 0 {
                    return flow;
                }
                match flow {
                    Flow::Unwind(code) if session.source_depth == 0 => state.record(code),
                    Flow::Discard(code) | Flow::Continue(code) => state.record(code),
                    other => return other,
                }
            }
        }
        base_line += text.bytes().filter(|&b| b == b'\n').count();
        input.clear();
        if n == 0 {
            break;
        }
    }
    state.finish()
}

// A syntax error on a later physical line must not undo an already executed
// complete command. Keep borrowed slices on the -c path.
pub fn text(
    session: &mut Session,
    source: &str,
    status: u8,
    tail: bool,
    check: bool,
    eval_line: Option<usize>,
) -> Flow {
    let first_line = eval_line.unwrap_or(1);
    if !session.verbose
        && let Some(program) = whole_group(session, source, first_line)
    {
        return if check {
            Flow::Continue(0)
        } else {
            match execute_program(session, program, status, tail) {
                Flow::Discard(code) => Flow::Continue(code),
                flow => flow,
            }
        };
    }
    let mut start = 0;
    let mut end = 0;
    let mut state = ScriptState::new(status);
    let mut base_line = first_line;
    let mut line_number = first_line - 1;
    // Successful lookahead consumes a command. Across misses, allow one
    // additional traversal of the source beyond the ordinary per-command window.
    const WINDOW: usize = 8192;
    let mut scan_budget = source.len();
    let mut check_tail = check && !session.verbose && source.len() <= 1024 * 1024;
    let mut edited: Option<String> = None;
    let mut lines = source.split_inclusive('\n');
    while let Some(line) = lines.next() {
        line_number += 1;
        if session.verbose {
            if line.ends_with('\n') {
                stderr_text!("{line}");
            } else {
                stderr_line!("{line}");
            }
        }
        let new_command = start == end;
        end += line.len();
        let mut last = end == source.len();
        let expand_history = tail
            && session.histexpand
            && session.history_enabled
            && (!session.command_string || session.command_history_expansion);
        if expand_history && state.here_doc.is_none() {
            match history_input(session, line, state.history_quote) {
                Some(std::borrow::Cow::Borrowed(_)) => {
                    if let Some(edited) = &mut edited {
                        edited.push_str(line);
                    }
                }
                changed => edited
                    .get_or_insert_with(|| source[start..end - line.len()].to_owned())
                    .push_str(changed.as_deref().unwrap_or("\n")),
            }
        } else if let Some(edited) = &mut edited {
            edited.push_str(line);
        }
        if !last && !expand_history && still_open(state.incomplete, line, session) {
            continue;
        }
        if !last
            && state
                .here_doc
                .as_mut()
                .is_some_and(|hint| !hint.may_finish(line))
        {
            continue;
        }
        state.here_doc = None;
        let mut text = edited.as_deref().unwrap_or(&source[start..end]);
        if new_command && blank_or_comment(text, session) {
            if !check
                && tail
                && (!session.interactive || session.command_history_expansion)
                && session.begin_history_line(text)
            {
                session.finish_history_line();
            }
            base_line = line_number + 1;
            start = end;
            edited = None;
            continue;
        }
        let probe = new_command && !last && !session.verbose && !expand_history;
        let parse = |session: &mut Session, unit: &str, eof| {
            if eval_line.is_some() {
                eval::parse(session, unit, eof, base_line)
            } else {
                parser::session_source_at(unit, eof, session, base_line).map(eval::Program::Fresh)
            }
        };
        let mut result = parse_prefix(session, text, probe, |s| parse(s, text, last));
        // Large incomplete compounds can otherwise reparse each growing prefix.
        // With -n there are no command effects between them. Keep short complete
        // lines on their streaming path, and fall back for precise diagnostics.
        if check_tail
            && !new_command
            && !expand_history
            && !last
            && text.len() >= 2048
            && matches!(result, Err(parser::Error::Incomplete(_)))
        {
            check_tail = false;
            if parser::validate_tail(&source[start..], session, base_line) {
                return Flow::Continue(0);
            }
        }
        if probe && matches!(result, Err(parser::Error::Incomplete(_))) {
            let limit = source
                .len()
                .min(end.saturating_add(WINDOW).saturating_add(scan_budget));
            // A sourced file has no per-input history or command-number
            // update. Preserve those boundaries for command strings and eval.
            let batch = if !tail && eval_line.is_none() && (check || session.source_depth > 0) {
                lookahead::function_run(text, &source.as_bytes()[end..], session, |size| {
                    parser::session_source_at(&source[start..end + size], false, session, base_line)
                })
                .map(|(p, n, lines)| (eval::Program::Fresh(p), n, lines))
            } else {
                None
            };
            match batch.map(Ok).unwrap_or_else(|| {
                buffered_compound(
                    text,
                    &source.as_bytes()[end..limit],
                    limit == source.len(),
                    session.reads_aliases(),
                    |size| parse(session, &source[start..end + size], false),
                )
            }) {
                Ok((program, consumed, extra_lines)) => {
                    end += consumed;
                    line_number += extra_lines;
                    lines = source[end..].split_inclusive('\n');
                    text = &source[start..end];
                    last = end == source.len();
                    result = Ok(program);
                }
                Err(inspected) => {
                    scan_budget = scan_budget.saturating_sub(inspected.saturating_sub(WINDOW))
                }
            }
        }
        match state.parsed(result, !last, session, check, base_line) {
            Parsed::ReadMore => continue,
            Parsed::Skip => {}
            Parsed::Failed(flow) => return flow,
            Parsed::Execute(program) => {
                let remember = tail
                    && (!session.interactive || session.command_history_expansion)
                    && session.begin_history_line(text);
                let flow = program.run(session, state.status, tail && last);
                session.command_number += usize::from(tail && !session.interactive);
                if remember {
                    session.finish_history_line();
                }
                match flow {
                    Flow::Discard(code) | Flow::Continue(code) => state.record(code),
                    other => return other,
                }
            }
        }
        base_line = line_number + 1;
        start = end;
        edited = None;
    }
    state.finish()
}

pub(crate) fn print_script(session: &Session, input: crate::cli::Input) -> u8 {
    use std::io::Write;
    let source = match input {
        crate::cli::Input::Command(text) => Ok(text),
        crate::cli::Input::File(path) => std::fs::read_to_string(path),
        crate::cli::Input::Stdin => std::io::read_to_string(std::io::stdin().lock()),
    };
    let source = match source {
        Ok(source) => source,
        Err(e) => {
            diagnostic!("{e}");
            return 1;
        }
    };
    match parser::pretty::format(&source, session) {
        Ok(text) => {
            let mut out = std::io::stdout().lock();
            let written = out
                .write_all(text.as_bytes())
                .and_then(|()| {
                    if text.is_empty() || text.ends_with('\n') {
                        Ok(())
                    } else {
                        out.write_all(b"\n")
                    }
                })
                .and_then(|()| out.flush());
            u8::from(written.is_err())
        }
        Err(e) => {
            diagnostic!("{}", e.message());
            2
        }
    }
}
