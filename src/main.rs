mod alias;
#[cfg(all(target_os = "linux", target_env = "musl"))]
mod allocator;
mod arithmetic;
mod builtins;
mod cli;
mod complete;
mod completion_spec;
mod editor;
mod eval;
mod execute;
mod expansion;
mod history;
mod input;
mod jobs;
mod parser;
mod pattern;
mod prompt;
mod session;
mod shell_time;
mod startup;
mod subshell;
mod test_expression;
mod traps;
use subshell::capture_substitution;
mod word;

use session::Session;
pub(crate) use startup::source_file;
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use execute::Flow;

use eval::{call_function, execute_program};

fn eval_text(session: &mut Session, source: &str, status: u8) -> Flow {
    input::text(session, source, status, false, false, None)
}

static INTERRUPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn interrupted() -> bool {
    INTERRUPTED.swap(false, std::sync::atomic::Ordering::Relaxed)
}
extern "C" fn interrupt(signal: libc::c_int) {
    if signal == libc::SIGINT {
        INTERRUPTED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn listen_for_interrupts() -> io::Result<()> {
    // A caught signal interrupts the terminal read and resets to its default
    // disposition on exec, so foreground programs still receive Ctrl-C.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = interrupt as *const () as usize;
    unsafe { libc::sigemptyset(&mut action.sa_mask) };
    for signal in [libc::SIGINT, libc::SIGQUIT, libc::SIGTERM] {
        let mut old = std::mem::MaybeUninit::uninit();
        if unsafe { libc::sigaction(signal, std::ptr::null(), old.as_mut_ptr()) } < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { old.assume_init().sa_sigaction } == libc::SIG_IGN {
            continue;
        }
        if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

// A foreground interrupt may arrive after the prompt is drawn but before read(2).
// pselect atomically restores the signal mask while waiting for input.
thread_local! {
    static INPUT_DEADLINE: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}
// Block child notifications before reaping to close the exit-before-sleep race.
// A key, signal or prompt deadline wakes the same wait.
fn wait_for_job_notice(
    jobs: &mut jobs::Jobs,
    mut display: impl FnMut(&mut jobs::Jobs) -> io::Result<()>,
) -> io::Result<()> {
    if !jobs.awaiting_notification() || editor::ready(0, 0) {
        return Ok(());
    }
    let mask = traps::WaitMask::for_children(0)?;
    loop {
        jobs.poll();
        if jobs.has_notification() {
            display(jobs)?;
        }
        if !jobs.awaiting_notification()
            || INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed)
            || traps::input_interrupted()
            || mask.input_or_child(input_timeout().as_ref())?
        {
            return Ok(());
        }
    }
}

fn input_timeout() -> Option<libc::timespec> {
    INPUT_DEADLINE.get().map(|end| {
        let left = end.saturating_duration_since(std::time::Instant::now());
        libc::timespec {
            tv_sec: left.as_secs() as _,
            tv_nsec: left.subsec_nanos() as _,
        }
    })
}

fn terminal_read(buf: &mut [u8], raw: bool) -> io::Result<usize> {
    unsafe {
        let mut blocked = std::mem::zeroed();
        let mut original = std::mem::zeroed();
        libc::sigfillset(&mut blocked);
        if libc::sigprocmask(libc::SIG_BLOCK, &blocked, &mut original) < 0 {
            return Err(io::Error::last_os_error());
        }
        let result = (|| {
            if INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed) || traps::input_interrupted()
            {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let mut read_set = std::mem::zeroed();
            libc::FD_ZERO(&mut read_set);
            libc::FD_SET(0, &mut read_set);
            let timeout = input_timeout();
            let selected = libc::pselect(
                1,
                &mut read_set,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                timeout
                    .as_ref()
                    .map_or(std::ptr::null(), std::ptr::from_ref),
                &original,
            );
            if selected == 0 {
                return Err(io::ErrorKind::TimedOut.into());
            }
            if selected < 0 {
                return Err(io::Error::last_os_error());
            }
            // A terminal interrupt can flush even bytes already reported readable.
            // VMIN=0 makes the editor read nonblocking after pselect without modifying
            // the open file description shared with external foreground programs.
            if raw {
                let n = libc::read(0, buf.as_mut_ptr().cast(), buf.len());
                if n < 0 {
                    return Err(io::Error::last_os_error());
                }
                if n == 0 {
                    let mut p = libc::pollfd {
                        fd: 0,
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    if libc::poll(&mut p, 1, 0) >= 0
                        && p.revents & (libc::POLLHUP | libc::POLLERR) == 0
                    {
                        return Err(io::ErrorKind::WouldBlock.into());
                    }
                }
                return Ok(n as usize);
            }
            let flags = libc::fcntl(0, libc::F_GETFL);
            if flags < 0 || libc::fcntl(0, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
                return Err(io::Error::last_os_error());
            }
            let n = libc::read(0, buf.as_mut_ptr().cast(), buf.len());
            let read = if n >= 0 {
                Ok(n as usize)
            } else {
                Err(io::Error::last_os_error())
            };
            if libc::fcntl(0, libc::F_SETFL, flags) < 0 {
                return Err(io::Error::last_os_error());
            }
            read
        })();
        libc::sigprocmask(libc::SIG_SETMASK, &original, std::ptr::null_mut());
        result
    }
}

fn read_terminal_line(source: &mut Vec<u8>, terminal: bool) -> io::Result<bool> {
    let seekable = !terminal && unsafe { libc::lseek(0, 0, libc::SEEK_CUR) } >= 0;
    let mut buf = [0u8; 4096];
    loop {
        let size = if terminal || seekable { buf.len() } else { 1 };
        let n = match terminal_read(&mut buf[..size], false) {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
            result => result?,
        };
        if n == 0 {
            return Ok(false);
        }
        let mut used = n;
        if seekable && let Some(end) = memchr::memchr(b'\n', &buf[..n]) {
            used = end + 1;
            if used < n
                && unsafe { libc::lseek(0, -((n - used) as libc::off_t), libc::SEEK_CUR) } < 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        source.extend_from_slice(&buf[..used]);
        if source.ends_with(b"\n") {
            return Ok(true);
        }
    }
}

fn interactive(session: &mut Session, status: u8, editing: bool) -> io::Result<u8> {
    let terminal_input = io::stdin().is_terminal();
    history::start_interactive(session);
    let result = interactive_loop(session, status, editing, terminal_input);
    history::finish_interactive(session);
    result
}

fn interactive_loop(
    session: &mut Session,
    mut status: u8,
    editing: bool,
    terminal_input: bool,
) -> io::Result<u8> {
    let mut source = Vec::new();
    let mut history = history::History::default();
    let mut eof_count = 0;
    let mut editable_history = String::new();
    loop {
        interrupted();
        if let Flow::Exit(code) = traps::dispatch(session, status) {
            return Ok(code);
        }
        session.jobs.poll();
        session.jobs.report_foreground();
        if (session.notify || session.posix) && session.jobs.monitor {
            session.jobs.report_changes(session.posix);
        }
        let terminal_size = session.refresh_terminal_size();
        if source.is_empty() {
            for hook in session.array_values("PROMPT_COMMAND") {
                if !hook.is_empty()
                    && let Flow::Exit(code) = eval_text(session, &hook, status)
                {
                    return Ok(code);
                }
            }
        }
        let prompt = prompt::render(session, !source.is_empty(), status);
        let timeout = session
            .get("TMOUT")
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|&n| n > 0);
        INPUT_DEADLINE.set(timeout.and_then(|n| {
            std::time::Instant::now().checked_add(std::time::Duration::from_secs(n))
        }));
        let result = if editing && (session.emacs || session.vi) {
            match editor::line(
                &prompt,
                &mut history,
                session,
                status,
                editor::Start::Command(std::mem::take(&mut editable_history)),
                source.is_empty(),
                terminal_size,
            ) {
                Ok(editor::Input::Exit(code)) => return Ok(code),
                Ok(editor::Input::Line(line)) => {
                    source.extend_from_slice(line.as_bytes());
                    Ok(Some(true))
                }
                Ok(editor::Input::Directory(result)) => {
                    INPUT_DEADLINE.set(None);
                    status = match result {
                        Ok(()) => 0,
                        Err(error) => {
                            eprintln!("tonari: cd: {error}");
                            1
                        }
                    };
                    continue;
                }
                Ok(editor::Input::End) => Ok(Some(false)),
                Ok(editor::Input::Cancel) => Ok(None),
                Err(error) => Err(error),
            }
        } else {
            let mut stderr = io::stderr().lock();
            stderr.write_all(prompt.as_bytes())?;
            stderr.flush()?;
            drop(stderr);
            let ready = if session.notify && terminal_input {
                wait_for_job_notice(&mut session.jobs, |jobs| {
                    let mut out = io::stderr().lock();
                    writeln!(out)?;
                    jobs.report_changes(session.posix);
                    out.write_all(prompt.as_bytes())?;
                    out.flush()
                })
            } else {
                Ok(())
            };
            ready
                .and_then(|()| read_terminal_line(&mut source, terminal_input))
                .map(Some)
        };
        INPUT_DEADLINE.set(None);
        let more = match result {
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                eprintln!("tonari: timed out waiting for input");
                if startup::defer_interactive_exit(session) {
                    continue;
                }
                return Ok(startup::logout(session, status, status));
            }
            Ok(Some(more)) => more,
            Ok(None) => {
                source.clear();
                if traps::pending().is_none() && session.traps.catches(libc::SIGINT) {
                    unsafe { libc::raise(libc::SIGINT) };
                }
                if traps::pending().is_none() {
                    status = 130;
                }
                continue;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                eprintln!();
                source.clear();
                if traps::pending().is_none() {
                    status = 130;
                }
                continue;
            }
            Err(e) => return Err(e),
        };
        if more {
            // The terminal can be resized while this line is being edited.
            session.refresh_terminal_size();
        }
        if !more && source.is_empty() {
            if session.ignoreeof && eof_count < session.ignoreeof_limit {
                eof_count += 1;
                eprintln!(
                    "Use \"{}\" to leave the shell.",
                    if session.login { "logout" } else { "exit" }
                );
                continue;
            }
            if startup::defer_interactive_exit(session) {
                continue;
            }
            return Ok(startup::logout(session, status, status));
        }
        eof_count = 0;
        let expanded;
        let text = std::str::from_utf8(&source);
        let text = if session.histexpand
            && let Ok(line) = text
        {
            match history::expand::expand(session, line) {
                Ok(event) => {
                    expanded = event.text;
                    if event.print {
                        eprint!("{}", expanded);
                        session.begin_history_line(&expanded);
                        session.finish_history_line();
                        source.clear();
                        continue;
                    }
                    if matches!(expanded, std::borrow::Cow::Owned(_)) {
                        if session.shopt.histverify && editing && (session.vi || session.emacs) {
                            editable_history = expanded.into_owned();
                            if editable_history.ends_with('\n') {
                                editable_history.pop();
                            }
                            source.clear();
                            continue;
                        }
                        eprint!("{}", expanded);
                    }
                    Ok(expanded.as_ref())
                }
                Err(e) => {
                    eprintln!("tonari: {e}");
                    if session.shopt.histreedit && editing && (session.vi || session.emacs) {
                        editable_history = line.strip_suffix('\n').unwrap_or(line).to_owned();
                    }
                    source.clear();
                    status = 1;
                    continue;
                }
            }
        } else {
            text
        };
        match text {
            Ok(text) => match parser::session_source(text, !more, session) {
                Ok(program) if program.is_empty() => {}
                Ok(program) => {
                    if let Some(p) = session.get("PS0").filter(|p| !p.is_empty()) {
                        eprint!("{}", prompt::render_text(session, p, status));
                    }
                    session.command_number += 1;
                    let record = session.begin_history_line(text);
                    let cwd = if record { session.get("PWD") } else { None };
                    let flow = if session.noexec {
                        Flow::Continue(0)
                    } else {
                        execute_program(session, program, status, false)
                    };
                    status = match flow {
                        Flow::Discard(s) | Flow::Continue(s) | Flow::Return(s) | Flow::Exit(s) => s,
                        _ => 1,
                    };
                    if record {
                        session.persist_current_history(cwd.as_deref(), status, &mut history);
                        session.finish_history_line();
                    }
                    if session.onecmd || matches!(flow, Flow::Exit(_) | Flow::Return(_)) {
                        return Ok(status);
                    }
                }
                Err(e) if more && e.incomplete() => continue,
                Err(e) => {
                    eprintln!("tonari: {}", e.message());
                    status = 2;
                }
            },
            Err(_) => {
                eprintln!("tonari: expected UTF-8 command text");
                status = 2;
            }
        }
        if !more {
            return Ok(status);
        }
        source.clear();
    }
}

fn shell_terminal() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

// Script fallback can reenter invoke() in a child at an arbitrary call depth.
static MAIN_STACK_ORIGIN: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

fn main() -> ExitCode {
    traps::enter_process();
    let marker = 0u8;
    MAIN_STACK_ORIGIN.get_or_init(|| std::ptr::from_ref(&marker).addr());
    invoke(std::env::args_os()).into()
}

fn invoke(args: impl Iterator<Item = std::ffi::OsString>) -> u8 {
    use cli::Action;
    let invocation = match cli::parse(args, shell_terminal) {
        Ok(Action::Run(invocation)) => invocation,
        Ok(Action::Version) => {
            println!("tonari {}", env!("CARGO_PKG_VERSION"));
            return 0;
        }
        Ok(Action::Help) => {
            println!(
                "Usage: tonari [--noprofile] [--norc | --rcfile FILE] [-efilnsu] [-o OPTION] [-O SHOPT] [-c COMMAND [NAME [ARG...]] | -s [ARG...] | FILE [ARG...]]"
            );
            return 0;
        }
        Err(e) => {
            eprintln!("tonari: {e}");
            return 2;
        }
    };
    run_invocation(invocation)
}

fn run_invocation(mut invocation: cli::Invocation) -> u8 {
    use cli::Input;
    let mut session = Session::default();
    session.started_at = unsafe { libc::time(std::ptr::null_mut()) };
    session.import_functions();
    session.initialize_shell_level();
    session.arg0 = std::mem::take(&mut invocation.arg0);
    session.positional = std::mem::take(&mut invocation.positional).into_iter();
    session.allexport = invocation.allexport;
    session.notify = invocation.notify;
    session.noexec = invocation.check;
    if let Some(enabled) = invocation.ignoreeof {
        if enabled {
            session.assign("IGNOREEOF", "10");
        } else {
            session.unset_variable("ignoreeof");
            session.unset_variable("IGNOREEOF");
        }
    } else if let Some(value) =
        std::env::var_os("IGNOREEOF").or_else(|| std::env::var_os("ignoreeof"))
    {
        session.set_eof_variable(Some(value.to_str().unwrap_or("")));
    }
    session.onecmd = invocation.onecmd;
    session.xtrace = invocation.xtrace;
    session.errexit = invocation.errexit;
    session.verbose = invocation.verbose;
    session.errtrace = invocation.errtrace;
    session.functrace = invocation.functrace;
    session.noglob = invocation.noglob;
    session.noclobber = invocation.noclobber;
    session.hashes.enabled = invocation.hashall;
    session.shopt.braceexpand = invocation.braceexpand;
    session.nounset = invocation.nounset;
    session.pipefail = invocation.pipefail;
    session.physical = invocation.physical;
    session.keyword = invocation.keyword;
    session.shopt.nolog = invocation.nolog;
    session.interactive = invocation.interactive;
    session.login = invocation.login;
    session.history_enabled = invocation.interactive;
    session.histexpand = invocation.histexpand.unwrap_or(session.interactive);
    // The selected keymap is also observable in a forced interactive script.
    // Terminal capabilities are checked separately when opening the editor.
    session.emacs = session.interactive && !invocation.noediting;
    if session.interactive
        && let Err(e) = session.traps.init_hangup()
    {
        eprintln!("tonari: SIGHUP: {e}");
        return 1;
    }
    session.jobs.monitor = invocation.monitor.unwrap_or(session.interactive);
    if session.jobs.monitor {
        session.jobs.init(session.interactive);
    }
    session.command_string = matches!(invocation.input, Input::Command(_));
    if !invocation.pretty
        && let Input::Command(text) = &mut invocation.input
    {
        if std::env::var_os("BASH_EXECUTION_STRING").is_some() {
            session.assign("BASH_EXECUTION_STRING", text);
        }
        session.execution_string = Some(std::rc::Rc::new(std::mem::take(text)));
    }
    session.command_history_expansion = invocation.histexpand == Some(true);
    session.stdin_script = matches!(invocation.input, Input::Stdin);
    session.set_posix(invocation.posix);
    session.shopt.progcomp_alias = session.interactive;
    session.inherit_options();
    for (enable, name) in std::mem::take(&mut invocation.shopts) {
        let options = match name {
            Some(name) => vec![if enable { "-s" } else { "-u" }.into(), name],
            None if enable => Vec::new(),
            None => vec!["-p".into()],
        };
        if session.shopt(&options) != 0 {
            return 2;
        }
    }
    if invocation.pretty {
        return input::print_script(&session, invocation.input);
    }
    if session.interactive {
        if session.get("PS1").is_none() {
            session.assign("PS1", "tonari$ ");
        }
        if session.get("HISTFILE").is_none() {
            if let Some(home) = session.get("HOME").or_else(crate::word::account_home) {
                let file = if session.posix {
                    ".sh_history"
                } else {
                    ".bash_history"
                };
                session.assign("HISTFILE", &format!("{home}/{file}"));
            }
        }
    }
    if session.interactive && listen_for_interrupts().is_err() {
        return 1;
    }
    let initial = match startup::initialize(&mut session, &invocation) {
        Flow::Discard(status) | Flow::Continue(status) => status,
        Flow::Return(status) | Flow::Exit(status) => {
            return traps::finish(&mut session, status);
        }
        Flow::Break(..) | Flow::LoopContinue(_) => return 1,
    };
    if invocation.debugger
        && !invocation.check
        && (!session.interactive || !matches!(invocation.input, Input::Stdin))
    {
        if let Flow::Exit(code) = startup::debugger(&mut session) {
            return traps::finish(&mut session, code);
        }
    }
    if invocation.as_sh && !session.posix {
        session.set_posix(true);
    }
    let flow = match invocation.input {
        Input::Command(_) => {
            let text = session.execution_string.clone().unwrap();
            input::text(&mut session, &text, initial, true, invocation.check, None)
        }
        Input::File(path) => match std::fs::File::open(&path) {
            Ok(file) => {
                session.push_source_path(&path);
                session.function_names.push("main".into());
                session.call_lines.push(0);
                input::file_script(&mut session, file, None, initial, invocation.check)
            }
            Err(e) => {
                eprintln!("tonari: {}: {e}", path.display());
                Flow::Continue(if e.kind() == io::ErrorKind::NotFound {
                    127
                } else {
                    126
                })
            }
        },
        Input::Stdin if invocation.interactive => match interactive(
            &mut session,
            initial,
            !invocation.noediting
                && shell_terminal()
                && std::env::var_os("TERM").is_some_and(|v| v != "dumb"),
        ) {
            Ok(code) => Flow::Continue(code),
            Err(e) => {
                eprintln!("tonari: {e}");
                Flow::Continue(1)
            }
        },
        Input::Stdin => input::stdin_script(&mut session, invocation.check),
    };
    let code = match flow {
        Flow::Discard(s) | Flow::Continue(s) | Flow::Exit(s) | Flow::Return(s) => s,
        Flow::Break(..) | Flow::LoopContinue(_) => 1,
    };
    traps::finish(&mut session, code)
}
