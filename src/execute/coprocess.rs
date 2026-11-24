use std::cell::Cell;
use std::io;
use std::os::fd::{AsRawFd, IntoRawFd, OwnedFd, RawFd};
use std::rc::Rc;

use crate::{Flow, Session, parser::Command};

pub(crate) struct Coprocess {
    name: String,
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

pub(super) fn replaced(session: &Session, destination: RawFd) {
    for job in &session.coprocesses {
        for fd in &job.fds {
            if fd.get() == Some(destination) {
                fd.set(None);
            }
        }
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
                session.unset_variable(&format!("{}_PID", job.name));
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
    if session.readonly(&name) || session.readonly(&pid_name) {
        return Err(io::Error::other("readonly coprocess variable"));
    }
    if let Some(active) = session.coprocesses.last() {
        eprintln!(
            "tonari: warning: coproc [{}:{}] still exists",
            active.pid, active.name
        );
    }
    let (input, writer) = super::pipe()?;
    let (reader, output) = super::pipe()?;
    let high = |fd: OwnedFd| -> io::Result<OwnedFd> {
        super::fd::duplicate_except(fd.as_raw_fd(), 10, |_| false).map(Into::into)
    };
    let reader = high(reader)?;
    let writer = high(writer)?;
    let mut launch = session.jobs.launch(true, false);
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        drop(reader);
        drop(writer);
        after_fork(session);
        session
            .traps
            .after_fork(session.errtrace, session.functrace);
        launch.pipeline_child();
        if super::duplicate(input.as_raw_fd(), 0).is_err()
            || super::duplicate(output.as_raw_fd(), 1).is_err()
        {
            unsafe { libc::_exit(1) }
        }
        drop(input);
        drop(output);
        session.jobs.forked(false);
        session.subshell_depth += 1;
        session.stdin_redirected = true;
        session.interactive = false;
        let code = match crate::eval::command(session, body, status, true) {
            Flow::Continue(s) | Flow::Discard(s) | Flow::Exit(s) | Flow::Return(s) => s,
            Flow::Break(_) | Flow::LoopContinue(_) => 0,
        };
        let code = crate::traps::finish(session, code);
        unsafe { libc::_exit(i32::from(code)) }
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
        pid,
        fds: [
            Cell::new(Some(reader.into_raw_fd())),
            Cell::new(Some(writer.into_raw_fd())),
        ],
    }));
    Ok(Flow::Continue(0))
}
