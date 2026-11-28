use std::ffi::{CStr, CString, OsStr, OsString};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::{PreparedCommand, named_builtin};
use crate::session::Session;

#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum Search {
    #[default]
    Normal,
    NoFunctions,
    BuiltinsOnly,
}

fn default_system_path() -> &'static std::ffi::OsStr {
    static PATH: std::sync::LazyLock<OsString> = std::sync::LazyLock::new(|| {
        let size = unsafe { libc::confstr(libc::_CS_PATH, std::ptr::null_mut(), 0) };
        if size > 0 {
            let mut bytes = vec![0; size];
            let len = unsafe { libc::confstr(libc::_CS_PATH, bytes.as_mut_ptr().cast(), size) };
            if (1..=size).contains(&len) {
                bytes.truncate(len - 1);
                return std::os::unix::ffi::OsStringExt::from_vec(bytes);
            }
        }
        "/bin:/usr/bin".into()
    });
    &PATH
}

// Unix PATH entries borrow the original byte string, including non-UTF-8 entries.
pub(super) fn search_directories(path: &OsStr) -> impl Iterator<Item = &Path> {
    path.as_bytes().split(|&b| b == b':').map(|bytes| {
        if bytes.is_empty() {
            Path::new(".")
        } else {
            Path::new(OsStr::from_bytes(bytes))
        }
    })
}

pub(super) fn path<'a>(cmd: &'a PreparedCommand, session: &'a Session) -> &'a OsStr {
    if cmd.default_path {
        default_system_path()
    } else {
        session.search_path(&cmd.assignments)
    }
}

pub(super) fn command_prefix(cmd: &mut PreparedCommand, session: &Session) {
    let (passed, search, default_path) = scan_prefix(&cmd.words, cmd.search, session);
    cmd.search = search;
    cmd.default_path |= default_path;
    if passed != 0 {
        cmd.words.drain(..passed);
    }
}

pub(super) fn scan_prefix(
    words: &[String],
    mut search: Search,
    session: &Session,
) -> (usize, Search, bool) {
    let mut use_default_path = false;
    let mut passed = 0;
    while let Some(name) = words.get(passed) {
        let builtin = match name.as_str() {
            "builtin" => true,
            "command" => false,
            _ => break,
        };
        if session.disabled_builtins.contains(name)
            || search == Search::Normal && session.functions.contains_key(name)
        {
            break;
        }
        if builtin {
            passed += 1;
            if words.get(passed).is_some_and(|s| s == "--") {
                passed += 1;
            }
            search = Search::BuiltinsOnly;
        } else {
            let mut i = passed + 1;
            let mut default_path = false;
            while let Some(opt) = words.get(i) {
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
            if words
                .get(i)
                .is_some_and(|arg| arg.starts_with('-') && arg != "-")
                && (i == passed + 1 || words[i - 1] != "--")
            {
                break;
            }
            passed = i;
            search = Search::NoFunctions;
            use_default_path |= default_path;
        }
    }
    (passed, search, use_default_path)
}

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Verbose,
    Name,
    Kind,
    Path,
}

pub(super) fn describe(cmd: &PreparedCommand, session: &Session) -> u8 {
    describe_words(&cmd.words, cmd.default_path, session, |out| {
        crate::builtins::write_stdout_for(&cmd.words[0], out)
    })
}

pub(crate) fn describe_words(
    words: &[String],
    use_default_path: bool,
    session: &Session,
    mut flush: impl FnMut(&mut Vec<u8>) -> u8,
) -> u8 {
    let command = words[0] == "command";
    let mut format = if command {
        Format::Name
    } else {
        Format::Verbose
    };
    let mut search_all = false;
    let mut force_path = false;
    let mut skip_functions = false;
    let mut default_path = use_default_path;
    let mut operands = &words[1..];
    while let Some(opt) = crate::builtins::flags::next(&mut operands) {
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
                    diagnostic!("{}: unsupported option {opt}", words[0]);
                    return 2;
                }
            }
        }
    }
    let search = std::cell::LazyCell::new(|| {
        if default_path { default_system_path() } else { session.search_path(&[]) }
    });
    let ignored = std::cell::LazyCell::new(|| IgnoredExecutables::new(session));
    let mut out = Vec::new();
    let mut any = false;
    let mut all = true;
    for name in operands {
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
                            session.print_function(&mut out, name, false)
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
        if !found
            && !search_all
            && !force_path
            && !default_path
            && let Some(path) = session.hashes.get(name)
        {
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
        if !found || search_all {
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
                let mut candidate = Candidate::new();
                let mut directories = search_directories(&search).peekable();
                let mut fallback = None;
                while let Some(dir) = directories.next() {
                    let Some(c) = candidate.join(dir, name) else {
                        continue;
                    };
                    let path = Path::new(OsStr::from_bytes(c.to_bytes()));
                    if ignored.contains(path) {
                        continue;
                    }
                    let Ok(mode) = candidate_mode(c) else {
                        continue;
                    };
                    if mode & libc::S_IFMT != libc::S_IFREG {
                        continue;
                    }
                    // A single-result Bash query prefers any executable, then
                    // the first regular file. If this is the last candidate,
                    // either permission result selects the same pathname.
                    if !search_all && fallback.is_none() && directories.peek().is_none()
                        || executable_mode(c, mode)
                    {
                        if report(path).is_err() {
                            return 1;
                        }
                        if !search_all {
                            fallback = None;
                            break;
                        }
                    } else if !search_all && fallback.is_none() {
                        fallback = Some(path.to_owned());
                    }
                }
                if let Some(path) = fallback
                    && report(&path).is_err()
                {
                    return 1;
                }
            }
        }
        if !found && format == Format::Verbose {
            if flush(&mut out) != 0 {
                return 1;
            }
            out.clear();
            diagnostic!("{}: {name}: not found", words[0]);
        }
        any |= found;
        all &= found;
    }
    if flush(&mut out) != 0 {
        return 1;
    }
    u8::from(
        !(if command && !operands.is_empty() {
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

// Most PATH candidates fit in the inline buffer. A long component reuses
// the overflow allocation for the rest of this search.
pub(crate) struct Candidate {
    short: [u8; 512],
    overflow: Vec<u8>,
}

impl Candidate {
    pub(crate) fn new() -> Self {
        Self {
            short: [0; 512],
            overflow: Vec::new(),
        }
    }

    pub(crate) fn join(&mut self, dir: &Path, name: &str) -> Option<&CStr> {
        let prefix = if name.starts_with('/') {
            &[][..]
        } else {
            dir.as_os_str().as_bytes()
        };
        let slash = !prefix.is_empty() && !prefix.ends_with(b"/");
        let offset = prefix.len() + usize::from(slash);
        let len = offset + name.len() + 1;
        let bytes = if len <= self.short.len() {
            &mut self.short[..len]
        } else {
            self.overflow.resize(len, 0);
            &mut self.overflow[..]
        };
        bytes[..prefix.len()].copy_from_slice(prefix);
        if slash {
            bytes[prefix.len()] = b'/';
        }
        bytes[offset..len - 1].copy_from_slice(name.as_bytes());
        bytes[len - 1] = 0;
        CStr::from_bytes_with_nul(bytes).ok()
    }
}

pub(crate) fn check_candidate(path: &CStr) -> std::io::Result<bool> {
    candidate_mode(path)
        .map(|mode| mode & libc::S_IFMT == libc::S_IFREG && executable_mode(path, mode))
}

fn candidate_mode(path: &CStr) -> std::io::Result<libc::mode_t> {
    let mut stat = std::mem::MaybeUninit::uninit();
    if unsafe { libc::stat(path.as_ptr(), stat.as_mut_ptr()) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { stat.assume_init() }.st_mode)
}

fn executable_mode(path: &CStr, mode: libc::mode_t) -> bool {
    // An execute ACL alone can satisfy X_OK on Darwin, even when exec
    // rejects a regular file with no executable mode bit.
    mode & 0o111 != 0 && unsafe { libc::access(path.as_ptr(), libc::X_OK) } == 0
}

fn check_executable(path: &Path) -> std::io::Result<bool> {
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return Ok(false);
    };
    check_candidate(&path)
}
