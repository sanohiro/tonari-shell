pub(crate) mod coprocess;
pub(crate) mod fd;
pub(crate) mod hash;
mod named_fd;
pub(crate) mod process_substitution;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
mod here_input;
mod lookup;
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

#[derive(Debug, PartialEq)]
pub enum Flow {
    Continue(u8),
    Discard(u8),
    Exit(u8),
    Return(u8),
    Break(u32),
    LoopContinue(u32),
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
    // Can run in a forked child. Avoid buffered output and heap allocation.
    for part in [b"tonari: ".as_slice(), name, message, b"\n"] {
        unsafe { libc::write(2, part.as_ptr().cast(), part.len()) };
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

fn open_redirect(path: &CStr, flags: i32) -> Option<OwnedFd> {
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
        write_error(path.to_bytes(), b": cannot open redirection");
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

fn apply(actions: &[Action<'_>]) -> bool {
    if let Err(e) = protect_fds(actions) {
        eprintln!("tonari: redirection: {e}");
        return false;
    }
    apply_protected(actions)
}

fn apply_protected(actions: &[Action<'_>]) -> bool {
    for a in actions {
        match a {
            Action::HereInput(dest, body) => {
                let redirected = here_input::open(body).and_then(|fd| {
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
                    return false;
                }
            }
            Action::Open(dest, path, flags) => {
                let Some(fd) = open_redirect(path, *flags) else {
                    return false;
                };
                if fd.as_raw_fd() == *dest {
                    let _ = fd.into_raw_fd();
                } else if duplicate(fd.as_raw_fd(), *dest).is_err() {
                    write_error(path.as_bytes(), b": cannot redirect");
                    return false;
                }
            }
            Action::Dup(src, dst) => {
                if duplicate(*src, *dst).is_err() {
                    write_error(b"redirection", b": invalid file descriptor");
                    return false;
                }
            }
            Action::Close(fd) | Action::CloseMoved(fd) => {
                unsafe { libc::close(*fd) };
            }
        }
    }
    true
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
    ignore_pipe: bool,
    // Owns the argument strings referenced by argv.
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
                std::env::split_paths(&path)
                    .map(|dir| CString::new(dir.join(name).as_os_str().as_bytes()).unwrap())
                    .collect()
            })
        };
        Self {
            words,
            argv,
            original_name: original,
            ignore_pipe: session.traps.ignores(libc::SIGPIPE),
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
        let shell = CString::new(std::env::current_exe()?.as_os_str().as_bytes())?;
        let mut args = Vec::with_capacity(self.argv.len() + 2);
        args.extend([shell.as_ptr(), c"--".as_ptr(), path.as_ptr()]);
        args.extend_from_slice(&self.argv[1..]);
        launch(&shell, &args)
    }

    fn script_fallback(&self, path: &CStr) -> i32 {
        self.with_script(path, |shell, args| {
            let env = self.env.as_ref().map_or_else(
                || inherited_env().cast::<*const libc::c_char>(),
                |e| e.pointers.as_ptr(),
            );
            unsafe { libc::execve(shell.as_ptr(), args.as_ptr(), env) };
            Err::<(), _>(io::Error::last_os_error())
        })
        .unwrap_err()
        .raw_os_error()
        .unwrap_or(libc::ENOEXEC)
    }

    fn exec(&self) -> u8 {
        // Rust ignores SIGPIPE in its own runtime. External commands need the
        // normal Unix disposition, including on the direct exec path.
        let program = self.original_name.as_ref().unwrap_or(&self.words[0]);
        if program.as_bytes().is_empty() {
            write_error(b"", b": command not found");
            return 127;
        }
        let old_pipe = if self.ignore_pipe {
            libc::SIG_ERR
        } else {
            unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) }
        };
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
                    error = self.script_fallback(path);
                    break;
                }
                libc::EACCES => error = libc::EACCES,
                e => {
                    error = e;
                    break;
                }
            }
        }
        if old_pipe != libc::SIG_ERR {
            unsafe { libc::signal(libc::SIGPIPE, old_pipe) };
        }
        let status = exec_status(error);
        write_error(
            program.as_bytes(),
            if status == 127 {
                b": command not found"
            } else {
                b": cannot execute"
            },
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

fn builtin(cmd: &PreparedCommand, last_status: u8, session: &Session) -> Option<Builtin> {
    if cmd.compound.is_some() {
        return Some(Builtin::Compound);
    }
    let Some(name) = cmd.words.first() else {
        return Some(Builtin::Status(cmd.empty_status));
    };
    if cmd.search == lookup::Search::Normal && session.functions.contains_key(name) {
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
        "return" => Some(Builtin::Return(match args.first() {
            None => Some(last_status),
            Some(s) => s.trim().parse::<i64>().ok().map(|n| n as u8),
        })),
        "exit" | "logout" => {
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
    let mut bindings = session.bind(&cmd.assignments);
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
        Builtin::Function => {
            crate::call_function(session, std::mem::take(&mut cmd.words), last_status)
        }
        Builtin::Trap => Flow::Continue(session.traps.builtin(args, session.interactive)),
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
        Builtin::Ulimit => {
            Flow::Continue(crate::builtins::limits::run(args, &mut io::stdout().lock()))
        }
        Builtin::Umask => Flow::Continue(crate::builtins::umask::run(args)),
        Builtin::Mapfile => crate::session::mapfile::run(session, &cmd.words[0], args, last_status),
        Builtin::Read => {
            let here = match cmd.redirects.as_slice() {
                [r] if matches!(r.target, Target::HereDoc { .. } | Target::HereString)
                    && r.word.len() <= libc::PIPE_BUF =>
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
        Builtin::Declare => session.declare(&cmd.words[0], args, &cmd.array_declarations),
        Builtin::Echo => Flow::Continue(crate::builtins::echo(args)),
        Builtin::Test => Flow::Continue(crate::builtins::test(&cmd.words, session)),
        Builtin::LoopControl => {
            if session.loop_depth == 0 {
                write_error(cmd.words[0].as_bytes(), b": only meaningful in a loop");
                Flow::Continue(0)
            } else if args.len() > 1 {
                write_error(cmd.words[0].as_bytes(), b": too many arguments");
                Flow::Exit(1)
            } else {
                let depth = args.first().map_or(Ok(1), |n| n.trim().parse::<i64>());
                match depth {
                    Ok(n) if n > 0 => {
                        let levels = (n as u64).min(u64::from(session.loop_depth)) as u32;
                        if cmd.words[0] == "break" {
                            Flow::Break(levels)
                        } else {
                            Flow::LoopContinue(levels)
                        }
                    }
                    _ => {
                        write_error(cmd.words[0].as_bytes(), b": invalid loop count");
                        Flow::Exit(1)
                    }
                }
            }
        }
        Builtin::Export => {
            let status = session.export(args, &mut bindings);
            Flow::Continue(apply_array_declarations(cmd, status, session))
        }
        Builtin::Unset => Flow::Continue(session.unset(args)),
        Builtin::DirectoryStack => {
            Flow::Continue(session.directory_stack(&cmd.words[0], args, &mut io::stdout().lock()))
        }
        Builtin::Directory => Flow::Continue(session.directory_builtin(&cmd.words[0], args)),
        Builtin::Source => crate::source_file(session, &cmd.words, last_status),
        Builtin::Eval => crate::eval_text(
            session,
            &(if args.first().is_some_and(|s| s == "--") {
                &args[1..]
            } else {
                args
            })
            .join(" "),
            last_status,
        ),
        Builtin::Return(status) => {
            if session.source_depth == 0 && session.function_depth == 0 {
                write_error(
                    b"return",
                    b": can only return from a function or sourced script",
                );
                Flow::Continue(1)
            } else if let Some(code) = status {
                if args.len() > 1 {
                    write_error(b"return", b": too many arguments");
                    Flow::Continue(1)
                } else {
                    session.traps.before_return = last_status;
                    Flow::Return(code)
                }
            } else {
                write_error(b"return", b": numeric argument required");
                Flow::Return(2)
            }
        }
        Builtin::Status(code) => Flow::Continue(code),
        Builtin::Exit(code) if args.is_empty() => Flow::Exit(crate::startup::logout(
            session,
            session.traps.exit_status.unwrap_or(code),
            last_status,
        )),
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
    session.restore(bindings);
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
    let status = external.exec();
    if session.interactive {
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
                crate::builtins::echo_into(words, &mut bytes);
                0
            }
            Self::Printf(words) => crate::session::printf::run(session, words, Some(&mut bytes)),
            Self::DirectoryStack(command, args) => {
                session.directory_stack(command, args, &mut bytes)
            }
            Self::Limits(args) => crate::builtins::limits::run(args, &mut bytes),
        };
        (bytes, status)
    }
}

fn run_builtin(
    builtin: Builtin,
    cmd: &mut PreparedCommand,
    session: &mut Session,
    last_status: u8,
    final_command: bool,
) -> io::Result<Flow> {
    if cmd.redirects.is_empty() {
        return Ok(eval_builtin(builtin, cmd, session, last_status, false));
    }
    let multiple;
    let one;
    let actions: &[Action] = if let [redirect] = cmd.redirects.as_slice() {
        one = [action(redirect)];
        &one
    } else {
        multiple = actions(cmd);
        &multiple
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
            return Ok(Flow::Continue(1));
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
            return Ok(Flow::Continue(1));
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
    if !final_command {
        for action in actions {
            if matches!(action, Action::CloseMoved(_)) {
                continue;
            }
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
    let applied = apply_protected(actions);
    let old_input = session.stdin_redirected;
    if applied && actions.iter().any(|a| a.destination() == 0) {
        session.stdin_redirected = true;
    }
    let flow = if applied {
        eval_builtin(builtin, cmd, session, last_status, false)
    } else {
        Flow::Continue(1)
    };
    if !matches!(builtin, Builtin::Exec) || !matches!(flow, Flow::Continue(0)) {
        session.stdin_redirected = old_input;
    }
    if applied && matches!(builtin, Builtin::Exec) && matches!(flow, Flow::Continue(0)) {
        for r in &cmd.redirects {
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
    restore.map(|()| flow)
}

enum Stage {
    Child(libc::pid_t),
    Status(u8),
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
fn reset_child_signals() {
    for signal in [libc::SIGINT, libc::SIGQUIT] {
        unsafe {
            let mut old: libc::sigaction = std::mem::zeroed();
            libc::sigaction(signal, std::ptr::null(), &mut old);
            if old.sa_sigaction != libc::SIG_IGN {
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
            Part::Substitution { .. } | Part::Process { .. } | Part::Arithmetic { .. } => true,
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
    session.subshell_depth += 1;
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
    let observable = raw.redirects.iter().any(redirection_observable)
        || raw
            .words
            .iter()
            .chain(raw.redirects.iter().map(|r| &r.word))
            .any(stage_expansion)
        || raw.assignments.iter().any(|(_, value)| match value {
            Assignment::Scalar(word) => stage_expansion(word),
            Assignment::Array(_, items) => items.iter().any(|(_, word)| stage_expansion(word)),
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
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        session
            .traps
            .after_fork(session.errtrace, session.functrace);
        reset_child_signals();
        launch.child();
        enter_stage(session, raw.compound.as_deref(), launch.background);
        if let Some(command) = raw.compound.as_deref_mut()
            && let crate::parser::Command::Subshell(body) = command
        {
            coprocess::after_fork(session);
            *command = crate::parser::Command::Group(std::mem::take(body));
        }
        session.stdin_redirected |= input.is_some();
        let code = if apply(&setup) {
            match run(vec![raw], last_status, true, session) {
                Ok(Flow::Discard(s) | Flow::Continue(s) | Flow::Exit(s) | Flow::Return(s)) => s,
                Ok(Flow::Break(_) | Flow::LoopContinue(_)) => 0,
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
    let builtin = builtin(cmd, last_status, session);
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
        && r.word.len() <= libc::PIPE_BUF
        && let Ok(reader) = here_input::open(r.word.as_bytes()).and_then(private_fd)
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
        && launch.group.is_none()
        && !launch.background
        && resolved
        && input.is_none()
        && output.is_none()
        && cmd.redirects.is_empty()
    {
        match External::new(cmd, session).spawn_resolved(None) {
            Ok(pid) => return Ok(Stage::Child(pid)),
            Err(e) if e.raw_os_error() == Some(libc::ENOEXEC) => {}
            Err(e) => return spawn_failure(&cmd.words[0], e),
        }
    }
    if builtin.is_none()
        && !resolved
        && !session.traps.ignores(libc::SIGPIPE)
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
    setup.extend(actions(cmd));
    let external = builtin.is_none().then(|| External::new(cmd, session));
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
        reset_child_signals();
        launch.child();
        enter_stage(session, cmd.compound.as_deref(), launch.background);
        session.stdin_redirected |= cmd.redirects.iter().any(|r| r.fd == 0);
        session.stdin_redirected |= input.is_some();
        let code = if !apply(&setup) {
            1
        } else if let Some(external) = external.as_ref() {
            external.exec()
        } else {
            coprocess::after_fork(session);
            match eval_builtin(builtin.unwrap(), cmd, session, last_status, true) {
                Flow::Discard(s) | Flow::Exit(s) | Flow::Return(s) | Flow::Continue(s) => s,
                Flow::Break(_) | Flow::LoopContinue(_) => 0,
            }
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

pub(super) fn capture_external(
    raw: SimpleCommand<Word>,
    session: &mut Session,
    last: u8,
) -> Result<(String, u8), &'static str> {
    let mut cmd = match prepare(raw, session, last, true) {
        Ok(cmd) => cmd,
        Err(error) => {
            eprintln!("tonari: {error}");
            return Ok((String::new(), error.status));
        }
    };
    let (reader, writer) = pipe().map_err(|_| "cannot open command substitution pipe")?;
    let stage = start(
        &mut cmd,
        None,
        Some(writer),
        None,
        last,
        session,
        Default::default(),
    )
    .map_err(|_| "cannot launch command substitution")?;
    let pid = match stage {
        Stage::Child(pid) => pid,
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
        if unsafe { libc::waitpid(pid, &mut status, 0) } >= 0 {
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
        "BASHPID" => Some(std::process::id().to_string()),
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
        if matches!(name, "builtin" | "command" | "set") || !builtin_names().contains(&name) {
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
            words,
            redirects,
            ..Self::default()
        }))
    }

    pub(crate) fn run_literal(&mut self, session: &mut Session, status: u8) -> Option<Flow> {
        if session.xtrace || session.traps.active() || session.jobs.monitor {
            return None;
        }
        if session.noclobber && self.redirects.iter().any(|r| r.target == Target::Write) {
            return None;
        }
        let selected = builtin(self, status, session)?;
        if matches!(selected, Builtin::Function | Builtin::Set) {
            return None;
        }
        let scope = session.substitutions.scope();
        let flow = run_builtin(selected, self, session, status, false).unwrap_or_else(|e| {
            eprintln!("tonari: {e}");
            Flow::Continue(1)
        });
        session.substitutions.finish(scope);
        Some(flow)
    }

    fn cacheable(&self) -> bool {
        !self.default_path && !self.assignments.iter().any(|(k, _)| k == "PATH")
    }
    fn cached_path<'a>(&'a self, session: &'a Session, name: &str) -> Option<&'a Path> {
        if !self.cacheable() || name.contains('/') {
            return None;
        }
        self.resolved.as_deref().or_else(|| {
            session
                .hashes
                .get(name)
                .filter(|path| !session.shopt.checkhash || lookup::executable(path))
        })
    }
}

fn trace_word(word: &str) -> std::borrow::Cow<'_, str> {
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
    let mut words = Vec::with_capacity(raw.words.len());
    let mut original = Expansion::new(session, last_status, &captured);
    let mut positions = raw
        .assignments
        .iter_mut()
        .filter_map(|(_, assignment)| match assignment {
            Assignment::Array(Some(position), _) => Some(position),
            _ => None,
        })
        .peekable();
    for (index, word) in raw.words.into_iter().enumerate() {
        if positions.peek().is_some_and(|p| **p == index) {
            *positions.next().unwrap() = words.len();
        }
        match word {
            Word::Literal(word) => words.push(word),
            word => original.append_fields(word, &mut words)?,
        }
    }
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
    // Each assignment sees its predecessors, while command arguments saw the
    // outer environment. Standalone assignments persist even on redirect failure.
    for (mut key, word) in raw.assignments {
        let mut exp = Expansion::with_assignments(
            session,
            last_status,
            &captured,
            &mut assignments,
            standalone && words.is_empty(),
        );
        let append = key.ends_with('+');
        if append {
            key.pop();
        }
        match word {
            Assignment::Array(operand, parts) => {
                if operand.is_none() && exp.session.readonly(&key) {
                    return Err(crate::expansion::Error::readonly(&key));
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
                        session.assign_array(&key, append, &items, false)?;
                    }
                } else if let Some(operand) = operand {
                    array_declarations.push(ArrayDeclaration {
                        operand,
                        name: key,
                        append,
                        items,
                    });
                } else {
                    return Err(
                        "array literals as command environment assignments are unsupported".into(),
                    );
                }
            }
            Assignment::Scalar(word) => {
                let value = exp.scalar(word)?;
                if let std::borrow::Cow::Owned(resolved) = exp.session.reference_name(&key)? {
                    key = resolved;
                }
                let base =
                    crate::session::array::subscript(&key).map_or(key.as_str(), |(base, _)| base);
                if exp.session.readonly(base) {
                    let error = crate::expansion::Error::readonly(base);
                    if words.is_empty() {
                        return Err(error);
                    }
                    eprintln!("tonari: {error}");
                    continue;
                }
                if let Some((base, subscript)) = crate::session::array::subscript(&key)
                    && exp.session.is_associative(base)
                {
                    if !words.is_empty() {
                        return Err("invalid indexed environment assignment".into());
                    }
                    if standalone {
                        let key = exp.key(subscript)?;
                        let old = exp.session.assoc_value(base, &key).map(str::to_owned);
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
                let value = if exp.session.integer(base) {
                    let old = if append {
                        let old = exp.lookup(&key).unwrap_or_default();
                        exp.arithmetic(&old)?
                    } else {
                        0
                    };
                    old.wrapping_add(exp.arithmetic(&value)?).to_string()
                } else if append {
                    exp.lookup(&key).unwrap_or_default() + &value
                } else {
                    value
                };
                if let Some((base, index)) = crate::session::array::subscript(&key) {
                    if !words.is_empty() {
                        return Err("invalid indexed environment assignment".into());
                    }
                    if standalone {
                        let i = session.resolve_index(base, index)?;
                        session.assign_element(base, i, value);
                    }
                    continue;
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
    if session.xtrace && (!cmd.words.is_empty() || !cmd.assignments.is_empty()) {
        use std::io::Write;
        let prefix = session.get("PS4").unwrap_or_else(|| "+ ".into());
        session.xtrace = false;
        let mut trace = crate::prompt::render_text(session, prefix, last_status);
        session.xtrace = true;
        let fd = session
            .get("BASH_XTRACEFD")
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(2);
        let mut first = true;
        for (name, value) in &cmd.assignments {
            if !first {
                trace.push(' ');
            }
            first = false;
            trace.push_str(name);
            trace.push('=');
            trace.push_str(&trace_word(value));
        }
        for word in &cmd.words {
            if !first {
                trace.push(' ');
            }
            first = false;
            trace.push_str(&trace_word(word));
        }
        trace.push('\n');
        if let Ok(mut file) = fd::duplicate_except(fd, 3, |_| false) {
            let _ = file.write_all(trace.as_bytes());
        }
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
            &captured,
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
    raw: Vec<Redirect<Word>>,
    exp: &mut Expansion<'_>,
) -> Result<Vec<Redirect>, crate::expansion::Error> {
    let mut redirects = Vec::with_capacity(raw.len());
    for r in raw {
        let word = match (r.target, r.word) {
            (Target::HereDoc { expand: true }, Word::Literal(body)) => {
                if body.contains(['$', '`', '\\']) {
                    exp.scalar(crate::parser::expansion_string(&body).map_err(|e| e.message())?)?
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
            (_, word) => {
                let mut fields = exp.fields(word)?.into_iter();
                let word = fields.next().ok_or("ambiguous redirect")?;
                if fields.next().is_some() {
                    return Err("ambiguous redirect".into());
                }
                word
            }
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
            variable: None,
            fd: r.fd,
            target,
            word,
        });
        if both {
            redirects.push(Redirect {
                variable: None,
                fd: 2,
                target: Target::Duplicate,
                word: "1".into(),
            });
        }
        if let Some(fd) = moved {
            redirects.push(Redirect {
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

pub fn run_pipeline(
    mut pipeline: Pipeline<Word>,
    last_status: u8,
    replace: bool,
    session: &mut Session,
    background: bool,
) -> io::Result<Flow> {
    let mut launch = session.jobs.launch(background, pipeline.len() == 1);
    let managed = background || launch.group.is_some();
    let name = if managed {
        pipeline
            .iter()
            .map(|c| {
                c.words
                    .iter()
                    .map(|word| word.as_literal().unwrap_or("command"))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join(" | ")
    } else {
        String::new()
    };
    if pipeline.len() == 1 && !background {
        let raw = pipeline.pop().unwrap();
        if raw.compound.is_none()
            && let Some(flow) = crate::traps::debug(session, last_status)
        {
            return Ok(flow);
        }
        if raw.words.is_empty()
            && raw.redirects.is_empty()
            && raw.compound.is_none()
            && !session.xtrace
            && let [(key, Assignment::Scalar(Word::Literal(value)))] = raw.assignments.as_slice()
            && session.assign_existing_literal(key, value)
        {
            return Ok(Flow::Continue(0));
        }
        let mut cmd = match prepare(raw, session, last_status, true) {
            Ok(cmd) => cmd,
            Err(e) => {
                eprintln!("tonari: {e}");
                return Ok(e.flow(session));
            }
        };
        if !cmd.named_redirects.is_empty() {
            return named_fd::run(cmd, session, last_status, replace, launch);
        }
        if let Some(builtin) = builtin(&cmd, last_status, session) {
            return run_builtin(builtin, &mut cmd, session, last_status, replace);
        }
        if !cmd.cacheable() {
            if cmd.assignments.iter().any(|(k, _)| k == "PATH") {
                session.hashes.clear();
            }
        } else if !replace {
            cmd.resolved = hash::resolve(session, &cmd.words[0], true);
        }
        if replace && launch.group.is_none() {
            let executable = External::new(&cmd, session);
            return Ok(Flow::Continue(if apply(&actions(&cmd)) {
                executable.exec()
            } else {
                1
            }));
        }
        let status = match start(&mut cmd, None, None, None, last_status, session, launch)? {
            Stage::Child(pid) if managed => {
                launch.parent(pid);
                session
                    .jobs
                    .finish(
                        launch,
                        vec![Ok(pid)],
                        name,
                        session.pipefail,
                        &mut session.pipe_status,
                    )?
                    .0
            }
            Stage::Child(pid) => wait(pid)?,
            Stage::Status(code) => code,
        };
        return Ok(Flow::Continue(status));
    }
    let mut input = if background && launch.group.is_none() && !session.stdin_redirected {
        Some(private_fd(std::fs::File::open("/dev/null")?.into())?)
    } else {
        None
    };
    let length = pipeline.len();
    let mut stages = Vec::with_capacity(length);
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
                if let Stage::Child(pid) = stage {
                    let _ = wait(pid);
                }
            }
            return Ok(flow);
        }
        let stage = (|| {
            let incoming = input.take();
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
                if let Stage::Child(pid) = stage {
                    launch.parent(pid);
                }
                stages.push(stage);
            }
            Err(e) => {
                drop(input);
                for stage in &stages {
                    if let Stage::Child(pid) = stage {
                        unsafe { libc::kill(*pid, libc::SIGKILL) };
                    }
                }
                for stage in stages {
                    if let Stage::Child(pid) = stage {
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
                Stage::Child(p) => Ok(p),
                Stage::Status(c) => Err(c),
            })
            .collect();
        let (code, pid) = session.jobs.finish(
            launch,
            processes,
            name,
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
            Stage::Child(pid) => wait(pid),
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
    Ok(Flow::Continue(status))
}
