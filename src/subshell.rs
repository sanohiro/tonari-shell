use std::io::Read;
use std::os::fd::{AsFd, AsRawFd, IntoRawFd};

use crate::{Flow, Session, execute_program, parser};

// The interpreter has one thread. A substitution inherits its variables and
// functions, changes its own cwd, and writes only stdout into the capture pipe.
pub fn capture_substitution(
    session: &Session,
    body: Vec<parser::Statement>,
    last: u8,
) -> Result<(String, u8), &'static str> {
    let (mut read, write) = std::io::pipe().map_err(|_| "cannot open command substitution pipe")?;
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err("cannot fork command substitution");
    }
    if pid == 0 {
        drop(read);
        if write.as_raw_fd() == 1 {
            let fd = write.into_raw_fd();
            if unsafe { libc::fcntl(fd, libc::F_SETFD, 0) } < 0 {
                unsafe { libc::_exit(1) }
            }
        } else {
            if unsafe { libc::dup2(write.as_raw_fd(), 1) } < 0 {
                unsafe { libc::_exit(1) }
            }
            drop(write);
        }
        let mut child = session.clone();
        child.jobs.detached();
        child.interactive = false;
        child.command_string = false;
        child.errexit = false;
        if let Some(path) = input_only(&body) {
            let code = (|| {
                let captured = std::cell::Cell::new(None);
                let fields = crate::expansion::Expansion::new(&mut child, last, &captured)
                    .fields(path)
                    .map_err(std::io::Error::other)?;
                let [filename] = fields.as_slice() else {
                    return Err(std::io::Error::other("ambiguous redirect"));
                };
                let mut file = std::fs::File::open(filename)
                    .map_err(|e| std::io::Error::new(e.kind(), format!("{filename}: {e}")))?;
                // Write straight to fd 1. Stdout's line buffer could retain the
                // last bytes across _exit when the file has no final newline.
                let mut stdout =
                    std::fs::File::from(std::io::stdout().as_fd().try_clone_to_owned()?);
                std::io::copy(&mut file, &mut stdout)?;
                Ok::<_, std::io::Error>(())
            })();
            if let Err(e) = &code {
                eprintln!("tonari: {e}");
            }
            unsafe { libc::_exit(i32::from(code.is_err())) }
        }
        let code = match execute_program(&mut child, body, last, true) {
            Flow::Continue(code) | Flow::Exit(code) | Flow::Return(code) => code,
            Flow::Break(_) | Flow::LoopContinue(_) => 1,
        };
        unsafe {
            libc::_exit(code as libc::c_int);
        }
    }
    // A writer can fill the pipe before it exits. Drain first, then reap even
    // when reading fails. The parent must drop its own writer to observe EOF.
    drop(write);
    let mut output = Vec::new();
    let result = read.read_to_end(&mut output);
    drop(read);
    let code = wait(pid)?;
    result.map_err(|_| "cannot read command substitution")?;
    output.retain(|&b| b != 0);
    while output.last() == Some(&b'\n') {
        output.pop();
    }
    let value =
        String::from_utf8(output).map_err(|_| "command substitution produced invalid UTF-8")?;
    Ok((value, code))
}

fn input_only(body: &[parser::Statement]) -> Option<parser::Word> {
    let [statement] = body else { return None };
    let parser::Command::Pipeline(stages) = &statement.command else {
        return None;
    };
    let [cmd] = stages.as_slice() else {
        return None;
    };
    let [redirect] = cmd.redirects.as_slice() else {
        return None;
    };
    (redirect.fd == 0
        && redirect.target == parser::Target::Read
        && cmd.words.is_empty()
        && cmd.assignments.is_empty()
        && !statement.negated)
        .then(|| redirect.word.clone())
}

fn wait(pid: libc::pid_t) -> Result<u8, &'static str> {
    let mut status = 0;
    loop {
        if unsafe { libc::waitpid(pid, &mut status, 0) } == pid {
            break;
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err("cannot wait for command substitution");
        }
    }
    let code = if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        libc::WEXITSTATUS(status)
    };
    Ok(code as u8)
}

pub fn launch_group(
    session: &mut Session,
    body: Vec<parser::Statement>,
    status: u8,
    background: bool,
) -> Flow {
    let mut launch = session.jobs.launch(background);
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        eprintln!("tonari: fork: {}", std::io::Error::last_os_error());
        return Flow::Continue(1);
    }
    if pid == 0 {
        launch.child();
        if background && launch.group.is_none() {
            let fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
            if fd < 0 || unsafe { libc::dup2(fd, 0) } < 0 {
                unsafe { libc::_exit(1) }
            }
            if fd != 0 {
                unsafe {
                    libc::close(fd);
                }
            }
        }
        session.jobs.detached();
        session.command_string = false;
        let flow = execute_program(session, body, status, true);
        let code = match flow {
            Flow::Continue(s) | Flow::Return(s) | Flow::Exit(s) => s,
            _ => 0,
        };
        unsafe { libc::_exit(code as libc::c_int) }
    }
    launch.parent(pid);
    if launch.group.is_none() && !background {
        return Flow::Continue(wait(pid).unwrap_or(1));
    }
    match session
        .jobs
        .finish(launch, vec![Ok(pid)], "command list".into(), false)
    {
        Ok((code, pid)) => {
            if background {
                session.last_background = pid;
            }
            Flow::Continue(code)
        }
        Err(e) => {
            eprintln!("tonari: {e}");
            Flow::Continue(1)
        }
    }
}
