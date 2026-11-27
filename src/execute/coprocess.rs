use std::cell::Cell;
use std::io;
use std::os::fd::{AsRawFd, IntoRawFd, OwnedFd, RawFd};
use std::rc::Rc;

use crate::{Flow, Session, parser::Command};

pub(crate) struct Coprocess {
    name: String,
    pid_name: String,
    pid: libc::pid_t,
    fds: [Cell<Option<RawFd>>; 2],
}

impl Coprocess {
    fn close(&self) {
        for fd in &self.fds {
            if let Some(raw) = fd.take() {
                unsafe { libc::close(raw) };
            }
        }
    }
}

impl Drop for Coprocess {
    fn drop(&mut self) {
        self.close();
    }
}

pub(super) fn replaced(session: &mut Session, destination: RawFd) {
    for i in 0..session.coprocesses.len() {
        let job = &session.coprocesses[i];
        let Some(end) = job.fds.iter().position(|fd| fd.get() == Some(destination)) else {
            continue;
        };
        job.fds[end].set(None);
        if !session.coprocesses[i + 1..]
            .iter()
            .any(|newer| newer.name == job.name)
        {
            let job = Rc::clone(job);
            session.assign_element(&job.name, end as i64, "-1".into());
        }
        return;
    }
}

pub(crate) fn after_fork(session: &mut Session) {
    for job in session.coprocesses.drain(..) {
        job.close();
    }
}

pub(crate) fn refresh(session: &mut Session) {
    session.jobs.poll();
    for i in (0..session.coprocesses.len()).rev() {
        if !session.jobs.has_live_pid(session.coprocesses[i].pid) {
            let job = session.coprocesses.remove(i);
            if !session.coprocesses.iter().any(|live| live.name == job.name) {
                session.unset_variable(&job.name);
            }
            if !session
                .coprocesses
                .iter()
                .any(|live| live.pid_name == job.pid_name)
            {
                session.unset_variable(&job.pid_name);
            }
            job.close();
        }
    }
}

pub(crate) fn launch(
    session: &mut Session,
    name: String,
    body: Command,
    status: u8,
) -> io::Result<Flow> {
    let pid_name = format!("{name}_PID");
    let name = match session
        .indexed_receiver(&name, true)
        .map_err(io::Error::other)?
    {
        std::borrow::Cow::Borrowed(_) => name,
        std::borrow::Cow::Owned(target) => target,
    };
    if session.readonly(&pid_name) {
        return Err(io::Error::other("readonly coprocess variable"));
    }
    if let Some(active) = session.coprocesses.last() {
        diagnostic!(
            "warning: coproc [{}:{}] still exists",
            active.pid,
            active.name
        );
    }
    let (input, writer) = super::pipe()?;
    let (reader, output) = super::pipe()?;
    let high = |fd: OwnedFd| -> io::Result<OwnedFd> {
        super::fd::duplicate_except(fd.as_raw_fd(), super::fd::SHELL_FD_MIN, |_| false)
            .map(Into::into)
    };
    let reader = high(reader)?;
    let writer = high(writer)?;
    let mut launch = session.jobs.launch(true, false);
    let pid = session.fork_shell()?;
    if pid == 0 {
        drop(reader);
        drop(writer);
        after_fork(session);
        launch.pipeline_child(&mut session.traps);
        if super::duplicate(input.as_raw_fd(), 0).is_err()
            || super::duplicate(output.as_raw_fd(), 1).is_err()
        {
            unsafe { libc::_exit(1) }
        }
        drop(input);
        drop(output);
        session.jobs.forked(false);
        session.enter_subshell();
        session.stdin_redirected = true;
        session.interactive = false;
        let code = crate::eval::command(session, body, status, true).child_status();
        crate::subshell::exit_child(session, code)
    }
    drop(input);
    drop(output);
    launch.parent(pid);
    session.jobs.finish(
        launch,
        vec![Ok(pid)],
        format!("coproc {name}"),
        false,
        &mut session.pipe_status,
    )?;
    session.last_background = Some(pid);
    session.replace_array(
        &name,
        vec![
            reader.as_raw_fd().to_string(),
            writer.as_raw_fd().to_string(),
        ],
    );
    session.assign(&pid_name, &pid.to_string());
    session.coprocesses.push(Rc::new(Coprocess {
        name,
        pid_name,
        pid,
        fds: [
            Cell::new(Some(reader.into_raw_fd())),
            Cell::new(Some(writer.into_raw_fd())),
        ],
    }));
    Ok(Flow::Continue(0))
}
