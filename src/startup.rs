use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use crate::cli::Invocation;
use crate::{Flow, Session, input};

struct Script {
    file: File,
    metadata: std::fs::Metadata,
}

fn open_script(path: &Path) -> io::Result<Script> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if metadata.is_dir() {
        return Err(io::Error::from_raw_os_error(libc::EISDIR));
    }
    Ok(Script { file, metadata })
}

fn sourced(
    session: &mut Session,
    script: Script,
    path: &Path,
    args: &[String],
    status: u8,
) -> Flow {
    // Guard the interpreter stack if startup files source one another recursively.
    if session.source_depth >= 128 {
        eprintln!("tonari: source nesting limit exceeded");
        return Flow::Continue(1);
    }
    let old = (!args.is_empty())
        .then(|| std::mem::replace(&mut session.positional, Vec::from(args).into_iter()));
    session.source_set_args = false;
    session.call_lines.push(session.line());
    let callsite = session.current_command.take();
    let caller_signal_line = session.signal_line;
    session.source_depth += 1;
    session.push_source_path(path);
    session.function_names.push("source".into());
    let saved_debug = session.traps.enter_source(session.functrace);
    let result = input::file_script(session, script.file, Some(script.metadata), status, false);
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
    if !args.is_empty() {
        session.source_set_args = false;
    }
    match result {
        Flow::Return(code) => Flow::Continue(code),
        other => other,
    }
}

pub fn source_file(session: &mut Session, words: &[String], status: u8, fatal: bool) -> Flow {
    let mut args = &words[1..];
    let mut search = None;
    while let Some((option, tail)) = args.split_first() {
        if option == "--" {
            args = tail;
            break;
        }
        if option == "-" || !option.starts_with('-') {
            break;
        }
        let Some(path) = option.strip_prefix("-p") else {
            eprintln!("tonari: source: {option}: invalid option");
            return Flow::Continue(2);
        };
        if path.is_empty() {
            let Some((path, tail)) = tail.split_first() else {
                eprintln!("tonari: source: -p requires a path");
                return Flow::Continue(2);
            };
            search = Some(path.as_str());
            args = tail;
        } else {
            search = Some(path);
            args = tail;
        }
    }
    let Some(name) = args.first() else {
        eprintln!("tonari: source: filename required");
        return Flow::Continue(2);
    };
    let direct = name.contains('/');
    let inherited = (search.is_none() && !direct && session.shopt.sourcepath)
        .then(|| session.get("PATH"))
        .flatten();
    if let Some(path) = search.or(inherited.as_deref()).filter(|_| !direct) {
        for dir in path.split(':') {
            let path = Path::new(dir).join(name);
            if let Ok(file) = open_script(&path) {
                return sourced(session, file, &path, &args[1..], status);
            }
        }
    }
    if !direct && (search.is_some() || session.posix && words[0] == "." && session.shopt.sourcepath)
    {
        eprintln!("tonari: .: {name}: file not found");
        return if fatal {
            Flow::Exit(1)
        } else {
            Flow::Continue(1)
        };
    }
    match open_script(Path::new(name)) {
        Ok(file) => sourced(session, file, Path::new(name), &args[1..], status),
        Err(e) => {
            eprintln!("tonari: source: {name}: {e}");
            if fatal {
                Flow::Exit(1)
            } else {
                Flow::Continue(1)
            }
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
        result
    } else {
        eprintln!("tonari: {}: cannot start debugger", path.display());
        Flow::Continue(0)
    };
    session.functrace = enabled;
    session.errtrace = enabled;
    result
}

fn optional(session: &mut Session, path: &Path) -> Option<Flow> {
    let file = open_script(path).ok()?;
    Some(sourced(session, file, path, &[], 0))
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
        .get("HOME")
        .map(|h| Path::new(&h).join(".bash_logout"));
    let system = cfg!(target_os = "linux").then(|| PathBuf::from("/etc/bash.bash_logout"));
    for path in home.into_iter().chain(system) {
        if let Ok(file) = open_script(&path)
            && let Flow::Exit(n) = sourced(session, file, &path, &[], previous)
        {
            return n;
        }
    }
    code
}

pub fn initialize(session: &mut Session, invocation: &Invocation) -> Flow {
    if invocation.check {
        return Flow::Continue(0);
    }
    if invocation.login && !invocation.noprofile && (invocation.as_sh || !session.posix) {
        if let Some(exit @ Flow::Exit(_)) = optional(session, Path::new("/etc/profile")) {
            return exit;
        }
        if let Some(home) = session.get("HOME") {
            let profiles: &[&str] = if invocation.as_sh {
                &[".profile"]
            } else {
                &[".bash_profile", ".bash_login", ".profile"]
            };
            for name in profiles {
                if let Some(flow) = optional(session, &Path::new(&home).join(name)) {
                    if let Flow::Exit(_) = flow {
                        return flow;
                    }
                    break;
                }
            }
        }
    }
    if invocation.interactive && !invocation.as_sh && !session.posix {
        if !invocation.login && !invocation.norc {
            #[cfg(target_os = "linux")]
            if let Some(exit @ Flow::Exit(_)) = optional(session, Path::new("/etc/bash.bashrc")) {
                return exit;
            }
            let rc = invocation.rcfile.clone().or_else(|| {
                session
                    .get("HOME")
                    .map(|h| PathBuf::from(h).join(".bashrc"))
            });
            if let Some(path) = rc {
                return optional(session, &path).unwrap_or(Flow::Continue(0));
            }
        }
    } else {
        let var = if invocation.interactive {
            "ENV"
        } else if invocation.as_sh || session.posix {
            return Flow::Continue(0);
        } else {
            "BASH_ENV"
        };
        let Some(path) = session.get(var).filter(|p| !p.is_empty()) else {
            return Flow::Continue(0);
        };
        if unsafe { libc::getuid() != libc::geteuid() || libc::getgid() != libc::getegid() } {
            return Flow::Continue(0);
        }
        let path = if path.contains(['$', '`', '\\']) {
            let word = match crate::parser::expansion_string(&path) {
                Ok(word) => word,
                Err(error) => {
                    eprintln!("tonari: {var}: {}", error.message());
                    return Flow::Continue(1);
                }
            };
            match crate::eval::expand_case(session, word, 0, false) {
                Ok(path) => path,
                Err(flow) => return flow,
            }
        } else {
            path
        };
        return optional(session, Path::new(&path)).unwrap_or(Flow::Continue(0));
    }
    Flow::Continue(0)
}
