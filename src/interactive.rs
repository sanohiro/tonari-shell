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

pub(super) fn terminal_read(buf: &mut [u8], raw: bool, terminal: bool) -> io::Result<usize> {
    unsafe {
        let mut blocked = std::mem::zeroed();
        libc::sigfillset(&mut blocked);
        let original = traps::SignalMask::block(&blocked)?;
        if INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed)
            || traps::input_interrupted()
            || raw && editor::RESIZED.load(std::sync::atomic::Ordering::Relaxed)
        {
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
        if raw || !terminal {
            // Another reader may consume a pipe between pselect and read.
            // Keep a blocking nonterminal read interruptible by the shell's traps.
            if !terminal {
                drop(original);
            }
            let n = libc::read(0, buf.as_mut_ptr().cast(), buf.len());
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            if raw && n == 0 {
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
        let n = match terminal_read(&mut buf[..size], false, terminal) {
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
    interactive_loop(session, status, editing, terminal_input)
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
    let mut incomplete = None;
    let mut history_quote = None;
    let mut here_doc: Option<parser::HereDocHint> = None;
    let mut paste = String::new();
    let mut paste_offset = 0;
    let mut paste_recorded = false;
    loop {
        if source.is_empty() {
            incomplete = None;
            history_quote = None;
            here_doc = None;
        }
        let start = source.len();
        interrupted();
        if let Flow::Exit(code) = traps::dispatch(session, status) {
            return Ok(code);
        }
        session.jobs.poll();
        session.jobs.report_foreground_notices();
        if session.jobs.monitor {
            session.jobs.report_changes(session.posix);
        }
        let queued = paste_offset < paste.len();
        if !queued && source.is_empty() {
            paste.clear();
            paste_offset = 0;
            paste_recorded = false;
        }
        let terminal_size = if queued {
            None
        } else {
            session.refresh_terminal_size()
        };
        if source.is_empty() && !queued {
            for hook in session.array_values("PROMPT_COMMAND") {
                if !hook.is_empty()
                    && let Flow::Exit(code) = eval_text(session, &hook, status)
                {
                    return Ok(code);
                }
            }
        }
        let line_editing = editing && (session.emacs || session.vi);
        let prompt = if queued {
            String::new()
        } else {
            prompt::render(session, !source.is_empty(), status, line_editing)
        };
        let timeout = session
            .scalar_setting("TMOUT")
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|&n| n > 0);
        let deadline = input_deadline(timeout.map(std::time::Duration::from_secs));
        let result = if queued {
            let rest = &paste[paste_offset..];
            let size = memchr::memchr(b'\n', rest.as_bytes()).map_or(rest.len(), |i| i + 1);
            source.extend_from_slice(&rest.as_bytes()[..size]);
            paste_offset += size;
            Ok(Some(true))
        } else if line_editing {
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
                    status = flow.child_status();
                    continue;
                }
                Ok(editor::Input::Line(line)) => {
                    if let Some(i) = memchr::memchr(b'\n', line.as_bytes())
                        && i + 1 < line.len()
                    {
                        source.extend_from_slice(&line.as_bytes()[..=i]);
                        paste = line;
                        paste_offset = i + 1;
                    } else {
                        paste.clear();
                        paste_offset = 0;
                        source.extend_from_slice(line.as_bytes());
                    }
                    Ok(Some(true))
                }
                Ok(editor::Input::Directory(result)) => {
                    status = match result {
                        Ok(()) => 0,
                        Err(error) => {
                            diagnostic!("cd: {}", builtins::os_message(&error));
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
            let ready = if session.jobs.notify && terminal_input {
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
                stderr_line!();
                if session.traps.read_pending() {
                    continue;
                }
                source.clear();
                if traps::pending().is_none() {
                    status = 130;
                }
                continue;
            }
            Err(e) => return Err(e),
        };
        if more && !queued {
            // The terminal can be resized while this line is being edited.
            session.refresh_terminal_size();
        }
        if !more && !source.is_empty() && session.ignoreeof && eof_count < session.ignoreeof_limit {
            diagnostic!("unexpected end of file");
            source.clear();
            status = 2;
        }
        if !more && source.is_empty() {
            if session.ignoreeof && eof_count < session.ignoreeof_limit {
                eof_count += 1;
                stderr_line!(
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
        let line = std::str::from_utf8(&source[start..]);
        if more && let Ok(line) = line {
            if let Some(hint) = &mut here_doc {
                if !hint.may_finish(line) {
                    continue;
                }
                here_doc = None;
            } else if session.histexpand {
                match history::expand::expand_line(session, line, history_quote) {
                    Ok(event) if event.print => {
                        stderr_text!("{}", event.text);
                        session.begin_history_line(&event.text);
                        session.finish_history_line();
                        source.clear();
                        continue;
                    }
                    Ok(event) => {
                        if let std::borrow::Cow::Owned(expanded) = event.text {
                            if session.shopt.histverify && line_editing {
                                editable_history = expanded.trim_end_matches('\n').to_owned();
                                source.truncate(start);
                                continue;
                            }
                            stderr_text!("{}", expanded);
                            source.truncate(start);
                            source.extend_from_slice(expanded.as_bytes());
                        }
                    }
                    Err(error) => {
                        diagnostic!("{error}");
                        if session.shopt.histreedit && line_editing {
                            editable_history = line.trim_end_matches('\n').to_owned();
                        }
                        source.truncate(start);
                        status = 1;
                        continue;
                    }
                }
            }
        }
        if more
            && incomplete.is_some()
            && let Ok(line) = std::str::from_utf8(&source[start..])
            && input::still_open(incomplete, line, session)
        {
            continue;
        }
        let text = std::str::from_utf8(&source);
        match text {
            Ok(text) => match parser::session_source(text, !more, session).or_else(|problem| {
                if !more
                    || !problem.incomplete()
                    || paste_offset == 0
                    || paste_offset == paste.len()
                    || start != 0
                {
                    return Err(problem);
                }
                let rest = &paste[paste_offset..];
                // Keep history expansion on its physical-line path whenever the
                // candidate unit contains a history or quick-substitution marker.
                let chars = session.scalar_setting("histchars");
                let parse = |size| {
                    let extra = &rest[..size];
                    if session.histexpand
                        && chars
                            .as_deref()
                            .unwrap_or("!^#")
                            .chars()
                            .take(2)
                            .any(|c| extra.contains(c))
                    {
                        return Err(parser::Error::Invalid(
                            "requires line-by-line history expansion",
                        ));
                    }
                    let mut unit = String::with_capacity(text.len() + size);
                    unit.push_str(text);
                    unit.push_str(extra);
                    parser::session_source(&unit, false, session)
                };
                match input::buffered_compound(
                    text,
                    rest.as_bytes(),
                    true,
                    session.reads_aliases(),
                    parse,
                ) {
                    Ok((program, used, _)) => {
                        paste_offset += used;
                        Ok(program)
                    }
                    Err(_) => Err(problem),
                }
            }) {
                Ok(program) if program.is_empty() => {}
                Ok(program) => {
                    if let Some(p) = session.get("PS0").filter(|p| !p.is_empty()) {
                        stderr_text!("{}", prompt::render_text(session, p, status));
                    }
                    session.command_number += 1;
                    let record = if paste_offset != 0 {
                        if paste_recorded {
                            false
                        } else {
                            paste_recorded = true;
                            session.begin_history_line(&paste)
                        }
                    } else {
                        session.begin_history_line(text)
                    };
                    let cwd = if record { session.get("PWD") } else { None };
                    let flow = if session.noexec {
                        Flow::Continue(0)
                    } else {
                        execute_program(session, program, status, false)
                    };
                    status = match flow {
                        Flow::Unwind(s)
                        | Flow::Discard(s)
                        | Flow::Continue(s)
                        | Flow::Return(s)
                        | Flow::Exit(s) => s,
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
                Err(e) if more && e.incomplete() => {
                    history_quote = e.pending_quote();
                    match e {
                        parser::Error::Incomplete(reason) => incomplete = Some(reason),
                        parser::Error::HereDoc(hint) => here_doc = Some(hint),
                        _ => {}
                    }
                    continue;
                }
                Err(e) => {
                    diagnostic!("{}", e.message());
                    status = 2;
                    if !more && !paste_recorded && session.begin_history_line(text) {
                        session.persist_current_history(
                            session.get("PWD").as_deref(),
                            status,
                            &mut history,
                        );
                        session.finish_history_line();
                    }
                }
            },
            Err(_) => {
                diagnostic!("expected UTF-8 command text");
                status = 2;
            }
        }
        if !more && !startup::defer_interactive_exit(session) {
            return Ok(startup::logout(session, status, status));
        }
        source.clear();
    }
}

pub(super) fn shell_terminal() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}
