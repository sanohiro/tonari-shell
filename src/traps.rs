// A signal handler can run while Rust holds allocator and I/O locks. It only
// sets a bit. The interpreter executes shell code at a command boundary.
use crate::{Flow, Session};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering::Relaxed};

pub(crate) const SIGNALS: &[(&str, i32)] = &[
    ("EXIT", 0),
    ("HUP", libc::SIGHUP),
    ("INT", libc::SIGINT),
    ("QUIT", libc::SIGQUIT),
    ("ILL", libc::SIGILL),
    ("TRAP", libc::SIGTRAP),
    ("ABRT", libc::SIGABRT),
    ("BUS", libc::SIGBUS),
    ("FPE", libc::SIGFPE),
    ("KILL", libc::SIGKILL),
    ("USR1", libc::SIGUSR1),
    ("SEGV", libc::SIGSEGV),
    ("USR2", libc::SIGUSR2),
    ("PIPE", libc::SIGPIPE),
    ("ALRM", libc::SIGALRM),
    ("TERM", libc::SIGTERM),
    ("CHLD", libc::SIGCHLD),
    ("CONT", libc::SIGCONT),
    ("STOP", libc::SIGSTOP),
    ("TSTP", libc::SIGTSTP),
    ("TTIN", libc::SIGTTIN),
    ("TTOU", libc::SIGTTOU),
    ("URG", libc::SIGURG),
    ("XCPU", libc::SIGXCPU),
    ("XFSZ", libc::SIGXFSZ),
    ("VTALRM", libc::SIGVTALRM),
    ("PROF", libc::SIGPROF),
    ("WINCH", libc::SIGWINCH),
    ("IO", libc::SIGIO),
    ("SYS", libc::SIGSYS),
];

static TERMINATING: AtomicI32 = AtomicI32::new(0);
extern "C" fn terminate(sig: i32) {
    let _ = TERMINATING.compare_exchange(0, sig, Relaxed, Relaxed);
}

static PENDING: AtomicU64 = AtomicU64::new(0);
extern "C" fn record(sig: i32) {
    PENDING.fetch_or(1 << sig, Relaxed);
}

// Pseudo-traps never enter the OS signal API or the pending signal bitset.
const DEBUG: i32 = i32::MAX - 2;
const ERR: i32 = i32::MAX - 1;
const RETURN: i32 = i32::MAX;
fn trap_number(name: &str) -> Option<i32> {
    if name.eq_ignore_ascii_case("DEBUG") {
        Some(DEBUG)
    } else if name.eq_ignore_ascii_case("ERR") {
        Some(ERR)
    } else if name.eq_ignore_ascii_case("RETURN") {
        Some(RETURN)
    } else {
        number(name)
    }
}

#[derive(Clone)]
pub(crate) struct Entry {
    command: String,
    original: libc::sigaction,
}

#[derive(Default, Clone)]
pub struct Traps {
    entries: BTreeMap<i32, Entry>,
    running: u64,
    handling_debug: bool,
    handling_error: bool,
    handling_return: bool,
    interactive_hup: bool,
    pub before_return: u8,
    implicit: BTreeMap<i32, libc::sigaction>,
    pub exit_status: Option<u8>,
}

pub(crate) fn number(s: &str) -> Option<i32> {
    let number = s.parse::<i32>().ok();
    let s = s
        .get(..3)
        .filter(|prefix| prefix.eq_ignore_ascii_case("SIG"))
        .map_or(s, |_| &s[3..]);
    SIGNALS
        .iter()
        .find(|(name, n)| s.eq_ignore_ascii_case(name) || number == Some(*n))
        .map(|s| s.1)
}

fn action(handler: usize) -> libc::sigaction {
    let mut a: libc::sigaction = unsafe { std::mem::zeroed() };
    a.sa_sigaction = handler;
    unsafe { libc::sigemptyset(&mut a.sa_mask) };
    a
}

impl Traps {
    pub fn in_action(&self) -> bool {
        self.handling_debug
            || self.running != 0
            || self.handling_error
            || self.handling_return
            || self.exit_status.is_some()
    }

    pub fn active(&self) -> bool {
        !self.entries.is_empty() || self.interactive_hup
    }

    pub fn init_hangup(&mut self) -> io::Result<()> {
        let mut old = action(libc::SIG_DFL);
        if unsafe { libc::sigaction(libc::SIGHUP, std::ptr::null(), &mut old) } < 0 {
            return Err(io::Error::last_os_error());
        }
        if old.sa_sigaction == libc::SIG_DFL {
            if unsafe {
                libc::sigaction(
                    libc::SIGHUP,
                    &action(terminate as *const () as usize),
                    std::ptr::null_mut(),
                )
            } < 0
            {
                return Err(io::Error::last_os_error());
            }
            self.interactive_hup = true;
        }
        Ok(())
    }

    pub fn ignores(&self, signal: i32) -> bool {
        self.entries
            .get(&signal)
            .is_some_and(|e| e.command.is_empty())
    }

    pub fn catches(&self, signal: i32) -> bool {
        self.entries
            .get(&signal)
            .is_some_and(|e| !e.command.is_empty())
    }

    // Bash runs EXIT cleanup on asynchronous termination before restoring the
    // default action and delivering the original signal to itself.
    fn watch_exit(&mut self, enable: bool) -> io::Result<()> {
        if !enable {
            for (sig, old) in std::mem::take(&mut self.implicit) {
                if !self.entries.contains_key(&sig) {
                    unsafe {
                        libc::sigaction(sig, &old, std::ptr::null_mut());
                    }
                }
            }
        } else if self.implicit.is_empty() {
            for sig in [
                libc::SIGHUP,
                libc::SIGINT,
                libc::SIGTERM,
                libc::SIGUSR1,
                libc::SIGUSR2,
                libc::SIGALRM,
                libc::SIGPIPE,
            ] {
                let mut old = action(libc::SIG_DFL);
                if let Some(entry) = self.entries.get(&sig) {
                    old = entry.original;
                } else if unsafe { libc::sigaction(sig, std::ptr::null(), &mut old) } < 0 {
                    return Err(io::Error::last_os_error());
                }
                if old.sa_sigaction != libc::SIG_DFL && sig != libc::SIGPIPE {
                    continue;
                }
                if !self.entries.contains_key(&sig)
                    && unsafe {
                        libc::sigaction(
                            sig,
                            &action(terminate as *const () as usize),
                            std::ptr::null_mut(),
                        )
                    } < 0
                {
                    return Err(io::Error::last_os_error());
                }
                if sig == libc::SIGPIPE && old.sa_sigaction == libc::SIG_IGN {
                    old = action(libc::SIG_DFL);
                }
                self.implicit.insert(sig, old);
            }
        }
        Ok(())
    }

    fn set(&mut self, sig: i32, command: &str, interactive: bool) -> io::Result<()> {
        if sig == 0 {
            self.watch_exit(command != "-" && !command.is_empty())?;
        }
        let real = sig < DEBUG && sig > 0 && sig != libc::SIGSTOP && sig != libc::SIGKILL;
        let mut original = action(libc::SIG_DFL);
        if let Some(old) = self.entries.get(&sig) {
            original = old.original;
        } else if let Some(old) = self.implicit.get(&sig) {
            original = *old;
        } else if real {
            if unsafe { libc::sigaction(sig, std::ptr::null(), &mut original) } < 0 {
                return Err(io::Error::last_os_error());
            }
            // Rust installs SIG_IGN for SIGPIPE on entry to main. A shell trap
            // reset restores its ordinary external-command disposition.
            if sig == libc::SIGPIPE && original.sa_sigaction == libc::SIG_IGN {
                original = action(libc::SIG_DFL);
            }
            if original.sa_sigaction == libc::SIG_IGN && !interactive && sig != libc::SIGPIPE {
                return Ok(());
            }
        }
        if real {
            let new = match command {
                "-" if self.implicit.contains_key(&sig)
                    || sig == libc::SIGHUP && self.interactive_hup =>
                {
                    action(terminate as *const () as usize)
                }
                "-" => original,
                "" if sig == libc::SIGCHLD => action(libc::SIG_DFL), // keep children waitable
                "" => action(libc::SIG_IGN),
                _ => action(record as *const () as usize),
            };
            if unsafe { libc::sigaction(sig, &new, std::ptr::null_mut()) } < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        if command == "-" {
            self.entries.remove(&sig);
        } else {
            self.entries.insert(
                sig,
                Entry {
                    command: command.into(),
                    original,
                },
            );
        }
        Ok(())
    }

    fn hide(&mut self, sig: i32, inherit: bool) -> Option<Entry> {
        if inherit {
            None
        } else {
            self.entries.remove(&sig)
        }
    }

    fn restore(&mut self, sig: i32, hidden: Option<Entry>) {
        if let Some(saved) = hidden {
            self.entries.entry(sig).or_insert(saved);
        }
    }

    pub fn enter_source(&mut self, traced: bool) -> Option<Entry> {
        self.hide(DEBUG, traced)
    }
    pub fn leave_source(&mut self, saved: Option<Entry>) {
        self.restore(DEBUG, saved);
    }
    pub fn debug_handler(&self) -> bool {
        !self.handling_debug && self.catches(DEBUG)
    }

    pub fn enter_function(
        &mut self,
        errtrace: bool,
        functrace: bool,
    ) -> (Option<Entry>, Option<Entry>, Option<Entry>) {
        if self.entries.is_empty() {
            return (None, None, None);
        }
        (
            self.hide(ERR, errtrace),
            self.hide(RETURN, functrace),
            self.hide(DEBUG, functrace),
        )
    }

    pub fn leave_function(&mut self, hidden: (Option<Entry>, Option<Entry>, Option<Entry>)) {
        self.restore(ERR, hidden.0);
        self.restore(RETURN, hidden.1);
        self.restore(DEBUG, hidden.2);
    }

    pub fn error_handler(&self) -> bool {
        self.catches(ERR) && !self.handling_error
    }

    pub fn after_fork(&mut self, errtrace: bool, functrace: bool) {
        if self.interactive_hup {
            self.interactive_hup = false;
            if !self.ignores(libc::SIGHUP) {
                unsafe {
                    libc::signal(libc::SIGHUP, libc::SIG_DFL);
                }
            }
            TERMINATING.store(0, Relaxed);
        }
        if self.entries.is_empty() {
            return;
        }
        let _ = self.watch_exit(false);
        self.exit_status = None;
        TERMINATING.store(0, Relaxed);
        self.entries.retain(|&sig, e| {
            if sig == ERR {
                return errtrace;
            }
            if matches!(sig, RETURN | DEBUG) {
                return functrace;
            }
            if sig > 0 && e.command.is_empty() {
                return true;
            }
            if sig > 0 && sig != libc::SIGKILL && sig != libc::SIGSTOP {
                unsafe { libc::signal(sig, libc::SIG_DFL) };
            }
            false
        });
        self.running = 0;
        PENDING.store(0, Relaxed);
    }

    pub fn builtin(&mut self, args: &[String], interactive: bool) -> u8 {
        let args = if args.first().is_some_and(|s| s == "--") {
            &args[1..]
        } else {
            args
        };
        let list = args.first().is_some_and(|s| s == "-p");
        if args.first().is_some_and(|s| s == "-l") {
            let mut out = io::stdout().lock();
            for &(name, n) in SIGNALS.iter().filter(|s| s.1 != 0) {
                if writeln!(out, "{n:2}) SIG{name}").is_err() {
                    return 1;
                }
            }
            return 0;
        }
        if list || args.is_empty() {
            let mut out = io::stdout().lock();
            let mut show = |sig| {
                if let Some(e) = self.entries.get(&sig) {
                    let name = if sig == DEBUG {
                        "DEBUG"
                    } else if sig == RETURN {
                        "RETURN"
                    } else if sig == ERR {
                        "ERR"
                    } else {
                        SIGNALS.iter().find(|s| s.1 == sig).unwrap().0
                    };
                    writeln!(
                        out,
                        "trap -- '{}' {}{name}",
                        e.command.replace('\'', "'\\''"),
                        if sig == 0 || sig >= DEBUG { "" } else { "SIG" }
                    )?;
                }
                Ok::<_, io::Error>(())
            };
            if args.len() < 2 {
                for &sig in self.entries.keys() {
                    if show(sig).is_err() {
                        return 1;
                    }
                }
            } else {
                for s in &args[1..] {
                    let Some(sig) = trap_number(s) else {
                        return invalid(s);
                    };
                    if show(sig).is_err() {
                        return 1;
                    }
                }
            }
            return 0;
        }
        let (command, signals) = if args[0].parse::<u32>().is_ok() {
            ("-", args)
        } else {
            (args[0].as_str(), &args[1..])
        };
        if signals.is_empty() {
            eprintln!("tonari: trap: expected a signal name");
            return 2;
        }
        let mut status = 0;
        for s in signals {
            if let Some(sig) = trap_number(s) {
                if let Err(e) = self.set(sig, command, interactive) {
                    eprintln!("tonari: trap: {s}: {e}");
                    status = 1;
                }
            } else {
                status = invalid(s);
            }
        }
        status
    }
}

fn invalid(s: &str) -> u8 {
    eprintln!("tonari: trap: {s}: invalid signal specification");
    1
}

pub fn hanging_up() -> bool {
    TERMINATING.load(Relaxed) == libc::SIGHUP
}

pub(crate) fn input_interrupted() -> bool {
    TERMINATING.load(Relaxed) != 0 || PENDING.load(Relaxed) != 0
}

pub fn pending() -> Option<i32> {
    let mask = PENDING.load(Relaxed);
    (mask != 0).then(|| mask.trailing_zeros() as i32)
}

pub fn debug(session: &mut Session, status: u8) -> Option<Flow> {
    if !session.traps.debug_handler() {
        return None;
    }
    session.traps.handling_debug = true;
    let result = action_text(session, DEBUG, status);
    session.traps.handling_debug = false;
    match result {
        Flow::Continue(2) if session.shopt.extdebug && session.function_depth > 0 => {
            Some(Flow::Return(2))
        }
        Flow::Continue(n) if session.shopt.extdebug && n != 0 => Some(Flow::Continue(n)),
        Flow::Continue(_) => None,
        other => Some(other),
    }
}

pub fn returning(session: &mut Session, flow: Flow) -> Flow {
    let status = match flow {
        Flow::Continue(code) => code,
        Flow::Return(_) => session.traps.before_return,
        _ => return flow,
    };
    if session.traps.handling_return || !session.traps.catches(RETURN) {
        return flow;
    }
    session.traps.handling_return = true;
    let result = action_text(session, RETURN, status);
    session.traps.handling_return = false;
    match result {
        Flow::Continue(_) => flow,
        other => other,
    }
}

pub fn error(session: &mut Session, status: u8) -> Flow {
    if !session.traps.error_handler() {
        return Flow::Continue(status);
    }
    session.traps.handling_error = true;
    let flow = action_text(session, ERR, status);
    session.traps.handling_error = false;
    match flow {
        Flow::Continue(_) => Flow::Continue(status),
        other => other,
    }
}

fn action_text(session: &mut Session, sig: i32, status: u8) -> Flow {
    let command = session.traps.entries[&sig].command.clone();
    let previous = session.pipe_status.clone();
    let flow = crate::eval_text(session, &command, status);
    session.pipe_status = previous;
    flow
}

pub fn dispatch(session: &mut Session, status: u8) -> Flow {
    if TERMINATING.load(Relaxed) != 0 {
        return Flow::Exit(status);
    }
    let mut pending = PENDING.load(Relaxed) & !session.traps.running;
    while pending != 0 {
        let sig = pending.trailing_zeros() as i32;
        let bit = 1 << sig;
        pending &= !bit;
        if PENDING.fetch_and(!bit, Relaxed) & bit == 0 {
            continue;
        }
        if let Some(command) = session.traps.entries.get(&sig).map(|e| e.command.clone()) {
            session.traps.running |= bit;
            let previous = session.pipe_status.clone();
            let flow = crate::eval_text(session, &command, status);
            session.pipe_status = previous;
            session.traps.running &= !bit;
            // Commands in a CHLD handler can create children of their own.
            if sig == libc::SIGCHLD {
                PENDING.fetch_and(!bit, Relaxed);
            }
            if !matches!(flow, Flow::Continue(_)) {
                return flow;
            }
        }
    }
    Flow::Continue(status)
}

pub fn finish(session: &mut Session, code: u8) -> u8 {
    if session.interactive
        && (TERMINATING.load(Relaxed) == libc::SIGHUP || session.login && session.shopt.huponexit)
    {
        session.jobs.hangup();
    }
    if !session.traps.active() {
        return code;
    }
    let mut code = match dispatch(session, code) {
        Flow::Exit(n) | Flow::Return(n) => n,
        _ => code,
    };
    let sig = TERMINATING.swap(0, Relaxed);
    if let Some(exit) = session.traps.entries.remove(&0) {
        let _ = session.traps.watch_exit(false);
        session.traps.exit_status = Some(code);
        if let Flow::Exit(n) = crate::eval_text(session, &exit.command, code) {
            code = n;
        }
        session.traps.exit_status = None;
    }
    if sig != 0 {
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
            libc::_exit(128 + sig);
        }
    }
    code
}

// An explicit wait is interruptible. Block signals while checking child and
// trap state, then atomically release the mask in sigsuspend. A foreground
// command instead finishes before Bash dispatches the trapped signal.
pub(crate) struct WaitMask {
    previous: libc::sigset_t,
    child: libc::sigaction,
    running: u64,
}

extern "C" fn wake(_: i32) {}

impl WaitMask {
    pub fn new(traps: &Traps) -> io::Result<Option<Self>> {
        if !traps.interactive_hup
            && traps.implicit.is_empty()
            && !traps
                .entries
                .iter()
                .any(|(&s, e)| s < ERR && s != 0 && s != libc::SIGCHLD && !e.command.is_empty())
        {
            return Ok(None);
        }
        Self::watch_children(traps).map(Some)
    }

    pub fn watch_children(traps: &Traps) -> io::Result<Self> {
        Self::for_children(traps.running)
    }

    pub(crate) fn for_children(running: u64) -> io::Result<Self> {
        unsafe {
            let mut mask = std::mem::zeroed();
            let mut previous = std::mem::zeroed();
            libc::sigfillset(&mut mask);
            if libc::sigprocmask(libc::SIG_BLOCK, &mask, &mut previous) < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut child = std::mem::zeroed();
            let res = libc::sigaction(libc::SIGCHLD, std::ptr::null(), &mut child);
            if res < 0
                || (child.sa_sigaction == libc::SIG_DFL || child.sa_sigaction == libc::SIG_IGN)
                    && libc::sigaction(
                        libc::SIGCHLD,
                        &action(wake as *const () as usize),
                        std::ptr::null_mut(),
                    ) < 0
            {
                let error = io::Error::last_os_error();
                libc::sigprocmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut());
                return Err(error);
            }
            Ok(Self {
                previous,
                child,
                running,
            })
        }
    }

    pub fn interrupt_status(&self) -> Option<u8> {
        let sig = TERMINATING.load(Relaxed);
        if sig != 0 {
            return Some(128 + sig as u8);
        }
        let bits = PENDING.load(Relaxed) & !(self.running | (1 << libc::SIGCHLD));
        (bits != 0).then(|| 128 + bits.trailing_zeros() as u8)
    }

    pub(crate) fn input_or_child(&self) -> io::Result<bool> {
        let mut fds = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut fds);
            libc::FD_SET(0, &mut fds);
            if libc::pselect(
                1,
                &mut fds,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
                &self.previous,
            ) >= 0
            {
                return Ok(true);
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
            libc::sigsuspend(&self.previous);
        }
    }
}

impl Drop for WaitMask {
    fn drop(&mut self) {
        unsafe {
            libc::sigaction(libc::SIGCHLD, &self.child, std::ptr::null_mut());
            libc::sigprocmask(libc::SIG_SETMASK, &self.previous, std::ptr::null_mut());
        }
    }
}
