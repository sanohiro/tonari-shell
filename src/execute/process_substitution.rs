use std::cell::Cell;
use std::collections::BTreeMap;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::rc::Rc;

use crate::{Session, parser::Statement};

struct Endpoint {
    fd: Cell<Option<RawFd>>,
    pid: libc::pid_t,
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        if let Some(fd) = self.fd.take() {
            unsafe { libc::close(fd) };
        }
    }
}

#[derive(Default, Clone)]
pub(crate) struct Substitutions {
    active: Vec<Rc<Endpoint>>,
    // A completed process may still be named by wait after its fd closes.
    pending: Vec<libc::pid_t>,
    finished: BTreeMap<libc::pid_t, u8>,
}

fn status(pid: libc::pid_t, options: i32) -> Option<u8> {
    let mut state = 0;
    loop {
        match crate::traps::waitpid(pid, &mut state, options) {
            n if n > 0 && libc::WIFEXITED(state) => return Some(libc::WEXITSTATUS(state) as u8),
            n if n > 0 && libc::WIFSIGNALED(state) => {
                return Some((128 + libc::WTERMSIG(state)) as u8);
            }
            -1 if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => continue,
            -1 => return Some(127),
            _ => return None,
        }
    }
}

impl Substitutions {
    pub(super) fn adopt(&mut self, mut other: Self, from: usize) {
        for end in other.active.drain(from..) {
            self.pending.push(end.pid);
            self.active.push(end);
        }
    }
    pub(crate) fn scope(&self) -> usize {
        self.active.len()
    }
    pub(crate) fn finish(&mut self, start: usize) {
        self.active.truncate(start);
        self.pending.retain(|&pid| {
            if let Some(code) = status(pid, libc::WNOHANG) {
                if self.finished.len() >= 4096 {
                    self.finished.pop_first();
                }
                self.finished.insert(pid, code);
                false
            } else {
                true
            }
        });
    }

    pub(crate) fn wait(
        &mut self,
        pid: libc::pid_t,
        mask: Option<&crate::traps::WaitMask>,
    ) -> Option<u8> {
        if let Some(code) = self.finished.remove(&pid) {
            return Some(code);
        }
        let i = self.pending.iter().position(|&child| child == pid)?;
        if let Some(mask) = mask {
            loop {
                if let Some(code) = mask.interrupt_status() {
                    return Some(code);
                }
                if let Some(code) = status(pid, libc::WNOHANG) {
                    self.pending.swap_remove(i);
                    return Some(code);
                }
                mask.suspend();
            }
        }
        self.pending.swap_remove(i);
        status(pid, 0)
    }

    pub(super) fn replaced(&mut self, fd: RawFd) {
        for end in &self.active {
            if end.fd.get() == Some(fd) {
                end.fd.set(None);
            }
        }
    }
}

pub(crate) fn launch(
    session: &mut Session,
    body: Vec<Statement>,
    input: bool,
    prior: u8,
    assignments: &[(String, String)],
) -> Result<String, &'static str> {
    let (read, write) = super::pipe().map_err(|_| "cannot create process substitution pipe")?;
    let (keep, child, target) = if input {
        (read, write, 1)
    } else {
        (write, read, 0)
    };
    // Favor numbers above the descriptors normally named by scripts. On a low
    // descriptor limit the pipe's own (nonstandard) number works equally well.
    let high = unsafe { libc::fcntl(keep.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 64) };
    let keep = if high >= 0 {
        drop(keep);
        unsafe { OwnedFd::from_raw_fd(high) }
    } else {
        keep
    };
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err("cannot fork process substitution");
    }
    if pid == 0 {
        session
            .traps
            .after_fork(session.errtrace, session.functrace);
        drop(keep);
        super::reset_child_signals();
        if super::duplicate(child.as_raw_fd(), target).is_err() {
            unsafe { libc::_exit(1) }
        }
        drop(child);
        session.enter_subshell();
        crate::execute::coprocess::after_fork(session);
        session.jobs.forked(false);
        session.interactive = false;
        session.substitutions.pending.clear();
        session.substitutions.finished.clear();
        session.bind(assignments);
        let status = crate::execute_program(session, body, prior, true).child_status();
        crate::subshell::exit_child(session, status)
    }
    drop(child);
    let fd = keep.as_raw_fd();
    if unsafe { libc::fcntl(fd, libc::F_SETFD, 0) } < 0 {
        drop(keep);
        unsafe { libc::kill(pid, libc::SIGKILL) };
        status(pid, 0);
        return Err("cannot inherit process substitution descriptor");
    }
    session.substitutions.active.push(Rc::new(Endpoint {
        fd: Cell::new(Some(keep.into_raw_fd())),
        pid,
    }));
    session.substitutions.pending.push(pid);
    session.last_background = Some(pid);
    Ok(format!("/dev/fd/{fd}"))
}
