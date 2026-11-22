use std::collections::BTreeSet;
use std::fs;
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

pub fn at(line: &str, cursor: usize, session: &mut Session) -> Option<Completion> {
    let context = Context::scan(line, cursor)?;
    let first = context.current == 0;
    let start = context.start;
    let prefix = context.words.get(context.current)?;
    let command = (!first).then(|| context.words[0].as_str());
    let generated = crate::completion_spec::for_tab(session, command, prefix, Some(&context));
    let (choices, noquote, nospace) = if let Some(generated) = generated {
        (generated.words, generated.noquote, generated.nospace)
    } else {
        let choices = if first && !prefix.contains('/') {
            command_names(prefix, session)
        } else {
            file_names(prefix, false, true, session)
        };
        (choices, false, false)
    };
    let mut common = choices.first()?.as_str();
    for choice in &choices[1..] {
        let len: usize = common
            .chars()
            .zip(choice.chars())
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a.len_utf8())
            .sum();
        common = &common[..len];
    }
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
    if !nospace && choices.len() == 1 && !common.ends_with('/') && cursor == line.len() {
        replacement.push(' ');
    }
    Some(Completion {
        start,
        replacement,
        choices,
    })
}

pub fn command_names(prefix: &str, session: &Session) -> Vec<String> {
    let mut choices = BTreeSet::new();
    for dir in std::env::split_paths(&session.search_path(&[])) {
        if let Ok(entries) = fs::read_dir(if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &dir
        }) {
            for entry in entries.flatten() {
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                if name.starts_with(prefix)
                    && entry
                        .metadata()
                        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                {
                    choices.insert(name);
                }
            }
        }
    }
    for name in session.functions.keys().chain(session.aliases.names()) {
        if name.starts_with(prefix) {
            choices.insert(name.clone());
        }
    }
    for &name in crate::execute::builtin_names() {
        if name.starts_with(prefix) {
            choices.insert(name.to_string());
        }
    }
    choices.into_iter().collect()
}

pub fn file_names(
    prefix: &str,
    directories_only: bool,
    mark_directories: bool,
    session: &Session,
) -> Vec<String> {
    let mut choices = BTreeSet::new();
    let (dir, base) = prefix
        .rfind('/')
        .map_or(("", prefix), |i| prefix.split_at(i + 1));
    let path = match dir.strip_prefix("~/") {
        Some(rest) => match session.get("HOME") {
            Some(home) => Path::new(&home).join(rest),
            None => return Vec::new(),
        },
        None => Path::new(if dir.is_empty() { "." } else { dir }).to_path_buf(),
    };
    let Ok(entries) = fs::read_dir(path) else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !name.starts_with(base) || (name.starts_with('.') && !base.starts_with('.')) {
            continue;
        }
        let is_dir = entry.path().is_dir();
        if directories_only && !is_dir {
            continue;
        }
        let suffix = if mark_directories && is_dir { "/" } else { "" };
        choices.insert(format!("{dir}{name}{suffix}"));
    }
    choices.into_iter().collect()
}
