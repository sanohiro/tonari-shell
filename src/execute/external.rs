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
    // Owns the argument strings referenced by argv.
    pub(super) diagnostic: Option<String>,
    pub(super) original_name: Option<CString>,
    pub(super) words: Vec<CString>,
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
        let mut words: Vec<_> = arguments
            .iter()
            .map(|w| CString::new(w.as_bytes()))
            .collect::<Result<_, _>>()?;
        let original = arg0.as_ref().map(|_| words[0].clone());
        if let Some(arg0) = arg0 {
            words[0] = CString::new(arg0)?;
        }
        let executable = original.as_ref().unwrap_or(&words[0]);
        let mut argv = Vec::with_capacity(words.len() + 1);
        argv.extend(words.iter().map(|w| w.as_ptr()));
        argv.push(std::ptr::null());
        let name = std::ffi::OsStr::from_bytes(executable.as_bytes());
        let cached = cmd.cached_path(session, &arguments[0]);
        let search_paths = if let Some(path) = cached {
            Some(vec![CString::new(path.as_os_str().as_bytes())?])
        } else {
            (!executable.as_bytes().contains(&b'/'))
                .then(|| {
                    let path = lookup::path(cmd, session);
                    let ignored = IgnoredExecutables::new(session);
                    lookup::search_directories(&path)
                        .map(|dir| dir.join(name))
                        .filter(|path| !ignored.contains(path))
                        .map(|path| CString::new(path.as_os_str().as_bytes()))
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
        };
        Ok(Self {
            words,
            argv,
            diagnostic: session.diagnostic_prefix(|| session.line()),
            original_name: original,
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

    pub(super) fn exec(&self, session: Option<&Session>) -> u8 {
        let program = self.original_name.as_ref().unwrap_or(&self.words[0]);
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
                    error = self.script_fallback(path, session);
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
        write_diagnostic(
            self.diagnostic.as_deref().unwrap_or("tonari: ").as_bytes(),
            program.as_bytes(),
            if status == 127 {
                b": command not found"
            } else {
                b": cannot execute"
            },
            b"",
        );
        status
    }
}
