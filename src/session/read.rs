use std::io;
use std::time::{Duration, Instant};

use super::Session;
use crate::parser::is_identifier;

struct Options<'a> {
    raw: bool,
    silent: bool,
    edit: Option<bool>,
    initial: Option<&'a str>,
    delimiter: u8,
    max_chars: Option<usize>,
    exact: bool,
    fd: i32,
    timeout: Option<Duration>,
    prompt: Option<&'a str>,
    names: &'a [String],
    array: Option<&'a str>,
}

fn options(args: &[String]) -> Result<Options<'_>, &'static str> {
    let mut opts = Options {
        raw: false,
        silent: false,
        edit: None,
        initial: None,
        delimiter: b'\n',
        max_chars: None,
        exact: false,
        fd: 0,
        timeout: None,
        prompt: None,
        names: &[],
        array: None,
    };
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        if arg == "--" {
            i += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        i += 1;
        let mut flags = arg[1..].chars();
        while let Some(flag) = flags.next() {
            if flag == 'e' || flag == 'E' {
                opts.edit = Some(flag == 'e');
                continue;
            }
            if flag == 's' {
                opts.silent = true;
                continue;
            }
            if flag == 'r' {
                opts.raw = true;
                continue;
            }
            if !matches!(flag, 'a' | 'd' | 'u' | 'n' | 'N' | 'p' | 't' | 'i') {
                return Err("unsupported option");
            }
            let value = if flags.as_str().is_empty() {
                let word = args.get(i).ok_or("option requires an argument")?;
                i += 1;
                word.as_str()
            } else {
                flags.as_str()
            };
            match flag {
                'a' => {
                    if !is_identifier(value) {
                        return Err("invalid array name");
                    }
                    opts.array = Some(value);
                }
                'd' => opts.delimiter = value.as_bytes().first().copied().unwrap_or(0),
                'n' | 'N' => {
                    opts.max_chars = Some(value.parse().map_err(|_| "invalid character count")?);
                    opts.exact = flag == 'N';
                }
                'u' => {
                    opts.fd = value.parse().map_err(|_| "invalid file descriptor")?;
                    if opts.fd < 0 {
                        return Err("invalid file descriptor");
                    }
                }
                'i' => opts.initial = Some(value),
                'p' => opts.prompt = Some(value),
                't' => {
                    opts.timeout = Some(
                        Duration::try_from_secs_f64(value.parse().map_err(|_| "invalid timeout")?)
                            .map_err(|_| "invalid timeout")?,
                    );
                }
                _ => unreachable!(),
            }
            break;
        }
    }
    opts.names = &args[i..];
    Ok(opts)
}

fn ready(fd: i32, timeout: Duration) -> io::Result<bool> {
    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let millis = timeout.as_millis().saturating_add(u128::from(
        !timeout.subsec_nanos().is_multiple_of(1_000_000),
    ));
    let result = unsafe { libc::poll(&mut descriptor, 1, millis.min(i32::MAX as u128) as i32) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result > 0 && descriptor.revents & libc::POLLNVAL == 0)
    }
}

pub(super) struct Input {
    pub(super) consume_all: bool,
    fd: i32,
    seekable: bool,
    data: [std::mem::MaybeUninit<u8>; 4096],
    position: usize,
    end: usize,
}

impl Input {
    pub(super) fn new(fd: i32) -> Self {
        Self {
            fd,
            consume_all: false,
            seekable: unsafe { libc::lseek(fd, 0, libc::SEEK_CUR) } >= 0,
            data: [std::mem::MaybeUninit::uninit(); 4096],
            position: 0,
            end: 0,
        }
    }

    pub(super) fn next(&mut self) -> io::Result<Option<u8>> {
        Ok(self.chunk(1, 1)?.first().copied())
    }

    fn chunk(&mut self, read_ahead: usize, take: usize) -> io::Result<&[u8]> {
        if self.position == self.end {
            // Reading beyond a delimiter on a pipe would take input from the
            // following shell command, since the kernel cannot rewind its fd.
            let capacity = if self.seekable || self.consume_all {
                self.data.len()
            } else {
                read_ahead.min(self.data.len())
            };
            let n = unsafe { libc::read(self.fd, self.data.as_mut_ptr().cast(), capacity) };
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            self.position = 0;
            self.end = n as usize;
        }
        let start = self.position;
        self.position += take.min(self.end - start);
        // read initialized the prefix ending at self.end.
        Ok(unsafe {
            std::slice::from_raw_parts(self.data.as_ptr().add(start).cast(), self.position - start)
        })
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        if self.seekable && self.end > self.position {
            unsafe {
                libc::lseek(
                    self.fd,
                    -((self.end - self.position) as libc::off_t),
                    libc::SEEK_CUR,
                )
            };
        }
    }
}

struct EchoGuard(i32, libc::termios);

impl EchoGuard {
    fn disable(fd: i32) -> Option<Self> {
        let mut original = std::mem::MaybeUninit::uninit();
        if unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) } != 0 {
            return None;
        }
        let original = unsafe { original.assume_init() };
        let mut modes = original;
        modes.c_lflag &= !(libc::ECHO | libc::ECHONL);
        (unsafe { libc::tcsetattr(fd, libc::TCSANOW, &modes) } == 0).then_some(Self(fd, original))
    }
}

impl Drop for EchoGuard {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.0, libc::TCSANOW, &self.1);
        }
    }
}

pub(super) fn run(session: &mut Session, args: &[String], here: Option<(i32, &[u8])>) -> u8 {
    let opts = match options(args) {
        Ok(opts) => opts,
        Err(e) => {
            eprintln!("tonari: read: {e}");
            return 2;
        }
    };
    if crate::execute::fd::is_private(opts.fd) {
        eprintln!("tonari: read: invalid file descriptor");
        return 1;
    }
    if let Some(invalid) = opts
        .names
        .iter()
        .find(|s| opts.array.is_none() && !session.valid_target(s))
    {
        eprintln!("tonari: read: {invalid}: invalid identifier");
        return 1;
    }
    if opts.timeout == Some(Duration::ZERO) {
        return u8::from(!ready(opts.fd, Duration::ZERO).unwrap_or(false));
    }
    let _echo = if opts.silent {
        EchoGuard::disable(opts.fd)
    } else {
        None
    };
    let mut edited = None;
    let edit = opts.edit.filter(|_| {
        opts.fd == 0
            && !opts.silent
            && opts.max_chars.is_none()
            && opts.delimiter == b'\n'
            && unsafe { libc::isatty(0) } == 1
    });
    if let Some(files_only) = edit {
        let mut history = crate::history::History::default();
        let prior =
            crate::INPUT_DEADLINE.replace(opts.timeout.and_then(|t| Instant::now().checked_add(t)));
        let result = crate::editor::line(
            opts.prompt.unwrap_or(""),
            &mut history,
            session,
            0,
            Some((opts.initial.unwrap_or(""), files_only)),
        );
        crate::INPUT_DEADLINE.set(prior);
        match result {
            Ok(crate::editor::Input::Line(text)) => edited = Some(text.into_bytes().into_iter()),
            Ok(crate::editor::Input::End) => edited = Some(Vec::new().into_iter()),
            Ok(crate::editor::Input::Cancel) => return 130,
            Ok(crate::editor::Input::Exit(code)) => return code,
            Err(e) if e.kind() == io::ErrorKind::TimedOut => return 128 + libc::SIGALRM as u8,
            Err(e) => {
                eprintln!("tonari: read: {e}");
                return 1;
            }
        }
    }
    if edit.is_none()
        && let Some(prompt) = opts.prompt
    {
        if unsafe { libc::isatty(opts.fd) } == 1 {
            eprint!("{prompt}");
        }
    }
    let mut line = Vec::new();
    let mut escaped = Vec::new();
    let mut slash = false;
    let mut chars = 0;
    let started = opts.timeout.map(|timeout| (Instant::now(), timeout));
    let mut input = Input::new(opts.fd);
    // Even an escaped delimiter can only postpone the end of a record. Read
    // through the first possible delimiter in one syscall and leave the rest
    // of the pipe available to subsequent consumers.
    let mut first_read = here
        .filter(|(fd, _)| {
            *fd == opts.fd
                && opts.max_chars.is_none()
                && opts.timeout.is_none()
                && edited.is_none()
                && opts.names.iter().all(|s| is_identifier(s))
        })
        .map_or(0, |(_, data)| {
            memchr::memchr(opts.delimiter, data).map_or(data.len(), |n| n + 1)
        });
    let status = loop {
        if opts.max_chars.is_some_and(|max| chars >= max) && std::str::from_utf8(&line).is_ok() {
            break 0;
        }
        if edited.is_none()
            && let Some((started, timeout)) = started
        {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero()
                || input.position == input.end && !ready(opts.fd, remaining).unwrap_or(false)
            {
                break 128 + libc::SIGALRM as u8;
            }
        }
        let input_byte = if let Some(bytes) = &mut edited {
            Ok(bytes.next())
        } else if opts.raw
            && opts.max_chars.is_none()
            && opts.timeout.is_none()
            && (input.seekable || first_read != 0)
        {
            match input.chunk(std::mem::take(&mut first_read).max(1), usize::MAX) {
                Ok([]) => Ok(None),
                Ok(bytes) => {
                    let end = memchr::memchr(opts.delimiter, bytes);
                    let data = &bytes[..end.unwrap_or(bytes.len())];
                    let unused = bytes.len() - data.len();
                    if memchr::memchr(0, data).is_none() {
                        line.extend_from_slice(data);
                    } else {
                        line.extend(data.iter().copied().filter(|&b| b != 0));
                    }
                    if end.is_some() {
                        input.position -= unused - 1;
                        break 0;
                    }
                    continue;
                }
                Err(e) => Err(e),
            }
        } else if opts.exact && opts.timeout.is_none() {
            // A character consumes at least one byte. This much buffered input
            // is ours even on a pipe with another command waiting after read.
            let remaining = opts.max_chars.unwrap().saturating_sub(chars).max(1);
            let take = if opts.raw { remaining } else { 1 };
            match input.chunk(remaining, take) {
                Ok([]) => Ok(None),
                Ok(bytes) if opts.raw => {
                    if bytes.is_ascii() && memchr::memchr(0, bytes).is_none() {
                        line.extend_from_slice(bytes);
                        chars += bytes.len();
                    } else {
                        for &b in bytes {
                            if b != 0 {
                                line.push(b);
                                chars += usize::from(b & 0xc0 != 0x80);
                            }
                        }
                    }
                    continue;
                }
                Ok(bytes) => Ok(Some(bytes[0])),
                Err(e) => Err(e),
            }
        } else {
            input
                .chunk(std::mem::take(&mut first_read).max(1), 1)
                .map(|s| s.first().copied())
        };
        let b = match input_byte {
            Ok(Some(b)) => b,
            Ok(None) => break 1,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                break 128 + crate::traps::pending().unwrap_or(libc::SIGINT) as u8;
            }
            Err(e) => {
                eprintln!("tonari: read: {e}");
                return 1;
            }
        };
        if slash {
            slash = false;
            if b == b'\n' {
                continue;
            }
            escaped.push(line.len());
        } else if !opts.exact && b == opts.delimiter {
            break 0;
        } else if b == b'\\' && !opts.raw {
            slash = true;
            continue;
        }
        if b == 0 {
            continue;
        }
        chars += usize::from(b & 0xc0 != 0x80);
        line.push(b);
    };
    let Ok(line) = String::from_utf8(line) else {
        eprintln!("tonari: read: expected UTF-8 input");
        return 1;
    };
    if opts.names.is_empty() && opts.array.is_none() {
        if session.readonly("REPLY") {
            eprintln!("tonari: read: REPLY: readonly variable");
            return 1;
        }
        session.assign("REPLY", &line);
        return status;
    }
    let ifs = if opts.exact {
        String::new()
    } else {
        session.get("IFS").unwrap_or_else(|| " \t\n".into())
    };
    if opts.array.is_none()
        && ifs.is_empty()
        && let [name] = opts.names
    {
        if let Err(e) = session.assign_target(name, &line) {
            eprintln!("tonari: read: {e}");
            return 1;
        }
        return status;
    }
    let (spans, end) = field_ranges(&line, &escaped, &ifs);
    if let Some(target) = opts.array {
        if session.is_associative(target) {
            eprintln!("tonari: read: {target}: not an indexed array");
            return 1;
        }
        if session.readonly(target) {
            eprintln!("tonari: read: {target}: readonly variable");
            return 1;
        }
        session.replace_array(
            target,
            spans.into_iter().map(|(s, e)| line[s..e].into()).collect(),
        );
        return status;
    }
    let count = opts.names.len();
    for (i, name) in opts.names.iter().enumerate() {
        let value = spans.get(i).map_or("", |&(start, finish)| {
            let finish = if i + 1 == count && spans.len() > count {
                end
            } else {
                finish
            };
            &line[start..finish]
        });
        if let Err(e) = session.assign_target(name, value) {
            eprintln!("tonari: read: {e}");
            return 1;
        }
    }
    status
}

// Store spans into the original line so the last named variable can retain
// separators between the remaining fields (e.g. "a  b  c").
fn field_ranges(line: &str, escaped: &[usize], ifs: &str) -> (Vec<(usize, usize)>, usize) {
    let mut spans = Vec::new();
    let mut start = 0;
    let mut end = 0;
    let mut after_space = false;
    crate::word::separators(line, ifs, |offset, sep| {
        if escaped.binary_search(&offset).is_ok() {
            return;
        }
        let space = matches!(sep, " " | "\t" | "\n");
        if start < offset {
            spans.push((start, offset));
            end = offset;
            after_space = space;
        } else if !space {
            if !after_space {
                spans.push((offset, offset));
            }
            after_space = false;
        }
        start = offset + sep.len();
        if !space {
            end = start;
        }
    });
    if start < line.len() {
        spans.push((start, line.len()));
        end = line.len();
    }
    (spans, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_retains_the_last_variables_original_separators() {
        let line = "  a   b c  ";
        let (spans, end) = field_ranges(line, &[], " \t\n");
        assert_eq!(&line[spans[1].0..end], "b c");
        let line = "x:y:z:";
        let (spans, end) = field_ranges(line, &[], ":");
        assert_eq!(&line[spans[1].0..end], "y:z:");
        assert_eq!(field_ranges("a b", &[1], " ").0, [(0, 3)]);
    }
}
