use super::*;

fn kill(jobs: &Jobs, mut args: &[String], posix: bool) -> u8 {
    let number = |s: &str| {
        if posix && s.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("SIG")) {
            None
        } else {
            crate::traps::number(s)
        }
    };
    if args.first().is_some_and(|s| s == "-l" || s == "-L") {
        let mut out = crate::builtins::stdout_for("kill");
        let mut status = 0;
        if args.len() == 1 {
            if posix {
                static NAMES: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
                    let mut text = String::new();
                    for (name, _) in crate::traps::signals().filter(|s| s.1 > 0) {
                        if !text.is_empty() {
                            text.push(' ');
                        }
                        text.push_str(name);
                    }
                    text.push('\n');
                    text
                });
                return u8::from(out.write_all(NAMES.as_bytes()).is_err());
            }
            return u8::from(crate::traps::print_signal_table(&mut out).is_err());
        } else {
            for arg in &args[1..] {
                let result = if let Ok(n) = arg.parse::<i32>() {
                    // A shell wait status encodes the signal once as 128 + signal.
                    let number = if n > 128 { n - 128 } else { n };
                    crate::traps::signals()
                        .find(|s| s.1 == number)
                        .map(|(name, _)| writeln!(out, "{name}"))
                } else {
                    number(arg).map(|n| writeln!(out, "{n}"))
                };
                let Some(result) = result else {
                    // stderr is unbuffered. Submit the whole diagnostic in one write.
                    let message = format!("tonari: kill: {arg}: invalid signal specification\n");
                    let _ = io::stderr().write_all(message.as_bytes());
                    status = 1;
                    continue;
                };
                if result.is_err() {
                    return 1;
                }
            }
        }
        return status;
    }
    let choice = match args.first().map(String::as_str) {
        Some(flag @ ("-s" | "-n")) => {
            let Some(signal) = args.get(1) else {
                diagnostic!("kill: {flag}: option requires an argument");
                return 1;
            };
            args = &args[2..];
            Some(signal.as_str())
        }
        Some(flag) if flag.len() > 2 && (flag.starts_with("-n") || flag.starts_with("-s")) => {
            args = &args[1..];
            Some(&flag[2..])
        }
        Some(flag) if flag.starts_with('-') && flag != "--" => {
            args = &args[1..];
            Some(&flag[1..])
        }
        _ => None,
    };
    let sig = if let Some(name) = choice {
        let Some(sig) = number(name) else {
            diagnostic!("kill: {name}: invalid signal specification");
            return 1;
        };
        sig
    } else {
        libc::SIGTERM
    };
    if args.first().is_some_and(|s| s == "--") {
        args = &args[1..];
    }
    if args.is_empty() {
        diagnostic!("kill: expected PID or job");
        return 2;
    }
    let mut status = 0;
    for spec in args {
        let send = |pid| unsafe { libc::kill(pid, sig) == 0 };
        if spec.starts_with('%') {
            let markers = marked_jobs(jobs.all_jobs());
            let Some(job) = list_position(jobs, spec, markers).and_then(|i| jobs.all_jobs().nth(i))
            else {
                diagnostic!("kill: {spec}: no such job");
                status = 1;
                continue;
            };
            let mut sent = false;
            if let Some(group) = job.group {
                sent = send(-group);
            } else {
                for process in &job.children {
                    if process.pid > 0 && !matches!(process.state, State::Done(_)) {
                        sent |= send(process.pid);
                    }
                }
            }
            if !sent {
                diagnostic!("kill: {spec}: could not signal target");
                status = 1;
            }
        } else if let Ok(pid) = spec.parse::<pid_t>() {
            if !send(pid) {
                diagnostic!("kill: ({pid}) - {}", io::Error::last_os_error());
                status = 1;
            }
        } else {
            diagnostic!("kill: `{spec}': not a pid or valid job spec");
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
        while let Some(arg) = crate::builtins::flags::next(&mut args) {
            let mut flags = arg[1..].chars();
            while let Some(flag) = flags.next() {
                match flag {
                    'n' => options.next = true,
                    'f' => options.force = true,
                    'p' => {
                        options.variable =
                            Some(crate::builtins::flags::value(flags.as_str(), &mut args)?);
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
        diagnostic!("wait: usage: wait [-fn] [-p var] [id ...]");
        return 2;
    };
    if let Some(name) = options.variable {
        if !session.valid_builtin_target(name) {
            diagnostic!("wait: {name}: not a valid identifier");
            return 1;
        }
        if session.unset_builtin_target(name) != 0 {
            return 1;
        }
    }
    let (code, pid) = if options.next {
        wait_next(session, ids, options.force)
    } else {
        wait_for_ids(session, ids, options.force)
    };
    if let (Some(name), Some(pid)) = (options.variable, pid)
        && session
            .assign_builtin_target(name, &pid.to_string())
            .is_err()
    {
        return 1;
    }
    code
}

fn wait_next(session: &mut Session, ids: &[String], force: bool) -> (u8, Option<pid_t>) {
    let jobs = &mut session.jobs;
    let mut selected = Vec::with_capacity(ids.len());
    let mut substitutions = Vec::new();
    for spec in ids {
        if let Some(index) = jobs.select(spec) {
            selected.push(jobs.table[index].id);
        } else if let Ok(pid) = spec.parse::<pid_t>()
            && let Some(&(_, code)) = jobs.finished.iter().rev().find(|(p, _)| *p == pid)
        {
            return (code, Some(pid));
        } else if let Ok(pid) = spec.parse::<pid_t>()
            && session.substitutions.knows(pid)
        {
            substitutions.push(pid);
        } else {
            diagnostic!("wait: {spec}: no such job");
        }
    }
    selected.sort_unstable();
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
        let (ready, mut running) = session
            .substitutions
            .poll_next(&substitutions, ids.is_empty());
        if let Some((code, pid)) = ready {
            return (code, Some(pid));
        }
        for (i, job) in jobs.table.iter().enumerate() {
            if !ids.is_empty() && selected.binary_search(&job.id).is_err() {
                continue;
            }
            if !job.active() && !job.stopped() {
                let job = jobs.table.remove(i);
                let pid = job.children.iter().rev().find(|p| p.pid > 0).map(|p| p.pid);
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
    let force = force || !session.jobs.monitor;
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
                    number.or_else(|| job.children.iter().rev().find(|p| p.pid > 0).map(|p| p.pid)),
                ),
                Err(_) => (127, None),
            };
            if !job.stopped() {
                let job = jobs.table.remove(i);
                if (session.interactive || !session.posix)
                    && !job.listed
                    && let Some(process) = job
                        .children
                        .iter()
                        .rev()
                        .find(|p| number.is_none_or(|n| n == p.pid))
                    && let Some(sig) = process.signal
                    && !matches!(sig, libc::SIGINT | libc::SIGTERM | libc::SIGPIPE)
                {
                    let reason = unsafe { libc::strsignal(sig) };
                    if !reason.is_null() {
                        stderr_line!(
                            "{} {}  {}",
                            process.pid,
                            unsafe { std::ffi::CStr::from_ptr(reason) }.to_string_lossy(),
                            job.name
                        );
                    }
                }
                if !session.posix {
                    jobs.remember(&job);
                }
            }
        } else if let Some(pid) = number {
            if let Some(i) = jobs.finished.iter().rposition(|(p, _)| *p == pid) {
                result = (jobs.finished[i].1, Some(pid));
                if session.posix {
                    jobs.finished.remove(i);
                }
            } else if let Some(code) = session.substitutions.wait(pid, mask.as_ref()) {
                if let Some(interrupted) = mask
                    .as_ref()
                    .and_then(crate::traps::WaitMask::interrupt_status)
                {
                    return (interrupted, None);
                }
                result = (code, Some(pid));
            } else {
                if !jobs.quiet {
                    diagnostic!("wait: {spec}: not a child of this shell");
                }
                result = (127, None);
            }
        } else {
            diagnostic!("wait: {spec}: not a valid child or job");
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

pub(super) fn marked_jobs<'a>(jobs: impl Iterator<Item = &'a Job>) -> (Option<u32>, Option<u32>) {
    let mut current = None;
    let mut previous = None;
    for job in jobs {
        let rank = Some((job.stopped_at, job.id));
        if rank > current {
            previous = current;
            current = rank;
        } else if rank > previous {
            previous = rank;
        }
    }
    (
        current.map(|(_, id)| id),
        previous.or(current).map(|(_, id)| id),
    )
}

pub(super) fn job_matches(name: &str, spec: &str) -> bool {
    match spec.strip_prefix('?') {
        Some(part) => name.contains(part),
        None => name.starts_with(spec),
    }
}

pub(super) fn list_position(
    jobs: &Jobs,
    spec: &str,
    markers: (Option<u32>, Option<u32>),
) -> Option<usize> {
    let mut all = jobs.all_jobs();
    let spec = spec.strip_prefix('%').unwrap_or(spec);
    let id = match spec {
        "" | "+" | "%" => markers.0,
        "-" => markers.1,
        s => s.parse::<u32>().ok(),
    };
    if let Some(id) = id {
        return all.position(|j| j.id == id);
    }
    let mut matches = all.enumerate().filter(|(_, j)| job_matches(&j.name, spec));
    let (index, _) = matches.next()?;
    matches.next().is_none().then_some(index)
}

fn list(jobs: &mut Jobs, args: &[String], posix: bool) -> u8 {
    list_to(jobs, args, posix, &mut crate::builtins::stdout_for("jobs"))
}

pub(super) fn list_to(jobs: &mut Jobs, args: &[String], posix: bool, out: &mut dyn Write) -> u8 {
    let mut mode = ' ';
    let mut changed = false;
    let mut filter = ' ';
    let mut args = args;
    while let Some(arg) = crate::builtins::flags::next(&mut args) {
        for flag in arg[1..].chars() {
            match flag {
                'p' | 'l' => mode = flag,
                'n' => changed = true,
                'r' | 's' => filter = flag,
                _ => {
                    diagnostic!("jobs: invalid option: {flag}");
                    return 2;
                }
            }
        }
    }
    let mut status = 0;
    let markers = marked_jobs(jobs.all_jobs());
    let all = if args.is_empty() {
        jobs.inherited.len() + jobs.table.len()
    } else {
        0
    };
    let mut selected = Vec::new();
    for spec in args {
        if let Some(index) = list_position(jobs, spec, markers) {
            selected.push(index);
        } else {
            diagnostic!("jobs: {spec}: no such job");
            status = 1;
        }
    }
    let (current, previous) = markers;
    for index in (0..all).chain(selected) {
        let inherited = jobs.inherited.len();
        let job = if index < inherited {
            &mut jobs.inherited[index]
        } else {
            &mut jobs.table[index - inherited]
        };
        let id = job.id;
        if (changed && job.listed)
            || (filter == 'r' && !job.active())
            || (filter == 's' && !job.stopped())
        {
            continue;
        }
        let pid = job
            .group
            .unwrap_or_else(|| job.children.iter().find(|p| p.pid > 0).map_or(0, |p| p.pid));
        if mode == 'p' {
            if writeln!(out, "{pid}").is_err() {
                return 1;
            }
            continue;
        }
        let state = job.display_state(posix);
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
                if child.pid <= 0 {
                    continue;
                }
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
        } else if writeln!(out, "[{id}]{marker}  {state:<26} {}{bg}", job.name).is_err() {
            return 1;
        }
        job.listed = true;
    }
    jobs.remove_reported();
    if out.flush().is_err() { 1 } else { status }
}

pub fn dispatch(words: &[String], session: &mut Session, last: u8) -> crate::Flow {
    if words[0] == "jobs" && words.get(1).is_some_and(|s| s == "-x") {
        let mut words = words[2..].iter();
        if words.clone().next().is_some_and(|s| s == "--") {
            words.next();
        }
        session.jobs.poll();
        let all = || session.jobs.all_jobs();
        let markers = marked_jobs(all());
        let command = crate::parser::SimpleCommand {
            words: words
                .map(|arg| {
                    let job = arg
                        .strip_prefix('%')
                        .and_then(|_| list_position(&session.jobs, arg, markers))
                        .and_then(|index| all().nth(index));
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
        "kill" => kill(jobs, &words[1..], session.posix),
        "jobs" => list(jobs, &words[1..], session.posix),
        "disown" => jobs.disown(&words[1..]),
        "bg" => {
            if !jobs.monitor {
                diagnostic!("bg: no job control");
                return 1;
            }
            let mut status = 0;
            let mut out = crate::builtins::stdout_for("bg");
            let operands = if words.get(1).is_some_and(|arg| arg == "--") {
                &words[2..]
            } else {
                &words[1..]
            };
            for spec in operands
                .iter()
                .map(String::as_str)
                .chain(operands.is_empty().then_some("%+"))
            {
                let Some(i) = jobs.select_job(spec) else {
                    diagnostic!("bg: {spec}: no such job");
                    status = 1;
                    continue;
                };
                let (current, previous) = marked_jobs(jobs.table.iter());
                let job = &mut jobs.table[i];
                if job.group.is_none() {
                    diagnostic!("bg: job {} started without job control", job.id);
                    status = 1;
                    continue;
                }
                if !job.stopped() {
                    diagnostic!("bg: job {} already in background", job.id);
                    continue;
                }
                if job.resume().is_err() {
                    status = 1;
                    continue;
                }
                session.last_background =
                    job.children.iter().rev().find(|p| p.pid > 0).map(|p| p.pid);
                let result = if session.posix {
                    writeln!(out, "[{}] {} &", job.id, job.name)
                } else {
                    let marker = if Some(job.id) == current {
                        '+'
                    } else if Some(job.id) == previous {
                        '-'
                    } else {
                        ' '
                    };
                    writeln!(out, "[{}]{marker} {} &", job.id, job.name)
                };
                if result.is_err() {
                    return 1;
                }
            }
            if out.flush().is_err() { 1 } else { status }
        }
        name => {
            if !jobs.monitor {
                diagnostic!("{name}: no job control");
                return 1;
            }
            let mut args = &words[1..];
            if args.first().is_some_and(|arg| arg == "--") {
                args = &args[1..];
            }
            let spec = args.first().map_or("%+", String::as_str);
            let Some(i) = jobs.select_job(spec) else {
                diagnostic!("{name}: {spec}: no such job");
                return 1;
            };
            if jobs.table[i].group.is_none() {
                diagnostic!(
                    "{name}: job {} started without job control",
                    jobs.table[i].id
                );
                return 1;
            }
            let mut job = jobs.table.remove(i);
            {
                let mut out = crate::builtins::stdout_for("fg");
                let _ = writeln!(out, "{}", job.name).and_then(|_| out.flush());
            }
            let modes = jobs
                .terminal()
                .and_then(|t| terminal_modes(t.fd.as_raw_fd()));
            let r = jobs.foreground(&mut job, true, modes);
            jobs.note_foreground_signal(&mut job);
            if job.stopped() {
                jobs.table.push(job);
                jobs.report_stopped();
            }
            r.unwrap_or(1)
        }
    }
}
