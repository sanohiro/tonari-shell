use super::*;

pub(crate) fn terminal_size(fd: std::os::fd::RawFd) -> Option<(u16, u16)> {
    let mut size = std::mem::MaybeUninit::<libc::winsize>::uninit();
    // SAFETY: TIOCGWINSZ initializes winsize on success.
    let size = unsafe {
        if libc::ioctl(fd, libc::TIOCGWINSZ, size.as_mut_ptr()) < 0 {
            return None;
        }
        size.assume_init()
    };
    Some((size.ws_row, size.ws_col))
}

pub(super) fn terminal_read(buf: &mut [u8], raw: bool) -> io::Result<usize> {
    unsafe {
        let mut blocked = std::mem::zeroed();
        libc::sigfillset(&mut blocked);
        let original = traps::SignalMask::block(&blocked)?;
        if INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed) || traps::input_interrupted() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let mut read_set = std::mem::zeroed();
        libc::FD_ZERO(&mut read_set);
        libc::FD_SET(0, &mut read_set);
        let timeout = input_timeout();
        let selected = libc::pselect(
            1,
            &mut read_set,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            timeout
                .as_ref()
                .map_or(std::ptr::null(), std::ptr::from_ref),
            &original.0,
        );
        if selected == 0 {
            return Err(io::ErrorKind::TimedOut.into());
        }
        if selected < 0 {
            return Err(io::Error::last_os_error());
        }
        // A terminal interrupt can flush even bytes already reported readable.
        // VMIN=0 makes the editor read nonblocking after pselect without modifying
        // the open file description shared with external foreground programs.
        if raw {
            let n = libc::read(0, buf.as_mut_ptr().cast(), buf.len());
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            if n == 0 {
                let mut p = libc::pollfd {
                    fd: 0,
                    events: libc::POLLIN,
                    revents: 0,
                };
                if libc::poll(&mut p, 1, 0) >= 0 && p.revents & (libc::POLLHUP | libc::POLLERR) == 0
                {
                    return Err(io::ErrorKind::WouldBlock.into());
                }
            }
            return Ok(n as usize);
        }
        let flags = libc::fcntl(0, libc::F_GETFL);
        if flags < 0 || libc::fcntl(0, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
        let n = libc::read(0, buf.as_mut_ptr().cast(), buf.len());
        let read = if n >= 0 {
            Ok(n as usize)
        } else {
            Err(io::Error::last_os_error())
        };
        if libc::fcntl(0, libc::F_SETFL, flags) < 0 {
            return Err(io::Error::last_os_error());
        }
        read
    }
}

fn read_terminal_line(source: &mut Vec<u8>, terminal: bool) -> io::Result<bool> {
    let seekable = !terminal && unsafe { libc::lseek(0, 0, libc::SEEK_CUR) } >= 0;
    let mut buf = [0u8; 4096];
    loop {
        let size = if terminal || seekable { buf.len() } else { 1 };
        let n = match terminal_read(&mut buf[..size], false) {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
            result => result?,
        };
        if n == 0 {
            return Ok(false);
        }
        let mut used = n;
        if seekable && let Some(end) = memchr::memchr(b'\n', &buf[..n]) {
            used = end + 1;
            if used < n
                && unsafe { libc::lseek(0, -((n - used) as libc::off_t), libc::SEEK_CUR) } < 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        source.extend_from_slice(&buf[..used]);
        if source.ends_with(b"\n") {
            return Ok(true);
        }
    }
}

pub(super) fn interactive(session: &mut Session, status: u8, editing: bool) -> io::Result<u8> {
    // Startup files may have replaced either descriptor or reassigned TERM.
    let terminal_input = io::stdin().is_terminal();
    let editing = editing
        && terminal_input
        && io::stderr().is_terminal()
        && session
            .scalar_setting("TERM")
            .is_some_and(|term| term != "dumb");
    history::start_interactive(session);
    let result = interactive_loop(session, status, editing, terminal_input);
    history::finish_interactive(session);
    result
}

pub(super) fn interactive_loop(
    session: &mut Session,
    mut status: u8,
    editing: bool,
    terminal_input: bool,
) -> io::Result<u8> {
    let mut source = Vec::new();
    let mut history = history::History::default();
    let mut eof_count = 0;
    let mut editable_history = String::new();
    loop {
        interrupted();
        if let Flow::Exit(code) = traps::dispatch(session, status) {
            return Ok(code);
        }
        session.jobs.poll();
        session.jobs.report_foreground();
        if (session.notify || session.posix) && session.jobs.monitor {
            session.jobs.report_changes(session.posix);
        }
        let terminal_size = session.refresh_terminal_size();
        if source.is_empty() {
            for hook in session.array_values("PROMPT_COMMAND") {
                if !hook.is_empty()
                    && let Flow::Exit(code) = eval_text(session, &hook, status)
                {
                    return Ok(code);
                }
            }
        }
        let line_editing = editing && (session.emacs || session.vi);
        let prompt = prompt::render(session, !source.is_empty(), status, line_editing);
        let timeout = session
            .get("TMOUT")
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|&n| n > 0);
        let deadline = input_deadline(timeout.map(std::time::Duration::from_secs));
        let result = if line_editing {
            match editor::line(
                &prompt,
                &mut history,
                session,
                status,
                editor::Start::Command(std::mem::take(&mut editable_history)),
                source.is_empty(),
                terminal_size,
            ) {
                Ok(editor::Input::Control(Flow::Exit(code))) => return Ok(code),
                Ok(editor::Input::Control(flow)) => {
                    source.clear();
                    status = flow.child_status();
                    continue;
                }
                Ok(editor::Input::Line(line)) => {
                    source.extend_from_slice(line.as_bytes());
                    Ok(Some(true))
                }
                Ok(editor::Input::Directory(result)) => {
                    status = match result {
                        Ok(()) => 0,
                        Err(error) => {
                            diagnostic!("cd: {error}");
                            1
                        }
                    };
                    continue;
                }
                Ok(editor::Input::End) => Ok(Some(false)),
                Ok(editor::Input::Cancel) => Ok(None),
                Err(error) => Err(error),
            }
        } else {
            let mut stderr = io::stderr().lock();
            stderr.write_all(prompt.as_bytes())?;
            stderr.flush()?;
            drop(stderr);
            let ready = if session.notify && terminal_input {
                wait_for_job_notice(&mut session.jobs, |jobs| {
                    let mut out = io::stderr().lock();
                    writeln!(out)?;
                    jobs.report_changes(session.posix);
                    out.write_all(prompt.as_bytes())?;
                    out.flush()
                })
            } else {
                Ok(())
            };
            ready
                .and_then(|()| read_terminal_line(&mut source, terminal_input))
                .map(Some)
        };
        drop(deadline);
        let more = match result {
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                diagnostic!("timed out waiting for input");
                if startup::defer_interactive_exit(session) {
                    continue;
                }
                return Ok(startup::logout(session, status, status));
            }
            Ok(Some(more)) => more,
            Ok(None) => {
                source.clear();
                if traps::pending().is_none() && session.traps.catches(libc::SIGINT) {
                    unsafe { libc::raise(libc::SIGINT) };
                }
                if traps::pending().is_none() {
                    status = 130;
                }
                continue;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                eprintln!();
                source.clear();
                if traps::pending().is_none() {
                    status = 130;
                }
                continue;
            }
            Err(e) => return Err(e),
        };
        if more {
            // The terminal can be resized while this line is being edited.
            session.refresh_terminal_size();
        }
        if !more && source.is_empty() {
            if session.ignoreeof && eof_count < session.ignoreeof_limit {
                eof_count += 1;
                eprintln!(
                    "Use \"{}\" to leave the shell.",
                    if session.login { "logout" } else { "exit" }
                );
                continue;
            }
            if startup::defer_interactive_exit(session) {
                continue;
            }
            return Ok(startup::logout(session, status, status));
        }
        eof_count = 0;
        let expanded;
        let text = std::str::from_utf8(&source);
        let text = if session.histexpand
            && let Ok(line) = text
        {
            match history::expand::expand(session, line) {
                Ok(event) => {
                    expanded = event.text;
                    if event.print {
                        eprint!("{}", expanded);
                        session.begin_history_line(&expanded);
                        session.finish_history_line();
                        source.clear();
                        continue;
                    }
                    if matches!(expanded, std::borrow::Cow::Owned(_)) {
                        if session.shopt.histverify && editing && (session.vi || session.emacs) {
                            editable_history = expanded.into_owned();
                            if editable_history.ends_with('\n') {
                                editable_history.pop();
                            }
                            source.clear();
                            continue;
                        }
                        eprint!("{}", expanded);
                    }
                    Ok(expanded.as_ref())
                }
                Err(e) => {
                    diagnostic!("{e}");
                    if session.shopt.histreedit && editing && (session.vi || session.emacs) {
                        editable_history = line.strip_suffix('\n').unwrap_or(line).to_owned();
                    }
                    source.clear();
                    status = 1;
                    continue;
                }
            }
        } else {
            text
        };
        match text {
            Ok(text) => match parser::session_source(text, !more, session) {
                Ok(program) if program.is_empty() => {}
                Ok(program) => {
                    if let Some(p) = session.get("PS0").filter(|p| !p.is_empty()) {
                        eprint!("{}", prompt::render_text(session, p, status));
                    }
                    session.command_number += 1;
                    let record = session.begin_history_line(text);
                    let cwd = if record { session.get("PWD") } else { None };
                    let flow = if session.noexec {
                        Flow::Continue(0)
                    } else {
                        execute_program(session, program, status, false)
                    };
                    status = match flow {
                        Flow::Discard(s) | Flow::Continue(s) | Flow::Return(s) | Flow::Exit(s) => s,
                        _ => 1,
                    };
                    if record {
                        session.persist_current_history(cwd.as_deref(), status, &mut history);
                        session.finish_history_line();
                    }
                    if session.onecmd || matches!(flow, Flow::Exit(_) | Flow::Return(_)) {
                        return Ok(status);
                    }
                }
                Err(e) if more && e.incomplete() => continue,
                Err(e) => {
                    diagnostic!("{}", e.message());
                    status = 2;
                }
            },
            Err(_) => {
                diagnostic!("expected UTF-8 command text");
                status = 2;
            }
        }
        if !more {
            return Ok(status);
        }
        source.clear();
    }
}

pub(super) fn shell_terminal() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}
