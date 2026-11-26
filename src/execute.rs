pub(crate) mod coprocess;
pub(crate) mod fd;
pub(crate) mod hash;
mod named_fd;
pub(crate) mod process_substitution;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
mod here_input;
mod lookup;
pub(crate) use lookup::IgnoredExecutables;
mod spawn;
pub(crate) use spawn::inherited_env;

use std::cell::Cell;
use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::process::{Command, Stdio};
use std::rc::Rc;

use crate::expansion::Expansion;
use crate::parser::{Assignment, Pipeline, Redirect, SimpleCommand, Target, Word};
use crate::session::Session;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Flow {
    Continue(u8),
    Discard(u8),
    Exit(u8),
    Return(u8),
    Break(u32, u8),
    LoopContinue(u32),
}

impl Flow {
    pub(crate) fn child_status(self) -> u8 {
        match self {
            Self::Continue(code) | Self::Exit(code) | Self::Return(code) | Self::Discard(code) => {
                code
            }
            Self::Break(_, code) => code,
            Self::LoopContinue(_) => 0,
        }
    }
}

enum Action<'a> {
    Open(RawFd, CString, i32),
    HereInput(RawFd, &'a [u8]),
    Dup(RawFd, RawFd),
    Close(RawFd),
    CloseMoved(RawFd),
}

fn errno() -> i32 {
    io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO)
}

fn write_error(name: &[u8], message: &[u8]) {
    write_error_detail(name, message, b"");
}

#[cold]
fn write_error_detail(name: &[u8], message: &[u8], detail: &[u8]) {
    write_diagnostic(b"tonari: ", name, message, detail);
}

#[cold]
fn write_diagnostic(prefix: &[u8], name: &[u8], message: &[u8], detail: &[u8]) {
    // This can run after fork. Emit the diagnostic in a single write when
    // possible, without buffering, heap allocation or taking a stdio lock.
    let mut buffers = [prefix, name, message, detail, b"\n"].map(|part| libc::iovec {
        iov_base: part.as_ptr().cast_mut().cast(),
        iov_len: part.len(),
    });
    let mut rest = buffers.as_mut_slice();
    while !rest.is_empty() {
        let n = unsafe { libc::writev(2, rest.as_ptr(), rest.len() as libc::c_int) };
        if n < 0 && errno() == libc::EINTR {
            continue;
        }
        if n <= 0 {
            break;
        }
        let mut used = n as usize;
        while let Some(first) = rest.first_mut() {
            if used < first.iov_len {
                first.iov_base = unsafe { first.iov_base.byte_add(used) };
                first.iov_len -= used;
                break;
            }
            used -= first.iov_len;
            rest = &mut rest[1..];
        }
    }
}

fn exec_status(error: i32) -> u8 {
    if matches!(error, libc::ENOENT | libc::ENOTDIR) {
        127
    } else {
        126
    }
}

fn action(r: &Redirect) -> Action<'_> {
    match r.target {
        Target::CloseMoved => Action::CloseMoved(r.fd),
        Target::Duplicate if r.word == "-" => Action::Close(r.fd),
        Target::Duplicate => Action::Dup(r.word.parse().unwrap(), r.fd),
        Target::HereDoc { .. } | Target::HereString => Action::HereInput(r.fd, r.word.as_bytes()),
        _ => {
            let flags = match r.target {
                Target::Read => libc::O_RDONLY,
                Target::ReadWrite => libc::O_RDWR | libc::O_CREAT,
                Target::NoClobber => libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                Target::Clobber | Target::Write => libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                Target::Append => libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND,
                Target::CloseMoved
                | Target::Duplicate
                | Target::OutputWord
                | Target::HereDoc { .. }
                | Target::HereString => unreachable!(),
            };
            Action::Open(r.fd, CString::new(r.word.as_bytes()).unwrap(), flags)
        }
    }
}

fn actions(cmd: &PreparedCommand) -> Vec<Action<'_>> {
    cmd.redirects.iter().map(action).collect()
}

fn duplicate(src: RawFd, dest: RawFd) -> io::Result<()> {
    loop {
        if unsafe { libc::dup2(src, dest) } >= 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn connect_address(
    family: i32,
    kind: i32,
    protocol: i32,
    addr: *const libc::sockaddr,
    size: libc::socklen_t,
) -> io::Result<OwnedFd> {
    let fd = unsafe { libc::socket(family, kind, protocol) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::connect(fd, addr, size) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(socket)
}

// Resolve services as well as numeric ports, and try every address returned for
// the requested protocol. The open descriptor has the same inheritance as open(2).
fn network_redirect(path: &CStr, address: &[u8], kind: i32) -> Option<OwnedFd> {
    let Some(separator) = address.iter().position(|&b| b == b'/') else {
        write_error(path.to_bytes(), b": expected host/port");
        return None;
    };
    let host = &address[..separator];
    let service_start = path.to_bytes().len() - address.len() + separator + 1;
    let service = CStr::from_bytes_with_nul(&path.to_bytes_with_nul()[service_start..]).ok()?;
    if host.is_empty() || service.to_bytes().is_empty() {
        write_error(path.to_bytes(), b": expected host/port");
        return None;
    }
    // Numeric IPv4 datagrams need no resolver or service database. Other address
    // forms, including scoped IPv6, use the system resolver below.
    if kind == libc::SOCK_DGRAM
        && let Some((ip, port)) = std::str::from_utf8(host)
            .ok()
            .and_then(|s| s.parse::<std::net::Ipv4Addr>().ok())
            .zip(
                service
                    .to_str()
                    .ok()
                    .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
                    .and_then(|s| s.parse::<u16>().ok()),
            )
    {
        let mut dest: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        dest.sin_family = libc::AF_INET as _;
        dest.sin_port = port.to_be();
        dest.sin_addr.s_addr = u32::from_ne_bytes(ip.octets());
        let size = std::mem::size_of_val(&dest) as libc::socklen_t;
        #[cfg(target_os = "macos")]
        {
            dest.sin_len = size as u8;
        }
        return match connect_address(
            libc::AF_INET,
            kind,
            0,
            std::ptr::from_ref(&dest).cast(),
            size,
        ) {
            Ok(fd) => Some(fd),
            Err(error) => {
                write_error_detail(path.to_bytes(), b": ", error.to_string().as_bytes());
                None
            }
        };
    }
    let host = CString::new(host).ok()?;
    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_socktype = kind;
    let mut list = std::ptr::null_mut();
    let result = unsafe { libc::getaddrinfo(host.as_ptr(), service.as_ptr(), &hints, &mut list) };
    if result != 0 {
        write_error_detail(
            path.to_bytes(),
            b": ",
            unsafe { CStr::from_ptr(libc::gai_strerror(result)) }.to_bytes(),
        );
        return None;
    }
    let mut current = list;
    let mut connected = None;
    let mut error = io::Error::from(io::ErrorKind::AddrNotAvailable);
    while let Some(entry) = unsafe { current.as_ref() } {
        match connect_address(
            entry.ai_family,
            kind,
            entry.ai_protocol,
            entry.ai_addr,
            entry.ai_addrlen,
        ) {
            Ok(fd) => {
                connected = Some(fd);
                break;
            }
            Err(e) => error = e,
        }
        current = entry.ai_next;
    }
    unsafe { libc::freeaddrinfo(list) };
    if connected.is_none() {
        write_error_detail(path.to_bytes(), b": ", error.to_string().as_bytes());
    }
    connected
}

fn open_redirect(path: &CStr, flags: i32) -> Option<OwnedFd> {
    if let Some(device) = path.to_bytes().strip_prefix(b"/dev/")
        && let Some((kind, address)) = device
            .strip_prefix(b"tcp/")
            .map(|a| (libc::SOCK_STREAM, a))
            .or_else(|| device.strip_prefix(b"udp/").map(|a| (libc::SOCK_DGRAM, a)))
    {
        return network_redirect(path, address, kind);
    }
    let mut fd = unsafe { libc::open(path.as_ptr(), flags, 0o666 as libc::c_int) };
    if fd < 0 && flags & libc::O_EXCL != 0 && errno() == libc::EEXIST {
        // Open without truncation and inspect the opened object. A path swap
        // cannot turn a permitted device write into truncation of a regular file.
        fd = unsafe { libc::open(path.as_ptr(), libc::O_WRONLY) };
        if fd >= 0 {
            let file = unsafe { std::fs::File::from_raw_fd(fd) };
            if file.metadata().is_ok_and(|m| !m.is_file()) {
                return Some(file.into());
            }
            write_error(path.to_bytes(), b": cannot overwrite existing file");
            return None;
        }
    }
    if fd < 0 {
        let error = errno();
        let mut buffer = [0 as libc::c_char; 128];
        let reason = if unsafe { libc::strerror_r(error, buffer.as_mut_ptr(), buffer.len()) } == 0 {
            unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_bytes()
        } else {
            b"cannot open redirection"
        };
        write_error_detail(path.to_bytes(), b": ", reason);
        return None;
    }
    Some(unsafe { OwnedFd::from_raw_fd(fd) })
}

impl Action<'_> {
    fn destination(&self) -> RawFd {
        match *self {
            Self::Open(fd, ..)
            | Self::HereInput(fd, _)
            | Self::Dup(_, fd)
            | Self::Close(fd)
            | Self::CloseMoved(fd) => fd,
        }
    }

    fn references(&self, fd: RawFd) -> bool {
        self.destination() == fd
            || match self {
                Self::Dup(src, _) => *src == fd,
                Self::Open(_, path, _) => path_fd(path) == Some(fd),
                _ => false,
            }
    }

    fn references_extra(&self) -> bool {
        self.destination() >= 3
            || match self {
                Self::Dup(src, _) => *src >= 3,
                Self::Open(_, path, _) => path_fd(path).is_some_and(|n| n >= 3),
                _ => false,
            }
    }
}

fn path_fd(path: &CStr) -> Option<RawFd> {
    let bytes = path.to_bytes();
    let number = bytes
        .strip_prefix(b"/dev/fd/")
        .or_else(|| bytes.strip_prefix(b"/proc/self/fd/"))?;
    std::str::from_utf8(number).ok()?.parse().ok()
}

fn protect_fds(actions: &[Action<'_>]) -> io::Result<()> {
    if actions.iter().any(Action::references_extra) {
        fd::protect(|fd| actions.iter().any(|a| a.references(fd)))?;
    }
    Ok(())
}

fn apply(actions: &[Action<'_>], pipe_limit: usize) -> bool {
    if let Err(e) = protect_fds(actions) {
        eprintln!("tonari: redirection: {e}");
        return false;
    }
    apply_protected(actions, pipe_limit).is_ok()
}

fn apply_protected(actions: &[Action<'_>], pipe_limit: usize) -> Result<(), usize> {
    for (index, a) in actions.iter().enumerate() {
        match a {
            Action::HereInput(dest, body) => {
                let redirected = here_input::open(body, pipe_limit).and_then(|fd| {
                    if fd.as_raw_fd() != *dest {
                        return duplicate(fd.as_raw_fd(), *dest);
                    }
                    if unsafe { libc::fcntl(*dest, libc::F_SETFD, 0) } < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    let _ = fd.into_raw_fd();
                    Ok(())
                });
                if let Err(error) = redirected {
                    eprintln!("tonari: here-document: {error}");
                    return Err(index);
                }
            }
            Action::Open(dest, path, flags) => {
                let Some(fd) = open_redirect(path, *flags) else {
                    return Err(index);
                };
                if fd.as_raw_fd() == *dest {
                    let _ = fd.into_raw_fd();
                } else if duplicate(fd.as_raw_fd(), *dest).is_err() {
                    write_error(path.as_bytes(), b": cannot redirect");
                    return Err(index);
                }
            }
            Action::Dup(src, dst) => {
                if let Err(error) = duplicate(*src, *dst) {
                    if error.raw_os_error() == Some(libc::EBADF) {
                        write_error(b"redirection", b": Bad file descriptor");
                    } else {
                        write_error(b"redirection", b": cannot duplicate file descriptor");
                    }
                    return Err(index);
                }
            }
            Action::Close(fd) | Action::CloseMoved(fd) => {
                unsafe { libc::close(*fd) };
            }
        }
    }
    Ok(())
}

// Pipe setup uses standard streams as destinations. Reserve distinct source
// numbers even when Tonari starts with a standard stream already closed.
pub(crate) fn private_fd(fd: OwnedFd) -> io::Result<OwnedFd> {
    if fd.as_raw_fd() > 2 {
        return Ok(fd);
    }
    let copy = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
    if copy < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(copy) })
}

fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let (read, write) = io::pipe()?;
    #[cfg(target_os = "linux")]
    {
        const TARGET: libc::c_int = 128 * 1024;
        let fd = read.as_raw_fd();
        let capacity = unsafe { libc::fcntl(fd, libc::F_GETPIPE_SZ) };
        // Preserve larger defaults and the reduced capacity used under quota pressure.
        if (64 * 1024..TARGET).contains(&capacity) {
            unsafe { libc::fcntl(fd, libc::F_SETPIPE_SZ, TARGET) };
        }
    }
    Ok((private_fd(read.into())?, private_fd(write.into())?))
}

pub(crate) struct Environment {
    // Commands without environment changes use execv and inherit environ.
    _strings: Vec<CString>,
    pointers: Vec<*const libc::c_char>,
    changes: Vec<(String, Option<String>)>,
}

impl Environment {
    fn prepare(session: &Session, assignments: &[(String, String)]) -> Option<Rc<Self>> {
        session.environment_changes(assignments).next()?;
        if assignments.is_empty() {
            if let Some(cached) = &*session.external_env.borrow() {
                if session.environment_changes(&[]).eq(cached
                    .changes
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_deref().map(std::borrow::Cow::Borrowed))))
                {
                    return Some(cached.clone());
                }
            }
        }
        let changes: std::collections::BTreeMap<_, _> =
            session.environment_changes(assignments).collect();
        // Tonari keeps its variable changes in Session. The original process environment
        // remains allocated and immutable for the lifetime of these borrowed C strings.
        let mut pointers = Vec::new();
        let mut inherited = inherited_env();
        unsafe {
            while !(*inherited).is_null() {
                let entry = *inherited;
                let bytes = CStr::from_ptr(entry).to_bytes();
                let key = bytes.split(|&b| b == b'=').next().unwrap_or_default();
                if !std::str::from_utf8(key).is_ok_and(|name| changes.contains_key(name)) {
                    pointers.push(entry.cast_const());
                }
                inherited = inherited.add(1);
            }
        }
        let mut strings = Vec::with_capacity(changes.len());
        for (name, value) in &changes {
            if let Some(value) = value {
                let mut entry = Vec::with_capacity(name.len() + value.len() + 2);
                entry.extend_from_slice(name.as_bytes());
                entry.push(b'=');
                entry.extend_from_slice(value.as_bytes());
                let entry = CString::new(entry).expect("validated environment value");
                pointers.push(entry.as_ptr());
                strings.push(entry);
            }
        }
        pointers.push(std::ptr::null());
        let cached_changes = if assignments.is_empty() {
            session
                .environment_changes(&[])
                .map(|(k, v)| (k.to_owned(), v.map(std::borrow::Cow::into_owned)))
                .collect()
        } else {
            Vec::new()
        };
        let env = Rc::new(Self {
            _strings: strings,
            pointers,
            changes: cached_changes,
        });
        if assignments.is_empty() {
            *session.external_env.borrow_mut() = Some(env.clone());
        }
        Some(env)
    }
}

struct External {
    // Owns the argument strings referenced by argv.
    diagnostic: Option<String>,
    original_name: Option<CString>,
    words: Vec<CString>,
    argv: Vec<*const libc::c_char>,
    search_paths: Option<Vec<CString>>,
    env: Option<Rc<Environment>>,
}

impl External {
    fn new(cmd: &PreparedCommand, session: &Session) -> Self {
        Self::for_words(cmd, session, &cmd.words, None, false)
    }
    fn for_words(
        cmd: &PreparedCommand,
        session: &Session,
        arguments: &[String],
        arg0: Option<String>,
        clear_env: bool,
    ) -> Self {
        let mut words: Vec<_> = arguments
            .iter()
            .map(|w| CString::new(w.as_bytes()).unwrap())
            .collect();
        let original = arg0.as_ref().map(|_| words[0].clone());
        if let Some(arg0) = arg0 {
            words[0] = CString::new(arg0).unwrap();
        }
        let executable = original.as_ref().unwrap_or(&words[0]);
        let mut argv = Vec::with_capacity(words.len() + 1);
        argv.extend(words.iter().map(|w| w.as_ptr()));
        argv.push(std::ptr::null());
        let name = std::ffi::OsStr::from_bytes(executable.as_bytes());
        let cached = cmd.cached_path(session, &arguments[0]);
        let search_paths = if let Some(path) = cached {
            Some(vec![CString::new(path.as_os_str().as_bytes()).unwrap()])
        } else {
            (!executable.as_bytes().contains(&b'/')).then(|| {
                let path = lookup::path(cmd, session);
                let ignored = IgnoredExecutables::new(session);
                std::env::split_paths(&path)
                    .map(|dir| {
                        if dir.as_os_str().is_empty() {
                            Path::new(".").join(name)
                        } else {
                            dir.join(name)
                        }
                    })
                    .filter(|path| !ignored.contains(path))
                    .map(|path| CString::new(path.as_os_str().as_bytes()).unwrap())
                    .collect()
            })
        };
        Self {
            words,
            argv,
            diagnostic: session.diagnostic_prefix(|| session.line()),
            original_name: original,
            search_paths,
            env: if clear_env {
                Some(Rc::new(Environment {
                    changes: Vec::new(),
                    _strings: Vec::new(),
                    pointers: vec![std::ptr::null()],
                }))
            } else {
                Environment::prepare(session, &cmd.assignments)
            },
        }
    }

    fn with_script<T>(
        &self,
        path: &CStr,
        launch: impl FnOnce(&CStr, &[*const libc::c_char]) -> io::Result<T>,
    ) -> io::Result<T> {
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;
        let path_os = std::ffi::OsStr::from_bytes(path.to_bytes());
        // The executable can be replaced between ENOEXEC and this open.
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path_os)?;
        let mut sample = [0u8; 128];
        let length = file.read(&mut sample)?;
        if !file.metadata()?.is_file() || sample[..length].contains(&0) {
            return Err(io::Error::from_raw_os_error(libc::ENOEXEC));
        }
        drop(file);
        let shell = CString::new(std::env::current_exe()?.as_os_str().as_bytes())?;
        let mut args = Vec::with_capacity(self.argv.len() + 2);
        args.extend([shell.as_ptr(), c"--".as_ptr(), path.as_ptr()]);
        args.extend_from_slice(&self.argv[1..]);
        launch(&shell, &args)
    }

    fn script_fallback(&self, path: &CStr, session: Option<&Session>) -> i32 {
        self.with_script(path, |shell, args| {
            let env = self.env.as_ref().map_or_else(
                || inherited_env().cast::<*const libc::c_char>(),
                |e| e.pointers.as_ptr(),
            );
            if let Some(session) = session {
                self.start_script(
                    session,
                    path,
                    shell,
                    args,
                    env,
                    spawn::ScriptProcess::Replace,
                );
            }
            unsafe { libc::execve(shell.as_ptr(), args.as_ptr(), env) };
            Err::<(), _>(io::Error::last_os_error())
        })
        .unwrap_err()
        .raw_os_error()
        .unwrap_or(libc::ENOEXEC)
    }

    fn exec(&self, session: Option<&Session>) -> u8 {
        let program = self.original_name.as_ref().unwrap_or(&self.words[0]);
        if program.as_bytes().is_empty() {
            write_error(b"", b": command not found");
            return 127;
        }
        let mut error = libc::ENOENT;
        let paths = self
            .search_paths
            .as_deref()
            .unwrap_or(std::slice::from_ref(program));
        for path in paths {
            unsafe {
                if let Some(env) = &self.env {
                    libc::execve(path.as_ptr(), self.argv.as_ptr(), env.pointers.as_ptr());
                } else {
                    libc::execv(path.as_ptr(), self.argv.as_ptr());
                }
            }
            match errno() {
                libc::ENOENT | libc::ENOTDIR => continue,
                libc::ENOEXEC => {
                    error = self.script_fallback(path, session);
                    break;
                }
                libc::EACCES => error = libc::EACCES,
                e => {
                    error = e;
                    break;
                }
            }
        }
        let status = exec_status(error);
        write_diagnostic(
            self.diagnostic.as_deref().unwrap_or("tonari: ").as_bytes(),
            program.as_bytes(),
            if status == 127 {
                b": command not found"
            } else {
                b": cannot execute"
            },
            b"",
        );
        status
    }
}

#[derive(Clone, Copy)]
enum Builtin {
    Status(u8),
    Exit(u8),
    InvalidExit,
    Control,
    NotLogin,
    History,
    ExtraExitArgs,
    Return(Option<u8>),
    Export,
    Unset,
    Directory,
    Source,
    Eval,
    Function,
    Compound,
    Exec,
    Declare,
    Set,
    Shopt,
    Hash,
    Shift,
    Getopts,
    DirectoryStack,
    Umask,
    Ulimit,
    Times,
    Mapfile,
    Let,
    LoopControl,
    AutoCd,
    Echo,
    Test,
    Read,
    Printf,
    Jobs,
    Describe,
    Alias,
    Trap,
    Completion,
    Unknown,
}

pub(crate) fn special_builtin(name: &str) -> bool {
    matches!(
        name,
        ":" | "."
            | "break"
            | "continue"
            | "eval"
            | "exec"
            | "exit"
            | "export"
            | "readonly"
            | "return"
            | "set"
            | "shift"
            | "times"
            | "trap"
            | "unset"
    )
}

fn posix_special(cmd: &PreparedCommand, session: &Session) -> bool {
    session.posix
        && cmd.search == lookup::Search::Normal
        && cmd.words.first().is_some_and(|w| special_builtin(w))
}

fn builtin(cmd: &PreparedCommand, last_status: u8, session: &Session) -> Option<Builtin> {
    if cmd.compound.is_some() {
        return Some(Builtin::Compound);
    }
    let Some(name) = cmd.words.first() else {
        return Some(Builtin::Status(cmd.empty_status));
    };
    if cmd.search == lookup::Search::Normal
        && session.functions.contains_key(name)
        && !(session.posix && (special_builtin(name) || name.contains('/')))
    {
        return Some(Builtin::Function);
    }
    if session.disabled_builtins.contains(name) {
        return (cmd.search == lookup::Search::BuiltinsOnly).then_some(Builtin::Unknown);
    }
    if name == "logout" && !session.login {
        return Some(Builtin::NotLogin);
    }
    named_builtin(name, &cmd.words[1..], last_status)
        .or_else(|| (cmd.search == lookup::Search::BuiltinsOnly).then_some(Builtin::Unknown))
        .or_else(|| {
            (session.shopt.autocd
                && session.interactive
                && cmd.search == lookup::Search::Normal
                && Path::new(name).is_dir()
                && (name.contains('/')
                    || !std::env::split_paths(&lookup::path(cmd, session))
                        .any(|d| lookup::executable(&d.join(name)))))
            .then_some(Builtin::AutoCd)
        })
}

pub(crate) fn builtin_names() -> &'static [&'static str] {
    &[
        ":",
        "[",
        ".",
        "break",
        "bind",
        "caller",
        "enable",
        "help",
        "suspend",
        "builtin",
        "cd",
        "command",
        "compgen",
        "complete",
        "compopt",
        "continue",
        "echo",
        "eval",
        "exit",
        "logout",
        "history",
        "fc",
        "exec",
        "export",
        "false",
        "local",
        "printf",
        "pwd",
        "read",
        "return",
        "set",
        "shift",
        "source",
        "test",
        "true",
        "type",
        "unset",
        "jobs",
        "disown",
        "fg",
        "bg",
        "kill",
        "wait",
        "shopt",
        "declare",
        "readonly",
        "typeset",
        "alias",
        "unalias",
        "trap",
        "getopts",
        "let",
        "hash",
        "umask",
        "ulimit",
        "times",
        "dirs",
        "pushd",
        "popd",
        "mapfile",
        "readarray",
    ]
}

fn status_operands(args: &[String]) -> &[String] {
    if args.first().is_some_and(|s| s == "--") {
        &args[1..]
    } else {
        args
    }
}

fn named_builtin(name: &str, args: &[String], last_status: u8) -> Option<Builtin> {
    match name {
        "bind" | "caller" | "enable" | "help" | "suspend" => Some(Builtin::Control),
        "history" | "fc" => Some(Builtin::History),
        "trap" => Some(Builtin::Trap),
        "alias" | "unalias" => Some(Builtin::Alias),
        "builtin" => Some(Builtin::Status(0)),
        "complete" | "compgen" | "compopt" => Some(Builtin::Completion),
        "command" | "type" => Some(Builtin::Describe),
        ":" | "true" => Some(Builtin::Status(0)),
        "false" => Some(Builtin::Status(1)),
        "printf" => Some(Builtin::Printf),
        "jobs" | "fg" | "bg" | "kill" | "wait" | "disown" => Some(Builtin::Jobs),
        "read" => Some(Builtin::Read),
        "set" => Some(Builtin::Set),
        "hash" => Some(Builtin::Hash),
        "shopt" => Some(Builtin::Shopt),
        "shift" => Some(Builtin::Shift),
        "mapfile" | "readarray" => Some(Builtin::Mapfile),
        "umask" => Some(Builtin::Umask),
        "ulimit" => Some(Builtin::Ulimit),
        "times" => Some(Builtin::Times),
        "getopts" => Some(Builtin::Getopts),
        "let" => Some(Builtin::Let),
        "local" | "declare" | "typeset" | "readonly" => Some(Builtin::Declare),
        "echo" => Some(Builtin::Echo),
        "test" | "[" => Some(Builtin::Test),
        "break" | "continue" => Some(Builtin::LoopControl),
        "export" => Some(Builtin::Export),
        "unset" => Some(Builtin::Unset),
        "dirs" | "pushd" | "popd" => Some(Builtin::DirectoryStack),
        "cd" | "pwd" => Some(Builtin::Directory),
        "source" | "." => Some(Builtin::Source),
        "eval" => Some(Builtin::Eval),
        "exec" => Some(Builtin::Exec),
        "return" => Some(Builtin::Return(match status_operands(args).first() {
            None => Some(last_status),
            Some(s) => s.trim().parse::<i64>().ok().map(|n| n as u8),
        })),
        "exit" | "logout" => {
            let args = status_operands(args);
            let status = match args.first() {
                None => last_status,
                Some(s) => match s.trim().parse::<i64>() {
                    Ok(n) => n as u8,
                    Err(_) => return Some(Builtin::InvalidExit),
                },
            };
            Some(if args.len() > 1 {
                Builtin::ExtraExitArgs
            } else {
                Builtin::Exit(status)
            })
        }
        _ => None,
    }
}

fn eval_builtin(
    builtin: Builtin,
    cmd: &mut PreparedCommand,
    session: &mut Session,
    last_status: u8,
    in_stage_child: bool,
) -> Flow {
    let persistent = !cmd.assignments.is_empty() && posix_special(cmd, session);
    let mut bindings = if persistent {
        session.persist_prefix(&mut cmd.assignments);
        Default::default()
    } else {
        session.bind(&cmd.assignments)
    };
    let args = cmd.words.get(1..).unwrap_or_default();
    let result = match builtin {
        Builtin::History => crate::history::commands::run(session, &cmd.words, last_status),
        Builtin::Control => Flow::Continue(match cmd.words[0].as_str() {
            "bind" => crate::editor::bindings::builtin(session, args),
            "caller" => crate::builtins::control::caller(session, args),
            "enable" => crate::builtins::control::enable(session, args),
            "help" => crate::builtins::control::help(args),
            _ => crate::builtins::control::suspend(session, args),
        }),
        Builtin::NotLogin => {
            write_error(b"logout", b": not login shell: use `exit'");
            Flow::Continue(1)
        }
        Builtin::Compound => match *cmd.compound.take().unwrap() {
            crate::parser::Command::Subshell(body) if in_stage_child => {
                crate::execute_program(session, body, last_status, true)
            }
            command => crate::eval::command(session, command, last_status, false),
        },
        Builtin::Exec => exec_builtin(cmd, session, args),
        Builtin::Function => crate::eval::call_with_bindings(
            session,
            std::mem::take(&mut cmd.words),
            last_status,
            false,
            &mut bindings,
        ),
        Builtin::Trap => Flow::Continue(session.traps.builtin(
            args,
            session.interactive,
            session.posix,
        )),
        Builtin::Alias => Flow::Continue(crate::alias::builtin(&cmd.words[0], args, session)),
        Builtin::Completion => Flow::Continue(crate::completion_spec::run(
            &cmd.words[0],
            args,
            last_status,
            session,
        )),
        Builtin::Describe => Flow::Continue(lookup::describe(cmd, session)),
        Builtin::Unknown => {
            write_error(cmd.words[0].as_bytes(), b": not a shell builtin");
            Flow::Continue(1)
        }
        Builtin::Printf => Flow::Continue(session.printf(args)),
        Builtin::Jobs => crate::jobs::dispatch(&cmd.words, session, last_status),
        Builtin::Times => Flow::Continue(crate::builtins::times(args)),
        Builtin::Ulimit => Flow::Continue(crate::builtins::limits::run(
            args,
            &mut io::stdout().lock(),
            session.posix,
        )),
        Builtin::Umask => Flow::Continue(crate::builtins::umask::run(args, session.posix)),
        Builtin::Mapfile => crate::session::mapfile::run(session, &cmd.words[0], args, last_status),
        Builtin::Read => {
            let here = match cmd.redirects.as_slice() {
                [r] if matches!(r.target, Target::HereDoc { .. } | Target::HereString)
                    && r.word.len() <= here_input::PIPE_LIMIT =>
                {
                    Some((r.fd, r.word.as_bytes()))
                }
                _ => None,
            };
            Flow::Continue(session.read(args, here))
        }
        Builtin::Set => Flow::Continue(session.set(&mut cmd.words)),
        Builtin::Hash => Flow::Continue(hash::run(session, args)),
        Builtin::Shopt => Flow::Continue(session.shopt(args)),
        Builtin::Getopts => Flow::Continue(session.getopts(args)),
        Builtin::Let => Flow::Continue(crate::arithmetic::let_builtin(args, session)),
        Builtin::Shift => Flow::Continue(session.shift(args)),
        Builtin::Declare => session.declare(
            &cmd.words[0],
            args,
            &cmd.array_declarations,
            last_status,
            &mut bindings,
        ),
        Builtin::Echo => Flow::Continue(crate::builtins::echo(args, session)),
        Builtin::Test => Flow::Continue(crate::builtins::test(&cmd.words, session)),
        Builtin::LoopControl => {
            if session.loop_depth == 0 && session.parent_loop_in_subshell {
                write_error(
                    cmd.words[0].as_bytes(),
                    b": loop control cannot cross a subshell boundary",
                );
                Flow::Exit(1)
            } else if session.loop_depth == 0 {
                write_error(cmd.words[0].as_bytes(), b": only meaningful in a loop");
                Flow::Continue(0)
            } else if args.len() > 1 {
                write_error(cmd.words[0].as_bytes(), b": too many arguments");
                // Bash abandons the current input on a usage error. File and
                // stdin readers can then resume at the next complete command.
                if session.command_string && !session.interactive {
                    Flow::Exit(1)
                } else {
                    Flow::Discard(1)
                }
            } else {
                let depth = args.first().map_or(Ok(1), |n| n.trim().parse::<i64>());
                match depth {
                    Ok(n) if n > 0 => {
                        let levels = (n as u64).min(u64::from(session.loop_depth)) as u32;
                        if cmd.words[0] == "break" {
                            Flow::Break(levels, 0)
                        } else {
                            Flow::LoopContinue(levels)
                        }
                    }
                    Err(_) => {
                        write_error(cmd.words[0].as_bytes(), b": numeric argument required");
                        Flow::Exit(128)
                    }
                    Ok(_) => {
                        write_error(cmd.words[0].as_bytes(), b": invalid loop count");
                        Flow::Break(session.loop_depth, 1)
                    }
                }
            }
        }
        Builtin::Export => {
            if args
                .iter()
                .take_while(|s| s.len() > 1 && s.starts_with('-') && *s != "--")
                .any(|s| s.contains(['a', 'A']))
            {
                session.declare(
                    "export",
                    args,
                    &cmd.array_declarations,
                    last_status,
                    &mut bindings,
                )
            } else {
                let status = session.export(args, &mut bindings);
                Flow::Continue(apply_array_declarations(cmd, status, session))
            }
        }
        Builtin::Unset => Flow::Continue(session.unset(args)),
        Builtin::DirectoryStack => {
            Flow::Continue(session.directory_stack(&cmd.words[0], args, &mut io::stdout().lock()))
        }
        Builtin::AutoCd => {
            eprintln!("cd -- {}", cmd.words.join(" "));
            cmd.words.insert(0, "--".into());
            Flow::Continue(session.directory_builtin("cd", &cmd.words))
        }
        Builtin::Directory => Flow::Continue(session.directory_builtin(&cmd.words[0], args)),
        Builtin::Source => crate::source_file(
            session,
            &cmd.words,
            last_status,
            !session.interactive && posix_special(cmd, session),
        ),
        Builtin::Eval if !cmd.assignments.is_empty() => {
            session.enter_scope(&mut bindings, true);
            let flow = crate::input::eval_builtin(session, args, last_status);
            session.leave_scope();
            flow
        }
        Builtin::Eval => crate::input::eval_builtin(session, args, last_status),
        Builtin::Return(status) => {
            if status.is_some() && status_operands(args).len() > 1 {
                write_error(b"return", b": too many arguments");
                Flow::Continue(1)
            } else if session.source_depth == 0 && session.function_depth == 0 {
                write_error(
                    b"return",
                    b": can only return from a function or sourced script",
                );
                Flow::Continue(2)
            } else if let Some(code) = status {
                session.traps.before_return = last_status;
                let code = if status_operands(args).is_empty() {
                    session
                        .traps
                        .implicit_status(session.function_depth + session.source_depth, false)
                        .unwrap_or(code)
                } else {
                    code
                };
                Flow::Return(code)
            } else {
                write_error(b"return", b": numeric argument required");
                Flow::Return(2)
            }
        }
        Builtin::Status(code) => Flow::Continue(code),
        Builtin::Exit(_) if crate::startup::defer_interactive_exit(session) => Flow::Continue(1),
        Builtin::Exit(code) if status_operands(args).is_empty() => {
            Flow::Exit(crate::startup::logout(
                session,
                session
                    .traps
                    .implicit_status(session.function_depth + session.source_depth, true)
                    .unwrap_or(code),
                last_status,
            ))
        }
        Builtin::Exit(code) => Flow::Exit(crate::startup::logout(session, code, last_status)),
        Builtin::InvalidExit => {
            write_error(b"exit", b": numeric argument required");
            Flow::Exit(2)
        }
        Builtin::ExtraExitArgs => {
            write_error(b"exit", b": too many arguments");
            Flow::Continue(1)
        }
    };
    let result = match result {
        Flow::Exit(code) => Flow::Exit(crate::traps::exit_action(session, code)),
        other => other,
    };
    if !persistent {
        session.restore(bindings);
    }
    if let Flow::Continue(status @ 1..) = result
        && !session.interactive
        && posix_special(cmd, session)
        && (matches!(builtin, Builtin::Export | Builtin::Declare | Builtin::Unset)
            || matches!(builtin, Builtin::Trap) && status == 2)
    {
        return Flow::Exit(status);
    }
    result
}

fn exec_builtin(cmd: &PreparedCommand, session: &Session, mut args: &[String]) -> Flow {
    let mut clear = false;
    let mut login = false;
    let mut arg0 = None;
    while let Some((opt, rest)) = args.split_first() {
        if opt == "--" {
            args = rest;
            break;
        }
        if !opt.starts_with('-') || opt == "-" {
            break;
        }
        args = rest;
        let mut flags = opt[1..].chars();
        while let Some(flag) = flags.next() {
            match flag {
                'c' => clear = true,
                'l' => login = true,
                'a' => {
                    let text = flags.as_str();
                    if text.is_empty() {
                        let Some((v, rest)) = args.split_first() else {
                            eprintln!("tonari: exec: -a requires a name");
                            return Flow::Continue(2);
                        };
                        arg0 = Some(v.clone());
                        args = rest;
                    } else {
                        arg0 = Some(text.to_string());
                    }
                    break;
                }
                _ => {
                    eprintln!("tonari: exec: invalid option {opt}");
                    return Flow::Continue(2);
                }
            }
        }
    }
    let Some(program) = args.first() else {
        return Flow::Continue(0);
    };
    if login {
        arg0 = Some(format!("-{}", arg0.as_ref().unwrap_or(program)));
    }
    let external = External::for_words(cmd, session, args, arg0, clear);
    let status = external.exec(None);
    if session.subshell_depth == 0 && (session.interactive || session.shopt.execfail) {
        Flow::Continue(status)
    } else {
        Flow::Exit(status)
    }
}

fn apply_array_declarations(cmd: &PreparedCommand, status: u8, session: &mut Session) -> u8 {
    if status != 0 || cmd.array_declarations.is_empty() {
        return status;
    }
    let global = cmd
        .words
        .iter()
        .skip(1)
        .take_while(|a| a.starts_with('-') && *a != "--")
        .any(|a| a.contains('g'));
    for d in &cmd.array_declarations {
        if let Err(e) = session.assign_array(&d.name, d.append, &d.items, global) {
            eprintln!("tonari: {}: {e}", cmd.words[0]);
            return 1;
        }
    }
    0
}

// The temporary output and the saved stdout are both private descriptors. A
// redirect inside the body can reuse their numbers without breaking restoration.
pub(crate) fn capture_stdout(run: impl FnOnce() -> Flow) -> io::Result<(Flow, Vec<u8>)> {
    use std::io::{Read, Seek, Write};
    use std::os::fd::BorrowedFd;
    io::stdout().flush()?;
    let saved = SavedFd::capture(&Action::Dup(1, 1), &[])?;
    let stream = unsafe { libc::tmpfile() };
    if stream.is_null() {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { fd::PrivateFile::duplicate(BorrowedFd::borrow_raw(libc::fileno(stream))) };
    unsafe {
        libc::fclose(stream);
    }
    let mut file = file?;
    duplicate(file.as_raw_fd(), 1)?;
    let flow = run();
    let flushed = io::stdout().flush();
    saved.restore(1)?;
    flushed?;
    file.rewind()?;
    let mut output = Vec::new();
    file.read_to_end(&mut output)?;
    Ok((flow, output))
}

enum SavedFd {
    Unchanged,
    Closed,
    Open(fd::PrivateFile),
}

impl SavedFd {
    fn capture(action: &Action<'_>, actions: &[Action<'_>]) -> io::Result<Self> {
        let fd = action.destination();
        match fd::duplicate_except(fd, 3, |n| actions.iter().any(|a| a.references(n))) {
            Ok(file) => Ok(Self::Open(fd::PrivateFile::track(file))),
            Err(e) if e.raw_os_error() == Some(libc::EBADF) => Ok(Self::Closed),
            Err(e) => {
                // GNU Bash closes the target of n>&- even when reserving its
                // undo descriptor fails. A failed duplicate keeps both ends.
                if matches!(action, Action::Close(_)) {
                    unsafe { libc::close(fd) };
                }
                Err(e)
            }
        }
    }

    fn restore(self, fd: RawFd) -> io::Result<()> {
        match self {
            Self::Unchanged => {}
            Self::Closed => {
                unsafe { libc::close(fd) };
            }
            Self::Open(file) => duplicate(file.as_raw_fd(), fd)?,
        }
        Ok(())
    }
}

// Only these forms produce bytes without changing shell variables, reading a
// descriptor or issuing diagnostics. Use the usual echo and printf renderers.
enum PlainOutput<'a> {
    Echo(&'a [String]),
    Printf(&'a [String]),
    DirectoryStack(&'a str, &'a [String]),
    Limits(&'a [String]),
}

impl<'a> PlainOutput<'a> {
    fn new(builtin: Builtin, cmd: &'a PreparedCommand, limit: usize) -> Option<Self> {
        if !cmd.assignments.is_empty() {
            return None;
        }
        let words = cmd.words.get(1..)?;
        match builtin {
            Builtin::Echo => {
                let bound = words
                    .iter()
                    .try_fold(1usize, |n, s| n.checked_add(s.len() + 1))?;
                (bound <= limit).then_some(Self::Echo(words))
            }
            Builtin::Printf => {
                let options_end = words.first().is_some_and(|s| s == "--");
                let (format, values) = words[usize::from(options_end)..].split_first()?;
                if !options_end && format.starts_with('-') && format != "-" {
                    return None;
                }
                let bound = format
                    .len()
                    .saturating_mul(values.len().max(1))
                    .saturating_add(values.iter().map(String::len).sum::<usize>());
                if bound > limit {
                    return None;
                }
                let mut rest = format.as_bytes();
                while let Some(i) = memchr::memchr(b'%', rest) {
                    if !matches!(rest.get(i + 1), Some(b'%' | b's'))
                        || i > 0 && rest[i - 1] == b'\\'
                    {
                        return None;
                    }
                    rest = &rest[i + 2..];
                }
                Some(Self::Printf(words))
            }
            _ => None,
        }
    }

    fn for_redirect(builtin: Builtin, cmd: &'a PreparedCommand) -> Option<Self> {
        if !cmd.assignments.is_empty() {
            return None;
        }
        let args = cmd.words.get(1..)?;
        match builtin {
            Builtin::DirectoryStack
                if cmd.words[0] == "dirs" || args.first().is_some_and(|s| s == "-n") =>
            {
                Some(Self::DirectoryStack(&cmd.words[0], args))
            }
            Builtin::Ulimit
                if args.iter().all(|arg| {
                    arg.strip_prefix('-')
                        .is_some_and(|flags| flags.bytes().all(|b| b.is_ascii_alphabetic()))
                }) =>
            {
                Some(Self::Limits(args))
            }
            _ => Self::new(builtin, cmd, usize::MAX),
        }
    }

    fn render(self, session: &mut Session) -> (Vec<u8>, u8) {
        let mut bytes = Vec::new();
        let status = match self {
            Self::Echo(words) => {
                crate::builtins::echo_into(words, &mut bytes, session);
                0
            }
            Self::Printf(words) => crate::session::printf::run(session, words, Some(&mut bytes)),
            Self::DirectoryStack(command, args) => {
                session.directory_stack(command, args, &mut bytes)
            }
            Self::Limits(args) => crate::builtins::limits::run(args, &mut bytes, session.posix),
        };
        (bytes, status)
    }
}

// A direct read knows the first record boundary from its redirect. Shell
// functions, groups and persistent descriptors can have several consumers:
// keep medium inputs seekable so buffered reads can return any unused bytes.
fn here_pipe_limit(builtin: Option<Builtin>, cmd: &PreparedCommand) -> usize {
    if builtin.is_none() || matches!(builtin, Some(Builtin::Read)) && cmd.redirects.len() == 1 {
        here_input::PIPE_LIMIT
    } else {
        libc::PIPE_BUF
    }
}

fn redirect_failure(cmd: &PreparedCommand, session: &Session) -> Flow {
    if !session.interactive && posix_special(cmd, session) {
        Flow::Exit(1)
    } else {
        Flow::Continue(1)
    }
}

fn can_install_trap(builtin: Builtin, cmd: &PreparedCommand) -> bool {
    matches!(
        builtin,
        Builtin::Trap | Builtin::Function | Builtin::Eval | Builtin::Source
    ) || matches!(builtin, Builtin::Compound)
        && !matches!(
            cmd.compound.as_deref(),
            Some(crate::parser::Command::Subshell(_))
        )
}

fn run_builtin(
    builtin: Builtin,
    cmd: &mut PreparedCommand,
    session: &mut Session,
    last_status: u8,
    final_command: bool,
) -> io::Result<Flow> {
    // The substitution child can become the function's last external command.
    // Keep the normal function scope and let each statement recheck active traps.
    if final_command
        && matches!(builtin, Builtin::Function)
        && session.subshell_depth > 0
        && cmd.redirects.is_empty()
        && cmd.assignments.is_empty()
        && session.substitutions.scope() == 0
    {
        return Ok(crate::call_function(
            session,
            std::mem::take(&mut cmd.words),
            last_status,
            true,
        ));
    }
    if cmd.redirects.is_empty() {
        return Ok(eval_builtin(builtin, cmd, session, last_status, false));
    }
    let one;
    let pair;
    let multiple;
    let actions: &[Action] = match cmd.redirects.as_slice() {
        [redirect] => {
            one = [action(redirect)];
            &one
        }
        [first, second] => {
            // A descriptor move expands to a duplicate followed by a close.
            pair = [action(first), action(second)];
            &pair
        }
        _ => {
            multiple = actions(cmd);
            &multiple
        }
    };
    protect_fds(actions)?;
    if !final_command
        && let [Action::Open(1, path, flags)] = actions
        && let Some(output) = PlainOutput::for_redirect(builtin, cmd)
    {
        // Reserve the same descriptor as the ordinary redirect, including
        // failure at RLIMIT_NOFILE. A byte-only builtin can write to the file
        // directly, avoiding two changes to the process-wide stdout.
        let _saved = SavedFd::capture(&actions[0], actions)?;
        let Some(fd) = open_redirect(path, *flags) else {
            return Ok(redirect_failure(cmd, session));
        };
        use std::io::Write;
        let (data, status) = output.render(session);
        let result = std::fs::File::from(fd).write_all(&data);
        return Ok(Flow::Continue(if result.is_err() { 1 } else { status }));
    }
    if let (Builtin::Status(code), [Action::Open(dest, path, flags)]) = (builtin, actions) {
        // A silent builtin needs the open's effects (including a blocking FIFO),
        // but never reads or writes the destination. Open before reserving any
        // backup, so /dev/fd paths see the caller's descriptor table.
        let Some(fd) = open_redirect(path, *flags) else {
            return Ok(redirect_failure(cmd, session));
        };
        // Bash can still fail to save a live destination at its descriptor limit.
        // This reservation drops before the single-threaded shell can exec.
        if fd.as_raw_fd() != *dest {
            let backup = unsafe { libc::fcntl(*dest, libc::F_DUPFD, 3) };
            if backup >= 0 {
                unsafe { libc::close(backup) };
            } else if errno() == libc::EBADF {
                duplicate(fd.as_raw_fd(), *dest)?;
                unsafe { libc::close(*dest) };
            } else {
                return Err(io::Error::last_os_error());
            }
        }
        return Ok(Flow::Continue(code));
    }
    let mut saved = [const { SavedFd::Unchanged }; 3];
    let mut extra = Vec::new();
    if !final_command || can_install_trap(builtin, cmd) {
        for action in actions {
            let fd = action.destination();
            if let Some(slot) = saved.get_mut(fd as usize) {
                if matches!(slot, SavedFd::Unchanged) {
                    *slot = SavedFd::capture(action, actions)?;
                }
            } else if !extra.iter().any(|(n, _)| *n == fd) {
                extra.push((fd, SavedFd::capture(action, actions)?));
            }
        }
    }
    let redirection = apply_protected(actions, here_pipe_limit(Some(builtin), cmd));
    let applied = redirection.is_ok();
    let old_input = session.stdin_redirected;
    if applied && actions.iter().any(|a| a.destination() == 0) {
        session.stdin_redirected = true;
    }
    let flow = if applied {
        eval_builtin(builtin, cmd, session, last_status, false)
    } else {
        redirect_failure(cmd, session)
    };
    if !matches!(builtin, Builtin::Exec) || !matches!(flow, Flow::Continue(0)) {
        session.stdin_redirected = old_input;
    }
    if applied && matches!(builtin, Builtin::Exec) && matches!(flow, Flow::Continue(0)) {
        for r in &cmd.redirects {
            if r.target == Target::Duplicate && r.word.parse::<RawFd>() == Ok(r.fd) {
                continue;
            }
            session.stdin_replaced |= r.fd == 0;
            session.substitutions.replaced(r.fd);
            coprocess::replaced(session, r.fd);
        }
        return Ok(flow);
    }
    let mut restore = Ok(());
    for (fd, original) in saved
        .into_iter()
        .enumerate()
        .map(|(fd, v)| (fd as RawFd, v))
        .chain(extra)
    {
        if let Err(e) = original.restore(fd) {
            restore = Err(e);
        }
    }
    restore?;
    // Failures in a compound command's body are tested by its own statement
    // list. A redirect failure happens before that list can run.
    if let Err(index) = redirection
        && matches!(builtin, Builtin::Compound)
        && session.tested_depth == 0
    {
        if session.traps.active() {
            if let Some(location) = &mut session.current_command {
                location.line = cmd.redirects[index].line;
            }
            let trap = crate::traps::error(session, 1);
            if !matches!(trap, Flow::Continue(_)) {
                return Ok(trap);
            }
        }
        if session.errexit {
            return Ok(Flow::Exit(1));
        }
    }
    Ok(flow)
}

enum Stage {
    Child(libc::pid_t),
    GroupedChild(libc::pid_t),
    Status(u8),
}

impl Stage {
    fn pid(&self) -> Option<libc::pid_t> {
        match *self {
            Self::Child(pid) | Self::GroupedChild(pid) => Some(pid),
            Self::Status(_) => None,
        }
    }

    fn join_group(&self, launch: &mut crate::jobs::Launch) {
        match *self {
            Self::Child(pid) => launch.parent(pid),
            Self::GroupedChild(pid) => launch.record_child(pid),
            Self::Status(_) => {}
        }
    }
}

fn connect_pipe(
    input: Option<&OwnedFd>,
    output: Option<&OwnedFd>,
    next_reader: Option<&OwnedFd>,
) -> Vec<Action<'static>> {
    let mut setup = Vec::new();
    for (fd, dest) in [(input, 0), (output, 1)] {
        if let Some(fd) = fd {
            setup.push(Action::Dup(fd.as_raw_fd(), dest));
            setup.push(Action::Close(fd.as_raw_fd()));
        }
    }
    if let Some(unused) = next_reader {
        setup.push(Action::Close(unused.as_raw_fd()));
    }
    setup
}
pub(crate) fn reset_child_signals() {
    reset_caught_signals(&[libc::SIGINT, libc::SIGQUIT, libc::SIGTERM]);
}

pub(crate) fn reset_caught_signals(signals: &[i32]) {
    for &signal in signals {
        unsafe {
            let mut old: libc::sigaction = std::mem::zeroed();
            libc::sigaction(signal, std::ptr::null(), &mut old);
            if old.sa_sigaction != libc::SIG_IGN && old.sa_sigaction != libc::SIG_DFL {
                libc::signal(signal, libc::SIG_DFL);
            }
        }
    }
}

fn stage_expansion(word: &Word) -> bool {
    use crate::word::Part;
    match word {
        Word::Literal(_) => false,
        Word::Expanded(parts) | Word::Declaration(parts) => parts.iter().any(|part| match part {
            Part::CurrentSubstitution { .. }
            | Part::Substitution { .. }
            | Part::Process { .. }
            | Part::Arithmetic { .. } => true,
            Part::Modify { .. } => true,
            Part::Nested(word) => stage_expansion(word),
            Part::Parameter { name, .. } => name == "BASHPID",
            Part::Alternative { name, op, word, .. } => {
                *op == crate::word::Alternative::Assign
                    || name == "BASHPID"
                    || stage_expansion(word)
            }
            Part::Trim { name, pattern, .. } => name == "BASHPID" || stage_expansion(pattern),
            _ => false,
        }),
    }
}

fn redirection_observable(r: &Redirect<Word>) -> bool {
    if r.variable.is_some() {
        return true;
    }
    stage_expansion(&r.word)
        || matches!(r.target, Target::HereDoc { expand: true })
            && r.word
                .as_literal()
                .is_some_and(|text| text.contains(['$', '`']))
}

// A parenthesized pipeline stage is already the subshell Bash exposes.
fn enter_stage(session: &mut Session, compound: Option<&crate::parser::Command>, background: bool) {
    let explicit = matches!(compound, Some(crate::parser::Command::Subshell(_)));
    session.enter_subshell();
    session.jobs.forked(!explicit && !background);
    if explicit {
        session.command_string = false;
    }
}

// Expansions that can read stdin or observe the stage PID must run after pipe
// installation. In particular, `printf x | echo "$(cat)"` consumes that pipe.
fn start_raw(
    mut raw: SimpleCommand<Word>,
    input: Option<OwnedFd>,
    output: Option<OwnedFd>,
    next_reader: Option<&OwnedFd>,
    last_status: u8,
    session: &mut Session,
    launch: crate::jobs::Launch,
) -> io::Result<Stage> {
    if session.keyword {
        separate_keyword_arguments(&mut raw);
    }
    let observable = needs_ordered_redirects(&raw, session)
        || raw.redirects.iter().any(redirection_observable)
        || raw
            .words
            .iter()
            .chain(raw.redirects.iter().map(|r| &r.word))
            .any(stage_expansion)
        || raw.assignments.iter().any(|(_, value)| match value {
            Assignment::Scalar(word) => stage_expansion(word),
            Assignment::Array(_, items, _) => items.iter().any(|(_, word)| stage_expansion(word)),
        });
    if !observable {
        return match prepare(raw, session, last_status, false) {
            Ok(mut cmd) => start(
                &mut cmd,
                input,
                output,
                next_reader,
                last_status,
                session,
                launch,
            ),
            Err(e) => {
                eprintln!("tonari: {e}");
                Ok(Stage::Status(e.status))
            }
        };
    }
    let setup = connect_pipe(input.as_ref(), output.as_ref(), next_reader);
    if !launch.background {
        session.jobs.refresh_snapshot();
    }
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        session
            .traps
            .after_fork(session.errtrace, session.functrace);
        launch.pipeline_child(&mut session.traps);
        enter_stage(session, raw.compound.as_deref(), launch.background);
        if let Some(command) = raw.compound.as_deref_mut()
            && let crate::parser::Command::Subshell(body) = command
        {
            coprocess::after_fork(session);
            *command = crate::parser::Command::Group(std::mem::take(body));
        }
        session.stdin_redirected |= input.is_some();
        let code = if apply(&setup, libc::PIPE_BUF) {
            match run(vec![raw], last_status, true, session) {
                Ok(flow) => flow.child_status(),
                Err(e) => {
                    eprintln!("tonari: {e}");
                    1
                }
            }
        } else {
            1
        };
        let code = crate::traps::finish(session, code);
        unsafe { libc::_exit(i32::from(code)) }
    }
    Ok(Stage::Child(pid))
}

fn start(
    cmd: &mut PreparedCommand,
    input: Option<OwnedFd>,
    output: Option<OwnedFd>,
    next_reader: Option<&OwnedFd>,
    last_status: u8,
    session: &mut Session,
    launch: crate::jobs::Launch,
) -> io::Result<Stage> {
    let builtin = builtin(cmd, last_status, session).or_else(|| {
        cmd.prepare_missing_hook(session)
            .then_some(Builtin::Function)
    });
    // Numeric exit and constant status builtins do not inspect either pipe end.
    // Keep a real child for job control and observable child-signal traps.
    if !launch.background
        && launch.group.is_none()
        && !session.traps.active()
        && cmd.redirects.is_empty()
        && (input.is_some() || output.is_some())
        && let Some(Builtin::Exit(status) | Builtin::Status(status)) = builtin
    {
        return Ok(Stage::Status(status));
    }
    // Pipeline lookup happens in a stage's environment. A private resolved
    // path permits spawn file actions without populating the parent's hash.
    if builtin.is_none()
        && ((!launch.background && launch.group.is_none() && (input.is_some() || output.is_some()))
            || (launch.background
                && launch.group.is_some()
                && input.is_none()
                && output.is_none()
                && cmd.redirects.is_empty()))
        && cmd.cacheable()
        && !cmd.default_path
        && !cmd.words[0].contains('/')
        && cmd.cached_path(session, &cmd.words[0]).is_none()
        && let Ok(Some(entry)) = hash::search(&cmd.words[0], session)
    {
        cmd.resolved = Some(entry.path);
    }
    let cached = cmd
        .words
        .first()
        .is_some_and(|name| cmd.cached_path(session, name).is_some());
    let custom_search = !cached
        && !cmd.words.first().is_some_and(|s| s.contains('/'))
        && (cmd.default_path || session.overrides_search_path(&cmd.assignments));
    let merged_pipe = output.as_ref().filter(|_| {
        matches!(cmd.redirects.as_slice(), [r] if r.fd == 2 && r.target == Target::Duplicate && r.word == "1")
    });
    if let Some(builtin) = builtin
        && output.is_some()
        && input.is_none()
        && !launch.background
        && launch.group.is_none()
        && !session.traps.active()
        && (cmd.redirects.is_empty() || merged_pipe.is_some())
        && let Some(plain) = PlainOutput::new(builtin, cmd, 512)
    {
        // POSIX guarantees room for at least 512 bytes in this new, empty
        // pipe. We still own its reader, so this write can finish before
        // starting the consumer. Managed jobs and trapped children need a PID.
        use std::io::Write;
        let (bytes, status) = plain.render(session);
        debug_assert!(bytes.len() <= 512);
        let result = std::fs::File::from(output.unwrap()).write_all(&bytes);
        return Ok(Stage::Status(if result.is_err() { 1 } else { status }));
    }
    if builtin.is_none()
        && launch.group.is_none()
        && !launch.background
        && (cached || cmd.words[0].contains('/'))
        && let [r] = cmd.redirects.as_slice()
        && r.fd == 0
        && matches!(r.target, Target::HereDoc { .. } | Target::HereString)
        && r.word.len() <= here_input::PIPE_LIMIT
        && let Ok(reader) =
            here_input::open(r.word.as_bytes(), here_input::PIPE_LIMIT).and_then(private_fd)
        && let Ok(pid) =
            External::new(cmd, session).spawn_with_dups(&[], Some(&reader), output.as_ref())
    {
        return Ok(Stage::Child(pid));
    }
    if builtin.is_none()
        && launch.group.is_none()
        && !launch.background
        && ((!cmd.redirects.is_empty() && merged_pipe.is_none())
            || input.is_some()
            || output.is_some())
        && (cached || cmd.words[0].contains('/'))
        && cmd.redirects.iter().all(spawn::standard_dup)
    {
        // Only descriptor copies can be retried without repeating visible effects.
        // Let the ordinary child apply them to its diagnostic if launch fails.
        let external = External::new(cmd, session);
        if let Ok(pid) = external.spawn_with_dups(&cmd.redirects, input.as_ref(), output.as_ref()) {
            return Ok(Stage::Child(pid));
        }
    }
    let resolved = cached || cmd.words.first().is_some_and(|name| name.contains('/'));
    if builtin.is_none()
        && (launch.group.is_none()
            || ((launch.background || !session.traps.has_explicit_traps())
                && launch.can_spawn_group()))
        && (!launch.background || launch.group.is_some())
        && resolved
        && input.is_none()
        && output.is_none()
        && cmd.redirects.is_empty()
    {
        match External::new(cmd, session).spawn_resolved(
            None,
            (launch.group.is_none() && cmd.assignments.is_empty()).then_some(&*session),
            launch,
        ) {
            Ok(pid) => {
                #[cfg(target_os = "macos")]
                launch.resume_spawned(pid);
                return Ok(if launch.group.is_some() {
                    Stage::GroupedChild(pid)
                } else {
                    Stage::Child(pid)
                });
            }
            Err(_) if launch.group.is_some() => {}
            Err(e) if e.raw_os_error() == Some(libc::ENOEXEC) => {}
            Err(e) => return spawn_failure(&cmd.words[0], e),
        }
    }
    if builtin.is_none()
        && !resolved
        && crate::traps::std_spawn_inherits_pipe(&session.traps)
        && (cmd.redirects.is_empty() || merged_pipe.is_some())
        && !custom_search
        && launch.group.is_none()
        && !launch.background
    {
        // Let the standard library select the platform's process-spawn path.
        let mut external = Command::new(&cmd.words[0]);
        external.arg0(&cmd.words[0]);
        external.args(&cmd.words[1..]);
        for (key, val) in session.environment_changes(&cmd.assignments) {
            if let Some(val) = val {
                external.env(key, val.as_ref());
            } else {
                external.env_remove(key);
            }
        }
        if let Some(writer) = merged_pipe {
            // Retain the original pipe ends until spawn succeeds. On failure
            // the ordinary child must send its diagnostic through the same pipe.
            let mut spawn = || -> io::Result<_> {
                if let Some(reader) = &input {
                    external.stdin(Stdio::from(reader.try_clone()?));
                }
                external.stdout(Stdio::from(writer.try_clone()?));
                external.stderr(Stdio::from(writer.try_clone()?));
                external.spawn()
            };
            if let Ok(child) = spawn() {
                return Ok(Stage::Child(child.id() as libc::pid_t));
            }
        } else {
            if let Some(input) = input {
                external.stdin(Stdio::from(input));
            }
            if let Some(output) = output {
                external.stdout(Stdio::from(output));
            }
            return match external.spawn() {
                Ok(child) => Ok(Stage::Child(child.id() as libc::pid_t)),
                Err(e) => spawn_failure(&cmd.words[0], e),
            };
        }
    }

    // Opening a FIFO can block until a later pipeline stage starts. Do those
    // opens in a child so the parent can launch the remaining stages first.
    let mut setup = connect_pipe(input.as_ref(), output.as_ref(), next_reader);
    // A trap installed by this command runs after its local redirects end.
    // An explicit subshell retains its enclosing redirects through shell exit.
    let scoped = !cmd.redirects.is_empty() && builtin.is_some_and(|b| can_install_trap(b, cmd));
    if !scoped {
        setup.extend(actions(cmd));
    }
    let external = builtin.is_none().then(|| External::new(cmd, session));
    if builtin.is_some() && !launch.background {
        session.jobs.refresh_snapshot();
    }
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        // External argv and environment are prepared before fork. A shell builtin
        // runs in a child copy of the single-threaded interpreter and may reenter it.
        // Restore signals caught by the interactive shell before blocking on a file.
        session
            .traps
            .after_fork(session.errtrace, session.functrace);
        launch.pipeline_child(&mut session.traps);
        enter_stage(session, cmd.compound.as_deref(), launch.background);
        if cmd.missing_hook && !launch.background {
            // The hook has a private execution environment. A normal command
            // search itself does not add a shell-language subshell level.
            session.subshell_depth -= 1;
        }
        session.stdin_redirected |= cmd.redirects.iter().any(|r| r.fd == 0);
        session.stdin_redirected |= input.is_some();
        let code = if !apply(&setup, here_pipe_limit(builtin, cmd)) {
            1
        } else if let Some(external) = external.as_ref() {
            external.exec(None)
        } else {
            coprocess::after_fork(session);
            let flow = if scoped {
                run_builtin(builtin.unwrap(), cmd, session, last_status, false).unwrap_or_else(
                    |e| {
                        eprintln!("tonari: {e}");
                        Flow::Continue(1)
                    },
                )
            } else {
                eval_builtin(builtin.unwrap(), cmd, session, last_status, true)
            };
            flow.child_status()
        };
        let code = crate::traps::finish(session, code);
        unsafe { libc::_exit(code.into()) }
    }
    Ok(Stage::Child(pid))
}

// A single external process supplies both the captured bytes and the exit status.
pub(super) fn can_capture_external(name: &str) -> bool {
    !name.is_empty() && named_builtin(name, &[], 0).is_none()
}

// The builtin's entire output fits in an empty pipe. Keep a pipe on stdin
// even for zero bytes so the consumer observes the same descriptor type.
pub(super) fn capture_with_writer(
    writer: &SimpleCommand<Word>,
    reader: &SimpleCommand<Word>,
    session: &mut Session,
    last: u8,
) -> Option<Result<(String, u8), &'static str>> {
    use std::io::Write;
    let cmd = prepare(writer.clone(), session, last, true).ok()?;
    let output = PlainOutput::new(builtin(&cmd, last, session)?, &cmd, 512)?;
    let (input, write) = pipe().ok()?;
    let (bytes, status) = output.render(session);
    if std::fs::File::from(write).write_all(&bytes).is_err() {
        return Some(Err("cannot write command substitution input"));
    }
    Some(
        capture_external(reader.clone(), session, last, Some(input)).map(|(text, code)| {
            (
                text,
                if code == 0 && session.pipefail {
                    status
                } else {
                    code
                },
            )
        }),
    )
}

pub(super) fn capture_external(
    raw: SimpleCommand<Word>,
    session: &mut Session,
    last: u8,
    input: Option<OwnedFd>,
) -> Result<(String, u8), &'static str> {
    let mut cmd = match prepare(raw, session, last, true) {
        Ok(cmd) => cmd,
        Err(error) => {
            eprintln!("tonari: {error}");
            return Ok((String::new(), error.status));
        }
    };
    // A substitution inherits the parent's hash entries; a fresh lookup must
    // never insert into the parent's hash table or change its hit counts.
    if let Some(name) = cmd.words.first()
        && !name.contains('/')
        && cmd.cacheable()
        && cmd.cached_path(session, name).is_none()
        && let Ok(Some(entry)) = hash::search(name, session)
    {
        cmd.resolved = Some(entry.path);
    }
    let (reader, writer) = pipe().map_err(|_| "cannot open command substitution pipe")?;
    // An external-only capture uses its command process as the substitution
    // scope. The command-not-found function must observe that same level.
    session.subshell_depth += 1;
    let stage = start(
        &mut cmd,
        input,
        Some(writer),
        None,
        last,
        session,
        Default::default(),
    );
    session.subshell_depth -= 1;
    let pid = match stage.map_err(|_| "cannot launch command substitution")? {
        Stage::Child(pid) | Stage::GroupedChild(pid) => pid,
        Stage::Status(code) => return Ok((String::new(), code)),
    };
    crate::subshell::read_output(std::fs::File::from(reader), pid)
}

fn spawn_failure(name: &str, error: io::Error) -> io::Result<Stage> {
    let code = error.raw_os_error().unwrap_or(libc::EIO);
    if matches!(
        code,
        libc::EAGAIN | libc::ENOMEM | libc::EMFILE | libc::ENFILE
    ) {
        return Err(error);
    }
    eprintln!("tonari: {name}: {error}");
    Ok(Stage::Status(exec_status(code)))
}

pub(super) fn wait(pid: libc::pid_t) -> io::Result<u8> {
    let mut status = 0;
    loop {
        if crate::traps::waitpid(pid, &mut status, 0) >= 0 {
            if libc::WIFSIGNALED(status) {
                return Ok((128 + libc::WTERMSIG(status)) as u8);
            }
            return Ok(libc::WEXITSTATUS(status) as u8);
        }
        if errno() != libc::EINTR {
            return Err(io::Error::last_os_error());
        }
    }
}

pub(crate) fn parameter(session: &Session, last_status: u8, name: &str) -> Option<String> {
    match name {
        "?" => Some(last_status.to_string()),
        "$" => Some(session.shell_pid().to_string()),
        "!" => session.last_background.map(|pid| pid.to_string()),
        "-" => Some(session.option_flags()),
        "0" => Some(session.arg0.clone()),
        _ => session.get(name),
    }
}

pub(crate) struct ArrayDeclaration {
    pub operand: usize,
    pub name: String,
    pub append: bool,
    pub items: crate::session::array::ArrayItems,
}

#[derive(Default)]
pub(crate) struct PreparedCommand {
    missing_hook: bool,
    named_redirects: Vec<Redirect<Word>>,
    resolved: Option<PathBuf>,
    array_declarations: Vec<ArrayDeclaration>,
    compound: Option<Box<crate::parser::Command>>,
    search: lookup::Search,
    default_path: bool,
    words: Vec<String>,
    redirects: Vec<Redirect>,
    assignments: Vec<(String, String)>,
    empty_status: u8,
    literal_has_keywords: bool,
}
impl PreparedCommand {
    pub(crate) fn literal_builtin(
        statement: &crate::parser::Statement,
        session: &mut Session,
    ) -> Option<Box<Self>> {
        let crate::parser::Command::Pipeline(stages) = &statement.command else {
            return None;
        };
        let [cmd] = stages.as_slice() else {
            return None;
        };
        if cmd.compound.is_some() || !cmd.assignments.is_empty() {
            return None;
        }
        let name = cmd.words.first()?.as_literal()?;
        if matches!(name, "builtin" | "command" | "set")
            || !(builtin_names().contains(&name)
                || name.starts_with('/') && cmd.redirects.is_empty())
        {
            return None;
        }
        let words = cmd
            .words
            .iter()
            .map(|word| word.as_literal().map(str::to_owned))
            .collect::<Option<Vec<_>>>()?;
        let redirects = if cmd.redirects.is_empty() {
            Vec::new()
        } else {
            // A literal redirect can reuse its parsed path or fd, but its open,
            // duplicate, move or close must still happen on every iteration.
            if session.noclobber
                || cmd.redirects.iter().any(|r| {
                    r.variable.is_some()
                        || r.word.as_literal().is_none()
                        || matches!(r.target, Target::HereDoc { expand: true })
                })
            {
                return None;
            }
            let captured = std::cell::Cell::new(None);
            prepare_redirects(
                cmd.redirects.clone(),
                &mut Expansion::new(session, 0, &captured),
            )
            .ok()?
        };
        Some(Box::new(Self {
            literal_has_keywords: cmd.keyword_args.is_some(),
            words,
            redirects,
            ..Self::default()
        }))
    }

    pub(crate) fn run_literal(&mut self, session: &mut Session, status: u8) -> Option<Flow> {
        if session.keyword && self.literal_has_keywords
            || session.xtrace
            || session.traps.active()
            || session.jobs.monitor
        {
            return None;
        }
        if session.noclobber && self.redirects.iter().any(|r| r.target == Target::Write) {
            return None;
        }
        let selected = builtin(self, status, session);
        if selected.is_none() && self.words[0].starts_with('/') && self.redirects.is_empty() {
            let scope = session.substitutions.scope();
            let result = start(self, None, None, None, status, session, Default::default())
                .and_then(|stage| match stage {
                    Stage::Child(pid) | Stage::GroupedChild(pid) => wait(pid),
                    Stage::Status(code) => Ok(code),
                });
            session.substitutions.finish(scope);
            session.remember_literal_argument(&self.words);
            return Some(Flow::Continue(result.unwrap_or_else(|e| {
                eprintln!("tonari: {e}");
                1
            })));
        }
        let selected = selected?;
        if matches!(selected, Builtin::Function | Builtin::Set) {
            return None;
        }
        let scope = session.substitutions.scope();
        let flow = run_builtin(selected, self, session, status, false).unwrap_or_else(|e| {
            eprintln!("tonari: {e}");
            Flow::Continue(1)
        });
        session.substitutions.finish(scope);
        session.remember_literal_argument(&self.words);
        Some(flow)
    }

    fn cacheable(&self) -> bool {
        !self.default_path && !self.assignments.iter().any(|(k, _)| k == "PATH")
    }
    // Bash invokes this function only for a missing PATH command. A present
    // but non-executable file and an explicit pathname keep their exec errors.
    const MISSING_HOOK: &'static str = "command_not_found_handle";

    fn select_missing_hook(&mut self) {
        self.words.insert(0, Self::MISSING_HOOK.to_owned());
        self.search = lookup::Search::Normal;
        self.missing_hook = true;
    }

    fn prepare_missing_hook(&mut self, session: &Session) -> bool {
        let Some(name) = self.words.first() else {
            return false;
        };
        if !session.functions.contains_key(Self::MISSING_HOOK)
            || name.contains('/')
            || self.cached_path(session, name).is_some()
        {
            return false;
        }
        match hash::search_in(name, session, &lookup::path(self, session)) {
            Err(hash::Absent) => {
                self.select_missing_hook();
                true
            }
            Ok(path) => {
                if self.cacheable() {
                    self.resolved = path.map(|entry| entry.path);
                }
                false
            }
        }
    }

    fn cached_path<'a>(&'a self, session: &'a Session, name: &str) -> Option<&'a Path> {
        if !self.cacheable() || name.contains('/') {
            return None;
        }
        self.resolved.as_deref().or_else(|| {
            session.hashes.get(name).filter(|path| {
                !(session.posix || session.shopt.checkhash) || lookup::executable(path)
            })
        })
    }
}

pub(crate) fn trace_line(session: &mut Session, status: u8, append: impl FnOnce(&mut String)) {
    let prefix = session.get("PS4").unwrap_or_default();
    session.xtrace = false;
    let mut line = crate::prompt::render_text(session, prefix, status);
    session.xtrace = true;
    let levels = session.trace_depth + session.source_depth;
    if levels > 0
        && let Some(first) = line.chars().next()
    {
        line.insert_str(0, &first.to_string().repeat(levels as usize));
    }
    append(&mut line);
    line.push('\n');
    let fd = session
        .scalar_setting("BASH_XTRACEFD")
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    let mut data = line.as_bytes();
    while !data.is_empty() {
        let n = unsafe { libc::write(fd, data.as_ptr().cast(), data.len()) };
        if n > 0 {
            data = &data[n as usize..];
        } else if n == 0 || errno() != libc::EINTR {
            break;
        }
    }
}

pub(crate) fn trace_word(word: &str) -> std::borrow::Cow<'_, str> {
    if word == "'" {
        return "\\'".into();
    }
    if word.bytes().any(|b| b.is_ascii_control()) {
        return String::from_utf8(crate::session::printf::ansi_quote(word.as_bytes(), false))
            .unwrap()
            .into();
    }
    if !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_alphanumeric() || "_+-/:.,=@%".contains(c))
    {
        return word.into();
    }
    format!("'{}'", word.replace('\'', "'\\''")).into()
}

fn prepare(
    mut raw: SimpleCommand<Word>,
    session: &mut Session,
    last_status: u8,
    standalone: bool,
) -> Result<PreparedCommand, crate::expansion::Error> {
    let captured = Cell::new(None);
    let words = expand_command_words(&mut raw, session, last_status, &captured)?;
    prepare_expanded(raw, words, session, last_status, standalone, &captured)
}

// The option is evaluated at execution: a previously parsed function may
// be called both with and without -k. Only syntactic assignments qualify.
fn separate_keyword_arguments(raw: &mut SimpleCommand<Word>) {
    let first_array = raw
        .assignments
        .iter()
        .position(|(_, a)| matches!(a, Assignment::Array(Some(_), ..)));
    if raw.keyword_args.is_none() && first_array.is_none() {
        return;
    }
    let mut arrays = first_array
        .map(|i| raw.assignments.split_off(i))
        .unwrap_or_default()
        .into_iter()
        .peekable();
    let keywords = raw
        .keyword_args
        .take()
        .map(Rc::unwrap_or_clone)
        .unwrap_or_default();
    let mut keywords = keywords.into_iter().peekable();
    raw.assignments.reserve(keywords.len() + arrays.len());
    let mut indices = 0..;
    raw.words.retain_mut(|word| {
        let index = indices.next().unwrap();
        if let Some((_, key)) = keywords.next_if(|(pos, _)| *pos == index) {
            if key.contains('[') {
                eprintln!("tonari: {key}: not a valid identifier");
                return false;
            }
            let mut value = std::mem::replace(word, Word::Literal(String::new()));
            value.remove_assignment_prefix(key.len() + 1);
            raw.assignments.push((key, Assignment::Scalar(value)));
            false
        } else if let Some((key, Assignment::Array(_, parts, source))) =
            arrays.next_if(|(_, a)| matches!(a, Assignment::Array(Some(pos), ..) if *pos == index))
        {
            raw.assignments
                .push((key, Assignment::Array(None, parts, source)));
            false
        } else {
            true
        }
    });
}

fn expand_command_words(
    raw: &mut SimpleCommand<Word>,
    session: &mut Session,
    last_status: u8,
    captured: &Cell<Option<u8>>,
) -> Result<Vec<String>, crate::expansion::Error> {
    let mut words = Vec::with_capacity(raw.words.len());
    if session.posix
        && let Some(operands) = raw.keyword_args.as_ref()
    {
        for (index, _) in operands.iter() {
            if let Word::Expanded(parts) = &mut raw.words[*index] {
                for part in parts {
                    if let crate::word::Part::Tilde(tilde) = part {
                        *part = crate::word::Part::Text(std::mem::take(tilde));
                    }
                }
            }
        }
    }
    let mut original = Expansion::new(session, last_status, captured);
    let mut positions = raw
        .assignments
        .iter_mut()
        .filter_map(|(_, assignment)| match assignment {
            Assignment::Array(Some(position), _, _) => Some(position),
            _ => None,
        })
        .peekable();
    for (index, word) in std::mem::take(&mut raw.words).into_iter().enumerate() {
        if positions.peek().is_some_and(|p| **p == index) {
            *positions.next().unwrap() = words.len();
        }
        match word {
            Word::Literal(word) => words.push(word),
            word => original.append_fields(word, &mut words)?,
        }
    }
    Ok(words)
}

fn prepare_expanded(
    raw: SimpleCommand<Word>,
    words: Vec<String>,
    session: &mut Session,
    last_status: u8,
    standalone: bool,
    captured: &Cell<Option<u8>>,
) -> Result<PreparedCommand, crate::expansion::Error> {
    // A lone assignment has no following word, redirect or trace that needs a copy.
    let direct_assignment = standalone
        && words.is_empty()
        && raw.assignments.len() == 1
        && raw.compound.is_none()
        && raw.redirects.is_empty()
        && !session.xtrace;
    let mut assignments = if direct_assignment {
        Vec::new()
    } else {
        Vec::with_capacity(raw.assignments.len())
    };
    let mut array_declarations = Vec::new();
    let mut traced_operands = Vec::new();
    // Each assignment sees its predecessors, while command arguments saw the
    // outer environment. Standalone assignments persist even on redirect failure.
    for (mut key, word) in raw.assignments {
        let mut exp = Expansion::with_assignments(
            session,
            last_status,
            captured,
            &mut assignments,
            standalone && words.is_empty(),
        );
        let append = key.ends_with('+');
        if append {
            key.pop();
        }
        match word {
            Assignment::Array(operand, parts, source) => {
                if exp.session.xtrace {
                    if let Some(index) = operand {
                        traced_operands.push((index, source));
                    } else {
                        trace_line(exp.session, last_status, |out| out.push_str(source.text()));
                    }
                }
                if operand.is_none() && exp.session.readonly(&key) {
                    return Err(crate::expansion::Error::readonly(&key));
                }
                if operand.is_none() && !words.is_empty() {
                    let mut text = if append {
                        exp.lookup(&key).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    text.push('(');
                    for (i, (index, mut word)) in parts.into_iter().enumerate() {
                        if i != 0 {
                            text.push(' ');
                        }
                        if let Some(index) = index {
                            text.push('[');
                            text.push_str(&exp.key(&index.index)?);
                            text.push_str(if index.append { "]+=" } else { "]=" });
                        }
                        // This command environment entry is a scalar. Preserve
                        // tildes, braces and globs while expanding substitutions.
                        if let Word::Expanded(parts) = &mut word {
                            for part in parts {
                                if let crate::word::Part::Tilde(s) = part {
                                    *part = crate::word::Part::Text(std::mem::take(s));
                                }
                            }
                        }
                        text.push_str(&exp.scalar(word)?);
                    }
                    text.push(')');
                    assignments.push((key, text));
                    continue;
                }
                let declared_assoc = operand.is_some()
                    && words
                        .iter()
                        .skip(1)
                        .take_while(|arg| arg.starts_with('-') && *arg != "--")
                        .any(|arg| arg.contains('A'));
                let items =
                    exp.array_items(parts, declared_assoc || exp.session.is_associative(&key))?;
                if words.is_empty() {
                    if standalone {
                        session
                            .assign_array(&key, append, &items, false)
                            .map_err(crate::expansion::Error::discard_line)?;
                        if let Some(error) = items.initializer_error() {
                            return Err(crate::expansion::Error::discard(error));
                        }
                    }
                } else if let Some(operand) = operand {
                    array_declarations.push(ArrayDeclaration {
                        operand,
                        name: key,
                        append,
                        items,
                    });
                }
            }
            Assignment::Scalar(mut word) => {
                let append =
                    append || direct_assignment && word.take_append_suffix(&key, exp.session);
                let value = exp.scalar(word)?;
                if exp.session.xtrace {
                    trace_line(exp.session, last_status, |out| {
                        out.push_str(&key);
                        out.push_str(if append { "+=" } else { "=" });
                        out.push_str(&trace_word(&value));
                    });
                }
                if let std::borrow::Cow::Owned(resolved) = exp.session.reference_name(&key)? {
                    key = resolved;
                }
                let target = crate::session::array::subscript(&key);
                let base = target.map_or(key.as_str(), |(base, _)| base);
                if exp.session.readonly(base) {
                    let error = crate::expansion::Error::readonly(base);
                    if words.is_empty() || exp.session.posix {
                        return Err(error);
                    }
                    eprintln!("tonari: {error}");
                    continue;
                }
                if target.is_some_and(|(_, index)| index.is_empty()) {
                    return Err(crate::expansion::Error::discard(format!(
                        "{key}: bad array subscript"
                    )));
                }
                if target.is_some() && !words.is_empty() {
                    eprintln!("tonari: {key}: not a valid identifier");
                    continue;
                }
                if let Some((base, subscript)) = target
                    && exp.session.is_associative(base)
                {
                    if standalone {
                        let key = exp.key(subscript)?;
                        let old = append
                            .then(|| exp.session.assoc_value(base, &key).map(str::to_owned))
                            .flatten();
                        let value = if exp.session.integer(base) {
                            exp.session.integer_value(old.as_deref(), &value, append)?
                        } else if append {
                            old.unwrap_or_default() + &value
                        } else {
                            value
                        };
                        exp.session.assign_key(base, key, value)?;
                    }
                    continue;
                }
                let index = if let Some((_, subscript)) = target {
                    if !standalone {
                        continue;
                    }
                    let n = if let Some(n) = Session::plain_decimal(subscript) {
                        n
                    } else {
                        exp.arithmetic(subscript)?
                    };
                    Some(
                        exp.session
                            .relative_index(base, n)
                            .map_err(crate::expansion::Error::discard)?,
                    )
                } else {
                    None
                };
                let old = |exp: &Expansion<'_>| match index {
                    Some(i) => exp.session.element(base, i),
                    None => exp.lookup(&key),
                };
                let value = if exp.session.integer(base) {
                    let old = if append {
                        let old = old(&exp).unwrap_or_default();
                        exp.arithmetic(&old)?
                    } else {
                        0
                    };
                    old.wrapping_add(exp.arithmetic(&value)?).to_string()
                } else if append {
                    if direct_assignment && exp.session.append_plain_scalar(&key, &value) {
                        continue;
                    }
                    old(&exp).unwrap_or_default() + &value
                } else {
                    value
                };
                if let Some(i) = index {
                    session.assign_element(base, i, value);
                    continue;
                }
                if session.is_nameref(&key) && !Session::valid_reference_target(&value) {
                    return Err(crate::expansion::Error::discard("invalid name reference"));
                }
                if direct_assignment {
                    session.assign_value(&key, value.into());
                    continue;
                }
                if standalone && words.is_empty() {
                    session.assign(&key, &value);
                }
                assignments.push((key, value));
            }
        }
    }
    let assignment_only = words.is_empty() && raw.compound.is_none();
    let mut cmd = PreparedCommand {
        missing_hook: false,
        literal_has_keywords: false,
        named_redirects: Vec::new(),
        resolved: None,
        array_declarations,
        compound: raw.compound,
        search: lookup::Search::Normal,
        default_path: false,
        words,
        redirects: Vec::new(),
        assignments,
        empty_status: captured.get().unwrap_or(0),
    };
    if session.xtrace && !cmd.words.is_empty() {
        trace_line(session, last_status, |out| {
            let mut operands = traced_operands.iter().peekable();
            for (i, word) in cmd.words.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                if let Some((_, source)) = operands.next_if(|(index, _)| *index == i) {
                    out.push_str(source.text());
                } else {
                    out.push_str(&trace_word(word));
                }
            }
        });
    }
    lookup::command_prefix(&mut cmd, session);
    if raw.redirects.iter().any(|r| r.variable.is_some()) {
        cmd.named_redirects = raw.redirects;
    } else if !raw.redirects.is_empty() {
        // Bash expands external/redirect-only redirections in a child context.
        // Clone only if an expansion can change a variable in that context.
        let child = assignment_only || builtin(&cmd, last_status, session).is_none();
        let process_scope = session.substitutions.scope();
        let mut isolated =
            (child && raw.redirects.iter().any(redirection_observable)).then(|| session.clone());
        let mut exp = Expansion::with_assignments(
            isolated.as_mut().unwrap_or(session),
            last_status,
            captured,
            if assignment_only {
                &mut cmd.assignments
            } else {
                &mut []
            },
            standalone && assignment_only,
        );
        let result = prepare_redirects(raw.redirects, &mut exp);
        if let Some(isolated) = isolated {
            session
                .substitutions
                .adopt(isolated.substitutions, process_scope);
        }
        cmd.redirects = result.map_err(|mut e| {
            if child {
                e.child_failure();
            }
            e
        })?;
        cmd.empty_status = captured.get().unwrap_or(0);
    }
    if assignment_only {
        cmd.assignments.clear();
    }
    Ok(cmd)
}

fn prepare_redirects(
    raw: impl IntoIterator<Item = Redirect<Word>>,
    exp: &mut Expansion<'_>,
) -> Result<Vec<Redirect>, crate::expansion::Error> {
    let raw = raw.into_iter();
    let mut redirects = Vec::with_capacity(raw.size_hint().0);
    for r in raw {
        let word = match (r.target, r.word) {
            (Target::HereDoc { expand: true }, Word::Literal(body)) => {
                if body.contains(['$', '`', '\\']) {
                    exp.scalar(
                        crate::parser::here_word(exp.session, &body).map_err(|e| e.message())?,
                    )?
                } else {
                    body
                }
            }
            (Target::HereString, word) => {
                let mut text = exp.scalar(word)?;
                text.push('\n');
                text
            }
            (_, Word::Literal(word)) => word,
            (_, word) => exp.redirect_path(word)?,
        };
        let mut word = word;
        let mut moved = None;
        let both = r.target == Target::OutputWord
            && !word.ends_with('-')
            && !word.bytes().all(|b| b.is_ascii_digit());
        let mut target = match r.target {
            Target::OutputWord if both => Target::Write,
            Target::OutputWord => Target::Duplicate,
            target => target,
        };
        if target == Target::Duplicate && word != "-" {
            let source = word.strip_suffix('-').unwrap_or(&word);
            let fd = source
                .parse::<RawFd>()
                .ok()
                .filter(|_| source.bytes().all(|b| b.is_ascii_digit()))
                .ok_or("invalid file descriptor")?;
            if source.len() != word.len() {
                moved = Some(fd);
                word.pop();
            }
        }
        if target == Target::Write && exp.session.noclobber {
            target = Target::NoClobber;
        }
        redirects.push(Redirect {
            line: r.line,
            variable: None,
            fd: r.fd,
            target,
            word,
        });
        if both {
            redirects.push(Redirect {
                line: r.line,
                variable: None,
                fd: 2,
                target: Target::Duplicate,
                word: "1".into(),
            });
        }
        if let Some(fd) = moved {
            redirects.push(Redirect {
                line: r.line,
                variable: None,
                fd,
                target: Target::CloseMoved,
                word: String::new(),
            });
        }
    }
    Ok(redirects)
}

pub fn run(
    pipeline: Pipeline<Word>,
    last_status: u8,
    replace: bool,
    session: &mut Session,
) -> io::Result<Flow> {
    run_pipeline(pipeline, last_status, replace, session, false)
}

fn job_label(
    pipeline: &[SimpleCommand<Word>],
    background: bool,
    session: &Session,
) -> Option<String> {
    // Literal arguments survive preparation, except for lookup prefixes.
    // Build their job label only if a foreground child is actually launched.
    if !background && !session.jobs.monitor {
        return None;
    }
    if pipeline.iter().any(|cmd| cmd.compound.is_some())
        && let Some(location) = &session.current_command
    {
        return Some(
            location
                .text()
                .trim()
                .trim_end_matches('&')
                .trim_end()
                .to_owned(),
        );
    }
    if !background
        && let [cmd] = pipeline
        && cmd.assignments.is_empty()
        && !matches!(
            cmd.words.first().and_then(Word::as_literal),
            Some("command" | "builtin")
        )
        && cmd.words.iter().all(|w| w.as_literal().is_some())
    {
        return None;
    }
    let mut label = String::new();
    for (stage, cmd) in pipeline.iter().enumerate() {
        if stage != 0 {
            label.push_str(" | ");
        }
        for (i, word) in cmd.words.iter().enumerate() {
            if i != 0 {
                label.push(' ');
            }
            label.push_str(word.as_literal().unwrap_or("command"));
        }
    }
    Some(label)
}

// In sh mode a regular command installs its redirects after argument expansion
// and before expanding its assignment prefixes (POSIX 2.9.1.1).
fn needs_ordered_redirects(raw: &SimpleCommand<Word>, session: &Session) -> bool {
    if !session.posix
        || raw.redirects.is_empty()
        || raw.assignments.is_empty()
        || raw.words.is_empty()
    {
        return false;
    }
    if let Some(name) = raw.words[0].as_literal() {
        if special_builtin(name) {
            return false;
        }
        // An infallible literal prefix only becomes visible during dispatch.
        // With literal file redirects the ordinary route has the same effects.
        if !session.xtrace
            && raw.words[1..].iter().all(|w| w.as_literal().is_some())
            && raw.assignments.iter().all(|(key, value)| {
                matches!(value, Assignment::Scalar(Word::Literal(_)))
                    && crate::parser::is_identifier(key)
                    && !session.is_nameref(key)
                    && !session.integer(key)
                    && !session.readonly(key)
            })
            && raw.redirects.iter().all(|r| {
                r.variable.is_none()
                    && matches!(
                        r.target,
                        Target::Read
                            | Target::Write
                            | Target::Clobber
                            | Target::Append
                            | Target::ReadWrite
                    )
                    && r.word.as_literal().is_some()
            })
        {
            return false;
        }
    }
    true
}

fn run_ordered(
    mut raw: SimpleCommand<Word>,
    name: Option<String>,
    status: u8,
    replace: bool,
    session: &mut Session,
) -> io::Result<Flow> {
    let captured = Cell::new(None);
    let words = match expand_command_words(&mut raw, session, status, &captured) {
        Ok(words) => words,
        Err(e) => {
            e.report("");
            return Ok(e.flow(session));
        }
    };
    let redirects = if words.first().is_some_and(|w| !special_builtin(w)) {
        std::mem::take(&mut raw.redirects)
    } else {
        Vec::new()
    };
    // An external utility expands its redirection operands in a child
    // environment. Only dynamic operands can expose that distinction.
    let child_redirects = redirects
        .iter()
        .any(|r| r.word.as_literal().is_none() || redirection_observable(r))
        && {
            let mut query = PreparedCommand {
                words: words.clone(),
                ..Default::default()
            };
            lookup::command_prefix(&mut query, session);
            builtin(&query, status, session).is_none()
        };
    named_fd::with_redirects(redirects, session, status, child_redirects, |session| {
        let cmd = match prepare_expanded(raw, words, session, status, true, &captured) {
            Ok(cmd) => cmd,
            Err(e) => {
                e.report("");
                return Ok((e.flow(session), false));
            }
        };
        let exec = matches!(builtin(&cmd, status, session), Some(Builtin::Exec));
        let flow = run_prepared(cmd, name, status, replace, session)?;
        let commit = exec && matches!(flow, Flow::Continue(0));
        Ok((flow, commit))
    })
}

fn run_prepared(
    mut cmd: PreparedCommand,
    name: Option<String>,
    last_status: u8,
    replace: bool,
    session: &mut Session,
) -> io::Result<Flow> {
    let simple = cmd.compound.is_none();
    if !cmd.named_redirects.is_empty() {
        let last = simple.then(|| cmd.words.last().cloned().unwrap_or_default());
        let launch = session.jobs.launch(false, true);
        let result = named_fd::run(cmd, session, last_status, replace, launch);
        if let Some(last) = last {
            session.last_argument = Some(last)
        }
        return result;
    }
    let selected = builtin(&cmd, last_status, session);
    // Function calls take ownership of their argv. Other commands can move
    // their final word into $_ after execution without copying its contents.
    let saved = matches!(selected, Some(Builtin::Function))
        .then(|| cmd.words.last().cloned())
        .flatten();
    let result = if let Some(builtin) = selected {
        run_builtin(builtin, &mut cmd, session, last_status, replace)
    } else {
        run_external(&mut cmd, name, last_status, replace, session)
    };
    if simple {
        session.last_argument = Some(saved.or_else(|| cmd.words.pop()).unwrap_or_default());
    }
    result
}

fn run_external(
    cmd: &mut PreparedCommand,
    name: Option<String>,
    last_status: u8,
    replace: bool,
    session: &mut Session,
) -> io::Result<Flow> {
    // A foreground builtin runs in this shell. Snapshot terminal modes
    // only when preparing to hand the terminal to a child process.
    let mut launch = session.jobs.launch(false, true);
    if !cmd.cacheable() {
        if cmd.assignments.iter().any(|(k, _)| k == "PATH") {
            session.hashes.clear();
        }
    } else if !replace {
        match hash::resolve(session, &cmd.words[0]) {
            Ok(path) => cmd.resolved = path,
            Err(hash::Absent)
                if session
                    .functions
                    .contains_key(PreparedCommand::MISSING_HOOK) =>
            {
                cmd.select_missing_hook();
            }
            Err(hash::Absent)
                if cmd.redirects.is_empty()
                    && launch.group.is_none()
                    && !session.traps.active() =>
            {
                let prefix = session.diagnostic_prefix(|| session.line());
                eprintln!(
                    "{}{}: command not found",
                    prefix.as_deref().unwrap_or("tonari: "),
                    cmd.words[0]
                );
                return Ok(Flow::Continue(127));
            }
            Err(hash::Absent) => {}
        }
    }
    if replace && launch.group.is_none() && !cmd.prepare_missing_hook(session) {
        let executable = External::new(cmd, session);
        return Ok(Flow::Continue(
            if apply(&actions(cmd), here_input::PIPE_LIMIT) {
                executable.exec(
                    (cmd.assignments.is_empty() && cmd.redirects.is_empty()).then_some(session),
                )
            } else {
                1
            },
        ));
    }
    let stage = start(cmd, None, None, None, last_status, session, launch)?;
    let status = match stage {
        Stage::Child(pid) | Stage::GroupedChild(pid) if launch.group.is_some() => {
            stage.join_group(&mut launch);
            session
                .jobs
                .finish(
                    launch,
                    vec![Ok(pid)],
                    name.unwrap_or_else(|| cmd.words.join(" ")),
                    session.pipefail,
                    &mut session.pipe_status,
                )?
                .0
        }
        Stage::Child(pid) | Stage::GroupedChild(pid) => wait(pid)?,
        Stage::Status(code) => code,
    };
    Ok(Flow::Continue(status))
}

fn foreground(
    mut raw: SimpleCommand<Word>,
    last_status: u8,
    replace: bool,
    session: &mut Session,
) -> io::Result<Flow> {
    if session.keyword {
        separate_keyword_arguments(&mut raw);
    }
    if raw.words.is_empty()
        && raw.redirects.is_empty()
        && raw.compound.is_none()
        && !session.xtrace
        && (raw.assignments.is_empty()
            || matches!(raw.assignments.as_slice(),
            [(key, Assignment::Scalar(Word::Literal(value)))] if session.assign_existing_literal(key, value)))
    {
        session.last_argument = Some(String::new());
        return Ok(Flow::Continue(0));
    }
    let name = job_label(std::slice::from_ref(&raw), false, session);
    if needs_ordered_redirects(&raw, session) {
        return run_ordered(raw, name, last_status, replace, session);
    }
    let cmd = match prepare(raw, session, last_status, true) {
        Ok(cmd) => cmd,
        Err(e) => {
            e.report("");
            return Ok(e.flow(session));
        }
    };
    run_prepared(cmd, name, last_status, replace, session)
}

fn lastpipe(
    raw: SimpleCommand<Word>,
    incoming: OwnedFd,
    last_status: u8,
    session: &mut Session,
) -> io::Result<Flow> {
    let pipe = Action::Dup(incoming.as_raw_fd(), 0);
    let saved = SavedFd::capture(&pipe, std::slice::from_ref(&pipe))?;
    duplicate(incoming.as_raw_fd(), 0)?;
    drop(incoming);
    let was_redirected = std::mem::replace(&mut session.stdin_redirected, true);
    let result = foreground(raw, last_status, false, session);
    session.stdin_redirected = was_redirected;
    saved.restore(0)?;
    result
}

pub fn run_pipeline(
    mut pipeline: Pipeline<Word>,
    last_status: u8,
    replace: bool,
    session: &mut Session,
    background: bool,
) -> io::Result<Flow> {
    if pipeline.len() == 1 && !background {
        let raw = pipeline.pop().unwrap();
        if raw.compound.is_none()
            && let Some(flow) = crate::traps::debug(session, last_status)
        {
            return Ok(flow);
        }
        return foreground(raw, last_status, replace, session);
    }
    let mut launch = session.jobs.launch(background, pipeline.len() == 1);
    let managed = background || launch.group.is_some();
    let name = job_label(&pipeline, background, session);
    let mut input =
        if background && launch.group.is_none() && (session.posix || !session.stdin_redirected) {
            Some(private_fd(std::fs::File::open("/dev/null")?.into())?)
        } else {
            None
        };
    let length = pipeline.len();
    let mut stages: Vec<Stage> = Vec::with_capacity(length);
    let mut last_flow = None;
    for (i, mut raw) in pipeline.into_iter().enumerate() {
        if !session.traps.in_action()
            && let Some(loc) = raw.location.take()
        {
            session.current_command = Some(loc);
        }
        if raw.compound.is_none()
            && let Some(flow) = crate::traps::debug(session, last_status)
        {
            drop(input);
            for stage in stages {
                if let Some(pid) = stage.pid() {
                    let _ = wait(pid);
                }
            }
            return Ok(flow);
        }
        let stage = (|| {
            let incoming = input.take();
            if i + 1 == length
                && !managed
                && session.shopt.lastpipe
                && let Some(reader) = incoming
            {
                let flow = lastpipe(raw, reader, last_status, session)?;
                let code = match flow {
                    Flow::Continue(code) => return Ok(Stage::Status(code)),
                    Flow::Exit(code) | Flow::Discard(code) | Flow::Return(code) => code,
                    Flow::Break(..) | Flow::LoopContinue(_) => 0,
                };
                last_flow = Some(flow);
                return Ok(Stage::Status(code));
            }
            let output = if i + 1 < length {
                let (reader, writer) = pipe()?;
                input = Some(reader);
                Some(writer)
            } else {
                None
            };
            start_raw(
                raw,
                incoming,
                output,
                input.as_ref(),
                last_status,
                session,
                launch,
            )
        })();
        match stage {
            Ok(stage) => {
                stage.join_group(&mut launch);
                stages.push(stage);
            }
            Err(e) => {
                drop(input);
                for stage in &stages {
                    if let Some(pid) = stage.pid() {
                        unsafe { libc::kill(pid, libc::SIGKILL) };
                    }
                }
                for stage in stages {
                    if let Some(pid) = stage.pid() {
                        let _ = wait(pid);
                    }
                }
                session.jobs.restore();
                return Err(e);
            }
        }
    }
    if managed {
        let processes = stages
            .into_iter()
            .map(|s| match s {
                Stage::Child(p) | Stage::GroupedChild(p) => Ok(p),
                Stage::Status(c) => Err(c),
            })
            .collect();
        let (code, pid) = session.jobs.finish(
            launch,
            processes,
            name.unwrap_or_default(),
            session.pipefail,
            &mut session.pipe_status,
        )?;
        if background {
            session.last_background = pid;
        }
        return Ok(Flow::Continue(code));
    }
    let mut status = 0;
    let mut wait_error = None;
    session.pipe_status.clear();
    for stage in stages {
        let result = match stage {
            Stage::Child(pid) | Stage::GroupedChild(pid) => wait(pid),
            Stage::Status(code) => Ok(code),
        };
        session.pipe_status.push(*result.as_ref().unwrap_or(&1));
        match result {
            Ok(code) if !session.pipefail || code != 0 => status = code,
            Ok(_) => {}
            Err(error) => wait_error = Some(error),
        }
    }
    if let Some(e) = wait_error {
        return Err(e);
    }
    Ok(last_flow.unwrap_or(Flow::Continue(status)))
}
