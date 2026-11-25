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
    if r.target != Target::Duplicate || !(0..=2).contains(&r.fd) {
        return false;
    }
    match r.word.as_str() {
        "0" => r.fd != 0,
        "1" => r.fd != 1,
        "2" => r.fd != 2,
        number => number.parse::<RawFd>().is_ok_and(|fd| {
            // A script may keep its log open on fd 5 or another inherited fd.
            // Close-on-exec descriptors can belong to a pipe or the script reader.
            fd > 2 && unsafe { libc::fcntl(fd, libc::F_GETFD) } == 0
        }),
    }
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
            self.spawn_resolved(Some(&actions), None, Default::default())
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
        launch: crate::jobs::Launch,
    ) -> io::Result<libc::pid_t> {
        let path = self
            .search_paths
            .as_ref()
            .and_then(|paths| paths.first())
            .unwrap_or(&self.words[0]);
        let spawn_with = |attributes| unsafe {
            let env = self
                .env
                .as_ref()
                .map_or_else(inherited_env, |e| e.pointers.as_ptr().cast());
            let spawn =
                |path: &std::ffi::CStr, argv: &[*const libc::c_char]| -> io::Result<libc::pid_t> {
                    let mut pid = 0;
                    check(libc::posix_spawn(
                        &mut pid,
                        path.as_ptr(),
                        actions.map_or(std::ptr::null(), MaybeUninit::as_ptr),
                        attributes,
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
        };
        let Some(group) = launch.group else {
            return spawn_with(std::ptr::null());
        };
        let mut attr = MaybeUninit::uninit();
        unsafe {
            check(libc::posix_spawnattr_init(attr.as_mut_ptr()))?;
            let result = (|| {
                check(libc::posix_spawnattr_setpgroup(attr.as_mut_ptr(), group))?;
                let flags = libc::POSIX_SPAWN_SETPGROUP;
                #[cfg(target_os = "macos")]
                let flags = if launch.spawn_terminal().is_some() {
                    flags | libc::POSIX_SPAWN_START_SUSPENDED
                } else {
                    flags
                };
                check(libc::posix_spawnattr_setflags(
                    attr.as_mut_ptr(),
                    flags as _,
                ))?;
                spawn_with(attr.as_ptr())
            })();
            libc::posix_spawnattr_destroy(attr.as_mut_ptr());
            result
        }
    }
    // A plain ENOEXEC command can reuse its process image for a fresh interpreter.
    // Use exec for managed jobs, other environment changes and inherited runtime
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
        // Both callers supply the session only for commands without prefix
        // assignments. Their prepared export delta is the child's environment.
        let changes = self
            .env
            .as_ref()
            .map_or(&[][..], |env| env.changes.as_slice());
        if changes
            .iter()
            .any(|(name, _)| !matches!(name.as_str(), "SHLVL" | "PATH" | "PWD" | "OLDPWD"))
        {
            return None;
        }
        let _ = std::io::stdout().flush();
        if process == ScriptProcess::Child {
            let pid = unsafe { libc::fork() };
            if pid != 0 {
                return (pid > 0).then_some(pid);
            }
            super::fd::close_in_script_child();
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
        // These variables are read by the fresh interpreter. Other changes
        // may need the dynamic loader or a new native runtime.
        for (name, value) in changes {
            match value {
                Some(value) => unsafe { std::env::set_var(name, value) },
                None => unsafe { std::env::remove_var(name) },
            }
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
