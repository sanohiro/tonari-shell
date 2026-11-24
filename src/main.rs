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
    for signal in [libc::SIGINT, libc::SIGQUIT] {
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
            let timeout = INPUT_DEADLINE.get().map(|end| {
                let left = end.saturating_duration_since(std::time::Instant::now());
                libc::timespec {
                    tv_sec: left.as_secs() as _,
                    tv_nsec: left.subsec_nanos() as _,
                }
            });
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

fn read_terminal_line(source: &mut Vec<u8>) -> io::Result<bool> {
    let mut buf = [0u8; 4096];
    loop {
        let n = match terminal_read(&mut buf, false) {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
            result => result?,
        };
        if n == 0 {
            return Ok(false);
        }
        source.extend_from_slice(&buf[..n]);
        if source.ends_with(b"\n") {
            return Ok(true);
        }
    }
}

fn interactive(session: &mut Session, mut status: u8, editing: bool) -> io::Result<u8> {
    session.jobs.init();
    session.initial_history = history::InitialJournal::snapshot(session).map(std::rc::Rc::new);
    let mut source = Vec::new();
    let mut history = history::History::default();
    let mut eof_count = 0;
    loop {
        interrupted();
        if let Flow::Exit(code) = traps::dispatch(session, status) {
            return Ok(code);
        }
        session.jobs.poll();
        session.refresh_terminal_size();
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
                None,
                source.is_empty(),
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
            read_terminal_line(&mut source).map(Some)
        };
        INPUT_DEADLINE.set(None);
        let more = match result {
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                eprintln!("tonari: timed out waiting for input");
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
        if !more && source.is_empty() {
            if session
                .get("IGNOREEOF")
                .is_some_and(|v| eof_count < v.parse::<usize>().unwrap_or(10))
            {
                eof_count += 1;
                eprintln!(
                    "Use \"{}\" to leave the shell.",
                    if session.login { "logout" } else { "exit" }
                );
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
                        eprint!("{}", expanded);
                    }
                    Ok(expanded.as_ref())
                }
                Err(e) => {
                    eprintln!("tonari: {e}");
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
                    let cwd = session.get("PWD");
                    let flow = execute_program(session, program, status, false);
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

fn main() -> ExitCode {
    invoke(std::env::args_os()).into()
}

fn invoke(args: impl Iterator<Item = std::ffi::OsString>) -> u8 {
    use cli::Action;
    let initial_terminal = std::cell::LazyCell::new(shell_terminal as fn() -> bool);
    let invocation = match cli::parse(args, || *initial_terminal) {
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
    run_invocation(invocation, initial_terminal)
}

fn run_invocation(
    mut invocation: cli::Invocation,
    initial_terminal: std::cell::LazyCell<bool, fn() -> bool>,
) -> u8 {
    use cli::Input;
    let mut session = Session::default();
    session.import_functions();
    session.initialize_shell_level();
    session.arg0 = std::mem::take(&mut invocation.arg0);
    session.positional = std::mem::take(&mut invocation.positional);
    session.allexport = invocation.allexport;
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
    session.interactive = invocation.interactive;
    session.login = invocation.login;
    session.history_enabled = invocation.interactive;
    session.histexpand = invocation.histexpand.unwrap_or(session.interactive);
    session.emacs = session.interactive
        && !invocation.noediting
        && *initial_terminal
        && !session.get("TERM").is_some_and(|s| s == "dumb");
    if session.interactive
        && let Err(e) = session.traps.init_hangup()
    {
        eprintln!("tonari: SIGHUP: {e}");
        return 1;
    }
    session.jobs.monitor = invocation.monitor.unwrap_or(session.interactive);
    session.command_string = matches!(invocation.input, Input::Command(_));
    session.shopt.expand_aliases = session.interactive;
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
    if session.interactive {
        if session.get("PS1").is_none() {
            session.assign("PS1", "tonari$ ");
        }
        if session.get("HISTFILE").is_none() {
            if let Some(home) = session.get("HOME") {
                session.assign("HISTFILE", &format!("{home}/.bash_history"));
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
        Flow::Break(_) | Flow::LoopContinue(_) => return 1,
    };
    let flow = match invocation.input {
        Input::Command(text) => {
            input::text(&mut session, &text, initial, true, invocation.check, None)
        }
        Input::File(path) => match std::fs::File::open(&path) {
            Ok(file) => {
                session
                    .source_names
                    .push(path.to_string_lossy().as_ref().into());
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
        Flow::Break(_) | Flow::LoopContinue(_) => 1,
    };
    traps::finish(&mut session, code)
}
