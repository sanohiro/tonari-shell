mod signals;
pub(crate) use signals::{number, print_signal_table, signals};
mod wait_mask;
pub(crate) const fn signal_status(signal: i32) -> u8 {
    (128 + signal) as u8
}
use wait_mask::is_os_signal;
pub(crate) use wait_mask::{SignalMask, WaitMask};
// A signal handler can run while Rust holds allocator and I/O locks. It only
// records pending events. The interpreter executes shell code at a command boundary.
use crate::{Flow, Session};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering::Relaxed};

static TERMINATING: AtomicI32 = AtomicI32::new(0);
extern "C" fn terminate(sig: i32) {
    let _ = TERMINATING.compare_exchange(0, sig, Relaxed, Relaxed);
}

// The highest user signal on Linux is 64. EXIT is synthetic and never
// enters these masks, leaving exactly one bit for each real signal.
const fn signal_bit(sig: i32) -> u64 {
    1u64 << (sig - 1)
}

static PENDING: AtomicU64 = AtomicU64::new(0);
extern "C" fn record(sig: i32) {
    PENDING.fetch_or(signal_bit(sig), Relaxed);
}

// A single kernel SIGCHLD can cover several waitable children. Count the
// statuses consumed by the interpreter, including when wait blocks on a
// different PID. A bare signal wakes the interpreter to check for statuses.
static WATCH_CHILDREN: AtomicBool = AtomicBool::new(false);
static CHILD_EVENTS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn waitpid(pid: libc::pid_t, status: &mut i32, flags: i32) -> libc::pid_t {
    let got = unsafe { libc::waitpid(pid, status, flags) };
    if got > 0 && WATCH_CHILDREN.load(Relaxed) && CHILD_EVENTS.fetch_add(1, Relaxed) == 0 {
        record(libc::SIGCHLD);
    }
    got
}

#[derive(Clone, Copy)]
pub(crate) enum ChildEvent {
    Exited(u8),
    Signaled(i32),
    Stopped(i32),
    Continued,
}

impl ChildEvent {
    // The input is the status word from a successful waitpid.
    pub(crate) fn decode(status: i32) -> Self {
        if libc::WIFEXITED(status) {
            Self::Exited(libc::WEXITSTATUS(status) as u8)
        } else if libc::WIFSTOPPED(status) {
            Self::Stopped(libc::WSTOPSIG(status))
        } else if libc::WIFCONTINUED(status) {
            Self::Continued
        } else {
            Self::Signaled(libc::WTERMSIG(status))
        }
    }

    pub(crate) fn exit_status(self) -> Option<u8> {
        match self {
            Self::Exited(code) => Some(code),
            Self::Signaled(signal) => Some(signal_status(signal)),
            Self::Stopped(_) | Self::Continued => None,
        }
    }
}

/// Retry interrupted waits while accounting for trapped SIGCHLD.
pub(crate) fn child_event(pid: libc::pid_t, flags: i32) -> io::Result<Option<ChildEvent>> {
    let mut status = 0;
    loop {
        match waitpid(pid, &mut status, flags) {
            0 => return Ok(None),
            -1 => {
                let err = io::Error::last_os_error();
                if err.kind() != io::ErrorKind::Interrupted {
                    return Err(err);
                }
            }
            _ => return Ok(Some(ChildEvent::decode(status))),
        }
    }
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
    command: std::rc::Rc<str>,
    original: libc::sigaction,
}

#[derive(Default, Clone)]
pub struct Traps {
    entries: BTreeMap<i32, Entry>,
    inherited_listing: Option<std::rc::Rc<BTreeMap<i32, Entry>>>,
    running: u64,
    handling_debug: bool,
    handling_error: bool,
    handling_return: bool,
    handling_exit: bool,
    interactive_hup: bool,
    interactive_handlers: bool,
    interactive_origin: bool,
    pub before_return: u8,
    current_action: Option<ActionStatus>,
    async_resettable: u64,
    implicit: Vec<(i32, libc::sigaction)>,
}

#[derive(Clone, Copy)]
struct ActionStatus {
    signal: i32,
    scope_depth: u32,
    status: u8,
}

fn action(handler: usize) -> libc::sigaction {
    let mut a: libc::sigaction = unsafe { std::mem::zeroed() };
    a.sa_sigaction = handler;
    unsafe { libc::sigemptyset(&mut a.sa_mask) };
    a
}

// Stable Rust overrides SIGPIPE before main. Save the disposition inherited at
// exec for developer builds. Distribution builds ask rustc to inherit it.
#[cfg(not(tonari_inherit_sigpipe))]
mod entry_pipe {
    use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
    static IGNORED: AtomicBool = AtomicBool::new(false);

    #[used]
    #[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]
    #[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__mod_init_func"))]
    static SAVE: extern "C" fn() = {
        extern "C" fn save() {
            let mut old = std::mem::MaybeUninit::uninit();
            unsafe {
                if libc::sigaction(libc::SIGPIPE, std::ptr::null(), old.as_mut_ptr()) == 0 {
                    IGNORED.store(old.assume_init().sa_sigaction == libc::SIG_IGN, Relaxed);
                }
            }
        }
        save
    };

    pub(super) fn restore() {
        if !IGNORED.load(Relaxed) {
            unsafe {
                libc::signal(libc::SIGPIPE, libc::SIG_DFL);
            }
        }
    }
    pub(super) fn inherited_ignore() -> bool {
        IGNORED.load(Relaxed)
    }
}

pub fn enter_process() {
    #[cfg(not(tonari_inherit_sigpipe))]
    entry_pipe::restore();
    // std installs its stack-overflow handlers only over SIG_DFL. An explicit
    // kill delivers just one signal, so returning from that handler loses it.
    reset_caught_signals(&[libc::SIGBUS, libc::SIGSEGV]);
}

pub(crate) fn reset_caught_signals(signals: &[i32]) {
    for &sig in signals {
        let mut old = std::mem::MaybeUninit::uninit();
        unsafe {
            if libc::sigaction(sig, std::ptr::null(), old.as_mut_ptr()) == 0
                && !matches!(
                    old.assume_init().sa_sigaction,
                    libc::SIG_DFL | libc::SIG_IGN
                )
            {
                libc::signal(sig, libc::SIG_DFL);
            }
        }
    }
}

pub fn std_spawn_inherits_pipe(traps: &Traps) -> bool {
    #[cfg(not(tonari_inherit_sigpipe))]
    {
        !entry_pipe::inherited_ignore() && !traps.ignores(libc::SIGPIPE)
    }
    #[cfg(tonari_inherit_sigpipe)]
    {
        let _ = traps;
        true
    }
}

impl Traps {
    pub(crate) fn in_nested_action(&self) -> bool {
        self.handling_debug || self.handling_error || self.handling_return
    }

    pub fn in_action_child(&self, scope_depth: u32) -> bool {
        self.current_action
            .is_some_and(|action| scope_depth > action.scope_depth)
    }

    pub fn in_action(&self) -> bool {
        self.current_action.is_some()
    }

    pub fn implicit_status(&self, scope_depth: u32, exiting: bool) -> Option<u8> {
        let action = self.current_action?;
        (action.signal != DEBUG
            && (scope_depth == action.scope_depth || exiting && action.signal == 0))
            .then_some(action.status)
    }

    pub fn has_explicit_traps(&self) -> bool {
        !self.entries.is_empty()
    }

    pub fn active(&self) -> bool {
        self.has_explicit_traps() || self.interactive_hup
    }

    pub fn init_hangup(&mut self) -> io::Result<()> {
        self.interactive_origin = true;
        self.interactive_handlers = true;
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

    pub fn ignore_async(&mut self, sig: i32) {
        let old = unsafe { libc::signal(sig, libc::SIG_IGN) };
        if old != libc::SIG_IGN && old != libc::SIG_ERR {
            self.async_resettable |= signal_bit(sig);
        }
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
                if old.sa_sigaction != libc::SIG_DFL {
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
                self.implicit.push((sig, old));
            }
        }
        Ok(())
    }

    fn implicit_action(&self, sig: i32) -> Option<&libc::sigaction> {
        self.implicit
            .iter()
            .find(|(n, _)| *n == sig)
            .map(|(_, old)| old)
    }

    fn set(&mut self, sig: i32, command: &str, interactive: bool) -> io::Result<()> {
        self.inherited_listing = None;
        if sig == 0 {
            self.watch_exit(command != "-" && !command.is_empty())?;
        }
        let real = is_os_signal(sig) && sig != libc::SIGSTOP && sig != libc::SIGKILL;
        let mut original = action(libc::SIG_DFL);
        if let Some(old) = self.entries.get(&sig) {
            original = old.original;
        } else if let Some(old) = self.implicit_action(sig) {
            original = *old;
        } else if real {
            if unsafe { libc::sigaction(sig, std::ptr::null(), &mut original) } < 0 {
                return Err(io::Error::last_os_error());
            }
            if original.sa_sigaction == libc::SIG_IGN {
                if !interactive
                    && !self.interactive_origin
                    && self.async_resettable & (signal_bit(sig)) == 0
                {
                    return Ok(());
                }
                original = if self.interactive_handlers
                    && matches!(sig, libc::SIGINT | libc::SIGQUIT | libc::SIGTERM)
                {
                    action(crate::interrupt as *const () as usize)
                } else {
                    action(libc::SIG_DFL)
                };
            }
        }
        if real {
            let new = match command {
                "-" if self.implicit_action(sig).is_some()
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
        if sig == libc::SIGCHLD {
            let watch = command != "-" && !command.is_empty();
            if !watch {
                CHILD_EVENTS.store(0, Relaxed);
            }
            WATCH_CHILDREN.store(watch && self.running & signal_bit(sig) == 0, Relaxed);
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
    pub fn return_handler(&self) -> bool {
        !self.handling_return && !self.handling_debug && self.catches(RETURN)
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
        self.handling_exit = false;
        self.current_action = None;
        // A still-running inherited handler must not re-enter itself.
        self.handling_debug &= functrace;
        self.handling_return &= functrace;
        self.handling_error &= errtrace;
        self.running = 0;
        PENDING.store(0, Relaxed);
        TERMINATING.store(0, Relaxed);
        WATCH_CHILDREN.store(false, Relaxed);
        CHILD_EVENTS.store(0, Relaxed);
        if self.interactive_handlers {
            self.interactive_handlers = false;
            reset_caught_signals(&[libc::SIGINT, libc::SIGQUIT, libc::SIGTERM, libc::SIGTSTP]);
        }
        if self.interactive_hup {
            self.interactive_hup = false;
            if !self.ignores(libc::SIGHUP) {
                unsafe {
                    libc::signal(libc::SIGHUP, libc::SIG_DFL);
                }
            }
        }
        if self.entries.is_empty() {
            return;
        }
        let _ = self.watch_exit(false);
        // Bash exposes the parent list to trap -p until the child changes a trap.
        // Only ignored dispositions and inherited shell handlers remain active.
        let old = std::mem::take(&mut self.entries);
        for (&sig, e) in &old {
            if self.keep_after_fork(sig, e, errtrace, functrace) {
                let mut child = e.clone();
                if is_os_signal(sig) && child.original.sa_sigaction != libc::SIG_IGN {
                    child.original = action(libc::SIG_DFL);
                }
                self.entries.insert(sig, child);
            }
        }
        if self.inherited_listing.is_none() {
            self.inherited_listing = Some(std::rc::Rc::new(old));
        }
    }

    fn keep_after_fork(&self, sig: i32, e: &Entry, errtrace: bool, functrace: bool) -> bool {
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
    }

    pub(crate) fn list<'a>(
        &self,
        names: impl ExactSizeIterator<Item = &'a str>,
        out: &mut dyn Write,
        defaults: bool,
        action_only: bool,
        posix: bool,
    ) -> u8 {
        let entries = self.inherited_listing.as_deref().unwrap_or(&self.entries);
        let mut show = |sig| {
            if let Some(e) = entries.get(&sig) {
                if action_only {
                    return writeln!(out, "{}", e.command);
                }
                let name = if sig == DEBUG {
                    "DEBUG"
                } else if sig == RETURN {
                    "RETURN"
                } else if sig == ERR {
                    "ERR"
                } else {
                    signals().find(|s| s.1 == sig).unwrap().0
                };
                writeln!(
                    out,
                    "trap -- {} {}{name}",
                    crate::word::quote::single_quote(&e.command),
                    if posix || sig == 0 || sig >= DEBUG {
                        ""
                    } else {
                        "SIG"
                    }
                )?;
            }
            if !entries.contains_key(&sig)
                && is_os_signal(sig)
                && let Some((name, _)) = signals().find(|s| s.1 == sig)
            {
                let ignored = if let Some(old) = self.implicit_action(sig) {
                    old.sa_sigaction == libc::SIG_IGN
                } else {
                    let mut old = std::mem::MaybeUninit::uninit();
                    unsafe {
                        libc::sigaction(sig, std::ptr::null(), old.as_mut_ptr()) == 0
                            && old.assume_init().sa_sigaction == libc::SIG_IGN
                    }
                };
                if !action_only && (defaults || ignored) {
                    writeln!(
                        out,
                        "trap -- {} {}{name}",
                        if ignored { "''" } else { "-" },
                        if posix { "" } else { "SIG" }
                    )?;
                }
            }
            Ok::<_, io::Error>(())
        };
        if names.len() == 0 {
            if entries.contains_key(&0) && show(0).is_err() {
                return 1;
            }
            for (_, sig) in signals() {
                if sig > 0 && !matches!(sig, libc::SIGSTOP | libc::SIGKILL) && show(sig).is_err() {
                    return 1;
                }
            }
            for &sig in entries.keys().filter(|s| **s >= DEBUG) {
                if show(sig).is_err() {
                    return 1;
                }
            }
        } else {
            let mut status = 0;
            for s in names {
                let Some(sig) = trap_number(s) else {
                    status = invalid(s);
                    continue;
                };
                if show(sig).is_err() {
                    return 1;
                }
            }
            return status;
        }
        0
    }

    pub fn builtin(&mut self, args: &[String], interactive: bool, posix: bool) -> u8 {
        let mut args = args;
        let (mut list, mut actions, mut signals) = (false, false, false);
        while let Some(arg) = crate::builtins::flags::next(&mut args) {
            for flag in arg[1..].bytes() {
                match flag {
                    b'p' => list = true,
                    b'P' => actions = true,
                    b'l' => signals = true,
                    _ => {
                        diagnostic!("trap: {arg}: invalid option");
                        return 2;
                    }
                }
            }
        }
        if actions && (list || args.is_empty()) {
            diagnostic!("trap: -P requires signal names and cannot be combined with -p");
            return 2;
        }
        if signals {
            let mut out = io::stdout().lock();
            return u8::from(print_signal_table(&mut out).is_err());
        }
        if list || actions || args.is_empty() {
            return self.list(
                args.iter().map(String::as_str),
                &mut io::stdout().lock(),
                list && posix,
                actions,
                posix,
            );
        }
        let (command, signals) = if args[0].parse::<u32>().is_ok()
            || !posix && args.len() == 1 && trap_number(&args[0]).is_some()
        {
            ("-", args)
        } else {
            (args[0].as_str(), &args[1..])
        };
        if signals.is_empty() {
            diagnostic!("trap: expected a signal name");
            return 2;
        }
        let mut status = 0;
        for s in signals {
            if let Some(sig) = trap_number(s) {
                if let Err(e) = self.set(sig, command, interactive) {
                    diagnostic!("trap: {s}: {e}");
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
    diagnostic!("trap: {s}: invalid signal specification");
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
    (mask != 0).then(|| 1 + mask.trailing_zeros() as i32)
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
    if !session.traps.return_handler() {
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

// GNU Bash reserves the three numbers immediately following its OS signal table.
fn with_trapsig(
    session: &mut Session,
    sig: i32,
    status: u8,
    run: impl FnOnce(&mut Session) -> Flow,
) -> Flow {
    #[cfg(target_os = "macos")]
    const NSIG: i32 = 32;
    #[cfg(target_os = "linux")]
    const NSIG: i32 = 65;
    let number = match sig {
        DEBUG => NSIG,
        ERR => NSIG + 1,
        RETURN => NSIG + 2,
        other => other,
    };
    let previous = session.traps.current_action.replace(ActionStatus {
        signal: sig,
        scope_depth: session.function_depth + session.source_depth,
        status,
    });
    let flow = session.with_trap_signal(number, run);
    session.traps.current_action = previous;
    flow
}

fn action_text(session: &mut Session, sig: i32, status: u8) -> Flow {
    let command = session.traps.entries[&sig].command.clone();
    let previous = session.pipe_status.clone();
    let flow = with_trapsig(session, sig, status, |session| {
        crate::input::text(session, &command, status, false, false, Some(1))
    });
    session.pipe_status = previous;
    flow
}

pub fn dispatch(session: &mut Session, status: u8) -> Flow {
    if TERMINATING.load(Relaxed) != 0 {
        return Flow::Exit(status);
    }
    let mut pending = PENDING.load(Relaxed) & !session.traps.running;
    while pending != 0 {
        let sig = 1 + pending.trailing_zeros() as i32;
        let bit = signal_bit(sig);
        pending &= !bit;
        if PENDING.fetch_and(!bit, Relaxed) & bit == 0 {
            continue;
        }
        if let Some(command) = session.traps.entries.get(&sig).map(|e| e.command.clone()) {
            let count = if sig == libc::SIGCHLD {
                session.jobs.poll();
                session.substitutions.finish(session.substitutions.scope());
                let n = CHILD_EVENTS.swap(0, Relaxed);
                if n == 0 {
                    continue;
                }
                // Children synchronously reaped by the action must not
                // recursively trigger the same action.
                WATCH_CHILDREN.store(false, Relaxed);
                n
            } else {
                1
            };
            session.traps.running |= bit;
            let previous = session.pipe_status.clone();
            let previous_line = session.signal_line.replace(1);
            let mut flow = Flow::Continue(status);
            for _ in 0..count {
                flow = with_trapsig(session, sig, status, |session| {
                    crate::input::text(session, &command, status, false, false, Some(1))
                });
                if !matches!(flow, Flow::Continue(_)) || count > 1 && !session.traps.catches(sig) {
                    break;
                }
            }
            session.signal_line = previous_line;
            session.pipe_status = previous;
            session.traps.running &= !bit;
            if sig == libc::SIGCHLD {
                WATCH_CHILDREN.store(session.traps.catches(sig), Relaxed);
            }
            if !matches!(flow, Flow::Continue(_)) {
                return flow;
            }
        }
    }
    Flow::Continue(status)
}

// An explicit exit runs with its command redirections, positional parameters
// and dynamically scoped locals still in effect. Natural EOF uses this too.
pub(crate) fn exit_action(session: &mut Session, code: u8) -> u8 {
    if session.traps.handling_exit {
        return code;
    }
    let Some(exit) = session.traps.entries.remove(&0) else {
        return code;
    };
    session.traps.handling_exit = true;
    let _ = session.traps.watch_exit(false);
    let outcome = with_trapsig(session, 0, code, |session| {
        crate::eval_text(session, &exit.command, code)
    });
    session.traps.handling_exit = false;
    if session.traps.entries.remove(&0).is_some() {
        let _ = session.traps.watch_exit(false);
    }
    if let Flow::Exit(n) = outcome { n } else { code }
}

pub fn finish(session: &mut Session, code: u8) -> u8 {
    if session.interactive
        && (TERMINATING.load(Relaxed) == libc::SIGHUP || session.login && session.shopt.huponexit)
    {
        session.jobs.hangup();
    }
    if !session.traps.active() && TERMINATING.load(Relaxed) == 0 {
        return code;
    }
    let mut code = match dispatch(session, code) {
        Flow::Exit(n) | Flow::Return(n) => n,
        _ => code,
    };
    let sig = TERMINATING.swap(0, Relaxed);
    code = exit_action(session, code);
    if sig != 0 {
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
            libc::_exit(128 + sig);
        }
    }
    code
}
