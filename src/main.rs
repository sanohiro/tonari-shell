// Diagnostics are best-effort, including when stderr is no longer writable.
macro_rules! stderr_text {
    ($($arg:tt)*) => { crate::write_stderr(format_args!($($arg)*)) };
}

macro_rules! stderr_line {
    () => { stderr_text!("\n") };
    ($($arg:tt)+) => { stderr_text!("{}\n", format_args!($($arg)+)) };
}

fn write_stderr(args: std::fmt::Arguments<'_>) {
    let _ = std::io::stderr().lock().write_fmt(args);
}

// Keep the prefix and stderr formatter in the cold path; each caller supplies its fields.
macro_rules! diagnostic {
    ($($arg:tt)*) => { crate::print_diagnostic(format_args!($($arg)*)) };
}

#[cold]
#[inline(never)]
fn print_diagnostic(args: std::fmt::Arguments<'_>) {
    stderr_line!("tonari: {args}");
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
    if !jobs.awaiting_notification() || editor::poll_readable(0, 0) {
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

thread_local! {
    // Worker threads have independent fixed-size stacks. A fork inherits the
    // current interpreter stack and its cached guard address.
    static MAIN_STACK_ORIGIN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static STACK_FLOOR: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn guarded_floor(low: usize, high: usize, limit: usize) -> usize {
    let usable = high.saturating_sub(low).min(limit);
    high - usable + (usable / 2).min(256 * 1024)
}

#[cold]
fn thread_stack_bounds() -> Option<(usize, usize)> {
    #[cfg(target_os = "macos")]
    let (low, size) = unsafe {
        let thread = libc::pthread_self();
        let size = libc::pthread_get_stacksize_np(thread);
        (
            libc::pthread_get_stackaddr_np(thread)
                .addr()
                .checked_sub(size)?,
            size,
        )
    };
    #[cfg(target_os = "linux")]
    let (low, size) = unsafe {
        let mut attr = std::mem::MaybeUninit::uninit();
        if libc::pthread_getattr_np(libc::pthread_self(), attr.as_mut_ptr()) != 0 {
            return None;
        }
        let mut attr = attr.assume_init();
        let mut low = std::ptr::null_mut();
        let mut size = 0;
        let result = libc::pthread_attr_getstack(&attr, &mut low, &mut size);
        libc::pthread_attr_destroy(&mut attr);
        if result != 0 {
            return None;
        }
        (low.addr(), size)
    };
    (size != 0).then_some((low, low.checked_add(size)?))
}

#[cfg(all(target_os = "linux", target_env = "musl"))]
fn main_stack_bounds() -> Option<(usize, usize)> {
    // musl's pthread_getattr_np reports only the currently mapped main
    // stack. Linux grows this mapping toward the preceding VMA.
    let origin = MAIN_STACK_ORIGIN.get();
    let maps = std::fs::read("/proc/self/maps").ok()?;
    let mut previous_end = 0usize;
    for line in maps.split(|&byte| byte == b'\n') {
        let range = std::str::from_utf8(line.split(|&byte| byte == b' ').next()?).ok()?;
        let (low, high) = range.split_once('-')?;
        let low = usize::from_str_radix(low, 16).ok()?;
        let high = usize::from_str_radix(high, 16).ok()?;
        if (low..high).contains(&origin) {
            let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
            let gap = usize::try_from(page).ok()?.saturating_mul(256);
            return Some((previous_end.saturating_add(gap).min(low), high));
        }
        previous_end = high;
    }
    None
}

#[cold]
fn update_stack_budget(soft: libc::rlim_t) {
    if MAIN_STACK_ORIGIN.get() == 0 {
        return;
    }
    #[cfg(all(target_os = "linux", target_env = "musl"))]
    let bounds = main_stack_bounds().or_else(thread_stack_bounds);
    #[cfg(not(all(target_os = "linux", target_env = "musl")))]
    let bounds = thread_stack_bounds();
    let limit = if soft == libc::RLIM_INFINITY {
        8 * 1024 * 1024
    } else {
        soft as usize
    };
    let floor = bounds.map_or(usize::MAX, |(low, high)| guarded_floor(low, high, limit));
    STACK_FLOOR.set(floor);
}

#[cold]
fn initial_stack_floor() -> usize {
    if MAIN_STACK_ORIGIN.get() == 0 {
        let floor = thread_stack_bounds().map_or(usize::MAX, |(low, high)| {
            guarded_floor(low, high, usize::MAX)
        });
        STACK_FLOOR.set(floor);
    } else {
        let mut limit = std::mem::MaybeUninit::uninit();
        let soft = if unsafe { libc::getrlimit(libc::RLIMIT_STACK, limit.as_mut_ptr()) } == 0 {
            unsafe { limit.assume_init() }.rlim_cur
        } else {
            libc::RLIM_INFINITY
        };
        update_stack_budget(soft);
    }
    STACK_FLOOR.get()
}

fn stack_budget_exceeded() -> bool {
    let marker = 0u8;
    stack_budget_exceeded_at(std::ptr::from_ref(&marker).addr())
}

#[inline]
fn stack_budget_exceeded_at(here: usize) -> bool {
    let floor = STACK_FLOOR.get();
    here <= if floor == 0 {
        initial_stack_floor()
    } else {
        floor
    }
}

#[cfg(test)]
#[test]
fn stack_limit_respects_the_allocation_and_soft_limit() {
    const MIB: usize = 1024 * 1024;
    let reserve = MIB / 4;
    assert_eq!(
        guarded_floor(15 * MIB, 16 * MIB, 8 * MIB),
        15 * MIB + reserve
    );
    assert_eq!(guarded_floor(8 * MIB, 16 * MIB, MIB), 15 * MIB + reserve);
    assert_eq!(
        guarded_floor(2 * MIB, 32 * MIB, 16 * MIB),
        16 * MIB + reserve
    );
}

#[cfg(test)]
#[test]
fn command_and_expansion_check_the_active_stack() {
    let mut session = Session::default();
    let statements = parser::session_source("{ value=ran; } 2>&1", true, &session).unwrap();
    let _guard = ScopedCell::enter(&STACK_FLOOR, usize::MAX);
    assert_eq!(
        execute_program(&mut session, statements, 0, false),
        Flow::Unwind(1)
    );
    assert!(parser::check_expansion_stack().is_err());
    assert!(session.get("value").is_none());
}

#[cfg(test)]
#[test]
fn recursion_guard_uses_the_worker_stack_bounds() {
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let marker = 0u8;
            let here = std::ptr::from_ref(&marker).addr();
            let (low, high) = thread_stack_bounds().unwrap();
            let floor = guarded_floor(low, high, usize::MAX);
            // An OS can reuse a larger cached thread stack. Test the actual
            // boundary, including both sides, rather than the requested size.
            assert!(floor > 0 && floor < here);
            assert!(!stack_budget_exceeded());
            assert!(stack_budget_exceeded_at(floor));
            assert!(!stack_budget_exceeded_at(floor + 1));
        })
        .unwrap()
        .join()
        .unwrap();
}

fn main() -> ExitCode {
    traps::enter_process();
    let marker = 0u8;
    MAIN_STACK_ORIGIN.set(std::ptr::from_ref(&marker).addr());
    invoke(std::env::args_os()).into()
}

fn invoke(args: impl Iterator<Item = std::ffi::OsString>) -> u8 {
    use cli::Action;
    let invocation = match cli::parse(args, shell_terminal) {
        Ok(Action::Run(invocation)) => invocation,
        Ok(Action::Version) => {
            return u8::from(
                writeln!(io::stdout().lock(), "tonari {}", env!("CARGO_PKG_VERSION")).is_err(),
            );
        }
        Ok(Action::Help) => {
            return u8::from(writeln!(io::stdout().lock(),
                "Usage: tonari [--noprofile] [--norc | --rcfile FILE] [-efilnsu] [-o OPTION] [-O SHOPT] [-c COMMAND [NAME [ARG...]] | -s [ARG...] | FILE [ARG...]]"
            ).is_err());
        }
        Err(e) => {
            diagnostic!("{e}");
            return 2;
        }
    };
    let mut session = Session::default();
    let code = run_invocation(&mut session, invocation);
    // Both callers exit the process after this invocation. The function table
    // owns parsed shell text, which the kernel reclaims in bulk. Coprocesses,
    // substitution endpoints and other session fields still drop normally.
    std::mem::forget(session.functions);
    code
}

fn apply_invocation_options(
    session: &mut Session,
    invocation: &mut cli::Invocation,
    env: &startup::Environment,
) -> Result<(), u8> {
    use cli::Input;
    session.initialize_shell_level(env);
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
    session.history_enabled = invocation.options.history.unwrap_or(invocation.interactive);
    session.histexpand = invocation.options.histexpand.unwrap_or(session.interactive);
    // The selected keymap is also observable in a forced interactive script.
    // Terminal capabilities are checked separately when opening the editor.
    session.vi =
        session.interactive && !invocation.noediting && invocation.options.vi.unwrap_or(false);
    session.emacs =
        session.interactive && !invocation.noediting && invocation.options.emacs.unwrap_or(true);
    if session.vi {
        session.bindings.map = editor::bindings::Keymap::Insert;
    }
    if session.interactive
        && let Err(e) = session.traps.init_hangup()
    {
        diagnostic!("SIGHUP: {e}");
        return Err(1);
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
    session.inherit_options(env);
    for (enable, name) in std::mem::take(&mut invocation.options.shopts) {
        let options = match name {
            Some(name) => vec![if enable { "-s" } else { "-u" }.into(), name],
            None if enable => Vec::new(),
            None => vec!["-p".into()],
        };
        if session.shopt(&options) != 0 {
            return Err(2);
        }
    }
    Ok(())
}

fn initialize_interactive_defaults(session: &mut Session) {
    if session.interactive {
        if session.get("PS1").is_none() {
            session.assign("PS1", "tonari$ ");
        }
        if session.get("PS2").is_none() {
            session.assign("PS2", "> ");
        }
        if session.get("HISTFILE").is_none()
            && let Some(home) = session.personal_home()
        {
            let file = if session.posix {
                ".sh_history"
            } else {
                ".bash_history"
            };
            session.assign(
                "HISTFILE",
                &std::path::Path::new(home.as_ref())
                    .join(file)
                    .to_string_lossy(),
            );
        }
    }
}

fn run_invocation(session: &mut Session, mut invocation: cli::Invocation) -> u8 {
    use cli::Input;
    session.started_at = unsafe { libc::time(std::ptr::null_mut()) };
    session.privileged = invocation.options.privileged;
    let changed_ids = match startup::settle_credentials(session.privileged) {
        Ok(changed) => changed,
        Err(e) => {
            diagnostic!("{e}");
            return 126;
        }
    };
    let env = startup::Environment::read(session, changed_ids);
    if session.privileged || changed_ids {
        session.unset_variable("CDPATH");
        session.unset_variable("GLOBIGNORE");
    }
    if let Err(code) = apply_invocation_options(session, &mut invocation, &env) {
        return code;
    }
    if invocation.pretty {
        return input::print_script(session, invocation.input);
    }
    initialize_interactive_defaults(session);
    if session.interactive && traps::listen_for_interrupts().is_err() {
        return 1;
    }
    let initial = match startup::initialize(session, &invocation, &env) {
        Flow::Unwind(status) | Flow::Discard(status) | Flow::Continue(status) => status,
        Flow::Return(status) | Flow::Exit(status) => {
            return traps::finish(session, status);
        }
        Flow::Break(..) | Flow::LoopContinue(_) => return traps::finish(session, 1),
    };
    if invocation.debugger
        && !invocation.options.check
        && (!session.interactive || !matches!(invocation.input, Input::Stdin))
        && let Flow::Exit(code) = startup::debugger(session)
    {
        return traps::finish(session, code);
    }
    if invocation.as_sh && !session.posix {
        session.set_posix(true);
    }
    let flow = match invocation.input {
        Input::Command(_) => {
            let text = session.execution_string.clone().unwrap();
            input::text(
                session,
                &text,
                initial,
                true,
                invocation.options.check,
                None,
            )
        }
        Input::File(path) => match startup::open_main_script(session, &path) {
            Ok((file, resolved)) => {
                let path = resolved.unwrap_or(path);
                session.push_source_path(&path);
                session.function_names.push("main".into());
                session.call_lines.push(0);
                input::file_script(session, file, None, initial, invocation.options.check)
            }
            Err(e) => {
                diagnostic!("{}: {}", path.display(), builtins::os_message(&e));
                Flow::Continue(if e.kind() == io::ErrorKind::NotFound {
                    127
                } else {
                    126
                })
            }
        },
        Input::Stdin if invocation.interactive => {
            match interactive(session, initial, !invocation.noediting) {
                Ok(code) => Flow::Continue(code),
                Err(e) => {
                    diagnostic!("{e}");
                    Flow::Continue(1)
                }
            }
        }
        Input::Stdin => input::stdin_script(session, invocation.options.check),
    };
    let code = match flow {
        Flow::Unwind(s)
        | Flow::Discard(s)
        | Flow::Continue(s)
        | Flow::Exit(s)
        | Flow::Return(s) => s,
        Flow::Break(..) | Flow::LoopContinue(_) => 1,
    };
    traps::finish_shell(
        session,
        code,
        invocation.interactive && session.stdin_script,
    )
}
