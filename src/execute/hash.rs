use crate::Session;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
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

    fn record(&mut self, name: &str, mut entry: Entry) -> PathBuf {
        entry.hits = 1;
        let path = entry.path.clone();
        self.entries.insert(name.into(), entry);
        path
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
    search_in(name, session, session.search_path(&[]))
}

pub(super) fn search_in(
    name: &str,
    session: &Session,
    path: &std::ffi::OsStr,
) -> Result<Option<Entry>, Absent> {
    let ignored = super::lookup::IgnoredExecutables::new(session);
    let mut relative = false;
    let mut absent = true;
    let mut candidate = super::lookup::Candidate::new();
    for dir in super::lookup::search_directories(path) {
        relative |= !dir.is_absolute();
        let Some(c) = candidate.join(dir, name) else {
            absent = false;
            continue;
        };
        let path = Path::new(std::ffi::OsStr::from_bytes(c.to_bytes()));
        if ignored.contains(path) {
            continue;
        }
        match super::lookup::check_candidate(c) {
            Ok(true) => {
                return Ok(Some(Entry {
                    path: path.to_owned(),
                    hits: 0,
                    relative_search: relative,
                }));
            }
            Err(e)
                if matches!(
                    e.raw_os_error(),
                    Some(libc::ENOENT | libc::ENOTDIR | libc::EISDIR)
                ) => {}
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
    Ok(search(name, session)?.map(|entry| session.hashes.record(name, entry)))
}

// Bash resolves a foreground external name in its parent before command
// redirections, including redirections whose operands are evaluated in a child.
pub(super) fn resolve_command(
    session: &mut Session,
    cmd: &super::PreparedCommand,
) -> Result<Option<PathBuf>, Absent> {
    if cmd.assignments.iter().any(|(key, _)| key == "PATH") {
        session.hashes.clear();
        return Ok(None);
    }
    let name = &cmd.words[0];
    if !cmd.default_path {
        return resolve(session, name);
    }
    if !session.hashes.enabled || name.is_empty() || name.contains('/') {
        return Ok(None);
    }
    // command -p bypasses an old hash and records the standard-path result.
    Ok(search_in(name, session, super::lookup::path(cmd, session))?
        .map(|entry| session.hashes.record(name, entry)))
}

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
    let print_entry = |out: &mut dyn Write, name: &str, path: &Path| -> io::Result<()> {
        let path = path.to_string_lossy();
        use crate::word::quote::shell_quote as quote;
        writeln!(out, "builtin hash -p {} {}", quote(&path), quote(name))
    };
    if args.is_empty() {
        let mut out = crate::builtins::listing_stdout("hash");
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
        let mut decimal = itoa::Buffer::new();
        let mut row = Vec::new();
        for (name, e) in &session.hashes.entries {
            let result = if list {
                print_entry(&mut out, name, &e.path)
            } else {
                let hits = decimal.format(e.hits).as_bytes();
                row.clear();
                row.resize(4usize.saturating_sub(hits.len()), b' ');
                row.extend_from_slice(hits);
                row.push(b'\t');
                row.extend_from_slice(e.path.to_string_lossy().as_bytes());
                row.push(b'\n');
                out.write_all(&row)
            };
            if result.is_err() {
                return 1;
            }
        }
        return out.flush().map_or(1, |()| 0);
    }
    let mut output = None;
    let mut status = 0;
    for name in args {
        let entry = if print {
            if let Some(path) = session.hashes.hit(name) {
                let out = output.get_or_insert_with(|| crate::builtins::stdout_for("hash"));
                let result = if list {
                    print_entry(out, name, path)
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
                diagnostic!("hash: {path}: Is a directory");
                status = 1;
                continue;
            }
            if let Some(entry) = session.hashes.entries.get_mut(name) {
                entry.path.clear();
                entry.path.push(path);
                entry.hits = 0;
                entry.relative_search = false;
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
    output.map_or(status, |mut out| out.flush().map_or(1, |()| status))
}
