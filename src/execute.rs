pub(crate) mod fd;
pub(crate) mod hash;
pub(crate) mod process_substitution;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
mod here_input;
mod lookup;

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

pub enum Flow {
    Continue(u8),
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
        Target::Duplicate if r.word == "-" => Action::Close(r.fd),
        Target::Duplicate => Action::Dup(r.word.parse().unwrap(), r.fd),
        Target::HereDoc { .. } | Target::HereString => Action::HereInput(r.fd, r.word.as_bytes()),
        _ => {
            let flags = match r.target {
                Target::Read => libc::O_RDONLY,
                Target::Write => libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                Target::Append => libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND,
                Target::Duplicate | Target::HereDoc { .. } | Target::HereString => unreachable!(),
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
    let fd = unsafe { libc::open(path.as_ptr(), flags, 0o666 as libc::c_int) };
    if fd < 0 {
        write_error(path.to_bytes(), b": cannot open redirection");
        return None;
    }
    Some(unsafe { OwnedFd::from_raw_fd(fd) })
}

impl Action<'_> {
    fn destination(&self) -> RawFd {
        match *self {
            Self::Open(fd, ..) | Self::HereInput(fd, _) | Self::Dup(_, fd) | Self::Close(fd) => fd,
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
            Action::Close(fd) => {
                unsafe { libc::close(*fd) };
            }
        }
    }
    true
}

// Pipe setup uses standard streams as destinations. Reserve distinct source
// numbers even when Tonari starts with a standard stream already closed.
fn private_fd(fd: OwnedFd) -> io::Result<OwnedFd> {
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
                    .map(|(k, v)| (k.as_str(), v.as_deref())))
                {
                    return Some(cached.clone());
                }
            }
        }
        let changes: std::collections::BTreeMap<_, _> =
            session.environment_changes(assignments).collect();
        let mut strings = Vec::new();
        let mut push = |key: &[u8], value: &[u8]| {
            let mut entry = Vec::with_capacity(key.len() + value.len() + 2);
            entry.extend_from_slice(key);
            entry.push(b'=');
            entry.extend_from_slice(value);
            strings.push(CString::new(entry).unwrap());
        };
        for (key, val) in std::env::vars_os() {
            if !key.to_str().is_some_and(|name| changes.contains_key(name)) {
                push(key.as_bytes(), val.as_bytes());
            }
        }
        for (key, val) in &changes {
            if let Some(val) = val {
                push(key.as_bytes(), val.as_bytes());
            }
        }
        let mut pointers: Vec<_> = strings.iter().map(|v| v.as_ptr()).collect();
        pointers.push(std::ptr::null());
        let cached_changes = if assignments.is_empty() {
            changes
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.map(str::to_owned)))
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

    // The foreground process inherits all descriptors here. With the executable
    // already resolved, Linux needs only argv, environ and SIGPIPE's exec default.
    #[cfg(target_os = "linux")]
    fn spawn_resolved(&self) -> io::Result<libc::pid_t> {
        unsafe extern "C" {
            static environ: *mut *mut libc::c_char;
        }
        let path = self
            .search_paths
            .as_ref()
            .and_then(|paths| paths.first())
            .unwrap_or(&self.words[0]);
        let mut attr = std::mem::MaybeUninit::uninit();
        let mut pid = 0;
        unsafe {
            let err = libc::posix_spawnattr_init(attr.as_mut_ptr());
            if err != 0 {
                return Err(io::Error::from_raw_os_error(err));
            }
            let mut signals = std::mem::zeroed();
            libc::sigemptyset(&mut signals);
            libc::sigaddset(&mut signals, libc::SIGPIPE);
            libc::posix_spawnattr_setsigdefault(attr.as_mut_ptr(), &signals);
            libc::posix_spawnattr_setflags(attr.as_mut_ptr(), libc::POSIX_SPAWN_SETSIGDEF as _);
            let env = self
                .env
                .as_ref()
                .map_or(environ as *const *mut libc::c_char, |e| {
                    e.pointers.as_ptr().cast()
                });
            let err = libc::posix_spawn(
                &mut pid,
                path.as_ptr(),
                std::ptr::null(),
                attr.as_ptr(),
                self.argv.as_ptr().cast(),
                env,
            );
            libc::posix_spawnattr_destroy(attr.as_mut_ptr());
            if err != 0 {
                return Err(io::Error::from_raw_os_error(err));
            }
        }
        Ok(pid)
    }

    fn exec(&self) -> u8 {
        // Rust ignores SIGPIPE in its own runtime. External commands need the
        // normal Unix disposition, including on the direct exec path.
        let program = self.original_name.as_ref().unwrap_or(&self.words[0]);
        if program.as_bytes().is_empty() {
            write_error(b"", b": command not found");
            return 127;
        }
        let old_pipe = unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
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
    LoopControl,
    Echo,
    Test,
    Read,
    Printf,
    Jobs,
    Describe,
    Alias,
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
    named_builtin(name, &cmd.words[1..], last_status)
        .or_else(|| (cmd.search == lookup::Search::BuiltinsOnly).then_some(Builtin::Unknown))
}

pub(crate) fn builtin_names() -> &'static [&'static str] {
    &[
        ":", "[", ".", "break", "builtin", "cd", "command", "compgen", "complete", "compopt",
        "continue", "echo", "eval", "exit", "exec", "export", "false", "local", "printf", "pwd",
        "read", "return", "set", "shift", "source", "test", "true", "type", "unset", "jobs", "fg",
        "bg", "wait", "shopt", "declare", "typeset", "alias", "unalias",
    ]
}

fn named_builtin(name: &str, args: &[String], last_status: u8) -> Option<Builtin> {
    match name {
        "alias" | "unalias" => Some(Builtin::Alias),
        "builtin" => Some(Builtin::Status(0)),
        "complete" | "compgen" | "compopt" => Some(Builtin::Completion),
        "command" | "type" => Some(Builtin::Describe),
        ":" | "true" => Some(Builtin::Status(0)),
        "false" => Some(Builtin::Status(1)),
        "printf" => Some(Builtin::Printf),
        "jobs" | "fg" | "bg" | "wait" => Some(Builtin::Jobs),
        "read" => Some(Builtin::Read),
        "set" => Some(Builtin::Set),
        "hash" => Some(Builtin::Hash),
        "shopt" => Some(Builtin::Shopt),
        "shift" => Some(Builtin::Shift),
        "local" | "declare" | "typeset" => Some(Builtin::Declare),
        "echo" => Some(Builtin::Echo),
        "test" | "[" => Some(Builtin::Test),
        "break" | "continue" => Some(Builtin::LoopControl),
        "export" => Some(Builtin::Export),
        "unset" => Some(Builtin::Unset),
        "cd" | "pwd" => Some(Builtin::Directory),
        "source" | "." => Some(Builtin::Source),
        "eval" => Some(Builtin::Eval),
        "exec" => Some(Builtin::Exec),
        "return" => Some(Builtin::Return(match args.first() {
            None => Some(last_status),
            Some(s) => s.trim().parse::<i64>().ok().map(|n| n as u8),
        })),
        "exit" => {
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
    cmd: &PreparedCommand,
    session: &mut Session,
    last_status: u8,
) -> Flow {
    let mut bindings = session.bind(&cmd.assignments);
    let args = cmd.words.get(1..).unwrap_or_default();
    let result = match builtin {
        Builtin::Compound => crate::eval::command(
            session,
            cmd.compound.as_deref().unwrap().clone(),
            last_status,
            false,
        ),
        Builtin::Exec => exec_builtin(cmd, session, args),
        Builtin::Function => crate::call_function(session, &cmd.words[0], &cmd.words, last_status),
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
        Builtin::Jobs => Flow::Continue(crate::jobs::builtin(&cmd.words, session)),
        Builtin::Read => Flow::Continue(session.read(args)),
        Builtin::Set => Flow::Continue(session.set(args)),
        Builtin::Hash => Flow::Continue(hash::run(session, args)),
        Builtin::Shopt => Flow::Continue(session.shopt(args)),
        Builtin::Shift => Flow::Continue(session.shift(args)),
        Builtin::Declare => {
            let status = session.declare(&cmd.words[0], args);
            Flow::Continue(apply_array_declarations(cmd, status, session))
        }
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
        Builtin::Directory => Flow::Continue(session.directory_builtin(&cmd.words[0], args)),
        Builtin::Source => crate::source_file(session, &cmd.words, last_status),
        Builtin::Eval => crate::eval_text(session, &args.join(" "), last_status),
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
                    Flow::Return(code)
                }
            } else {
                write_error(b"return", b": numeric argument required");
                Flow::Return(2)
            }
        }
        Builtin::Status(code) => Flow::Continue(code),
        Builtin::Exit(code) => Flow::Exit(code),
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
    fn capture(fd: RawFd, actions: &[Action<'_>]) -> io::Result<Self> {
        match fd::duplicate_except(fd, 3, |n| actions.iter().any(|a| a.references(n))) {
            Ok(file) => Ok(Self::Open(fd::PrivateFile::track(file))),
            Err(e) if e.raw_os_error() == Some(libc::EBADF) => Ok(Self::Closed),
            Err(e) => Err(e),
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

fn run_builtin(
    builtin: Builtin,
    cmd: &PreparedCommand,
    session: &mut Session,
    last_status: u8,
    final_command: bool,
) -> io::Result<Flow> {
    if cmd.redirects.is_empty() {
        return Ok(eval_builtin(builtin, cmd, session, last_status));
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
            let fd = action.destination();
            if let Some(slot) = saved.get_mut(fd as usize) {
                if matches!(slot, SavedFd::Unchanged) {
                    *slot = SavedFd::capture(fd, actions)?;
                }
            } else if !extra.iter().any(|(n, _)| *n == fd) {
                extra.push((fd, SavedFd::capture(fd, actions)?));
            }
        }
    }
    let applied = apply(actions);
    let flow = if applied {
        eval_builtin(builtin, cmd, session, last_status)
    } else {
        Flow::Continue(1)
    };
    if applied && matches!(builtin, Builtin::Exec) && matches!(flow, Flow::Continue(0)) {
        for a in actions {
            session.substitutions.replaced(a.destination());
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
    Failed(u8),
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
    stage_expansion(&r.word)
        || matches!(r.target, Target::HereDoc { expand: true })
            && r.word
                .as_literal()
                .is_some_and(|text| text.contains(['$', '`']))
}

// Expansions that can read stdin or observe the stage PID must run after pipe
// installation. In particular, `printf x | echo "$(cat)"` consumes that pipe.
fn start_raw(
    raw: SimpleCommand<Word>,
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
            Assignment::Array(items) => items.iter().any(|(_, word)| stage_expansion(word)),
        });
    if !observable {
        return match prepare(raw, session, last_status, false) {
            Ok(cmd) => start(
                &cmd,
                input,
                output,
                next_reader,
                last_status,
                session,
                launch,
            ),
            Err(e) => {
                eprintln!("tonari: {e}");
                Ok(Stage::Failed(e.status))
            }
        };
    }
    let setup = connect_pipe(input.as_ref(), output.as_ref(), next_reader);
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        reset_child_signals();
        launch.child();
        session.jobs.detached();
        let code = if apply(&setup) {
            match run(vec![raw], last_status, true, session) {
                Ok(Flow::Continue(s) | Flow::Exit(s) | Flow::Return(s)) => s,
                Ok(Flow::Break(_) | Flow::LoopContinue(_)) => 0,
                Err(e) => {
                    eprintln!("tonari: {e}");
                    1
                }
            }
        } else {
            1
        };
        unsafe { libc::_exit(i32::from(code)) }
    }
    Ok(Stage::Child(pid))
}

fn start(
    cmd: &PreparedCommand,
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
        .and_then(|name| cmd.cached_path(session, name));
    let custom_search = cached.is_none()
        && !cmd.words.first().is_some_and(|s| s.contains('/'))
        && (cmd.default_path || session.overrides_search_path(&cmd.assignments));
    let merged_pipe = output.as_ref().filter(|_| {
        matches!(cmd.redirects.as_slice(), [r] if r.fd == 2 && r.target == Target::Duplicate && r.word == "1")
    });
    if builtin.is_none()
        && (cmd.redirects.is_empty() || merged_pipe.is_some())
        && !custom_search
        && launch.group.is_none()
        && !launch.background
    {
        #[cfg(target_os = "linux")]
        if input.is_none() && output.is_none() && (cached.is_some() || cmd.words[0].contains('/')) {
            match External::new(cmd, session).spawn_resolved() {
                Ok(pid) => return Ok(Stage::Child(pid)),
                // Keep the existing shell-script fallback and error diagnostics.
                Err(e) if e.raw_os_error() == Some(libc::ENOEXEC) => (),
                Err(e) => return spawn_failure(&cmd.words[0], e),
            }
        }
        // Let the standard library select the platform's process-spawn path.
        let explicit;
        let program = if let Some(path) = cached {
            if path.components().count() == 1 && !path.is_absolute() {
                explicit = Path::new(".").join(path);
                explicit.as_path()
            } else {
                path
            }
        } else {
            Path::new(&cmd.words[0])
        };
        let mut external = Command::new(program);
        external.arg0(&cmd.words[0]);
        external.args(&cmd.words[1..]);
        for (key, val) in session.environment_changes(&cmd.assignments) {
            if let Some(val) = val {
                external.env(key, val);
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
        reset_child_signals();
        launch.child();
        session.jobs.detached();
        let code = if !apply(&setup) {
            1
        } else if let Some(external) = external.as_ref() {
            external.exec()
        } else {
            match eval_builtin(builtin.unwrap(), cmd, session, last_status) {
                Flow::Exit(s) | Flow::Return(s) | Flow::Continue(s) => s,
                Flow::Break(_) | Flow::LoopContinue(_) => 0,
            }
        };
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
    let cmd = match prepare(raw, session, last, true) {
        Ok(cmd) => cmd,
        Err(error) => {
            eprintln!("tonari: {error}");
            return Ok((String::new(), error.status));
        }
    };
    let (reader, writer) = pipe().map_err(|_| "cannot open command substitution pipe")?;
    let stage = start(
        &cmd,
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
        Stage::Failed(code) => return Ok((String::new(), code)),
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
    Ok(Stage::Failed(exec_status(code)))
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

struct ArrayDeclaration {
    name: String,
    append: bool,
    items: Vec<(Option<i64>, String)>,
}

struct PreparedCommand {
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
    fn cacheable(&self) -> bool {
        !self.default_path && !self.assignments.iter().any(|(k, _)| k == "PATH")
    }
    fn cached_path<'a>(&'a self, session: &'a Session, name: &str) -> Option<&'a Path> {
        if !self.cacheable() || name.contains('/') {
            return None;
        }
        self.resolved
            .as_deref()
            .or_else(|| session.hashes.get(name))
    }
}

fn prepare(
    raw: SimpleCommand<Word>,
    session: &mut Session,
    last_status: u8,
    standalone: bool,
) -> Result<PreparedCommand, crate::expansion::Error> {
    let captured = Cell::new(None);
    let mut words = Vec::with_capacity(raw.words.len());
    let mut original = Expansion::new(session, last_status, &captured);
    for word in raw.words {
        match word {
            Word::Literal(word) => words.push(word),
            word => words.extend(original.fields(word)?),
        }
    }
    let mut assignments = Vec::with_capacity(raw.assignments.len());
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
            Assignment::Array(parts) => {
                let mut items = Vec::with_capacity(parts.len());
                for (index, word) in parts {
                    if let Some(index) = index {
                        let index = exp.arithmetic(&index)?;
                        items.push((Some(index), exp.scalar(word)?));
                    } else {
                        items.extend(exp.fields(word)?.into_iter().map(|s| (None, s)));
                    }
                }
                if words.is_empty() {
                    if standalone {
                        session.assign_array(&key, append, &items, false)?;
                    }
                } else if matches!(
                    words[0].as_str(),
                    "declare" | "local" | "typeset" | "export"
                ) {
                    array_declarations.push(ArrayDeclaration {
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
                let value = if append {
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
                if standalone && words.is_empty() {
                    session.assign(&key, &value);
                }
                assignments.push((key, value));
            }
        }
    }
    let assignment_only = words.is_empty() && raw.compound.is_none();
    let mut cmd = PreparedCommand {
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
    lookup::command_prefix(&mut cmd, session);
    if !raw.redirects.is_empty() {
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
                e.fatal = false;
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
        if r.target == Target::Duplicate
            && word != "-"
            && (word.is_empty()
                || !word.bytes().all(|b| b.is_ascii_digit())
                || word.parse::<RawFd>().is_err())
        {
            return Err("invalid file descriptor".into());
        }
        redirects.push(Redirect {
            fd: r.fd,
            target: r.target,
            word,
        });
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
    let mut launch = session.jobs.launch(background);
    let managed = background || launch.group.is_some();
    let name = if managed {
        pipeline
            .iter()
            .map(|c| {
                c.words
                    .first()
                    .and_then(Word::as_literal)
                    .unwrap_or("command")
            })
            .collect::<Vec<_>>()
            .join(" | ")
    } else {
        String::new()
    };
    if pipeline.len() == 1 && !background {
        let mut cmd = match prepare(pipeline.pop().unwrap(), session, last_status, true) {
            Ok(cmd) => cmd,
            Err(e) => {
                eprintln!("tonari: {e}");
                return Ok(e.flow(session));
            }
        };
        if let Some(builtin) = builtin(&cmd, last_status, session) {
            return run_builtin(builtin, &cmd, session, last_status, replace);
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
        let status = match start(&cmd, None, None, None, last_status, session, launch)? {
            Stage::Child(pid) if managed => {
                launch.parent(pid);
                session
                    .jobs
                    .finish(launch, vec![Ok(pid)], name, session.pipefail)?
                    .0
            }
            Stage::Child(pid) => wait(pid)?,
            Stage::Failed(code) => code,
        };
        return Ok(Flow::Continue(status));
    }
    let mut input = if background && launch.group.is_none() {
        Some(private_fd(std::fs::File::open("/dev/null")?.into())?)
    } else {
        None
    };
    let length = pipeline.len();
    let mut stages = Vec::with_capacity(length);
    for (i, raw) in pipeline.into_iter().enumerate() {
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
                Stage::Failed(c) => Err(c),
            })
            .collect();
        let (code, pid) = session
            .jobs
            .finish(launch, processes, name, session.pipefail)?;
        if background {
            session.last_background = pid;
        }
        return Ok(Flow::Continue(code));
    }
    let mut status = 0;
    let mut wait_error = None;
    for stage in stages {
        let result = match stage {
            Stage::Child(pid) => wait(pid),
            Stage::Failed(code) => Ok(code),
        };
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
