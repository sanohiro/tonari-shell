// Named descriptors have effects while redirections are still being expanded.
// For example, the second redirect in 'exec {out}>file 3>&$out' sees the first.
use super::*;
use std::os::fd::RawFd;

#[derive(Default)]
struct Scope {
    restore: Vec<(RawFd, SavedFd)>,
    changed: Vec<RawFd>,
    isolated: bool,
}

impl Scope {
    fn redirect(
        &mut self,
        session: &mut Session,
        mut raw: Redirect<Word>,
        status: u8,
        captured: &Cell<Option<u8>>,
    ) -> io::Result<bool> {
        let variable = raw.variable.take();
        let mut exp = Expansion::new(session, status, captured);
        let expanded = prepare_redirects([raw], &mut exp).map_err(io::Error::other)?;
        for (i, mut r) in expanded.into_iter().enumerate() {
            let name = variable.as_deref().filter(|_| i == 0);
            let close = r.target == Target::Duplicate && r.word == "-";
            if let Some(name) = name {
                if close {
                    let element = Expansion::new(session, status, captured)
                        .assignment_element(name)
                        .map_err(io::Error::other)?;
                    let value = match element {
                        Some(element) => element.value(session),
                        None => session.get(name),
                    };
                    r.fd = value
                        .and_then(|s| s.parse::<i32>().ok())
                        .filter(|&fd| fd >= 0)
                        .ok_or_else(|| io::Error::other(format!("{name}: ambiguous redirect")))?;
                } else {
                    if session.readonly(name) {
                        diagnostic!("{name}: readonly variable");
                        return Err(io::Error::other(format!(
                            "{name}: cannot assign fd to variable"
                        )));
                    }
                    // Open first so /dev/fd and duplicate sources see the original table.
                    let source;
                    let action = action(&r);
                    protect_fds(std::slice::from_ref(&action))?;
                    let fd = match action {
                        Action::Open(_, path, flags) => {
                            let Some(opened) = open_redirect(path, flags) else {
                                return Ok(false);
                            };
                            source = opened;
                            source.as_raw_fd()
                        }
                        Action::HereInput(_, text) => {
                            source = here_input::open(text, libc::PIPE_BUF, session)?;
                            source.as_raw_fd()
                        }
                        Action::Dup(fd, _) => {
                            if fd::is_private(fd) {
                                return Err(io::Error::other(format!("{fd}: Bad file descriptor")));
                            }
                            fd
                        }
                        _ => unreachable!(),
                    };
                    let allocated =
                        unsafe { libc::fcntl(fd, libc::F_DUPFD, crate::execute::fd::SHELL_FD_MIN) };
                    if allocated < 0 {
                        let error = io::Error::last_os_error();
                        return Err(io::Error::other(format!(
                            "{fd}: {}",
                            crate::builtins::os_message(&error)
                        )));
                    }
                    if let Err(error) = session.assign_target(name, &allocated.to_string()) {
                        unsafe {
                            libc::close(allocated);
                        }
                        return Err(io::Error::other(error));
                    }
                    if self.isolated || session.shopt.varredir_close {
                        self.restore.push((allocated, SavedFd::Closed));
                    }
                    continue;
                }
            }
            let action = action(&r);
            let actions = std::slice::from_ref(&action);
            protect_fds(actions)?;
            if (name.is_none() || close) && !self.restore.iter().any(|(n, _)| *n == r.fd) {
                self.restore
                    .push((r.fd, SavedFd::capture(&action, actions)?));
            }
            if apply_protected(actions, libc::PIPE_BUF, session).is_err() {
                return Ok(false);
            }
            match action {
                Action::Dup(from, to) if from == to => {}
                _ => self.changed.push(r.fd),
            }
            session.stdin_redirected |= r.fd == 0;
        }
        Ok(true)
    }

    fn install(
        &mut self,
        redirects: Vec<Redirect<Word>>,
        session: &mut Session,
        status: u8,
        child_redirects: bool,
        captured: &Cell<Option<u8>>,
    ) -> Option<(Flow, usize)> {
        for raw in redirects {
            let line = raw.line;
            let flow = match self.redirect(session, raw, status, captured) {
                Ok(true) => continue,
                Ok(false) => Flow::Continue(1),
                Err(error) => {
                    if let Some(expansion) = error
                        .get_ref()
                        .and_then(|e| e.downcast_ref::<crate::expansion::Error>())
                    {
                        expansion.report("");
                        if child_redirects {
                            Flow::Continue(expansion.status)
                        } else {
                            expansion.flow(session)
                        }
                    } else {
                        diagnostic!("{}", crate::builtins::os_message(&error));
                        Flow::Continue(1)
                    }
                }
            };
            return Some((flow, line));
        }
        None
    }

    fn finish(self, session: &mut Session, committed: bool) -> io::Result<()> {
        if committed {
            for fd in self.changed {
                fd_replaced(session, fd);
            }
        } else {
            SavedFd::restore_all(self.restore.into_iter().rev())?;
        }
        Ok(())
    }
}

pub(super) fn with_redirects(
    redirects: Vec<Redirect<Word>>,
    session: &mut Session,
    status: u8,
    child_redirects: bool,
    captured: &Cell<Option<u8>>,
    on_failure: impl FnOnce(&mut Session, Flow, usize) -> Flow,
    run: impl FnOnce(&mut Session) -> io::Result<(Flow, bool)>,
) -> io::Result<Flow> {
    let old_input = session.stdin_redirected;
    let mut scope = Scope {
        isolated: child_redirects,
        ..Scope::default()
    };
    let saved_substitutions = session.substitutions.scope();
    let mut child = child_redirects.then(|| session.clone());
    let env = child.as_mut().unwrap_or(session);
    let applied = scope.install(redirects, env, status, child_redirects, captured);
    if let Some(child) = child {
        session.stdin_redirected = child.stdin_redirected;
        session
            .substitutions
            .adopt(child.substitutions, saved_substitutions);
    }
    if let Some((flow, line)) = applied {
        session.stdin_redirected = old_input;
        scope.finish(session, false)?;
        return Ok(on_failure(session, flow, line));
    }
    let result = run(session);
    let committed = matches!(result, Ok((_, true)));
    if !committed {
        session.stdin_redirected = old_input;
    }
    scope.finish(session, committed)?;
    result.map(|(flow, _)| flow)
}

fn command(mut cmd: PreparedCommand, session: &mut Session, status: u8) -> io::Result<Flow> {
    let captured = Cell::new(None);
    let fatal = matches!(builtin::redirect_failure(&cmd, session), Flow::Exit(_));
    let compound = cmd.compound.is_some();
    with_redirects(
        std::mem::take(&mut cmd.deferred_redirects),
        session,
        status,
        false,
        &captured,
        |session, flow, line| match flow {
            Flow::Continue(code) if fatal => Flow::Exit(code),
            Flow::Continue(code) if compound => {
                builtin::compound_redirect_failure(session, code, line)
            }
            other => other,
        },
        |session| {
            cmd.empty_status = captured.get().unwrap_or(cmd.empty_status);
            if let Some(b) = builtin(&cmd, status, session).or_else(|| {
                cmd.prepare_missing_hook(session)
                    .then_some(Builtin::Function)
            }) {
                let flow = eval_builtin(b, &mut cmd, session, status, false);
                let commit = matches!(b, Builtin::Exec) && matches!(flow, Flow::Continue(0));
                Ok((flow, commit))
            } else {
                Ok((
                    Flow::Continue(External::new(&cmd, session)?.exec(session, false)),
                    false,
                ))
            }
        },
    )
}

pub(super) fn run(
    cmd: PreparedCommand,
    session: &mut Session,
    status: u8,
    replace: bool,
    mut launch: crate::jobs::Launch,
) -> io::Result<Flow> {
    let isolated_null = cmd.words.is_empty()
        && cmd.compound.is_none()
        && cmd
            .deferred_redirects
            .iter()
            .any(|r| r.variable.is_some() || r.fd == 0);
    let local = !isolated_null && builtin(&cmd, status, session).is_some();
    if local || replace && launch.group.is_none() {
        return command(cmd, session, status);
    }
    let pid = session.fork_shell()?;
    if pid == 0 {
        launch.pipeline_child(&mut session.traps);
        session.jobs.forked(false);
        coprocess::after_fork(session);
        let code = match command(cmd, session, status) {
            Ok(flow) => flow.child_status(),
            Err(e) => {
                diagnostic!("{e}");
                1
            }
        };
        crate::subshell::exit_child(session, code)
    }
    launch.parent(pid);
    if launch.group.is_none() {
        return wait(pid).map(Flow::Continue);
    }
    let (code, _) = session.jobs.finish(
        launch,
        vec![Ok(pid)],
        cmd.words.join(" "),
        false,
        &mut session.pipe_status,
    )?;
    Ok(Flow::Continue(code))
}
