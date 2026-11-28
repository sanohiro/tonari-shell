use super::*;

#[derive(Clone, Copy)]
pub(super) enum Builtin {
    Status(u8),
    Exit(u8),
    InvalidExit,
    Control,
    NotLogin,
    History,
    ExtraExitArgs,
    Return(Option<u8>),
    Export,
    Unset,
    Directory,
    Source,
    Eval,
    Function,
    Compound,
    Exec,
    Declare,
    Set,
    Shopt,
    Hash,
    Shift,
    Getopts,
    DirectoryStack,
    Umask,
    Ulimit,
    Times,
    Mapfile,
    Let,
    LoopControl,
    AutoCd,
    Echo,
    Test,
    Read,
    Printf,
    Jobs,
    Describe,
    Alias,
    Trap,
    Completion,
    Unknown,
}

pub(crate) fn special_builtin(name: &str) -> bool {
    matches!(
        name,
        ":" | "."
            | "break"
            | "continue"
            | "eval"
            | "exec"
            | "exit"
            | "export"
            | "readonly"
            | "return"
            | "set"
            | "shift"
            | "times"
            | "trap"
            | "unset"
    )
}

fn posix_special(cmd: &PreparedCommand, session: &Session) -> bool {
    session.posix
        && cmd.search == lookup::Search::Normal
        && cmd.words.first().is_some_and(|w| special_builtin(w))
}

pub(super) fn builtin(
    cmd: &PreparedCommand,
    last_status: u8,
    session: &Session,
) -> Option<Builtin> {
    if cmd.compound.is_some() {
        return Some(Builtin::Compound);
    }
    builtin_words(
        &cmd.words,
        cmd.search,
        cmd.empty_status,
        last_status,
        session,
        || lookup::path(cmd, session),
    )
}

pub(super) fn builtin_words<'a>(
    words: &[String],
    search: lookup::Search,
    empty_status: u8,
    last_status: u8,
    session: &Session,
    path: impl FnOnce() -> &'a std::ffi::OsStr,
) -> Option<Builtin> {
    let Some(name) = words.first() else {
        return Some(Builtin::Status(empty_status));
    };
    if search == lookup::Search::Normal
        && session.functions.contains_key(name)
        && !(session.posix && (special_builtin(name) || name.contains('/')))
    {
        return Some(Builtin::Function);
    }
    if session.disabled_builtins.contains(name) {
        return (search == lookup::Search::BuiltinsOnly).then_some(Builtin::Unknown);
    }
    if name == "logout" && !session.login {
        return Some(Builtin::NotLogin);
    }
    named_builtin(name, &words[1..], last_status)
        .or_else(|| (search == lookup::Search::BuiltinsOnly).then_some(Builtin::Unknown))
        .or_else(|| {
            (session.shopt.autocd
                && session.interactive
                && search == lookup::Search::Normal
                && Path::new(name).is_dir()
                && (name.contains('/')
                    || !std::env::split_paths(&path()).any(|d| lookup::executable(&d.join(name)))))
            .then_some(Builtin::AutoCd)
        })
}

macro_rules! builtin_registry {
    ($args:ident, $last:ident; $( $name:literal => $kind:expr ),+ $(,)?) => {
        pub(crate) fn builtin_names() -> &'static [&'static str] { &[$($name),+] }
        pub(super) fn named_builtin(name: &str, $args: &[String], $last: u8) -> Option<Builtin> {
            Some(match name { $($name => $kind,)+ _ => return None })
        }
    };
}

builtin_registry! { args, last_status;
    ":" => Builtin::Status(0),
    "[" => Builtin::Test,
    "." => Builtin::Source,
    "break" => Builtin::LoopControl,
    "bind" => Builtin::Control,
    "caller" => Builtin::Control,
    "enable" => Builtin::Control,
    "help" => Builtin::Control,
    "suspend" => Builtin::Control,
    "builtin" => Builtin::Status(0),
    "cd" => Builtin::Directory,
    "command" => Builtin::Describe,
    "compgen" => Builtin::Completion,
    "complete" => Builtin::Completion,
    "compopt" => Builtin::Completion,
    "continue" => Builtin::LoopControl,
    "echo" => Builtin::Echo,
    "eval" => Builtin::Eval,
    "exit" => exit_builtin(args, last_status),
    "logout" => exit_builtin(args, last_status),
    "history" => Builtin::History,
    "fc" => Builtin::History,
    "exec" => Builtin::Exec,
    "export" => Builtin::Export,
    "false" => Builtin::Status(1),
    "local" => Builtin::Declare,
    "printf" => Builtin::Printf,
    "pwd" => Builtin::Directory,
    "read" => Builtin::Read,
    "return" => Builtin::Return(status_value(status_operands(args), last_status)),
    "set" => Builtin::Set,
    "shift" => Builtin::Shift,
    "source" => Builtin::Source,
    "test" => Builtin::Test,
    "true" => Builtin::Status(0),
    "type" => Builtin::Describe,
    "unset" => Builtin::Unset,
    "jobs" => Builtin::Jobs,
    "disown" => Builtin::Jobs,
    "fg" => Builtin::Jobs,
    "bg" => Builtin::Jobs,
    "kill" => Builtin::Jobs,
    "wait" => Builtin::Jobs,
    "shopt" => Builtin::Shopt,
    "declare" => Builtin::Declare,
    "readonly" => Builtin::Declare,
    "typeset" => Builtin::Declare,
    "alias" => Builtin::Alias,
    "unalias" => Builtin::Alias,
    "trap" => Builtin::Trap,
    "getopts" => Builtin::Getopts,
    "let" => Builtin::Let,
    "hash" => Builtin::Hash,
    "umask" => Builtin::Umask,
    "ulimit" => Builtin::Ulimit,
    "times" => Builtin::Times,
    "dirs" => Builtin::DirectoryStack,
    "pushd" => Builtin::DirectoryStack,
    "popd" => Builtin::DirectoryStack,
    "mapfile" => Builtin::Mapfile,
    "readarray" => Builtin::Mapfile,
}

fn status_operands(args: &[String]) -> &[String] {
    if args.first().is_some_and(|s| s == "--") {
        &args[1..]
    } else {
        args
    }
}

fn status_value(args: &[String], previous: u8) -> Option<u8> {
    args.first().map_or(Some(previous), |arg| {
        arg.trim().parse::<i64>().ok().map(|n| n as u8)
    })
}

fn exit_builtin(args: &[String], last_status: u8) -> Builtin {
    let args = status_operands(args);
    let Some(status) = status_value(args, last_status) else {
        return Builtin::InvalidExit;
    };
    if args.len() > 1 {
        Builtin::ExtraExitArgs
    } else {
        Builtin::Exit(status)
    }
}

pub(super) fn eval_builtin(
    builtin: Builtin,
    cmd: &mut PreparedCommand,
    session: &mut Session,
    last_status: u8,
    in_stage_child: bool,
) -> Flow {
    let persistent = !cmd.assignments.is_empty() && posix_special(cmd, session);
    let mut bindings = if persistent {
        session.persist_prefix(
            &mut cmd.assignments,
            matches!(builtin, Builtin::Eval | Builtin::Source),
        );
        Default::default()
    } else {
        session.bind(&cmd.assignments)
    };
    let args = cmd.words.get(1..).unwrap_or_default();
    let result = match builtin {
        Builtin::History => crate::history::commands::run(session, &cmd.words, last_status),
        Builtin::Control => Flow::Continue(match cmd.words[0].as_str() {
            "bind" => crate::editor::bindings::builtin(session, args),
            "caller" => crate::builtins::control::caller(session, args),
            "enable" => crate::builtins::control::enable(session, args),
            "help" => crate::builtins::control::help(args),
            _ => crate::builtins::control::suspend(session, args),
        }),
        Builtin::NotLogin => {
            write_error(b"logout", b": not login shell: use `exit'");
            Flow::Continue(1)
        }
        Builtin::Compound => crate::eval::compound(
            session,
            cmd.compound.take().unwrap(),
            last_status,
            in_stage_child,
        ),
        Builtin::Exec => exec_builtin(cmd, session, args),
        Builtin::Function => crate::eval::call_with_bindings(
            session,
            std::mem::take(&mut cmd.words),
            last_status,
            false,
            &mut bindings,
        ),
        Builtin::Trap => Flow::Continue(session.traps.builtin(
            args,
            session.interactive,
            session.posix,
        )),
        Builtin::Alias => Flow::Continue(crate::alias::builtin(&cmd.words[0], args, session)),
        Builtin::Completion => Flow::Continue(crate::completion_spec::run(
            &cmd.words[0],
            args,
            last_status,
            session,
        )),
        Builtin::Describe => Flow::Continue(lookup::describe(cmd, session)),
        Builtin::Unknown => {
            write_error(cmd.words[0].as_bytes(), b": not a shell builtin");
            Flow::Continue(1)
        }
        Builtin::Printf => Flow::Continue(session.printf(args)),
        Builtin::Jobs => crate::jobs::dispatch(&cmd.words, session, last_status),
        Builtin::Times => Flow::Continue(crate::builtins::times(args)),
        Builtin::Ulimit => Flow::Continue(crate::builtins::limits::run(
            args,
            &mut crate::builtins::stdout_for("ulimit"),
            session.posix,
        )),
        Builtin::Umask => Flow::Continue(crate::builtins::umask::run(args, session.posix)),
        Builtin::Mapfile | Builtin::Read => {
            let here = match cmd.redirects.as_slice() {
                [r] if r.fd == 0
                    && matches!(r.target, Target::HereDoc { .. } | Target::HereString) =>
                {
                    Some((r.fd, r.word.as_bytes()))
                }
                _ => None,
            };
            if matches!(builtin, Builtin::Read) {
                session.read(args, here, last_status)
            } else {
                crate::session::mapfile::run(session, &cmd.words[0], args, last_status, here)
            }
        }
        Builtin::Set => Flow::Continue(session.set(&mut cmd.words)),
        Builtin::Hash => Flow::Continue(hash::run(session, args)),
        Builtin::Shopt => Flow::Continue(session.shopt(args)),
        Builtin::Getopts => session.getopts(args),
        Builtin::Let => crate::arithmetic::let_builtin(args, session),
        Builtin::Shift => Flow::Continue(session.shift(args)),
        Builtin::Declare => session.declare(
            &cmd.words[0],
            args,
            &cmd.array_declarations,
            last_status,
            &mut bindings,
        ),
        Builtin::Echo => Flow::Continue(crate::builtins::echo(args, session)),
        Builtin::Test => Flow::Continue(crate::builtins::test(&cmd.words, session)),
        Builtin::LoopControl => {
            if session.loop_depth == 0 && session.parent_loop_in_subshell {
                write_error(
                    cmd.words[0].as_bytes(),
                    b": loop control cannot cross a subshell boundary",
                );
                Flow::Exit(1)
            } else if session.loop_depth == 0 {
                if !session.posix {
                    write_error(
                        cmd.words[0].as_bytes(),
                        b": only meaningful in a `for', `while', or `until' loop",
                    );
                }
                Flow::Continue(0)
            } else if args.len() > 1 {
                write_error(cmd.words[0].as_bytes(), b": too many arguments");
                // Bash abandons the current input on a usage error. File and
                // stdin readers can then resume at the next complete command.
                if session.command_string && !session.interactive {
                    Flow::Exit(1)
                } else {
                    Flow::Discard(1)
                }
            } else {
                let depth = args.first().map_or(Ok(1), |n| n.trim().parse::<i64>());
                match depth {
                    Ok(n) if n > 0 => {
                        let levels = (n as u64).min(u64::from(session.loop_depth)) as u32;
                        if cmd.words[0] == "break" {
                            Flow::Break(levels, 0)
                        } else {
                            Flow::LoopContinue(levels)
                        }
                    }
                    Err(_) => {
                        diagnostic!("{}: {}: numeric argument required", cmd.words[0], args[0]);
                        Flow::Exit(2)
                    }
                    Ok(_) => {
                        diagnostic!("{}: {}: loop count out of range", cmd.words[0], args[0]);
                        Flow::Break(session.loop_depth, 1)
                    }
                }
            }
        }
        Builtin::Export => {
            if args
                .iter()
                .take_while(|s| crate::builtins::flags::cluster(s))
                .any(|s| s.contains(['a', 'A']))
            {
                session.declare(
                    "export",
                    args,
                    &cmd.array_declarations,
                    last_status,
                    &mut bindings,
                )
            } else {
                let status = session.export(args, &mut bindings);
                Flow::Continue(apply_array_declarations(cmd, status, session))
            }
        }
        Builtin::Unset => session.unset(args),
        Builtin::DirectoryStack => Flow::Continue(session.directory_stack(
            &cmd.words[0],
            args,
            &mut crate::builtins::listing_stdout(&cmd.words[0]),
        )),
        Builtin::AutoCd => {
            stderr_line!("cd -- {}", cmd.words.join(" "));
            cmd.words.insert(0, "--".into());
            let status = session.directory_builtin("cd", &cmd.words);
            cmd.words.remove(0);
            Flow::Continue(status)
        }
        Builtin::Directory => Flow::Continue(session.directory_builtin(&cmd.words[0], args)),
        Builtin::Source => crate::source_file(
            session,
            &cmd.words,
            last_status,
            !session.interactive && posix_special(cmd, session),
        ),
        Builtin::Eval if !cmd.assignments.is_empty() => {
            session.enter_scope(&mut bindings, true);
            let flow = crate::input::eval_builtin(session, args, last_status);
            session.leave_scope();
            flow
        }
        Builtin::Eval => crate::input::eval_builtin(session, args, last_status),
        Builtin::Return(status) => {
            if status.is_some() && status_operands(args).len() > 1 {
                write_error(b"return", b": too many arguments");
                Flow::Continue(1)
            } else if session.source_depth == 0 && session.function_depth == 0 {
                write_error(
                    b"return",
                    b": can only return from a function or sourced script",
                );
                Flow::Continue(2)
            } else if let Some(code) = status {
                session.traps.before_return = last_status;
                let code = if status_operands(args).is_empty() {
                    session
                        .traps
                        .implicit_status(session.function_depth + session.source_depth, false)
                        .unwrap_or(code)
                } else {
                    code
                };
                Flow::Return(code)
            } else {
                diagnostic!(
                    "return: {}: numeric argument required",
                    status_operands(args)[0]
                );
                Flow::Return(2)
            }
        }
        Builtin::Status(code) => Flow::Continue(code),
        Builtin::Exit(_) if crate::startup::defer_interactive_exit(session) => Flow::Continue(1),
        Builtin::Exit(code) if status_operands(args).is_empty() => {
            Flow::Exit(crate::startup::logout(
                session,
                session
                    .traps
                    .implicit_status(session.function_depth + session.source_depth, true)
                    .unwrap_or(code),
                last_status,
            ))
        }
        Builtin::Exit(code) => Flow::Exit(crate::startup::logout(session, code, last_status)),
        Builtin::InvalidExit => {
            diagnostic!(
                "exit: {}: numeric argument required",
                status_operands(args)[0]
            );
            Flow::Continue(2)
        }
        Builtin::ExtraExitArgs => {
            write_error(b"exit", b": too many arguments");
            Flow::Continue(1)
        }
    };
    // Bash exits a function in its dynamic execution environment, including
    // the function's redirections and positional parameters.
    let result = if session.function_depth != 0 || session.source_depth != 0 {
        crate::traps::leaving_scope(session, result)
    } else {
        result
    };
    if !persistent {
        session.restore(bindings);
    }
    if let Flow::Continue(status @ 1..) = result
        && !session.interactive
        && posix_special(cmd, session)
        && (matches!(builtin, Builtin::Export | Builtin::Declare | Builtin::Unset)
            || matches!(builtin, Builtin::Trap) && status == 2)
    {
        return Flow::Exit(status);
    }
    result
}

#[inline(never)]
fn exec_builtin(cmd: &PreparedCommand, session: &Session, mut args: &[String]) -> Flow {
    let mut clear = false;
    let mut login = false;
    let mut arg0 = None;
    while let Some(opt) = crate::builtins::flags::next(&mut args) {
        let mut flags = opt[1..].chars();
        while let Some(flag) = flags.next() {
            match flag {
                'c' => clear = true,
                'l' => login = true,
                'a' => {
                    let Some(v) = crate::builtins::flags::value(flags.as_str(), &mut args) else {
                        diagnostic!("exec: -a requires a name");
                        return Flow::Continue(2);
                    };
                    arg0 = Some(v.to_owned());
                    break;
                }
                _ => {
                    diagnostic!("exec: invalid option {opt}");
                    return Flow::Continue(2);
                }
            }
        }
    }
    let Some(program) = args.first() else {
        return Flow::Continue(0);
    };
    if login {
        arg0 = Some(format!("-{}", arg0.as_ref().unwrap_or(program)));
    }
    let status = match External::for_words(cmd, session, args, arg0, clear) {
        Ok(external) => external.exec(session, false),
        Err(error) => {
            diagnostic!("exec: {error}");
            126
        }
    };
    if session.subshell_depth == 0 && (session.interactive || session.shopt.execfail) {
        Flow::Continue(status)
    } else {
        Flow::Exit(status)
    }
}

fn apply_array_declarations(cmd: &mut PreparedCommand, status: u8, session: &mut Session) -> u8 {
    if status != 0 || cmd.array_declarations.is_empty() {
        return status;
    }
    let global = cmd
        .words
        .iter()
        .skip(1)
        .take_while(|a| crate::builtins::flags::cluster(a))
        .any(|a| a.contains('g'));
    for d in cmd.array_declarations.drain(..) {
        if let Err(e) = session.assign_array(&d.name, d.append, d.items, global) {
            diagnostic!("{}: {e}", cmd.words[0]);
            return 1;
        }
    }
    0
}

// Only these forms produce bytes without changing shell variables, reading a
// descriptor or issuing diagnostics. Use the usual echo and printf renderers.
pub(super) enum PlainOutput<'a> {
    Echo(&'a [String]),
    Printf(&'a [String]),
    DirectoryStack(&'a str, &'a [String]),
    Pwd(Option<bool>),
    Limits(&'a [String]),
}

impl<'a> PlainOutput<'a> {
    pub(super) fn new(builtin: Builtin, cmd: &'a PreparedCommand, limit: usize) -> Option<Self> {
        if !cmd.assignments.is_empty() {
            return None;
        }
        let words = cmd.words.get(1..)?;
        match builtin {
            Builtin::Echo => {
                let bound = words
                    .iter()
                    .try_fold(1usize, |n, s| n.checked_add(s.len() + 1))?;
                (bound <= limit).then_some(Self::Echo(words))
            }
            Builtin::Printf => {
                let options_end = words.first().is_some_and(|s| s == "--");
                let (format, values) = words[usize::from(options_end)..].split_first()?;
                if !options_end && format.starts_with('-') && format != "-" {
                    return None;
                }
                let bound = format
                    .len()
                    .saturating_mul(values.len().max(1))
                    .saturating_add(values.iter().map(String::len).sum::<usize>());
                if bound > limit {
                    return None;
                }
                let mut rest = format.as_bytes();
                while let Some(i) = memchr::memchr(b'%', rest) {
                    if !matches!(rest.get(i + 1), Some(b'%' | b's'))
                        || i > 0 && rest[i - 1] == b'\\'
                    {
                        return None;
                    }
                    rest = &rest[i + 2..];
                }
                Some(Self::Printf(words))
            }
            _ => None,
        }
    }

    fn for_redirect(builtin: Builtin, cmd: &'a PreparedCommand) -> Option<Self> {
        if !cmd.assignments.is_empty() {
            return None;
        }
        let args = cmd.words.get(1..)?;
        match builtin {
            Builtin::DirectoryStack
                if cmd.words[0] == "dirs" || args.first().is_some_and(|s| s == "-n") =>
            {
                Some(Self::DirectoryStack(&cmd.words[0], args))
            }
            Builtin::Ulimit
                if args.iter().all(|arg| {
                    arg.strip_prefix('-')
                        .is_some_and(|flags| flags.bytes().all(|b| b.is_ascii_alphabetic()))
                }) =>
            {
                Some(Self::Limits(args))
            }
            Builtin::Directory if cmd.words[0] == "pwd" => match args {
                [] => Some(Self::Pwd(None)),
                [opt] if opt == "-L" || opt == "-P" => Some(Self::Pwd(Some(opt == "-P"))),
                _ => None,
            },
            _ => Self::new(builtin, cmd, usize::MAX),
        }
    }

    pub(super) fn render(self, session: &mut Session) -> (Vec<u8>, u8) {
        let mut bytes = Vec::new();
        let status = match self {
            Self::Echo(words) => {
                crate::builtins::echo_into(words, &mut bytes, session);
                0
            }
            Self::Printf(words) => crate::session::printf::run(session, words, Some(&mut bytes)),
            Self::Pwd(physical) => {
                match session.write_pwd(physical.unwrap_or(session.physical), &mut bytes) {
                    Ok(()) => 0,
                    Err(e) => {
                        diagnostic!("pwd: {e}");
                        1
                    }
                }
            }
            Self::DirectoryStack(command, args) => {
                session.directory_stack(command, args, &mut bytes)
            }
            Self::Limits(args) => crate::builtins::limits::run(args, &mut bytes, session.posix),
        };
        (bytes, status)
    }
}

// A direct read knows the first record boundary from its redirect. Shell
// functions, groups and persistent descriptors can have several consumers:
// keep medium inputs seekable so buffered reads can return any unused bytes.
pub(super) fn here_pipe_limit(builtin: Option<Builtin>, cmd: &PreparedCommand) -> usize {
    if builtin.is_none() || matches!(builtin, Some(Builtin::Read)) && cmd.redirects.len() == 1 {
        here_input::PIPE_LIMIT
    } else {
        libc::PIPE_BUF
    }
}

pub(super) fn redirect_failure(cmd: &PreparedCommand, session: &Session) -> Flow {
    if !session.interactive && posix_special(cmd, session) {
        Flow::Exit(1)
    } else {
        Flow::Continue(1)
    }
}

pub(super) fn can_install_trap(builtin: Builtin, cmd: &PreparedCommand) -> bool {
    matches!(
        builtin,
        Builtin::Trap | Builtin::Function | Builtin::Eval | Builtin::Source
    ) || matches!(builtin, Builtin::Compound)
        && !matches!(
            cmd.compound.as_deref(),
            Some(crate::parser::Command::Subshell(_))
        )
}

fn silent_redirect(
    code: u8,
    dest: RawFd,
    path: &str,
    flags: i32,
    cmd: &PreparedCommand,
    session: &mut Session,
) -> io::Result<Flow> {
    // A silent builtin needs the open's effects (including a blocking FIFO),
    // but never reads or writes the destination. Open before reserving any
    // backup, so /dev/fd paths see the caller's descriptor table.
    let Some(fd) = open_redirect(path, flags) else {
        return Ok(redirect_failure(cmd, session));
    };
    // Bash can still fail to save a live destination at its descriptor limit.
    // This reservation drops before the single-threaded shell can exec.
    if fd.as_raw_fd() != dest {
        let backup =
            unsafe { libc::fcntl(dest, libc::F_DUPFD, crate::execute::fd::FIRST_NONSTANDARD) };
        if backup >= 0 {
            unsafe { libc::close(backup) };
        } else if errno() == libc::EBADF {
            if let Err(error) = duplicate(fd.as_raw_fd(), dest) {
                diagnostic!(
                    "redirection: {dest}: {}",
                    crate::builtins::os_message(&error)
                );
                return Ok(redirect_failure(cmd, session));
            }
            unsafe { libc::close(dest) };
        } else {
            diagnostic!(
                "redirection: {dest}: {}",
                crate::builtins::os_message(&io::Error::last_os_error())
            );
            return Ok(redirect_failure(cmd, session));
        }
    }
    Ok(Flow::Continue(code))
}

pub(super) fn output_status(name: &str, result: io::Result<()>, status: u8) -> u8 {
    if let Err(error) = result {
        if error.kind() == io::ErrorKind::BrokenPipe && crate::traps::terminating_pipe() {
            return 1;
        }
        diagnostic!(
            "{name}: write error: {}",
            crate::builtins::os_message(&error)
        );
        1
    } else {
        status
    }
}

pub(super) fn run_builtin(
    builtin: Builtin,
    cmd: &mut PreparedCommand,
    session: &mut Session,
    last_status: u8,
    final_command: bool,
) -> io::Result<Flow> {
    // The substitution child can become the function's last external command.
    // Keep the normal function scope and let each statement recheck active traps.
    if final_command
        && matches!(builtin, Builtin::Function)
        && session.subshell_depth > 0
        && cmd.redirects.is_empty()
        && cmd.assignments.is_empty()
        && session.substitutions.scope() == 0
    {
        return Ok(crate::call_function(
            session,
            std::mem::take(&mut cmd.words),
            last_status,
            true,
        ));
    }
    if cmd.redirects.is_empty() {
        return Ok(eval_builtin(builtin, cmd, session, last_status, false));
    }
    run_redirected_builtin(builtin, cmd, session, last_status, final_command)
}

// Preserve descriptor restoration while the body runs. Keep this frame off
// the call stack for ordinary functions and eval without redirections.
#[inline(never)]
fn run_redirected_builtin(
    builtin: Builtin,
    cmd: &mut PreparedCommand,
    session: &mut Session,
    last_status: u8,
    final_command: bool,
) -> io::Result<Flow> {
    let one;
    let pair;
    let multiple;
    let actions: &[Action] = match cmd.redirects.as_slice() {
        [redirect] => {
            one = [action(redirect)];
            &one
        }
        [first, second] => {
            // A descriptor move expands to a duplicate followed by a close.
            pair = [action(first), action(second)];
            &pair
        }
        _ => {
            multiple = actions(cmd);
            &multiple
        }
    };
    protect_fds(actions)?;
    // A short here input normally needs an empty pipe. Keep that resource
    // check, including EMFILE. An inert reader can consume the bytes without
    // writing the pipe or replacing stdin. Callbacks and alternate fds use the
    // normal installation, as do closed standard streams.
    if matches!(builtin, Builtin::Read | Builtin::Mapfile)
        && let [Action::HereInput(0, bytes)] = actions
        && !bytes.is_empty()
        && bytes.len() <= here_input::PIPE_LIMIT
        && !session.traps.observes_commands()
        && (cmd.assignments.is_empty()
            || matches!(cmd.assignments.as_slice(), [(key, _)] if key == "IFS" && !session.is_nameref(key) && !session.integer(key)))
        && let Some(input) =
            session.memory_input(matches!(builtin, Builtin::Read), &cmd.words[1..], bytes)
        && let Ok((reader, writer)) = io::pipe()
        && reader.as_raw_fd() > 2
        && writer.as_raw_fd() > 2
    {
        drop((reader, writer));
        let bindings = session.bind(&cmd.assignments);
        let result = input.run(session, &cmd.words[0], last_status);
        session.restore(bindings);
        return Ok(result);
    }
    if !final_command
        && let [Action::Open(1, path, flags)] = actions
        && let Some(output) = PlainOutput::for_redirect(builtin, cmd)
    {
        // Reserve the same descriptor as the ordinary redirect, including
        // failure at RLIMIT_NOFILE. A byte-only builtin can write to the file
        // directly, avoiding two changes to the process-wide stdout.
        let reservation = SavedFd::reserve(&actions[0], actions)?;
        let Some(fd) = open_redirect(path, *flags) else {
            return Ok(redirect_failure(cmd, session));
        };
        // No shell code ran while the untracked reservation was live.
        drop(reservation);
        use std::io::Write;
        let mut file = std::fs::File::from(fd);
        if let PlainOutput::Pwd(physical) = output
            && !physical.unwrap_or(session.physical)
            && !session.posix
        {
            let path = match session.logical_directory_ref() {
                Ok(path) => path.as_os_str().as_bytes(),
                Err(e) => {
                    diagnostic!("pwd: {e}");
                    return Ok(Flow::Continue(1));
                }
            };
            let mut line = [0u8; 512];
            if path.len() < line.len() {
                line[..path.len()].copy_from_slice(path);
                line[path.len()] = b'\n';
                return Ok(Flow::Continue(output_status(
                    "pwd",
                    file.write_all(&line[..=path.len()]),
                    0,
                )));
            }
        }
        let (data, status) = output.render(session);
        let result = file.write_all(&data);
        return Ok(Flow::Continue(output_status(&cmd.words[0], result, status)));
    }
    if cmd.assignments.is_empty()
        && let (Builtin::Status(code), [Action::Open(dest, path, flags)]) = (builtin, actions)
    {
        return silent_redirect(code, *dest, path, *flags, cmd, session);
    }
    let mut saved = [const { SavedFd::Unchanged }; 3];
    let mut extra = Vec::new();
    if !final_command || can_install_trap(builtin, cmd) || session.traps.catches(0) {
        for action in actions {
            let fd = action.destination();
            if let Some(slot) = saved.get_mut(fd as usize) {
                if matches!(slot, SavedFd::Unchanged) {
                    *slot = SavedFd::capture(action, actions)?;
                }
            } else if !extra.iter().any(|(n, _)| *n == fd) {
                extra.push((fd, SavedFd::capture(action, actions)?));
            }
        }
    }
    let redirection = apply_protected(actions, here_pipe_limit(Some(builtin), cmd), session);
    let applied = redirection.is_ok();
    let old_input = session.stdin_redirected;
    if applied && actions.iter().any(|a| a.destination() == 0) {
        session.stdin_redirected = true;
    }
    let flow = if applied {
        eval_builtin(builtin, cmd, session, last_status, false)
    } else {
        redirect_failure(cmd, session)
    };
    if !matches!(builtin, Builtin::Exec) || !matches!(flow, Flow::Continue(0)) {
        session.stdin_redirected = old_input;
    }
    if applied && matches!(builtin, Builtin::Exec) && matches!(flow, Flow::Continue(0)) {
        for r in &cmd.redirects {
            if r.target == Target::Duplicate && r.word.parse::<RawFd>() == Ok(r.fd) {
                continue;
            }
            fd_replaced(session, r.fd);
        }
        return Ok(flow);
    }
    if let Err(error) = SavedFd::restore_all(
        saved
            .into_iter()
            .enumerate()
            .map(|(fd, v)| (fd as RawFd, v))
            .chain(extra),
    ) {
        diagnostic!("redirection: cannot restore file descriptor: {error}");
        if matches!(flow, Flow::Continue(_)) {
            return Ok(Flow::Continue(1));
        }
    }
    // Failures in a compound command's body are tested by its own statement
    // list. A redirect failure happens before that list can run.
    if let Err(index) = redirection
        && matches!(builtin, Builtin::Compound)
    {
        return Ok(compound_redirect_failure(
            session,
            1,
            cmd.redirects[index].line,
        ));
    }
    Ok(flow)
}

pub(super) fn compound_redirect_failure(session: &mut Session, status: u8, line: usize) -> Flow {
    if session.tested_depth == 0 {
        if session.traps.active() {
            if let Some(location) = &mut session.current_command {
                location.line = line;
            }
            let trap = crate::traps::error(session, status);
            if !matches!(trap, Flow::Continue(_)) {
                return trap;
            }
        }
        if session.errexit {
            return Flow::Exit(status);
        }
    }
    Flow::Continue(status)
}
