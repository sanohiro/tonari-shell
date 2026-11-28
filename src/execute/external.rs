use super::*;

pub(crate) struct Environment {
    // Commands without environment changes use execv and inherit environ.
    _strings: Vec<CString>,
    pub(super) pointers: Vec<*const libc::c_char>,
    pub(super) changes: Vec<(String, Option<String>)>,
}

impl Environment {
    pub(super) fn prepare(
        session: &Session,
        assignments: &[(String, String)],
    ) -> io::Result<Option<Rc<Self>>> {
        let mut live = session.environment_changes(assignments).peekable();
        if live.peek().is_none() {
            return Ok(None);
        }
        if assignments.is_empty()
            && let Some(cached) = &*session.external_env.borrow()
        {
            let mut entries = cached.changes.iter();
            if live.by_ref().all(|(key, value)| {
                entries
                    .next()
                    .is_some_and(|(k, v)| key == k && value.as_deref() == v.as_deref())
            }) && entries.next().is_none()
            {
                return Ok(Some(cached.clone()));
            }
            live = session.environment_changes(assignments).peekable();
        }
        let mut cached_changes = Vec::new();
        let changes: std::collections::BTreeMap<_, _> = live
            .map(|(k, v)| {
                if assignments.is_empty() {
                    cached_changes.push((k.to_owned(), v.as_deref().map(str::to_owned)));
                }
                (k, v)
            })
            .collect();
        // SAFETY: ordinary assignments live in Session. Script reentry replaces
        // process variables only after it has abandoned this environment and
        // never returns. Each inherited entry is a NUL-terminated C string.
        let mut pointers = Vec::new();
        let mut inherited = inherited_env();
        unsafe {
            while !(*inherited).is_null() {
                let entry = *inherited;
                let bytes = CStr::from_ptr(entry).to_bytes();
                let key = bytes.split(|&b| b == b'=').next().unwrap_or_default();
                if !std::str::from_utf8(key).is_ok_and(|name| changes.contains_key(name)) {
                    pointers.push(entry.cast_const());
                }
                inherited = inherited.add(1);
            }
        }
        let mut strings = Vec::with_capacity(changes.len());
        for (name, value) in &changes {
            if let Some(value) = value {
                let mut entry = Vec::with_capacity(name.len() + value.len() + 2);
                entry.extend_from_slice(name.as_bytes());
                entry.push(b'=');
                entry.extend_from_slice(value.as_bytes());
                let entry = CString::new(entry)?;
                pointers.push(entry.as_ptr());
                strings.push(entry);
            }
        }
        pointers.push(std::ptr::null());
        let env = Rc::new(Self {
            _strings: strings,
            pointers,
            changes: cached_changes,
        });
        if assignments.is_empty() {
            *session.external_env.borrow_mut() = Some(env.clone());
        }
        Ok(Some(env))
    }
}

pub(super) struct External {
    pub(super) program: CString,
    _arg0: Option<CString>,
    // Fully built before taking any pointers, then immutable through spawn/exec.
    pub(super) argument_bytes: Vec<u8>,
    pub(super) argv: Vec<*const libc::c_char>,
    pub(super) search_paths: Option<Vec<CString>>,
    pub(super) env: Option<Rc<Environment>>,
}

impl External {
    pub(super) fn new(cmd: &PreparedCommand, session: &Session) -> io::Result<Self> {
        Self::for_words(cmd, session, &cmd.words, None, false)
    }
    pub(super) fn for_words(
        cmd: &PreparedCommand,
        session: &Session,
        arguments: &[String],
        arg0: Option<String>,
        clear_env: bool,
    ) -> io::Result<Self> {
        let program = CString::new(arguments[0].as_bytes())?;
        let arg0 = arg0.map(CString::new).transpose()?;
        let tail = &arguments[1..];
        let mut argument_bytes = Vec::with_capacity(tail.iter().map(|w| w.len() + 1).sum());
        for word in tail {
            if word.as_bytes().contains(&0) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "data provided contains a nul byte",
                ));
            }
            argument_bytes.extend_from_slice(word.as_bytes());
            argument_bytes.push(0);
        }
        let mut argv = Vec::with_capacity(arguments.len() + 1);
        argv.push(arg0.as_ref().unwrap_or(&program).as_ptr());
        let mut offset = 0;
        for word in tail {
            argv.push(argument_bytes[offset..].as_ptr().cast());
            offset += word.len() + 1;
        }
        argv.push(std::ptr::null());
        let name = std::ffi::OsStr::from_bytes(program.as_bytes());
        let cached = cmd.cached_path(session, &arguments[0]);
        let search_paths = if let Some(path) = cached {
            Some(vec![CString::new(path.as_os_str().as_bytes())?])
        } else {
            (!arguments[0].contains('/'))
                .then(|| {
                    let path = lookup::path(cmd, session);
                    let ignored = IgnoredExecutables::new(session);
                    lookup::search_directories(&path)
                        .map(|dir| dir.join(name))
                        .filter(|path| !ignored.contains(path))
                        .map(|path| CString::new(path.into_os_string().into_encoded_bytes()))
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
        };
        Ok(Self {
            argument_bytes,
            argv,
            program,
            _arg0: arg0,
            search_paths,
            env: if clear_env {
                Some(Rc::new(Environment {
                    changes: Vec::new(),
                    _strings: Vec::new(),
                    pointers: vec![std::ptr::null()],
                }))
            } else {
                Environment::prepare(session, &cmd.assignments)?
            },
        })
    }

    pub(super) fn with_script<T>(
        &self,
        path: &CStr,
        launch: impl FnOnce(&CStr, &[*const libc::c_char]) -> io::Result<T>,
    ) -> io::Result<T> {
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;
        let path_os = std::ffi::OsStr::from_bytes(path.to_bytes());
        // The executable can be replaced between ENOEXEC and this open.
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path_os)?;
        let mut sample = [0u8; 128];
        let length = file.read(&mut sample)?;
        if !file.metadata()?.is_file() || sample[..length].contains(&0) {
            return Err(io::Error::from_raw_os_error(libc::ENOEXEC));
        }
        drop(file);
        let shell = CString::new(std::env::current_exe()?.as_os_str().as_bytes())?;
        let mut args = Vec::with_capacity(self.argv.len() + 2);
        args.extend([shell.as_ptr(), c"--".as_ptr(), path.as_ptr()]);
        args.extend_from_slice(&self.argv[1..]);
        launch(&shell, &args)
    }

    fn script_fallback(&self, path: &CStr, session: Option<&Session>) -> i32 {
        self.with_script(path, |shell, args| {
            let env = self.env.as_ref().map_or_else(
                || inherited_env().cast::<*const libc::c_char>(),
                |e| e.pointers.as_ptr(),
            );
            if let Some(session) = session {
                self.start_script(
                    session,
                    path,
                    shell,
                    args,
                    env,
                    spawn::ScriptProcess::Replace,
                );
            }
            unsafe { libc::execve(shell.as_ptr(), args.as_ptr(), env) };
            Err::<(), _>(io::Error::last_os_error())
        })
        .unwrap_err()
        .raw_os_error()
        .unwrap_or(libc::ENOEXEC)
    }

    pub(super) fn exec(&self, session: &Session, fallback_in_session: bool) -> u8 {
        let program = &self.program;
        if program.as_bytes().is_empty() {
            write_error(b"", b": command not found");
            return 127;
        }
        let mut error = libc::ENOENT;
        let paths = self
            .search_paths
            .as_deref()
            .unwrap_or(std::slice::from_ref(program));
        for path in paths {
            unsafe {
                if let Some(env) = &self.env {
                    libc::execve(path.as_ptr(), self.argv.as_ptr(), env.pointers.as_ptr());
                } else {
                    libc::execv(path.as_ptr(), self.argv.as_ptr());
                }
            }
            match errno() {
                libc::ENOENT | libc::ENOTDIR => continue,
                libc::ENOEXEC => {
                    error = self.script_fallback(path, fallback_in_session.then_some(session));
                    break;
                }
                libc::EACCES => error = libc::EACCES,
                e => {
                    error = e;
                    break;
                }
            }
        }
        let status = exec_status(error);
        let path = std::path::Path::new(std::ffi::OsStr::from_bytes(program.as_bytes()));
        let reason: &[u8] = match error {
            libc::ENOENT | libc::ENOTDIR if !program.as_bytes().contains(&b'/') => b": command not found",
            libc::ENOENT | libc::ENOTDIR => b": No such file or directory",
            libc::EISDIR => b": Is a directory",
            libc::EACCES if path.is_dir() => b": Is a directory",
            libc::EACCES | libc::EPERM => b": Permission denied",
            _ => b": cannot execute",
        };
        let prefix = session.diagnostic_prefix(|| session.line());
        write_diagnostic(prefix.as_deref().unwrap_or("tonari: ").as_bytes(), program.as_bytes(), reason, b"");
        status
    }
}
