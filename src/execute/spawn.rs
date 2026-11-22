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
            self.spawn_resolved(Some(&actions))
        })();
        unsafe {
            libc::posix_spawn_file_actions_destroy(actions.as_mut_ptr());
        }
        result
    }

    pub(super) fn spawn_resolved(
        &self,
        actions: Option<&MaybeUninit<libc::posix_spawn_file_actions_t>>,
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
                let mut pid = 0;
                check(libc::posix_spawn(
                    &mut pid,
                    path.as_ptr(),
                    actions.map_or(std::ptr::null(), MaybeUninit::as_ptr),
                    attr.as_ptr(),
                    self.argv.as_ptr().cast(),
                    env,
                ))?;
                Ok(pid)
            })();
            libc::posix_spawnattr_destroy(attr.as_mut_ptr());
            result
        }
    }
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
