mod interactive;
use interactive::{interactive, shell_terminal, terminal_read};
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
struct ScopedCell<T: Copy + 'static> {
    slot: &'static std::thread::LocalKey<std::cell::Cell<T>>,
    previous: T,
}

impl<T: Copy + 'static> ScopedCell<T> {
    fn enter(slot: &'static std::thread::LocalKey<std::cell::Cell<T>>, value: T) -> Self {
        Self {
            slot,
            previous: slot.replace(value),
        }
    }
}

impl<T: Copy + 'static> Drop for ScopedCell<T> {
    fn drop(&mut self) {
        self.slot.set(self.previous);
    }
}

fn input_deadline(timeout: Option<std::time::Duration>) -> ScopedCell<Option<std::time::Instant>> {
    ScopedCell::enter(
        &INPUT_DEADLINE,
        timeout.and_then(|t| std::time::Instant::now().checked_add(t)),
    )
}

#[test]
fn nested_deadlines_restore_on_early_return() {
    let outer = input_deadline(Some(std::time::Duration::from_secs(5)));
    let previous = INPUT_DEADLINE.get().unwrap();
    fn cancel() -> Option<()> {
        let _inner = input_deadline(None);
        assert!(input_timeout().is_none());
        None?;
        Some(())
    }
    assert!(cancel().is_none());
    assert_eq!(INPUT_DEADLINE.get(), Some(previous));
    drop(outer);
    assert!(input_timeout().is_none());
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

// Reserve half the soft limit for callers, signal frames and nested execution.
#[cold]
fn stack_budget_exceeded() -> bool {
    const STACK_RESERVE: u64 = 256 * 1024;
    let Some(origin) = MAIN_STACK_ORIGIN.get() else {
        return true;
    };
    let mut limit = std::mem::MaybeUninit::uninit();
    if unsafe { libc::getrlimit(libc::RLIMIT_STACK, limit.as_mut_ptr()) } != 0 {
        return true;
    }
    let used = origin.abs_diff(std::ptr::from_ref(&limit).addr()) as u64;
    used + STACK_RESERVE >= unsafe { limit.assume_init() }.rlim_cur / 2
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
