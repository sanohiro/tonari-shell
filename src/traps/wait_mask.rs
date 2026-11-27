use super::*;

// An explicit wait is interruptible. Block signals while checking child and
// trap state, then atomically release the mask in sigsuspend. A foreground
// command instead finishes before Bash dispatches the trapped signal.
pub(crate) struct SignalMask(pub(crate) libc::sigset_t);

impl SignalMask {
    pub(crate) fn block(set: &libc::sigset_t) -> io::Result<Self> {
        let mut previous = unsafe { std::mem::zeroed() };
        if unsafe { libc::sigprocmask(libc::SIG_BLOCK, set, &mut previous) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(previous))
    }
}

impl Drop for SignalMask {
    fn drop(&mut self) {
        unsafe {
            libc::sigprocmask(libc::SIG_SETMASK, &self.0, std::ptr::null_mut());
        }
    }
}

pub(crate) struct WaitMask {
    previous: SignalMask,
    child: libc::sigaction,
    running: u64,
}

extern "C" fn wake(_: i32) {}

pub(super) fn is_os_signal(sig: i32) -> bool {
    sig > 0 && sig < DEBUG
}

#[test]
fn shell_traps_alone_do_not_install_a_wait_mask() {
    let mut traps = Traps::default();
    for sig in [DEBUG, ERR, RETURN] {
        traps.set(sig, ":", false).unwrap();
    }
    assert!(WaitMask::new(&traps).unwrap().is_none());
}

impl WaitMask {
    pub fn new(traps: &Traps) -> io::Result<Option<Self>> {
        if !traps.interactive_hup
            && traps.implicit.is_empty()
            && !traps
                .entries
                .iter()
                .any(|(&s, e)| is_os_signal(s) && s != libc::SIGCHLD && !e.command.is_empty())
        {
            return Ok(None);
        }
        Self::watch_children(traps).map(Some)
    }

    pub fn watch_children(traps: &Traps) -> io::Result<Self> {
        Self::for_children(traps.running)
    }

    pub(crate) fn for_children(running: u64) -> io::Result<Self> {
        let mut mask = unsafe { std::mem::zeroed() };
        unsafe { libc::sigfillset(&mut mask) };
        let previous = SignalMask::block(&mask)?;
        let child = disposition(libc::SIGCHLD)?;
        if matches!(child.sa_sigaction, libc::SIG_DFL | libc::SIG_IGN) {
            set_disposition(libc::SIGCHLD, &action(wake as *const () as usize))?;
        }
        Ok(Self {
            previous,
            child,
            running,
        })
    }

    pub fn interrupt_status(&self) -> Option<u8> {
        let sig = TERMINATING.load(Relaxed);
        if sig != 0 {
            return Some(crate::traps::signal_status(sig));
        }
        let bits = PENDING.load(Relaxed) & !(self.running | (signal_bit(libc::SIGCHLD)));
        (bits != 0).then(|| 129 + bits.trailing_zeros() as u8)
    }

    pub(crate) fn input_or_child(&self, timeout: Option<&libc::timespec>) -> io::Result<bool> {
        let mut fds = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut fds);
            libc::FD_SET(0, &mut fds);
            let mut mask = self.previous.0;
            libc::sigdelset(&mut mask, libc::SIGCHLD);
            let result = libc::pselect(
                1,
                &mut fds,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                timeout.map_or(std::ptr::null(), std::ptr::from_ref),
                &mask,
            );
            if result > 0 {
                return Ok(true);
            }
            if result == 0 {
                return Err(io::ErrorKind::TimedOut.into());
            }
        }
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            Ok(false)
        } else {
            Err(err)
        }
    }

    pub fn suspend(&self) {
        unsafe {
            let mut mask = self.previous.0;
            libc::sigdelset(&mut mask, libc::SIGCHLD);
            libc::sigsuspend(&mask);
        }
    }
}

impl Drop for WaitMask {
    fn drop(&mut self) {
        let _ = set_disposition(libc::SIGCHLD, &self.child);
    }
}
