use crate::pattern::directory::scan;
use std::fs;
use std::ops::ControlFlow;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::Session;

pub struct Completion {
    pub start: usize,
    pub replacement: String,
    pub choices: Vec<String>,
}

pub struct Context<'a> {
    pub line: &'a str,
    pub cursor: usize,
    pub words: Vec<String>,
    pub current: usize,
    pub start: usize,
    pub command_start: usize,
}

impl<'a> Context<'a> {
    /// Tokenize the current simple command for completion without evaluating it.
    pub(crate) fn scan(line: &'a str, cursor: usize) -> Option<Self> {
        if line[cursor..]
            .chars()
            .next()
            .is_some_and(|c| !c.is_whitespace() && !"|;&".contains(c))
        {
            return None;
        }
        let mut words = Vec::new();
        let mut word = String::new();
        let mut quote = None;
        let mut escaped = false;
        let mut in_word = false;
        let mut start = 0;
        let mut command_start = 0;
        let mut current = None;
        for (i, c) in line.char_indices() {
            if i == cursor {
                current = Some(words.len());
                if !in_word {
                    words.push(String::new());
                }
            }
            if escaped {
                word.push(c);
                escaped = false;
            } else if let Some(q) = quote {
                if c == q {
                    quote = None;
                } else if c == '\\' && q == '"' {
                    escaped = true;
                } else {
                    word.push(c);
                }
            } else if c.is_whitespace() || "|;&".contains(c) {
                if in_word {
                    words.push(std::mem::take(&mut word));
                }
                if c == '\n' || "|;&".contains(c) {
                    if i >= cursor {
                        in_word = false;
                        break;
                    }
                    words.clear();
                }
                if i < cursor {
                    start = i + c.len_utf8();
                }
                in_word = false;
            } else {
                if !in_word && words.is_empty() && i < cursor {
                    command_start = i;
                }
                in_word = true;
                match c {
                    '\\' => escaped = true,
                    '\'' | '"' => quote = Some(c),
                    _ => word.push(c),
                }
            }
        }
        if current.is_none() {
            current = Some(words.len());
            words.push(word);
        } else if in_word {
            words.push(word);
        }
        Some(Self {
            line,
            cursor,
            words,
            current: current?,
            start,
            command_start,
        })
    }
}

struct Common {
    text: String,
    unique: bool,
}

impl Common {
    fn new(text: String) -> Self {
        Self { text, unique: true }
    }

    fn could_change(&self, name: &str) -> bool {
        if self.unique {
            name != self.text
        } else {
            !name.starts_with(&self.text)
        }
    }

    // Called for a distinct, validated candidate. Stop as soon as further
    // candidates cannot change the common prefix or make it unique again.
    fn narrow(&mut self, name: &str, lower_bound: &str) -> ControlFlow<()> {
        self.unique = false;
        let mut end = self
            .text
            .bytes()
            .zip(name.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        while !self.text.is_char_boundary(end) {
            end -= 1;
        }
        self.text.truncate(end);
        if self.text == lower_bound {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    fn from_names<'a>(mut names: impl Iterator<Item = &'a str>, lower_bound: &str) -> Option<Self> {
        let mut common = Self::new(names.next()?.to_owned());
        for name in names {
            if common.could_change(name) && common.narrow(name, lower_bound).is_break() {
                break;
            }
        }
        Some(common)
    }
}

pub enum Source {
    Words(Vec<String>),
    LiteralWords(std::rc::Rc<crate::completion_spec::WordList>),
    Files,
    Directories,
    Commands,
}

impl Source {
    fn matches(self, prefix: &str, session: &Session, list: bool) -> Option<(Common, Vec<String>)> {
        let words = match self {
            Self::Words(words) => {
                return Some((
                    Common::from_names(words.iter().map(String::as_str), "")?,
                    words,
                ));
            }
            Self::LiteralWords(source) => {
                let ifs = session.get("IFS").unwrap_or_else(|| " \t\n".into());
                if let Some(matches) = source.indexed(prefix, &ifs) {
                    let mut common = Common::new(source.word(*matches.first()?).to_owned());
                    if matches.len() > 1 {
                        let _ = common.narrow(source.word(*matches.last()?), prefix);
                    }
                    let choices = if list {
                        matches.iter().map(|&r| source.word(r).to_owned()).collect()
                    } else {
                        Vec::new()
                    };
                    return Some((common, choices));
                }
                let names = source.literal_matches(prefix, &ifs);
                if !list {
                    return Some((Common::from_names(names, prefix)?, Vec::new()));
                }
                let mut choices: Vec<_> = names.map(str::to_owned).collect();
                choices.sort_unstable();
                choices.dedup();
                choices
            }
            Self::Commands if list => command_names(prefix, session),
            Self::Commands => {
                return Some((common_command(prefix, session)?, Vec::new()));
            }
            Self::Files | Self::Directories => {
                let directories_only = matches!(self, Self::Directories);
                if !list {
                    return Some((common_file(prefix, directories_only, session)?, Vec::new()));
                }
                file_names(prefix, directories_only, true, session)
            }
        };
        // These generators return sorted choices. The first and last determine
        // the common prefix of the whole list, including multibyte boundaries.
        let mut common = Common::new(words.first()?.clone());
        if words.len() > 1 {
            let _ = common.narrow(words.last()?, prefix);
        }
        Some((common, words))
    }
}

pub fn at(
    line: &str,
    cursor: usize,
    session: &mut Session,
    list: bool,
    files_only: bool,
) -> Option<Completion> {
    let context = Context::scan(line, cursor)?;
    let first = context.current == 0;
    let start = context.start;
    let prefix = context.words.get(context.current)?;
    let command = (!first).then(|| context.words[0].as_str());
    let default_source = if files_only {
        Source::Files
    } else if session.shopt.hostcomplete
        && let Some(host) = prefix.strip_prefix('@')
    {
        let mut names: Vec<_> = host_names(host, session)
            .into_iter()
            .map(|s| format!("@{s}"))
            .collect();
        names.sort_unstable();
        names.dedup();
        if names.is_empty() {
            Source::Files
        } else {
            Source::Words(names)
        }
    } else if first && !prefix.contains('/') {
        Source::Commands
    } else {
        Source::Files
    };
    // Registered definitions own the candidate list, including an explicitly
    // empty result. Only the shell's default completion offers learned arguments.
    let mut options = (false, false);
    let generated = (!files_only)
        .then(|| crate::completion_spec::for_tab(session, command, prefix, Some(&context)))
        .flatten();
    let ordinary = || {
        let dirs = matches!(command, Some("cd" | "pushd")) && !files_only;
        if prefix.is_empty()
            && !files_only
            && context.current > 1
            && context.words[context.current - 1].starts_with('-')
        {
            let names = crate::history::arguments::candidates(
                session,
                &context.words,
                context.current,
                dirs,
            );
            if !names.is_empty() {
                return alternatives(names, prefix);
            }
        }
        let files = matches!(default_source, Source::Files | Source::Directories);
        let source = if dirs && files {
            Source::Directories
        } else {
            default_source
        };
        source
            .matches(prefix, session, list)
            .or_else(|| files.then(|| abbreviated(prefix, dirs, session)).flatten())
            .or_else(|| {
                if files_only || first || prefix.is_empty() {
                    return None;
                }
                alternatives(
                    crate::history::arguments::candidates(
                        session,
                        &context.words,
                        context.current,
                        dirs,
                    ),
                    prefix,
                )
            })
    };
    let matches = if let Some(generated) = generated {
        options = (generated.noquote, generated.nospace);
        let directory = matches!(generated.source, Source::Directories);
        let paths = directory || matches!(generated.source, Source::Files);
        let result = generated.source.matches(prefix, session, list).or_else(|| {
            paths
                .then(|| abbreviated(prefix, directory, session))
                .flatten()
        });
        if result.is_none() && generated.fallback {
            options = (false, false);
            ordinary()
        } else {
            result
        }
    } else {
        ordinary()
    };
    let (common, choices) = matches?;
    let (noquote, nospace) = options;
    let unique = common.unique;
    let common = common.text;
    // Re-quote the complete word. A unique file followed by Enter must remain
    // a single literal argument even when it contains shell metacharacters.
    let mut replacement = String::new();
    for (i, c) in common.char_indices() {
        if noquote {
            replacement.push(c);
            continue;
        }
        if c == '\n' {
            replacement.push_str("'\n'");
            continue;
        }
        let leading_tilde = i == 0 && common.starts_with("~/");
        let ordinary = c.is_alphanumeric()
            || "/._-".contains(c)
            || !c.is_ascii() && !c.is_whitespace() && !c.is_control();
        if !leading_tilde && !ordinary {
            replacement.push('\\');
        }
        replacement.push(c);
    }
    if !nospace && unique && !common.ends_with('/') && cursor == line.len() {
        replacement.push(' ');
    }
    Some(Completion {
        start,
        replacement,
        choices,
    })
}

fn visit_commands(
    prefix: &str,
    session: &Session,
    visit: &mut dyn FnMut(&str, Option<(&Path, u8)>) -> ControlFlow<()>,
) -> ControlFlow<()> {
    for name in crate::execute::builtin_names()
        .iter()
        .copied()
        .filter(|n| !session.disabled_builtins.contains(*n))
        .chain(session.functions.keys().map(String::as_str))
        .chain(session.aliases.names().map(String::as_str))
    {
        if name.starts_with(prefix) {
            visit(name, None)?;
        }
    }
    let editing = crate::editor::preview::active();
    let mut entries = 0usize;
    for dir in std::env::split_paths(&session.search_path(&[])) {
        let path = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &dir
        };
        scan(path, &mut |raw, kind| {
            if editing && entries % 256 == 0 {
                crate::editor::preview::scan_input();
            }
            entries += 1;
            if !matches!(raw.to_bytes(), b"." | b"..")
                && raw.to_bytes().starts_with(prefix.as_bytes())
                && let Ok(name) = raw.to_str()
            {
                return visit(name, Some((path, kind)));
            }
            ControlFlow::Continue(())
        })?;
    }
    ControlFlow::Continue(())
}

fn executable(name: &str, entry: Option<(&Path, u8)>) -> bool {
    entry.is_none_or(|(path, kind)| {
        matches!(kind, libc::DT_REG | libc::DT_LNK | libc::DT_UNKNOWN)
            && fs::metadata(path.join(name))
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

fn common_command(prefix: &str, session: &Session) -> Option<Common> {
    let mut common: Option<Common> = None;
    let _ = visit_commands(prefix, session, &mut |name, entry| {
        if let Some(known) = &mut common {
            // Only a name that could shorten the prefix or change uniqueness
            // needs a filesystem permission check.
            if known.could_change(name) && executable(name, entry) {
                return known.narrow(name, prefix);
            }
        } else if executable(name, entry) {
            common = Some(Common::new(name.to_owned()));
        }
        ControlFlow::Continue(())
    });
    common
}

pub(crate) fn user_names(prefix: &str) -> Vec<String> {
    let mut names = Vec::new();
    // Consume each libc-owned entry before requesting the next one.
    unsafe {
        libc::setpwent();
        loop {
            let entry = libc::getpwent();
            if entry.is_null() {
                break;
            }
            if let Ok(name) = std::ffi::CStr::from_ptr((*entry).pw_name).to_str()
                && name.starts_with(prefix)
            {
                names.push(name.to_owned());
            }
        }
        libc::endpwent();
    }
    names
}

pub(crate) fn host_names(prefix: &str, session: &Session) -> Vec<String> {
    let path = session
        .get("HOSTFILE")
        .unwrap_or_else(|| "/etc/hosts".into());
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .flat_map(|line| {
            line.split('#')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .skip(1)
        })
        .filter(|s| s.starts_with(prefix))
        .map(str::to_owned)
        .collect()
}

pub fn command_names(prefix: &str, session: &Session) -> Vec<String> {
    let mut names = Vec::new();
    let _ = visit_commands(prefix, session, &mut |name, entry| {
        if executable(name, entry) {
            names.push(name.to_owned());
        }
        ControlFlow::Continue(())
    });
    names.sort_unstable();
    names.dedup();
    names
}

struct Directory<'a> {
    dir: &'a str,
    base: &'a str,
    path: PathBuf,
}

impl<'a> Directory<'a> {
    fn open(prefix: &'a str, session: &Session) -> Option<Self> {
        let (dir, base) = prefix
            .rfind('/')
            .map_or(("", prefix), |i| prefix.split_at(i + 1));
        let path = match dir.strip_prefix("~/") {
            Some(rest) => Path::new(&session.get("HOME")?).join(rest),
            None => Path::new(if dir.is_empty() { "." } else { dir }).to_owned(),
        };
        Some(Self { dir, base, path })
    }

    fn visit(&self, mut visitor: impl FnMut(&str, u8) -> ControlFlow<()>) {
        let mut entries = 0usize;
        let editing = crate::editor::preview::active();
        let _ = scan(&self.path, &mut |raw, kind| {
            if editing && entries % 256 == 0 {
                crate::editor::preview::scan_input();
            }
            entries += 1;
            let bytes = raw.to_bytes();
            if bytes.starts_with(self.base.as_bytes())
                && !matches!(bytes, b"." | b"..")
                && (!bytes.starts_with(b".") || self.base.starts_with('.'))
                && let Ok(name) = raw.to_str()
            {
                return visitor(name, kind);
            }
            ControlFlow::Continue(())
        });
    }

    fn collect(&self, directories_only: bool, mark_directories: bool, choices: &mut Vec<String>) {
        let dir = self.dir;
        self.visit(|name, kind| {
            let is_dir = (directories_only || mark_directories) && self.is_directory(name, kind);
            if directories_only && !is_dir {
                return ControlFlow::Continue(());
            }
            let mut path = String::with_capacity(dir.len() + name.len() + 1);
            path.push_str(dir);
            path.push_str(name);
            if mark_directories && is_dir {
                path.push('/');
            }
            choices.push(path);
            ControlFlow::Continue(())
        });
    }

    fn is_directory(&self, name: &str, kind: u8) -> bool {
        kind == libc::DT_DIR
            || matches!(kind, libc::DT_LNK | libc::DT_UNKNOWN) && self.path.join(name).is_dir()
    }
}

fn alternatives(names: Vec<String>, prefix: &str) -> Option<(Common, Vec<String>)> {
    let mut common = Common::from_names(names.iter().map(String::as_str), "")?;
    if !common.unique && !common.text.starts_with(prefix) {
        common.text = prefix.to_owned();
    }
    Some((common, names))
}

fn abbreviated(prefix: &str, dirs: bool, session: &Session) -> Option<(Common, Vec<String>)> {
    let (base, rest) = if let Some(rest) = prefix.strip_prefix("~/") {
        ("~/", rest)
    } else if let Some(rest) = prefix.strip_prefix('/') {
        ("/", rest)
    } else {
        ("", prefix)
    };
    let (parents, leaf) = rest.rsplit_once('/')?;
    let mut paths = vec![base.to_owned()];
    for part in parents.split('/').filter(|s| !s.is_empty()) {
        let mut next = Vec::new();
        for mut path in paths {
            path.push_str(part);
            let directory = Directory::open(&path, session)?;
            if directory.path.join(part).is_dir() {
                path.push('/');
                next.push(path);
            } else if !matches!(part, "." | "..") {
                directory.visit(|name, kind| {
                    if directory.is_directory(name, kind) {
                        next.push(format!("{}{name}/", directory.dir));
                    }
                    if next.len() > 128 {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                });
            }
            if next.len() > 128 {
                return None;
            }
        }
        if next.is_empty() {
            return None;
        }
        paths = next;
    }
    for path in &mut paths {
        path.push_str(leaf);
    }
    let branches: Vec<_> = paths
        .iter()
        .filter_map(|p| Directory::open(p, session))
        .collect();
    let mut matches = branch_matches(&branches, dirs);
    matches.sort_unstable();
    matches.dedup();
    alternatives(matches, prefix)
}

// Only independent, sufficiently large directories share workers. The editor
// retains the interpreter and the tty; workers finish before Tab can run code.
fn branch_matches(branches: &[Directory<'_>], dirs: bool) -> Vec<String> {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };
    use std::{thread, time::Duration};
    let large = branches.len() >= 4
        && branches
            .iter()
            .map(|d| fs::metadata(&d.path).map_or(0, |m| m.len()))
            .sum::<u64>()
            >= 128 * 1024;
    let workers = if large {
        thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(4)
            .min(branches.len())
    } else {
        1
    };
    if workers == 1 {
        let mut choices = Vec::new();
        for d in branches {
            d.collect(dirs, true, &mut choices);
        }
        return choices;
    }
    let next = AtomicUsize::new(0);
    let scan = || {
        let mut choices = Vec::new();
        while let Some(d) = branches.get(next.fetch_add(1, Ordering::Relaxed)) {
            d.collect(dirs, true, &mut choices);
        }
        choices
    };
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        for _ in 1..workers {
            let sender = sender.clone();
            let scan = &scan;
            if thread::Builder::new()
                .spawn_scoped(scope, move || {
                    let _ = sender.send(scan());
                })
                .is_err()
            {
                break;
            }
        }
        drop(sender);
        let mut matches = scan();
        loop {
            match receiver.recv_timeout(Duration::from_millis(1)) {
                Ok(part) => matches.extend(part),
                Err(mpsc::RecvTimeoutError::Timeout) => crate::editor::preview::poll(),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        matches
    })
}

fn common_file(prefix: &str, directories_only: bool, session: &Session) -> Option<Common> {
    let directory = Directory::open(prefix, session)?;
    let mut first_kind = libc::DT_UNKNOWN;
    let mut common: Option<Common> = None;
    directory.visit(|name, kind| {
        if let Some(known) = &mut common {
            if known.could_change(name) && (!directories_only || directory.is_directory(name, kind))
            {
                return known.narrow(name, directory.base);
            }
        } else if !directories_only || directory.is_directory(name, kind) {
            first_kind = kind;
            common = Some(Common::new(name.to_owned()));
        }
        ControlFlow::Continue(())
    });
    let mut common = common?;
    if common.unique && (directories_only || directory.is_directory(&common.text, first_kind)) {
        common.text.push('/');
    }
    common.text.insert_str(0, directory.dir);
    Some(common)
}

pub fn file_names(
    prefix: &str,
    directories_only: bool,
    mark_directories: bool,
    session: &Session,
) -> Vec<String> {
    let Some(directory) = Directory::open(prefix, session) else {
        return Vec::new();
    };
    let mut choices = Vec::new();
    directory.collect(directories_only, mark_directories, &mut choices);
    choices.sort_unstable();
    choices.dedup();
    choices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_prefix_respects_utf8_and_duplicate_names() {
        let same = Common::from_names(["雪", "雪"].into_iter(), "").unwrap();
        assert!(same.unique);
        let split = Common::from_names(["x雪", "x雲"].into_iter(), "").unwrap();
        assert!(!split.unique);
        assert_eq!(split.text, "x");
    }
}
