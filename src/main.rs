mod alias;
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
    input::text(session, source, status, false, false)
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

fn read_terminal_line(source: &mut Vec<u8>) -> io::Result<bool> {
    let mut buf = [0u8; 4096];
    loop {
        let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n == 0 {
            return Ok(false);
        }
        source.extend_from_slice(&buf[..n as usize]);
        if source.ends_with(b"\n") {
            return Ok(true);
        }
    }
}

fn interactive(session: &mut Session, mut status: u8, editing: bool) -> io::Result<u8> {
    session.jobs.init();
    let mut source = Vec::new();
    let mut history = history::History::default();
    loop {
        interrupted();
        if let Flow::Exit(code) = traps::dispatch(session, status) {
            return Ok(code);
        }
        session.jobs.poll();
        session.refresh_terminal_size();
        if source.is_empty() {
            if let Some(hook) = session.get("PROMPT_COMMAND").filter(|s| !s.is_empty()) {
                if let Flow::Exit(code) = eval_text(session, &hook, status) {
                    return Ok(code);
                }
            }
        }
        let prompt = prompt::render(session, !source.is_empty(), status);
        let result = if editing {
            editor::line(&prompt, &mut history, session).map(|input| match input {
                editor::Input::Line(line) => {
                    source.extend_from_slice(line.as_bytes());
                    Some(true)
                }
                editor::Input::End => Some(false),
                editor::Input::Cancel => None,
            })
        } else {
            let mut stderr = io::stderr().lock();
            stderr.write_all(prompt.as_bytes())?;
            stderr.flush()?;
            drop(stderr);
            read_terminal_line(&mut source).map(Some)
        };
        let more = match result {
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
            return Ok(status);
        }
        match std::str::from_utf8(&source) {
            Ok(text) => match parser::session_source(text, !more, session) {
                Ok(program) if program.is_empty() => {}
                Ok(program) => {
                    let record = history.accepts(session, text);
                    let cwd = session.get("PWD");
                    let flow = execute_program(session, program, status, false);
                    status = match flow {
                        Flow::Continue(s) | Flow::Return(s) | Flow::Exit(s) => s,
                        _ => 1,
                    };
                    if record && history.accepts(session, text) {
                        history.record(session, text, cwd.as_deref(), status);
                    }
                    if matches!(flow, Flow::Exit(_) | Flow::Return(_)) {
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

fn main() -> ExitCode {
    use cli::{Action, Input};
    let args = std::env::args_os();
    let terminal = || io::stdin().is_terminal() && io::stderr().is_terminal();
    let invocation = match cli::parse(args, terminal) {
        Ok(Action::Run(invocation)) => invocation,
        Ok(Action::Version) => {
            println!("tonari {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Ok(Action::Help) => {
            println!(
                "Usage: tonari [--noprofile] [--norc | --rcfile FILE] [-efilnsu] [-o pipefail] [-c COMMAND [NAME [ARG...]] | -s [ARG...] | FILE [ARG...]]"
            );
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("tonari: {e}");
            return ExitCode::from(2);
        }
    };
    let mut invocation = invocation;
    let mut session = Session::default();
    session.arg0 = std::mem::take(&mut invocation.arg0);
    session.positional = std::mem::take(&mut invocation.positional);
    session.errexit = invocation.errexit;
    session.noglob = invocation.noglob;
    session.noclobber = invocation.noclobber;
    session.shopt.braceexpand = invocation.braceexpand;
    session.nounset = invocation.nounset;
    session.pipefail = invocation.pipefail;
    session.interactive = invocation.interactive;
    session.command_string = matches!(invocation.input, Input::Command(_));
    session.shopt.expand_aliases = session.interactive;
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
        return ExitCode::FAILURE;
    }
    let initial = match startup::initialize(&mut session, &invocation) {
        Flow::Continue(status) => status,
        Flow::Return(status) | Flow::Exit(status) => {
            return ExitCode::from(traps::finish(&mut session, status));
        }
        Flow::Break(_) | Flow::LoopContinue(_) => return ExitCode::from(1),
    };
    let flow = match invocation.input {
        Input::Command(text) => input::text(&mut session, &text, initial, true, invocation.check),
        Input::File(path) => match std::fs::File::open(&path) {
            Ok(file) => {
                session
                    .source_names
                    .push(path.to_string_lossy().into_owned());
                session.function_names.push("main".into());
                input::file_script(&mut session, file, initial, invocation.check)
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
                && terminal()
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
        Flow::Continue(s) | Flow::Exit(s) | Flow::Return(s) => s,
        Flow::Break(_) | Flow::LoopContinue(_) => 1,
    };
    ExitCode::from(traps::finish(&mut session, code))
}
