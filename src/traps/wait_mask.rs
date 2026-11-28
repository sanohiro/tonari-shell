use super::*;
use std::time::Duration;

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

// Darwin exports its dynamically sized fd_set ABI under a separate symbol.
#[cfg(target_os = "macos")]
unsafe extern "C" {
    #[link_name = "pselect$DARWIN_EXTSN"]
    fn pselect_input(
        nfds: libc::c_int,
        readfds: *mut libc::fd_set,
        writefds: *mut libc::fd_set,
        exceptfds: *mut libc::fd_set,
        timeout: *const libc::timespec,
        sigmask: *const libc::sigset_t,
    ) -> libc::c_int;
}
#[cfg(not(target_os = "macos"))]
use libc::pselect as pselect_input;

pub(crate) fn wait_readable(
    fd: i32,
    timeout: Option<Duration>,
    watch: Option<&Traps>,
) -> io::Result<bool> {
    // Darwin rejects sufficiently large tv_sec values. Each bounded call
    // rebuilds its descriptor set, which pselect can modify on timeout.
    const MAX_WAIT: Duration = Duration::from_secs(i32::MAX as u64);
    if let Some(total) = timeout.filter(|&t| t > MAX_WAIT) {
        let start = std::time::Instant::now();
        loop {
            let remaining = total.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Ok(false);
            }
            if wait_readable(fd, Some(remaining.min(MAX_WAIT)), watch)? {
                return Ok(true);
            }
        }
    }
    // A single ordinary fd_set covers the common case. Both supported kernels
    // read ceil(nfds / word_bits) words, including for EOF on character devices.
    let index = fd as usize / libc::FD_SETSIZE;
    let mut first = unsafe { std::mem::zeroed() };
    let mut extra;
    let sets = if index == 0 {
        std::slice::from_mut(&mut first)
    } else {
        // Bound the allocation to a real descriptor owned by this process.
        if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
            return Err(io::Error::last_os_error());
        }
        extra = vec![first; index + 1];
        &mut extra[..]
    };
    unsafe { libc::FD_SET(fd % libc::FD_SETSIZE as i32, &mut sets[index]) };
    let time = timeout.map(|t| libc::timespec {
        tv_sec: t.as_secs().min(i64::MAX as u64) as _,
        tv_nsec: t.subsec_nanos().into(),
    });
    let mask = if let Some(traps) = watch {
        let mut all = unsafe { std::mem::zeroed() };
        unsafe { libc::sigfillset(&mut all) };
        let mask = SignalMask::block(&all)?;
        if traps.read_pending() || crate::INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        Some(mask)
    } else {
        None
    };
    let result = unsafe {
        pselect_input(
            fd + 1,
            sets.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            time.as_ref().map_or(std::ptr::null(), std::ptr::from_ref),
            mask.as_ref().map_or(std::ptr::null(), |m| &m.0),
        )
    };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result > 0)
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
        if !traps.interactive_handlers
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
        if crate::INTERRUPTED.load(Relaxed) {
            return Some(crate::traps::signal_status(libc::SIGINT));
        }
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
