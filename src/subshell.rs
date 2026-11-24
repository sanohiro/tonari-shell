use std::io::{self, Read};
use std::os::fd::{AsFd, AsRawFd, IntoRawFd};

use crate::{Flow, Session, execute_program, parser};

// A zero size on a virtual filesystem can describe a live stream. Keep its
// open and read in the child. Unknown filesystems take the same path.
fn regular_storage(query: impl FnOnce(&mut libc::statfs) -> libc::c_int) -> bool {
    let mut info = unsafe { std::mem::zeroed() };
    if query(&mut info) != 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    return matches!(
        info.f_type as _,
        libc::EXT4_SUPER_MAGIC
            | libc::XFS_SUPER_MAGIC
            | libc::BTRFS_SUPER_MAGIC
            | libc::TMPFS_MAGIC
            | libc::OVERLAYFS_SUPER_MAGIC
            | libc::F2FS_SUPER_MAGIC
    );
    #[cfg(target_os = "macos")]
    {
        let name = info.f_fstypename.map(|c| c as u8);
        return name.starts_with(b"apfs\0") || name.starts_with(b"hfs\0");
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    false
}

fn read_regular_file(file: &std::fs::File, size: u64, output: &mut Vec<u8>) -> io::Result<()> {
    // One extra byte distinguishes EOF from a stale size without growing a full buffer.
    const MAX_INITIAL_SIZE: usize = 64 * 1024 * 1024;
    output.try_reserve_exact(size.min(MAX_INITIAL_SIZE as u64) as usize + 1)?;
    loop {
        output.try_reserve(1)?;
        let tail = output.spare_capacity_mut();
        // read initializes only the returned bytes in this writable allocation.
        let n = unsafe {
            libc::read(
                file.as_raw_fd(),
                tail.as_mut_ptr().cast(),
                tail.len().min(MAX_INITIAL_SIZE + 1),
            )
        };
        if n == 0 {
            return Ok(());
        }
        if n < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        // The kernel initialized this prefix of the spare capacity.
        unsafe { output.set_len(output.len() + n as usize) };
    }
}

// The interpreter has one thread. A substitution inherits its variables and
// functions, changes its own cwd, and writes only stdout into the capture pipe.
pub fn capture_substitution(
    session: &mut Session,
    body: Vec<parser::Statement>,
    last: u8,
) -> Result<(String, u8), &'static str> {
    #[cfg(target_os = "linux")]
    let mut child_input = None;
    // A literal read of an ordinary file has no substitutions to isolate.
    // Keep streams, virtual files, error reporting and trap execution in the child.
    if !session.traps.active()
        && !session.xtrace
        && let Some(path) = input_only(&body).and_then(parser::Word::as_literal)
        && let Ok(before) = std::fs::metadata(path)
        && before.is_file()
    {
        use std::os::unix::fs::OpenOptionsExt;
        if before.len() == 0
            && !std::ffi::CString::new(path)
                .is_ok_and(|p| regular_storage(|info| unsafe { libc::statfs(p.as_ptr(), info) }))
        {
            // Resolve process-relative names such as /proc/self/stat in the
            // caller. O_PATH pins the object without opening a stream for I/O.
            #[cfg(target_os = "linux")]
            if let Ok(file) = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_PATH)
                .open(path)
            {
                child_input = crate::execute::private_fd(file.into()).ok();
            }
        } else if let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            && let Ok(metadata) = file.metadata()
            && metadata.is_file()
            && (metadata.len() > 0
                || regular_storage(|info| unsafe { libc::fstatfs(file.as_raw_fd(), info) }))
        {
            let mut output = Vec::new();
            if let Err(e) = read_regular_file(&file, metadata.len(), &mut output) {
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
                #[cfg(target_os = "linux")]
                let file = match &child_input {
                    Some(fd) => std::fs::File::open(format!("/proc/self/fd/{}", fd.as_raw_fd()))
                        .or_else(|_| std::fs::File::open(filename)),
                    None => std::fs::File::open(filename),
                };
                #[cfg(not(target_os = "linux"))]
                let file = std::fs::File::open(filename);
                let mut file =
                    file.map_err(|e| io::Error::new(e.kind(), format!("{filename}: {e}")))?;
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
    mut read: impl Read + std::os::fd::AsRawFd,
    pid: libc::pid_t,
) -> Result<(String, u8), &'static str> {
    let mut output = Vec::new();
    let result = if crate::editor::preview::can_wait() {
        crate::editor::preview::read_capture(&mut read, &mut output)
    } else {
        read.read_to_end(&mut output).map(|_| ())
    };
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
    // An assigned capture can outlive the command. Release large reservations
    // when a short read, discarded NULs or trailing newlines left little text.
    if output.capacity() > 8192 && output.len() < output.capacity() / 4 {
        output.shrink_to_fit();
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
        let external = match program {
            "jobs"
                if body.len() == 1
                    && session.jobs.is_empty()
                    && !session.traps.has_explicit_traps()
                    && (args.is_empty()
                        || matches!(args, [flag] if flag.as_literal() == Some("-p"))) =>
            {
                false
            }
            "jobs" => return None,
            "printf" if body.len() > 1 => {
                // A dynamic format, -v or a %n conversion can assign a variable
                // used by the next command. Keep such lists in a real subshell.
                let fmt = args.first()?.as_literal()?;
                if fmt.starts_with('-') || (fmt.contains('%') && fmt.contains('n')) {
                    return None;
                }
                false
            }
            "printf" | "echo" | ":" | "true" | "false" => false,
            // A child drops monitor mode and normally clears errexit. An exported
            // SHELLOPTS lets even a literal external command observe those changes.
            _ if body.len() == 1
                && crate::execute::can_capture_external(program)
                && !((session.errexit || session.jobs.monitor)
                    && session.is_exported("SHELLOPTS")) =>
            {
                true
            }
            _ => return None,
        };
        let safe = args.iter().all(|word| match word {
            Word::Literal(_) => true,
            Word::Expanded(parts) => parts.iter().all(|part| match part {
                Part::Text(_) => true,
                Part::Parameter { name, quoted } => {
                    if let Some(base) = name.strip_suffix("[@]") {
                        return external
                            && *quoted
                            && is_identifier(base)
                            && session.stored_array(base);
                    }
                    is_identifier(name)
                        && !session.is_nameref(name)
                        && !matches!(
                            name.as_str(),
                            "BASHPID"
                                | "BASH_SUBSHELL"
                                | "BASH_COMMAND"
                                | "LINENO"
                                | "SHELLOPTS"
                                | "RANDOM"
                                | "SRANDOM"
                                | "_"
                                | "PIPESTATUS"
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
        if name == "jobs" {
            return Some(Ok((String::new(), 0)));
        }
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
    launch_list(session, background, |s| {
        execute_program(s, body, status, true)
    })
}

pub(crate) fn launch_borrowed(
    session: &mut Session,
    body: &[parser::Statement],
    status: u8,
) -> Flow {
    launch_list(session, false, |s| {
        crate::eval::borrowed_program(s, body, status, true)
    })
}

fn launch_list(
    session: &mut Session,
    background: bool,
    run: impl FnOnce(&mut Session) -> Flow,
) -> Flow {
    let mut launch = session.jobs.launch(background, false);
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
        let flow = run(session);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::OwnedFd;

    #[test]
    fn capture_uses_eof_when_the_size_hint_is_stale() {
        for hint in [1, 5, 4096] {
            let (reader, mut writer) = io::pipe().unwrap();
            writer.write_all(b"hello").unwrap();
            drop(writer);
            let mut bytes = Vec::new();
            read_regular_file(
                &std::fs::File::from(OwnedFd::from(reader)),
                hint,
                &mut bytes,
            )
            .unwrap();
            assert_eq!(bytes, b"hello");
        }
    }

    #[test]
    fn trimmed_capture_releases_the_large_input_buffer() {
        let mut bytes = vec![b'\n'; 1024 * 1024];
        bytes[0] = b'x';
        let (text, _) = captured_output(bytes, 0).unwrap();
        assert_eq!(text, "x");
        assert!(text.capacity() < 8192);
    }
}
