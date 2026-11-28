mod builtins;
use crate::Session;
use crate::traps::{ChildEvent, signal_status};

// Completed jobs and process substitutions remain available to a later wait by PID.
pub(crate) const MAX_FINISHED_STATUSES: usize = 4096;
use crate::execute::fd::PrivateFile;
pub use builtins::dispatch;
use builtins::*;
use libc::pid_t;
use std::io::{self, Write};
use std::os::fd::{AsFd, AsRawFd, RawFd};

#[derive(Clone)]
struct Terminal {
    fd: PrivateFile,
    // The owning shell and the process group to restore after a foreground job.
    shell_group: pid_t,
    original_group: pid_t,
    owner: pid_t,
}

#[derive(Clone, Copy, Default)]
pub struct Launch {
    pub group: Option<pid_t>,
    tty: Option<RawFd>,
    pub background: bool,
    pub single: bool,
    defer_tty: bool,
    shell_modes: Option<libc::termios>,
}

pub(crate) const JOB_SIGNALS: [i32; 5] = [
    libc::SIGTSTP,
    libc::SIGTTIN,
    libc::SIGTTOU,
    libc::SIGINT,
    libc::SIGQUIT,
];

impl Launch {
    pub fn can_spawn_group(self) -> bool {
        if self.tty.is_none() || self.defer_tty {
            return true;
        }
        #[cfg(target_os = "macos")]
        {
            // A blocked SIGCONT would remain pending in the new executable.
            let mut mask = std::mem::MaybeUninit::uninit();
            unsafe {
                libc::sigprocmask(libc::SIG_BLOCK, std::ptr::null(), mask.as_mut_ptr()) == 0
                    && libc::sigismember(mask.as_ptr(), libc::SIGCONT) == 0
            }
        }
        #[cfg(not(target_os = "macos"))]
        false
    }

    #[cfg(target_os = "macos")]
    pub fn spawn_terminal(self) -> Option<RawFd> {
        self.tty.filter(|_| !self.defer_tty)
    }

    #[cfg(target_os = "macos")]
    pub fn resume_spawned(self, pid: pid_t) {
        if let Some(fd) = self.spawn_terminal() {
            // Darwin starts the new image suspended. Give it the terminal
            // before its first instruction, including a read in an initializer.
            // Consume Darwin's synthetic stopped event before sending SIGCONT.
            // WSTOPPED leaves a real exit available to the normal job waiter.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            loop {
                let result = unsafe {
                    libc::waitid(
                        libc::P_PID,
                        pid as _,
                        &mut info,
                        libc::WSTOPPED | libc::WNOHANG,
                    )
                };
                if result == 0 && info.si_pid == 0 {
                    // The scheduler has not posted the synthetic stop yet. An
                    // intervening exit must remain waitable by the job owner.
                    if unsafe {
                        libc::waitid(
                            libc::P_PID,
                            pid as _,
                            &mut info,
                            libc::WEXITED | libc::WSTOPPED | libc::WNOWAIT,
                        )
                    } == 0
                    {
                        if info.si_code == libc::CLD_STOPPED {
                            continue;
                        }
                        return;
                    }
                } else if result == 0 {
                    break;
                }
                if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                    break;
                }
            }
            give_terminal(fd, self.group.filter(|&g| g != 0).unwrap_or(pid));
            unsafe { libc::kill(pid, libc::SIGCONT) };
        }
    }

    pub fn pipeline_child(self, traps: &mut crate::traps::Traps) {
        if self.group.is_none() && !self.background {
            crate::execute::reset_child_signals();
        }
        self.child(traps);
    }

    pub fn child(self, traps: &mut crate::traps::Traps) {
        if let Some(group) = self.group {
            if !assign_process_group(0, group) {
                unsafe { libc::_exit(1) }
            }
            if let Some(tty) = self.tty.filter(|_| !self.defer_tty) {
                give_terminal(tty, unsafe { libc::getpgrp() });
            }
            crate::traps::reset_caught_signals(&JOB_SIGNALS);
        } else if self.background {
            for sig in [libc::SIGINT, libc::SIGQUIT] {
                traps.ignore_async(sig);
            }
        }
    }

    pub fn parent(&mut self, pid: pid_t) {
        self.record_child(pid);
        if let Some(group) = self.group {
            assign_process_group(pid, group);
        }
    }

    pub fn record_child(&mut self, pid: pid_t) {
        if self.group == Some(0) {
            self.group = Some(pid);
        }
    }
}

fn assign_process_group(pid: pid_t, group: pid_t) -> bool {
    (unsafe { libc::setpgid(pid, group) == 0 }) || group_assignment_raced(pid, group)
}

#[cold]
fn group_assignment_raced(pid: pid_t, group: pid_t) -> bool {
    if io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
        return false;
    }
    let leader = if pid == 0 {
        unsafe { libc::getpid() }
    } else {
        pid
    };
    let target = if group == 0 { leader } else { group };
    // Darwin can return EPERM to one caller while the other is creating the
    // same group. Its new membership may not yet be visible to getpgid.
    for attempt in 0..4 {
        let current = unsafe { libc::getpgid(pid) };
        if current == target {
            return true;
        }
        if current < 0 || target != leader {
            return false;
        }
        if attempt < 2 {
            unsafe { libc::sched_yield() };
        } else {
            let delay = libc::timespec {
                tv_sec: 0,
                tv_nsec: 100_000,
            };
            unsafe { libc::nanosleep(&delay, std::ptr::null_mut()) };
        }
        if unsafe { libc::setpgid(pid, group) } == 0 {
            return true;
        }
        if io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
            break;
        }
    }
    unsafe { libc::getpgid(pid) == target }
}

fn terminal_modes(fd: RawFd) -> Option<libc::termios> {
    let mut modes = std::mem::MaybeUninit::uninit();
    if unsafe { libc::tcgetattr(fd, modes.as_mut_ptr()) } == 0 {
        Some(unsafe { modes.assume_init() })
    } else {
        None
    }
}

fn give_terminal(fd: RawFd, pgid: pid_t) {
    give_terminal_with_modes(fd, pgid, None);
}

#[cfg(test)]
#[test]
fn group_setup_accepts_only_the_requested_group_after_a_failed_setpgid() {
    let outer_group = unsafe { libc::getpgrp() };
    for (target, expected) in [(0, 0), (outer_group, 1)] {
        let launch = Launch {
            group: Some(target),
            ..Default::default()
        };
        let mut traps = crate::traps::Traps::default();
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            // A session leader always gets EPERM from setpgid, even for its own group.
            if unsafe { libc::setsid() } < 0 {
                unsafe { libc::_exit(2) }
            }
            launch.child(&mut traps);
            unsafe { libc::_exit(0) }
        }
        assert_eq!(crate::execute::wait(pid).unwrap(), expected);
    }
}

fn give_terminal_with_modes(fd: RawFd, pgid: pid_t, modes: Option<&libc::termios>) {
    // The shell is temporarily in the background when it takes the terminal back.
    unsafe {
        let mut block = std::mem::zeroed();
        libc::sigemptyset(&mut block);
        libc::sigaddset(&mut block, libc::SIGTTOU);
        let Ok(_mask) = crate::traps::SignalMask::block(&block) else {
            return;
        };
        while libc::tcsetpgrp(fd, pgid) < 0 {
            if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                return;
            }
        }
        if let Some(modes) = modes {
            while libc::tcsetattr(fd, libc::TCSANOW, modes) < 0
                && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
            {}
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Running,
    Stopped(u8),
    Done(u8),
}
#[derive(Clone)]
struct Process {
    pid: pid_t,
    signal: Option<i32>,
    state: State,
}
#[derive(Clone)]
struct Job {
    id: u32,
    foreground_notice: bool,
    stopped_at: u64,
    group: Option<pid_t>,
    children: Vec<Process>,
    name: String,
    pipefail: bool,
    modes: Option<libc::termios>,
    signaled: bool,
    listed: bool,
    nohup: bool,
}

fn signal_description(signal: i32) -> Option<String> {
    let description = unsafe { libc::strsignal(signal) };
    (!description.is_null()).then(|| {
        unsafe { std::ffi::CStr::from_ptr(description) }
            .to_string_lossy()
            .into_owned()
    })
}

impl Process {
    fn long_state(&self, posix: bool) -> String {
        match self.state {
            State::Running => "Running".into(),
            State::Stopped(code) => {
                signal_description(i32::from(code) - 128).unwrap_or_else(|| "Stopped".into())
            }
            State::Done(code) => {
                self.signal
                    .and_then(signal_description)
                    .unwrap_or_else(|| match code {
                        0 => "Done".into(),
                        _ if posix => format!("Done({code})"),
                        _ => format!("Exit {code}"),
                    })
            }
        }
    }
}

impl Job {
    fn hangup(&self) {
        if self.nohup {
            return;
        }
        let send = |target, stopped| unsafe {
            libc::kill(target, libc::SIGHUP);
            if stopped {
                libc::kill(target, libc::SIGCONT);
            }
        };
        if let Some(group) = self.group {
            if self.active() || self.stopped() {
                send(-group, self.stopped());
            }
        } else {
            for child in &self.children {
                if child.pid > 0 && !matches!(child.state, State::Done(_)) {
                    send(child.pid, matches!(child.state, State::Stopped(_)));
                }
            }
        }
    }
    fn active(&self) -> bool {
        self.children.iter().any(|p| p.state == State::Running)
    }
    fn stopped(&self) -> bool {
        self.children
            .iter()
            .any(|p| matches!(p.state, State::Stopped(_)))
    }
    fn display_state(&self, posix: bool) -> std::borrow::Cow<'static, str> {
        if self.active() {
            return "Running".into();
        }
        if let Some(code) = self.children.iter().rev().find_map(|p| match p.state {
            State::Stopped(code) => Some(code),
            _ => None,
        }) {
            if posix
                && let Some((name, _)) =
                    crate::traps::signals().find(|(_, sig)| signal_status(*sig) == code)
            {
                return format!("Stopped(SIG{name})").into();
            }
            return "Stopped".into();
        }
        if let Some(reason) = self
            .children
            .last()
            .and_then(|child| child.signal)
            .and_then(signal_description)
        {
            return reason.into();
        }
        match self.status() {
            0 => "Done".into(),
            code if posix => format!("Done({code})").into(),
            code => format!("Exit {code}").into(),
        }
    }

    fn last_pid(&self) -> Option<pid_t> {
        self.children
            .iter()
            .rev()
            .find(|p| p.pid > 0)
            .map(|p| p.pid)
    }

    fn status(&self) -> u8 {
        for p in self.children.iter().rev() {
            match p.state {
                State::Stopped(code) => return code,
                State::Done(code) if !self.pipefail || code != 0 => return code,
                _ => {}
            }
        }
        0
    }
    fn update(&mut self, pid: pid_t, event: ChildEvent) {
        crate::traps::job_changed();
        if let Some(p) = self.children.iter_mut().find(|p| p.pid == pid) {
            self.listed = false;
            p.state = match event {
                ChildEvent::Stopped(signal) => State::Stopped(signal_status(signal)),
                ChildEvent::Continued => State::Running,
                ChildEvent::Signaled(signal) => {
                    self.signaled = true;
                    p.signal = Some(signal);
                    State::Done(signal_status(signal))
                }
                ChildEvent::Exited(code) => State::Done(code),
            };
            if matches!(event, ChildEvent::Continued) || self.stopped_at != 0 && !self.stopped() {
                self.stopped_at = 0;
            } else if self.stopped_at == 0
                && (matches!(event, ChildEvent::Stopped(_)) || self.children.len() > 1)
                && !self.active()
                && self.stopped()
            {
                thread_local! { static NEXT_STOP: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }
                self.stopped_at = NEXT_STOP.get() + 1;
                NEXT_STOP.set(self.stopped_at);
            }
        }
    }
    fn wait(
        &mut self,
        mask: Option<&crate::traps::WaitMask>,
        force: bool,
        foreground_tty: bool,
    ) -> io::Result<Option<u8>> {
        let mut group = self.group;
        while self.active() || (force && self.stopped()) {
            if foreground_tty && crate::traps::hanging_up() {
                self.hangup();
                return Ok(Some(crate::traps::signal_status(libc::SIGHUP)));
            }
            if let Some(code) = mask.and_then(crate::traps::WaitMask::interrupt_status) {
                return Ok(Some(code));
            }
            let pid = group.map(|g| -g).unwrap_or_else(|| {
                self.children
                    .iter()
                    .find(|p| {
                        p.state == State::Running || (force && matches!(p.state, State::Stopped(_)))
                    })
                    .unwrap()
                    .pid
            });
            let mut status = 0;
            let flags =
                libc::WUNTRACED | libc::WCONTINUED | if mask.is_some() { libc::WNOHANG } else { 0 };
            let got = crate::traps::waitpid(pid, &mut status, flags);
            if got == 0 {
                mask.unwrap().suspend();
                continue;
            }
            if got < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if e.raw_os_error() == Some(libc::ECHILD) {
                    if group.take().is_none() {
                        self.update(pid, ChildEvent::Exited(127));
                    }
                    continue;
                }
                return Err(e);
            }
            self.update(got, ChildEvent::decode(status));
        }
        Ok(None)
    }
    fn resume(&mut self) -> io::Result<()> {
        if let Some(group) = self.group
            && unsafe { libc::kill(-group, libc::SIGCONT) } < 0
        {
            return Err(io::Error::last_os_error());
        }
        self.stopped_at = 0;
        for p in &mut self.children {
            if matches!(p.state, State::Stopped(_)) {
                p.state = State::Running;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct Jobs {
    pub monitor: bool,
    pub notify: bool,
    pub(crate) posix_interactive: bool,
    pub(crate) running_prompt_hook: bool,
    exit_warning: Option<usize>,
    pub(crate) foreground_interrupted: bool,
    pub(crate) quiet: bool,
    tty: Option<Terminal>,
    table: Vec<Job>,
    detached: Vec<pid_t>,
    inherited: Vec<Job>,
    // Bash keeps completed background statuses available to wait by PID.
    finished: std::collections::VecDeque<(pid_t, u8)>,
}

impl Jobs {
    fn all_jobs(&self) -> impl DoubleEndedIterator<Item = &Job> + Clone {
        self.inherited.iter().chain(&self.table)
    }

    pub(crate) fn defer_exit(&mut self, check_running: bool, command: usize, posix: bool) -> bool {
        if self.table.is_empty()
            || self
                .exit_warning
                .is_some_and(|n| command.wrapping_sub(n) <= 1)
        {
            return false;
        }
        self.poll();
        if self.table.iter().any(Job::stopped) {
            stderr_line!("There are stopped jobs.");
        } else if check_running && self.table.iter().any(Job::active) {
            stderr_line!("There are running jobs.");
        } else {
            return false;
        }
        self.exit_warning = Some(command);
        if check_running {
            list_to(self, &[], posix, &mut io::stderr().lock());
        }
        true
    }

    pub(crate) fn count(&self) -> usize {
        self.table.len() + self.inherited.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.table.is_empty() && self.inherited.is_empty()
    }

    pub fn init(&mut self, interactive: bool) {
        if self.tty.is_some() {
            return;
        }
        let old = unsafe { libc::getpgrp() };
        let fd = if unsafe { libc::isatty(libc::STDERR_FILENO) } == 1 {
            PrivateFile::duplicate(io::stderr().as_fd())
        } else {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/tty")
                .and_then(PrivateFile::new)
        };
        let Ok(fd) = fd else { return };
        let mut stops = 0;
        loop {
            let foreground = unsafe { libc::tcgetpgrp(fd.as_raw_fd()) };
            if foreground == old {
                break;
            }
            // Noninteractive callers keep running without controlling a tty.
            // Bound retries for an orphaned group or a blocked SIGTTIN.
            if !interactive
                || foreground < 0
                || stops == 16
                || crate::traps::stop_for_terminal().is_err()
            {
                return;
            }
            stops += 1;
        }
        let owner = unsafe { libc::getpid() };
        let shell_group = if interactive { owner } else { old };
        if old != shell_group {
            if unsafe { libc::setpgid(0, 0) } < 0 {
                return;
            }
            give_terminal(fd.as_raw_fd(), owner);
        }
        // A caught disposition becomes default on exec. Preserve a signal
        // explicitly ignored by our parent across child process creation.
        extern "C" fn stay_running(_: i32) {}
        if crate::traps::disposition(libc::SIGTSTP)
            .is_ok_and(|old| old.sa_sigaction != libc::SIG_IGN)
        {
            unsafe {
                libc::signal(libc::SIGTSTP, stay_running as *const () as usize);
            }
        }
        self.tty = Some(Terminal {
            fd,
            owner,
            shell_group,
            original_group: old,
        });
    }
    fn terminal(&self) -> Option<&Terminal> {
        self.tty
            .as_ref()
            .filter(|t| t.owner == unsafe { libc::getpid() })
    }
    pub fn launch(&self, background: bool, single: bool) -> Launch {
        let tty = self.terminal().filter(|_| self.monitor);
        let foreground_fd = tty.filter(|_| !background).map(|t| t.fd.as_raw_fd());
        Launch {
            group: self.monitor.then_some(0),
            tty: foreground_fd,
            defer_tty: single && foreground_fd.is_some() && crate::editor::preview::can_wait(),
            shell_modes: foreground_fd.and_then(terminal_modes),
            background,
            single,
        }
    }
    // A command substitution or foreground pipeline builtin may list its
    // parent's jobs, but cannot reap children owned by the parent process.
    pub fn refresh_snapshot(&mut self) {
        if !self.table.is_empty() {
            self.poll();
        }
    }

    pub fn forked(&mut self, list_parent: bool) {
        if list_parent {
            self.inherited.append(&mut self.table);
        } else {
            self.inherited.clear();
        }
        if self.tty.is_some() {
            crate::traps::reset_caught_signals(&[libc::SIGTSTP]);
        }
        self.tty = None;
        self.running_prompt_hook = false;
        self.posix_interactive = false;
        self.foreground_interrupted = false;
        self.monitor = false;
        self.table.clear();
        self.detached.clear();
        self.finished.clear();
    }
    pub(crate) fn restore_initial_group(&self) {
        if let Some(tty) = self.terminal()
            && tty.shell_group != tty.original_group
            && unsafe {
                libc::getpgrp() == tty.shell_group
                    && libc::tcgetpgrp(tty.fd.as_raw_fd()) == tty.shell_group
            }
            && unsafe { libc::setpgid(0, tty.original_group) } == 0
        {
            give_terminal(tty.fd.as_raw_fd(), tty.original_group);
        }
    }

    pub(crate) fn detach_started(&mut self, pids: impl Iterator<Item = pid_t>) {
        // A later segment's setup can fail after earlier processes have begun.
        // Keep reaping those children without waiting for a stopped process.
        self.detached.extend(pids);
    }
    pub fn restore(&self) {
        if let Some(t) = self.terminal() {
            give_terminal(t.fd.as_raw_fd(), t.shell_group);
        }
    }

    pub fn finish(
        &mut self,
        launch: Launch,
        stages: Vec<Result<pid_t, u8>>,
        name: String,
        pipefail: bool,
        statuses: &mut crate::session::PipelineStatus,
    ) -> io::Result<(u8, Option<pid_t>)> {
        let children = stages
            .into_iter()
            .map(|s| match s {
                Ok(pid) => Process {
                    signal: None,
                    pid,
                    state: State::Running,
                },
                Err(code) => Process {
                    signal: None,
                    pid: 0,
                    state: State::Done(code),
                },
            })
            .collect();
        let mut job = Job {
            id: 0,
            foreground_notice: false,
            stopped_at: 0,
            group: launch.group.filter(|g| *g != 0),
            children,
            name,
            pipefail,
            modes: None,
            signaled: false,
            listed: false,
            nohup: false,
        };
        let last = job.last_pid();
        if launch.background {
            let id = self.store(job);
            if !self.quiet && self.terminal().is_some() {
                stderr_line!("[{id}] {}", last.unwrap_or(0));
            }
            Ok((0, last))
        } else {
            if launch.defer_tty
                && job.children.len() == 1
                && job.active()
                && let Some(pid) = job.group
                && let Ok(status) = crate::editor::preview::wait_child(pid)
            {
                let event = ChildEvent::decode(status);
                if matches!(event, ChildEvent::Stopped(libc::SIGTTIN | libc::SIGTTOU)) {
                    if let Some(t) = self.terminal() {
                        give_terminal(t.fd.as_raw_fd(), pid);
                    }
                    unsafe {
                        libc::kill(-pid, libc::SIGCONT);
                    }
                } else {
                    job.update(pid, event);
                }
            }
            let code = if job.active() || job.stopped() || !launch.defer_tty {
                self.foreground(&mut job, false, launch.shell_modes)?
            } else {
                job.status()
            };
            statuses.clear();
            for child in &job.children {
                if let State::Stopped(code) | State::Done(code) = child.state {
                    statuses.push(code);
                }
            }
            self.note_foreground_signal(&mut job);
            if job.stopped() {
                self.store(job);
                self.report_stopped(self.table.len() - 1);
            } else if job.foreground_notice {
                self.store(job);
            }
            Ok((code, None))
        }
    }
    fn report_stopped(&mut self, index: usize) {
        if !self.quiet {
            let markers = marked_jobs(self.table.iter());
            let job = &mut self.table[index];
            let mut out = io::stderr().lock();
            if self.tty.is_some() {
                let _ = writeln!(out);
            }
            let _ = job_line(&mut out, job, job_marker(job.id, markers), "Stopped");
            job.listed = true;
        }
    }
    fn note_foreground_signal(&mut self, job: &mut Job) {
        if !job.signaled || job.stopped() {
            return;
        }
        // A foreground process group owns the terminal during the wait. The
        // shell does not receive the keystroke itself. Use waitpid's signal
        // information so an explicit exit status of 130 remains an ordinary exit.
        if self.tty.is_some()
            && job.children.iter().find_map(|child| child.signal) == Some(libc::SIGINT)
        {
            self.foreground_interrupted = true;
        }
        if let Some(sig) = job.children.last().and_then(|c| c.signal) {
            if self.posix_interactive
                && self.monitor
                && !self.notify
                && !self.running_prompt_hook
                && !matches!(sig, libc::SIGINT | libc::SIGPIPE)
            {
                job.foreground_notice = true;
            } else {
                self.report_signal(sig, &job.name);
            }
        }
    }

    pub(crate) fn report_signal(&self, signal: i32, command: &str) {
        if !self.quiet
            && !matches!(signal, libc::SIGINT | libc::SIGPIPE)
            && let Some(reason) = signal_description(signal)
        {
            stderr_line!("{reason:<26} {command}");
        }
    }

    fn remove_reported(&mut self) {
        for i in (0..self.table.len()).rev() {
            let job = &self.table[i];
            if job.listed && !job.active() && !job.stopped() {
                let done = self.table.remove(i);
                if !done.foreground_notice {
                    self.remember(&done);
                }
            }
        }
    }

    fn remember(&mut self, job: &Job) {
        if let Some(pid) = job.last_pid() {
            if self.finished.len() == MAX_FINISHED_STATUSES {
                self.finished.pop_front();
            }
            self.finished.push_back((pid, job.status()));
        }
    }

    fn store(&mut self, mut job: Job) -> u32 {
        // New IDs follow both tables. Waiting and disowning preserve their order.
        let id = self.all_jobs().next_back().map_or(1, |last| last.id + 1);
        job.id = id;
        self.table.push(job);
        crate::traps::job_changed();
        id
    }
    fn foreground(
        &self,
        job: &mut Job,
        resume: bool,
        original: Option<libc::termios>,
    ) -> io::Result<u8> {
        let tty = self.terminal();
        if let (Some(t), Some(group)) = (tty, job.group) {
            // Changing terminal modes after handoff can stop the shell with
            // SIGTTOU. Keep the signal blocked through both operations.
            give_terminal_with_modes(t.fd.as_raw_fd(), group, job.modes.as_ref());
        }
        let result = (|| {
            if resume {
                job.resume()?;
            }
            job.wait(None, false, tty.is_some())
        })();
        if let Some(t) = tty {
            self.restore();
            if job.stopped() {
                job.modes = terminal_modes(t.fd.as_raw_fd());
            }
            // A successful stty command intentionally changes the next prompt.
            // Suspended or interrupted full-screen programs often leave raw mode on.
            if (job.stopped() || job.signaled || result.is_err())
                && let Some(modes) = original
            {
                unsafe {
                    libc::tcsetattr(t.fd.as_raw_fd(), libc::TCSANOW, &modes);
                }
            }
        }
        result.map(|interrupt| interrupt.unwrap_or_else(|| job.status()))
    }
    pub fn completion_names(&mut self, action: &str, prefix: &str) -> Vec<String> {
        self.poll();
        self.inherited
            .iter()
            .chain(&self.table)
            .filter(|job| match action {
                "running" => job.active() && !job.stopped(),
                "stopped" => job.stopped(),
                _ => true,
            })
            .filter_map(|job| job.name.split_whitespace().next())
            .filter(|name| name.starts_with(prefix))
            .map(str::to_owned)
            .collect()
    }

    pub fn hangup(&self) {
        for job in &self.table {
            job.hangup();
        }
    }

    fn disown(&mut self, args: &[String]) -> u8 {
        let (mut all, mut running, mut nohup) = (false, false, false);
        let mut specs = args;
        while let Some(option) = crate::builtins::flags::next(&mut specs) {
            for flag in option[1..].chars() {
                match flag {
                    'a' => all = true,
                    'r' => running = true,
                    'h' => nohup = true,
                    _ => {
                        diagnostic!("disown: invalid option {option}");
                        return 2;
                    }
                }
            }
        }
        if specs.is_empty() && (all || running) {
            for i in (0..self.table.len()).rev() {
                if !running || self.table[i].active() {
                    self.detach(i, nohup, false);
                }
            }
            return 0;
        }
        let mut status = 0;
        for spec in specs
            .iter()
            .map(String::as_str)
            .chain(specs.is_empty().then_some("%+"))
        {
            if let Some(i) = self.select(spec, crate::builtins::integer_in_range(spec)) {
                self.detach(i, nohup, true);
            } else {
                diagnostic!("disown: {spec}: no such job");
                status = 1;
            }
        }
        status
    }

    fn detach(&mut self, index: usize, nohup: bool, keep_status: bool) {
        if nohup {
            self.table[index].nohup = true;
            return;
        }
        let job = self.table.remove(index);
        // A targeted disown retains the last known pipeline status for wait PID.
        if keep_status {
            self.remember(&job);
        }
        self.detached.extend(
            job.children
                .into_iter()
                .filter(|p| p.pid > 0 && !matches!(p.state, State::Done(_)))
                .map(|p| p.pid),
        );
    }

    pub(crate) fn has_live_pid(&self, pid: pid_t) -> bool {
        self.detached.contains(&pid)
            || self.table.iter().any(|job| {
                job.children
                    .iter()
                    .any(|child| child.pid == pid && !matches!(child.state, State::Done(_)))
            })
    }

    pub(crate) fn awaiting_notification(&self) -> bool {
        self.monitor
            && self
                .table
                .iter()
                .any(|job| job.active() || !job.listed || job.stopped())
    }

    pub(crate) fn has_notification(&self) -> bool {
        self.table.iter().any(|job| !job.active() && !job.listed)
    }

    pub(crate) fn report_changes(&mut self, posix: bool) {
        let markers = marked_jobs(self.table.iter());
        for job in &mut self.table {
            if !job.active() && !job.listed {
                let state = job.display_state(posix);
                if job.foreground_notice {
                    stderr_line!("{state:<26} {}", job.name);
                } else {
                    let _ = job_line(
                        &mut io::stderr().lock(),
                        job,
                        job_marker(job.id, markers),
                        &state,
                    );
                }
                job.listed = true;
            }
        }
        self.remove_reported();
    }

    pub fn poll(&mut self) {
        self.detached.retain(|pid| {
            let result = crate::traps::waitpid(*pid, &mut 0, libc::WNOHANG);
            result == 0
                || result < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
        });
        for job in &mut self.table {
            for i in 0..job.children.len() {
                let p = &job.children[i];
                if matches!(p.state, State::Done(_)) {
                    continue;
                }
                let mut status = 0;
                let got = crate::traps::waitpid(
                    p.pid,
                    &mut status,
                    libc::WNOHANG | libc::WUNTRACED | libc::WCONTINUED,
                );
                if got > 0 {
                    job.update(got, ChildEvent::decode(status));
                } else if got < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD)
                {
                    // No further status can arrive for an already reaped child.
                    job.update(p.pid, ChildEvent::Exited(127));
                }
            }
        }
    }
    fn select_job(&self, spec: &str) -> Option<usize> {
        if let Ok(id) = spec.parse::<u32>() {
            self.table.iter().position(|j| j.id == id)
        } else {
            self.select(spec, None)
        }
    }
    fn select(&self, spec: &str, pid: Option<pid_t>) -> Option<usize> {
        match spec {
            "%" | "%+" | "%%" | "%-" => {
                let (current, previous) = marked_jobs(self.table.iter());
                let id = if spec == "%-" { previous } else { current }?;
                self.table.iter().position(|job| job.id == id)
            }
            s if s.starts_with('%') => {
                let name = &s[1..];
                if let Ok(id) = name.parse::<u32>() {
                    return self.table.iter().position(|j| j.id == id);
                }
                let mut hits = self
                    .table
                    .iter()
                    .enumerate()
                    .filter(|(_, job)| job_matches(&job.name, name));
                let (i, _) = hits.next()?;
                hits.next().is_none().then_some(i)
            }
            _ => {
                let pid = pid.filter(|&pid| pid > 0)?;
                self.table
                    .iter()
                    .position(|j| j.children.iter().any(|p| p.pid == pid))
            }
        }
    }
}
