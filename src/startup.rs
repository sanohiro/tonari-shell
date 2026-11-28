use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use crate::cli::Invocation;
use crate::{Flow, Session, input};

// A login shell spawned by a normal sudo/su login already has matching IDs.
// When invoked with unequal IDs, drop them before reading shell-controlled
// environment or files. Tonari does not provide a set-ID execution facility.
pub(crate) fn settle_credentials(privileged: bool) -> io::Result<bool> {
    unsafe {
        let uid = libc::getuid();
        let gid = libc::getgid();
        if uid == libc::geteuid() && gid == libc::getegid() {
            return Ok(false);
        }
        if privileged {
            return Err(io::Error::other(
                "retaining elevated effective credentials is unsupported",
            ));
        }
        if libc::setregid(gid, gid) != 0
            || libc::setreuid(uid, uid) != 0
            || libc::geteuid() != uid
            || libc::getegid() != gid
        {
            return Err(io::Error::other("cannot drop elevated credentials"));
        }
    }
    Ok(true)
}

#[derive(Default)]
pub(crate) struct Environment {
    ssh: bool,
    skip_startup_files: bool,
    pub shell_level: i64,
    pub execution_string: bool,
    pub posix: bool,
    pub ignore_eof: Option<String>,
    pub compat: Option<String>,
    pub shellopts: Option<String>,
    pub bashopts: Option<String>,
}

impl Environment {
    pub fn read(session: &mut Session, changed_ids: bool) -> Self {
        use std::ffi::CStr;
        let mut settings = Self {
            skip_startup_files: changed_ids,
            ..Self::default()
        };
        let import = !session.privileged && !changed_ids;
        let mut lower_eof = None;
        let mut pwd = None;
        let mut entry = crate::execute::inherited_env();

        // SAFETY: the single interpreter thread is still in initialization.
        // No shell code runs or changes environ, and no borrowed entry escapes.
        unsafe {
            while !(*entry).is_null() {
                let raw = *entry;
                entry = entry.add(1);
                if !matches!(*raw as u8, b'B' | b'P' | b'S' | b'I' | b'i') {
                    continue;
                }
                let bytes = CStr::from_ptr(raw).to_bytes();
                let Some(i) = memchr::memchr(b'=', bytes) else {
                    continue;
                };
                let key = &bytes[..i];
                let value = &bytes[i + 1..];
                let utf8 = || std::str::from_utf8(value).ok();
                match key {
                    b"PWD" => {
                        use std::os::unix::ffi::OsStrExt;
                        pwd = Some(Path::new(std::ffi::OsStr::from_bytes(value)));
                    }
                    b"SSH_CLIENT" | b"SSH2_CLIENT" => settings.ssh = true,
                    b"SHLVL" => {
                        settings.shell_level = utf8()
                            .and_then(crate::builtins::integer_operand)
                            .unwrap_or(0)
                    }
                    b"POSIXLY_CORRECT" => settings.posix = true,
                    b"BASH_EXECUTION_STRING" => settings.execution_string = true,
                    b"IGNOREEOF" => settings.ignore_eof = Some(utf8().unwrap_or("").into()),
                    b"ignoreeof" => lower_eof = Some(utf8().unwrap_or("").into()),
                    b"BASH_COMPAT" => settings.compat = utf8().map(str::to_owned),
                    b"SHELLOPTS" if import => settings.shellopts = utf8().map(str::to_owned),
                    b"BASHOPTS" if import => settings.bashopts = utf8().map(str::to_owned),
                    _ if key.starts_with(b"BASH_FUNC_") && import => session.import_function(bytes),
                    _ => {}
                }
            }
        }
        session.initialize_directory(pwd);
        settings.ignore_eof = settings.ignore_eof.or(lower_eof);
        settings
    }
}

struct Script {
    file: File,
    metadata: std::fs::Metadata,
}

pub(crate) fn open_main_script(
    session: &Session,
    path: &Path,
) -> io::Result<(File, Option<PathBuf>)> {
    match File::open(path) {
        Ok(file) => Ok((file, None)),
        Err(first)
            if first.kind() == io::ErrorKind::NotFound
                && !path.as_os_str().as_encoded_bytes().contains(&b'/') =>
        {
            if let Some(search) = session.scalar_setting("PATH") {
                let dirs = crate::execute::search_directories(search.as_ref().as_ref())
                    .filter(|dir| dir != &Path::new("."));
                if let Some((candidate, script)) = search_scripts(dirs, path).map_err(|(_, e)| e)? {
                    return Ok((script.file, Some(candidate)));
                }
            }
            Err(first)
        }
        Err(e) => Err(e),
    }
}

fn open_script(path: &Path) -> io::Result<Script> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if metadata.is_dir() {
        return Err(io::Error::from_raw_os_error(libc::EISDIR));
    }
    Ok(Script { file, metadata })
}

fn search_scripts<'a>(
    dirs: impl Iterator<Item = &'a Path>,
    name: &Path,
) -> Result<Option<(PathBuf, Script)>, (PathBuf, io::Error)> {
    for dir in dirs {
        let candidate = dir.join(name);
        match open_script(&candidate) {
            Ok(script) => return Ok(Some((candidate, script))),
            Err(e)
                if matches!(
                    e.raw_os_error(),
                    Some(libc::ENOENT | libc::ENOTDIR | libc::EISDIR | libc::EACCES | libc::ELOOP)
                ) => {}
            Err(e) => return Err((candidate, e)),
        }
    }
    Ok(None)
}

fn sourced(
    session: &mut Session,
    script: Script,
    path: &Path,
    args: &[String],
    status: u8,
) -> Flow {
    // Guard the interpreter stack if startup files source one another recursively.
    if crate::stack_budget_exceeded() {
        diagnostic!("source nesting limit exceeded");
        return Flow::Unwind(1);
    }
    let old = (!args.is_empty())
        .then(|| std::mem::replace(&mut session.positional, Vec::from(args).into_iter()));
    let outer_set_args = std::mem::take(&mut session.source_set_args);
    session.call_lines.push(session.line());
    let callsite = session.current_command.take();
    let caller_signal_line = session.signal_line;
    session.source_depth += 1;
    session.push_source_path(path);
    session.function_names.push("source".into());
    let saved_debug = session.traps.enter_source(session.functrace);
    let result = input::file_script(session, script.file, Some(script.metadata), status, false);
    let result = crate::traps::leaving_scope(session, result);
    session.signal_line = caller_signal_line;
    session.function_names.pop();
    session.source_names.pop();
    session.source_depth -= 1;
    session.call_lines.pop();
    session.current_command = callsite;
    let result = crate::traps::returning(session, result);
    session.traps.leave_source(saved_debug);
    if let Some(old) = old
        && (session.function_depth != 0 || !session.source_set_args)
    {
        session.positional = old;
    }
    session.source_set_args = outer_set_args || args.is_empty() && session.source_set_args;
    result
}

pub fn source_file(session: &mut Session, words: &[String], status: u8, fatal: bool) -> Flow {
    let command = words[0].as_str();
    let failure = if fatal {
        Flow::Exit(1)
    } else {
        Flow::Continue(1)
    };
    let finish = |flow| match flow {
        Flow::Return(n) => Flow::Continue(n),
        other => other,
    };
    let mut args = &words[1..];
    let mut search = None;
    while let Some(option) = crate::builtins::flags::next(&mut args) {
        let Some(path) = option.strip_prefix("-p") else {
            diagnostic!("{command}: {option}: invalid option");
            return Flow::Continue(2);
        };
        let Some(path) = crate::builtins::flags::value(path, &mut args) else {
            diagnostic!("{command}: -p requires a path");
            return Flow::Continue(2);
        };
        search = Some(path);
    }
    let Some(name) = args.first() else {
        diagnostic!("{command}: filename required");
        return Flow::Continue(2);
    };
    let direct = name.contains('/');
    let inherited = (search.is_none() && !direct && session.shopt.sourcepath)
        .then(|| session.scalar_setting("PATH"))
        .flatten();
    if let Some(path) = search.or(inherited.as_deref()).filter(|_| !direct) {
        match search_scripts(
            crate::execute::search_directories(path.as_ref()),
            Path::new(name),
        ) {
            Ok(Some((path, file))) => {
                return finish(sourced(session, file, &path, &args[1..], status));
            }
            Ok(None) => {}
            Err((path, e)) => {
                diagnostic!(
                    "{command}: {}: {}",
                    path.display(),
                    crate::builtins::os_message(&e)
                );
                return failure;
            }
        }
    }
    if !direct && (search.is_some() || session.posix && words[0] == "." && session.shopt.sourcepath)
    {
        diagnostic!("{command}: {name}: file not found");
        return failure;
    }
    match open_script(Path::new(name)) {
        Ok(file) => finish(sourced(session, file, Path::new(name), &args[1..], status)),
        Err(e) => {
            diagnostic!("{command}: {name}: {}", crate::builtins::os_message(&e));
            failure
        }
    }
}

pub(crate) fn debugger(session: &mut Session) -> Flow {
    let path = Path::new(
        option_env!("DEBUGGER_START_FILE").unwrap_or("/usr/share/bashdb/bashdb-main.inc"),
    );
    let script = open_script(path).ok();
    let enabled = script.is_some();
    session.shopt.extdebug = enabled;
    let result = if let Some(script) = script {
        session
            .debug_base
            .get_or_init(|| session.positional.as_slice().to_vec());
        let errexit = std::mem::replace(&mut session.errexit, false);
        let result = sourced(session, script, path, &[], 0);
        session.errexit = errexit;
        match result {
            Flow::Return(n) => Flow::Continue(n),
            other => other,
        }
    } else {
        diagnostic!("{}: cannot start debugger", path.display());
        Flow::Continue(0)
    };
    session.functrace = enabled;
    session.errtrace = enabled;
    result
}

fn optional(session: &mut Session, path: &Path, status: u8) -> Option<Flow> {
    match open_script(path) {
        Ok(file) if file.metadata.is_file() && file.metadata.len() == 0 => {
            Some(Flow::Continue(status))
        }
        Ok(file) => Some(match sourced(session, file, path, &[], status) {
            Flow::Return(_) => Flow::Continue(session.traps.before_return),
            other => other,
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => {
            if e.raw_os_error() == Some(libc::EISDIR) {
                diagnostic!("{}: is a directory", path.display());
            } else {
                diagnostic!("{}: {}", path.display(), crate::builtins::os_message(&e));
            }
            Some(Flow::Continue(status))
        }
    }
}

pub fn defer_interactive_exit(session: &mut Session) -> bool {
    session.interactive
        && session.subshell_depth == 0
        && session.jobs.defer_exit(
            session.shopt.checkjobs,
            session.command_number,
            session.posix,
        )
}

pub fn logout(session: &mut Session, code: u8, previous: u8) -> u8 {
    if !session.login || session.shell_pid() != std::process::id() || session.logout_started {
        return code;
    }
    session.logout_started = true;
    let home = session
        .personal_home()
        .map(|h| Path::new(h.as_ref()).join(".bash_logout"));
    let system = cfg!(target_os = "linux").then(|| PathBuf::from("/etc/bash.bash_logout"));
    for path in home.into_iter().chain(system) {
        if let Some(Flow::Exit(n)) = optional(session, &path, previous) {
            return n;
        }
    }
    code
}

fn bashrc(session: &mut Session, invocation: &Invocation) -> Flow {
    #[cfg(target_os = "linux")]
    let status = match optional(session, Path::new("/etc/bash.bashrc"), 0) {
        Some(Flow::Continue(n)) => n,
        Some(flow) => return flow,
        None => 0,
    };
    #[cfg(not(target_os = "linux"))]
    let status = 0;
    invocation
        .rcfile
        .clone()
        .or_else(|| {
            session
                .personal_home()
                .map(|p| Path::new(p.as_ref()).join(".bashrc"))
        })
        .and_then(|path| optional(session, &path, status))
        .unwrap_or(Flow::Continue(status))
}

fn remote_stdin() -> bool {
    let mut address = std::mem::MaybeUninit::<libc::sockaddr_storage>::uninit();
    let mut size = std::mem::size_of_val(&address) as libc::socklen_t;
    unsafe { libc::getpeername(0, address.as_mut_ptr().cast(), &mut size) == 0 }
}

pub fn initialize(session: &mut Session, invocation: &Invocation, env: &Environment) -> Flow {
    if invocation.options.check || env.skip_startup_files {
        return Flow::Continue(0);
    }
    if !invocation.login
        && !invocation.norc
        && !invocation.as_sh
        && !session.privileged
        && !invocation.interactive
        && session.command_string
        && env.shell_level <= 0
        && (env.ssh || remote_stdin())
    {
        return bashrc(session, invocation);
    }
    let mut status = 0;
    if invocation.login && !invocation.noprofile && (invocation.as_sh || !session.posix) {
        if let Some(flow) = optional(session, Path::new("/etc/profile"), status) {
            match flow {
                Flow::Continue(n) => status = n,
                other => return other,
            }
        }
        if let Some(home) = session.personal_home().map(|s| PathBuf::from(s.as_ref())) {
            let profiles: &[&str] = if invocation.as_sh {
                &[".profile"]
            } else {
                &[".bash_profile", ".bash_login", ".profile"]
            };
            for name in profiles {
                if let Some(flow) = optional(session, &home.join(name), status) {
                    match flow {
                        Flow::Continue(n) => status = n,
                        other => return other,
                    }
                    break;
                }
            }
        }
    }
    if invocation.interactive && !invocation.as_sh && !session.posix {
        if !invocation.login && !invocation.norc {
            return bashrc(session, invocation);
        }
    } else {
        let var = if invocation.interactive {
            "ENV"
        } else if invocation.as_sh || session.posix {
            return Flow::Continue(status);
        } else {
            "BASH_ENV"
        };
        let Some(path) = session.get(var).filter(|p| !p.is_empty()) else {
            return Flow::Continue(status);
        };
        if session.privileged || invocation.as_su && invocation.login {
            return Flow::Continue(status);
        }
        let path = if path.contains(['$', '`', '\\']) {
            let word = match crate::parser::expansion_string(&path) {
                Ok(word) => word,
                Err(error) => {
                    diagnostic!("{var}: {}", error.message());
                    return Flow::Continue(1);
                }
            };
            match crate::eval::expand_case(session, word, status, false) {
                Ok(path) => path,
                Err(flow) => return flow,
            }
        } else {
            path
        };
        return optional(session, Path::new(&path), status).unwrap_or(Flow::Continue(status));
    }
    Flow::Continue(status)
}
