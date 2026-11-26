mod eval;
pub(crate) use eval::{EvalCache, builtin as eval_builtin};

use crate::execute::fd::PrivateFile;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek};
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;

use crate::{Flow, Session, execute_program, parser};

// External commands inherit fd 0. Any unread text buffered by the shell must
// still be available to a child which reads stdin (for example, "cat\n...").
pub fn stdin_script(session: &mut Session, check: bool) -> Flow {
    match stdin_reader() {
        Ok((reader, shared_offset)) => run(session, reader, shared_offset, true, None, 0, check),
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

// Installers and version managers often wrap a whole script in one brace group.
// Bash reads such a unit before running any of it. Parse that unit once rather
// than reparsing its growing prefix after every newline.
fn starts_group(bytes: &[u8]) -> bool {
    bytes
        .split(|&c| c == b'\n')
        .map(<[u8]>::trim_ascii_start)
        .find(|line| !line.is_empty() && !line.starts_with(b"#"))
        .is_some_and(|line| line[0] == b'{' && line.get(1).is_none_or(u8::is_ascii_whitespace))
}

fn whole_group(session: &Session, text: &str, first_line: usize) -> Option<Vec<parser::Statement>> {
    if !starts_group(text.as_bytes()) {
        return None;
    }
    let program = parser::session_source_at(text, false, session, first_line).ok()?;
    if matches!(program.as_slice(), [s] if matches!(s.command, parser::Command::Group(_))) {
        Some(program)
    } else {
        None
    }
}

fn file_group(
    session: &Session,
    file: &File,
    meta: &std::fs::Metadata,
) -> Option<Vec<parser::Statement>> {
    if !meta.is_file() || !(32 * 1024..=8 * 1024 * 1024).contains(&meta.len()) {
        return None;
    }
    let mut prefix = [0; 4096];
    file.read_exact_at(&mut prefix, 0).ok()?;
    if !starts_group(&prefix) {
        return None;
    }
    let mut bytes = vec![0; meta.len() as usize];
    bytes[..prefix.len()].copy_from_slice(&prefix);
    file.read_exact_at(&mut bytes[prefix.len()..], prefix.len() as u64)
        .ok()?;
    // pread preserves the offset for the ordinary incremental fallback.
    whole_group(session, std::str::from_utf8(&bytes).ok()?, 1)
}

fn executable_header(bytes: &[u8]) -> bool {
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
        && let Some(program) = file_group(session, &file, meta)
    {
        drop(file);
        return if check {
            Flow::Continue(0)
        } else {
            match execute_program(session, program, status, false) {
                Flow::Discard(code) => Flow::Continue(code),
                flow => flow,
            }
        };
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
            run(session, reader, false, false, file_len, status, check)
        }
        Err(e) => error(e, 1),
    }
}

fn error(message: impl std::fmt::Display, status: u8) -> Flow {
    eprintln!("tonari: {message}");
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
    } else if matches!(problem, parser::Error::ArrayToken) && !session.errexit {
        if session.posix && !session.interactive {
            Flow::Exit(1)
        } else {
            Flow::Continue(1)
        }
    } else if session.posix && !session.interactive {
        Flow::Exit(2)
    } else {
        Flow::Continue(2)
    };
    if let Some(prefix) = session.diagnostic_prefix(|| first_line) {
        eprintln!("{prefix}{}", problem.message());
    } else if session.source_depth > 0
        && let Some(path) = session.source_names.last()
    {
        eprintln!(
            "tonari: {path}: command starting at line {first_line}: {}",
            problem.message()
        );
    } else {
        eprintln!("tonari: {}", problem.message());
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
            eprintln!("tonari: {error}");
            return None;
        }
    };
    if matches!(event.text, std::borrow::Cow::Owned(_)) {
        eprint!("{}", event.text);
        if !event.text.ends_with('\n') {
            eprintln!();
        }
    }
    if event.print {
        session.begin_history_line(&event.text);
        session.finish_history_line();
        return None;
    }
    Some(event.text)
}

// A sourced profile often has a long compound command. Try whole units already
// in the buffer, leaving following commands for the current alias/options state.
// On a miss report the distance inspected, so string input can bound repeated scans.
fn buffered_compound<P: AsRef<[parser::Statement]>>(
    first: &str,
    buffered: &[u8],
    at_eof: bool,
    aliases: bool,
    mut parse: impl FnMut(usize) -> Result<P, parser::Error>,
) -> Result<(P, usize, usize), usize> {
    let header = first.lines().next().ok_or(0usize)?.trim_ascii_start();
    let key = header
        .split(|c: char| c.is_ascii_whitespace() || c == ';')
        .next()
        .unwrap_or_default();
    let close: &[u8] = match key {
        "if" => b"fi",
        "case" => b"esac",
        "while" | "until" | "for" | "select" => b"done",
        "function" | "{" => b"}",
        _ if header.contains("()") => b"}",
        _ => return Err(0),
    };
    let mut start = 0;
    let mut nested = 0usize;
    let mut needs_open_brace = close == b"}" && !header.contains('{');
    let mut deferred = None;
    let mut probe = |size, lines| {
        // Only the final ordinary read may emit an unterminated here-doc warning.
        match parse(size) {
            Ok(program) if matches!(program.as_ref(), [s] if aliases || s.location.end > first.len()) => {
                Ok(Some((program, size, lines)))
            }
            // Expanded alias locations describe shell text, not physical input.
            Ok(_) if aliases => Err(size),
            Ok(program) => {
                // Quoted text and here-documents may look like nested headers.
                // If the scan overshot, the parser already located the first
                // command. Reparse its physical line boundary so option and
                // alias changes still take effect before the following read.
                let end = program
                    .as_ref()
                    .first()
                    .and_then(|s| s.location.end.checked_sub(first.len()))
                    .ok_or(size)?;
                let newline =
                    memchr::memchr(b'\n', buffered.get(end..size).ok_or(size)?).ok_or(size)?;
                let boundary = end + newline + 1;
                if boundary >= size {
                    return Err(size);
                }
                let unit = parse(boundary).map_err(|_| size)?;
                if !matches!(unit.as_ref(), [s] if s.location.end > first.len()) {
                    return Err(size);
                }
                let lines = memchr::memchr_iter(b'\n', &buffered[..boundary]).count();
                Ok(Some((unit, boundary, lines)))
            }
            Err(parser::Error::Incomplete(_) | parser::Error::HereDoc(_)) => Ok(None),
            _ => Err(size),
        }
    };
    let final_line = at_eof && !buffered.is_empty() && !buffered.ends_with(b"\n");
    let ends = memchr::memchr_iter(b'\n', buffered).chain(final_line.then_some(buffered.len()));
    for (i, end) in ends.enumerate() {
        let size = (end + 1).min(buffered.len());
        let next = &buffered[start..end];
        start = size;
        // A closing reserved word is a token. Suffixes such as `${value}`
        // and `config_done` cannot complete the compound.
        let separator = |b: &u8| b.is_ascii_whitespace() || b";&|()<>".contains(b);
        let next = next.trim_ascii();
        // Inner terminators otherwise cause a full reparse of a growing prefix
        // at every fi/done/brace. Count common nested headers as a hint only.
        // Quotes and here-documents can resemble shell code. The parser still
        // validates the entire unit, and a missed boundary uses the line reader.
        let word = next.split(|b| b.is_ascii_whitespace() || *b == b';').next();
        let opens = match close {
            b"fi" => word == Some(b"if"),
            b"esac" => word == Some(b"case"),
            b"done" => matches!(word, Some(b"for" | b"while" | b"until" | b"select")),
            _ if needs_open_brace && next.starts_with(b"{") => {
                needs_open_brace = false;
                false
            }
            _ => {
                next == b"{"
                    || next.ends_with(b"{")
                        && std::str::from_utf8(next).is_ok_and(parser::opens_function)
            }
        };
        nested += usize::from(opens);
        let begins = next
            .strip_prefix(close)
            .is_some_and(|rest| rest.first().is_none_or(separator));
        let ends = next
            .strip_suffix(close)
            .is_some_and(|rest| rest.last().is_none_or(separator));
        if !begins && !ends {
            continue;
        }
        if nested > 0 {
            nested -= 1;
            deferred = Some((size, i + 1));
            continue;
        }
        deferred = None;
        if let Some(unit) = probe(size, i + 1)? {
            return Ok(unit);
        }
    }
    // A literal header inside a quote or here-document can hide the outer close.
    if let Some((size, lines)) = deferred
        && let Some(unit) = probe(size, lines)?
    {
        return Ok(unit);
    }
    Err(buffered.len())
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

enum Parsed<P> {
    Program(P),
    More(Option<parser::HereDocHint>),
    Recovered,
    Failed(Flow),
}

fn parsed<P>(
    result: Result<P, parser::Error>,
    more: bool,
    session: &Session,
    check: bool,
    line: usize,
) -> Parsed<P> {
    match result {
        Ok(program) => Parsed::Program(program),
        Err(parser::Error::HereDoc(hint)) if more => Parsed::More(Some(hint)),
        Err(parser::Error::Incomplete(_)) if more => Parsed::More(None),
        Err(e @ parser::Error::ArrayToken)
            if !check && !session.posix && !session.errexit && !session.noexec =>
        {
            syntax_error(session, e, line);
            Parsed::Recovered
        }
        Err(e) => Parsed::Failed(syntax_error(session, e, line)),
    }
}

fn run(
    session: &mut Session,
    mut reader: BufReader<PrivateFile>,
    mut shared_offset: bool,
    from_stdin: bool,
    file_len: Option<u64>,
    mut status: u8,
    check: bool,
) -> Flow {
    let mut assignment_value = String::new();
    let mut base_line = 1;
    let mut input = Vec::new();
    let mut here_doc: Option<parser::HereDocHint> = None;
    let mut history_quote = None;
    let mut ran = false;
    loop {
        let line_start = input.len();
        let n = match reader.read_until(b'\n', &mut input) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
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
            && here_doc.is_none()
            && session.histexpand
            && session.history_enabled
            && session.source_depth == 0
        {
            let Ok(line) = std::str::from_utf8(&input[line_start..]) else {
                return error("expected UTF-8 command text", 2);
            };
            match history_input(session, line, history_quote) {
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
        if let Some(hint) = &mut here_doc {
            let Ok(line) = std::str::from_utf8(&input[line_start..]) else {
                return error("expected UTF-8 command text", 2);
            };
            if n != 0 && !hint.may_finish(line) {
                continue;
            }
            here_doc = None;
        }
        let Ok(text) = std::str::from_utf8(&input) else {
            return error("expected UTF-8 command text", 2);
        };
        // At a new physical line these tokens cannot open a command. Keep
        // accumulated compound commands, history expansion and NUL diagnostics.
        if line_start == 0 && n != 0 && blank_or_comment(text, session) {
            if !check
                && !session.interactive
                && session.source_depth == 0
                && session.begin_history_line(text)
            {
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
            && !session.traps.active()
            && !session.traps.in_action()
            && session.coprocesses.is_empty()
            && session.jobs.is_empty()
            && let Some(command) = text.strip_suffix('\n')
            && assign_buffered_scalar(session, command, &mut assignment_value)
        {
            session.current_command = Some(parser::Location::label(command.into(), base_line));
            session.pipe_status.single(0);
            session.command_number += usize::from(session.source_depth == 0);
            session.substitutions.finish(session.substitutions.scope());
            ran = true;
            status = 0;
            base_line += 1;
            input.clear();
            continue;
        }
        let buffered = line_start == 0 && n != 0 && !shared_offset && !session.verbose;
        let open_function = buffered && parser::opens_function(text) && !session.reads_aliases();
        let mut result = if open_function {
            // A function header cannot execute before its compound body.
            Err(parser::Error::Incomplete(parser::Incomplete::Message(
                "unfinished function definition",
            )))
        } else {
            parser::session_source_at(text, n == 0, session, base_line)
        };
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
            if let Ok((program, consumed, lines)) = buffered_compound(
                text,
                reader.buffer(),
                false,
                session.reads_aliases(),
                |size| {
                    let rest = std::str::from_utf8(&reader.buffer()[..size])
                        .map_err(|_| parser::Error::Invalid("expected UTF-8 command text"))?;
                    let mut unit = String::with_capacity(text.len() + size);
                    unit.push_str(text);
                    unit.push_str(rest);
                    parser::session_source_at(&unit, false, session, base_line)
                },
            ) {
                reader.consume(consumed);
                base_line += lines;
                result = Ok(program);
            }
        }
        history_quote = result.as_ref().err().and_then(parser::Error::pending_quote);
        match parsed(result, n != 0, session, check, base_line) {
            Parsed::More(hint) => {
                here_doc = hint;
                continue;
            }
            Parsed::Recovered => {
                status = 1;
                session.pipe_status.single(status);
                ran = true;
            }
            Parsed::Failed(flow) => return flow,
            Parsed::Program(program) if !program.is_empty() && !check => {
                if shared_offset && !reader.buffer().is_empty() {
                    // Unlike stream_position(), this discards the read-ahead buffer.
                    #[allow(clippy::seek_from_current)]
                    if let Err(e) = reader.seek(io::SeekFrom::Current(0)) {
                        return error(e, 1);
                    }
                }
                ran = true;
                let remember = session.source_depth == 0
                    && !session.interactive
                    && session.begin_history_line(text);
                let flow = execute_program(session, program, status, false);
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
                    Flow::Discard(code) | Flow::Continue(code) => status = code,
                    other => return other,
                }
            }
            Parsed::Program(_) => {}
        }
        base_line += text.bytes().filter(|&b| b == b'\n').count();
        input.clear();
        if n == 0 {
            break;
        }
    }
    Flow::Continue(if ran { status } else { 0 })
}

// A syntax error on a later physical line must not undo an already executed
// complete command. Keep borrowed slices on the -c path.
pub fn text(
    session: &mut Session,
    source: &str,
    mut status: u8,
    tail: bool,
    check: bool,
    eval_line: Option<usize>,
) -> Flow {
    let first_line = eval_line.unwrap_or(1);
    if !session.verbose
        && source.len() >= 32 * 1024
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
    let mut here_doc: Option<parser::HereDocHint> = None;
    let mut history_quote = None;
    let mut ran = false;
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
                eprint!("{line}");
            } else {
                eprintln!("{line}");
            }
        }
        let new_command = start == end;
        end += line.len();
        let mut last = end == source.len();
        let expand_history = tail
            && session.histexpand
            && session.history_enabled
            && (!session.command_string || session.command_history_expansion);
        if expand_history && here_doc.is_none() {
            match history_input(session, line, history_quote) {
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
        if !last && here_doc.as_mut().is_some_and(|hint| !hint.may_finish(line)) {
            continue;
        }
        here_doc = None;
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
        let open_function = probe && parser::opens_function(text) && !session.reads_aliases();
        let parse = |session: &mut Session, unit: &str, eof| {
            if eval_line.is_some() {
                eval::parse(session, unit, eof, base_line)
            } else {
                parser::session_source_at(unit, eof, session, base_line).map(eval::Program::Fresh)
            }
        };
        let mut result = if open_function {
            Err(parser::Error::Incomplete(parser::Incomplete::Message(
                "unfinished function definition",
            )))
        } else {
            parse(session, text, last)
        };
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
            match buffered_compound(
                text,
                &source.as_bytes()[end..limit],
                limit == source.len(),
                session.reads_aliases(),
                |size| parse(session, &source[start..end + size], false),
            ) {
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
        history_quote = result.as_ref().err().and_then(parser::Error::pending_quote);
        match parsed(result, !last, session, check, base_line) {
            Parsed::More(hint) => {
                here_doc = hint;
                continue;
            }
            Parsed::Recovered => {
                status = 1;
                session.pipe_status.single(status);
                ran = true;
            }
            Parsed::Failed(flow) => return flow,
            Parsed::Program(program) if !program.as_ref().is_empty() && !check => {
                ran = true;
                let remember = tail
                    && (!session.interactive || session.command_history_expansion)
                    && session.begin_history_line(text);
                let flow = program.run(session, status, tail && last);
                session.command_number += usize::from(tail && !session.interactive);
                if remember {
                    session.finish_history_line();
                }
                match flow {
                    Flow::Discard(code) | Flow::Continue(code) => status = code,
                    other => return other,
                }
            }
            Parsed::Program(_) => {}
        }
        base_line = line_number + 1;
        start = end;
        edited = None;
    }
    Flow::Continue(if ran { status } else { 0 })
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
            eprintln!("tonari: {e}");
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
            eprintln!("tonari: {}", e.message());
            2
        }
    }
}
