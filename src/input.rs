use crate::execute::fd::PrivateFile;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek};
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;

use crate::{Flow, Session, execute_program, parser};

// External commands inherit fd 0. Any unread text buffered by the shell must
// still be available to a child which reads stdin (for example, "cat\n...").
pub fn stdin_script(session: &mut Session, check: bool) -> Flow {
    let setup = (|| {
        let mut file = PrivateFile::duplicate(io::stdin().as_fd())?;
        let seekable = file.stream_position().is_ok();
        let reader = if seekable {
            BufReader::new(file)
        } else {
            BufReader::with_capacity(1, file)
        };
        Ok::<_, io::Error>((reader, seekable))
    })();
    match setup {
        Ok((reader, shared_offset)) => run(session, reader, shared_offset, 0, check),
        Err(e) => error(e, 1),
    }
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

fn whole_group(session: &Session, text: &str) -> Option<Vec<parser::Statement>> {
    if !starts_group(text.as_bytes()) {
        return None;
    }
    let program = parser::session_source(text, false, session).ok()?;
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
    whole_group(session, std::str::from_utf8(&bytes).ok()?)
}

pub fn file_script(
    session: &mut Session,
    file: File,
    metadata: Option<std::fs::Metadata>,
    status: u8,
    check: bool,
) -> Flow {
    let metadata = if session.verbose {
        None
    } else {
        metadata.or_else(|| file.metadata().ok())
    };
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
    // Keep complete profile functions in view across the usual 8 KiB buffer
    // boundary. Verbose mode must still report each line as it is read.
    let capacity = metadata
        .filter(|m| session.source_depth > 0 && m.is_file())
        .map_or(8192, |m| m.len().clamp(8192, 128 * 1024) as usize);
    match PrivateFile::new(file) {
        Ok(file) => run(
            session,
            BufReader::with_capacity(capacity, file),
            false,
            status,
            check,
        ),
        Err(e) => error(e, 1),
    }
}

fn error(message: impl std::fmt::Display, status: u8) -> Flow {
    eprintln!("tonari: {message}");
    Flow::Continue(status)
}

fn history_input<'a>(
    session: &mut Session,
    text: &'a str,
) -> Result<std::borrow::Cow<'a, str>, Flow> {
    let event = crate::history::expand::expand(session, text).map_err(|e| error(e, 1))?;
    if matches!(event.text, std::borrow::Cow::Owned(_)) {
        eprint!("{}", event.text);
        if !event.text.ends_with('\n') {
            eprintln!();
        }
    }
    if event.print {
        session.begin_history_line(&event.text);
        session.finish_history_line();
        return Ok("".into());
    }
    Ok(event.text)
}

// A sourced profile often has a long compound command. Try whole units already
// in the file buffer, leaving following commands for the current alias/options state.
fn buffered_compound(
    session: &Session,
    first: &str,
    buffered: &[u8],
    line: usize,
) -> Option<(Vec<parser::Statement>, usize, usize)> {
    let header = first.trim_ascii_start();
    let key = header
        .split(|c: char| c.is_ascii_whitespace() || c == ';')
        .next()?;
    let close: &[u8] = match key {
        "if" => b"fi",
        "case" => b"esac",
        "while" | "until" | "for" | "select" => b"done",
        "function" | "{" => b"}",
        _ if header.lines().next()?.contains("()") => b"}",
        _ => return None,
    };
    let mut size = 0;
    for (i, next) in buffered.split_inclusive(|&c| c == b'\n').enumerate() {
        size += next.len();
        // The last available slice may end in the middle of a physical line.
        if !next.ends_with(b"\n") || !next.trim_ascii_start().starts_with(close) {
            continue;
        }
        let mut text = String::with_capacity(first.len() + size);
        text.push_str(first);
        text.push_str(std::str::from_utf8(&buffered[..size]).ok()?);
        match parser::session_source_at(&text, false, session, line) {
            Ok(program) if matches!(program.as_slice(), [s] if s.location.end > first.len()) => {
                return Some((program, size, i + 1));
            }
            Err(parser::Error::Incomplete(_) | parser::Error::HereDoc(_)) => continue,
            _ => return None,
        }
    }
    None
}

fn run(
    session: &mut Session,
    mut reader: BufReader<PrivateFile>,
    shared_offset: bool,
    mut status: u8,
    check: bool,
) -> Flow {
    let mut base_line = 1;
    let mut input = Vec::new();
    let mut here_doc: Option<parser::HereDocHint> = None;
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
        let expanded;
        let text = if session.histexpand && session.history_enabled && session.source_depth == 0 {
            expanded = match history_input(session, text) {
                Ok(s) => s,
                Err(flow) => return flow,
            };
            expanded.as_ref()
        } else {
            text
        };
        // At a new physical line these tokens cannot open a command. Keep
        // accumulated compound commands, history expansion and NUL diagnostics.
        if line_start == 0 && n != 0 && {
            let first = text.trim_start_matches([' ', '\t']);
            first.is_empty()
                || first == "\n"
                || first.starts_with('#') && !first.trim_end_matches('\n').contains(['\n', '\0'])
        } {
            base_line += text.bytes().filter(|&b| b == b'\n').count();
            input.clear();
            continue;
        }
        let mut result = parser::session_source_at(text, n == 0, session, base_line);
        if matches!(result, Err(parser::Error::Incomplete(_)))
            && line_start == 0
            && !shared_offset
            && !session.verbose
            && session.source_depth > 0
            && let Some((program, consumed, lines)) =
                buffered_compound(session, text, reader.buffer(), base_line)
        {
            reader.consume(consumed);
            base_line += lines;
            result = Ok(program);
        }
        match result {
            Err(parser::Error::HereDoc(hint)) if n != 0 => {
                here_doc = Some(hint);
                continue;
            }
            Err(parser::Error::Incomplete(_)) if n != 0 => continue,
            Err(e) => return error(e.message(), 2),
            Ok(program) if !program.is_empty() && !check => {
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
            Ok(_) => {}
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
pub fn text(session: &mut Session, source: &str, mut status: u8, tail: bool, check: bool) -> Flow {
    if !session.verbose && source.len() >= 32 * 1024 {
        if let Some(program) = whole_group(session, source) {
            return if check {
                Flow::Continue(0)
            } else {
                match execute_program(session, program, status, tail) {
                    Flow::Discard(code) => Flow::Continue(code),
                    flow => flow,
                }
            };
        }
    }
    let mut start = 0;
    let mut end = 0;
    let mut here_doc: Option<parser::HereDocHint> = None;
    let mut ran = false;
    let mut base_line = 1;
    for (line_index, line) in source.split_inclusive('\n').enumerate() {
        if session.verbose {
            eprint!("{line}");
        }
        end += line.len();
        let last = end == source.len();
        if !last && here_doc.as_mut().is_some_and(|hint| !hint.may_finish(line)) {
            continue;
        }
        here_doc = None;
        let text = &source[start..end];
        let expanded;
        let text = if tail && session.histexpand && session.history_enabled {
            expanded = match history_input(session, text) {
                Ok(s) => s,
                Err(flow) => return flow,
            };
            expanded.as_ref()
        } else {
            text
        };
        let result = parser::session_source_at(text, last, session, base_line);
        match result {
            Err(parser::Error::HereDoc(hint)) if !last => {
                here_doc = Some(hint);
                continue;
            }
            Err(parser::Error::Incomplete(_)) if !last => continue,
            Err(e) => return error(e.message(), 2),
            Ok(program) if !program.is_empty() && !check => {
                ran = true;
                let remember = tail && !session.interactive && session.begin_history_line(text);
                let flow = execute_program(session, program, status, tail && last);
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
            Ok(_) => {}
        }
        base_line = line_index + 2;
        start = end;
    }
    Flow::Continue(if ran { status } else { 0 })
}
