use crate::Session;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(crate) struct Entry {
    pub(super) path: PathBuf,
    hits: u64,
    relative_search: bool,
}
#[derive(Clone)]
pub(crate) struct Table {
    pub enabled: bool,
    entries: BTreeMap<String, Entry>,
}

impl Default for Table {
    fn default() -> Self {
        Self {
            enabled: true,
            entries: BTreeMap::new(),
        }
    }
}

impl Table {
    pub(crate) fn items(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .filter_map(|(k, e)| Some((k.as_str(), e.path.to_str()?)))
    }

    pub(crate) fn path_value(&self, key: &str) -> Option<&str> {
        self.entries.get(key)?.path.to_str()
    }

    pub(crate) fn bind_path(&mut self, key: String, value: String) {
        if self.enabled {
            self.entries.insert(
                key,
                Entry {
                    path: value.into(),
                    hits: 0,
                    relative_search: false,
                },
            );
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn directory_changed(&mut self) {
        self.entries.retain(|_, e| !e.relative_search);
    }

    pub(super) fn get(&self, name: &str) -> Option<&Path> {
        self.enabled
            .then(|| self.entries.get(name))
            .flatten()
            .map(|e| e.path.as_path())
    }

    pub(super) fn hit(&mut self, name: &str) -> Option<&Path> {
        if !self.enabled {
            return None;
        }
        let e = self.entries.get_mut(name)?;
        e.hits = e.hits.saturating_add(1);
        Some(&e.path)
    }
}

// Err(Absent) means every PATH candidate was definitively missing.
// Ok(None) leaves permission failures and existing non-executables to exec.
pub(super) struct Absent;

pub(super) fn search(name: &str, session: &Session) -> Result<Option<Entry>, Absent> {
    search_in(name, session, &session.search_path(&[]))
}

pub(super) fn search_in(
    name: &str,
    session: &Session,
    path: &std::ffi::OsStr,
) -> Result<Option<Entry>, Absent> {
    let ignored = super::lookup::IgnoredExecutables::new(session);
    let mut relative = false;
    let mut absent = true;
    for dir in super::lookup::search_directories(path) {
        relative |= !dir.is_absolute();
        let path = dir.join(name);
        if ignored.contains(&path) {
            continue;
        }
        match super::lookup::check_executable(&path) {
            Ok(true) => {
                return Ok(Some(Entry {
                    path,
                    hits: 0,
                    relative_search: relative,
                }));
            }
            Err(e) if matches!(e.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR)) => {}
            _ => absent = false,
        }
    }
    if absent { Err(Absent) } else { Ok(None) }
}

pub(super) fn resolve(session: &mut Session, name: &str) -> Result<Option<PathBuf>, Absent> {
    if !session.hashes.enabled || name.contains('/') || name.is_empty() {
        return Ok(None);
    }
    if let Some(path) = session.hashes.hit(name) {
        if !(session.posix || session.shopt.checkhash) || super::lookup::executable(path) {
            return Ok(Some(path.into()));
        }
        session.hashes.entries.remove(name);
    }
    let Some(mut entry) = search(name, session)? else {
        return Ok(None);
    };
    entry.hits = 1;
    let path = entry.path.clone();
    session.hashes.entries.insert(name.into(), entry);
    Ok(Some(path))
}

#[inline(never)]
pub(super) fn run(session: &mut Session, mut args: &[String]) -> u8 {
    if !session.hashes.enabled {
        diagnostic!("hash: hashing disabled");
        return 1;
    }
    let (mut list, mut remove, mut print, mut clear, mut supplied) =
        (false, false, false, false, None);
    while let Some(opt) = crate::builtins::flags::next(&mut args) {
        let mut flags = opt[1..].chars();
        while let Some(flag) = flags.next() {
            match flag {
                'l' => list = true,
                'd' => remove = true,
                't' => print = true,
                'r' => clear = true,
                'p' => {
                    let Some(path) = crate::builtins::flags::value(flags.as_str(), &mut args)
                    else {
                        diagnostic!("hash: -p requires a pathname");
                        return 2;
                    };
                    supplied = Some(path);
                    break;
                }
                _ => {
                    diagnostic!("hash: {opt}: invalid option");
                    return 2;
                }
            }
        }
    }
    if clear {
        session.hashes.clear();
        return 0;
    }
    let mut out = io::stdout().lock();
    let print_entry = |out: &mut io::StdoutLock<'_>, name: &str, path: &Path| -> io::Result<()> {
        let path = path.to_string_lossy();
        use crate::word::quote::shell_quote as quote;
        writeln!(out, "builtin hash -p {} {}", quote(&path), quote(name))
    };
    if args.is_empty() {
        if remove || print || supplied.is_some() {
            diagnostic!("hash: command name required");
            return 1;
        }
        if !session.posix
            && (!list || session.shopt.compat <= 50 && session.hashes.entries.is_empty())
        {
            let label = if session.hashes.entries.is_empty() {
                "hash: hash table empty"
            } else {
                "hits\tcommand"
            };
            if writeln!(out, "{label}").is_err() {
                return 1;
            }
        }
        for (name, e) in &session.hashes.entries {
            let result = if list {
                print_entry(&mut out, name, &e.path)
            } else {
                writeln!(out, "{:4}\t{}", e.hits, e.path.display())
            };
            if result.is_err() {
                return 1;
            }
        }
        return out.flush().map_or(1, |()| 0);
    }
    let mut status = 0;
    for name in args {
        let entry = if print {
            if let Some(path) = session.hashes.hit(name) {
                let result = if list {
                    print_entry(&mut out, name, path)
                } else if args.len() > 1 {
                    writeln!(out, "{name}\t{}", path.display())
                } else {
                    writeln!(out, "{}", path.display())
                };
                if result.is_err() {
                    return 1;
                }
                continue;
            }
            None
        } else if remove {
            if session.hashes.entries.remove(name).is_some() {
                continue;
            }
            None
        } else if let Some(path) = supplied {
            if Path::new(path).is_dir() {
                diagnostic!("hash: {path}: is a directory");
                status = 1;
                continue;
            }
            Some(Entry {
                path: path.into(),
                hits: 0,
                relative_search: false,
            })
        } else {
            if name.contains('/')
                || super::named_builtin(name, &[], 0).is_some()
                || session.functions.contains_key(name)
            {
                continue;
            }
            session.hashes.entries.remove(name);
            search(name, session).ok().flatten()
        };
        if let Some(entry) = entry {
            session.hashes.entries.insert(name.into(), entry);
        } else {
            diagnostic!("hash: {name}: not found");
            status = 1;
        }
    }
    out.flush().map_or(1, |()| status)
}
