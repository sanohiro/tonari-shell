use std::io;
use std::time::{Duration, Instant};

use super::Session;
use crate::Flow;
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

fn options(args: &[String]) -> Result<Options<'_>, (u8, &'static str)> {
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
    let mut args = args;
    while let Some(arg) = crate::builtins::flags::next(&mut args) {
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
                return Err((2, "unsupported option"));
            }
            let value = crate::builtins::flags::value(flags.as_str(), &mut args)
                .ok_or((2, "option requires an argument"))?;
            match flag {
                'a' => {
                    if !is_identifier(value) {
                        return Err((1, "invalid array name"));
                    }
                    opts.array = Some(value);
                }
                'd' => opts.delimiter = value.as_bytes().first().copied().unwrap_or(0),
                'n' | 'N' => {
                    opts.max_chars =
                        Some(value.parse().map_err(|_| (1, "invalid character count"))?);
                    opts.exact = flag == 'N';
                }
                'u' => {
                    opts.fd = value.parse().map_err(|_| (1, "invalid file descriptor"))?;
                    if opts.fd < 0 {
                        return Err((1, "invalid file descriptor"));
                    }
                }
                'i' => opts.initial = Some(value),
                'p' => opts.prompt = Some(value),
                't' => {
                    opts.timeout = Some(timeout(value).ok_or((1, "invalid timeout"))?);
                }
                _ => unreachable!(),
            }
            break;
        }
    }
    opts.names = args;
    Ok(opts)
}

// The process numeric locale remains C. Shell LC_NUMERIC is applied explicitly
// by printf. Keep decimal/exponent syntax and the Duration range check.
fn timeout(value: &str) -> Option<Duration> {
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || b"+-.eE".contains(&b))
    {
        return None;
    }
    let input = std::ffi::CString::new(value).ok()?;
    let mut end = std::ptr::null_mut();
    let seconds = unsafe { libc::strtod(input.as_ptr(), &mut end) };
    if end.cast_const() != unsafe { input.as_ptr().add(value.len()) } {
        return None;
    }
    Duration::try_from_secs_f64(seconds).ok()
}

pub(super) struct Input {
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
            seekable: unsafe { libc::lseek(fd, 0, libc::SEEK_CUR) } >= 0,
            data: [std::mem::MaybeUninit::uninit(); 4096],
            position: 0,
            end: 0,
        }
    }

    // False asks the caller to recompute its deadline and any input lookahead.
    // A handler can replace the descriptor or consume input through another name.
    pub(super) fn wait(
        &mut self,
        session: &mut Session,
        last: u8,
        command: &str,
        remaining: Option<Duration>,
        watch: bool,
    ) -> Result<bool, Flow> {
        // A pipe cannot return bytes already fetched for this read. Consume
        // that prefix before giving a trap access to the underlying fd.
        if watch && (self.seekable || self.position == self.end) && session.traps.read_pending() {
            let fd = self.fd;
            self.rewind();
            let flow = crate::traps::dispatch(session, last);
            if !matches!(flow, Flow::Continue(_)) {
                return Err(flow);
            }
            *self = Self::new(fd);
            return Ok(false);
        }
        if watch
            && crate::INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed)
            && crate::interrupted()
        {
            return Err(Flow::Discard(crate::traps::signal_status(libc::SIGINT)));
        }
        if remaining == Some(Duration::ZERO) {
            return Err(Flow::Continue(crate::traps::signal_status(libc::SIGALRM)));
        }
        if self.position != self.end || remaining.is_none() && self.seekable {
            return Ok(true);
        }
        if watch && remaining.is_none() && !self.seekable {
            // Data already queued requires no signal-mask change or blocking wait.
            // Check the live byte count; other readers can share this descriptor.
            let mut queued: libc::c_int = 0;
            if unsafe { libc::ioctl(self.fd, libc::FIONREAD, &mut queued) } == 0 && queued > 0 {
                return Ok(true);
            }
        }
        match crate::traps::wait_readable(self.fd, remaining, watch.then_some(&session.traps)) {
            Ok(true) => Ok(true),
            Ok(false) => Err(Flow::Continue(crate::traps::signal_status(libc::SIGALRM))),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(false),
            Err(error) => {
                diagnostic!("{command}: {error}");
                Err(Flow::Continue(1))
            }
        }
    }

    pub(super) fn next(&mut self) -> io::Result<Option<u8>> {
        Ok(self.chunk(1, 1)?.first().copied())
    }

    fn chunk(&mut self, read_ahead: usize, take: usize) -> io::Result<&[u8]> {
        if self.position == self.end {
            // Reading beyond a delimiter on a pipe would take input from the
            // following shell command, since the kernel cannot rewind its fd.
            let capacity = if self.seekable {
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

impl Input {
    fn rewind(&mut self) {
        if self.seekable && self.end > self.position {
            unsafe {
                libc::lseek(
                    self.fd,
                    -((self.end - self.position) as libc::off_t),
                    libc::SEEK_CUR,
                )
            };
        }
        self.position = self.end;
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        self.rewind();
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

pub(super) fn run(
    session: &mut Session,
    args: &[String],
    here: Option<(i32, &[u8])>,
    last: u8,
) -> Flow {
    let opts = match options(args) {
        Ok(opts) => opts,
        Err((code, e)) => {
            diagnostic!("read: {e}");
            return Flow::Continue(code);
        }
    };
    if crate::execute::fd::is_private(opts.fd)
        || (opts.max_chars == Some(0) || opts.timeout.is_some())
            && unsafe { libc::fcntl(opts.fd, libc::F_GETFD) } < 0
    {
        diagnostic!("read: {}: invalid file descriptor", opts.fd);
        return Flow::Continue(1);
    }
    if let Some(invalid) = opts
        .names
        .iter()
        .find(|s| opts.array.is_none() && !session.valid_builtin_target(s))
    {
        diagnostic!("read: {invalid}: invalid identifier");
        return Flow::Continue(1);
    }
    if opts.timeout == Some(Duration::ZERO) {
        return match crate::traps::wait_readable(opts.fd, Some(Duration::ZERO), None) {
            Ok(readable) => Flow::Continue(u8::from(!readable)),
            Err(error) => {
                diagnostic!("read: {error}");
                Flow::Continue(1)
            }
        };
    }
    // The normal redirect installed a fresh descriptor. A plain assignment
    // has no callback or subscript that could observe its file position.
    if let Some((source, bytes)) = here
        && source == opts.fd
        && opts.timeout.is_none()
        && opts.edit.is_none()
        && opts.max_chars.is_none()
        && opts.array.is_none()
        && opts.delimiter == b'\n'
        && opts.names.iter().all(|s| session.plain_read_target(s))
        && (!opts.names.is_empty() || session.plain_read_target("REPLY"))
    {
        let end = memchr::memchr(b'\n', bytes).unwrap_or(bytes.len());
        let line = &bytes[..end];
        if (opts.raw || !line.contains(&b'\\'))
            && !line.contains(&0)
            && let Ok(text) = std::str::from_utf8(line)
        {
            return assign_fields(session, &opts, text, &[], u8::from(end == bytes.len()));
        }
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
        let deadline = crate::input_deadline(opts.timeout);
        let result = crate::editor::line(
            opts.prompt.unwrap_or(""),
            &mut history,
            session,
            last,
            crate::editor::Start::Read {
                initial: opts.initial.unwrap_or(""),
                files_only,
            },
            false,
            None,
        );
        drop(deadline);
        match result {
            Ok(crate::editor::Input::Line(text)) => edited = Some(text.into_bytes().into_iter()),
            Ok(crate::editor::Input::Directory(_)) => unreachable!(),
            Ok(crate::editor::Input::End) => edited = Some(Vec::new().into_iter()),
            Ok(crate::editor::Input::Cancel) => return Flow::Discard(130),
            Ok(crate::editor::Input::Control(flow)) => return flow,
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                return Flow::Continue(crate::traps::signal_status(libc::SIGALRM));
            }
            Err(e) => {
                diagnostic!("read: {e}");
                return Flow::Continue(1);
            }
        }
    }
    if edit.is_none()
        && let Some(prompt) = opts.prompt
        && unsafe { libc::isatty(opts.fd) } == 1
    {
        eprint!("{prompt}");
    }
    let mut line = Vec::new();
    let mut escaped = Vec::new();
    let mut slash = false;
    let mut chars = 0;
    let mut validated = 0;
    let started = opts.timeout.map(|timeout| (Instant::now(), timeout));
    let mut input = Input::new(opts.fd);
    // Even an escaped delimiter can only postpone the end of a record. Read
    // in chunks through the first possible delimiter and leave the rest
    // of the pipe available to subsequent consumers.
    let mut known_prefix = here
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
    let mut watch = session.interactive || session.traps.active();
    let status = loop {
        if let Some(limit) = opts.max_chars {
            match std::str::from_utf8(&line[validated..]) {
                Ok(_) if chars >= limit => break 0,
                Ok(_) => validated = line.len(),
                Err(e) if e.error_len().is_none() => validated += e.valid_up_to(),
                Err(_) => break 1,
            }
        }
        let remaining = started.map(|(start, timeout)| timeout.saturating_sub(start.elapsed()));
        if edited.is_none() && (watch || remaining.is_some()) {
            match input.wait(session, last, "read", remaining, watch) {
                Ok(true) => {}
                Ok(false) => {
                    known_prefix = 0;
                    watch = session.interactive || session.traps.active();
                    continue;
                }
                Err(Flow::Continue(code)) if code > 128 => break code,
                Err(flow) => return flow,
            }
        }
        let input_byte = if let Some(bytes) = &mut edited {
            Ok(bytes.next())
        } else if opts.raw
            && opts.max_chars.is_none()
            && opts.timeout.is_none()
            && (input.seekable || known_prefix != 0)
        {
            match input.chunk(known_prefix.max(1), usize::MAX) {
                Ok([]) => Ok(None),
                Ok(bytes) => {
                    known_prefix = known_prefix.saturating_sub(bytes.len());
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
            input.chunk(known_prefix.max(1), 1).map(|s| {
                known_prefix = known_prefix.saturating_sub(s.len());
                s.first().copied()
            })
        };
        let b = match input_byte {
            Ok(Some(b)) => b,
            Ok(None) => break 1,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                diagnostic!("read: {e}");
                return Flow::Continue(1);
            }
        };
        if slash {
            slash = false;
            if b == b'\n' || b == 0 {
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
        diagnostic!("read: expected UTF-8 input");
        return Flow::Continue(1);
    };
    assign_fields(session, &opts, &line, &escaped, status)
}

fn assign_fields(
    session: &mut Session,
    opts: &Options<'_>,
    line: &str,
    escaped: &[usize],
    status: u8,
) -> Flow {
    if opts.names.is_empty() && opts.array.is_none() {
        if session.readonly("REPLY") {
            diagnostic!("read: REPLY: readonly variable");
            return Flow::Continue(2);
        }
        return match session.assign_builtin_target("REPLY", line) {
            Ok(()) => Flow::Continue(status),
            Err(error) => input_assignment_error(session, "REPLY", error, 2),
        };
    }
    let ifs = if opts.exact {
        String::new()
    } else {
        session
            .get("IFS")
            .unwrap_or_else(|| crate::word::DEFAULT_IFS.into())
    };
    if opts.array.is_none()
        && let [name] = opts.names
        && (ifs.is_empty() || escaped.is_empty() && ifs.bytes().all(|b| matches!(b, 9..=13 | b' ')))
    {
        let value = if ifs.is_empty() {
            line
        } else {
            line.trim_matches(|c: char| ifs.contains(c))
        };
        if let Err(e) = session.assign_builtin_target(name, value) {
            return input_assignment_error(session, name, e, 1);
        }
        return Flow::Continue(status);
    }
    let (spans, end) = field_ranges(line, escaped, &ifs);
    if let Some(target) = opts.array {
        let target = match session.indexed_receiver(target, false) {
            Ok(target) => target,
            Err(e) => {
                diagnostic!("read: {target}: {e}");
                return Flow::Continue(1);
            }
        };
        if session.integer(&target) {
            session.replace_array(&target, Vec::new());
            for (index, (s, e)) in spans.into_iter().enumerate() {
                if let Err(error) = session.assign_input_element(&target, index as i64, &line[s..e])
                {
                    diagnostic!("read: {error}");
                    return Flow::Discard(1);
                }
            }
        } else {
            session.replace_array(
                &target,
                spans.into_iter().map(|(s, e)| line[s..e].into()).collect(),
            );
        }
        return Flow::Continue(status);
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
        if let Err(e) = session.assign_builtin_target(name, value) {
            return input_assignment_error(session, name, e, if i + 1 < count { 2 } else { 1 });
        }
    }
    Flow::Continue(status)
}

fn input_assignment_error(
    session: &Session,
    name: &str,
    error: &'static str,
    readonly_status: u8,
) -> Flow {
    diagnostic!("read: {name}: {error}");
    if error == crate::session::READONLY {
        return Flow::Continue(readonly_status);
    }
    if session.reference_name(name).is_ok_and(|target| {
        let base = super::array::subscript(&target).map_or(target.as_ref(), |(base, _)| base);
        session.integer(base)
    }) {
        Flow::Discard(1)
    } else {
        Flow::Continue(1)
    }
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
        let space = matches!(sep, " " | "\t" | "\n" | "\r" | "\x0b" | "\x0c");
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
        let (spans, end) = field_ranges(line, &[], crate::word::DEFAULT_IFS);
        assert_eq!(&line[spans[1].0..end], "b c");
        let line = "x:y:z:";
        let (spans, end) = field_ranges(line, &[], ":");
        assert_eq!(&line[spans[1].0..end], "y:z:");
        assert_eq!(field_ranges("a b", &[1], " ").0, [(0, 3)]);
    }
}
