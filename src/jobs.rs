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
    fn wait(&mut self) -> io::Result<()> {
        while self.active() {
            let pid = self.group.map(|g| -g).unwrap_or_else(|| {
                self.children
                    .iter()
                    .find(|p| p.state == State::Running)
                    .unwrap()
                    .pid
            });
            let mut status = 0;
            let got =
                unsafe { libc::waitpid(pid, &mut status, libc::WUNTRACED | libc::WCONTINUED) };
            if got < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            self.update(got, status);
        }
        Ok(())
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
            if job.stopped() {
                self.store(job);
                eprintln!("[{}]+ Stopped", self.serial);
            }
            Ok((code, None))
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
            job.wait()
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
        result.map(|()| job.status())
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

pub fn builtin(words: &[String], session: &mut Session) -> u8 {
    let jobs = &mut session.jobs;
    jobs.poll();
    match words[0].as_str() {
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
            jobs.table.retain(|j| j.active() || j.stopped());
            u8::from(out.flush().is_err())
        }
        "wait" => {
            if words.len() == 1 {
                for j in &mut jobs.table {
                    let _ = j.wait();
                }
                jobs.table.retain(Job::stopped);
                if let Some(pid) = session.last_background {
                    session.substitutions.wait(pid);
                }
                return 0;
            }
            let mut code = 0;
            for spec in &words[1..] {
                if let Some(index) = jobs.select(spec) {
                    let j = &mut jobs.table[index];
                    code = j.wait().map(|()| j.status()).unwrap_or(127);
                    if !j.stopped() {
                        jobs.table.remove(index);
                    }
                } else if let Some(s) = spec
                    .parse()
                    .ok()
                    .and_then(|pid| session.substitutions.wait(pid))
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
