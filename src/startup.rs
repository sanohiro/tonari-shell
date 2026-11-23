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
    let old = (!args.is_empty()).then(|| std::mem::replace(&mut session.positional, args.to_vec()));
    let callsite = session.current_command.clone();
    session.call_lines.push(session.line());
    session.source_depth += 1;
    session
        .source_names
        .push(path.to_string_lossy().into_owned());
    session.function_names.push("source".into());
    let saved_debug = session.traps.enter_source(session.functrace);
    let result = input::file_script(session, script.file, Some(script.metadata), status, false);
    let result = crate::traps::returning(session, result);
    session.traps.leave_source(saved_debug);
    session.function_names.pop();
    session.source_names.pop();
    session.source_depth -= 1;
    session.call_lines.pop();
    session.current_command = callsite;
    if let Some(old) = old {
        session.positional = old;
    }
    match result {
        Flow::Return(code) => Flow::Continue(code),
        other => other,
    }
}

pub fn source_file(session: &mut Session, words: &[String], status: u8) -> Flow {
    let args = &words[1..];
    let args = if args.first().is_some_and(|s| s == "--") {
        &args[1..]
    } else {
        args
    };
    let Some(name) = args.first() else {
        eprintln!("tonari: source: filename required");
        return Flow::Continue(2);
    };
    if let Some(path) = (session.shopt.sourcepath && !name.contains('/'))
        .then(|| session.get("PATH"))
        .flatten()
    {
        for dir in std::env::split_paths(&path) {
            let path = dir.join(name);
            if let Ok(file) = open_script(&path) {
                return sourced(session, file, &path, &args[1..], status);
            }
        }
    }
    match open_script(Path::new(name)) {
        Ok(file) => sourced(session, file, Path::new(name), &args[1..], status),
        Err(e) => {
            eprintln!("tonari: source: {name}: {e}");
            Flow::Continue(1)
        }
    }
}

fn optional(session: &mut Session, path: &Path) -> Option<Flow> {
    let file = open_script(path).ok()?;
    Some(sourced(session, file, path, &[], 0))
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
    if invocation.login && !invocation.noprofile {
        if let Some(exit @ Flow::Exit(_)) = optional(session, Path::new("/etc/profile")) {
            return exit;
        }
        if let Some(home) = session.get("HOME") {
            for name in [".bash_profile", ".bash_login", ".profile"] {
                if let Some(flow) = optional(session, &Path::new(&home).join(name)) {
                    if let Flow::Exit(_) = flow {
                        return flow;
                    }
                    break;
                }
            }
        }
    }
    if invocation.interactive {
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
    } else if let Some(path) = session.get("BASH_ENV").filter(|p| !p.is_empty()) {
        let path = if path.contains(['$', '`', '\\']) {
            let word = match crate::parser::expansion_string(&path) {
                Ok(word) => word,
                Err(error) => {
                    eprintln!("tonari: BASH_ENV: {}", error.message());
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
