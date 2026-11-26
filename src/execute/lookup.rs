use std::ffi::{CString, OsString};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::{PreparedCommand, named_builtin};
use crate::session::Session;

#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum Search {
    #[default]
    Normal,
    NoFunctions,
    BuiltinsOnly,
}

pub(super) fn path(cmd: &PreparedCommand, session: &Session) -> OsString {
    if cmd.default_path {
        let size = unsafe { libc::confstr(libc::_CS_PATH, std::ptr::null_mut(), 0) };
        if size > 0 {
            let mut bytes = vec![0u8; size];
            unsafe {
                libc::confstr(libc::_CS_PATH, bytes.as_mut_ptr().cast(), bytes.len());
            }
            bytes.pop();
            return std::os::unix::ffi::OsStringExt::from_vec(bytes);
        }
        "/bin:/usr/bin".into()
    } else {
        session.search_path(&cmd.assignments)
    }
}

pub(super) fn command_prefix(cmd: &mut PreparedCommand, session: &Session) {
    let mut passed = 0;
    while let Some(name) = cmd.words.get(passed) {
        if session.disabled_builtins.contains(name)
            || cmd.search == Search::Normal && session.functions.contains_key(name)
        {
            break;
        }
        match name.as_str() {
            "builtin" => {
                passed += 1;
                if cmd.words.get(passed).is_some_and(|s| s == "--") {
                    passed += 1;
                }
                cmd.search = Search::BuiltinsOnly;
            }
            "command" => {
                let mut i = passed + 1;
                let mut default_path = false;
                while let Some(opt) = cmd.words.get(i) {
                    if opt == "--" {
                        i += 1;
                        break;
                    }
                    if !opt.starts_with('-') || opt == "-" {
                        break;
                    }
                    // A query or usage error is handled by the builtin itself.
                    if !opt[1..].chars().all(|flag| flag == 'p') {
                        break;
                    }
                    default_path = true;
                    i += 1;
                }
                if cmd
                    .words
                    .get(i)
                    .is_some_and(|arg| arg.starts_with('-') && arg != "-")
                    && (i == passed + 1 || cmd.words[i - 1] != "--")
                {
                    break;
                }
                passed = i;
                cmd.search = Search::NoFunctions;
                cmd.default_path |= default_path;
            }
            _ => break,
        }
    }
    if passed != 0 {
        cmd.words.drain(..passed);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Verbose,
    Name,
    Kind,
    Path,
}

pub(super) fn describe(cmd: &PreparedCommand, session: &Session) -> u8 {
    let command = cmd.words[0] == "command";
    let mut format = if command {
        Format::Name
    } else {
        Format::Verbose
    };
    let mut search_all = false;
    let mut force_path = false;
    let mut skip_functions = false;
    let mut default_path = cmd.default_path;
    let mut index = 1;
    while let Some(opt) = cmd.words.get(index) {
        if opt == "--" {
            index += 1;
            break;
        }
        if !opt.starts_with('-') || opt == "-" {
            break;
        }
        for flag in opt[1..].chars() {
            match (command, flag) {
                (true, 'p') => default_path = true,
                (true, 'v') => format = Format::Name,
                (true, 'V') => format = Format::Verbose,
                (false, 't') => format = Format::Kind,
                (false, 'f') => skip_functions = true,
                (false, 'a') => search_all = true,
                (false, 'P') => {
                    force_path = true;
                    format = Format::Path;
                }
                (false, 'p') => format = Format::Path,
                _ => {
                    eprintln!("tonari: {}: unsupported option {opt}", cmd.words[0]);
                    return 2;
                }
            }
        }
        index += 1;
    }
    let search = std::cell::LazyCell::new(|| {
        if default_path {
            "/bin:/usr/bin".into()
        } else {
            session.search_path(&[])
        }
    });
    let mut out = Vec::new();
    let mut any = false;
    let mut all = true;
    for name in &cmd.words[index..] {
        let mut found = false;
        if !force_path
            && session.shopt.expand_aliases
            && let Some(value) = session.aliases.get(name)
        {
            let result = match format {
                Format::Verbose => writeln!(out, "{name} is aliased to `{value}'"),
                Format::Kind => writeln!(out, "alias"),
                Format::Name => writeln!(out, "{}", crate::alias::declaration(name, value)),
                Format::Path => Ok(()),
            };
            if result.is_err() {
                return 1;
            }
            found = true;
        }
        let keyword = crate::parser::reserved(name) || name == "]]";
        let special = session.posix
            && super::special_builtin(name)
            && !session.disabled_builtins.contains(name);
        for (present, kind, label) in [
            (keyword, "keyword", "a shell keyword"),
            (special, "builtin", "a special shell builtin"),
            (
                !skip_functions && session.functions.contains_key(name),
                "function",
                "a function",
            ),
            (
                !special
                    && !session.disabled_builtins.contains(name)
                    && named_builtin(name, &[], 0).is_some(),
                "builtin",
                "a shell builtin",
            ),
        ] {
            if !force_path && present && (!found || search_all) {
                let result = match format {
                    Format::Verbose => writeln!(out, "{name} is {label}").and_then(|()| {
                        if kind == "function" {
                            session.print_function(&mut out, name)
                        } else {
                            Ok(())
                        }
                    }),
                    Format::Kind => writeln!(out, "{kind}"),
                    Format::Name => writeln!(out, "{name}"),
                    Format::Path => Ok(()),
                };
                if result.is_err() {
                    return 1;
                }
                found = true;
                if !search_all {
                    break;
                }
            }
        }
        if !found && !search_all && !force_path && !default_path {
            if let Some(path) = session.hashes.get(name) {
                let result = match format {
                    Format::Verbose => writeln!(out, "{name} is hashed ({})", path.display()),
                    Format::Kind => writeln!(out, "file"),
                    Format::Name | Format::Path => writeln!(out, "{}", path.display()),
                };
                if result.is_err() {
                    return 1;
                }
                found = true;
            }
        }
        if !found || search_all {
            let ignored = IgnoredExecutables::new(session);
            let mut report = |path: &Path| {
                found = true;
                let absolute;
                let path = if session.posix
                    && command
                    && matches!(format, Format::Name | Format::Verbose)
                    && path.is_relative()
                {
                    absolute = session.logical_directory()?.join(path);
                    &absolute
                } else {
                    path
                };
                match format {
                    Format::Verbose => writeln!(out, "{name} is {}", path.display()),
                    Format::Kind => writeln!(out, "file"),
                    Format::Name | Format::Path => writeln!(out, "{}", path.display()),
                }
            };
            if name.contains('/') {
                let path = Path::new(name);
                if executable(path) && report(path).is_err() {
                    return 1;
                }
            } else {
                for dir in std::env::split_paths(&*search) {
                    let path = if dir.as_os_str().is_empty() {
                        PathBuf::from(".").join(name)
                    } else {
                        dir.join(name)
                    };
                    if !ignored.contains(&path) && executable(&path) {
                        if report(&path).is_err() {
                            return 1;
                        }
                        if !search_all {
                            break;
                        }
                    }
                }
            }
        }
        if !found && format == Format::Verbose {
            eprintln!("tonari: {}: {name}: not found", cmd.words[0]);
        }
        any |= found;
        all &= found;
    }
    if crate::builtins::write_stdout_for(&cmd.words[0], &out) != 0 {
        return 1;
    }
    u8::from(
        !(if command && index < cmd.words.len() {
            any
        } else {
            all
        }),
    )
}

#[derive(Default)]
pub(crate) struct IgnoredExecutables {
    patterns: Option<std::rc::Rc<[crate::pattern::extended::Pattern]>>,
}

struct IgnoreCache {
    value: String,
    extended: bool,
    locale: crate::session::CharacterLocale,
    patterns: std::rc::Rc<[crate::pattern::extended::Pattern]>,
}

impl IgnoredExecutables {
    pub(crate) fn new(session: &Session) -> Self {
        thread_local! {
            static CACHE: std::cell::RefCell<Option<IgnoreCache>> = const { std::cell::RefCell::new(None) };
        }
        let owned;
        let value = match session.stored_scalar("EXECIGNORE") {
            Some(value) => value,
            None => {
                owned = session.get("EXECIGNORE");
                owned.as_deref().unwrap_or("")
            }
        };
        if value.is_empty() {
            return Self::default();
        }
        let extended = session.shopt.glob.extglob;
        let locale = session.character_locale();
        CACHE.with_borrow_mut(|cache| {
            if let Some(prev) = cache
                && prev.extended == extended
                && prev.locale == locale
                && prev.value == value
            {
                return Self {
                    patterns: Some(prev.patterns.clone()),
                };
            }
            let patterns = value
                .split(':')
                .filter(|p| !p.is_empty())
                .map(|p| crate::pattern::extended::Pattern::insensitive(p, extended, locale))
                .collect::<std::rc::Rc<[_]>>();
            *cache = Some(IgnoreCache {
                value: value.into(),
                extended,
                locale,
                patterns: patterns.clone(),
            });
            Self {
                patterns: Some(patterns),
            }
        })
    }

    pub(crate) fn active(&self) -> bool {
        self.patterns.as_ref().is_some_and(|p| !p.is_empty())
    }

    pub(crate) fn contains(&self, path: &Path) -> bool {
        self.patterns.as_ref().is_some_and(|patterns| {
            path.to_str()
                .is_some_and(|s| patterns.iter().any(|p| p.accepts(s, false)))
        })
    }
}

pub(super) fn executable(path: &Path) -> bool {
    check_executable(path).unwrap_or(false)
}

pub(super) fn check_executable(path: &Path) -> std::io::Result<bool> {
    if !path.metadata()?.is_file() {
        return Ok(false);
    }
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return Ok(false);
    };
    Ok(unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 })
}
