// Keep the prefix and stderr formatter in the cold path; each caller supplies its fields.
macro_rules! diagnostic {
    ($($arg:tt)*) => { crate::print_diagnostic(format_args!($($arg)*)) };
}

#[cold]
#[inline(never)]
fn print_diagnostic(args: std::fmt::Arguments<'_>) {
    eprintln!("tonari: {args}");
}

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

fn remaining_input_time() -> Option<std::time::Duration> {
    INPUT_DEADLINE
        .get()
        .map(|end| end.saturating_duration_since(std::time::Instant::now()))
}

fn input_timeout() -> Option<libc::timespec> {
    remaining_input_time().map(|left| libc::timespec {
        tv_sec: left.as_secs() as _,
        tv_nsec: left.subsec_nanos() as _,
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
            diagnostic!("{e}");
            return 2;
        }
    };
    run_invocation(invocation)
}

fn run_invocation(mut invocation: cli::Invocation) -> u8 {
    use cli::Input;
    let mut session = Session::default();
    session.started_at = unsafe { libc::time(std::ptr::null_mut()) };
    let env = startup::Environment::read(&mut session, invocation.interactive);
    session.initialize_shell_level(&env);
    session.arg0 = std::mem::take(&mut invocation.arg0);
    session.positional = std::mem::take(&mut invocation.positional).into_iter();
    session.allexport = invocation.options.allexport;
    session.notify = invocation.options.notify;
    session.noexec = invocation.options.check;
    if let Some(enabled) = invocation.options.ignoreeof {
        if enabled {
            session.assign("IGNOREEOF", "10");
        } else {
            session.unset_variable("ignoreeof");
            session.unset_variable("IGNOREEOF");
        }
    } else if let Some(value) = env.ignore_eof.as_deref() {
        session.set_eof_variable(Some(value));
    }
    session.onecmd = invocation.options.onecmd;
    session.xtrace = invocation.options.xtrace;
    session.errexit = invocation.options.errexit;
    session.verbose = invocation.options.verbose;
    session.errtrace = invocation.options.errtrace;
    session.functrace = invocation.options.functrace;
    session.noglob = invocation.options.noglob;
    session.noclobber = invocation.options.noclobber;
    session.hashes.enabled = invocation.options.hashall;
    session.shopt.braceexpand = invocation.options.braceexpand;
    session.nounset = invocation.options.nounset;
    session.pipefail = invocation.options.pipefail;
    session.physical = invocation.options.physical;
    session.keyword = invocation.options.keyword;
    session.shopt.nolog = invocation.options.nolog;
    session.interactive = invocation.interactive;
    session.login = invocation.login;
    session.history_enabled = invocation.interactive;
    session.histexpand = invocation.options.histexpand.unwrap_or(session.interactive);
    // The selected keymap is also observable in a forced interactive script.
    // Terminal capabilities are checked separately when opening the editor.
    session.emacs = session.interactive && !invocation.noediting;
    if session.interactive
        && let Err(e) = session.traps.init_hangup()
    {
        diagnostic!("SIGHUP: {e}");
        return 1;
    }
    session.jobs.monitor = invocation.options.monitor.unwrap_or(session.interactive);
    if session.jobs.monitor {
        session.jobs.init(session.interactive);
    }
    session.command_string = matches!(invocation.input, Input::Command(_));
    if !invocation.pretty
        && let Input::Command(text) = &mut invocation.input
    {
        if env.execution_string {
            session.assign("BASH_EXECUTION_STRING", text);
        }
        session.execution_string = Some(std::rc::Rc::new(std::mem::take(text)));
    }
    session.command_history_expansion = invocation.options.histexpand == Some(true);
    session.stdin_script = matches!(invocation.input, Input::Stdin);
    if let Some(enabled) = invocation
        .options
        .posix
        .filter(|&enabled| enabled != env.posix)
    {
        session.set_posix(enabled);
    } else {
        session.apply_posix_mode(env.posix);
    }
    session.shopt.progcomp_alias = session.interactive;
    session.inherit_options(&env);
    for (enable, name) in std::mem::take(&mut invocation.options.shopts) {
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
        if !env.prompt.unwrap_or(false) {
            session.assign("PS1", "tonari$ ");
        }
        if !env.history_file.unwrap_or(false)
            && let Some(home) = session.get("HOME").or_else(crate::word::account_home)
        {
            let file = if session.posix {
                ".sh_history"
            } else {
                ".bash_history"
            };
            session.assign("HISTFILE", &format!("{home}/{file}"));
        }
    }
    if session.interactive && traps::listen_for_interrupts().is_err() {
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
        && !invocation.options.check
        && (!session.interactive || !matches!(invocation.input, Input::Stdin))
        && let Flow::Exit(code) = startup::debugger(&mut session)
    {
        return traps::finish(&mut session, code);
    }
    if invocation.as_sh && !session.posix {
        session.set_posix(true);
    }
    let flow = match invocation.input {
        Input::Command(_) => {
            let text = session.execution_string.clone().unwrap();
            input::text(
                &mut session,
                &text,
                initial,
                true,
                invocation.options.check,
                None,
            )
        }
        Input::File(path) => match std::fs::File::open(&path) {
            Ok(file) => {
                session.push_source_path(&path);
                session.function_names.push("main".into());
                session.call_lines.push(0);
                input::file_script(&mut session, file, None, initial, invocation.options.check)
            }
            Err(e) => {
                diagnostic!("{}: {e}", path.display());
                Flow::Continue(if e.kind() == io::ErrorKind::NotFound {
                    127
                } else {
                    126
                })
            }
        },
        Input::Stdin if invocation.interactive => {
            match interactive(&mut session, initial, !invocation.noediting) {
                Ok(code) => Flow::Continue(code),
                Err(e) => {
                    diagnostic!("{e}");
                    Flow::Continue(1)
                }
            }
        }
        Input::Stdin => input::stdin_script(&mut session, invocation.options.check),
    };
    let code = match flow {
        Flow::Discard(s) | Flow::Continue(s) | Flow::Exit(s) | Flow::Return(s) => s,
        Flow::Break(..) | Flow::LoopContinue(_) => 1,
    };
    traps::finish(&mut session, code)
}
