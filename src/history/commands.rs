use super::{Entries, History, InitialJournal, Session};
use crate::Flow;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::fd::FromRawFd;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

#[inline(never)]
pub(crate) fn run(session: &mut Session, words: &[String], status: u8) -> Flow {
    let args = &words[1..];
    if let Some(snapshot) = session.initial_history.take()
        && !(words[0] == "history" && args.first().is_some_and(|s| s == "-c"))
    {
        let _ = snapshot.load(session);
    }
    if words[0] == "fc" {
        match fc(session, args, status) {
            Ok(flow) => flow,
            Err(e) => {
                diagnostic!("fc: {e}");
                Flow::Continue(1)
            }
        }
    } else {
        match history(session, args) {
            Ok(status) => Flow::Continue(status),
            Err(e) => {
                diagnostic!("history: {e}");
                Flow::Continue(if e.kind() == io::ErrorKind::InvalidInput {
                    2
                } else {
                    1
                })
            }
        }
    }
}

fn error(message: &str) -> io::Error {
    io::Error::other(message)
}

fn history(session: &mut Session, mut args: &[String]) -> io::Result<u8> {
    let mut operation = None;
    while let Some(flag) = crate::builtins::flags::next(&mut args) {
        for c in flag[1..].chars() {
            if c == 'c' {
                let h = session.history_mut();
                *h = Entries {
                    imported: h.imported.take(),
                    incomplete_files: std::mem::take(&mut h.incomplete_files),
                    ..Entries::default()
                };
            } else if "dspawrn".contains(c) && operation.is_none() {
                operation = Some(c);
            } else if !"dspawrn".contains(c) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("-{c}: invalid option"),
                ));
            } else {
                return Err(error("invalid option combination"));
            }
        }
        if operation.is_some() {
            break;
        }
        if args.is_empty() {
            return Ok(0);
        }
    }
    match operation {
        Some('s') => {
            if !args.is_empty() {
                session.history_mut().discard_current();
                let text = args.join(" ");
                if session.record_replay(&text) && session.interactive {
                    History::persist(session, &text, session.get("PWD").as_deref(), 0);
                }
            }
        }
        Some('d') => {
            let arg = args
                .first()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "position required"))?;
            let h = session.history_mut();
            let (a, b) = arg
                .char_indices()
                .skip(1)
                .find(|&(_, c)| c == '-')
                .map_or((arg.as_str(), arg.as_str()), |(i, _)| {
                    (&arg[..i], &arg[i + 1..])
                });
            let first = position(h, a, h.lines.len())?;
            let last = position(h, b, h.lines.len())?;
            if first > last {
                return Err(error("history position out of range"));
            }
            h.remove_range(first, last + 1);
        }
        Some('p') => {
            session.history_mut().discard_current();
            let mut out = io::LineWriter::new(crate::builtins::StdoutFd(None));
            let mut status = 0;
            for arg in args {
                match super::expand::expand(session, arg) {
                    Ok(result) => writeln!(out, "{}", result.text)?,
                    Err(e) => {
                        diagnostic!("history: {arg}: {e}");
                        status = 1;
                    }
                }
            }
            return Ok(status);
        }
        Some(op) => history_file(session, op, args)?,
        None => {
            if args.len() > 1 {
                return Err(error("too many arguments"));
            }
            let count = args
                .first()
                .map(|x| x.parse::<usize>())
                .transpose()
                .map_err(|_| error("numeric argument required"))?
                .unwrap_or(usize::MAX);
            if let Some(h) = session.history.as_deref() {
                let format = session.get("HISTTIMEFORMAT").filter(|s| !s.is_empty());
                let mut out = io::BufWriter::new(crate::builtins::StdoutFd(None));
                let mut list = |stamp: &dyn Fn(i64) -> String| -> io::Result<()> {
                    for i in h.lines.len().saturating_sub(count)..h.lines.len() {
                        let timestamp =
                            if format.is_some() && h.timestamps[i] == super::UNKNOWN_TIME {
                                "??".into()
                            } else {
                                stamp(h.timestamps[i])
                            };
                        writeln!(out, "{:5}  {timestamp}{}", h.base + i + 1, h.lines[i])?;
                    }
                    out.flush()
                };
                if let Some(f) = &format {
                    crate::shell_time::with_formatter(session, f, list)?;
                } else {
                    list(&|_| String::new())?;
                }
            }
        }
    }
    Ok(0)
}

fn position(h: &Entries, arg: &str, end: usize) -> io::Result<usize> {
    let n = arg
        .parse::<i64>()
        .map_err(|_| error("invalid history position"))?;
    let i = if n < 0 {
        end as i64 + n
    } else {
        n - 1 - h.base as i64
    };
    usize::try_from(i)
        .ok()
        .filter(|i| *i < end)
        .ok_or_else(|| error(&format!("{arg}: history position out of range")))
}

fn locked_history(path: &Path, options: &OpenOptions) -> io::Result<File> {
    loop {
        let file = options.open(path)?;
        super::lock_exclusive(&file)?;
        let opened = file.metadata()?;
        let current = std::fs::metadata(path)?;
        if (opened.dev(), opened.ino()) == (current.dev(), current.ino()) {
            return Ok(file);
        }
        // A simultaneous limit change atomically replaced the old inode.
    }
}

// Read just the retained suffix, including both bytes of a marker when its
// start crosses a block boundary. Ignore empty lines and a trailing orphan marker.
fn timestamped_suffix(
    file: &File,
    len: u64,
    mut budget: usize,
    count_markers: bool,
) -> io::Result<u64> {
    const BLOCK: usize = 16384;
    let mut buf = [0u8; BLOCK + 2];
    let mut end = len;
    let mut content_lines = 0usize;
    while end > 0 {
        let start = end.saturating_sub(BLOCK as u64);
        let n = (end - start) as usize;
        let used = n + (len - end).min(2) as usize;
        file.read_exact_at(&mut buf[..used], start)?;
        for newline in memchr::memrchr_iter(b'\n', &buf[..n]) {
            let index = newline + 1;
            let tail = &buf[index..used];
            match tail {
                [b'#', digit, ..] if digit.is_ascii_digit() => {
                    if content_lines > 0 {
                        let lines = content_lines.saturating_add(usize::from(count_markers));
                        if lines >= budget {
                            return Ok(start + index as u64);
                        }
                        budget -= lines;
                    }
                    content_lines = 0;
                }
                [] | [b'\n', ..] | [b'\r', b'\n', ..] => {}
                _ => content_lines = content_lines.saturating_add(1),
            }
        }
        end = start;
    }
    Ok(0)
}

fn create_temp_file(path: PathBuf) -> io::Result<(File, PathBuf)> {
    let mut bytes = path.into_os_string().into_vec();
    if bytes.contains(&0) {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    bytes.push(0);
    let fd = unsafe { libc::mkstemp(bytes.as_mut_ptr().cast()) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        let error = io::Error::last_os_error();
        unsafe { libc::unlink(bytes.as_ptr().cast()) };
        return Err(error);
    }
    bytes.pop();
    Ok((file, std::ffi::OsString::from_vec(bytes).into()))
}

// Replace the target only after writing and restoring its permissions.
fn replace_history(
    path: &Path,
    mode: std::fs::Permissions,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<std::fs::Metadata> {
    let (mut copy, temp) = create_temp_file(path.with_file_name(".tonari-history-XXXXXX"))?;
    let result = (|| {
        write(&mut copy)?;
        copy.set_permissions(mode)?;
        let meta = copy.metadata()?;
        std::fs::rename(&temp, path)?;
        Ok(meta)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

pub(super) fn limit_plain_file(
    path: &Path,
    limit: usize,
    timed: bool,
    seen: Option<&mut InitialJournal>,
) -> io::Result<()> {
    let path = std::fs::canonicalize(path)?;
    let mut source = locked_history(
        &path,
        OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK),
    )?;
    let metadata = source.metadata()?;
    if !metadata.is_file() || metadata.len() <= limit as u64 {
        return Ok(());
    }
    let mut pos = metadata.len();
    let mut header = [0; 2];
    if limit > 0
        && source.read_at(&mut header, 0)? == 2
        && header[0] == b'#'
        && header[1].is_ascii_digit()
    {
        pos = timestamped_suffix(&source, metadata.len(), limit, !timed)?;
        if pos == 0 {
            return Ok(());
        }
    } else if limit > 0 {
        pos = super::tail_lines_offset(&source, pos, limit, true)?;
        if pos == 0 {
            return Ok(());
        }
    }
    source.seek(SeekFrom::Start(pos))?;
    let new = replace_history(&path, metadata.permissions(), |copy| {
        io::copy(&mut source, copy).map(|_| ())
    })?;
    if let Some(seen) = seen
        && seen.identity == (metadata.dev(), metadata.ino())
    {
        seen.identity = (new.dev(), new.ino());
        seen.len = seen.len.saturating_sub(pos);
    }
    Ok(())
}

// A history file may be copied and extended under another name. Only skip
// previously imported bytes when that prefix still denotes the same commands.
fn imported_prefix(
    seen: &InitialJournal,
    file: &File,
    path: &Path,
    meta: &std::fs::Metadata,
) -> io::Result<u64> {
    if seen.len > meta.len() {
        return Ok(0);
    }
    if seen.identity == (meta.dev(), meta.ino()) {
        return Ok(seen.len);
    }
    if path == seen.path {
        return Ok(0);
    }
    let Ok(old) = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&seen.path)
    else {
        return Ok(0);
    };
    let original = old.metadata()?;
    if (original.dev(), original.ino()) != seen.identity
        || original.len() < seen.len
        || !original.is_file()
    {
        return Ok(0);
    }
    let (mut left, mut right) = ([0u8; 4096], [0u8; 4096]);
    let mut pos = 0;
    while pos < seen.len {
        let n = (seen.len - pos).min(left.len() as u64) as usize;
        old.read_exact_at(&mut left[..n], pos)?;
        file.read_exact_at(&mut right[..n], pos)?;
        if left[..n] != right[..n] {
            return Ok(0);
        }
        pos += n as u64;
    }
    Ok(seen.len)
}

fn import_entry(h: &mut Entries, value: String, time: i64, limit: usize) {
    h.append_line(value, false, false);
    h.timestamps.push(time);
    h.current = None;
    // Trim bounded batches while the intermediate window is unobservable.
    if h.lines.len().saturating_sub(limit) >= 1024 {
        h.trim_to(limit);
    }
}

fn bash_timestamp(line: &str) -> Option<i64> {
    let value = line.strip_prefix('#')?;
    let end = value
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(value.len());
    value[..end].parse().ok()
}

// Validate complete buffered lines together. Read through a boundary only
// when a single line spans buffers, or when an error needs its exact line.
fn read_history_lines(
    reader: &mut impl BufRead,
    private: bool,
    mut visit: impl FnMut(&str, usize),
) -> io::Result<()> {
    let mut spill = String::new();
    loop {
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            return Ok(());
        }
        if let Some(end) = memchr::memrchr(b'\n', bytes)
            && let Ok(lines) = std::str::from_utf8(&bytes[..=end])
        {
            for line in lines.split_terminator('\n') {
                visit(line.strip_suffix('\r').unwrap_or(line), line.len() + 1);
            }
            reader.consume(end + 1);
        } else {
            spill.clear();
            let count = reader.read_line(&mut spill)?;
            let line = match spill.strip_suffix('\n') {
                Some(line) => line,
                None if private => return Ok(()),
                None => spill.as_str(),
            };
            visit(line.strip_suffix('\r').unwrap_or(line), count);
        }
    }
}

pub(super) fn history_file(session: &mut Session, op: char, args: &[String]) -> io::Result<()> {
    if args.len() > 1 {
        return Err(error("too many arguments"));
    }
    let timestamped = session.get("HISTTIMEFORMAT").is_some();
    let histfile = session.get("HISTFILE");
    let chosen = args.first().map(String::as_str).or(histfile.as_deref());
    let default = session
        .personal_home()
        .map(|s| Path::new(s.as_ref()).join(".bash_history"));
    let chosen = chosen
        .filter(|s| !s.is_empty())
        .ok_or_else(|| error("history file is not set"))?;
    let private = args.is_empty() && default.as_deref() == Some(Path::new(chosen));
    if private && matches!(op, 'a' | 'w') {
        return Ok(());
    } // The private journal is written when a command finishes.
    let path = if private {
        super::file(session)
    } else {
        Some(PathBuf::from(chosen))
    };
    let Some(path) = path else { return Ok(()) };
    if matches!(op, 'r' | 'n') {
        read_history_file(session, path, op, private, timestamped)
    } else {
        write_history_file(session, &path, op, timestamped)
    }
}

// Only the final HISTSIZE valid records can survive an explicit private
// import. Count the earlier events for Bash's numbering without allocating
// or decoding their commands. Malformed tails use the ordinary reader.
fn bounded_journal(file: &mut File, len: u64, limit: usize) -> io::Result<Option<(u64, usize)>> {
    if len < 128 * 1024 || limit == 0 || limit as u64 > len / 8 {
        return Ok(None);
    }
    let start = super::tail_lines_offset(file, len, limit, false)?;
    if start == 0 {
        return Ok(None);
    }
    let mut count = |start, bytes| -> io::Result<usize> {
        file.seek(SeekFrom::Start(start))?;
        let mut rows = 0usize;
        let mut reader = BufReader::new((&*file).take(bytes));
        read_history_lines(&mut reader, true, |line, _| {
            if let Some(command) = super::command_field(line)
                && (!command.contains('\\') || super::decode(command).is_some())
            {
                rows += 1;
            }
        })?;
        Ok(rows)
    };
    if count(start, len - start)? < limit {
        return Ok(None);
    }
    Ok(Some((start, count(0, start)?)))
}

fn read_history_file(
    session: &mut Session,
    path: PathBuf,
    op: char,
    private: bool,
    timestamped: bool,
) -> io::Result<()> {
    let mut file = match File::open(&path) {
        Err(e) if private && e.kind() == io::ErrorKind::NotFound => return Ok(()),
        other => other?,
    };
    let meta = file.metadata()?;
    let identity = (meta.dev(), meta.ino());
    let limit = session.history_limit("HISTSIZE", super::DEFAULT_HISTSIZE);
    let skip = if op == 'r' && private && meta.is_file() {
        let retained = bounded_journal(&mut file, meta.len(), limit).ok().flatten();
        file.seek(SeekFrom::Start(0))?;
        if let Some((offset, skipped)) = retained {
            let history = session.history_mut();
            history.trim_to(0);
            history.base += skipped;
            offset
        } else {
            0
        }
    } else if op == 'n' && meta.is_file() {
        session
            .history
            .as_deref()
            .and_then(|h| h.imported.as_ref())
            .map_or(Ok(0), |seen| imported_prefix(seen, &file, &path, &meta))?
    } else {
        0
    };
    let mut reader = BufReader::new(file);
    let marked = if private || !meta.is_file() {
        false
    } else if skip > 0 {
        let mut header = [0; 2];
        reader.get_ref().read_at(&mut header, 0)? == 2
            && header[0] == b'#'
            && header[1].is_ascii_digit()
    } else {
        matches!(reader.fill_buf()?, [b'#', digit, ..] if digit.is_ascii_digit())
    };
    let mut offset = skip;
    let mut timestamp = None;
    let mut marker_offset = None;
    let mut multiline = marked && timestamped;
    let mut recognize = marked || timestamped;
    let default_time = if timestamped || marked {
        unsafe { libc::time(std::ptr::null_mut()) }
    } else {
        super::UNKNOWN_TIME
    };
    if skip > 0 {
        reader.seek(SeekFrom::Start(skip))?;
    }
    let owner = session.shell_pid();
    let h = session.history_mut();
    let mut pending: Option<(String, i64)> = None;
    let mut previous = if skip > 0 {
        h.pending.iter().rposition(|pending| !pending)
    } else {
        None
    };
    let result = read_history_lines(&mut reader, private, |line, n| {
        offset += n as u64;
        let (value, seconds) = if private {
            let Some(record) = super::journal_record(line) else {
                return;
            };
            let pid = record.pid.parse::<u32>().ok();
            if op == 'n' && pid == Some(owner) {
                return;
            }
            (
                super::decode(record.command),
                record
                    .time
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(super::UNKNOWN_TIME),
            )
        } else {
            if offset == n as u64
                && line.starts_with('#')
                && line.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
            {
                recognize = true;
                multiline = timestamped;
            }
            if recognize && let Some(ts) = bash_timestamp(line) {
                if let Some((value, time)) = pending.take() {
                    import_entry(h, value, time, limit);
                }
                previous = None;
                timestamp = Some(ts);
                marker_offset = Some(offset - n as u64);
                return;
            }
            if multiline
                && timestamp.is_none()
                && let Some(last) = pending.as_mut().map(|(text, _)| text).or_else(|| {
                    previous.and_then(|i| {
                        h.erase_counts = None;
                        h.lines.get_mut(i)
                    })
                })
            {
                let added = 1 + line.len();
                if last.capacity() - last.len() < added {
                    // Only multiline entries need headroom for continuation lines.
                    last.reserve(added.max(32));
                }
                last.push('\n');
                last.push_str(line);
                return;
            }
            if line.is_empty() {
                return;
            }
            marker_offset = None;
            (
                Some(line.to_owned()),
                timestamp.take().unwrap_or(default_time),
            )
        };
        if let Some(value) = value
            && limit != 0
        {
            if multiline && !private {
                pending = Some((value, seconds));
            } else {
                import_entry(h, value, seconds, limit);
            }
        }
    });
    if let Some((value, time)) = pending {
        import_entry(h, value, time, limit);
    }
    h.trim_to(limit);
    if result.is_err() {
        if !h.incomplete_files.contains(&identity) {
            h.incomplete_files.push(identity);
        }
    } else if op == 'r' {
        h.incomplete_files.retain(|id| *id != identity);
    }
    h.imported = Some(InitialJournal {
        path,
        identity,
        len: marker_offset.unwrap_or(offset),
    });
    result
}

fn write_history_file(
    session: &mut Session,
    path: &Path,
    op: char,
    timestamped: bool,
) -> io::Result<()> {
    let file = locked_history(
        path,
        OpenOptions::new()
            .write(true)
            .create(true)
            .append(op == 'a')
            .mode(0o600),
    )?;
    let meta = file.metadata()?;
    let h = session.history_mut();
    if op == 'w' && h.incomplete_files.contains(&(meta.dev(), meta.ino())) {
        return Err(error("refusing to overwrite incompletely read history"));
    }
    let first = if op == 'a' { h.written } else { 0 };
    let write = |target: &File| -> io::Result<()> {
        let mut output = io::BufWriter::new(target);
        if op == 'a' && meta.is_file() && meta.len() > 0 && h.pending[first..].contains(&true) {
            // Retain write-only and FIFO semantics on the primary descriptor.
            // Only a readable regular file needs this optional boundary check.
            let input = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(path);
            if let Ok(input) = input {
                let original = input.metadata()?;
                if (original.dev(), original.ino()) == (meta.dev(), meta.ino()) {
                    let mut last = [0];
                    input.read_exact_at(&mut last, meta.len() - 1)?;
                    if last != [b'\n'] {
                        output.write_all(b"\n")?;
                    }
                }
            }
        }
        for i in first..h.lines.len() {
            if op == 'a' && !h.pending[i] {
                continue;
            }
            let time = h.timestamps[i];
            if timestamped && time != super::UNKNOWN_TIME {
                writeln!(output, "#{time}")?;
            }
            writeln!(output, "{}", h.lines[i])?;
        }
        output.flush()
    };
    if op == 'w' && meta.is_file() {
        let path = std::fs::canonicalize(path)?;
        replace_history(&path, meta.permissions(), |file| write(file))?;
    } else {
        write(&file)?;
    }
    h.pending[first..].fill(false);
    h.written = h.lines.len();
    Ok(())
}

fn fc(session: &mut Session, mut args: &[String], status: u8) -> io::Result<Flow> {
    let (mut list, mut reverse, mut numbers, mut substitute) = (false, false, true, false);
    let mut editor = None;
    while let Some((arg, rest)) = args.split_first() {
        if arg == "--" {
            args = rest;
            break;
        }
        if !arg.starts_with('-') || arg == "-" || arg.parse::<i64>().is_ok() {
            break;
        }
        args = rest;
        let mut flags = arg[1..].chars();
        while let Some(flag) = flags.next() {
            match flag {
                'l' => list = true,
                'r' => reverse = true,
                'n' => numbers = false,
                's' => substitute = true,
                'e' => {
                    editor = Some(
                        crate::builtins::flags::value(flags.as_str(), &mut args)
                            .ok_or_else(|| error("editor required"))?,
                    );
                    break;
                }
                _ => return Err(error("invalid option")),
            }
        }
    }
    let replacements = if substitute {
        args.iter().take_while(|word| word.contains('=')).count()
    } else {
        0
    };
    let (replacements, args) = args.split_at(replacements);
    if session.posix && args.len() > if substitute { 1 } else { 2 } {
        return Err(error("too many arguments"));
    }
    let h = session.history.as_deref();
    let end = h.map_or(0, |h| h.current.unwrap_or(h.lines.len()));
    if end == 0 {
        return if list && !substitute {
            Ok(Flow::Continue(0))
        } else {
            Err(error("no command found"))
        };
    }
    let h = h.unwrap();
    let select = |arg: &String, first: bool| {
        if let Ok(n) = arg.parse::<i64>() {
            if n < 0 {
                return Ok(end.saturating_sub(n.unsigned_abs() as usize));
            }
            if n == 0 {
                return if arg.starts_with('-') {
                    if list {
                        Ok(h.lines.len() - 1)
                    } else {
                        Err(error("invalid history specification"))
                    }
                } else {
                    Ok(end - 1)
                };
            }
            let index = (n as usize).checked_sub(h.base + 1);
            Ok(index
                .filter(|i| *i < end)
                .unwrap_or(if first && !substitute { 0 } else { end - 1 }))
        } else {
            h.lines[..end]
                .iter()
                .rposition(|s| s.starts_with(arg))
                .ok_or_else(|| error("no command found"))
        }
    };
    let mut first = args
        .first()
        .map(|arg| select(arg, true))
        .transpose()?
        .unwrap_or_else(|| end.saturating_sub(if list { 16 } else { 1 }));
    let mut last = args
        .get(1)
        .map(|arg| select(arg, false))
        .transpose()?
        .unwrap_or(if list { first.max(end - 1) } else { first });
    if first > last {
        std::mem::swap(&mut first, &mut last);
        reverse = true;
    }
    let mut source = String::new();
    let mut out = list.then(|| io::LineWriter::new(crate::builtins::StdoutFd(None)));
    for step in 0..=last - first {
        let i = if reverse { last - step } else { first + step };
        if list && numbers {
            source.push_str(&(h.base + i + 1).to_string());
        }
        if list {
            source.push_str(if session.posix { "\t" } else { "\t " });
        }
        source.push_str(&h.lines[i]);
        source.push('\n');
        if source.len() >= 16384
            && let Some(out) = &mut out
        {
            out.write_all(source.as_bytes())?;
            source.clear();
        }
    }
    if let Some(mut out) = out {
        out.write_all(source.as_bytes())?;
        return Ok(Flow::Continue(0));
    }
    for replacement in replacements {
        let (before, after) = replacement.split_once('=').unwrap();
        if !before.is_empty() && source.contains(before) {
            source = source.replace(before, after);
        }
    }
    if !substitute && editor != Some("-") {
        let program = editor
            .map(str::to_owned)
            .filter(|s| !s.is_empty())
            .or_else(|| session.get("FCEDIT").filter(|s| !s.is_empty()))
            .or_else(|| session.get("EDITOR").filter(|s| !s.is_empty()))
            .unwrap_or_else(|| if session.posix { "ed" } else { "vi" }.into());
        source = match edit_text(session, &program, &source, status)? {
            Ok(s) => s,
            Err(f) => return Ok(f),
        };
    }
    session.history_mut().discard_current();
    if substitute || session.shopt.cmdhist {
        let entry = source.strip_suffix('\n').unwrap_or(&source);
        if !substitute && !session.shopt.lithist && entry.contains('\n') {
            session.record_replay(&crate::parser::history_command(entry));
        } else {
            session.record_replay(entry);
        }
    } else {
        for line in source.lines() {
            session.record_replay(line);
        }
    }
    stderr_text!("{source}");
    Ok(crate::eval_text(session, &source, status))
}

pub(crate) fn edit_text(
    session: &mut Session,
    program: &str,
    source: &str,
    status: u8,
) -> io::Result<Result<String, Flow>> {
    let directory = session
        .get("TMPDIR")
        .filter(|s| !s.is_empty())
        .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    let (mut file, path) = create_temp_file(directory.join("tonari-fc.XXXXXX"))?;
    let edited: io::Result<Result<String, Flow>> = (|| {
        file.write_all(source.as_bytes())?;
        drop(file);
        let name = path
            .to_str()
            .ok_or_else(|| error("expected UTF-8 temporary path"))?;
        let mut command = String::with_capacity(program.len() + name.len() + 3);
        command.push_str(program);
        command.push(' ');
        crate::word::quote::push_single_quote(&mut command, name);
        let flow = crate::eval_text(session, &command, status);
        if !matches!(flow, Flow::Continue(0)) {
            return Ok(Err(flow));
        }
        Ok(Ok(std::fs::read_to_string(&path)?))
    })();
    let _ = std::fs::remove_file(path);
    edited
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_records_survive_buffer_boundaries() {
        let bytes = "雪\r\n\nlast".as_bytes();
        for capacity in 1..=bytes.len() + 1 {
            for private in [true, false] {
                let mut reader = BufReader::with_capacity(capacity, bytes);
                let mut seen = Vec::new();
                read_history_lines(&mut reader, private, |s, n| seen.push((s.to_owned(), n)))
                    .unwrap();
                let mut expected = vec![("雪".to_owned(), 5), ("".to_owned(), 1)];
                if !private {
                    expected.push(("last".to_owned(), 4));
                }
                assert_eq!(seen, expected);
            }
            let mut invalid = BufReader::with_capacity(capacity, &b"one\n\xff\n"[..]);
            let mut prefix = Vec::new();
            let result = read_history_lines(&mut invalid, false, |s, _| prefix.push(s.to_owned()));
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
            assert_eq!(prefix, ["one"]);
        }
    }
}
