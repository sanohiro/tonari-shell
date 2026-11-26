// Named descriptors have effects while redirections are still being expanded.
// For example, the second redirect in 'exec {out}>file 3>&$out' sees the first.
use super::*;
use std::os::fd::RawFd;

#[derive(Default)]
struct Scope {
    restore: Vec<(RawFd, SavedFd)>,
    changed: Vec<RawFd>,
}

impl Scope {
    fn redirect(
        &mut self,
        session: &mut Session,
        mut raw: Redirect<Word>,
        status: u8,
    ) -> io::Result<bool> {
        let variable = raw.variable.take();
        let captured = Cell::new(None);
        let mut exp = Expansion::new(session, status, &captured);
        let expanded = prepare_redirects([raw], &mut exp).map_err(io::Error::other)?;
        for (i, mut r) in expanded.into_iter().enumerate() {
            let name = variable.as_deref().filter(|_| i == 0);
            let close = r.target == Target::Duplicate && r.word == "-";
            if let Some(name) = name {
                if close {
                    let element = Expansion::new(session, status, &captured)
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
                        return Err(io::Error::other(format!("{name}: readonly variable")));
                    }
                    // Open first so /dev/fd and duplicate sources see the original table.
                    let source;
                    let fd = match action(&r) {
                        Action::Open(_, path, flags) => {
                            let Some(opened) = open_redirect(&path, flags) else {
                                return Ok(false);
                            };
                            source = opened;
                            source.as_raw_fd()
                        }
                        Action::HereInput(_, text) => {
                            source = here_input::open(text, libc::PIPE_BUF)?;
                            source.as_raw_fd()
                        }
                        Action::Dup(fd, _) => {
                            if fd::is_private(fd) {
                                return Err(io::Error::from_raw_os_error(libc::EBADF));
                            }
                            fd
                        }
                        _ => unreachable!(),
                    };
                    let allocated = unsafe { libc::fcntl(fd, libc::F_DUPFD, 10) };
                    if allocated < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    if let Err(error) = session.assign_target(name, &allocated.to_string()) {
                        unsafe {
                            libc::close(allocated);
                        }
                        return Err(io::Error::other(error));
                    }
                    if session.shopt.varredir_close {
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
            if apply_protected(actions, libc::PIPE_BUF).is_err() {
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
    ) -> io::Result<Option<Flow>> {
        for raw in redirects {
            match self.redirect(session, raw, status) {
                Ok(true) => {}
                Ok(false) => return Ok(Some(Flow::Continue(1))),
                Err(error) => {
                    let Some(expansion) = error
                        .get_ref()
                        .and_then(|e| e.downcast_ref::<crate::expansion::Error>())
                    else {
                        return Err(error);
                    };
                    expansion.report("");
                    return Ok(Some(if child_redirects {
                        Flow::Continue(expansion.status)
                    } else {
                        expansion.flow(session)
                    }));
                }
            }
        }
        Ok(None)
    }

    fn finish(self, session: &mut Session, committed: bool) -> io::Result<()> {
        if committed {
            for fd in self.changed {
                session.stdin_replaced |= fd == 0;
                session.substitutions.replaced(fd);
                super::coprocess::replaced(session, fd);
            }
        } else {
            let mut result = Ok(());
            for (fd, saved) in self.restore.into_iter().rev() {
                if let Err(e) = saved.restore(fd) {
                    result = Err(e);
                }
            }
            result?;
        }
        Ok(())
    }
}

pub(super) fn with_redirects(
    redirects: Vec<Redirect<Word>>,
    session: &mut Session,
    status: u8,
    child_redirects: bool,
    run: impl FnOnce(&mut Session) -> io::Result<(Flow, bool)>,
) -> io::Result<Flow> {
    let old_input = session.stdin_redirected;
    let mut scope = Scope::default();
    let saved_substitutions = session.substitutions.scope();
    let mut child = child_redirects.then(|| session.clone());
    let env = child.as_mut().unwrap_or(session);
    let applied = scope.install(redirects, env, status, child_redirects);
    if let Some(child) = child {
        session.stdin_redirected = child.stdin_redirected;
        session
            .substitutions
            .adopt(child.substitutions, saved_substitutions);
    }
    let result = match applied {
        Ok(None) => run(session),
        Ok(Some(flow)) => Ok((flow, false)),
        Err(error) => Err(error),
    };
    let committed = matches!(result, Ok((_, true)));
    if !committed {
        session.stdin_redirected = old_input;
    }
    scope.finish(session, committed)?;
    result.map(|(flow, _)| flow)
}

fn command(mut cmd: PreparedCommand, session: &mut Session, status: u8) -> io::Result<Flow> {
    with_redirects(
        std::mem::take(&mut cmd.named_redirects),
        session,
        status,
        false,
        |session| {
            if let Some(b) = builtin(&cmd, status, session).or_else(|| {
                cmd.prepare_missing_hook(session)
                    .then_some(Builtin::Function)
            }) {
                let flow = eval_builtin(b, &mut cmd, session, status, false);
                let commit = matches!(b, Builtin::Exec) && matches!(flow, Flow::Continue(0));
                Ok((flow, commit))
            } else {
                Ok((
                    Flow::Continue(External::new(&cmd, session).exec(None)),
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
    let local = (!cmd.words.is_empty() || cmd.compound.is_some())
        && builtin(&cmd, status, session).is_some();
    if local || replace && launch.group.is_none() {
        return command(cmd, session, status);
    }
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        launch.pipeline_child(&mut session.traps);
        let code = match command(cmd, session, status) {
            Ok(Flow::Continue(s) | Flow::Discard(s) | Flow::Exit(s) | Flow::Return(s)) => s,
            Ok(_) => 1,
            Err(e) => {
                eprintln!("tonari: {e}");
                1
            }
        };
        unsafe { libc::_exit(i32::from(code)) }
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
