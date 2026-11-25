use crate::Session;
use crate::execute::fd::PrivateFile;
use libc::pid_t;
use std::io::{self, Write};
use std::os::fd::{AsFd, AsRawFd, RawFd};

#[derive(Clone)]
struct Terminal {
    fd: PrivateFile,
    // The interactive shell leads its own foreground process group.
    owner: pid_t,
}

#[derive(Clone, Copy, Default)]
pub struct Launch {
    pub group: Option<pid_t>,
    tty: Option<RawFd>,
    pub background: bool,
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
            let mut info = std::mem::MaybeUninit::uninit();
            loop {
                let result = unsafe {
                    libc::waitid(
                        libc::P_PID,
                        pid as _,
                        info.as_mut_ptr(),
                        libc::WSTOPPED | libc::WNOHANG,
                    )
                };
                if result == 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                    break;
                }
            }
            give_terminal(fd, self.group.filter(|&g| g != 0).unwrap_or(pid));
            unsafe { libc::kill(pid, libc::SIGCONT) };
        }
    }

    pub fn pipeline_child(self) {
        if self.group.is_none() && !self.background {
            crate::execute::reset_child_signals();
        }
        self.child();
    }

    pub fn child(self) {
        if let Some(group) = self.group {
            if unsafe { libc::setpgid(0, group) } < 0 {
                unsafe { libc::_exit(1) }
            }
            if let Some(tty) = self.tty.filter(|_| !self.defer_tty) {
                give_terminal(tty, unsafe { libc::getpgrp() });
            }
            crate::execute::reset_caught_signals(&JOB_SIGNALS);
        } else if self.background {
            for sig in [libc::SIGINT, libc::SIGQUIT] {
                unsafe {
                    libc::signal(sig, libc::SIG_IGN);
                }
            }
        }
    }

    pub fn parent(&mut self, pid: pid_t) {
        if let Some(group) = self.group {
            let group = if group == 0 { pid } else { group };
            unsafe {
                libc::setpgid(pid, group);
            }
            self.group = Some(group);
        }
    }
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

fn give_terminal_with_modes(fd: RawFd, pgid: pid_t, modes: Option<&libc::termios>) {
    // The shell is temporarily in the background when it takes the terminal back.
    unsafe {
        let mut block = std::mem::zeroed();
        let mut old = std::mem::zeroed();
        libc::sigemptyset(&mut block);
        libc::sigaddset(&mut block, libc::SIGTTOU);
        libc::sigprocmask(libc::SIG_BLOCK, &block, &mut old);
        libc::tcsetpgrp(fd, pgid);
        if let Some(modes) = modes {
            libc::tcsetattr(fd, libc::TCSANOW, modes);
        }
        libc::sigprocmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
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
    state: State,
}
#[derive(Clone)]
struct Job {
    id: u32,
    group: Option<pid_t>,
    children: Vec<Process>,
    name: String,
    pipefail: bool,
    modes: Option<libc::termios>,
    signaled: bool,
    listed: bool,
    nohup: bool,
}

impl Job {
    fn hangup(&self) {
        if !self.nohup
            && (self.active() || self.stopped())
            && let Some(group) = self.group
        {
            unsafe {
                libc::kill(-group, libc::SIGHUP);
                if self.stopped() {
                    libc::kill(-group, libc::SIGCONT);
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
    fn update(&mut self, pid: pid_t, status: i32) {
        if let Some(p) = self.children.iter_mut().find(|p| p.pid == pid) {
            self.listed = false;
            p.state = if libc::WIFSTOPPED(status) {
                State::Stopped((128 + libc::WSTOPSIG(status)) as u8)
            } else if libc::WIFCONTINUED(status) {
                State::Running
            } else if libc::WIFSIGNALED(status) {
                self.signaled = true;
                State::Done((128 + libc::WTERMSIG(status)) as u8)
            } else {
                State::Done(libc::WEXITSTATUS(status) as u8)
            };
        }
    }
    fn wait(
        &mut self,
        mask: Option<&crate::traps::WaitMask>,
        force: bool,
        foreground_tty: bool,
    ) -> io::Result<Option<u8>> {
        while self.active() || (force && self.stopped()) {
            if foreground_tty && crate::traps::hanging_up() {
                self.hangup();
                return Ok(Some(128 + libc::SIGHUP as u8));
            }
            if let Some(code) = mask.and_then(crate::traps::WaitMask::interrupt_status) {
                return Ok(Some(code));
            }
            let pid = self.group.map(|g| -g).unwrap_or_else(|| {
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
            let got = unsafe { libc::waitpid(pid, &mut status, flags) };
            if got == 0 {
                mask.unwrap().suspend();
                continue;
            }
            if got < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            self.update(got, status);
        }
        Ok(None)
    }
    fn resume(&mut self) -> io::Result<()> {
        if let Some(group) = self.group {
            if unsafe { libc::kill(-group, libc::SIGCONT) } < 0 {
                return Err(io::Error::last_os_error());
            }
        }
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
    tty: Option<Terminal>,
    table: Vec<Job>,
    detached: Vec<pid_t>,
    inherited: Vec<Job>,
    serial: u32,
    // Bash keeps completed background statuses available to wait by PID.
    finished: std::collections::VecDeque<(pid_t, u8)>,
}

impl Jobs {
    pub(crate) fn count(&self) -> usize {
        self.table.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.table.is_empty() && self.inherited.is_empty()
    }

    pub fn init(&mut self) {
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
        if unsafe { libc::tcgetpgrp(fd.as_raw_fd()) } != old {
            return;
        }
        let owner = unsafe { libc::getpid() };
        if old != owner {
            if unsafe { libc::setpgid(0, 0) } < 0 {
                return;
            }
            give_terminal(fd.as_raw_fd(), owner);
        }
        // A caught disposition becomes default on exec. Preserve a signal
        // explicitly ignored by our parent across child process creation.
        extern "C" fn stay_running(_: i32) {}
        unsafe {
            let mut previous = std::mem::MaybeUninit::uninit();
            if libc::sigaction(libc::SIGTSTP, std::ptr::null(), previous.as_mut_ptr()) == 0
                && previous.assume_init().sa_sigaction != libc::SIG_IGN
            {
                libc::signal(libc::SIGTSTP, stay_running as *const () as usize);
            }
        }
        self.tty = Some(Terminal { fd, owner });
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
        }
    }
    // A command substitution or foreground pipeline builtin may list its
    // parent's jobs, but cannot reap children owned by the parent process.
    pub fn forked(&mut self, list_parent: bool) {
        if list_parent {
            self.inherited.append(&mut self.table);
        } else {
            self.inherited.clear();
        }
        self.tty = None;
        self.monitor = false;
        self.table.clear();
        self.detached.clear();
        self.serial = 0;
        self.finished.clear();
    }
    pub fn restore(&self) {
        if let Some(t) = self.terminal() {
            give_terminal(t.fd.as_raw_fd(), t.owner);
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
                    pid,
                    state: State::Running,
                },
                Err(code) => Process {
                    pid: 0,
                    state: State::Done(code),
                },
            })
            .collect();
        let mut job = Job {
            id: 0,
            group: launch.group.filter(|g| *g != 0),
            children,
            name,
            pipefail,
            modes: None,
            signaled: false,
            listed: false,
            nohup: false,
        };
        let last = job
            .children
            .iter()
            .rev()
            .find(|p| p.pid != 0)
            .map(|p| p.pid);
        if launch.background {
            self.store(job);
            if self.terminal().is_some() {
                eprintln!("[{}] {}", self.serial, last.unwrap_or(0));
            }
            Ok((0, last))
        } else {
            if launch.defer_tty
                && job.children.len() == 1
                && job.active()
                && let Some(pid) = job.group
                && let Ok(status) = crate::editor::preview::wait_child(pid)
            {
                if libc::WIFSTOPPED(status)
                    && matches!(libc::WSTOPSIG(status), libc::SIGTTIN | libc::SIGTTOU)
                {
                    if let Some(t) = self.terminal() {
                        give_terminal(t.fd.as_raw_fd(), pid);
                    }
                    unsafe {
                        libc::kill(-pid, libc::SIGCONT);
                    }
                } else {
                    job.update(pid, status);
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
            if job.stopped() {
                self.store(job);
                eprintln!("[{}]+ Stopped", self.serial);
            }
            Ok((code, None))
        }
    }
    fn remember(&mut self, job: &Job) {
        if let Some(pid) = job.children.last().map(|p| p.pid).filter(|&p| p > 0) {
            if self.finished.len() == 4096 {
                self.finished.pop_front();
            }
            self.finished.push_back((pid, job.status()));
        }
    }

    fn store(&mut self, mut job: Job) {
        self.serial = self
            .inherited
            .iter()
            .chain(&self.table)
            .map(|j| j.id)
            .max()
            .unwrap_or(0)
            + 1;
        job.id = self.serial;
        self.table.push(job);
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
            if job.stopped() || job.signaled || result.is_err() {
                if let Some(modes) = original {
                    unsafe {
                        libc::tcsetattr(t.fd.as_raw_fd(), libc::TCSANOW, &modes);
                    }
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
        while let Some((option, rest)) = specs.split_first() {
            if option == "--" {
                specs = rest;
                break;
            }
            if !option.starts_with('-') || option == "-" {
                break;
            }
            for flag in option[1..].chars() {
                match flag {
                    'a' => all = true,
                    'r' => running = true,
                    'h' => nohup = true,
                    _ => {
                        eprintln!("tonari: disown: invalid option {option}");
                        return 2;
                    }
                }
            }
            specs = rest;
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
            if let Some(i) = self.select(spec) {
                self.detach(i, nohup, true);
            } else {
                eprintln!("tonari: disown: {spec}: no such job");
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

    pub fn poll(&mut self) {
        self.detached.retain(|pid| {
            let result = unsafe { libc::waitpid(*pid, std::ptr::null_mut(), libc::WNOHANG) };
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
                let got = unsafe {
                    libc::waitpid(
                        p.pid,
                        &mut status,
                        libc::WNOHANG | libc::WUNTRACED | libc::WCONTINUED,
                    )
                };
                if got > 0 {
                    job.update(got, status);
                }
            }
        }
    }
    fn select(&self, spec: &str) -> Option<usize> {
        match spec {
            "" | "%" | "%+" | "%%" => self.table.len().checked_sub(1),
            "%-" => self.table.len().checked_sub(2),
            s if s.starts_with('%') => {
                let name = &s[1..];
                if let Ok(id) = name.parse::<u32>() {
                    return self.table.iter().position(|j| j.id == id);
                }
                let mut hits =
                    self.table
                        .iter()
                        .enumerate()
                        .filter(|(_, job)| match name.strip_prefix('?') {
                            Some(part) => job.name.contains(part),
                            None => job.name.starts_with(name),
                        });
                let (i, _) = hits.next()?;
                hits.next().is_none().then_some(i)
            }
            s => {
                let pid = s.parse::<pid_t>().ok()?;
                self.table
                    .iter()
                    .position(|j| j.children.iter().any(|p| p.pid == pid))
            }
        }
    }
}

fn kill(jobs: &Jobs, mut args: &[String]) -> u8 {
    if args.first().is_some_and(|s| s == "-l" || s == "-L") {
        let mut out = io::stdout().lock();
        if args.len() == 1 {
            for &(name, n) in crate::traps::SIGNALS.iter().filter(|s| s.1 != 0) {
                if writeln!(out, "{n:2}) SIG{name}").is_err() {
                    return 1;
                }
            }
        } else {
            for arg in &args[1..] {
                let result = if let Ok(n) = arg.parse::<i32>() {
                    let Some((name, _)) = crate::traps::SIGNALS.iter().find(|s| s.1 == n % 128)
                    else {
                        return 1;
                    };
                    writeln!(out, "{name}")
                } else if let Some(n) = crate::traps::number(arg) {
                    writeln!(out, "{n}")
                } else {
                    return 1;
                };
                if result.is_err() {
                    return 1;
                }
            }
        }
        return 0;
    }
    let sig = match args.first().map(String::as_str) {
        Some("-s" | "-n") if args.len() >= 2 => {
            let sig = crate::traps::number(&args[1]);
            args = &args[2..];
            sig
        }
        Some(s) if s.len() > 2 && (s.starts_with("-n") || s.starts_with("-s")) => {
            let signal = crate::traps::number(&s[2..]);
            args = &args[1..];
            signal
        }
        Some(s) if s.starts_with('-') && s != "--" => {
            args = &args[1..];
            crate::traps::number(&s[1..])
        }
        _ => Some(libc::SIGTERM),
    };
    let Some(sig) = sig else {
        eprintln!("tonari: kill: invalid signal");
        return 1;
    };
    if args.first().is_some_and(|s| s == "--") {
        args = &args[1..];
    }
    if args.is_empty() {
        eprintln!("tonari: kill: expected PID or job");
        return 2;
    }
    let mut status = 0;
    for spec in args {
        let send = |pid| unsafe { libc::kill(pid, sig) == 0 };
        let ok = if spec.starts_with('%') {
            jobs.select(spec).is_some_and(|i| {
                let job = &jobs.table[i];
                if let Some(group) = job.group {
                    send(-group)
                } else {
                    let mut delivered = false;
                    for p in &job.children {
                        if p.pid > 0 && !matches!(p.state, State::Done(_)) {
                            delivered |= send(p.pid);
                        }
                    }
                    delivered
                }
            })
        } else {
            spec.parse::<pid_t>().is_ok_and(send)
        };
        if !ok {
            eprintln!("tonari: kill: {spec}: could not signal target");
            status = 1;
        }
    }
    status
}

#[derive(Default)]
struct WaitOptions<'a> {
    next: bool,
    force: bool,
    variable: Option<&'a str>,
}

impl<'a> WaitOptions<'a> {
    fn parse(mut args: &'a [String]) -> Option<(Self, &'a [String])> {
        let mut options = Self::default();
        while let Some((arg, rest)) = args.split_first() {
            if arg == "--" {
                return Some((options, rest));
            }
            if !arg.starts_with('-') || arg == "-" {
                break;
            }
            args = rest;
            let mut flags = arg[1..].chars();
            while let Some(flag) = flags.next() {
                match flag {
                    'n' => options.next = true,
                    'f' => options.force = true,
                    'p' => {
                        options.variable = Some(if flags.as_str().is_empty() {
                            let (name, rest) = args.split_first()?;
                            args = rest;
                            name
                        } else {
                            flags.as_str()
                        });
                        break;
                    }
                    _ => return None,
                }
            }
        }
        Some((options, args))
    }
}

fn wait_builtin(args: &[String], session: &mut Session) -> u8 {
    let Some((options, ids)) = WaitOptions::parse(args) else {
        eprintln!("tonari: wait: usage: wait [-fn] [-p var] [id ...]");
        return 2;
    };
    if let Some(name) = options.variable {
        if !session.valid_target(name) {
            eprintln!("tonari: wait: {name}: not a valid identifier");
            return 1;
        }
        if session.unset(&["-v".into(), name.into()]) != 0 {
            return 1;
        }
    }
    let (code, pid) = if options.next {
        wait_next(session, ids, options.force)
    } else {
        wait_for_ids(session, ids, options.force)
    };
    if let (Some(name), Some(pid)) = (options.variable, pid) {
        if session.assign_target(name, &pid.to_string()).is_err() {
            return 1;
        }
    }
    code
}

fn wait_next(session: &mut Session, ids: &[String], force: bool) -> (u8, Option<pid_t>) {
    let jobs = &mut session.jobs;
    let mut selected = Vec::with_capacity(ids.len());
    for spec in ids {
        if let Some(index) = jobs.select(spec) {
            selected.push(jobs.table[index].id);
        } else {
            eprintln!("tonari: wait: {spec}: no such job");
        }
    }
    // Blocking SIGCHLD before polling closes the check/sleep race. Reap
    // only our job PIDs so an unrelated process substitution retains its status.
    let Ok(mask) = crate::traps::WaitMask::watch_children(&session.traps) else {
        return (1, None);
    };
    loop {
        if let Some(code) = mask.interrupt_status() {
            return (code, None);
        }
        jobs.poll();
        let mut running = false;
        for (i, job) in jobs.table.iter().enumerate() {
            if !ids.is_empty() && !selected.contains(&job.id) {
                continue;
            }
            if !job.active() && !job.stopped() {
                let job = jobs.table.remove(i);
                let pid = job.children.last().map(|p| p.pid);
                jobs.remember(&job);
                return (job.status(), pid);
            }
            running |= job.active() || (force && job.stopped());
        }
        if !running {
            return (127, None);
        }
        mask.suspend();
    }
}

fn wait_for_ids(session: &mut Session, ids: &[String], force: bool) -> (u8, Option<pid_t>) {
    let Ok(mask) = crate::traps::WaitMask::new(&session.traps) else {
        return (1, None);
    };
    let jobs = &mut session.jobs;
    if ids.is_empty() {
        for job in &mut jobs.table {
            if let Ok(Some(code)) = job.wait(mask.as_ref(), force, false) {
                return (code, None);
            }
        }
        jobs.table.retain(Job::stopped);
        jobs.finished.clear();
        if let Some(pid) = session.last_background {
            let _ = session.substitutions.wait(pid, mask.as_ref());
            if let Some(code) = mask
                .as_ref()
                .and_then(crate::traps::WaitMask::interrupt_status)
            {
                return (code, None);
            }
        }
        return (0, None);
    }
    let mut result = (0, None);
    for spec in ids {
        let number = spec.parse::<pid_t>().ok().filter(|&pid| pid > 0);
        if let Some(i) = jobs.select(spec) {
            let job = &mut jobs.table[i];
            result = match job.wait(mask.as_ref(), force, false) {
                Ok(Some(code)) => return (code, None),
                Ok(None) => (
                    job.status(),
                    number.or_else(|| job.children.last().map(|p| p.pid)),
                ),
                Err(_) => (127, None),
            };
            if !job.stopped() {
                let job = jobs.table.remove(i);
                jobs.remember(&job);
            }
        } else if let Some(pid) = number {
            if let Some((_, code)) = jobs.finished.iter().rev().find(|(p, _)| *p == pid) {
                result = (*code, Some(pid));
            } else if let Some(code) = session.substitutions.wait(pid, mask.as_ref()) {
                if let Some(interrupted) = mask
                    .as_ref()
                    .and_then(crate::traps::WaitMask::interrupt_status)
                {
                    return (interrupted, None);
                }
                result = (code, Some(pid));
            } else {
                eprintln!("tonari: wait: {spec}: not a child of this shell");
                result = (127, None);
            }
        } else {
            eprintln!("tonari: wait: {spec}: not a valid child or job");
            result = (
                if spec.starts_with('%') || spec.bytes().all(|b| b.is_ascii_digit()) {
                    127
                } else {
                    1
                },
                None,
            );
        }
    }
    result
}

fn job_id(jobs: &Jobs, spec: &str) -> Option<u32> {
    let mut all = jobs.inherited.iter().chain(&jobs.table).rev();
    let spec = spec.strip_prefix('%').unwrap_or(spec);
    match spec {
        "" | "+" | "%" => all.next().map(|j| j.id),
        "-" => all.nth(1).map(|j| j.id),
        s => {
            if let Ok(id) = s.parse::<u32>() {
                return all.find(|job| job.id == id).map(|job| job.id);
            }
            let mut matches = all.filter(|j| {
                s.strip_prefix('?')
                    .map_or_else(|| j.name.starts_with(s), |part| j.name.contains(part))
            });
            let id = matches.next()?.id;
            matches.next().is_none().then_some(id)
        }
    }
}

fn list(jobs: &mut Jobs, args: &[String]) -> u8 {
    let mut mode = ' ';
    let mut changed = false;
    let mut filter = ' ';
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next_if(|s| s.starts_with('-') && s.len() > 1) {
        if arg == "--" {
            break;
        }
        for flag in arg[1..].chars() {
            match flag {
                'p' | 'l' => mode = flag,
                'n' => changed = true,
                'r' | 's' => filter = flag,
                _ => {
                    eprintln!("tonari: jobs: invalid option: {flag}");
                    return 2;
                }
            }
        }
    }
    let mut status = 0;
    let mut ids = Vec::new();
    let all = args.peek().is_none();
    for spec in args {
        if let Some(id) = job_id(jobs, spec) {
            ids.push(id);
        } else {
            eprintln!("tonari: jobs: {spec}: no such job");
            status = 1;
        }
    }
    if all {
        ids.extend(jobs.inherited.iter().chain(&jobs.table).map(|j| j.id));
    }
    let current = job_id(jobs, "%+");
    let previous = job_id(jobs, "%-");
    let mut out = io::stdout().lock();
    for id in ids {
        let Some(job) = jobs
            .inherited
            .iter_mut()
            .chain(&mut jobs.table)
            .find(|j| j.id == id)
        else {
            continue;
        };
        if (changed && job.listed)
            || (filter == 'r' && !job.active())
            || (filter == 's' && !job.stopped())
        {
            continue;
        }
        let pid = job
            .group
            .unwrap_or_else(|| job.children.first().map_or(0, |p| p.pid));
        if mode == 'p' {
            if writeln!(out, "{pid}").is_err() {
                return 1;
            }
            continue;
        }
        let state = if job.active() {
            "Running".into()
        } else if job.stopped() {
            "Stopped".into()
        } else if job.status() == 0 {
            "Done".into()
        } else {
            format!("Exit {}", job.status())
        };
        let marker = if Some(id) == current {
            '+'
        } else if Some(id) == previous {
            '-'
        } else {
            ' '
        };
        let bg = if job.active() { " &" } else { "" };
        if mode == 'l' {
            let mut names = job.name.split(" | ");
            for (stage, child) in job.children.iter().enumerate() {
                let name = names.next().unwrap_or("command");
                let suffix = if stage + 1 == job.children.len() {
                    bg
                } else {
                    ""
                };
                let line = if stage == 0 {
                    writeln!(
                        out,
                        "[{id}]{marker} {} {state:<23} {name}{suffix}",
                        child.pid
                    )
                } else {
                    writeln!(
                        out,
                        "     {}                       | {name}{suffix}",
                        child.pid
                    )
                };
                if line.is_err() {
                    return 1;
                }
            }
        } else if writeln!(out, "[{id}]{marker}  {state:<23} {}{bg}", job.name).is_err() {
            return 1;
        }
        job.listed = true;
    }
    for i in (0..jobs.table.len()).rev() {
        let job = &jobs.table[i];
        if job.listed && !job.active() && !job.stopped() {
            let done = jobs.table.remove(i);
            jobs.remember(&done);
        }
    }
    if out.flush().is_err() { 1 } else { status }
}

pub fn dispatch(words: &[String], session: &mut Session, last: u8) -> crate::Flow {
    if words[0] == "jobs" && words.get(1).is_some_and(|s| s == "-x") {
        let mut words = words[2..].iter();
        if words.clone().next().is_some_and(|s| s == "--") {
            words.next();
        }
        let command = crate::parser::SimpleCommand {
            words: words
                .map(|arg| {
                    let id = arg
                        .starts_with('%')
                        .then(|| job_id(&session.jobs, arg))
                        .flatten();
                    let job = session
                        .jobs
                        .inherited
                        .iter()
                        .chain(&session.jobs.table)
                        .find(|j| Some(j.id) == id);
                    let text = job.map_or_else(
                        || arg.clone(),
                        |j| {
                            j.group
                                .unwrap_or_else(|| unsafe { libc::getpgrp() })
                                .to_string()
                        },
                    );
                    crate::word::Word::Literal(text)
                })
                .collect(),
            ..Default::default()
        };
        return crate::execute::run(vec![command], last, false, session)
            .unwrap_or(crate::Flow::Continue(1));
    }
    crate::Flow::Continue(builtin(words, session))
}

pub fn builtin(words: &[String], session: &mut Session) -> u8 {
    if words[0] == "wait" {
        return wait_builtin(&words[1..], session);
    }
    let jobs = &mut session.jobs;
    jobs.poll();
    match words[0].as_str() {
        "kill" => kill(jobs, &words[1..]),
        "jobs" => list(jobs, &words[1..]),
        "disown" => jobs.disown(&words[1..]),
        name => {
            if !jobs.monitor {
                eprintln!("tonari: {name}: no job control");
                return 1;
            }
            let Some(i) = jobs.select(words.get(1).map(String::as_str).unwrap_or("")) else {
                eprintln!("tonari: {name}: no such job");
                return 1;
            };
            if jobs.table[i].group.is_none() {
                eprintln!(
                    "tonari: {name}: job {} started without job control",
                    jobs.table[i].id
                );
                return 1;
            }
            if name == "bg" {
                if !jobs.table[i].stopped() {
                    eprintln!("tonari: bg: job {} already in background", jobs.table[i].id);
                    return 0;
                }
                let marker = if i + 1 == jobs.table.len() {
                    '+'
                } else if i + 2 == jobs.table.len() {
                    '-'
                } else {
                    ' '
                };
                let job = &mut jobs.table[i];
                if job.resume().is_err() {
                    return 1;
                }
                let mut out = io::stdout().lock();
                return u8::from(
                    writeln!(out, "[{}]{marker} {} &", job.id, job.name)
                        .and_then(|_| out.flush())
                        .is_err(),
                );
            }
            let mut job = jobs.table.remove(i);
            {
                let mut out = io::stdout().lock();
                let _ = writeln!(out, "{}", job.name).and_then(|_| out.flush());
            }
            let modes = jobs
                .terminal()
                .and_then(|t| terminal_modes(t.fd.as_raw_fd()));
            let r = jobs.foreground(&mut job, true, modes);
            if job.stopped() {
                jobs.table.push(job);
            }
            r.unwrap_or(1)
        }
    }
}
