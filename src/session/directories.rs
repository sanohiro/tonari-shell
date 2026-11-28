use super::Session;
use super::{Component, DirectoryError, MetadataExt, OsStrExt, PathBuf, Rc};
use std::io::{self, Write};
use std::path::Path;

pub(super) fn physical_path() -> io::Result<std::path::PathBuf> {
    with_physical_path(|path| Ok(path.to_owned()))
}

fn with_physical_path<T>(use_path: impl FnOnce(&Path) -> io::Result<T>) -> io::Result<T> {
    #[cfg(target_os = "macos")]
    if let Some(path) = vnode_cwd(&mut [0; 12 + libc::PATH_MAX as usize]) {
        return use_path(path);
    }
    // Avoid getcwd(NULL, 0)'s growing allocation for ordinary paths.
    let mut path = [std::mem::MaybeUninit::<libc::c_char>::uninit(); 512];
    let result = unsafe { libc::getcwd(path.as_mut_ptr().cast(), path.len()) };
    if !result.is_null() {
        let bytes = unsafe { std::ffi::CStr::from_ptr(result) }.to_bytes();
        return use_path(Path::new(std::ffi::OsStr::from_bytes(bytes)));
    }
    let error = io::Error::last_os_error();
    if !matches!(
        error.raw_os_error(),
        Some(libc::ERANGE | libc::ENAMETOOLONG)
    ) {
        return Err(error);
    }
    let path = match std::env::current_dir() {
        Err(e) if e.raw_os_error() == Some(libc::ENAMETOOLONG) => long_physical_path(),
        result => result,
    }?;
    use_path(&path)
}

// Darwin can resolve the current vnode's physical path in one attribute query.
// Validate the returned name before printing it: the cwd may be unlinked or
// replaced, and volumes can truncate or omit the variable-length attribute.
#[cfg(target_os = "macos")]
fn vnode_cwd(buf: &mut [u8]) -> Option<&Path> {
    let mut attrs = unsafe { std::mem::zeroed::<libc::attrlist>() };
    attrs.bitmapcount = libc::ATTR_BIT_MAP_COUNT as _;
    attrs.commonattr = libc::ATTR_CMN_FULLPATH;
    let status = unsafe {
        libc::getattrlist(
            c".".as_ptr(),
            (&raw mut attrs).cast(),
            buf.as_mut_ptr().cast(),
            buf.len(),
            libc::FSOPT_REPORT_FULLSIZE,
        )
    };
    if status != 0 {
        return None;
    }
    // The packed result is a u32 length followed by an attrreference_t.
    let word = |offset| {
        buf.get(offset..offset + 4)?
            .try_into()
            .ok()
            .map(u32::from_ne_bytes)
    };
    let returned = word(0)? as usize;
    let start = 4usize.checked_add(word(4)? as usize)?;
    let end = start.checked_add(word(8)? as usize)?;
    let bytes = buf.get(..returned)?.get(start..end)?.strip_suffix(&[0])?;
    let path = Path::new(std::ffi::OsStr::from_bytes(bytes));
    (path.is_absolute() && same_directory(path, Path::new("."))).then_some(path)
}

// Validate a cached absolute name component by component when it will not fit
// in a single pathname argument. Physical pwd must reject every symbolic link.
pub(super) fn long_cwd_matches(path: &Path, physical: bool) -> bool {
    use std::fs::File;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    let Ok(mut directory) = File::open("/") else {
        return false;
    };
    let mut bytes = Vec::new();
    let flags = libc::O_DIRECTORY
        | libc::O_RDONLY
        | libc::O_CLOEXEC
        | if physical { libc::O_NOFOLLOW } else { 0 };
    for part in path.components() {
        let std::path::Component::Normal(name) = part else {
            continue;
        };
        bytes.clear();
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        let fd = unsafe { libc::openat(directory.as_raw_fd(), bytes.as_ptr().cast(), flags) };
        if fd < 0 {
            return false;
        }
        directory = unsafe { File::from_raw_fd(fd) };
    }
    match (directory.metadata(), std::fs::metadata(".")) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

// Some getcwd implementations cannot return a name longer than PATH_MAX.
// Walk open directories without changing the shell's working directory.
#[cold]
fn long_physical_path() -> io::Result<std::path::PathBuf> {
    use std::ffi::{CStr, OsStr};
    use std::fs::File;
    use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    let mut child = File::open(".")?;
    let mut here = child.metadata()?;
    let mut parts = Vec::new();
    loop {
        let fd = unsafe {
            libc::openat(
                child.as_raw_fd(),
                c"..".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let parent = unsafe { File::from_raw_fd(fd) };
        let up = parent.metadata()?;
        if (here.dev(), here.ino()) == (up.dev(), up.ino()) {
            break;
        }
        let reader = parent.try_clone()?;
        let stream = unsafe { libc::fdopendir(reader.as_raw_fd()) };
        if stream.is_null() {
            return Err(io::Error::last_os_error());
        }
        let _ = reader.into_raw_fd();
        let found = loop {
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                break None;
            }
            if !matches!(unsafe { (*entry).d_type }, libc::DT_DIR | libc::DT_UNKNOWN) {
                continue;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            if matches!(name.to_bytes(), b"." | b"..") {
                continue;
            }
            let mut st = std::mem::MaybeUninit::uninit();
            if unsafe {
                libc::fstatat(
                    fd,
                    name.as_ptr(),
                    st.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } == 0
            {
                let st = unsafe { st.assume_init() };
                #[allow(
                    clippy::unnecessary_cast,
                    reason = "dev_t is i32 on Darwin and u64 on Linux; MetadataExt::dev returns u64"
                )]
                let device = st.st_dev as u64;
                if device == here.dev() && st.st_ino == here.ino() {
                    break Some(OsStr::from_bytes(name.to_bytes()).to_owned());
                }
            }
        };
        unsafe { libc::closedir(stream) };
        parts.push(found.ok_or_else(|| io::Error::from_raw_os_error(libc::ENOENT))?);
        child = parent;
        here = up;
    }
    let mut path = std::path::PathBuf::from("/");
    path.extend(parts.into_iter().rev());
    Ok(path)
}

fn index(word: &str, len: usize) -> Option<usize> {
    let n = word.get(1..)?.parse::<usize>().ok()?;
    if n >= len {
        return None;
    }
    match word.as_bytes()[0] {
        b'+' => Some(n),
        b'-' => Some(len - 1 - n),
        _ => None,
    }
}

impl Session {
    fn stack_path(&self, i: usize) -> Option<&str> {
        if i == 0 {
            return self.logical_directory_ref().ok()?.to_str();
        }
        self.dir_stack
            .len()
            .checked_sub(i)
            .map(|n| self.dir_stack[n].as_str())
    }

    pub(super) fn stack_entry(&self, i: usize) -> Option<String> {
        self.stack_path(i).map(str::to_owned)
    }

    fn show_stack(
        &self,
        out: &mut dyn Write,
        only: Option<usize>,
        long: bool,
        vertical: bool,
        numbered: bool,
    ) -> u8 {
        let home = (!long).then(|| self.get("HOME")).flatten();
        let first = only.unwrap_or(0);
        let last = only.unwrap_or(self.dir_stack.len());
        for i in first..=last {
            let Some(path) = self.stack_path(i) else {
                return 1;
            };
            let name = home
                .as_deref()
                .filter(|h| !h.is_empty() && *h != "/")
                .and_then(|h| path.strip_prefix(h))
                .filter(|s| s.is_empty() || s.starts_with('/'));
            let result = (|| {
                if numbered {
                    if i < 10 {
                        out.write_all(b" ")?;
                    }
                    out.write_all(itoa::Buffer::new().format(i).as_bytes())?;
                    out.write_all(b"  ")?;
                }
                if let Some(tail) = name {
                    out.write_all(b"~")?;
                    out.write_all(tail.as_bytes())?;
                } else {
                    out.write_all(path.as_bytes())?;
                }
                out.write_all(if vertical || numbered || i == last {
                    b"\n"
                } else {
                    b" "
                })
            })();
            if result.is_err() {
                return 1;
            }
        }
        u8::from(out.flush().is_err())
    }

    pub fn directory_stack(&mut self, command: &str, args: &[String], out: &mut dyn Write) -> u8 {
        #[derive(Clone, Copy, PartialEq)]
        enum StackOp { List, Push, Pop }
        let op = match command {
            "dirs" => StackOp::List, "pushd" => StackOp::Push, "popd" => StackOp::Pop,
            _ => unreachable!("directory stack builtin dispatch"),
        };
        let (mut long, mut vertical, mut numbered, mut clear, mut no_cd) =
            (false, false, false, false, false);
        let mut args = args.iter().peekable();
        while let Some(arg) =
            args.next_if(|a| a.starts_with('-') && a.len() > 1 && !a.as_bytes()[1].is_ascii_digit())
        {
            if arg == "--" {
                break;
            }
            for flag in arg[1..].chars() {
                match (op, flag) {
                    (StackOp::List, 'l') => long = true,
                    (StackOp::List, 'p') => vertical = true,
                    (StackOp::List, 'v') => numbered = true,
                    (StackOp::List, 'c') => clear = true,
                    (StackOp::Push | StackOp::Pop, 'n') => no_cd = true,
                    _ => {
                        diagnostic!("{command}: invalid option -{flag}");
                        return 2;
                    }
                }
            }
        }
        let operand = args.next().map(String::as_str);
        if args.next().is_some() {
            diagnostic!("{command}: too many arguments");
            return 2;
        }
        if clear {
            self.dir_stack.clear();
            return 0;
        }
        let len = 1 + self.dir_stack.len();
        let slot = match operand {
            Some(text)
                if text.len() > 1
                    && text.starts_with(['-', '+'])
                    && text.as_bytes()[1..].iter().all(u8::is_ascii_digit) =>
            {
                let Some(i) = index(text, len) else {
                    diagnostic!("{command}: {text}: directory stack index out of range");
                    return 1;
                };
                Some(i)
            }
            Some(text) if op != StackOp::Push => {
                diagnostic!("{command}: {text}: invalid option");
                let usage = if op == StackOp::Pop { "popd [-n] [+N | -N]" } else { "dirs [-clpv] [+N] [-N]" };
                stderr_line!("{command}: usage: {usage}");
                return 2;
            }
            _ => None,
        };
        if op == StackOp::List {
            return self.show_stack(out, slot, long, vertical, numbered);
        }
        let result = (|| -> io::Result<()> {
            if op == StackOp::Pop {
                let i = slot.unwrap_or(0);
                if self.dir_stack.is_empty() {
                    return Err(io::Error::other("directory stack empty"));
                }
                if i == 0 {
                    if !no_cd {
                        let target = self.dir_stack.last().unwrap().clone();
                        self.change_directory(Path::new(&target), self.physical, false, true)?;
                    }
                    self.dir_stack.pop();
                } else {
                    self.dir_stack.remove(len - 1 - i);
                }
            } else if let Some(i) = slot {
                let cwd = self
                    .logical_directory()?
                    .into_os_string()
                    .into_string()
                    .map_err(|_| io::Error::other("expected UTF-8 directory name"))?;
                let mut paths: Vec<_> = std::iter::once(cwd)
                    .chain(self.dir_stack.iter().rev().cloned())
                    .collect();
                paths.rotate_left(i);
                if !no_cd {
                    self.change_directory(Path::new(&paths[0]), self.physical, false, true)?;
                }
                self.dir_stack = paths.into_iter().skip(1).rev().collect();
            } else if let Some(dir) = operand {
                if no_cd {
                    self.dir_stack.push(dir.to_owned());
                } else {
                    let cwd = self
                        .stack_entry(0)
                        .ok_or_else(|| io::Error::other("cannot read current directory"))?;
                    if dir == "-" {
                        let old = self.get("OLDPWD").ok_or_else(|| io::Error::other("OLDPWD not set"))?;
                        self.change_directory(Path::new(&old), self.physical, true, true)?;
                    } else {
                        self.change_directory(Path::new(dir), self.physical, false, true)?;
                    }
                    self.dir_stack.push(cwd);
                }
            } else if !no_cd {
                let cwd = self
                    .stack_entry(0)
                    .ok_or_else(|| io::Error::other("cannot read current directory"))?;
                let next = self
                    .dir_stack
                    .last()
                    .ok_or_else(|| io::Error::other("no other directory"))?
                    .clone();
                self.change_directory(Path::new(&next), self.physical, false, true)?;
                *self.dir_stack.last_mut().unwrap() = cwd;
            }
            Ok(())
        })();
        if let Err(e) = result {
            diagnostic!("{command}: {e}");
            return 1;
        }
        if op == StackOp::Push && no_cd && (operand.is_none() || slot.is_some()) {
            return 0;
        }
        self.show_stack(out, None, false, false, false)
    }
}

impl Session {
    pub(crate) fn logical_directory_ref(&self) -> io::Result<&Path> {
        if let Some(cwd) = self.logical_cwd.get() {
            return Ok(cwd);
        }
        if let Some(pwd) = std::env::var_os("PWD") {
            let path = Path::new(&pwd);
            if path.is_absolute()
                && !path.components().any(|c| c == Component::ParentDir)
                && same_directory(path, Path::new("."))
            {
                return Ok(self.logical_cwd.get_or_init(|| path.into()));
            }
        }
        let cwd = physical_path()?;
        Ok(self.logical_cwd.get_or_init(|| cwd))
    }

    pub fn logical_directory(&self) -> io::Result<PathBuf> {
        self.logical_directory_ref().map(Path::to_owned)
    }

    pub(crate) fn capture_pwd(&mut self, physical: bool, out: &mut dyn Write) -> io::Result<()> {
        let initialized = self.logical_cwd.get().is_some();
        let result = self.write_pwd(physical, out);
        // A substitution must not establish the parent's logical directory.
        if !initialized {
            self.logical_cwd.take();
        }
        result
    }

    pub(crate) fn write_pwd(&mut self, physical: bool, out: &mut dyn Write) -> io::Result<()> {
        if !physical && !self.posix {
            return write_path(self.logical_directory_ref()?, out);
        }
        if physical {
            if let Some(path) = self.logical_cwd.get()
                && path.as_os_str().len() >= libc::PATH_MAX as usize
                && long_cwd_matches(path, true)
            {
                return write_path(path, out);
            }
            return with_physical_path(|path| write_path(path, out));
        }
        let path = {
            self.logical_directory().and_then(|path| {
                if same_directory(&path, Path::new("."))
                    || path.as_os_str().len() >= libc::PATH_MAX as usize
                        && long_cwd_matches(&path, false)
                {
                    Ok(path)
                } else {
                    let actual = physical_path()?;
                    self.logical_cwd = std::cell::OnceCell::from(actual.clone());
                    Ok(actual)
                }
            })
        }?;
        write_path(&path, out)
    }

    pub fn directory_builtin(&mut self, name: &str, args: &[String]) -> u8 {
        let mut physical = self.physical;
        let mut require_pwd = false;
        let mut args = args;
        while let Some(opt) = crate::builtins::flags::next(&mut args) {
            for flag in opt[1..].chars() {
                match flag {
                    'L' => physical = false,
                    'P' => physical = true,
                    'e' if name == "cd" => require_pwd = true,
                    _ => {
                        diagnostic!("{name}: unsupported option {opt}");
                        return 2;
                    }
                }
            }
        }
        let result = if name == "pwd" {
            self.write_pwd(physical, &mut crate::builtins::StdoutFd(None))
        } else {
            if args.len() > 1 {
                diagnostic!("cd: too many arguments");
                return 1;
            }
            let arg = args.first().map(String::as_str);
            let (target, previous) = match arg {
                None => (self.get("HOME"), false),
                Some("-") => (self.get("OLDPWD"), true),
                Some(s) => (Some(s.into()), false),
            };
            let Some(target) = target else {
                diagnostic!("cd: {} not set", if previous { "OLDPWD" } else { "HOME" });
                return 1;
            };
            if target.is_empty() {
                diagnostic!("cd: null directory");
                return 1;
            }
            if self.posix && physical {
                match self.try_change_directory(
                    Path::new(&target),
                    physical,
                    previous,
                    arg.is_some(),
                ) {
                    Ok(()) => return 0,
                    Err(DirectoryError::NoPath(e)) => {
                        if require_pwd {
                            diagnostic!("cd: {e}");
                            return 1;
                        }
                        return 0;
                    }
                    Err(DirectoryError::Other(e)) => {
                        diagnostic!("cd: {e}");
                        return if require_pwd { 2 } else { 1 };
                    }
                }
            }
            self.change_directory(Path::new(&target), physical, previous, arg.is_some())
        };
        result.map_or_else(
            |e| {
                diagnostic!("{name}: {e}");
                1
            },
            |()| 0,
        )
    }

    pub(crate) fn revisit_directory(&mut self, forward: bool) -> io::Result<bool> {
        let Some(visits) = &mut self.visits else {
            return Ok(false);
        };
        let mut index = visits.current;
        while let Some(next) = index.checked_add_signed(if forward { 1 } else { -1 }) {
            let Some(path) = visits.paths.get(next) else {
                break;
            };
            // Visits already contain normalized absolute paths. Resolve them once in the kernel.
            match std::env::set_current_dir(path) {
                Ok(()) => {
                    let cwd = path.clone();
                    Rc::make_mut(visits).current = next;
                    let old_pwd = self.get("PWD");
                    self.hashes.directory_changed();
                    self.finish_directory_change(cwd, old_pwd, false)?;
                    return Ok(true);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) =>
                {
                    index = next
                }
                Err(e) => return Err(io::Error::new(e.kind(), format!("{}: {e}", path.display()))),
            }
        }
        Ok(false)
    }

    pub(super) fn change_directory(
        &mut self,
        target: &Path,
        physical: bool,
        previous: bool,
        explicit: bool,
    ) -> io::Result<()> {
        self.try_change_directory(target, physical, previous, explicit)
            .map_err(|e| match e {
                DirectoryError::NoPath(e) | DirectoryError::Other(e) => e,
            })
    }

    fn try_change_directory(
        &mut self,
        target: &Path,
        physical: bool,
        previous: bool,
        explicit: bool,
    ) -> Result<(), DirectoryError> {
        let old_pwd = self.get("PWD");
        let base = self.logical_directory();
        let cdpath = if explicit
            && !previous
            && matches!(target.components().next(), Some(Component::Normal(_)))
        {
            self.get("CDPATH")
        } else {
            None
        };
        let mut paths = cdpath
            .iter()
            .flat_map(std::env::split_paths)
            .map(|dir| {
                let print = !dir.as_os_str().is_empty();
                (dir.join(target), print)
            })
            .chain(std::iter::once_with(|| (target.to_owned(), previous)));
        let mut variable = target
            .to_str()
            .filter(|_| explicit && !previous && self.shopt.cdable_vars);
        let mut spell = self.interactive && self.shopt.cdspell && explicit && !previous;
        let mut error = io::Error::from(io::ErrorKind::NotFound);
        loop {
            let (candidate, print, via_variable) = if let Some((path, print)) = paths.next() {
                (path, print, false)
            } else if let Some(value) = variable.take().and_then(|name| self.get(name)) {
                (PathBuf::from(value), false, true)
            } else if std::mem::take(&mut spell)
                && let Some(path) = crate::complete::corrected_directory(target)
            {
                (path, false, true)
            } else {
                break;
            };
            if candidate.as_os_str().is_empty() {
                continue;
            }
            let logical = if physical {
                None
            } else {
                let joined = if candidate.is_absolute() {
                    Some(candidate.clone())
                } else {
                    base.as_ref().ok().map(|base| base.join(&candidate))
                };
                joined
                    .as_deref()
                    .and_then(|path| normalize_directory(path).ok())
            };
            // A renamed ancestor can invalidate the logical path while the
            // process still has a valid current directory for a relative chdir.
            let logical = match logical {
                Some(path) => match std::env::set_current_dir(&path) {
                    Ok(()) => Some(path),
                    Err(e)
                        if e.raw_os_error() == Some(libc::ENAMETOOLONG)
                            && candidate.is_relative() =>
                    {
                        // The full name exceeded the kernel argument limit. Relative
                        // chdir still establishes this logical name, including symlinks.
                        if let Err(err) = std::env::set_current_dir(&candidate) {
                            error = err;
                            continue;
                        }
                        Some(path)
                    }
                    Err(e) if self.posix => {
                        error = e;
                        continue;
                    }
                    Err(_) => None,
                },
                None => None,
            };
            if logical.is_none()
                && let Err(e) = std::env::set_current_dir(&candidate)
            {
                error = e;
                continue;
            }
            let cwd = match logical {
                Some(cwd) => cwd,
                None => match physical_path().and_then(|path| {
                    path.to_str().map(|_| ()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 directory"))?;
                    Ok(path)
                }) {
                    Ok(path) => path,
                    Err(e) => {
                        // Physical names outside Tonari's text domain are rare.
                        // Recover through the previous logical path on this branch
                        // only. If an ancestor was also renamed, forget the stale
                        // name and retain OLDPWD for explicit recovery.
                        if e.kind() == io::ErrorKind::InvalidData && base.as_ref()
                            .is_ok_and(|old| std::env::set_current_dir(old).is_ok())
                        {
                            self.hashes.directory_changed();
                            return Err(DirectoryError::Other(e));
                        }
                        self.hashes.directory_changed();
                        self.logical_cwd.take();
                        if let Some(old) = &old_pwd { self.assign("OLDPWD", old); }
                        self.unset_variable("PWD");
                        return Err(DirectoryError::NoPath(e));
                    }
                },
            };
            self.hashes.directory_changed();
            self.logical_cwd.take();
            if self.interactive && base.as_deref().ok() != Some(cwd.as_path()) {
                Rc::make_mut(self.visits.get_or_insert_with(Rc::default)).record(base.ok(), &cwd);
            }
            if via_variable {
                print_path(&candidate)?;
            }
            return self
                .finish_directory_change(cwd, old_pwd, print)
                .map_err(DirectoryError::Other);
        }
        Err(DirectoryError::Other(io::Error::new(
            error.kind(),
            format!("{}: {error}", target.display()),
        )))
    }

    fn finish_directory_change(
        &mut self,
        cwd: PathBuf,
        old_pwd: Option<String>,
        print: bool,
    ) -> io::Result<()> {
        self.logical_cwd = std::cell::OnceCell::from(cwd.clone());
        let old_readonly = self.readonly("OLDPWD");
        if !old_readonly {
            if let Some(old) = old_pwd {
                self.assign("OLDPWD", &old);
            } else {
                let old = self.variables.entry("OLDPWD".into()).or_default();
                old.declared = true;
                *old.scalar_mut() = None;
            }
        }
        let pwd_readonly = self.readonly("PWD");
        self.assign(
            "PWD",
            cwd.to_str()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 directory"))?,
        );
        if print {
            print_path(&cwd)?;
        }
        if old_readonly || pwd_readonly {
            return Err(io::Error::other(if old_readonly {
                "OLDPWD: readonly variable"
            } else {
                "PWD: readonly variable"
            }));
        }
        Ok(())
    }
}

fn same_directory(left: &Path, right: &Path) -> bool {
    match (std::fs::metadata(left), std::fs::metadata(right)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

fn normalize_directory(path: &Path) -> io::Result<PathBuf> {
    let mut result = PathBuf::new();
    for component in path.components() {
        if component == Component::ParentDir {
            if !std::fs::metadata(&result)?.is_dir() {
                return Err(io::Error::from_raw_os_error(libc::ENOTDIR));
            }
            result.pop();
        } else {
            result.push(component);
        }
    }
    Ok(result)
}

fn print_path(path: &Path) -> io::Result<()> {
    write_path(
        path,
        &mut io::LineWriter::new(crate::builtins::StdoutFd(None)),
    )
}

fn write_path(path: &Path, out: &mut dyn Write) -> io::Result<()> {
    let bytes = path.as_os_str().as_bytes();
    let mut line = [0; 512];
    if bytes.len() < line.len() {
        line[..bytes.len()].copy_from_slice(bytes);
        line[bytes.len()] = b'\n';
        out.write_all(&line[..=bytes.len()])?;
    } else {
        out.write_all(bytes)?;
        out.write_all(b"\n")?;
    }
    out.flush()
}
