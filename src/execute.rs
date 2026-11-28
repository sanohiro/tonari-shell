mod redirect;
use redirect::*;
pub(crate) use redirect::{capture_stdout, open_null_device};
mod external;
pub(crate) use external::Environment;
use external::External;
mod builtin;
use builtin::*;
pub(crate) use builtin::{builtin_names, special_builtin};
mod prepare;
use prepare::*;
pub(crate) use prepare::{ArrayDeclaration, PreparedCommand};
pub(crate) mod coprocess;
pub(crate) mod fd;
pub(crate) mod hash;
mod named_fd;
pub(crate) mod process_substitution;
use std::path::{Path, PathBuf};
mod here_input;
mod lookup;
pub(crate) use lookup::{Candidate, IgnoredExecutables, check_candidate};
mod spawn;
pub(crate) use spawn::inherited_env;

use std::cell::Cell;
use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::rc::Rc;

use crate::expansion::Expansion;
use crate::parser::{Assignment, Pipeline, Redirect, SimpleCommand, Target, Word};
use crate::session::Session;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Flow {
    Continue(u8),
    Discard(u8),
    Exit(u8),
    Return(u8),
    Break(u32, u8),
    LoopContinue(u32),
}

impl Flow {
    pub(crate) fn child_status(self) -> u8 {
        match self {
            Self::Continue(code) | Self::Exit(code) | Self::Return(code) | Self::Discard(code) => {
                code
            }
            Self::Break(_, code) => code,
            Self::LoopContinue(_) => 0,
        }
    }
}

fn errno() -> i32 {
    io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO)
}

fn write_error(name: &[u8], message: &[u8]) {
    write_error_detail(name, message, b"");
}

#[cold]
fn write_error_detail(name: &[u8], message: &[u8], detail: &[u8]) {
    write_diagnostic(b"tonari: ", name, message, detail);
}

#[cold]
fn write_diagnostic(prefix: &[u8], name: &[u8], message: &[u8], detail: &[u8]) {
    // This can run after fork. Emit the diagnostic in a single write when
    // possible, without buffering, heap allocation or taking a stdio lock.
    let mut buffers = [prefix, name, message, detail, b"\n"].map(|part| libc::iovec {
        iov_base: part.as_ptr().cast_mut().cast(),
        iov_len: part.len(),
    });
    let mut rest = buffers.as_mut_slice();
    while !rest.is_empty() {
        let n = unsafe { libc::writev(2, rest.as_ptr(), rest.len() as libc::c_int) };
        if n < 0 && errno() == libc::EINTR {
            continue;
        }
        if n <= 0 {
            break;
        }
        let mut used = n as usize;
        while let Some(first) = rest.first_mut() {
            if used < first.iov_len {
                first.iov_base = unsafe { first.iov_base.byte_add(used) };
                first.iov_len -= used;
                break;
            }
            used -= first.iov_len;
            rest = &mut rest[1..];
        }
    }
}

fn exec_status(error: i32) -> u8 {
    if matches!(error, libc::ENOENT | libc::ENOTDIR) {
        127
    } else {
        126
    }
}

// Pipe setup uses standard streams as destinations. Reserve distinct source
// numbers even when Tonari starts with a standard stream already closed.
pub(crate) fn private_fd(fd: OwnedFd) -> io::Result<OwnedFd> {
    if fd.as_raw_fd() > 2 {
        return Ok(fd);
    }
    let copy = unsafe {
        libc::fcntl(
            fd.as_raw_fd(),
            libc::F_DUPFD_CLOEXEC,
            crate::execute::fd::FIRST_NONSTANDARD,
        )
    };
    if copy < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(copy) })
}

fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let (read, write) = io::pipe()?;
    #[cfg(target_os = "linux")]
    {
        const TARGET: libc::c_int = 128 * 1024;
        let fd = read.as_raw_fd();
        let capacity = unsafe { libc::fcntl(fd, libc::F_GETPIPE_SZ) };
        // Preserve larger defaults and the reduced capacity used under quota pressure.
        if (64 * 1024..TARGET).contains(&capacity) {
            unsafe { libc::fcntl(fd, libc::F_SETPIPE_SZ, TARGET) };
        }
    }
    Ok((private_fd(read.into())?, private_fd(write.into())?))
}

enum Stage {
    Child(libc::pid_t),
    GroupedChild(libc::pid_t),
    Status(u8),
}

impl Stage {
    fn pid(&self) -> Option<libc::pid_t> {
        match *self {
            Self::Child(pid) | Self::GroupedChild(pid) => Some(pid),
            Self::Status(_) => None,
        }
    }

    fn join_group(&self, launch: &mut crate::jobs::Launch) {
        match *self {
            Self::Child(pid) => launch.parent(pid),
            Self::GroupedChild(pid) => launch.record_child(pid),
            Self::Status(_) => {}
        }
    }
}

fn connect_pipe(
    input: Option<&OwnedFd>,
    output: Option<&OwnedFd>,
    next_reader: Option<&OwnedFd>,
) -> Vec<Action<'static>> {
    let mut setup = Vec::new();
    for (fd, dest) in [(input, 0), (output, 1)] {
        if let Some(fd) = fd {
            setup.push(Action::Dup(fd.as_raw_fd(), dest));
            setup.push(Action::Close(fd.as_raw_fd()));
        }
    }
    if let Some(unused) = next_reader {
        setup.push(Action::Close(unused.as_raw_fd()));
    }
    setup
}
pub(crate) fn reset_child_signals() {
    crate::traps::reset_caught_signals(&[libc::SIGINT, libc::SIGQUIT, libc::SIGTERM]);
}

fn stage_expansion(word: &Word) -> bool {
    use crate::word::Part;
    match word {
        Word::Literal(_) => false,
        Word::Expanded(parts) | Word::Declaration(parts) => parts.iter().any(|part| match part {
            Part::CurrentSubstitution { .. }
            | Part::Substitution { .. }
            | Part::Process { .. }
            | Part::Arithmetic { .. } => true,
            Part::Modify { .. } => true,
            Part::Nested(word) => stage_expansion(word),
            Part::Parameter { name, .. } => name == "BASHPID",
            Part::Alternative { name, op, word, .. } => {
                *op == crate::word::Alternative::Assign
                    || name == "BASHPID"
                    || stage_expansion(word)
            }
            Part::Trim { name, pattern, .. } => name == "BASHPID" || stage_expansion(pattern),
            _ => false,
        }),
    }
}

fn redirection_observable(r: &Redirect<Word>) -> bool {
    if r.variable.is_some() {
        return true;
    }
    stage_expansion(&r.word)
        || matches!(r.target, Target::HereDoc { expand: true })
            && r.word
                .as_literal()
                .is_some_and(|text| text.contains(['$', '`']))
}

// A parenthesized pipeline stage is already the subshell Bash exposes.
fn enter_stage(session: &mut Session, compound: Option<&crate::parser::Command>, background: bool) {
    let explicit = matches!(compound, Some(crate::parser::Command::Subshell(_)));
    session.enter_subshell();
    session.jobs.forked(!explicit && !background);
    if explicit {
        session.command_string = false;
    }
}

// Expansions that can read stdin or observe the stage PID must run after pipe
// installation. In particular, `printf x | echo "$(cat)"` consumes that pipe.
fn start_raw(
    mut raw: SimpleCommand<Word>,
    input: Option<OwnedFd>,
    output: Option<OwnedFd>,
    next_reader: Option<&OwnedFd>,
    last_status: u8,
    session: &mut Session,
    launch: crate::jobs::Launch,
) -> io::Result<Stage> {
    if session.keyword {
        separate_keyword_arguments(&mut raw);
    }
    let observable = needs_ordered_redirects(&raw, session)
        || raw.redirects.iter().any(redirection_observable)
        || raw.words.iter().any(stage_expansion)
        || raw.assignments.iter().any(|(_, value)| match value {
            Assignment::Scalar(word) => stage_expansion(word),
            Assignment::Array(_, items, _) => items.iter().any(|(_, word)| stage_expansion(word)),
        });
    if !observable {
        return match prepare(raw, session, last_status, false) {
            Ok(mut cmd) => start(
                &mut cmd,
                input,
                output,
                next_reader,
                last_status,
                session,
                launch,
            ),
            Err(e) => {
                diagnostic!("{e}");
                Ok(Stage::Status(e.status))
            }
        };
    }
    let setup = connect_pipe(input.as_ref(), output.as_ref(), next_reader);
    if !launch.background {
        session.jobs.refresh_snapshot();
    }
    let pid = session.fork_shell()?;
    if pid == 0 {
        launch.pipeline_child(&mut session.traps);
        enter_stage(session, raw.compound.as_deref(), launch.background);
        coprocess::after_fork(session);
        if let Some(command) = raw.compound.as_deref_mut()
            && let crate::parser::Command::Subshell(body) = command
        {
            *command = crate::parser::Command::Group(std::mem::take(body));
        }
        session.stdin_redirected |= input.is_some();
        let code = if apply(&setup, libc::PIPE_BUF) {
            match run(vec![raw], last_status, true, session) {
                Ok(flow) => flow.child_status(),
                Err(e) => {
                    diagnostic!("{e}");
                    1
                }
            }
        } else {
            1
        };
        crate::subshell::exit_child(session, code)
    }
    Ok(Stage::Child(pid))
}

fn start(
    cmd: &mut PreparedCommand,
    input: Option<OwnedFd>,
    output: Option<OwnedFd>,
    next_reader: Option<&OwnedFd>,
    last_status: u8,
    session: &mut Session,
    launch: crate::jobs::Launch,
) -> io::Result<Stage> {
    let builtin = builtin(cmd, last_status, session).or_else(|| {
        cmd.prepare_missing_hook(session)
            .then_some(Builtin::Function)
    });
    // Numeric exit and constant status builtins do not inspect either pipe end.
    // Keep a real child for job control and observable child-signal traps.
    if !launch.background
        && launch.group.is_none()
        && !session.traps.active()
        && cmd.redirects.is_empty()
        && (input.is_some() || output.is_some())
        && let Some(Builtin::Exit(status) | Builtin::Status(status)) = builtin
    {
        return Ok(Stage::Status(status));
    }
    // Resolve the stage privately when pipeline or disabled-hash lookup has
    // not populated the parent's table. Leave PATH assignments to the child.
    if builtin.is_none()
        && ((!launch.background
            && launch.group.is_none()
            && (input.is_some() || output.is_some() || cmd.redirects.is_empty()))
            || (launch.background
                && launch.group.is_some()
                && input.is_none()
                && output.is_none()
                && cmd.redirects.is_empty()))
        && cmd.cacheable()
        && !cmd.words[0].contains('/')
        && cmd.cached_path(session, &cmd.words[0]).is_none()
        && let Ok(Some(entry)) = hash::search(&cmd.words[0], session)
    {
        cmd.resolved = Some(entry.path);
    }
    let cached = cmd
        .words
        .first()
        .is_some_and(|name| cmd.cached_path(session, name).is_some());
    let merged_pipe = output.as_ref().filter(|_| {
        matches!(cmd.redirects.as_slice(), [r] if r.fd == 2 && r.target == Target::Duplicate && r.word == "1")
    });
    if let Some(builtin) = builtin
        && output.is_some()
        && input.is_none()
        && !launch.background
        && launch.group.is_none()
        && !session.traps.active()
        && (cmd.redirects.is_empty() || merged_pipe.is_some())
        && let Some(plain) = PlainOutput::new(builtin, cmd, 512)
    {
        // POSIX guarantees room for at least 512 bytes in this new, empty
        // pipe. We still own its reader, so this write can finish before
        // starting the consumer. Managed jobs and trapped children need a PID.
        use std::io::Write;
        let (bytes, status) = plain.render(session);
        debug_assert!(bytes.len() <= 512);
        let result = std::fs::File::from(output.unwrap()).write_all(&bytes);
        return Ok(Stage::Status(if result.is_err() { 1 } else { status }));
    }
    if builtin.is_none()
        && launch.group.is_none()
        && !launch.background
        && (cached || cmd.words[0].contains('/'))
        && let [r] = cmd.redirects.as_slice()
        && r.fd == 0
        && matches!(r.target, Target::HereDoc { .. } | Target::HereString)
        && r.word.len() <= here_input::PIPE_LIMIT
        && let Ok(reader) =
            here_input::open(r.word.as_bytes(), here_input::PIPE_LIMIT).and_then(private_fd)
        && let Ok(pid) =
            External::new(cmd, session)?.spawn_redirected(&[], None, Some(&reader), output.as_ref())
    {
        return Ok(Stage::Child(pid));
    }
    if builtin.is_none()
        && launch.group.is_none()
        && !launch.background
        && ((!cmd.redirects.is_empty() && merged_pipe.is_none())
            || input.is_some()
            || output.is_some())
        && (cached || cmd.words[0].contains('/'))
    {
        let copies = cmd
            .redirects
            .first()
            .is_none_or(|r| r.target == Target::Duplicate && (0..=2).contains(&r.fd));
        let null = open_null_redirect(&cmd.redirects);
        let opened = null.as_ref().map(|(dest, fd)| (*dest, fd));
        if copies || opened.is_some() {
            let external = External::new(cmd, session)?;
            if let Ok(pid) =
                external.spawn_redirected(&cmd.redirects, opened, input.as_ref(), output.as_ref())
            {
                return Ok(Stage::Child(pid));
            }
            if opened.is_some() {
                // Preserve this same open file description for ENOEXEC and diagnostics.
                return fork_stage(
                    cmd,
                    builtin,
                    (input.as_ref(), output.as_ref(), next_reader),
                    opened,
                    last_status,
                    session,
                    launch,
                );
            }
        }
    }
    let resolved = cached || cmd.words.first().is_some_and(|name| name.contains('/'));
    if builtin.is_none()
        && (launch.group.is_none()
            || ((launch.background || !session.traps.has_explicit_traps())
                && launch.can_spawn_group()))
        && (!launch.background || launch.group.is_some())
        && resolved
        && input.is_none()
        && output.is_none()
        && cmd.redirects.is_empty()
    {
        match External::new(cmd, session)?.spawn_resolved(
            None,
            (launch.group.is_none() && cmd.assignments.is_empty()).then_some(&*session),
            launch,
        ) {
            Ok(pid) => {
                #[cfg(target_os = "macos")]
                launch.resume_spawned(pid);
                return Ok(if launch.group.is_some() {
                    Stage::GroupedChild(pid)
                } else {
                    Stage::Child(pid)
                });
            }
            Err(_) if launch.group.is_some() => {}
            Err(e) if e.raw_os_error() == Some(libc::ENOEXEC) => {}
            Err(e) => return spawn_failure(&cmd.words[0], e),
        }
    }
    fork_stage(
        cmd,
        builtin,
        (input.as_ref(), output.as_ref(), next_reader),
        None,
        last_status,
        session,
        launch,
    )
}

fn fork_stage(
    cmd: &mut PreparedCommand,
    builtin: Option<Builtin>,
    pipes: (Option<&OwnedFd>, Option<&OwnedFd>, Option<&OwnedFd>),
    opened: Option<(RawFd, &OwnedFd)>,
    last_status: u8,
    session: &mut Session,
    launch: crate::jobs::Launch,
) -> io::Result<Stage> {
    let (input, output, next_reader) = pipes;
    // Opening a FIFO can block until a later pipeline stage starts. Do those
    // opens in a child so the parent can launch the remaining stages first.
    let mut setup = connect_pipe(input, output, next_reader);
    // A trap installed by this command runs after its local redirects end.
    // An explicit subshell retains its enclosing redirects through shell exit.
    let scoped = !cmd.redirects.is_empty() && builtin.is_some_and(|b| can_install_trap(b, cmd));
    if let Some((dest, fd)) = opened {
        setup.push(Action::Dup(fd.as_raw_fd(), dest));
    } else if !scoped {
        setup.extend(actions(cmd));
    }
    let external = builtin
        .is_none()
        .then(|| External::new(cmd, session))
        .transpose()?;
    if builtin.is_some() && !launch.background {
        session.jobs.refresh_snapshot();
    }
    let pid = session.fork_shell()?;
    if pid == 0 {
        // External argv and environment are prepared before fork. A shell builtin
        // runs in a child copy of the single-threaded interpreter and may reenter it.
        // Restore signals caught by the interactive shell before blocking on a file.
        launch.pipeline_child(&mut session.traps);
        enter_stage(session, cmd.compound.as_deref(), launch.background);
        if cmd.missing_hook && !launch.background {
            // The hook has a private execution environment. A normal command
            // search itself does not add a shell-language subshell level.
            session.subshell_depth -= 1;
        }
        session.stdin_redirected |= cmd.redirects.iter().any(|r| r.fd == 0);
        session.stdin_redirected |= input.is_some();
        let code = if !apply(&setup, here_pipe_limit(builtin, cmd)) {
            1
        } else if let Some(external) = external.as_ref() {
            external.exec(None)
        } else {
            coprocess::after_fork(session);
            let flow = if scoped {
                run_builtin(builtin.unwrap(), cmd, session, last_status, false).unwrap_or_else(
                    |e| {
                        diagnostic!("{e}");
                        Flow::Continue(1)
                    },
                )
            } else {
                eval_builtin(builtin.unwrap(), cmd, session, last_status, true)
            };
            flow.child_status()
        };
        crate::subshell::exit_child(session, code)
    }
    Ok(Stage::Child(pid))
}

// A single external process supplies both the captured bytes and the exit status.
pub(super) fn can_capture_external(name: &str) -> bool {
    !name.is_empty() && named_builtin(name, &[], 0).is_none()
}

// The builtin's entire output fits in an empty pipe. Keep a pipe on stdin
// even for zero bytes so the consumer observes the same descriptor type.
pub(super) fn capture_with_writer(
    writer: &SimpleCommand<Word>,
    reader: &SimpleCommand<Word>,
    session: &mut Session,
    last: u8,
) -> Option<Result<(String, u8), &'static str>> {
    use std::io::Write;
    let cmd = prepare(writer.clone(), session, last, true).ok()?;
    let output = PlainOutput::new(builtin(&cmd, last, session)?, &cmd, 512)?;
    let (input, write) = pipe().ok()?;
    let (bytes, status) = output.render(session);
    if std::fs::File::from(write).write_all(&bytes).is_err() {
        return Some(Err("cannot write command substitution input"));
    }
    Some(
        capture_external(reader.clone(), session, last, Some(input)).map(|(text, code)| {
            (
                text,
                if code == 0 && session.pipefail {
                    status
                } else {
                    code
                },
            )
        }),
    )
}

pub(super) fn capture_external(
    raw: SimpleCommand<Word>,
    session: &mut Session,
    last: u8,
    input: Option<OwnedFd>,
) -> Result<(String, u8), &'static str> {
    let mut cmd = match prepare(raw, session, last, true) {
        Ok(cmd) => cmd,
        Err(error) => {
            diagnostic!("{error}");
            return Ok((String::new(), error.status));
        }
    };
    // A substitution inherits the parent's hash entries; a fresh lookup must
    // never insert into the parent's hash table or change its hit counts.
    if let Some(name) = cmd.words.first()
        && !name.contains('/')
        && cmd.cacheable()
        && cmd.cached_path(session, name).is_none()
        && let Ok(Some(entry)) = hash::search(name, session)
    {
        cmd.resolved = Some(entry.path);
    }
    let (reader, writer) = pipe().map_err(|_| "cannot open command substitution pipe")?;
    // An external-only capture uses its command process as the substitution
    // scope. The command-not-found function must observe that same level.
    session.subshell_depth += 1;
    let stage = start(
        &mut cmd,
        input,
        Some(writer),
        None,
        last,
        session,
        Default::default(),
    );
    session.subshell_depth -= 1;
    let pid = match stage.map_err(|_| "cannot launch command substitution")? {
        Stage::Child(pid) | Stage::GroupedChild(pid) => pid,
        Stage::Status(code) => return Ok((String::new(), code)),
    };
    crate::subshell::read_output(std::fs::File::from(reader), pid)
}

fn spawn_failure(name: &str, error: io::Error) -> io::Result<Stage> {
    let code = error.raw_os_error().unwrap_or(libc::EIO);
    if matches!(
        code,
        libc::EAGAIN | libc::ENOMEM | libc::EMFILE | libc::ENFILE
    ) {
        return Err(error);
    }
    diagnostic!("{name}: {error}");
    Ok(Stage::Status(exec_status(code)))
}

pub(super) fn wait(pid: libc::pid_t) -> io::Result<u8> {
    loop {
        if let Some(code) =
            crate::traps::child_event(pid, 0)?.and_then(crate::traps::ChildEvent::exit_status)
        {
            return Ok(code);
        }
    }
}

pub(crate) fn parameter(session: &Session, last_status: u8, name: &str) -> Option<String> {
    match name {
        "?" => Some(last_status.to_string()),
        "$" => Some(session.shell_pid().to_string()),
        "!" => session.last_background.map(|pid| pid.to_string()),
        "-" => Some(session.option_flags()),
        "0" => Some(session.arg0.clone()),
        _ => session.get(name),
    }
}

pub(crate) fn trace_line(session: &mut Session, status: u8, append: impl FnOnce(&mut String)) {
    let prefix = session.get("PS4").unwrap_or_default();
    session.xtrace = false;
    let mut line = crate::prompt::render_text(session, prefix, status);
    session.xtrace = true;
    let levels = session.trace_depth + session.source_depth;
    if levels > 0
        && let Some(first) = line.chars().next()
    {
        let mut nested = String::with_capacity(line.len() + first.len_utf8() * levels as usize);
        nested.extend(std::iter::repeat_n(first, levels as usize));
        nested.push_str(&line);
        line = nested;
    }
    append(&mut line);
    line.push('\n');
    let fd = session
        .scalar_setting("BASH_XTRACEFD")
        .and_then(|v| v.parse().ok())
        .filter(|&fd| !fd::is_private(fd))
        .unwrap_or(2);
    let mut data = line.as_bytes();
    while !data.is_empty() {
        let n = unsafe { libc::write(fd, data.as_ptr().cast(), data.len()) };
        if n > 0 {
            data = &data[n as usize..];
        } else if n == 0 || errno() != libc::EINTR {
            break;
        }
    }
}

pub(crate) fn trace_word(word: &str) -> std::borrow::Cow<'_, str> {
    if word == "'" {
        return "\\'".into();
    }
    if word.bytes().any(|b| b.is_ascii_control()) {
        return String::from_utf8(crate::word::quote::ansi_quote(word.as_bytes(), false))
            .unwrap()
            .into();
    }
    if !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_alphanumeric() || "_+-/:.,=@%".contains(c))
    {
        return word.into();
    }
    crate::word::quote::single_quote(word).into()
}

pub fn run(
    pipeline: Pipeline<Word>,
    last_status: u8,
    replace: bool,
    session: &mut Session,
) -> io::Result<Flow> {
    run_pipeline(pipeline, last_status, replace, session, false)
}

fn job_label(
    pipeline: &[SimpleCommand<Word>],
    background: bool,
    session: &Session,
) -> Option<String> {
    // Literal arguments survive preparation, except for lookup prefixes.
    // Build their job label only if a foreground child is actually launched.
    if !background && !session.jobs.monitor {
        return None;
    }
    if pipeline.iter().any(|cmd| cmd.compound.is_some())
        && let Some(location) = &session.current_command
    {
        return Some(
            location
                .text()
                .trim()
                .trim_end_matches('&')
                .trim_end()
                .to_owned(),
        );
    }
    if !background
        && let [cmd] = pipeline
        && cmd.assignments.is_empty()
        && !matches!(
            cmd.words.first().and_then(Word::as_literal),
            Some("command" | "builtin")
        )
        && cmd.words.iter().all(|w| w.as_literal().is_some())
    {
        return None;
    }
    let mut label = String::new();
    for (stage, cmd) in pipeline.iter().enumerate() {
        if stage != 0 {
            label.push_str(" | ");
        }
        for (i, word) in cmd.words.iter().enumerate() {
            if i != 0 {
                label.push(' ');
            }
            label.push_str(word.as_literal().unwrap_or("command"));
        }
    }
    Some(label)
}

// In sh mode a regular command installs its redirects after argument expansion
// and before expanding its assignment prefixes (POSIX 2.9.1.1).
fn needs_ordered_redirects(raw: &SimpleCommand<Word>, session: &Session) -> bool {
    if !session.posix
        || raw.redirects.is_empty()
        || raw.assignments.is_empty()
        || raw.words.is_empty()
    {
        return false;
    }
    if let Some(name) = raw.words[0].as_literal() {
        if special_builtin(name) {
            return false;
        }
        // An infallible literal prefix only becomes visible during dispatch.
        // With literal file redirects the ordinary route has the same effects.
        if !session.xtrace
            && raw.words[1..].iter().all(|w| w.as_literal().is_some())
            && raw.assignments.iter().all(|(key, value)| {
                matches!(value, Assignment::Scalar(Word::Literal(_)))
                    && crate::parser::is_identifier(key)
                    && !session.is_nameref(key)
                    && !session.integer(key)
                    && !session.readonly(key)
            })
            && raw.redirects.iter().all(|r| {
                r.variable.is_none()
                    && matches!(
                        r.target,
                        Target::Read
                            | Target::Write
                            | Target::Clobber
                            | Target::Append
                            | Target::ReadWrite
                    )
                    && r.word.as_literal().is_some()
            })
        {
            return false;
        }
    }
    true
}

fn run_ordered(
    mut raw: SimpleCommand<Word>,
    name: Option<String>,
    status: u8,
    replace: bool,
    session: &mut Session,
) -> io::Result<Flow> {
    let captured = Cell::new(None);
    let words = match expand_command_words(&mut raw, session, status, &captured) {
        Ok(words) => words,
        Err(e) => {
            e.report("");
            return Ok(e.flow(session));
        }
    };
    let redirects = if words.first().is_some_and(|w| !special_builtin(w)) {
        std::mem::take(&mut raw.redirects)
    } else {
        Vec::new()
    };
    // An external utility expands its redirection operands in a child
    // environment. Only dynamic operands can expose that distinction.
    let child_redirects = redirects
        .iter()
        .any(|r| r.word.as_literal().is_none() || redirection_observable(r))
        && {
            let (skip, search, _) = lookup::scan_prefix(&words, lookup::Search::Normal, session);
            builtin_words(&words[skip..], search, 0, status, session, || {
                session.search_path(&[])
            })
            .is_none()
        };
    named_fd::with_redirects(redirects, session, status, child_redirects, |session| {
        let cmd = match prepare_expanded(raw, words, session, status, true, &captured) {
            Ok(cmd) => cmd,
            Err(e) => {
                e.report("");
                return Ok((e.flow(session), false));
            }
        };
        let exec = matches!(builtin(&cmd, status, session), Some(Builtin::Exec));
        let flow = run_prepared(cmd, name, status, replace, session)?;
        let commit = exec && matches!(flow, Flow::Continue(0));
        Ok((flow, commit))
    })
}

fn run_prepared(
    mut cmd: PreparedCommand,
    name: Option<String>,
    last_status: u8,
    replace: bool,
    session: &mut Session,
) -> io::Result<Flow> {
    let simple = cmd.compound.is_none();
    if !cmd.named_redirects.is_empty() {
        let last = simple.then(|| cmd.words.last().cloned().unwrap_or_default());
        let launch = session.jobs.launch(false, true);
        let result = named_fd::run(cmd, session, last_status, replace, launch);
        if let Some(last) = last {
            session.last_argument = Some(last)
        }
        return result;
    }
    let selected = builtin(&cmd, last_status, session);
    // Functions and set can consume argv before $_ is updated.
    let saved = matches!(selected, Some(Builtin::Function | Builtin::Set))
        .then(|| cmd.words.last().cloned())
        .flatten();
    let result = if let Some(builtin) = selected {
        run_builtin(builtin, &mut cmd, session, last_status, replace)
    } else {
        run_external(&mut cmd, name, last_status, replace, session)
    };
    if simple {
        session.last_argument = Some(saved.or_else(|| cmd.words.pop()).unwrap_or_default());
    }
    result
}

fn run_external(
    cmd: &mut PreparedCommand,
    name: Option<String>,
    last_status: u8,
    replace: bool,
    session: &mut Session,
) -> io::Result<Flow> {
    // A foreground builtin runs in this shell. Snapshot terminal modes
    // only when preparing to hand the terminal to a child process.
    let mut launch = session.jobs.launch(false, true);
    if !cmd.cacheable() {
        if cmd.assignments.iter().any(|(k, _)| k == "PATH") {
            session.hashes.clear();
        }
    } else if !replace {
        match hash::resolve(session, &cmd.words[0]) {
            Ok(path) => cmd.resolved = path,
            Err(hash::Absent)
                if session
                    .functions
                    .contains_key(PreparedCommand::MISSING_HOOK) =>
            {
                cmd.select_missing_hook();
            }
            Err(hash::Absent)
                if cmd.redirects.is_empty()
                    && launch.group.is_none()
                    && !session.traps.active() =>
            {
                let prefix = session.diagnostic_prefix(|| session.line());
                eprintln!(
                    "{}{}: command not found",
                    prefix.as_deref().unwrap_or("tonari: "),
                    cmd.words[0]
                );
                return Ok(Flow::Continue(127));
            }
            Err(hash::Absent) => {}
        }
    }
    if replace && launch.group.is_none() && !cmd.prepare_missing_hook(session) {
        let executable = External::new(cmd, session)?;
        return Ok(Flow::Continue(
            if apply(&actions(cmd), here_input::PIPE_LIMIT) {
                executable.exec(
                    (cmd.assignments.is_empty() && cmd.redirects.is_empty()).then_some(session),
                )
            } else {
                1
            },
        ));
    }
    let stage = start(cmd, None, None, None, last_status, session, launch)?;
    let status = match stage {
        Stage::Child(pid) | Stage::GroupedChild(pid) if launch.group.is_some() => {
            stage.join_group(&mut launch);
            session
                .jobs
                .finish(
                    launch,
                    vec![Ok(pid)],
                    name.unwrap_or_else(|| cmd.words.join(" ")),
                    session.pipefail,
                    &mut session.pipe_status,
                )?
                .0
        }
        Stage::Child(pid) | Stage::GroupedChild(pid) => wait(pid)?,
        Stage::Status(code) => code,
    };
    Ok(Flow::Continue(status))
}

fn foreground(
    mut raw: std::borrow::Cow<'_, SimpleCommand<Word>>,
    last_status: u8,
    replace: bool,
    session: &mut Session,
) -> io::Result<Flow> {
    if session.keyword {
        separate_keyword_arguments(raw.to_mut());
    }
    if raw.words.is_empty() && raw.redirects.is_empty() && raw.compound.is_none() && !session.xtrace
    {
        let assigned = match raw.assignments.as_slice() {
            [] => true,
            [(key, Assignment::Scalar(word))] => assign_existing_word(session, key, word),
            _ => false,
        };
        if assigned {
            session.last_argument = Some(String::new());
            return Ok(Flow::Continue(0));
        }
    }
    prepared_foreground(raw, last_status, replace, session)
}

fn prepared_foreground(
    raw: std::borrow::Cow<'_, SimpleCommand<Word>>,
    last_status: u8,
    replace: bool,
    session: &mut Session,
) -> io::Result<Flow> {
    let name = job_label(std::slice::from_ref(raw.as_ref()), false, session);
    if needs_ordered_redirects(&raw, session) {
        return run_ordered(raw.into_owned(), name, last_status, replace, session);
    }
    let prepared = match raw {
        std::borrow::Cow::Borrowed(stage)
            if stage.compound.is_none()
                && stage.assignments.is_empty()
                && stage.redirects.is_empty()
                && (!session.posix || stage.keyword_args.is_empty()) =>
        {
            prepare::borrowed_arguments(&stage.words, session, last_status)
        }
        _ => prepare(raw.into_owned(), session, last_status, true),
    };
    let cmd = match prepared {
        Ok(cmd) => cmd,
        Err(e) => {
            e.report("");
            return Ok(e.flow(session));
        }
    };
    run_prepared(cmd, name, last_status, replace, session)
}

fn lastpipe(
    raw: SimpleCommand<Word>,
    incoming: OwnedFd,
    last_status: u8,
    session: &mut Session,
) -> io::Result<Flow> {
    let pipe = Action::Dup(incoming.as_raw_fd(), 0);
    let saved = SavedFd::capture(&pipe, std::slice::from_ref(&pipe))?;
    duplicate(incoming.as_raw_fd(), 0)?;
    drop(incoming);
    let was_redirected = std::mem::replace(&mut session.stdin_redirected, true);
    let result = foreground(std::borrow::Cow::Owned(raw), last_status, false, session);
    session.stdin_redirected = was_redirected;
    saved.restore(0)?;
    result
}

pub(crate) fn literal_assignment(
    session: &mut Session,
    text: &str,
    end: usize,
    status: u8,
    tail: bool,
) -> io::Result<Flow> {
    if let Some(flow) = crate::traps::debug(session, status) {
        return Ok(flow);
    }
    let key = &text[..end];
    let value = &text[end + 1..];
    if !session.xtrace && session.assign_existing_literal(key, value) {
        session.last_argument = Some(String::new());
        return Ok(Flow::Continue(0));
    }
    scalar_command(
        key.to_owned(),
        Word::Literal(value.into()),
        status,
        tail,
        session,
    )
}

pub(crate) fn run_scalar(
    pair: std::borrow::Cow<'_, (String, Word)>,
    last_status: u8,
    replace: bool,
    session: &mut Session,
) -> io::Result<Flow> {
    if let Some(flow) = crate::traps::debug(session, last_status) {
        return Ok(flow);
    }
    if !session.xtrace && assign_existing_word(session, &pair.0, &pair.1) {
        session.last_argument = Some(String::new());
        return Ok(Flow::Continue(0));
    }
    let (key, word) = pair.into_owned();
    scalar_command(key, word, last_status, replace, session)
}

fn scalar_command(
    key: String,
    word: Word,
    last_status: u8,
    replace: bool,
    session: &mut Session,
) -> io::Result<Flow> {
    let command = SimpleCommand {
        assignments: vec![(key, Assignment::Scalar(word))],
        ..Default::default()
    };
    prepared_foreground(
        std::borrow::Cow::Owned(command),
        last_status,
        replace,
        session,
    )
}

fn assign_existing_word(session: &mut Session, key: &str, word: &Word) -> bool {
    if let Some(value) = word.as_literal() {
        return session.assign_existing_literal(key, value);
    }
    if let Word::Expanded(parts) = word {
        use crate::word::Part;
        let parts = match parts.as_slice() {
            [Part::Text(empty), rest @ ..] if empty.is_empty() => rest,
            other => other,
        };
        match parts {
            [Part::Arithmetic { expression, .. }] => {
                if let Some(Ok(n)) = expression.compiled(session, false) {
                    return session.assign_existing_literal(key, itoa::Buffer::new().format(n));
                }
            }
            [Part::Parameter { name, .. } | Part::BraceParameter(name)]
                if name.starts_with('#') =>
            {
                if let Some(n) = session.stored_char_count(&name[1..]) {
                    return session.assign_existing_literal(key, itoa::Buffer::new().format(n));
                }
            }
            [
                Part::Parameter { name, .. } | Part::BraceParameter(name),
                Part::Text(suffix) | Part::Pattern(suffix),
            ] if name == key && session.can_append_scalar(key) => {
                return session.append_plain_scalar(key, suffix);
            }
            _ => {}
        }
    }
    word.stored_parameter(session)
        .map(str::to_owned)
        .is_some_and(|value| session.assign_existing_literal(key, &value))
}

pub(crate) fn run_single(
    raw: std::borrow::Cow<'_, SimpleCommand<Word>>,
    last_status: u8,
    replace: bool,
    session: &mut Session,
) -> io::Result<Flow> {
    if raw.compound.is_none()
        && let Some(flow) = crate::traps::debug(session, last_status)
    {
        return Ok(flow);
    }
    foreground(raw, last_status, replace, session)
}

pub fn run_pipeline(
    mut pipeline: Pipeline<Word>,
    last_status: u8,
    replace: bool,
    session: &mut Session,
    background: bool,
) -> io::Result<Flow> {
    if pipeline.len() == 1 && !background {
        return run_single(
            std::borrow::Cow::Owned(pipeline.pop().unwrap()),
            last_status,
            replace,
            session,
        );
    }
    let mut launch = session.jobs.launch(background, pipeline.len() == 1);
    let managed = background || launch.group.is_some();
    let name = job_label(&pipeline, background, session);
    let mut input =
        if background && launch.group.is_none() && (session.posix || !session.stdin_redirected) {
            Some(private_fd(std::fs::File::open("/dev/null")?.into())?)
        } else {
            None
        };
    let length = pipeline.len();
    let mut stages: Vec<Stage> = Vec::with_capacity(length);
    let mut last_flow = None;
    for (i, mut raw) in pipeline.into_iter().enumerate() {
        if !session.traps.in_action()
            && let Some(loc) = raw.location.take()
        {
            session.current_command = Some(*loc);
        }
        if raw.compound.is_none()
            && let Some(flow) = crate::traps::debug(session, last_status)
        {
            drop(input);
            session.jobs.restore();
            for stage in stages {
                if let Some(pid) = stage.pid() {
                    let _ = wait(pid);
                }
            }
            return Ok(flow);
        }
        let stage = (|| {
            let incoming = input.take();
            if i + 1 == length
                && !managed
                && session.shopt.lastpipe
                && let Some(reader) = incoming
            {
                let flow = lastpipe(raw, reader, last_status, session)?;
                let code = match flow {
                    Flow::Continue(code) => return Ok(Stage::Status(code)),
                    Flow::Exit(code) | Flow::Discard(code) | Flow::Return(code) => code,
                    Flow::Break(..) | Flow::LoopContinue(_) => 0,
                };
                last_flow = Some(flow);
                return Ok(Stage::Status(code));
            }
            let output = if i + 1 < length {
                let (reader, writer) = pipe()?;
                input = Some(reader);
                Some(writer)
            } else {
                None
            };
            start_raw(
                raw,
                incoming,
                output,
                input.as_ref(),
                last_status,
                session,
                launch,
            )
        })();
        match stage {
            Ok(stage) => {
                stage.join_group(&mut launch);
                stages.push(stage);
            }
            Err(e) => {
                drop(input);
                for stage in &stages {
                    if let Some(pid) = stage.pid() {
                        unsafe { libc::kill(pid, libc::SIGKILL) };
                    }
                }
                for stage in stages {
                    if let Some(pid) = stage.pid() {
                        let _ = wait(pid);
                    }
                }
                session.jobs.restore();
                return Err(e);
            }
        }
    }
    if managed {
        let processes = stages
            .into_iter()
            .map(|s| match s {
                Stage::Child(p) | Stage::GroupedChild(p) => Ok(p),
                Stage::Status(c) => Err(c),
            })
            .collect();
        let (code, pid) = session.jobs.finish(
            launch,
            processes,
            name.unwrap_or_default(),
            session.pipefail,
            &mut session.pipe_status,
        )?;
        if background {
            session.last_background = pid;
        }
        return Ok(Flow::Continue(code));
    }
    let mut status = 0;
    let mut wait_error = None;
    session.pipe_status.clear();
    for stage in stages {
        let result = match stage {
            Stage::Child(pid) | Stage::GroupedChild(pid) => wait(pid),
            Stage::Status(code) => Ok(code),
        };
        session.pipe_status.push(*result.as_ref().unwrap_or(&1));
        match result {
            Ok(code) if !session.pipefail || code != 0 => status = code,
            Ok(_) => {}
            Err(error) => wait_error = Some(error),
        }
    }
    if let Some(e) = wait_error {
        return Err(e);
    }
    Ok(last_flow.unwrap_or(Flow::Continue(status)))
}
