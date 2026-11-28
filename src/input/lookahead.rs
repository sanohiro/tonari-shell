use crate::{Session, parser};
use std::fs::File;
use std::os::unix::fs::FileExt;

const BULK_CLOSE_MIN_BYTES: usize = 1024;
const GROUP_MIN_BYTES: usize = 32 * 1024;

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

pub(super) fn whole_group(
    session: &Session,
    text: &str,
    first_line: usize,
) -> Option<Vec<parser::Statement>> {
    if text.len() < GROUP_MIN_BYTES || !starts_group(text.as_bytes()) {
        return None;
    }
    let program = parser::session_source_at(text, false, session, first_line).ok()?;
    if matches!(program.as_slice(), [s] if matches!(s.command, parser::Command::Group(_))) {
        Some(program)
    } else {
        None
    }
}

pub(super) fn file_group(
    session: &Session,
    file: &File,
    meta: &std::fs::Metadata,
) -> Option<(Vec<parser::Statement>, usize)> {
    if !meta.is_file() || !(GROUP_MIN_BYTES as u64..=8 * 1024 * 1024).contains(&meta.len()) {
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
    let program = whole_group(session, std::str::from_utf8(&bytes).ok()?, 1)?;
    Some((program, 1 + memchr::memchr_iter(b'\n', &bytes).count()))
}

pub(super) fn parse_buffered(
    first: &str,
    rest: &[u8],
    session: &Session,
    line: usize,
) -> Result<Vec<parser::Statement>, parser::Error> {
    let rest = std::str::from_utf8(rest)
        .map_err(|_| parser::Error::Invalid("expected UTF-8 command text"))?;
    let mut unit = String::with_capacity(first.len() + rest.len());
    unit.push_str(first);
    unit.push_str(rest);
    parser::session_source_at(&unit, false, session, line)
}

// A script on regular stdin shares its offset with read and external commands.
// Inspect one larger window without advancing that offset. Commit only the
// boundary of a complete unit accepted by the ordinary parser.
pub(super) fn shared_compound(
    reader: &mut std::io::BufReader<crate::execute::fd::PrivateFile>,
    first: &str,
    session: &Session,
    line: usize,
) -> Option<(Vec<parser::Statement>, usize)> {
    use std::io::{Seek, SeekFrom};
    const WINDOW: u64 = 64 * 1024;

    let offset = reader.stream_position().ok()?;
    let file = reader.get_ref();
    let meta = file.metadata().ok()?;
    let bytes = meta.len().saturating_sub(offset).min(WINDOW) as usize;
    if !meta.is_file() || bytes <= reader.buffer().len() {
        return None;
    }
    let mut data = vec![0; bytes];
    let n = file.read_at(&mut data, offset).ok()?;
    let (program, consumed, lines) = buffered_compound(first, &data[..n], false, false, |count| {
        parse_buffered(first, &data[..count], session, line)
    })
    .ok()?;
    reader
        .seek(SeekFrom::Start(offset + consumed as u64))
        .ok()?;
    Some((program, lines))
}

// Adjacent function declarations can share one source buffer. Only commit
// a batch after the normal parser has verified every top-level statement.
pub(super) fn function_run(
    first: &str,
    rest: &[u8],
    session: &Session,
    parse: impl FnOnce(usize) -> Result<Vec<parser::Statement>, parser::Error>,
) -> Option<(Vec<parser::Statement>, usize, usize)> {
    if session.interactive
        || session.xtrace
        || session.traps.observes_commands()
        || session.reads_aliases()
        || !parser::opens_function(first)
    {
        return None;
    }
    // Bound both speculative scanning and the source retained by one function.
    const WINDOW: usize = 8192;
    let bytes = &rest[..rest.len().min(WINDOW.saturating_sub(first.len()))];
    let mut body_start = 0;
    let mut end = 0;
    let mut count = 0;
    for offset in memchr::memmem::find_iter(bytes, b"\n}\n") {
        // A one-line body already has a cheaper parser that stores literal
        // wrappers without constructing their argument vectors.
        if bytes
            .get(body_start..offset)
            .is_none_or(|body| memchr::memchr(b'\n', body.trim_ascii()).is_none())
        {
            break;
        }
        end = offset + 3;
        count += 1;
        let rest = &bytes[end..];
        let Some(n) = memchr::memchr(b'\n', rest) else {
            break;
        };
        if !std::str::from_utf8(&rest[..n]).is_ok_and(parser::opens_function) {
            break;
        }
        body_start = end + n + 1;
    }
    if count < 4 {
        return None;
    }
    let program = parse(end).ok()?;
    if program.len() != count
        || !program.iter().all(|s| {
            !s.negated
                && s.condition == parser::Condition::Always
                && matches!(s.command, parser::Command::Function { .. })
        })
    {
        return None;
    }
    Some((
        program,
        end,
        memchr::memchr_iter(b'\n', &bytes[..end]).count(),
    ))
}

fn separator(b: &u8) -> bool {
    b.is_ascii_whitespace() || b";&|()<>".contains(b)
}

// In `:; fi 2>log` the terminator is an interior token. The same rule
// covers a case arm ending in `;; esac`. The parser validates every hint.
fn inline_close(line: &[u8], close: &[u8]) -> bool {
    memchr::memchr2_iter(b';', b'&', line).any(|i| {
        line[i + 1..]
            .trim_ascii_start()
            .strip_prefix(close)
            .is_some_and(|rest| rest.first().is_none_or(separator))
    })
}

// A sourced profile often has a long compound command. Try whole units already
// in the buffer, leaving following commands for the current alias/options state.
// On a miss report the distance inspected, so string input can bound repeated scans.
pub(crate) fn buffered_compound<P: AsRef<[parser::Statement]>>(
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
        _ if parser::opens_function(header) => b"}",
        _ if header.starts_with('(') || header.contains("=(") || header.contains("$(") => b")",
        _ if header.contains("()") => b"}",
        _ => return Err(0),
    };
    let mut start = 0;
    let mut nested = 0usize;
    let mut needs_open_brace = close == b"}" && !header.contains('{');
    let mut deferred = None;
    let mut next_probe = 0;
    let mut probe = |size, lines| {
        // Only the final ordinary read may emit an unterminated here-doc warning.
        match parse(size) {
            // Alias expansion changes offsets. Its callers only speculate on
            // private input and accept a single compound.
            Ok(program) if aliases && program.as_ref().len() == 1 => {
                Ok(Some((program, size, lines)))
            }
            Ok(_) if aliases => Err(size),
            Ok(program) => {
                // A quoted header and a later comment resembling its terminator
                // can overshoot even when the parser returned just one statement.
                // A command reading shared stdin must see those trailing bytes.
                let end = program
                    .as_ref()
                    .first()
                    .and_then(|s| s.location.end.checked_sub(first.len()))
                    .filter(|&end| end > 0 && end <= size)
                    .ok_or(size)?;
                let boundary = memchr::memchr(b'\n', &buffered[end..size])
                    .map_or(size, |newline| end + newline + 1);
                if boundary == size {
                    return Ok(Some((program, size, lines)));
                }
                // Reparse the physical closing line, including any other
                // commands on it. Here-docs or escaped newlines can require
                // more input, in which case leave the reader untouched.
                let unit = parse(boundary).map_err(|_| size)?;
                if unit
                    .as_ref()
                    .first()
                    .is_none_or(|s| s.location.end != first.len() + end)
                {
                    return Err(size);
                }
                let lines = memchr::memchr_iter(b'\n', &buffered[..boundary]).count();
                Ok(Some((unit, boundary, lines)))
            }
            Err(parser::Error::Incomplete(_) | parser::Error::HereDoc(_)) => Ok(None),
            _ => Err(size),
        }
    };
    // Indented compound commands inside functions can have many closing
    // braces. Try the first flush-left brace before probing their prefixes.
    // This is one bounded hint; a brace in a quote or here-doc can miss.
    const FUNCTION_CLOSE_WINDOW: usize = 8192;
    let nearby = &buffered[..buffered.len().min(FUNCTION_CLOSE_WINDOW)];
    if close == b"}"
        && !aliases
        && parser::opens_function(header)
        && let Some(at) = memchr::memmem::find(nearby, b"\n}")
        && let Some(len) = memchr::memchr(b'\n', &nearby[at + 2..])
    {
        let size = at + 3 + len;
        let lines = memchr::memchr_iter(b'\n', &buffered[..size]).count();
        if let Some(unit) = probe(size, lines)? {
            return Ok(unit);
        }
    }
    if close == b")" {
        // Inner substitutions, arithmetic and quoted parentheses can resemble
        // a terminator. Probe candidate physical lines at doubling distances.
        let mut probe_after = 0;
        let mut deferred = None;
        let mut offset = 0;
        while let Some(hit) = memchr::memchr(b')', &buffered[offset..]) {
            let at = offset + hit;
            let end = match memchr::memchr(b'\n', &buffered[at..]) {
                Some(n) => at + n + 1,
                None if at_eof => buffered.len(),
                None => break,
            };
            offset = end;
            if end < probe_after {
                deferred = Some(end);
                continue;
            }
            deferred = None;
            let lines = memchr::memchr_iter(b'\n', &buffered[..end]).count()
                + usize::from(!buffered[..end].ends_with(b"\n"));
            if let Some(unit) = probe(end, lines)? {
                return Ok(unit);
            }
            probe_after = end.saturating_mul(2);
        }
        if let Some(end) = deferred {
            let lines = memchr::memchr_iter(b'\n', &buffered[..end]).count()
                + usize::from(!buffered[..end].ends_with(b"\n"));
            if let Some(unit) = probe(end, lines)? {
                return Ok(unit);
            }
        }
        return Err(buffered.len());
    }
    // Large, flat compounds often have only one possible terminator. Locate
    // its line in bulk; nested or ambiguous text uses the line scan below.
    if !aliases && buffered.len() >= BULK_CLOSE_MIN_BYTES {
        let mut hits = memchr::memmem::find_iter(buffered, close);
        let at = hits.next().ok_or(buffered.len())?;
        if hits.next().is_none()
            && (at == 0 || separator(&buffered[at - 1]))
            && buffered.get(at + close.len()).is_none_or(separator)
        {
            let size = match memchr::memchr(b'\n', &buffered[at..]) {
                Some(offset) => at + offset + 1,
                None if at_eof => buffered.len(),
                None => return Err(buffered.len()),
            };
            let bytes = &buffered[..size];
            let lines =
                memchr::memchr_iter(b'\n', bytes).count() + usize::from(!bytes.ends_with(b"\n"));
            return probe(size, lines).ok().flatten().ok_or(buffered.len());
        }
    }
    let final_line = at_eof && !buffered.is_empty() && !buffered.ends_with(b"\n");
    let ends = memchr::memchr_iter(b'\n', buffered).chain(final_line.then_some(buffered.len()));
    for (i, end) in ends.enumerate() {
        let size = (end + 1).min(buffered.len());
        let next = &buffered[start..end];
        start = size;
        // A closing reserved word is a token. Suffixes such as `${value}`
        // and `config_done` cannot complete the compound.
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
        let inline = !begins && !ends;
        if inline && !inline_close(next, close) {
            continue;
        }
        if nested > 0 {
            nested -= 1;
            deferred = Some((size, i + 1));
            continue;
        }
        if size < next_probe {
            deferred = Some((size, i + 1));
            continue;
        }
        deferred = None;
        if let Some(unit) = probe(size, i + 1)? {
            return Ok(unit);
        }
        // Quoted and here-document data can contain complete fake terminator
        // lines as well as inline ones. Bound the sum of reparsed prefixes.
        next_probe = size.saturating_mul(2);
    }
    // A literal header inside a quote or here-document can hide the outer close.
    if let Some((size, lines)) = deferred
        && let Some(unit) = probe(size, lines)?
    {
        return Ok(unit);
    }
    Err(buffered.len())
}

#[test]
fn recognizes_redirected_terminators_without_reparsing_each_line() {
    for (header, end) in [
        ("{ :\n", ":; } 2>log"),
        ("if :; then\n", ":; fi 2>&1"),
        ("for k in x; do\n", ":; done >out"),
        ("case x in x)\n", ": ;; esac >out"),
    ] {
        let body = format!("{}{end}\n", "v=1\n".repeat(1000));
        let mut calls = 0;
        let (_, consumed, _) = buffered_compound(header, body.as_bytes(), true, false, |size| {
            calls += 1;
            parser::parse(&format!("{header}{}", &body[..size]))
        })
        .unwrap();
        assert_eq!((calls, consumed), (1, body.len()));
    }
}

#[test]
fn inline_terminators_inside_heredocs_have_bounded_parse_attempts() {
    let first = "if :; then\n";
    let text = format!(
        ": <<'END'\n{}END\n:; fi 2>log",
        "fi\n:; fi 2>fake\n".repeat(128)
    );
    let mut attempts = 0;
    let (_, consumed, lines) = buffered_compound(first, text.as_bytes(), true, false, |size| {
        attempts += 1;
        parser::parse(&format!("{first}{}", &text[..size]))
    })
    .unwrap();
    assert_eq!((consumed, lines), (text.len(), text.lines().count()));
    assert!(
        attempts <= 12,
        "{attempts} probes for 256 false terminators"
    );
}

#[test]
fn indented_groups_in_functions_probe_the_outer_boundary_once() {
    let first = "configured()\n";
    let unit = "{\n    : || {\n        v=0\n    }\n    echo \"$v\"\n}\n";
    let following = "next()\n{\n    v=1\n}\n".repeat(100);
    let text = format!("{unit}{following}");
    let mut attempts = Vec::new();
    let (_, consumed, lines) = buffered_compound(first, text.as_bytes(), true, false, |size| {
        attempts.push(size);
        parser::parse(&format!("{first}{}", &text[..size]))
    })
    .unwrap();
    assert_eq!(attempts, [unit.len()]);
    assert_eq!((consumed, lines), (unit.len(), unit.lines().count()));
}

#[test]
fn flush_left_braces_in_function_data_preserve_the_read_boundary() {
    let first = "configured()\n";
    for unit in [
        "{\n    v='line\n}\nlast'\n}\n",
        "{\n    cat <<EOF\n}\nEOF\n}\n",
        "{\n    v=2\n    }\n",
        "{\n    v=3\n} >out\n",
    ] {
        let text = format!("{unit}next()\n{{\n    :\n}}\n");
        let (_, consumed, lines) = buffered_compound(first, text.as_bytes(), true, false, |size| {
            parser::parse(&format!("{first}{}", &text[..size]))
        })
        .unwrap();
        assert_eq!((consumed, lines), (unit.len(), unit.lines().count()));
    }
}
