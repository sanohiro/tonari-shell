use std::fs;
use std::ops::ControlFlow;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

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
}

impl<'a> Context<'a> {
    /// Tokenize the current simple command for completion without evaluating it.
    fn scan(line: &'a str, cursor: usize) -> Option<Self> {
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

pub fn at(line: &str, cursor: usize, session: &mut Session, list: bool) -> Option<Completion> {
    let context = Context::scan(line, cursor)?;
    let first = context.current == 0;
    let start = context.start;
    let prefix = context.words.get(context.current)?;
    let command = (!first).then(|| context.words[0].as_str());
    let default_source = if first && !prefix.contains('/') {
        Source::Commands
    } else {
        Source::Files
    };
    // First Tab inserts a common prefix. Only a listing needs every candidate.
    let mut options = (false, false);
    let matches = if let Some(generated) =
        crate::completion_spec::for_tab(session, command, prefix, Some(&context))
    {
        options = (generated.noquote, generated.nospace);
        let result = generated.source.matches(prefix, session, list);
        if result.is_none() && generated.fallback {
            options = (false, false);
            default_source.matches(prefix, session, list)
        } else {
            result
        }
    } else {
        default_source.matches(prefix, session, list)
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
        if !leading_tilde && !c.is_alphanumeric() && !"/._-".contains(c) {
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
    visit: &mut dyn FnMut(&str, Option<&fs::DirEntry>) -> ControlFlow<()>,
) -> ControlFlow<()> {
    for name in crate::execute::builtin_names()
        .iter()
        .copied()
        .chain(session.functions.keys().map(String::as_str))
        .chain(session.aliases.names().map(String::as_str))
    {
        if name.starts_with(prefix) {
            visit(name, None)?;
        }
    }
    for dir in std::env::split_paths(&session.search_path(&[])) {
        let path = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &dir
        };
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str().filter(|n| n.starts_with(prefix)) {
                    visit(name, Some(&entry))?;
                }
            }
        }
    }
    ControlFlow::Continue(())
}

fn executable(entry: Option<&fs::DirEntry>) -> bool {
    entry.is_none_or(|e| {
        fs::metadata(e.path()).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

fn common_command(prefix: &str, session: &Session) -> Option<Common> {
    let mut common: Option<Common> = None;
    let _ = visit_commands(prefix, session, &mut |name, entry| {
        if let Some(known) = &mut common {
            // Only a name that could shorten the prefix or change uniqueness
            // needs a filesystem permission check.
            if known.could_change(name) && executable(entry) {
                return known.narrow(name, prefix);
            }
        } else if executable(entry) {
            common = Some(Common::new(name.to_owned()));
        }
        ControlFlow::Continue(())
    });
    common
}

pub fn command_names(prefix: &str, session: &Session) -> Vec<String> {
    let mut names = Vec::new();
    let _ = visit_commands(prefix, session, &mut |name, entry| {
        if executable(entry) {
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
    entries: fs::ReadDir,
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
        Some(Self {
            dir,
            base,
            entries: fs::read_dir(path).ok()?,
        })
    }

    fn matches(self) -> impl Iterator<Item = (String, fs::DirEntry)> + 'a {
        self.entries.flatten().filter_map(move |entry| {
            let name = entry.file_name().into_string().ok()?;
            (name.starts_with(self.base) && (!name.starts_with('.') || self.base.starts_with('.')))
                .then_some((name, entry))
        })
    }
}

fn is_directory(entry: &fs::DirEntry) -> bool {
    match entry.file_type() {
        Ok(kind) if !kind.is_symlink() => kind.is_dir(),
        _ => entry.path().is_dir(),
    }
}

fn common_file(prefix: &str, directories_only: bool, session: &Session) -> Option<Common> {
    let directory = Directory::open(prefix, session)?;
    let (dir, base) = (directory.dir, directory.base);
    let mut matches = directory.matches();
    let (name, first) = matches.find(|(_, e)| !directories_only || is_directory(e))?;
    let mut common = Common::new(name);
    for (name, entry) in matches {
        if common.could_change(&name)
            && (!directories_only || is_directory(&entry))
            && common.narrow(&name, base).is_break()
        {
            break;
        }
    }
    // A slash can extend the common prefix only when this is the sole match.
    if common.unique && (directories_only || is_directory(&first)) {
        common.text.push('/');
    }
    common.text.insert_str(0, dir);
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
    let dir = directory.dir;
    let mut choices = Vec::new();
    for (name, entry) in directory.matches() {
        let is_dir = (directories_only || mark_directories) && is_directory(&entry);
        if directories_only && !is_dir {
            continue;
        }
        let mut path = String::with_capacity(dir.len() + name.len() + 1);
        path.push_str(dir);
        path.push_str(&name);
        if mark_directories && is_dir {
            path.push('/');
        }
        choices.push(path);
    }
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
