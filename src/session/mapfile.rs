use super::array::{Elements, Records};
use super::read::Input;
use super::{Session, is_identifier};
use crate::Flow;
use std::io::{self, BufRead, Read};
use std::os::fd::{AsRawFd, BorrowedFd};

pub(crate) struct Options<'a> {
    name: &'a str,
    fd: i32,
    origin: Option<u32>,
    skip: u32,
    limit: u32,
    delimiter: super::read::Delimiter<'a>,
    trim: bool,
    callback: Option<&'a str>,
    quantum: u32,
}

impl<'a> Options<'a> {
    fn parse(args: &'a [String]) -> Result<Self, (u8, String)> {
        let mut result = Self {
            name: "MAPFILE",
            fd: 0,
            origin: None,
            skip: 0,
            limit: 0,
            delimiter: super::read::Delimiter::NEWLINE,
            trim: false,
            callback: None,
            quantum: 5000,
        };
        let mut args = args;
        while let Some(arg) = crate::builtins::flags::next(&mut args) {
            let mut flags = arg[1..].chars();
            while let Some(flag) = flags.next() {
                if flag == 't' {
                    result.trim = true;
                    continue;
                }
                if !"dOnsucC".contains(flag) {
                    return Err((2, format!("-{flag}: invalid option")));
                }
                let value = crate::builtins::flags::value(flags.as_str(), &mut args)
                    .ok_or_else(|| (2, format!("-{flag}: option requires an argument")))?;
                match flag {
                    'd' => result.delimiter = super::read::Delimiter::first(value),
                    'O' => {
                        result.origin = Some(
                            value
                                .parse()
                                .map_err(|_| (1, format!("{value}: invalid array origin")))?,
                        )
                    }
                    'n' => {
                        result.limit = value
                            .parse()
                            .map_err(|_| (1, format!("{value}: invalid line count")))?
                    }
                    's' => {
                        result.skip = value
                            .parse()
                            .map_err(|_| (1, format!("{value}: invalid line count")))?
                    }
                    'u' => {
                        result.fd = value
                            .parse()
                            .map_err(|_| (1, format!("{value}: invalid file descriptor")))?
                    }
                    'C' => result.callback = Some(value),
                    'c' => {
                        result.quantum = value
                            .parse()
                            .ok()
                            .filter(|&n| n > 0)
                            .ok_or_else(|| (1, format!("{value}: invalid callback quantum")))?
                    }
                    _ => unreachable!(),
                }
                break;
            }
        }
        if let Some(name) = args.first() {
            result.name = name;
        }
        let name = result.name;
        if !is_identifier(name) {
            return Err((1, format!("{name}: not a valid identifier")));
        }
        Ok(result)
    }
}

// Full-array reads can keep their block buffer when only an interrupt or
// termination can stop the builtin. Both are handled after storing the prefix.
struct BulkInput<'a> {
    file: std::fs::File,
    signals: Option<&'a crate::traps::Traps>,
}

impl Read for BulkInput<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        loop {
            let result = if let Some(traps) = self.signals {
                crate::traps::wait_readable(self.file.as_raw_fd(), None, Some(traps))
                    .and_then(|_| self.file.read(out))
            } else {
                self.file.read(out)
            };
            if let Err(e) = &result
                && e.kind() == io::ErrorKind::Interrupted
                && !self.signals.is_some_and(|t| {
                    t.read_pending()
                        || crate::INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed)
                })
            {
                continue;
            }
            return result;
        }
    }
}

// A full replacement has no callback or later reader that can observe a
// partially filled array. Build dense indices in order with one bulk load.
fn read_all(
    options: &Options<'_>,
    here: Option<&[u8]>,
    signals: Option<&crate::traps::Traps>,
    store: impl FnMut(&str),
) -> io::Result<()> {
    if let Some(mut bytes) = here {
        return records(&mut bytes, options, store);
    }
    let fd = unsafe { BorrowedFd::borrow_raw(options.fd) }.try_clone_to_owned()?;
    let mut reader = io::BufReader::with_capacity(
        64 * 1024,
        BulkInput {
            file: fd.into(),
            signals,
        },
    );
    let result = records(&mut reader, options, store);
    if result.is_err() && !reader.buffer().is_empty() {
        // Preserve the next record for another reader on the same seekable fd.
        let unread = reader.buffer().len() as libc::off_t;
        unsafe {
            libc::lseek(options.fd, -unread, libc::SEEK_CUR);
        }
    }
    result
}

fn records(
    reader: &mut dyn BufRead,
    options: &Options<'_>,
    mut store: impl FnMut(&str),
) -> io::Result<()> {
    let mut pending = Vec::new();
    let mut skip = options.skip;
    let delimiter = options.delimiter.bytes();
    let delim = delimiter[0];
    let mut append = |bytes: &[u8], has_nul: bool| -> io::Result<()> {
        if skip > 0 {
            skip -= 1;
            return Ok(());
        }
        let bytes = if has_nul {
            &bytes[..memchr::memchr(0, bytes).unwrap_or(bytes.len())]
        } else {
            bytes
        };
        store(std::str::from_utf8(bytes).map_err(|_| io::Error::other("expected UTF-8 input"))?);
        Ok(())
    };
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            if !pending.is_empty() {
                append(&pending, true)?;
            }
            break;
        }
        let n = buf.len();
        let has_nul = delim != 0 && memchr::memchr(0, buf).is_some();
        // Only the unfinished record at the end of a buffer spans two reads.
        let mut start = 0;
        let last = *delimiter.last().unwrap();
        for i in memchr::memchr_iter(last, buf) {
            let record = &buf[start..=i];
            if delimiter.len() > 1 && !record.ends_with(delimiter) {
                // A UTF-8 delimiter can begin in the previous buffer and end
                // here. Check only its missing prefix across that boundary.
                let missing = delimiter.len().saturating_sub(record.len());
                if missing == 0
                    || !pending.ends_with(&delimiter[..missing])
                    || record != &delimiter[missing..]
                {
                    continue;
                }
            }
            start = i + 1;
            let remove = if options.trim || delim == 0 {
                delimiter.len()
            } else {
                0
            };
            let result = if pending.is_empty() {
                append(&record[..record.len() - remove], has_nul)
            } else {
                pending.extend_from_slice(record);
                let result = append(&pending[..pending.len() - remove], true);
                pending.clear();
                result
            };
            if let Err(e) = result {
                reader.consume(start);
                return Err(e);
            }
        }
        pending.extend_from_slice(&buf[start..]);
        reader.consume(n);
    }
    Ok(())
}

#[inline(never)]
pub(super) fn prepare_memory<'a>(
    session: &Session,
    args: &'a [String],
    bytes: &'a [u8],
) -> Option<super::MemoryInput<'a>> {
    let opts = Options::parse(args).ok()?;
    (opts.fd == 0
        && opts.limit == 0
        && opts.callback.is_none()
        && session.memory_array_target(opts.name))
    .then_some(super::MemoryInput::Mapfile(opts, bytes))
}

pub(crate) fn run(
    session: &mut Session,
    command: &str,
    args: &[String],
    last: u8,
    here: Option<(i32, &[u8])>,
) -> Flow {
    let options = match Options::parse(args) {
        Ok(options) => options,
        Err((code, message)) => {
            diagnostic!("{command}: {message}");
            return Flow::Continue(code);
        }
    };
    run_parsed(session, command, options, last, here)
}

pub(super) fn run_parsed(
    session: &mut Session,
    command: &str,
    options: Options<'_>,
    mut last: u8,
    here: Option<(i32, &[u8])>,
) -> Flow {
    let fd = options.fd;
    if crate::execute::fd::is_private(fd) || unsafe { libc::fcntl(fd, libc::F_GETFD) } == -1 {
        diagnostic!("{command}: {fd}: invalid file descriptor");
        return Flow::Continue(1);
    }
    let destination = match session.indexed_receiver(options.name, false) {
        Ok(target) => target,
        Err(e) => {
            diagnostic!("{command}: {}: {e}", options.name);
            return Flow::Continue(1);
        }
    };
    let here = here
        .filter(|&(source, _)| source == fd && destination == options.name)
        .map(|(_, bytes)| bytes);
    let options = Options {
        name: &destination,
        ..options
    };
    let origin = i64::from(options.origin.unwrap_or(0));
    let integer = session.integer(options.name);
    let letter_case = session
        .variables
        .get(options.name)
        .and_then(|v| v.letter_case);
    let mut watch = session.interactive || session.traps.active();
    if options.limit == 0 && options.callback.is_none() && !integer && !session.traps.read_handler()
    {
        let signals = watch.then_some(&session.traps);
        let result = if options.origin.is_some() {
            let mut var = if letter_case.is_none() {
                session.variables.remove(options.name)
            } else {
                session.declared_variable(options.name, false)
            }
            .unwrap_or_default();
            var.declared = true;
            var.array_assigned = true;
            let elements = var.make_array();
            let mut index = origin;
            let result = read_all(&options, here, signals, |line| {
                elements.insert(index, session.convert_case(line.to_owned(), letter_case));
                index += 1;
            });
            session.variables.insert(options.name.into(), var);
            result
        } else {
            let mut lines = Records::default();
            let result = read_all(&options, here, signals, |line| {
                if letter_case.is_some() {
                    lines.push(&session.convert_case(line.to_owned(), letter_case));
                } else {
                    lines.push(line);
                }
            });
            session.store_elements(options.name, Elements::Records(lines), false);
            result
        };
        if watch && session.traps.read_pending() {
            return crate::traps::dispatch(session, last);
        }
        if watch && crate::interrupted() {
            return Flow::Discard(crate::traps::signal_status(libc::SIGINT));
        }
        return match result {
            Ok(()) => Flow::Continue(0),
            Err(e) => {
                diagnostic!("{command}: {e}");
                Flow::Continue(1)
            }
        };
    }
    let mut input = Input::new(fd);
    if options.origin.is_none() {
        session.replace_array(options.name, Vec::new());
    } else {
        let var = session.variables.entry(options.name.into()).or_default();
        if var.elements.is_none() {
            var.make_array();
        }
        var.declared = true;
        var.array_assigned = true;
    }
    let mut skip = options.skip;
    let mut count = 0u64;
    let mut callback_command = String::new();
    let mut bytes = Vec::new();
    while options.limit == 0 || count < u64::from(options.limit) {
        bytes.clear();
        let mut any = false;
        let mut truncated = false;
        let mut delimiter_prefix = 0;
        loop {
            if watch {
                match input.wait(session, last, command, None, true) {
                    Ok(true) => {}
                    Ok(false) => {
                        watch = session.interactive || session.traps.active();
                        continue;
                    }
                    Err(flow) => return flow,
                }
            }
            // A regular descriptor can return its unused suffix before a
            // callback runs. Consume only one byte at a time on shared pipes.
            if input.seekable && !watch && options.delimiter.bytes().len() == 1 {
                match input.chunk(1, usize::MAX) {
                    Ok([]) => break,
                    Ok(chunk) => {
                        any = true;
                        let delim = options.delimiter.bytes()[0];
                        let end = memchr::memchr(delim, chunk);
                        let consumed = end.map_or(chunk.len(), |i| i + 1);
                        let retained =
                            consumed - usize::from(end.is_some() && (options.trim || delim == 0));
                        if !truncated {
                            let nul = memchr::memchr(0, &chunk[..retained]);
                            bytes.extend_from_slice(&chunk[..nul.unwrap_or(retained)]);
                            truncated = nul.is_some();
                        }
                        if end.is_some() {
                            let unused = chunk.len() - consumed;
                            input.unconsume(unused);
                            break;
                        }
                        continue;
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        diagnostic!("{command}: {e}");
                        return Flow::Continue(1);
                    }
                }
            }
            match input.next() {
                Ok(Some(b)) => {
                    any = true;
                    if options.delimiter.advance(b, &mut delimiter_prefix) {
                        if !truncated {
                            if options.trim {
                                bytes.truncate(bytes.len() - (options.delimiter.bytes().len() - 1));
                            } else if b != 0 {
                                bytes.push(b);
                            }
                        }
                        break;
                    }
                    if b == 0 {
                        truncated = true;
                    } else if !truncated {
                        bytes.push(b);
                    }
                }
                Ok(None) => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    diagnostic!("{command}: {e}");
                    return Flow::Continue(1);
                }
            }
        }
        if !any {
            break;
        }
        if skip > 0 {
            skip -= 1;
            continue;
        }
        let Ok(line) = std::str::from_utf8(&bytes) else {
            diagnostic!("{command}: expected UTF-8 input");
            return Flow::Continue(1);
        };
        let index = origin + count as i64;
        count += 1;
        if let Some(callback) = options.callback
            && count.is_multiple_of(u64::from(options.quantum))
        {
            // A callback can read the same descriptor or replace it.
            drop(input);
            callback_command.clear();
            callback_command.push_str(callback);
            callback_command.push(' ');
            callback_command.push_str(itoa::Buffer::new().format(index));
            callback_command.push(' ');
            crate::word::quote::push_single_quote(&mut callback_command, line);
            match crate::eval_text(session, &callback_command, last) {
                Flow::Continue(code) => last = code,
                flow => return flow,
            }
            if session.readonly(options.name) {
                diagnostic!("{command}: {}: readonly variable", options.name);
                return Flow::Continue(1);
            }
            input = Input::new(fd);
            watch = session.interactive || session.traps.active();
        }
        if let Err(error) = session.assign_input_element(options.name, index, line) {
            diagnostic!("{command}: {error}");
            return Flow::Discard(1);
        }
    }
    Flow::Continue(0)
}
