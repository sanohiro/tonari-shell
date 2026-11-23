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
    // A literal read of an ordinary file has no substitutions to isolate.
    // Keep streams, virtual files, error reporting and trap execution in the child.
    if !session.traps.active()
        && !session.xtrace
        && let Some(path) = input_only(&body).and_then(parser::Word::as_literal)
        && std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
    {
        use std::os::unix::fs::OpenOptionsExt;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            && file.metadata().is_ok_and(|m| m.is_file() && m.len() > 0)
        {
            let mut output = Vec::new();
            if let Err(e) = file.read_to_end(&mut output) {
                eprintln!("tonari: {e}");
                return captured_output(output, 1);
            }
            return captured_output(output, 0);
        }
    }
    if let Some(result) = capture_simple(session, &body, last) {
        return result;
    }
    let (read, write) = std::io::pipe().map_err(|_| "cannot open command substitution pipe")?;
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err("cannot fork command substitution");
    }
    if pid == 0 {
        session
            .traps
            .after_fork(session.errtrace, session.functrace);
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
        session.subshell_depth += 1;
        crate::execute::coprocess::after_fork(session);
        session.jobs.forked(true);
        session.interactive = false;
        session.command_string = false;
        session.errexit &= session.shopt.inherit_errexit;
        if let Some(path) = input_only(&body) {
            let code = (|| {
                let captured = std::cell::Cell::new(None);
                let fields = crate::expansion::Expansion::new(session, last, &captured)
                    .fields(path.clone())
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
            Flow::Discard(code) | Flow::Continue(code) | Flow::Exit(code) | Flow::Return(code) => {
                code
            }
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
    if output.contains(&0) {
        output.retain(|&b| b != 0);
    }
    while output.last() == Some(&b'\n') {
        output.pop();
    }
    let value =
        String::from_utf8(output).map_err(|_| "command substitution produced invalid UTF-8")?;
    Ok((value, code))
}

// Validate the whole list before executing any builtin. A later function,
// redirect or effectful word must send the complete substitution through fork.
fn capture_simple(
    session: &mut Session,
    body: &[parser::Statement],
    last: u8,
) -> Option<Result<(String, u8), &'static str>> {
    use crate::word::{Part, Word, is_identifier};
    if body.is_empty()
        || session.xtrace
        || session.functrace && session.traps.debug_handler()
        || session.nounset
        || session.errexit && session.shopt.inherit_errexit
        || session.errtrace && session.traps.error_handler()
    {
        return None;
    }
    let command = |statement: &parser::Statement| {
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
            || session.disabled_builtins.contains(program)
        {
            return None;
        }
        match program {
            "printf" if body.len() > 1 => {
                // A dynamic format, -v or a %n conversion can assign a variable
                // used by the next command. Keep such lists in a real subshell.
                let fmt = args.first()?.as_literal()?;
                if fmt.starts_with('-') || (fmt.contains('%') && fmt.contains('n')) {
                    return None;
                }
            }
            "printf" | "echo" | ":" | "true" | "false" => {}
            _ if body.len() == 1 && crate::execute::can_capture_external(program) => {}
            _ => return None,
        }
        let safe = args.iter().all(|word| match word {
            Word::Literal(_) => true,
            Word::Expanded(parts) => parts.iter().all(|part| match part {
                Part::Text(_) => true,
                Part::Parameter { name, .. } => {
                    is_identifier(name)
                        && !matches!(
                            name.as_str(),
                            "BASHPID" | "BASH_SUBSHELL" | "RANDOM" | "SRANDOM" | "_" | "PIPESTATUS"
                        )
                }
                _ => false,
            }),
            Word::Declaration(_) => false,
        });
        safe.then_some(())
    };
    if !body.iter().all(|s| command(s).is_some()) {
        return None;
    }
    let mut output = Vec::new();
    let mut code = last;
    let mut fields = Vec::new();
    for statement in body {
        let parser::Command::Pipeline(stages) = &statement.command else {
            unreachable!()
        };
        let cmd = &stages[0];
        let name = cmd.words[0].as_literal().unwrap();
        if !matches!(name, "echo" | "printf" | ":" | "true" | "false") {
            return Some(crate::execute::capture_external(cmd.clone(), session, last));
        }
        let captured = std::cell::Cell::new(None);
        let mut env = crate::expansion::Expansion::new(session, code, &captured);
        fields.clear();
        for arg in &cmd.words[1..] {
            fields.extend(env.fields(arg.clone()).ok()?);
        }
        code = match name {
            "echo" => {
                crate::builtins::echo_into(&fields, &mut output);
                0
            }
            "printf" => crate::session::printf::run(session, &fields, Some(&mut output)),
            "true" | ":" => 0,
            "false" => 1,
            _ => unreachable!(),
        };
    }
    Some(captured_output(output, code))
}

fn input_only(body: &[parser::Statement]) -> Option<&parser::Word> {
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
        && cmd.compound.is_none()
        && cmd.words.is_empty()
        && cmd.assignments.is_empty()
        && !statement.negated)
        .then_some(&redirect.word)
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
        session
            .traps
            .after_fork(session.errtrace, session.functrace);
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
        session.subshell_depth += 1;
        crate::execute::coprocess::after_fork(session);
        session.jobs.forked(false);
        session.command_string = false;
        let flow = execute_program(session, body, status, true);
        let code = match flow {
            Flow::Discard(s) | Flow::Continue(s) | Flow::Return(s) | Flow::Exit(s) => s,
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
