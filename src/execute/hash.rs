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

fn search(name: &str, session: &Session) -> Option<Entry> {
    let path = session.search_path(&[]);
    let mut relative = false;
    for dir in std::env::split_paths(&path) {
        relative |= !dir.is_absolute();
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &dir
        };
        let path = dir.join(name);
        if super::lookup::executable(&path) {
            return Some(Entry {
                path,
                hits: 0,
                relative_search: relative,
            });
        }
    }
    None
}

pub(super) fn resolve(session: &mut Session, name: &str, remember: bool) -> Option<PathBuf> {
    if !session.hashes.enabled || name.contains('/') || name.is_empty() {
        return None;
    }
    if let Some(path) = session.hashes.hit(name) {
        if !session.shopt.checkhash || super::lookup::executable(path) {
            return Some(path.into());
        }
        session.hashes.entries.remove(name);
    }
    if !remember {
        return None;
    }
    let mut entry = search(name, session)?;
    entry.hits = 1;
    let path = entry.path.clone();
    session.hashes.entries.insert(name.into(), entry);
    Some(path)
}

pub(super) fn run(session: &mut Session, mut args: &[String]) -> u8 {
    if !session.hashes.enabled {
        eprintln!("tonari: hash: hashing disabled");
        return 1;
    }
    let (mut list, mut remove, mut print, mut clear, mut supplied) =
        (false, false, false, false, None);
    while let Some((opt, rest)) = args.split_first() {
        if opt == "--" {
            args = rest;
            break;
        }
        if !opt.starts_with('-') || opt == "-" {
            break;
        }
        args = rest;
        let mut flags = opt[1..].chars();
        while let Some(flag) = flags.next() {
            match flag {
                'l' => list = true,
                'd' => remove = true,
                't' => print = true,
                'r' => clear = true,
                'p' => {
                    if flags.as_str().is_empty() {
                        let Some((path, rest)) = args.split_first() else {
                            eprintln!("tonari: hash: -p requires a pathname");
                            return 2;
                        };
                        supplied = Some(path.as_str());
                        args = rest;
                    } else {
                        supplied = Some(flags.as_str());
                    }
                    break;
                }
                _ => {
                    eprintln!("tonari: hash: {opt}: invalid option");
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
        let quote = |s: &str| {
            if !s.is_empty()
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-._/".contains(&c))
            {
                s.into()
            } else {
                format!("'{}'", s.replace('\'', "'\\''"))
            }
        };
        writeln!(out, "builtin hash -p {} {}", quote(&path), quote(name))
    };
    if args.is_empty() {
        if remove || print || supplied.is_some() {
            eprintln!("tonari: hash: command name required");
            return 1;
        }
        if !list {
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
                eprintln!("tonari: hash: {path}: is a directory");
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
            search(name, session)
        };
        if let Some(entry) = entry {
            session.hashes.entries.insert(name.into(), entry);
        } else {
            eprintln!("tonari: hash: {name}: not found");
            status = 1;
        }
    }
    out.flush().map_or(1, |()| status)
}
