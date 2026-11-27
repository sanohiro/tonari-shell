use super::array::{Elements, Records};
use super::read::Input;
use super::{Session, is_identifier};
use crate::Flow;
use std::io::{self, BufRead};
use std::os::fd::BorrowedFd;

struct Options<'a> {
    name: &'a str,
    fd: i32,
    origin: Option<u32>,
    skip: u32,
    limit: u32,
    delimiter: u8,
    trim: bool,
    callback: Option<&'a str>,
    quantum: u32,
}

impl<'a> Options<'a> {
    fn parse(args: &'a [String]) -> Result<Self, (u8, &'static str)> {
        let mut result = Self {
            name: "MAPFILE",
            fd: 0,
            origin: None,
            skip: 0,
            limit: 0,
            delimiter: b'\n',
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
                    return Err((2, "invalid option"));
                }
                let value = crate::builtins::flags::value(flags.as_str(), &mut args)
                    .ok_or((2, "option requires an argument"))?;
                match flag {
                    'd' => result.delimiter = value.as_bytes().first().copied().unwrap_or(0),
                    'O' => {
                        result.origin =
                            Some(value.parse().map_err(|_| (1, "invalid array origin"))?)
                    }
                    'n' => result.limit = value.parse().map_err(|_| (1, "invalid line count"))?,
                    's' => result.skip = value.parse().map_err(|_| (1, "invalid line count"))?,
                    'u' => result.fd = value.parse().map_err(|_| (1, "invalid file descriptor"))?,
                    'C' => result.callback = Some(value),
                    'c' => {
                        result.quantum = value
                            .parse()
                            .ok()
                            .filter(|&n| n > 0)
                            .ok_or((1, "invalid callback quantum"))?
                    }
                    _ => unreachable!(),
                }
                break;
            }
        }
        if let Some(name) = args.first() {
            result.name = name;
        }
        if !is_identifier(result.name) {
            return Err((1, "invalid array name"));
        }
        Ok(result)
    }
}

// A full replacement has no callback or later reader that can observe a
// partially filled array. Build dense indices in order with one bulk load.
fn read_all(options: &Options<'_>, here: Option<&[u8]>, store: impl FnMut(&str)) -> io::Result<()> {
    if let Some(mut bytes) = here {
        return records(&mut bytes, options, store);
    }
    let fd = unsafe { BorrowedFd::borrow_raw(options.fd) }.try_clone_to_owned()?;
    let mut reader = io::BufReader::with_capacity(64 * 1024, std::fs::File::from(fd));
    records(&mut reader, options, store)
}

fn records(
    reader: &mut dyn BufRead,
    options: &Options<'_>,
    mut store: impl FnMut(&str),
) -> io::Result<()> {
    let mut pending = Vec::new();
    let mut skip = options.skip;
    let delim = options.delimiter;
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
        store(std::str::from_utf8(bytes).map_err(io::Error::other)?);
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
        for delimiter in memchr::memchr_iter(delim, buf) {
            let record = &buf[start..=delimiter];
            start = delimiter + 1;
            let end = record.len() - usize::from(options.trim || delim == 0);
            if pending.is_empty() {
                append(&record[..end], has_nul)?;
            } else {
                pending.extend_from_slice(&record[..end]);
                append(&pending, true)?;
                pending.clear();
            }
        }
        pending.extend_from_slice(&buf[start..]);
        reader.consume(n);
    }
    Ok(())
}

#[inline(never)]
pub(crate) fn run(
    session: &mut Session,
    command: &str,
    args: &[String],
    last: u8,
    here: Option<(i32, &[u8])>,
) -> Flow {
    let mut options = match Options::parse(args) {
        Ok(options) => options,
        Err((code, message)) => {
            diagnostic!("{command}: {message}");
            return Flow::Continue(code);
        }
    };
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
    options.name = &destination;
    let origin = i64::from(options.origin.unwrap_or(0));
    let integer = session.integer(options.name);
    let letter_case = session
        .variables
        .get(options.name)
        .and_then(|v| v.letter_case);
    if options.limit == 0 && options.callback.is_none() && !integer {
        let result = if options.origin.is_some() {
            let mut var = session
                .declared_variable(options.name, false)
                .unwrap_or_default();
            var.declared = true;
            var.array_assigned = true;
            let elements = var.make_array();
            let mut index = origin;
            let result = read_all(&options, here, |line| {
                elements.insert(index, session.convert_case(line.to_owned(), letter_case));
                index += 1;
            });
            session.variables.insert(options.name.into(), var);
            result
        } else {
            let mut lines = Records::default();
            let result = read_all(&options, here, |line| {
                if letter_case.is_some() {
                    lines.push(&session.convert_case(line.to_owned(), letter_case));
                } else {
                    lines.push(line);
                }
            });
            session.store_elements(options.name, Elements::Records(lines), false);
            result
        };
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
        let mut var = session
            .declared_variable(options.name, false)
            .unwrap_or_default();
        var.make_array();
        var.declared = true;
        var.array_assigned = true;
        session.variables.insert(options.name.into(), var);
    }
    let mut skip = options.skip;
    let mut count = 0u64;
    let mut bytes = Vec::new();
    while options.limit == 0 || count < u64::from(options.limit) {
        bytes.clear();
        let mut any = false;
        let mut truncated = false;
        loop {
            match input.next() {
                Ok(Some(b)) => {
                    any = true;
                    if b == options.delimiter {
                        if !options.trim && b != 0 && !truncated {
                            bytes.push(b);
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
            let callback_command = format!(
                "{callback} {index} {}",
                crate::word::quote::single_quote(line)
            );
            match crate::eval_text(session, &callback_command, last) {
                Flow::Continue(_) => {}
                flow => return flow,
            }
            if session.readonly(options.name) {
                diagnostic!("{command}: {}: readonly variable", options.name);
                return Flow::Continue(1);
            }
            input = Input::new(fd);
        }
        if let Err(error) = session.assign_input_element(options.name, index, line) {
            diagnostic!("{command}: {error}");
            return Flow::Discard(1);
        }
    }
    Flow::Continue(0)
}
