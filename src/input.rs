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
        let mut file = PrivateFile::new(File::from(io::stdin().as_fd().try_clone_to_owned()?))?;
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

fn file_group(session: &Session, file: &File) -> Option<Vec<parser::Statement>> {
    let meta = file.metadata().ok()?;
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

pub fn file_script(session: &mut Session, file: File, status: u8, check: bool) -> Flow {
    if let Some(program) = file_group(session, &file) {
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
    match PrivateFile::new(file) {
        Ok(file) => run(session, BufReader::new(file), false, status, check),
        Err(e) => error(e, 1),
    }
}

fn error(message: impl std::fmt::Display, status: u8) -> Flow {
    eprintln!("tonari: {message}");
    Flow::Continue(status)
}

fn run(
    session: &mut Session,
    mut reader: BufReader<PrivateFile>,
    shared_offset: bool,
    mut status: u8,
    check: bool,
) -> Flow {
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
        let result = parser::session_source(text, n == 0, session);
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
                match execute_program(session, program, status, false) {
                    Flow::Discard(code) | Flow::Continue(code) => status = code,
                    other => return other,
                }
            }
            Ok(_) => {}
        }
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
    if source.len() >= 32 * 1024 {
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
    for line in source.split_inclusive('\n') {
        end += line.len();
        let last = end == source.len();
        if !last && here_doc.as_mut().is_some_and(|hint| !hint.may_finish(line)) {
            continue;
        }
        here_doc = None;
        let text = &source[start..end];
        let result = parser::session_source(text, last, session);
        match result {
            Err(parser::Error::HereDoc(hint)) if !last => {
                here_doc = Some(hint);
                continue;
            }
            Err(parser::Error::Incomplete(_)) if !last => continue,
            Err(e) => return error(e.message(), 2),
            Ok(program) if !program.is_empty() && !check => {
                ran = true;
                match execute_program(session, program, status, tail && last) {
                    Flow::Discard(code) | Flow::Continue(code) => status = code,
                    other => return other,
                }
            }
            Ok(_) => {}
        }
        start = end;
    }
    Flow::Continue(if ran { status } else { 0 })
}
