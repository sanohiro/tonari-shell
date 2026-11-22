use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::process::{Command, Stdio};

use crate::parser::{Pipeline, SimpleCommand, Target};

pub enum Flow {
    Continue(u8),
    Exit(u8),
}

enum Action {
    Open(RawFd, CString, i32),
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

fn actions(cmd: &SimpleCommand) -> Vec<Action> {
    cmd.redirects
        .iter()
        .map(|r| match r.target {
            Target::Duplicate if r.word == "-" => Action::Close(r.fd),
            Target::Duplicate => Action::Dup(r.word.parse().unwrap(), r.fd),
            _ => {
                let flags = match r.target {
                    Target::Read => libc::O_RDONLY,
                    Target::Write => libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                    Target::Append => libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND,
                    Target::Duplicate => unreachable!(),
                };
                Action::Open(r.fd, CString::new(r.word.as_bytes()).unwrap(), flags)
            }
        })
        .collect()
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

fn apply(actions: &[Action]) -> bool {
    for a in actions {
        match a {
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

// Keep private descriptors clear of the 0..=2 range addressed by this parser,
// even when Tonari starts with a standard stream already closed.
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

struct External {
    // Owns the argument strings referenced by argv.
    words: Vec<CString>,
    argv: Vec<*const libc::c_char>,
    search_paths: Option<Vec<CString>>,
}

impl External {
    fn new(words: &[String]) -> Self {
        let words: Vec<_> = words
            .iter()
            .map(|w| CString::new(w.as_bytes()).unwrap())
            .collect();
        let mut argv = Vec::with_capacity(words.len() + 1);
        argv.extend(words.iter().map(|w| w.as_ptr()));
        argv.push(std::ptr::null());
        let name = std::ffi::OsStr::from_bytes(words[0].as_bytes());
        let search_paths = (!words[0].as_bytes().contains(&b'/')).then(|| {
            let path = std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into());
            std::env::split_paths(&path)
                .map(|dir| CString::new(dir.join(name).as_os_str().as_bytes()).unwrap())
                .collect()
        });
        Self {
            words,
            argv,
            search_paths,
        }
    }

    fn exec(&self) -> u8 {
        // Rust ignores SIGPIPE in its own runtime. External commands need the
        // normal Unix disposition, including on the direct exec path.
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
        if self.words[0].as_bytes().is_empty() {
            write_error(b"", b": command not found");
            return 127;
        }
        let mut error = libc::ENOENT;
        let paths = self
            .search_paths
            .as_deref()
            .unwrap_or(std::slice::from_ref(&self.words[0]));
        for path in paths {
            unsafe { libc::execv(path.as_ptr(), self.argv.as_ptr()) };
            match errno() {
                libc::ENOENT | libc::ENOTDIR => continue,
                libc::EACCES => error = libc::EACCES,
                e => {
                    error = e;
                    break;
                }
            }
        }
        let status = exec_status(error);
        write_error(
            self.words[0].as_bytes(),
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
}

fn builtin(cmd: &SimpleCommand, last_status: u8) -> Option<Builtin> {
    match cmd.words.first().map(String::as_str) {
        None | Some(":" | "true") => Some(Builtin::Status(0)),
        Some("false") => Some(Builtin::Status(1)),
        Some("exit") => {
            let status = match cmd.words.get(1) {
                None => last_status,
                Some(s) => match s.trim().parse::<i64>() {
                    Ok(n) => n as u8,
                    Err(_) => return Some(Builtin::InvalidExit),
                },
            };
            Some(if cmd.words.len() > 2 {
                Builtin::ExtraExitArgs
            } else {
                Builtin::Exit(status)
            })
        }
        _ => None,
    }
}

fn eval_builtin(builtin: Builtin) -> Flow {
    match builtin {
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
    }
}

enum SavedFd {
    Unchanged,
    Closed,
    Open(OwnedFd),
}

impl SavedFd {
    fn capture(fd: RawFd) -> io::Result<Self> {
        let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
        if copy >= 0 {
            Ok(Self::Open(unsafe { OwnedFd::from_raw_fd(copy) }))
        } else if errno() == libc::EBADF {
            Ok(Self::Closed)
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

fn run_builtin(builtin: Builtin, actions: &[Action], final_command: bool) -> io::Result<Flow> {
    if actions.is_empty() {
        return Ok(eval_builtin(builtin));
    }
    if let (Builtin::Status(code), [Action::Open(dest, path, flags)]) = (builtin, actions) {
        // A silent builtin needs the open's effects (including a blocking FIFO),
        // but never reads or writes the destination. Open before reserving any
        // backup, so /dev/fd paths see the caller's descriptor table.
        let Some(fd) = open_redirect(path, *flags) else {
            return Ok(Flow::Continue(1));
        };
        // Bash can still fail to save a live destination at its descriptor limit.
        if fd.as_raw_fd() != *dest && matches!(SavedFd::capture(*dest)?, SavedFd::Closed) {
            // With multiple closed standard streams the target can differ from
            // the new descriptor. dup2 checks it against the process fd limit.
            duplicate(fd.as_raw_fd(), *dest)?;
            unsafe { libc::close(*dest) };
        }
        return Ok(Flow::Continue(code));
    }
    let mut saved = [const { SavedFd::Unchanged }; 3];
    // A live shell needs only the original value of each destination. When -c
    // exits after this command, the kernel will close its descriptors anyway.
    if !final_command {
        for action in actions {
            let fd = match *action {
                Action::Open(fd, ..) | Action::Dup(_, fd) | Action::Close(fd) => fd,
            };
            let slot = &mut saved[fd as usize];
            if matches!(slot, SavedFd::Unchanged) {
                *slot = SavedFd::capture(fd)?;
            }
        }
    }
    let flow = if apply(actions) {
        eval_builtin(builtin)
    } else {
        Flow::Continue(1)
    };
    let mut restore = Ok(());
    for (fd, previous) in saved.into_iter().enumerate() {
        match previous {
            SavedFd::Unchanged => {}
            SavedFd::Closed => unsafe {
                libc::close(fd as RawFd);
            },
            SavedFd::Open(copy) => {
                if let Err(error) = duplicate(copy.as_raw_fd(), fd as RawFd) {
                    restore = Err(error);
                }
            }
        }
    }
    restore.map(|()| flow)
}

enum Stage {
    Child(libc::pid_t),
    Failed(u8),
}

fn start(
    cmd: &SimpleCommand,
    input: Option<OwnedFd>,
    output: Option<OwnedFd>,
    next_reader: Option<&OwnedFd>,
    last_status: u8,
) -> io::Result<Stage> {
    let builtin = builtin(cmd, last_status);
    if builtin.is_none() && cmd.redirects.is_empty() {
        // Let the standard library select the platform's process-spawn path.
        let mut external = Command::new(&cmd.words[0]);
        external.args(&cmd.words[1..]);
        if let Some(input) = input {
            external.stdin(Stdio::from(input));
        }
        if let Some(output) = output {
            external.stdout(Stdio::from(output));
        }
        return match external.spawn() {
            Ok(child) => Ok(Stage::Child(child.id() as libc::pid_t)),
            Err(e) => {
                let code = e.raw_os_error().unwrap_or(libc::EIO);
                if matches!(
                    code,
                    libc::EAGAIN | libc::ENOMEM | libc::EMFILE | libc::ENFILE
                ) {
                    return Err(e);
                }
                eprintln!("tonari: {}: {e}", cmd.words[0]);
                Ok(Stage::Failed(exec_status(code)))
            }
        };
    }

    // Opening a FIFO can block until a later pipeline stage starts. Do those
    // opens in a child so the parent can launch the remaining stages first.
    let mut setup = Vec::new();
    for (fd, dest) in [(input.as_ref(), 0), (output.as_ref(), 1)] {
        if let Some(fd) = fd {
            setup.push(Action::Dup(fd.as_raw_fd(), dest));
            setup.push(Action::Close(fd.as_raw_fd()));
        }
    }
    if let Some(unused) = next_reader {
        setup.push(Action::Close(unused.as_raw_fd()));
    }
    setup.extend(actions(cmd));
    let external = builtin.is_none().then(|| External::new(&cmd.words));
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        // All allocations happen before fork. Child paths end with exec or _exit.
        // Restore signals caught by the interactive shell before blocking on a file.
        for signal in [libc::SIGINT, libc::SIGQUIT] {
            unsafe {
                let mut old: libc::sigaction = std::mem::zeroed();
                libc::sigaction(signal, std::ptr::null(), &mut old);
                if old.sa_sigaction != libc::SIG_IGN {
                    libc::signal(signal, libc::SIG_DFL);
                }
            }
        }
        let code = if !apply(&setup) {
            1
        } else if let Some(external) = external.as_ref() {
            external.exec()
        } else {
            match eval_builtin(builtin.unwrap()) {
                Flow::Exit(s) | Flow::Continue(s) => s,
            }
        };
        unsafe { libc::_exit(code.into()) }
    }
    Ok(Stage::Child(pid))
}

fn wait(pid: libc::pid_t) -> io::Result<u8> {
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

pub fn run(pipeline: &Pipeline, last_status: u8, replace: bool) -> io::Result<Flow> {
    if let [cmd] = pipeline.as_slice() {
        if let Some(builtin) = builtin(cmd, last_status) {
            return run_builtin(builtin, &actions(cmd), replace);
        }
        if replace {
            // There is no more shell work after the final simple command of -c.
            let executable = External::new(&cmd.words);
            return Ok(Flow::Continue(if apply(&actions(cmd)) {
                executable.exec()
            } else {
                1
            }));
        }
    }

    let mut input = None;
    let mut stages = Vec::with_capacity(pipeline.len());
    for (i, cmd) in pipeline.iter().enumerate() {
        let stage = (|| {
            let incoming = input.take();
            let output = if i + 1 < pipeline.len() {
                let (reader, writer) = pipe()?;
                input = Some(reader);
                Some(writer)
            } else {
                None
            };
            start(cmd, incoming, output, input.as_ref(), last_status)
        })();
        match stage {
            Ok(stage) => stages.push(stage),
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
                return Err(e);
            }
        }
    }
    let mut status = 0;
    let mut wait_error = None;
    for stage in stages {
        match stage {
            Stage::Child(pid) => match wait(pid) {
                Ok(s) => status = s,
                Err(e) => wait_error = Some(e),
            },
            Stage::Failed(s) => status = s,
        }
    }
    if let Some(e) = wait_error {
        return Err(e);
    }
    Ok(Flow::Continue(status))
}
