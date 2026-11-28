mod helper;

use std::io::{self, Read};
use std::os::fd::{AsFd, AsRawFd, IntoRawFd};

use crate::{Flow, Session, execute_program, parser};

// A zero size on a virtual filesystem can describe a live stream. Keep its
// open and read in the child. Unknown filesystems take the same path.
fn regular_storage(query: impl FnOnce(&mut libc::statfs) -> libc::c_int) -> bool {
    let mut info = unsafe { std::mem::zeroed() };
    if query(&mut info) != 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    return matches!(
        info.f_type as _,
        libc::EXT4_SUPER_MAGIC
            | libc::XFS_SUPER_MAGIC
            | libc::BTRFS_SUPER_MAGIC
            | libc::TMPFS_MAGIC
            | libc::OVERLAYFS_SUPER_MAGIC
            | libc::F2FS_SUPER_MAGIC
    );
    #[cfg(target_os = "macos")]
    {
        let name = info.f_fstypename.map(|c| c as u8);
        name.starts_with(b"apfs\0") || name.starts_with(b"hfs\0")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    false
}

fn read_regular_file(file: &std::fs::File, size: u64, output: &mut Vec<u8>) -> io::Result<()> {
    // One extra byte distinguishes EOF from a stale size without growing a full buffer.
    const MAX_INITIAL_SIZE: usize = 64 * 1024 * 1024;
    output.try_reserve_exact(size.min(MAX_INITIAL_SIZE as u64) as usize + 1)?;
    loop {
        output.try_reserve(1)?;
        if crate::input::read_spare(file, output, MAX_INITIAL_SIZE + 1)? == 0 {
            return Ok(());
        }
    }
}

fn capture_arguments(
    words: &[parser::Word],
    session: &mut Session,
    last: u8,
) -> Option<Vec<String>> {
    if !words.iter().all(|w| {
        w.as_literal().is_some() || !session.nounset && (inert_operand(w, session, false) ||
            matches!(w, parser::Word::Expanded(parts) if matches!(parts.as_slice(),
                [crate::word::Part::Parameter {name, quoted: true}]
                if name.strip_suffix("[@]").is_some_and(|base| inert_parameter(base, session) && session.stored_array(base)))))
    }) {
        return None;
    }
    let captured = std::cell::Cell::new(None);
    let mut env = crate::expansion::Expansion::new(session, last, &captured);
    let mut args = Vec::with_capacity(words.len());
    for word in words {
        if let Some(text) = word.as_literal() {
            args.push(text.into());
        } else {
            env.append_fields(word.clone(), &mut args).ok()?;
        }
    }
    Some(args)
}

pub(crate) fn current_substitution(
    session: &mut Session,
    body: &[parser::Statement],
    last: u8,
    reply: bool,
) -> Result<(String, u8), crate::expansion::Error> {
    if body.is_empty() {
        return Ok((String::new(), 0));
    }
    // A single output builtin with inert operands can write to memory directly. Everything
    // that can redirect stdout or observe traps still executes in a real fd scope.
    if !reply
        && !session.traps.observes_commands()
        && !session.xtrace
        && let Some(cmd) = sole_plain_command(body)
        && (!session.keyword || cmd.keyword_args.is_empty())
        && let Some(name @ ("echo" | "printf")) =
            cmd.words.first().and_then(parser::Word::as_literal)
        && !session.functions.contains_key(name)
        && !session.disabled_builtins.contains(name)
        && !session.noexec
        && (name == "echo"
            || cmd
                .words
                .get(1)
                .and_then(parser::Word::as_literal)
                .is_some_and(|format| {
                    !format.starts_with('-') && !crate::session::printf::may_assign_count(format)
                }))
        && let Some(words) = capture_arguments(&cmd.words, session, last)
    {
        let mut output = Vec::new();
        let code = if name == "echo" {
            crate::builtins::echo_into(&words[1..], &mut output, session);
            0
        } else {
            crate::session::printf::run(session, &words[1..], Some(&mut output))
        };
        session.remember_literal_argument(&words);
        if code != 0
            && session.shopt.inherit_errexit
            && session.errexit
            && session.tested_depth == 0
        {
            return Err(crate::expansion::Error::control(Flow::Exit(code)));
        }
        return Ok(captured_output(output, code)?);
    }
    let callsite = session.current_command.clone();
    let pipeline = session.pipe_status.clone();
    let restore_errexit =
        (!session.shopt.inherit_errexit).then(|| std::mem::take(&mut session.errexit));
    let verbose = std::mem::take(&mut session.verbose);
    session.enter_function(&mut Default::default());
    session.substitution_scopes += 1;
    if reply {
        session.local_reply(last);
    }
    let mut run = || crate::eval::borrowed_program(session, body, last, false);
    let captured = if reply {
        let flow = run();
        Ok((flow, Vec::new()))
    } else {
        crate::execute::capture_stdout(run)
    };
    let value = reply.then(|| session.get("REPLY").unwrap_or_default());
    session.substitution_scopes -= 1;
    session.leave_function();
    session.verbose = verbose;
    if let Some(saved) = restore_errexit {
        session.errexit = saved;
    }
    session.pipe_status = pipeline;
    session.current_command = callsite;
    let (flow, output) = captured.map_err(|_| "cannot capture current-shell output")?;
    let status = match flow {
        Flow::Continue(n) | Flow::Return(n) => n,
        other => return Err(crate::expansion::Error::control(other)),
    };
    match value {
        Some(value) => Ok((value, status)),
        None => Ok(captured_output(output, status)?),
    }
}

pub fn capture_substitution(
    session: &mut Session,
    body: &[parser::Statement],
    last: u8,
) -> Result<(String, u8), &'static str> {
    #[cfg(target_os = "linux")]
    let mut child_input = None;
    // A literal read of an ordinary file has no substitutions to isolate.
    // Keep streams, virtual files, error reporting and trap execution in the child.
    if !session.traps.observes_commands()
        && !session.xtrace
        && let Some(path) = input_only(body)
            .and_then(|word| stored_path(word, session, session.posix && !session.interactive))
        && let Ok(before) = std::fs::metadata(path)
        && before.is_file()
    {
        use std::os::unix::fs::OpenOptionsExt;
        if before.len() == 0
            && !std::ffi::CString::new(path)
                .is_ok_and(|p| regular_storage(|info| unsafe { libc::statfs(p.as_ptr(), info) }))
        {
            // Resolve process-relative names such as /proc/self/stat in the
            // caller. O_PATH pins the object without opening a stream for I/O.
            #[cfg(target_os = "linux")]
            if let Ok(file) = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_PATH)
                .open(path)
            {
                child_input = crate::execute::private_fd(file.into()).ok();
            }
        } else if let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            && let Ok(metadata) = file.metadata()
            && metadata.is_file()
            && (metadata.len() > 0
                || regular_storage(|info| unsafe { libc::fstatfs(file.as_raw_fd(), info) }))
        {
            let mut output = Vec::new();
            if let Err(e) = read_regular_file(&file, metadata.len(), &mut output) {
                diagnostic!("{e}");
                return captured_output(output, 1);
            }
            return captured_output(output, 0);
        }
    }
    if let Some(result) = capture_simple(session, body, last) {
        return result;
    }
    session.jobs.refresh_snapshot();
    let (read, write) = std::io::pipe().map_err(|_| "cannot open command substitution pipe")?;
    let pid = session
        .fork_shell()
        .map_err(|_| "cannot fork command substitution")?;
    if pid == 0 {
        drop(read);
        if write.as_raw_fd() == 1 {
            let fd = write.into_raw_fd();
            if unsafe { libc::fcntl(fd, libc::F_SETFD, 0) } < 0 {
                unsafe { libc::_exit(1) }
            }
        } else {
            if unsafe { libc::dup2(write.as_raw_fd(), 1) } < 0 {
                unsafe { libc::_exit(1) }
            }
            drop(write);
        }
        // fork already isolated the complete session, including shell variables.
        session.enter_subshell();
        session.substitution_shell = true;
        session.trace_depth += 1;
        crate::execute::coprocess::after_fork(session);
        session.jobs.forked(true);
        session.interactive = false;
        session.command_string = false;
        session.errexit &= session.shopt.inherit_errexit;
        if let Some(path) = input_only(body) {
            let code = (|| {
                let captured = std::cell::Cell::new(None);
                let filename = crate::expansion::Expansion::new(session, last, &captured)
                    .redirect_path(path.clone())
                    .map_err(std::io::Error::other)?;
                #[cfg(target_os = "linux")]
                let file = match &child_input {
                    Some(fd) => std::fs::File::open(format!("/proc/self/fd/{}", fd.as_raw_fd()))
                        .or_else(|_| std::fs::File::open(&filename)),
                    None => std::fs::File::open(&filename),
                };
                #[cfg(not(target_os = "linux"))]
                let file = std::fs::File::open(&filename);
                let mut file = file.map_err(|e| {
                    io::Error::new(
                        e.kind(),
                        format!("{filename}: {}", crate::builtins::os_message(&e)),
                    )
                })?;
                // Write straight to fd 1. Stdout's line buffer could retain the
                // last bytes across _exit when the file has no final newline.
                let mut stdout =
                    std::fs::File::from(std::io::stdout().as_fd().try_clone_to_owned()?);
                std::io::copy(&mut file, &mut stdout)?;
                Ok::<_, std::io::Error>(())
            })();
            if let Err(e) = &code {
                diagnostic!("{e}");
            }
            unsafe { libc::_exit(i32::from(code.is_err())) }
        }
        let code = crate::eval::borrowed_program(session, body, last, true).child_status();
        crate::subshell::exit_child(session, code)
    }
    // A writer can fill the pipe before it exits. Drain first, then reap even
    // when reading fails. The parent must drop its own writer to observe EOF.
    drop(write);
    read_output(read, pid)
}

pub(super) fn read_output(
    mut read: impl Read + std::os::fd::AsRawFd,
    pid: libc::pid_t,
) -> Result<(String, u8), &'static str> {
    let mut output = Vec::new();
    let result = if crate::editor::preview::can_wait() {
        crate::editor::preview::read_capture(&mut read, &mut output)
    } else {
        read.read_to_end(&mut output).map(|_| ())
    };
    drop(read);
    let code = crate::execute::wait(pid).map_err(|_| "cannot wait for command substitution")?;
    result.map_err(|_| "cannot read command substitution")?;
    captured_output(output, code)
}

fn captured_output(mut output: Vec<u8>, code: u8) -> Result<(String, u8), &'static str> {
    if output.contains(&0) {
        diagnostic!("warning: command substitution: ignored null byte in input");
        output.retain(|&b| b != 0);
    }
    while output.last() == Some(&b'\n') {
        output.pop();
    }
    // An assigned capture can outlive the command. Release large reservations
    // when a short read, discarded NULs or trailing newlines left little text.
    if output.capacity() > 8192 && output.len() < output.capacity() / 4 {
        output.shrink_to_fit();
    }
    let value =
        String::from_utf8(output).map_err(|_| "command substitution produced invalid UTF-8")?;
    Ok((value, code))
}

fn sole_plain_command(body: &[parser::Statement]) -> Option<&parser::SimpleCommand<parser::Word>> {
    sole_command(body).filter(|cmd| cmd.redirects.is_empty())
}

fn sole_command(body: &[parser::Statement]) -> Option<&parser::SimpleCommand<parser::Word>> {
    let [statement] = body else {
        return None;
    };
    let cmd = statement.single_stage()?;
    (!statement.negated
        && statement.condition == parser::Condition::Always
        && cmd.compound.is_none()
        && cmd.assignments.is_empty())
    .then_some(cmd)
}

// A single builtin helper can borrow the caller's variables when neither its
// operands nor its output can observe the omitted function and subshell frames.
fn wrapped_capture(
    session: &mut Session,
    body: &[parser::Statement],
    last: u8,
) -> Option<Result<(String, u8), &'static str>> {
    let call = sole_plain_command(body)?;
    let (name, arguments) = call.words.split_first()?;
    let name = name.as_literal()?;
    let function = session.functions.get(name)?;
    if session.function_nesting_full()
        || session.shopt.extdebug
        || session.traced_functions.contains(name)
        || session.keyword && !call.keyword_args.is_empty()
    {
        return None;
    }
    let Some(cmd) = sole_command(function.definition.body.statements()) else {
        let definition = function.definition.clone();
        return helper::capture(session, definition.body.statements(), arguments, last);
    };
    let (program, operands) = cmd.words.split_first()?;
    let path = program.as_literal()?;
    if path.starts_with('/')
        && cmd.redirects.is_empty()
        && arguments.is_empty()
        && !session.traps.observes_commands()
        && !session.is_exported("SHELLOPTS")
        && !session.functions.contains_key(path)
        && operands.iter().all(|arg| arg.as_literal().is_some())
    {
        return Some(crate::execute::capture_external(
            cmd.clone(),
            session,
            last,
            None,
        ));
    }
    if !can_elide_child(session)
        || session.functions.contains_key(path)
        || session.disabled_builtins.contains(path)
        || session.keyword && !cmd.keyword_args.is_empty()
    {
        return None;
    }
    let (program, operands) = if matches!(path, "builtin" | "command") {
        let (name, operands) = operands.split_first()?;
        let name = name.as_literal()?;
        if session.disabled_builtins.contains(name) {
            return None;
        }
        (name, operands)
    } else {
        (path, operands)
    };
    match program {
        "echo" | ":" | "true" | "false" => {}
        "printf" => {
            let format = operands.first()?.as_literal()?;
            if format.starts_with('-') || crate::session::printf::may_assign_count(format) {
                return None;
            }
        }
        _ => return None,
    }
    if !arguments
        .iter()
        .chain(operands)
        .all(|arg| inert_operand(arg, session, false))
    {
        return None;
    }
    let definition = function.definition.clone();
    let captured = std::cell::Cell::new(None);
    let mut env = crate::expansion::Expansion::new(session, last, &captured);
    let mut fields = Vec::with_capacity(arguments.len());
    for argument in arguments {
        env.append_fields(argument.clone(), &mut fields).ok()?;
    }
    let outer = std::mem::replace(&mut session.positional, fields.into_iter());
    let result = capture_simple(session, definition.body.statements(), last);
    session.positional = outer;
    result
}

// Caught OS signals reset at exec. Parent handlers still run after the
// foreground wait. CHLD and inherited shell tracing can observe the child.
fn can_elide_child(session: &Session) -> bool {
    !session.traps.catches(libc::SIGCHLD)
        && !session.functrace
        && !session.errtrace
        && !session.traps.in_action()
}

fn capture_program<'a>(word: &'a parser::Word, session: &'a Session) -> Option<&'a str> {
    if let Some(name) = word.as_literal() {
        return Some(name);
    }
    if !can_elide_child(session) {
        return None;
    }
    stored_path(word, session, false).filter(|path| path.starts_with('/'))
}

fn stored_path<'a>(
    word: &'a parser::Word,
    session: &'a Session,
    allow_unquoted: bool,
) -> Option<&'a str> {
    use crate::word::{Part, Word, is_identifier};
    if let Some(name) = word.as_literal() {
        return Some(name);
    }
    let Word::Expanded(parts) = word else {
        return None;
    };
    let [Part::Parameter { name, quoted }] = parts.as_slice() else {
        return None;
    };
    if !quoted && !allow_unquoted {
        return None;
    }
    // A quoted stored pathname supplies exactly one word. Preserve resolution
    // of function overrides and the ordinary subshell for dynamic shell state.
    let path = if name.starts_with(|c: char| c.is_ascii_digit()) {
        let index = name.parse::<usize>().ok()?.checked_sub(1)?;
        session.positional.as_slice().get(index)?
    } else if is_identifier(name) && !matches!(name.as_ref(), "_" | "BASH_COMMAND" | "LINENO") {
        session.stored_scalar(name)?
    } else {
        return None;
    };
    Some(path)
}

// Only a single output/status builtin with inert operands can be evaluated
// inside another substitution without observing the missing subshell scope.
fn nested_output(word: &parser::Word, session: &Session, depth: usize) -> bool {
    use crate::word::{Part, Word};
    match word {
        Word::Literal(_) => true,
        Word::Expanded(parts) => parts.iter().all(|part| match part {
            Part::Text(_) => true,
            Part::Substitution { body, .. } if depth < 8 => {
                sole_plain_command(body).is_some_and(|cmd| {
                    cmd.words.split_first().is_some_and(|(program, args)| {
                        program.as_literal().is_some_and(|name| {
                            matches!(name, "echo" | ":" | "true" | "false")
                                && !session.functions.contains_key(name)
                                && !session.disabled_builtins.contains(name)
                        }) && args
                            .iter()
                            .all(|arg| nested_output(arg, session, depth + 1))
                    })
                })
            }
            _ => false,
        }),
        Word::Declaration(_) => false,
    }
}

fn inert_parameter(name: &str, session: &Session) -> bool {
    matches!(name, "@" | "*" | "#")
        || (!name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()))
        || (parser::is_identifier(name)
            && !matches!(
                name,
                "BASHPID"
                    | "BASH_SUBSHELL"
                    | "BASH_COMMAND"
                    | "LINENO"
                    | "SHELLOPTS"
                    | "RANDOM"
                    | "SRANDOM"
                    | "_"
                    | "PIPESTATUS"
                    | "FUNCNAME"
                    | "BASH_LINENO"
                    | "BASH_SOURCE"
                    | "BASH_ARGV"
                    | "BASH_ARGC"
            )
            && !session.is_nameref(name))
}

// Pattern and unquoted literal segments inside a parameter operator are data.
// A top-level wildcard still goes through the ordinary pathname expansion path.
fn inert_operand(word: &parser::Word, session: &Session, operand: bool) -> bool {
    use crate::word::{Alternative, Modification, Part, Word};
    let Word::Expanded(parts) = word else {
        return matches!(word, Word::Literal(_));
    };
    let inner = |w| inert_operand(w, session, true);
    parts.iter().all(|part| match part {
        Part::Text(_) => true,
        Part::Pattern(_) | Part::Unquoted(_) => operand,
        Part::Parameter { name, .. } => inert_parameter(name, session),
        Part::Trim { name, pattern, .. } => inert_parameter(name, session) && inner(pattern),
        Part::Alternative {
            name,
            op: Alternative::Default | Alternative::Alternate,
            word,
            ..
        } => inert_parameter(name, session) && inner(word),
        Part::Modify {
            name, operation, ..
        } if inert_parameter(name, session) => match operation.as_ref() {
            Modification::Replace {
                pattern,
                replacement,
                ..
            } => inner(pattern) && inner(replacement),
            Modification::Case { pattern, .. } => inner(pattern),
            Modification::Slice { offset, length } => {
                let numeric = |w: &Word| {
                    w.as_literal()
                        .is_some_and(|s| crate::arithmetic::inert(s, session))
                };
                numeric(offset) && length.as_ref().is_none_or(numeric)
            }
            Modification::Quote | Modification::Attributes | Modification::Escapes => true,
            _ => false,
        },
        Part::Arithmetic { expression, .. } => crate::arithmetic::inert(expression, session),
        _ => false,
    })
}

// Validate the whole list before executing any builtin. A later function,
// redirect or effectful word must send the complete substitution through fork.
fn safe_capture_word(
    word: &crate::word::Word,
    session: &Session,
    external: bool,
    body: &[parser::Statement],
    capture_line: &std::cell::Cell<Option<usize>>,
) -> bool {
    use crate::word::{Part, Word, is_identifier};
    if inert_operand(word, session, false) {
        return true;
    }
    let Word::Expanded(parts) = word else {
        return false;
    };
    // LINENO and array arguments have separate capture rules.
    parts.iter().all(|part| match part {
        Part::Text(_) => true,
        Part::Parameter { name, quoted } => {
            if name == "LINENO"
                && !session.is_nameref(name)
                && !external
                && can_elide_child(session)
                && sole_plain_command(body).is_some()
            {
                capture_line.set(Some(body[0].location.line));
                return true;
            }
            inert_parameter(name, session)
                || name.strip_suffix("[@]").is_some_and(|base| {
                    external && *quoted && is_identifier(base) && session.stored_array(base)
                })
        }
        _ => false,
    })
}

fn capture_query(
    session: &mut Session,
    cmd: &parser::SimpleCommand<parser::Word>,
    last: u8,
) -> Option<Result<(String, u8), &'static str>> {
    let (name, operands) = cmd.words.split_first()?;
    let name = name.as_literal()?;
    if !matches!(name, "pwd" | "type" | "command" | "compgen")
        || session.functions.contains_key(name)
        || session.disabled_builtins.contains(name)
        || session.keyword && !cmd.keyword_args.is_empty()
        || session.noexec
        || session.traps.observes_commands()
        || !can_elide_child(session)
    {
        return None;
    }
    let mut output = Vec::new();
    let status = if name == "compgen" {
        let args = capture_arguments(operands, session, last)?;
        crate::completion_spec::capture_candidates(session, &args, last, &mut output)?
    } else if name == "pwd" {
        // POSIX logical pwd can repair the shell's directory cache after a rename.
        if session.posix {
            return None;
        }
        let physical = match operands {
            [] => session.physical,
            [opt] => match opt.as_literal()? {
                "-P" => true,
                "-L" => false,
                _ => return None,
            },
            _ => return None,
        };
        session.capture_pwd(physical, &mut output).ok()?;
        0
    } else {
        if name == "command"
            && !operands.first()?.as_literal().is_some_and(|opt| {
                opt.strip_prefix('-').is_some_and(|s| {
                    s.contains(['v', 'V']) && s.bytes().all(|b| matches!(b, b'v' | b'V' | b'p'))
                })
            })
        {
            return None;
        }
        if name == "command" && session.posix {
            return None;
        }
        let words = capture_arguments(&cmd.words, session, last)?;
        crate::execute::describe_words(&words, false, session, |chunk| {
            if output.is_empty() {
                std::mem::swap(&mut output, chunk);
            } else {
                output.append(chunk);
            }
            0
        })
    };
    Some(captured_output(output, status))
}

fn capture_simple(
    session: &mut Session,
    body: &[parser::Statement],
    last: u8,
) -> Option<Result<(String, u8), &'static str>> {
    use crate::word::Word;
    if body.is_empty()
        || session.traps.catches(libc::SIGCHLD)
        || session.xtrace
        || session.functrace && session.traps.debug_handler()
        || session.nounset
        || session.errexit && session.shopt.inherit_errexit
        || session.errtrace && session.traps.error_handler()
    {
        return None;
    }
    if let Some(cmd) = sole_plain_command(body)
        && (!session.keyword || cmd.keyword_args.is_empty())
        && let [name, flag, names @ ..] = cmd.words.as_slice()
        && name.as_literal() == Some("trap")
        && (flag.as_literal() == Some("-p") || flag.as_literal() == Some("-P") && !names.is_empty())
        && names
            .iter()
            .all(|word| word.as_literal().is_some_and(|s| !s.starts_with('-')))
        && !session.functions.contains_key("trap")
        && !session.disabled_builtins.contains("trap")
    {
        let mut out = Vec::new();
        let status = session.traps.list(
            names.iter().map(|word| word.as_literal().unwrap()),
            &mut out,
            session.posix && flag.as_literal() == Some("-p"),
            flag.as_literal() == Some("-P"),
            session.posix,
        );
        return Some(captured_output(out, status));
    }
    if let Some(cmd) = sole_plain_command(body)
        && let Some(result) = capture_query(session, cmd, last)
    {
        return Some(result);
    }
    let capture_line = std::cell::Cell::new(None);
    let command = |cmd: &parser::SimpleCommand<Word>| {
        if session.keyword && !cmd.keyword_args.is_empty() {
            return None;
        }
        let (first, args) = cmd.words.split_first()?;
        let first = capture_program(first, session)?;
        let (program, args) = if matches!(first, "command" | "builtin") {
            let (target, args) = args.split_first()?;
            let target = target.as_literal()?;
            if target.starts_with('-')
                || session.disabled_builtins.contains(target)
                || first == "builtin" && crate::execute::can_capture_external(target)
            {
                return None;
            }
            (target, args)
        } else {
            (first, args)
        };
        if cmd.compound.is_some()
            || !cmd.assignments.is_empty()
            || session.functions.contains_key(first)
            || session.disabled_builtins.contains(first)
        {
            return None;
        }
        let external = match program {
            "jobs"
                if body.len() == 1
                    && session.jobs.is_empty()
                    && !session.traps.observes_commands()
                    && (args.is_empty()
                        || matches!(args, [flag] if flag.as_literal() == Some("-p"))) =>
            {
                false
            }
            "jobs" => return None,
            "printf" if body.len() > 1 => {
                // A dynamic format, -v or a %n conversion can assign a variable
                // used by the next command. Keep such lists in a real subshell.
                let fmt = args.first()?.as_literal()?;
                if fmt.starts_with('-') || crate::session::printf::may_assign_count(fmt) {
                    return None;
                }
                false
            }
            "printf" | "echo" | ":" | "true" | "false" => false,
            // A child drops monitor mode and normally clears errexit. An exported
            // SHELLOPTS lets even a literal external command observe those changes.
            _ if body.len() == 1
                && crate::execute::can_capture_external(program)
                && !((session.errexit || session.jobs.monitor)
                    && session.is_exported("SHELLOPTS")) =>
            {
                true
            }
            _ => return None,
        };
        if !args.iter().all(|arg| {
            safe_capture_word(arg, session, external, body, &capture_line)
                || can_elide_child(session) && nested_output(arg, session, 0)
        }) {
            return None;
        }
        let safe = match cmd.redirects.as_slice() {
            [] => true,
            [r] if !external => {
                // The string-only formatter never writes a diagnostic. Open the
                // device after expansion, exactly where the redirect takes effect.
                body.len() == 1
                    && !session.noclobber
                    && program == "printf"
                    && matches!(
                        args.first().and_then(Word::as_literal),
                        Some("%s" | "%s\n" | "%s\\n")
                    )
                    && r.variable.is_none()
                    && r.fd == 2
                    && r.target == parser::Target::Write
                    && r.word.as_literal() == Some("/dev/null")
            }
            [r] if external && r.variable.is_none() => match (r.fd, r.target) {
                (2, parser::Target::Write) => r.word.as_literal() == Some("/dev/null"),
                (0, parser::Target::HereString) => {
                    can_elide_child(session)
                        && safe_capture_word(&r.word, session, external, body, &capture_line)
                }
                (0, parser::Target::HereDoc { expand }) if can_elide_child(session) => {
                    let text = r.word.as_literal()?;
                    !expand
                        || parser::expansion_string(text).is_ok_and(|word| {
                            safe_capture_word(&word, session, external, body, &capture_line)
                        })
                }
                _ => false,
            },
            _ => false,
        };
        safe.then_some(())
    };
    if let [statement] = body
        && !statement.negated
        && statement.condition == parser::Condition::Always
        && let parser::Command::Pipeline(stages) = &statement.command
        && let [writer, reader] = stages.as_slice()
        && writer.redirects.is_empty()
        && reader.redirects.is_empty()
        && matches!(
            writer.words.first().and_then(Word::as_literal),
            Some("echo" | "printf")
        )
        && reader
            .words
            .first()
            .and_then(Word::as_literal)
            .is_some_and(crate::execute::can_capture_external)
        && !session.jobs.monitor
        && can_elide_child(session)
        && command(writer).is_some()
        && command(reader).is_some()
    {
        return crate::execute::capture_with_writer(writer, reader, session, last);
    }
    if !body.iter().all(|s| {
        !s.negated && s.condition == parser::Condition::Always
            && matches!(&s.command, parser::Command::Pipeline(stages) if matches!(stages.as_slice(), [cmd] if command(cmd).is_some()))
    }) {
        return wrapped_capture(session, body, last);
    }
    let mut output = Vec::new();
    let mut code = last;
    let mut fields = Vec::new();
    for statement in body {
        let parser::Command::Pipeline(stages) = &statement.command else {
            unreachable!()
        };
        let cmd = &stages[0];
        let words = if matches!(cmd.words[0].as_literal(), Some("command" | "builtin")) {
            &cmd.words[1..]
        } else {
            &cmd.words[..]
        };
        let name = words[0].as_literal();
        if name == Some("jobs") {
            return Some(Ok((String::new(), 0)));
        }
        if !matches!(name, Some("echo" | "printf" | ":" | "true" | "false")) {
            return Some(crate::execute::capture_external(
                cmd.clone(),
                session,
                last,
                None,
            ));
        }
        let captured = std::cell::Cell::new(None);
        let original = session.signal_line;
        if let Some(line) = capture_line.get() {
            session.signal_line = Some(line);
        }
        let mut env = crate::expansion::Expansion::new(session, code, &captured);
        fields.clear();
        let expanded = words[1..]
            .iter()
            .try_for_each(|arg| env.append_fields(arg.clone(), &mut fields));
        session.signal_line = original;
        if let Err(error) = expanded {
            // Earlier commands may already have printed a diagnostic. Finish
            // the validated list with the ordinary expansion error recovery.
            error.report("");
            match error.flow(session) {
                Flow::Continue(status) => {
                    code = status;
                    continue;
                }
                flow => return Some(captured_output(output, flow.child_status())),
            }
        }
        if !cmd.redirects.is_empty() {
            let device = crate::execute::open_null_device(libc::O_WRONLY)?;
            // A real substitution needs two free descriptors for its capture pipe.
            // Defer to it on descriptor exhaustion or an unusual device.
            device.try_clone().ok()?;
        }
        code = match name.unwrap() {
            "echo" => {
                crate::builtins::echo_into(&fields, &mut output, session);
                0
            }
            "printf" => crate::session::printf::run(session, &fields, Some(&mut output)),
            "true" | ":" => 0,
            "false" => 1,
            _ => unreachable!(),
        };
    }
    Some(captured_output(output, code))
}

fn input_only(body: &[parser::Statement]) -> Option<&parser::Word> {
    let [statement] = body else {
        return None;
    };
    let cmd = statement.single_stage()?;
    let [redirect] = cmd.redirects.as_slice() else {
        return None;
    };
    (redirect.fd == 0
        && redirect.target == parser::Target::Read
        && cmd.compound.is_none()
        && cmd.words.is_empty()
        && cmd.assignments.is_empty()
        && !statement.negated)
        .then_some(&redirect.word)
}

// Only the final command may write. Until it is validated, the prefix has no
// observable effects, so a fallback cannot duplicate any output.
fn foreground_builtin(
    session: &mut Session,
    body: &[parser::Statement],
    mut status: u8,
) -> Option<Flow> {
    if session.jobs.monitor
        || session.xtrace
        || session.errexit
        || session.noexec
        || session.traps.observes_commands()
        || !can_elide_child(session)
    {
        return None;
    }
    for (i, statement) in body.iter().enumerate() {
        if statement.negated || statement.condition != parser::Condition::Always {
            return None;
        }
        if let parser::Command::LiteralAssignment(text, len) = &statement.command
            && session.capture_local(&text[..len.get()])
        {
            status = 0;
            continue;
        }
        if let Some(cmd) = statement.single_stage()
            && cmd.compound.is_none()
            && cmd.words.is_empty()
            && cmd.redirects.is_empty()
            && !cmd.assignments.is_empty()
            && cmd.assignments.iter().all(|(k, v)| {
                session.capture_local(k)
                    && matches!(v, parser::Assignment::Scalar(w) if w.as_literal().is_some())
            })
        {
            status = 0;
            continue;
        }
        let cmd = sole_plain_command(std::slice::from_ref(statement))?;
        let (program, operands) = cmd.words.split_first()?;
        let name = program.as_literal()?;
        if session.functions.contains_key(name)
            || session.disabled_builtins.contains(name)
            || session.keyword && !cmd.keyword_args.is_empty()
            || !operands.iter().all(|w| w.as_literal().is_some())
        {
            return None;
        }
        let last = i + 1 == body.len();
        status = match name {
            ":" | "true" => 0,
            "false" => 1,
            "exit" if last => match operands {
                [] => status,
                [n] => n.as_literal()?.parse::<u8>().ok()?,
                _ => return None,
            },
            "echo" | "printf" if last => {
                if name == "printf"
                    && !matches!(operands.first()?.as_literal()?, "%s" | "%s\n" | "%s\\n")
                {
                    return None;
                }
                if !foreground_output() {
                    return None;
                }
                let args: Vec<_> = operands
                    .iter()
                    .map(|w| w.as_literal().unwrap().to_owned())
                    .collect();
                if name == "echo" {
                    crate::builtins::echo(&args, session)
                } else {
                    session.printf(&args)
                }
            }
            _ => return None,
        };
    }
    Some(Flow::Continue(status))
}

// A broken pipe or a file-size limit can terminate a real child by signal.
// Do not expose the parent shell to those signals on an output shortcut.
fn foreground_output() -> bool {
    let mut st = unsafe { std::mem::zeroed::<libc::stat>() };
    if unsafe { libc::fstat(1, &mut st) } != 0 {
        return false;
    }
    match st.st_mode & libc::S_IFMT {
        libc::S_IFREG => {
            let mut limit = unsafe { std::mem::zeroed::<libc::rlimit>() };
            unsafe {
                libc::getrlimit(libc::RLIMIT_FSIZE, &mut limit) == 0
                    && limit.rlim_cur == libc::RLIM_INFINITY
            }
        }
        libc::S_IFCHR => {
            if unsafe { libc::tcgetpgrp(1) == libc::getpgrp() } {
                return true;
            }
            let mut null = unsafe { std::mem::zeroed::<libc::stat>() };
            unsafe {
                libc::stat(c"/dev/null".as_ptr(), &mut null) == 0
                    && null.st_mode & libc::S_IFMT == libc::S_IFCHR
                    && st.st_rdev == null.st_rdev
            }
        }
        _ => false,
    }
}

pub fn launch_group(
    session: &mut Session,
    body: Vec<parser::Statement>,
    status: u8,
    background: bool,
    parenthesized: bool,
) -> Flow {
    if !background && let Some(flow) = foreground_builtin(session, &body, status) {
        return flow;
    }
    launch_list(session, background, parenthesized, |s| {
        execute_program(s, body, status, true)
    })
}

pub(crate) fn launch_borrowed(
    session: &mut Session,
    body: &[parser::Statement],
    status: u8,
) -> Flow {
    if let Some(flow) = foreground_builtin(session, body, status) {
        return flow;
    }
    launch_list(session, false, true, |s| {
        crate::eval::borrowed_program(s, body, status, true)
    })
}

fn launch_list(
    session: &mut Session,
    background: bool,
    parenthesized: bool,
    run: impl FnOnce(&mut Session) -> Flow,
) -> Flow {
    let mut launch = session.jobs.launch(background, false);
    let pid = match session.fork_shell() {
        Ok(pid) => pid,
        Err(e) => {
            diagnostic!("fork: {e}");
            return Flow::Continue(1);
        }
    };
    if pid == 0 {
        launch.child(&mut session.traps);
        if background && launch.group.is_none() && (session.posix || !session.stdin_redirected) {
            let fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
            if fd < 0 || unsafe { libc::dup2(fd, 0) } < 0 {
                unsafe { libc::_exit(1) }
            }
            if fd != 0 {
                unsafe {
                    libc::close(fd);
                }
            }
        }
        session.enter_subshell();
        session.parenthesized_subshell = parenthesized;
        session.pipeline_subshell = false;
        crate::execute::coprocess::after_fork(session);
        session.jobs.forked(false);
        session.command_string = false;
        let code = run(session).child_status();
        crate::subshell::exit_child(session, code)
    }
    launch.parent(pid);
    if launch.group.is_none() && !background {
        return Flow::Continue(crate::execute::wait(pid).unwrap_or(1));
    }
    match session.jobs.finish(
        launch,
        vec![Ok(pid)],
        session.current_command.as_ref().map_or_else(
            || "command list".into(),
            |location| crate::execute::job_command(location.text()).into(),
        ),
        false,
        &mut session.pipe_status,
    ) {
        Ok((code, pid)) => {
            if background {
                session.last_background = pid;
            }
            Flow::Continue(code)
        }
        Err(e) => {
            diagnostic!("{e}");
            Flow::Continue(1)
        }
    }
}

/// Run shell EXIT handling before terminating the forked process.
pub(crate) fn exit_child(session: &mut Session, code: u8) -> ! {
    let code = crate::traps::finish(session, code);
    unsafe { libc::_exit(i32::from(code)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::OwnedFd;

    #[test]
    fn capture_uses_eof_when_the_size_hint_is_stale() {
        for hint in [1, 5, 4096] {
            let (reader, mut writer) = io::pipe().unwrap();
            writer.write_all(b"hello").unwrap();
            drop(writer);
            let mut bytes = Vec::new();
            read_regular_file(
                &std::fs::File::from(OwnedFd::from(reader)),
                hint,
                &mut bytes,
            )
            .unwrap();
            assert_eq!(bytes, b"hello");
        }
    }

    #[test]
    fn trimmed_capture_releases_the_large_input_buffer() {
        let mut bytes = vec![b'\n'; 1024 * 1024];
        bytes[0] = b'x';
        let (text, _) = captured_output(bytes, 0).unwrap();
        assert_eq!(text, "x");
        assert!(text.capacity() < 8192);
    }
}
