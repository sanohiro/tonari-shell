pub(crate) mod escapes;
pub(crate) mod limits;
pub(crate) mod umask;

use std::ffi::CString;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;

use crate::Session;

// Builtins may run in a child which exits with _exit. Flush explicitly before
// descriptors are restored in the parent or closed in the child.
pub fn echo(args: &[String]) -> u8 {
    let mut out = Vec::new();
    echo_into(args, &mut out);
    let mut stdout = io::stdout().lock();
    u8::from(
        stdout
            .write_all(&out)
            .and_then(|()| stdout.flush())
            .is_err(),
    )
}

pub(crate) fn echo_into(args: &[String], out: &mut Vec<u8>) {
    let mut newline = true;
    let mut escapes = false;
    let mut args = args.iter().peekable();
    while let Some(arg) = args.peek() {
        let Some(options) = arg.strip_prefix('-') else {
            break;
        };
        if options.is_empty() || !options.bytes().all(|c| matches!(c, b'n' | b'e' | b'E')) {
            break;
        }
        for flag in options.bytes() {
            match flag {
                b'n' => newline = false,
                b'e' => escapes = true,
                _ => escapes = false,
            }
        }
        args.next();
    }
    for (i, arg) in args.enumerate() {
        if i != 0 {
            out.push(b' ');
        }
        if escapes {
            if !escapes::decode(arg.as_bytes(), out, escapes::Mode::Echo) {
                newline = false;
                break;
            }
        } else {
            out.extend_from_slice(arg.as_bytes());
        }
    }
    if newline {
        out.push(b'\n');
    }
}

pub fn test(words: &[String], session: &mut Session) -> u8 {
    let mut args = &words[1..];
    if words[0] == "[" {
        if args.last().map(String::as_str) != Some("]") {
            eprintln!("tonari: [: missing ]");
            return 2;
        }
        args = &args[..args.len() - 1];
    }
    match test_args(args, session) {
        Ok(value) => u8::from(!value),
        Err(msg) => {
            eprintln!("tonari: {}: {msg}", words[0]);
            2
        }
    }
}

fn test_args(args: &[String], session: &mut Session) -> Result<bool, &'static str> {
    match args {
        [] => return Ok(false),
        [s] => return Ok(!s.is_empty()),
        [op, value] if op == "!" => return Ok(value.is_empty()),
        [op, value] => return unary(op, value, session),
        [left, op, right] if binary_op(op) => return binary(left, op, right),
        [first, rest @ ..] if first == "!" && args.len() <= 4 => {
            return Ok(!test_args(rest, session)?);
        }
        _ => {}
    }
    // test's legacy expression grammar groups parentheses and binds -a more
    // tightly than -o. A word used as a unary operand may itself look like an operator.
    let mut tokens = args;
    let result = disjunction(&mut tokens, session)?;
    if tokens.is_empty() {
        Ok(result)
    } else {
        Err("unexpected argument")
    }
}

fn disjunction(tokens: &mut &[String], session: &mut Session) -> Result<bool, &'static str> {
    let mut result = conjunction(tokens, session)?;
    while tokens.first().is_some_and(|s| s == "-o") {
        *tokens = &tokens[1..];
        result |= conjunction(tokens, session)?;
    }
    Ok(result)
}

fn conjunction(tokens: &mut &[String], session: &mut Session) -> Result<bool, &'static str> {
    let mut result = primary(tokens, session)?;
    while tokens.first().is_some_and(|s| s == "-a") {
        *tokens = &tokens[1..];
        result &= primary(tokens, session)?;
    }
    Ok(result)
}

fn primary(tokens: &mut &[String], session: &mut Session) -> Result<bool, &'static str> {
    let (first, rest) = tokens.split_first().ok_or("argument expected")?;
    *tokens = rest;
    if let [op, right, tail @ ..] = rest {
        if binary_op(op) {
            *tokens = tail;
            return binary(first, op, right);
        }
    }
    if first == "!" {
        return Ok(!primary(tokens, session)?);
    }
    if first == "(" {
        let value = disjunction(tokens, session)?;
        if tokens.first().is_none_or(|v| v != ")") {
            return Err("missing )");
        }
        *tokens = &tokens[1..];
        return Ok(value);
    }
    if first.starts_with('-') && first.len() == 2 {
        if let Some((arg, tail)) = rest.split_first() {
            *tokens = tail;
            return unary(first, arg, session);
        }
    }
    Ok(!first.is_empty())
}

fn binary_op(op: &str) -> bool {
    matches!(
        op,
        "=" | "=="
            | "!="
            | "<"
            | ">"
            | "-eq"
            | "-ne"
            | "-gt"
            | "-ge"
            | "-lt"
            | "-le"
            | "-nt"
            | "-ot"
            | "-ef"
    )
}

pub(crate) fn binary(left: &str, op: &str, right: &str) -> Result<bool, &'static str> {
    Ok(match op {
        "=" | "==" => left == right,
        "!=" => left != right,
        "<" => left < right,
        ">" => left > right,
        "-nt" | "-ot" | "-ef" => {
            let a = fs::metadata(left).ok();
            let b = fs::metadata(right).ok();
            match (op, a, b) {
                ("-nt", Some(a), Some(b)) => a.modified().ok() > b.modified().ok(),
                ("-ot", Some(a), Some(b)) => a.modified().ok() < b.modified().ok(),
                ("-ef", Some(a), Some(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
                ("-nt", Some(_), None) | ("-ot", None, Some(_)) => true,
                _ => false,
            }
        }
        _ => {
            let a = left
                .trim()
                .parse::<i64>()
                .map_err(|_| "integer expression expected")?;
            let b = right
                .trim()
                .parse::<i64>()
                .map_err(|_| "integer expression expected")?;
            match op {
                "-eq" => a == b,
                "-ne" => a != b,
                "-gt" => a > b,
                "-ge" => a >= b,
                "-lt" => a < b,
                _ => a <= b,
            }
        }
    })
}

pub(crate) fn unary(op: &str, arg: &str, session: &mut Session) -> Result<bool, &'static str> {
    match op {
        "-n" => return Ok(!arg.is_empty()),
        "-z" => return Ok(arg.is_empty()),
        "-v" => {
            if let Some((name, expr)) = crate::session::array::subscript(arg)
                && session.is_associative(name)
            {
                let key = session.subscript_key(expr)?;
                return Ok(session.assoc_value(name, &key).is_some());
            }
            return Ok(session.get(arg).is_some());
        }
        "-t" => {
            return Ok(arg
                .parse::<i32>()
                .is_ok_and(|fd| unsafe { libc::isatty(fd) == 1 }));
        }
        "-L" | "-h" => return Ok(Path::new(arg).is_symlink()),
        "-r" | "-w" | "-x" => {
            let path = CString::new(arg).map_err(|_| "invalid path")?;
            let mode = match op {
                "-r" => libc::R_OK,
                "-w" => libc::W_OK,
                _ => libc::X_OK,
            };
            return Ok(unsafe {
                libc::faccessat(libc::AT_FDCWD, path.as_ptr(), mode, libc::AT_EACCESS) == 0
            });
        }
        "-a" | "-e" | "-d" | "-f" | "-s" | "-b" | "-c" | "-p" | "-S" | "-O" | "-G" | "-u"
        | "-g" | "-k" => {}
        _ => return Err("unknown unary operator"),
    }
    let Ok(meta) = fs::metadata(arg) else {
        return Ok(false);
    };
    Ok(match op {
        "-a" | "-e" => true,
        "-d" => meta.is_dir(),
        "-f" => meta.is_file(),
        "-s" => meta.len() != 0,
        "-b" => meta.file_type().is_block_device(),
        "-c" => meta.file_type().is_char_device(),
        "-p" => meta.file_type().is_fifo(),
        "-S" => meta.file_type().is_socket(),
        "-O" => meta.uid() == unsafe { libc::geteuid() },
        "-G" => meta.gid() == unsafe { libc::getegid() },
        "-u" => meta.mode() & 0o4000 != 0,
        "-g" => meta.mode() & 0o2000 != 0,
        _ => meta.mode() & 0o1000 != 0,
    })
}

pub fn times(args: &[String]) -> u8 {
    if let Some(flag) = args
        .first()
        .filter(|s| s.starts_with('-') && s.len() > 1 && *s != "--")
    {
        eprintln!("tonari: times: {flag}: invalid option");
        return 2;
    }
    let mut out = io::stdout().lock();
    for who in [libc::RUSAGE_SELF, libc::RUSAGE_CHILDREN] {
        let mut usage = std::mem::MaybeUninit::uninit();
        if unsafe { libc::getrusage(who, usage.as_mut_ptr()) } == -1 {
            return 1;
        }
        let usage = unsafe { usage.assume_init() };
        for (time, end) in [(usage.ru_utime, ' '), (usage.ru_stime, '\n')] {
            let millis = i128::from(time.tv_sec) * 1000 + (i128::from(time.tv_usec) + 500) / 1000;
            if write!(
                out,
                "{}m{}.{:03}s{end}",
                millis / 60_000,
                millis / 1000 % 60,
                millis % 1000
            )
            .is_err()
            {
                return 1;
            }
        }
    }
    u8::from(out.flush().is_err())
}
