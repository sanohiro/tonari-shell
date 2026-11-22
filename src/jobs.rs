use crate::Session;
use crate::execute::fd::PrivateFile;
use libc::pid_t;
use std::io::{self, Write};
use std::os::fd::{AsFd, AsRawFd, RawFd};

#[derive(Clone)]
struct Terminal {
    fd: PrivateFile,
    owner: pid_t,
    group: pid_t,
}

#[derive(Clone, Copy, Default)]
pub struct Launch {
    pub group: Option<pid_t>,
    tty: Option<RawFd>,
    pub background: bool,
    shell_modes: Option<libc::termios>,
}

impl Launch {
    pub fn child(self) {
        if let Some(group) = self.group {
            if unsafe { libc::setpgid(0, group) } < 0 {
                unsafe { libc::_exit(1) }
            }
            if let Some(tty) = self.tty {
                give_terminal(tty, unsafe { libc::getpgrp() });
            }
            for sig in [
                libc::SIGTSTP,
                libc::SIGTTIN,
                libc::SIGTTOU,
                libc::SIGINT,
                libc::SIGQUIT,
            ] {
                unsafe {
                    libc::signal(sig, libc::SIG_DFL);
                }
            }
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
    // The shell is temporarily in the background when it takes the terminal back.
    unsafe {
        let mut block = std::mem::zeroed();
        let mut old = std::mem::zeroed();
        libc::sigemptyset(&mut block);
        libc::sigaddset(&mut block, libc::SIGTTOU);
        libc::sigprocmask(libc::SIG_BLOCK, &block, &mut old);
        libc::tcsetpgrp(fd, pgid);
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
}

impl Job {
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
    fn wait(&mut self, mask: Option<&crate::traps::WaitMask>) -> io::Result<Option<u8>> {
        while self.active() {
            if let Some(code) = mask.and_then(crate::traps::WaitMask::interrupt_status) {
                return Ok(Some(code));
            }
            let pid = self.group.map(|g| -g).unwrap_or_else(|| {
                self.children
                    .iter()
                    .find(|p| p.state == State::Running)
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
    tty: Option<Terminal>,
    table: Vec<Job>,
    serial: u32,
    // Bash keeps completed background statuses available to wait by PID.
    finished: std::collections::VecDeque<(pid_t, u8)>,
}

impl Jobs {
    pub fn init(&mut self) {
        let Ok(fd) = io::stderr().as_fd().try_clone_to_owned() else {
            return;
        };
        let old = unsafe { libc::getpgrp() };
        let owner = unsafe { libc::getpid() };
        if unsafe { libc::tcgetpgrp(fd.as_raw_fd()) } != old {
            return;
        }
        if old != owner && unsafe { libc::setpgid(0, 0) } < 0 {
            return;
        }
        let group = unsafe { libc::getpgrp() };
        if old != group {
            give_terminal(fd.as_raw_fd(), group);
        }
        unsafe {
            libc::signal(libc::SIGTSTP, libc::SIG_IGN);
        }
        let Ok(fd) = PrivateFile::new(fd.into()) else {
            return;
        };
        self.tty = Some(Terminal { fd, owner, group });
    }
    fn terminal(&self) -> Option<&Terminal> {
        self.tty
            .as_ref()
            .filter(|t| t.owner == unsafe { libc::getpid() })
    }
    pub fn launch(&self, background: bool) -> Launch {
        let tty = self.terminal();
        let foreground_fd = tty.filter(|_| !background).map(|t| t.fd.as_raw_fd());
        Launch {
            group: tty.map(|_| 0),
            tty: foreground_fd,
            shell_modes: foreground_fd.and_then(terminal_modes),
            background,
        }
    }
    pub fn detached(&mut self) {
        self.tty = None;
        self.table.clear();
        self.serial = 0;
        self.finished.clear();
    }
    pub fn restore(&self) {
        if let Some(t) = self.terminal() {
            give_terminal(t.fd.as_raw_fd(), t.group);
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
            let code = self.foreground(&mut job, false, launch.shell_modes)?;
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
        self.serial = self.table.iter().map(|j| j.id).max().unwrap_or(0) + 1;
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
            give_terminal(t.fd.as_raw_fd(), group);
            if let Some(modes) = &job.modes {
                unsafe {
                    libc::tcsetattr(t.fd.as_raw_fd(), libc::TCSANOW, modes);
                }
            }
        }
        let result = (|| {
            if resume {
                job.resume()?;
            }
            job.wait(None)
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
        result.map(|_| job.status())
    }
    pub fn poll(&mut self) {
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
                let id = s[1..].parse::<u32>().ok()?;
                self.table.iter().position(|j| j.id == id)
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

pub fn builtin(words: &[String], session: &mut Session) -> u8 {
    let jobs = &mut session.jobs;
    jobs.poll();
    match words[0].as_str() {
        "kill" => kill(jobs, &words[1..]),
        "jobs" => {
            let pid_only = words.get(1).is_some_and(|s| s == "-p");
            let mut out = io::stdout().lock();
            for j in &jobs.table {
                let result = if pid_only {
                    writeln!(
                        out,
                        "{}",
                        j.group
                            .unwrap_or_else(|| j.children.first().map_or(0, |p| p.pid))
                    )
                } else {
                    let state = if j.active() {
                        "Running"
                    } else if j.stopped() {
                        "Stopped"
                    } else {
                        "Done"
                    };
                    writeln!(out, "[{}] {state}\t{}", j.id, j.name)
                };
                if result.is_err() {
                    return 1;
                }
            }
            for i in (0..jobs.table.len()).rev() {
                if !jobs.table[i].active() && !jobs.table[i].stopped() {
                    let job = jobs.table.remove(i);
                    jobs.remember(&job);
                }
            }
            u8::from(out.flush().is_err())
        }
        "wait" => {
            let Ok(mask) = crate::traps::WaitMask::new(&session.traps) else {
                return 1;
            };
            if words.len() == 1 {
                for j in &mut jobs.table {
                    if let Ok(Some(code)) = j.wait(mask.as_ref()) {
                        return code;
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
                        return code;
                    }
                }
                return 0;
            }
            let mut code = 0;
            for spec in &words[1..] {
                if let Some(index) = jobs.select(spec) {
                    let j = &mut jobs.table[index];
                    code = match j.wait(mask.as_ref()) {
                        Ok(Some(interrupt)) => return interrupt,
                        Ok(None) => j.status(),
                        Err(_) => 127,
                    };
                    if !j.stopped() {
                        let job = jobs.table.remove(index);
                        jobs.remember(&job);
                    }
                } else if let Some((_, saved)) = spec
                    .parse::<pid_t>()
                    .ok()
                    .and_then(|pid| jobs.finished.iter().rev().find(|(p, _)| *p == pid))
                {
                    code = *saved;
                } else if let Some(s) = spec
                    .parse()
                    .ok()
                    .and_then(|pid| session.substitutions.wait(pid, mask.as_ref()))
                {
                    code = s;
                } else {
                    eprintln!("tonari: wait: {spec}: unknown child");
                    code = 127;
                }
            }
            code
        }
        name => {
            if jobs.terminal().is_none() {
                eprintln!("tonari: {name}: no job control");
                return 1;
            }
            let Some(i) = jobs.select(words.get(1).map(String::as_str).unwrap_or("")) else {
                eprintln!("tonari: {name}: no such job");
                return 1;
            };
            if name == "bg" {
                return u8::from(jobs.table[i].resume().is_err());
            }
            let mut job = jobs.table.remove(i);
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
