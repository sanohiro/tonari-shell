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
        let mut args = args.iter().peekable();
        while let Some(arg) = args.next_if(|arg| arg.starts_with('-') && arg.len() > 1) {
            if arg == "--" {
                break;
            }
            let mut flags = arg[1..].chars();
            while let Some(flag) = flags.next() {
                if flag == 't' {
                    result.trim = true;
                    continue;
                }
                if !"dOnsucC".contains(flag) {
                    return Err((2, "invalid option"));
                }
                let value = if flags.as_str().is_empty() {
                    args.next()
                        .ok_or((2, "option requires an argument"))?
                        .as_str()
                } else {
                    flags.as_str()
                };
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
        if let Some(name) = args.next() {
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
fn read_all(options: &Options<'_>) -> io::Result<Vec<String>> {
    let fd = unsafe { BorrowedFd::borrow_raw(options.fd) }.try_clone_to_owned()?;
    let mut reader = io::BufReader::new(std::fs::File::from(fd));
    let mut lines = Vec::new();
    let mut pending = Vec::new();
    let mut skip = options.skip;
    let delim = options.delimiter;
    let mut append = |bytes: &[u8], has_nul: bool| -> io::Result<()> {
        if skip > 0 {
            skip -= 1;
            return Ok(());
        }
        let bytes = if has_nul {
            &bytes[..bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len())]
        } else {
            bytes
        };
        lines.push(
            std::str::from_utf8(bytes)
                .map_err(io::Error::other)?
                .to_owned(),
        );
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
        let has_nul = delim != 0 && buf.contains(&0);
        // Only the unfinished record at the end of a buffer spans two reads.
        for record in buf.split_inclusive(|&b| b == delim) {
            if record.last() != Some(&delim) {
                pending.extend_from_slice(record);
                continue;
            }
            let end = record.len() - usize::from(options.trim || delim == 0);
            if pending.is_empty() {
                append(&record[..end], has_nul)?;
            } else {
                pending.extend_from_slice(&record[..end]);
                append(&pending, true)?;
                pending.clear();
            }
        }
        reader.consume(n);
    }
    Ok(lines)
}

pub(crate) fn run(session: &mut Session, command: &str, args: &[String], last: u8) -> Flow {
    let options = match Options::parse(args) {
        Ok(options) => options,
        Err((code, message)) => {
            eprintln!("tonari: {command}: {message}");
            return Flow::Continue(code);
        }
    };
    if session.readonly(options.name) {
        eprintln!("tonari: {command}: {}: readonly variable", options.name);
        return Flow::Continue(1);
    }
    if session.is_associative(options.name) {
        eprintln!("tonari: {command}: {}: not an indexed array", options.name);
        return Flow::Continue(1);
    }
    let fd = options.fd;
    if crate::execute::fd::is_private(fd) || unsafe { libc::fcntl(fd, libc::F_GETFD) } == -1 {
        eprintln!("tonari: {command}: {fd}: invalid file descriptor");
        return Flow::Continue(1);
    }
    if options.limit == 0 && options.callback.is_none() && options.origin.is_none() {
        return match read_all(&options) {
            Ok(lines) => {
                session.replace_array(options.name, lines);
                Flow::Continue(0)
            }
            Err(e) => {
                eprintln!("tonari: {command}: {e}");
                Flow::Continue(1)
            }
        };
    }
    let mut input = Input::new(fd);
    input.consume_all = options.callback.is_none() && options.limit == 0;
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
    let origin = i64::from(options.origin.unwrap_or(0));
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
                    eprintln!("tonari: {command}: {e}");
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
            eprintln!("tonari: {command}: expected UTF-8 input");
            return Flow::Continue(1);
        };
        let index = origin + count as i64;
        count += 1;
        if let Some(callback) = options.callback
            && count.is_multiple_of(u64::from(options.quantum))
        {
            // A callback can read the same descriptor or replace it.
            drop(input);
            let command = format!(
                "{callback} {index} {}",
                crate::completion_spec::single_quote(line)
            );
            match crate::eval_text(session, &command, last) {
                Flow::Continue(_) => {}
                flow => return flow,
            }
            if session.readonly(options.name) {
                eprintln!("tonari: {command}: {}: readonly variable", options.name);
                return Flow::Continue(1);
            }
            input = Input::new(fd);
        }
        session.assign_element(options.name, index, line.into());
    }
    Flow::Continue(0)
}
