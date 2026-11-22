use std::io::Read;
use std::os::fd::{AsFd, AsRawFd, IntoRawFd};

use crate::{Flow, Session, execute_program, parser};

// The interpreter has one thread. A substitution inherits its variables and
// functions, changes its own cwd, and writes only stdout into the capture pipe.
pub fn capture_substitution(
    session: &mut Session,
    body: Vec<parser::Statement>,
    last: u8,
) -> Result<(String, u8), &'static str> {
    if let Some(result) = capture_simple(session, &body, last) {
        return result;
    }
    let (read, write) = std::io::pipe().map_err(|_| "cannot open command substitution pipe")?;
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err("cannot fork command substitution");
    }
    if pid == 0 {
        session.traps.after_fork();
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
        // fork already isolated the complete session, including shell variables.
        session.jobs.detached();
        session.interactive = false;
        session.command_string = false;
        session.errexit = false;
        if let Some(path) = input_only(&body) {
            let code = (|| {
                let captured = std::cell::Cell::new(None);
                let fields = crate::expansion::Expansion::new(session, last, &captured)
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
        let code = match execute_program(session, body, last, true) {
            Flow::Continue(code) | Flow::Exit(code) | Flow::Return(code) => code,
            Flow::Break(_) | Flow::LoopContinue(_) => 1,
        };
        let code = crate::traps::finish(session, code);
        unsafe {
            libc::_exit(code as libc::c_int);
        }
    }
    // A writer can fill the pipe before it exits. Drain first, then reap even
    // when reading fails. The parent must drop its own writer to observe EOF.
    drop(write);
    read_output(read, pid)
}

pub(super) fn read_output(
    mut read: impl Read,
    pid: libc::pid_t,
) -> Result<(String, u8), &'static str> {
    let mut output = Vec::new();
    let result = read.read_to_end(&mut output);
    drop(read);
    let code = crate::execute::wait(pid).map_err(|_| "cannot wait for command substitution")?;
    result.map_err(|_| "cannot read command substitution")?;
    captured_output(output, code)
}

fn captured_output(mut output: Vec<u8>, code: u8) -> Result<(String, u8), &'static str> {
    output.retain(|&b| b != 0);
    while output.last() == Some(&b'\n') {
        output.pop();
    }
    let value =
        String::from_utf8(output).map_err(|_| "command substitution produced invalid UTF-8")?;
    Ok((value, code))
}

// Only expand words whose value is independent of a new process identity.
// Functions, redirects, nested substitutions and effectful expansions use the
// ordinary subshell. printf still has a private variable scope for %n and -v.
fn capture_simple(
    session: &mut Session,
    body: &[parser::Statement],
    last: u8,
) -> Option<Result<(String, u8), &'static str>> {
    use crate::word::{Part, Word, is_identifier};
    let [statement] = body else { return None };
    let parser::Command::Pipeline(stages) = &statement.command else {
        return None;
    };
    let [cmd] = stages.as_slice() else {
        return None;
    };
    let (program, args) = cmd.words.split_first()?;
    let program = program.as_literal()?;
    if statement.negated
        || statement.condition != parser::Condition::Always
        || cmd.compound.is_some()
        || !cmd.redirects.is_empty()
        || !cmd.assignments.is_empty()
        || session.functions.contains_key(program)
        || (program != "printf" && !crate::execute::can_capture_external(program))
    {
        return None;
    }
    if !args.iter().all(|word| match word {
        Word::Literal(_) => true,
        Word::Expanded(parts) => parts.iter().all(|part| match part {
            Part::Text(_) => true,
            Part::Parameter { name, .. } => {
                is_identifier(name)
                    && !matches!(
                        name.as_str(),
                        "BASHPID" | "BASH_SUBSHELL" | "RANDOM" | "SRANDOM"
                    )
            }
            _ => false,
        }),
        Word::Declaration(_) => false,
    }) {
        return None;
    }
    if program != "printf" {
        // Only ordinary reads reached this point. The external process supplies
        // isolation itself. nounset errors still need the subshell's option context.
        return (!session.nounset)
            .then(|| crate::execute::capture_external(cmd.clone(), session, last));
    }
    let status = std::cell::Cell::new(None);
    let mut expansion = crate::expansion::Expansion::new(session, last, &status);
    let mut fields = Vec::with_capacity(args.len());
    for arg in args {
        fields.extend(expansion.fields(arg.clone()).ok()?);
    }
    let mut output = Vec::new();
    let code = crate::session::printf::run(session, &fields, Some(&mut output));
    Some(captured_output(output, code))
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
        session.traps.after_fork();
        launch.child();
        if background && launch.group.is_none() && !session.stdin_redirected {
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
        let code = crate::traps::finish(session, code);
        unsafe { libc::_exit(code as libc::c_int) }
    }
    launch.parent(pid);
    if launch.group.is_none() && !background {
        return Flow::Continue(crate::execute::wait(pid).unwrap_or(1));
    }
    match session.jobs.finish(
        launch,
        vec![Ok(pid)],
        "command list".into(),
        false,
        &mut session.pipe_status,
    ) {
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
