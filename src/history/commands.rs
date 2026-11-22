use super::{Entries, History, Session};
use crate::Flow;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub(crate) fn run(session: &mut Session, words: &[String], status: u8) -> Flow {
    let args = &words[1..];
    if words[0] == "fc" {
        match fc(session, args, status) {
            Ok(flow) => flow,
            Err(e) => {
                eprintln!("tonari: fc: {e}");
                Flow::Continue(1)
            }
        }
    } else {
        match history(session, args) {
            Ok(()) => Flow::Continue(0),
            Err(e) => {
                eprintln!("tonari: history: {e}");
                Flow::Continue(1)
            }
        }
    }
}

fn error(message: &str) -> io::Error {
    io::Error::other(message)
}

fn history(session: &mut Session, mut args: &[String]) -> io::Result<()> {
    let mut operation = None;
    while let Some((flag, rest)) = args.split_first() {
        if flag == "--" {
            args = rest;
            break;
        }
        if !flag.starts_with('-') || flag == "-" {
            break;
        }
        for c in flag[1..].chars() {
            if c == 'c' {
                session.history = None;
            } else if "dspawrn".contains(c) && operation.is_none() {
                operation = Some(c);
            } else {
                return Err(error("invalid option combination"));
            }
        }
        args = rest;
        if operation.is_some() {
            break;
        }
        if args.is_empty() {
            return Ok(());
        }
    }
    match operation {
        Some('s') => {
            if !args.is_empty() {
                session.history_mut().discard_current();
                let text = args.join(" ");
                session.push_history(&text, false);
                if session.interactive && !session.history_lines().is_empty() {
                    History::persist(session, &text, session.get("PWD").as_deref(), 0);
                }
            }
        }
        Some('d') => {
            let arg = args.first().ok_or_else(|| error("position required"))?;
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
            for i in (first..=last).rev() {
                h.remove(i);
            }
        }
        Some('p') => {
            session.history_mut().discard_current();
            let mut out = io::stdout().lock();
            for arg in args {
                writeln!(out, "{}", super::expand::expand(session, arg)?.text)?;
            }
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
                let mut out = io::stdout().lock();
                for (i, line) in h
                    .lines
                    .iter()
                    .enumerate()
                    .skip(h.lines.len().saturating_sub(count))
                {
                    let stamp = format
                        .as_ref()
                        .map(|f| crate::prompt::format_time(f, h.timestamps[i]))
                        .unwrap_or_default();
                    writeln!(out, "{:5}  {stamp}{line}", h.base + i + 1)?;
                }
            }
        }
    }
    Ok(())
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
        .ok_or_else(|| error("history position out of range"))
}

fn history_file(session: &mut Session, op: char, args: &[String]) -> io::Result<()> {
    if args.len() > 1 {
        return Err(error("too many arguments"));
    }
    let histfile = session.get("HISTFILE");
    let chosen = args.first().map(String::as_str).or(histfile.as_deref());
    let default = session
        .get("HOME")
        .map(|s| Path::new(&s).join(".bash_history"));
    let private =
        args.is_empty() && (chosen.is_none() || default.as_deref() == chosen.map(Path::new));
    if private && matches!(op, 'a' | 'w') {
        return Ok(());
    } // The private journal is written when a command finishes.
    let path = if private {
        super::file(session)
    } else {
        chosen.filter(|s| !s.is_empty()).map(PathBuf::from)
    };
    let Some(path) = path else { return Ok(()) };
    if matches!(op, 'r' | 'n') {
        let file = match File::open(&path) {
            Err(e) if private && e.kind() == io::ErrorKind::NotFound => return Ok(()),
            other => other?,
        };
        let skip = if op == 'n' {
            session
                .history
                .as_deref()
                .and_then(|h| h.imported.as_ref())
                .filter(|(p, _)| p == &path)
                .map_or(0, |(_, n)| *n)
        } else {
            0
        };
        let mut count = 0;
        for line in BufReader::new(file).lines() {
            let line = line?;
            count += 1;
            if count <= skip {
                continue;
            }
            let value = if private {
                let mut fields = line.splitn(5, '\t');
                if fields.next() != Some("1") {
                    continue;
                }
                let pid = fields.next().and_then(|s| s.parse::<u32>().ok());
                if op == 'n' && pid == Some(std::process::id()) {
                    continue;
                }
                fields.nth(2).and_then(super::decode)
            } else {
                Some(line)
            };
            if let Some(value) = value {
                session.push_history(&value, false);
            }
        }
        let h = session.history_mut();
        h.imported = Some((path, count));
        h.written = h.lines.len();
    } else {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .append(op == 'a')
            .truncate(false)
            .mode(0o600)
            .open(&path)?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let h = session.history_mut();
        let first = if op == 'a' {
            h.written
        } else {
            if file.metadata()?.is_file() {
                file.set_len(0)?;
            }
            0
        };
        for line in &h.lines[first..] {
            writeln!(file, "{line}")?;
        }
        h.written = h.lines.len();
    }
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
        for flag in arg[1..].chars() {
            match flag {
                'l' => list = true,
                'r' => reverse = true,
                'n' => numbers = false,
                's' => substitute = true,
                'e' => {
                    let (e, rest) = args.split_first().ok_or_else(|| error("editor required"))?;
                    editor = Some(e.as_str());
                    args = rest;
                }
                _ => return Err(error("invalid option")),
            }
        }
    }
    let substitution = if substitute {
        args.first().and_then(|a| a.split_once('='))
    } else {
        None
    };
    if substitution.is_some() {
        args = &args[1..];
    }
    let h = session
        .history
        .as_deref()
        .ok_or_else(|| error("no command found"))?;
    let end = h.current.unwrap_or(h.lines.len());
    if end == 0 {
        return Err(error("no command found"));
    }
    let select = |arg: &String| {
        if arg.parse::<i64>().is_ok() {
            position(h, arg, end)
        } else {
            h.lines[..end]
                .iter()
                .rposition(|s| s.starts_with(arg))
                .ok_or_else(|| error("no command found"))
        }
    };
    let mut first = args
        .first()
        .map(select)
        .transpose()?
        .unwrap_or_else(|| end.saturating_sub(if list { 16 } else { 1 }));
    let mut last =
        args.get(1)
            .map(select)
            .transpose()?
            .unwrap_or(if list { end - 1 } else { first });
    if first > last {
        std::mem::swap(&mut first, &mut last);
        reverse = true;
    }
    let mut source = String::new();
    for step in 0..=last - first {
        let i = if reverse { last - step } else { first + step };
        if list && numbers {
            source.push_str(&(h.base + i + 1).to_string());
        }
        if list {
            source.push_str("\t ");
        }
        source.push_str(&h.lines[i]);
        source.push('\n');
    }
    if list {
        io::stdout().lock().write_all(source.as_bytes())?;
        return Ok(Flow::Continue(0));
    }
    if let Some((before, after)) = substitution {
        source = source.replace(before, after);
    }
    if !substitute && editor != Some("-") {
        let program = editor
            .map(str::to_owned)
            .or_else(|| session.get("FCEDIT"))
            .or_else(|| session.get("EDITOR"))
            .unwrap_or_else(|| "vi".into());
        let directory = session
            .get("TMPDIR")
            .map_or_else(std::env::temp_dir, PathBuf::from);
        let mut template = directory
            .join("tonari-fc.XXXXXX")
            .as_os_str()
            .as_bytes()
            .to_vec();
        template.push(0);
        let fd = unsafe { libc::mkstemp(template.as_mut_ptr().cast()) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let path = Path::new(std::ffi::OsStr::from_bytes(&template[..template.len() - 1]));
        let edited: io::Result<Result<String, Flow>> = (|| {
            let mut file = unsafe { File::from_raw_fd(fd) };
            file.write_all(source.as_bytes())?;
            drop(file);
            let name = path
                .to_str()
                .ok_or_else(|| error("expected UTF-8 temporary path"))?;
            let flow = crate::eval_text(
                session,
                &format!("{program} '{}'", name.replace('\'', "'\"'\"'")),
                status,
            );
            if !matches!(flow, Flow::Continue(0)) {
                return Ok(Err(flow));
            }
            Ok(Ok(std::fs::read_to_string(path)?))
        })();
        let _ = std::fs::remove_file(path);
        source = match edited? {
            Ok(s) => s,
            Err(f) => return Ok(f),
        };
    }
    session.history_mut().discard_current();
    for line in source.lines() {
        session.push_history(line, false);
    }
    eprint!("{source}");
    Ok(crate::eval_text(session, &source, status))
}
