use super::{External, Redirect, Target};
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};

fn check(code: libc::c_int) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code))
    }
}

pub(super) fn standard_dup(r: &Redirect) -> bool {
    let source = match r.word.as_str() {
        "0" => 0,
        "1" => 1,
        "2" => 2,
        _ => return false,
    };
    r.target == Target::Duplicate && (0..=2).contains(&r.fd) && source != r.fd
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ScriptProcess {
    Child,
    Replace,
}

impl External {
    pub(super) fn spawn_with_dups(
        &self,
        redirects: &[Redirect],
        input: Option<&OwnedFd>,
        output: Option<&OwnedFd>,
    ) -> io::Result<libc::pid_t> {
        let mut actions = MaybeUninit::uninit();
        unsafe {
            check(libc::posix_spawn_file_actions_init(actions.as_mut_ptr()))?;
        }
        let result = (|| {
            let mut dup = |src: RawFd, dest: RawFd| unsafe {
                check(libc::posix_spawn_file_actions_adddup2(
                    actions.as_mut_ptr(),
                    src,
                    dest,
                ))
            };
            // The original pipe ends are close-on-exec and above fd 2.
            for (pipe, dest) in [(input, 0), (output, 1)] {
                if let Some(fd) = pipe {
                    dup(fd.as_raw_fd(), dest)?;
                }
            }
            for r in redirects {
                dup(r.word.parse().unwrap(), r.fd)?;
            }
            self.spawn_resolved(Some(&actions), None)
        })();
        unsafe {
            libc::posix_spawn_file_actions_destroy(actions.as_mut_ptr());
        }
        result
    }

    pub(super) fn spawn_resolved(
        &self,
        actions: Option<&MaybeUninit<libc::posix_spawn_file_actions_t>>,
        session: Option<&crate::Session>,
    ) -> io::Result<libc::pid_t> {
        let path = self
            .search_paths
            .as_ref()
            .and_then(|paths| paths.first())
            .unwrap_or(&self.words[0]);
        let mut attr = MaybeUninit::uninit();
        unsafe {
            check(libc::posix_spawnattr_init(attr.as_mut_ptr()))?;
            let result = (|| {
                let mut signals = std::mem::zeroed();
                libc::sigemptyset(&mut signals);
                if !self.ignore_pipe {
                    libc::sigaddset(&mut signals, libc::SIGPIPE);
                }
                check(libc::posix_spawnattr_setsigdefault(
                    attr.as_mut_ptr(),
                    &signals,
                ))?;
                check(libc::posix_spawnattr_setflags(
                    attr.as_mut_ptr(),
                    libc::POSIX_SPAWN_SETSIGDEF as _,
                ))?;
                let env = self
                    .env
                    .as_ref()
                    .map_or_else(inherited_env, |e| e.pointers.as_ptr().cast());
                let spawn = |path: &std::ffi::CStr,
                             argv: &[*const libc::c_char]|
                 -> io::Result<libc::pid_t> {
                    let mut pid = 0;
                    check(libc::posix_spawn(
                        &mut pid,
                        path.as_ptr(),
                        actions.map_or(std::ptr::null(), MaybeUninit::as_ptr),
                        attr.as_ptr(),
                        argv.as_ptr().cast(),
                        env,
                    ))?;
                    Ok(pid)
                };
                match spawn(path, &self.argv) {
                    Err(e) if e.raw_os_error() == Some(libc::ENOEXEC) => {
                        self.with_script(path, |shell, args| {
                            if let Some(session) = session
                                && let Some(pid) = self.start_script(
                                    session,
                                    path,
                                    shell,
                                    args,
                                    env.cast(),
                                    ScriptProcess::Child,
                                )
                            {
                                return Ok(pid);
                            }
                            spawn(shell, args)
                        })
                    }
                    result => result,
                }
            })();
            libc::posix_spawnattr_destroy(attr.as_mut_ptr());
            result
        }
    }
    // A plain ENOEXEC command can reuse its process image for a fresh interpreter.
    // Use exec for managed jobs, changed environments and inherited runtime
    // state that needs an actual image transition.
    pub(super) fn start_script(
        &self,
        session: &crate::Session,
        path: &std::ffi::CStr,
        shell: &std::ffi::CStr,
        args: &[*const libc::c_char],
        env: *const *const libc::c_char,
        process: ScriptProcess,
    ) -> Option<libc::pid_t> {
        use std::ffi::{OsStr, OsString};
        use std::io::Write;
        use std::os::unix::ffi::OsStrExt;

        thread_local! {
            static REENTERED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }
        // A nested fallback must release the previous interpreter stack,
        // including when a child would inherit that stack through fork.
        if REENTERED.get() {
            return None;
        }
        if session.interactive
            || session.traps.active()
            || session.traps.in_action()
            || crate::editor::preview::active()
        {
            return None;
        }
        let mut shell_level = None;
        for (name, value) in session.environment_changes(&[]) {
            if name != "SHLVL" {
                return None;
            }
            shell_level = Some(value);
        }
        let _ = std::io::stdout().flush();
        if process == ScriptProcess::Child {
            let pid = unsafe { libc::fork() };
            if pid != 0 {
                return (pid > 0).then_some(pid);
            }
        }
        if has_close_on_exec_fds() {
            if process == ScriptProcess::Replace {
                return None;
            }
            unsafe {
                libc::execve(shell.as_ptr(), args.as_ptr(), env);
                libc::_exit(126);
            }
        }
        REENTERED.set(true);
        if process == ScriptProcess::Replace {
            // exec() had restored the external command's default disposition.
            // A newly entered Rust main ignores SIGPIPE.
            unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
        }
        // The interpreter is single-threaded. Apply exactly the exported
        // change, including export -n, before starting the ordinary script entry point.
        match shell_level {
            Some(Some(value)) => unsafe { std::env::set_var("SHLVL", value.as_ref()) },
            Some(None) => unsafe { std::env::remove_var("SHLVL") },
            None => (),
        }
        let argv = [
            OsString::from("tonari"),
            OsString::from("--"),
            OsStr::from_bytes(path.to_bytes()).into(),
        ]
        .into_iter()
        .chain(
            self.words[1..]
                .iter()
                .map(|w| OsStr::from_bytes(w.as_bytes()).to_owned()),
        );
        let status = crate::invoke(argv);
        let _ = std::io::stdout().flush();
        unsafe { libc::_exit(i32::from(status)) }
    }
}

fn has_close_on_exec_fds() -> bool {
    // Drop the directory iterator before inspecting the numbers: it creates
    // a close-on-exec fd of its own. No descriptors open between the two steps.
    let Ok(entries) = std::fs::read_dir("/dev/fd") else {
        return true;
    };
    let mut fds = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { return true };
        let Some(fd) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<RawFd>().ok())
        else {
            continue;
        };
        fds.push(fd);
    }
    fds.into_iter().any(|fd| {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 {
            io::Error::last_os_error().raw_os_error() != Some(libc::EBADF)
        } else {
            flags & libc::FD_CLOEXEC != 0
        }
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn inherited_env() -> *const *mut libc::c_char {
    unsafe { *libc::_NSGetEnviron() }
}

#[cfg(target_os = "linux")]
pub(crate) fn inherited_env() -> *const *mut libc::c_char {
    unsafe extern "C" {
        static environ: *mut *mut libc::c_char;
    }
    unsafe { environ }
}
