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
    noquote: bool,
    append_space: bool,
    source_prefix: usize,
}

pub(crate) fn default_word_breaks(hostname: bool) -> &'static str {
    if hostname {
        " \t\n\"'@><=;|&(:"
    } else {
        " \t\n\"'><=;|&(:"
    }
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
    // The callback receives source spelling, including quotes and backslashes.
    // Readline keeps a run of punctuation as one COMP_WORDS entry, while the
    // replacement starts after that punctuation.
    pub(crate) fn programmable(&self, breaks: &str, hostname: bool) -> Self {
        let mut words = Vec::new();
        let mut token = self.command_start;
        let mut punctuation = false;
        let mut quote = None;
        let mut escaped = false;
        let mut end = self.line.len();
        let mut current = None;
        let mut start = self.cursor;
        let mut emit = |a: usize, b: usize, punct: bool| {
            if current.is_none() && self.cursor < a {
                current = Some(words.len());
                words.push(String::new());
            }
            if current.is_none() && self.cursor >= a && self.cursor <= b {
                current = Some(words.len());
                start = if punct { self.cursor } else { a };
                if !punct
                    && hostname
                    && a > self.command_start
                    && self.line.as_bytes()[a - 1] == b'@'
                    && a == self.start + 1
                {
                    start = a - 1;
                }
            }
            words.push(self.line[a..b].to_owned());
        };
        for (off, c) in self.line[self.command_start..].char_indices() {
            let i = self.command_start + off;
            let protected = escaped || quote.is_some() || matches!(c, '\'' | '"' | '\\');
            if escaped {
                escaped = false;
            } else if let Some(q) = quote {
                if c == q {
                    quote = None;
                } else if c == '\\' && q == '"' {
                    escaped = true;
                }
            } else if matches!(c, '\'' | '"') {
                quote = Some(c);
            } else if c == '\\' {
                escaped = true;
            }
            if !protected && matches!(c, ';' | '|' | '&' | '\n') {
                end = i;
                break;
            }
            let split = !protected && breaks.contains(c);
            let white = split && matches!(c, ' ' | '\t');
            if white || punctuation != split {
                if token < i {
                    emit(token, i, punctuation);
                }
                token = i + if white { c.len_utf8() } else { 0 };
            }
            punctuation = split && !white;
        }
        if token < end {
            emit(token, end, punctuation);
        }
        let current = current.unwrap_or_else(|| {
            words.push(String::new());
            words.len() - 1
        });
        Self {
            line: self.line,
            cursor: self.cursor,
            words,
            current,
            start,
            command_start: self.command_start,
        }
    }

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
                    command_start = i + c.len_utf8();
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
                let ifs = session
                    .get("IFS")
                    .unwrap_or_else(|| crate::word::DEFAULT_IFS.into());
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
                let ignored = (!directories_only)
                    .then(|| session.get("FIGNORE"))
                    .flatten()
                    .filter(|s| !s.is_empty());
                if !list && !session.bindings.ignore_case {
                    let directory = Directory::open(prefix, session)?;
                    let common = if let Some(patterns) = ignored.as_deref() {
                        let suffixes: Vec<_> =
                            patterns.split(':').filter(|s| !s.is_empty()).collect();
                        common_file(&directory, directories_only, |name| {
                            !suffixes.iter().any(|s| {
                                name.ends_with(s)
                                    || s.strip_suffix(name)
                                        .is_some_and(|rest| directory.dir.ends_with(rest))
                            })
                        })
                        .or_else(|| {
                            (!session.shopt.force_fignore)
                                .then(|| common_file(&directory, directories_only, |_| true))
                                .flatten()
                        })
                    } else {
                        common_file(&directory, directories_only, |_| true)
                    };
                    return Some((common?, Vec::new()));
                }
                let mut choices = file_names(
                    prefix,
                    directories_only,
                    !session.bindings.unmarked_dirs,
                    session,
                );
                filter_ignored(
                    &mut choices,
                    ignored.as_deref(),
                    session.shopt.force_fignore,
                );
                if session.bindings.ignore_case {
                    let locale = session.character_locale();
                    let first = choices
                        .iter()
                        .find(|s| s.starts_with(prefix))
                        .or_else(|| choices.first())?;
                    let mut len = first.len();
                    for name in &choices {
                        len = first[..len]
                            .chars()
                            .zip(name.chars())
                            .take_while(|&(a, b)| {
                                crate::pattern::same_letter(a, b, locale)
                                    || session.bindings.map_case
                                        && matches!((a, b), ('-', '_') | ('_', '-'))
                            })
                            .map(|(c, _)| c.len_utf8())
                            .sum();
                    }
                    let common = Common {
                        text: first[..len].into(),
                        unique: choices.len() == 1,
                    };
                    return Some((common, if list { choices } else { Vec::new() }));
                }
                choices
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
    let configured = !files_only
        && session.shopt.progcomp
        && session.completions.configured(command, prefix, session);
    let adjusted = if configured {
        let breaks = session.completion_breaks();
        let plain = breaks == default_word_breaks(session.shopt.hostcomplete)
            && line[context.command_start..]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b" _-./\t".contains(&b));
        (!plain).then(|| context.programmable(&breaks, session.shopt.hostcomplete))
    } else {
        None
    };
    let callback = adjusted.as_ref().unwrap_or(&context);
    let callback_prefix = &line[callback.start..cursor];
    let generated = if configured && (callback.current != 0 || command.is_none()) {
        crate::completion_spec::for_tab(session, command, callback_prefix, Some(callback))
    } else {
        None
    };
    let ordinary = || {
        if first && prefix.is_empty() && !files_only && session.shopt.no_empty_cmd_completion {
            return None;
        }
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
    if let Some(generated) = generated {
        let paths = matches!(generated.source, Source::Directories | Source::Files);
        let dirs = matches!(generated.source, Source::Directories);
        let matches = generated
            .source
            .matches(callback_prefix, session, list)
            .or_else(|| {
                paths
                    .then(|| abbreviated(callback_prefix, dirs, session))
                    .flatten()
            });
        if matches.is_some() || !generated.fallback {
            let (common, choices) = matches?;
            return Some(Completion::new(
                callback.start,
                common,
                choices,
                generated.noquote,
                !generated.nospace && cursor == line.len(),
            ));
        }
    }
    if let Some((head, rest)) = line[start..cursor].split_once('/')
        && (head.starts_with('$') || head == "~" && session.shopt.direxpand)
    {
        let value = if head == "~" {
            session.get("HOME")
        } else {
            let name = &head[1..];
            let name = name
                .strip_prefix('{')
                .and_then(|s| s.strip_suffix('}'))
                .unwrap_or(name);
            crate::parser::is_identifier(name)
                .then(|| session.get(name))
                .flatten()
        };
        if let Some(value) = value.filter(|s| !s.is_empty()) {
            let path = if session.shopt.direxpand {
                std::path::absolute(value)
                    .ok()?
                    .into_os_string()
                    .into_string()
                    .ok()?
            } else {
                value
            };
            let parent = format!("{}/", path.trim_end_matches('/'));
            // The scanner has already removed quoting from the current word.
            let rest = context
                .words
                .get(context.current)?
                .split_once('/')
                .map_or(rest, |(_, tail)| tail);
            let expanded = format!("{parent}{rest}");
            let source = if command == Some("cd") {
                Source::Directories
            } else {
                Source::Files
            };
            if let Some((mut common, mut choices)) = source.matches(&expanded, session, list) {
                let mut offset = 0;
                if !session.shopt.direxpand {
                    // Preserve only syntax typed by the user. Names read from
                    // the directory remain literal even if they contain '$'.
                    let display = &line[start..start + head.len() + 1];
                    for name in std::iter::once(&mut common.text).chain(&mut choices) {
                        if name.starts_with(&parent) {
                            name.replace_range(..parent.len(), display);
                        }
                    }
                    offset = display.len();
                    if !session.shopt.complete_fullquote {
                        // An already typed variable reference in the leaf is
                        // also eligible for expansion when full quoting is off.
                        if let Some(n) = rest.strip_prefix('$')
                            && crate::parser::is_identifier(n)
                            && common
                                .text
                                .get(offset..)
                                .is_some_and(|tail| tail.starts_with(rest))
                        {
                            offset += rest.len();
                        }
                    }
                }
                let append_space = cursor == line.len();
                return Some(Completion {
                    start,
                    replacement: quote_choice(
                        &common.text,
                        false,
                        append_space && common.unique,
                        offset,
                    ),
                    choices,
                    noquote: false,
                    append_space,
                    source_prefix: offset,
                });
            }
        }
    }
    if !files_only
        && let Some(variable) =
            variable_completion(&line[start..cursor], cursor, line.len(), session, list)
    {
        return Some(variable);
    }
    let (common, choices) = ordinary()?;
    Some(Completion::new(
        start,
        common,
        choices,
        false,
        cursor == line.len(),
    ))
}

fn variable_completion(
    raw: &str,
    cursor: usize,
    end: usize,
    session: &Session,
    list: bool,
) -> Option<Completion> {
    let (quote, raw) = match raw.as_bytes().first() {
        Some(&q @ (b'\'' | b'"')) => (Some(char::from(q)), &raw[1..]),
        _ => (None, raw),
    };
    let tail = raw.strip_prefix('$')?;
    let (braced, prefix) = tail.strip_prefix('{').map_or((false, tail), |p| (true, p));
    if !prefix.is_empty() && !crate::parser::is_identifier(prefix) {
        return None;
    }
    let names = session.variable_names(prefix);
    let mut common = Common::new(names.first()?.clone());
    if names.len() > 1 {
        let _ = common.narrow(names.last()?, prefix);
    }
    let close = |name: &mut String| {
        if braced {
            name.push('}');
        }
        if let Some(q) = quote {
            name.push(q);
        }
    };
    if common.unique {
        close(&mut common.text);
    }
    let choices = if list {
        names
            .into_iter()
            .map(|mut n| {
                close(&mut n);
                n
            })
            .collect()
    } else {
        Vec::new()
    };
    Some(Completion::new(
        cursor - prefix.len(),
        common,
        choices,
        true,
        cursor == end,
    ))
}

impl Completion {
    fn new(
        start: usize,
        common: Common,
        choices: Vec<String>,
        noquote: bool,
        append_space: bool,
    ) -> Self {
        Self {
            start,
            replacement: quote_choice(&common.text, noquote, append_space && common.unique, 0),
            source_prefix: 0,
            choices,
            noquote,
            append_space,
        }
    }

    pub fn select(&self, index: usize) -> String {
        quote_choice(
            &self.choices[index],
            self.noquote,
            self.append_space,
            self.source_prefix,
        )
    }
}

fn quote_choice(common: &str, noquote: bool, append_space: bool, source_prefix: usize) -> String {
    // Re-quote the complete word. A unique file followed by Enter must remain
    // a single literal argument even when it contains shell metacharacters.
    let mut replacement = String::new();
    for (i, c) in common.char_indices() {
        if noquote || i < source_prefix {
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
    if append_space && !common.ends_with('/') {
        replacement.push(' ');
    }
    replacement
}

type CommandVisitor<'a> = dyn FnMut(&str, Option<(&Path, u8)>) -> ControlFlow<()> + 'a;

fn visit_commands(
    prefix: &str,
    session: &Session,
    visit: &mut CommandVisitor<'_>,
) -> ControlFlow<()> {
    for name in crate::execute::builtin_names()
        .iter()
        .copied()
        .filter(|n| !session.disabled_builtins.contains(*n))
        .chain(crate::completion_spec::KEYWORDS.iter().copied())
        .chain(session.functions.keys().map(String::as_str))
        .chain(session.aliases.names().map(String::as_str))
    {
        if name.starts_with(prefix) {
            visit(name, None)?;
        }
    }
    let editing = crate::editor::preview::active();
    let mut entries = 0usize;
    let ignored = crate::execute::IgnoredExecutables::new(session);
    for dir in std::env::split_paths(&session.search_path(&[])) {
        let path = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &dir
        };
        scan(path, prefix.as_bytes(), &mut |raw, kind| {
            if editing && entries.is_multiple_of(256) {
                crate::editor::preview::scan_input();
            }
            entries += 1;
            if let Some(raw) = raw
                && !matches!(raw.to_bytes(), b"." | b"..")
                && let Ok(name) = raw.to_str()
                && (!ignored.active() || !ignored.contains(&path.join(name)))
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

pub(crate) fn group_names(prefix: &str) -> Vec<String> {
    let mut names = Vec::new();
    unsafe {
        libc::setgrent();
        while let Some(group) = libc::getgrent().as_ref() {
            let bytes = std::ffi::CStr::from_ptr(group.gr_name).to_bytes();
            if bytes.starts_with(prefix.as_bytes())
                && let Ok(name) = std::str::from_utf8(bytes)
            {
                names.push(name.to_owned());
            }
        }
        libc::endgrent();
    }
    names
}

pub(crate) fn service_names(prefix: &str) -> Vec<String> {
    use std::os::unix::fs::MetadataExt;
    struct Previous {
        file: (u64, u64, i64, i64, u64),
        prefix: String,
        names: Vec<String>,
    }
    thread_local! { static PREVIOUS: std::cell::RefCell<Option<Previous>> = const { std::cell::RefCell::new(None) }; }
    let path = "/etc/services";
    let Ok(metadata) = std::fs::metadata(path) else {
        return Vec::new();
    };
    let key = (
        metadata.dev(),
        metadata.ino(),
        metadata.ctime(),
        metadata.ctime_nsec(),
        metadata.len(),
    );
    PREVIOUS.with_borrow_mut(|last| {
        if let Some(entry) = last
            && entry.file == key
            && entry.prefix == prefix
        {
            return entry.names.clone();
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        let names: Vec<_> = text
            .lines()
            .filter_map(|line| {
                let mut fields = line.split('#').next()?.split_ascii_whitespace();
                let primary = fields.next()?;
                fields.next()?;
                Some(std::iter::once(primary).chain(fields))
            })
            .flatten()
            .filter(|name| name.starts_with(prefix))
            .map(str::to_owned)
            .collect();
        *last = Some(Previous {
            file: key,
            prefix: prefix.into(),
            names: names.clone(),
        });
        names
    })
}

pub(crate) fn signal_names(prefix: &str) -> Vec<String> {
    static ALL: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let names = ALL.get_or_init(|| {
        let mut signals: Vec<_> = crate::traps::signals().collect();
        signals.sort_unstable_by_key(|&(_, n)| n);
        let mut names: Vec<_> = signals
            .into_iter()
            .map(|(name, n)| {
                if n == 0 {
                    name.into()
                } else {
                    format!("SIG{name}")
                }
            })
            .collect();
        names.extend(["DEBUG", "ERR", "RETURN"].map(str::to_owned));
        names
    });
    names
        .iter()
        .filter(|n| n.starts_with(prefix))
        .cloned()
        .collect()
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

// Prefer a transposition over a single inserted, deleted or substituted scalar.
fn spelling_distance(a: &str, b: &str) -> Option<u8> {
    let mut a = a.chars();
    let mut b = b.chars();
    loop {
        let x = a.next();
        let y = b.next();
        if x == y {
            if x.is_none() {
                return Some(0);
            }
            continue;
        }
        let ar = a.as_str();
        let br = b.as_str();
        if x.is_some() && y.is_some() && a.next() == y && b.next() == x && a.as_str() == b.as_str()
        {
            return Some(1);
        }
        let initial = |ch: Option<char>, tail: &str, text: &str| {
            ch.and_then(|c| text.strip_prefix(c))
                .is_some_and(|s| s == tail)
        };
        return (ar == br || initial(x, ar, br) || initial(y, br, ar)).then_some(2);
    }
}

pub(crate) fn corrected_directory(input: &Path) -> Option<PathBuf> {
    let mut path = PathBuf::new();
    let mut changed = false;
    for part in input.components() {
        let std::path::Component::Normal(word) = part else {
            path.push(part);
            continue;
        };
        if path.join(word).is_dir() {
            path.push(word);
            continue;
        }
        let word = word.to_str()?;
        let parent = if path.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &path
        };
        let mut best = None;
        let mut visited = 0usize;
        let editing = crate::editor::preview::active();
        let _ = scan(parent, &[], &mut |raw, kind| {
            if editing && visited.is_multiple_of(256) {
                crate::editor::preview::poll();
            }
            visited += 1;
            if !matches!(kind, libc::DT_DIR | libc::DT_LNK | libc::DT_UNKNOWN) {
                return ControlFlow::Continue(());
            }
            let Some(bytes) = raw.map(|n| n.to_bytes()) else {
                return ControlFlow::Continue(());
            };
            if bytes.len().abs_diff(word.len()) <= 4
                && let Ok(name) = std::str::from_utf8(bytes)
                && !matches!(name, "." | "..")
                && let Some(rank) = spelling_distance(name, word)
                && best
                    .as_ref()
                    .is_none_or(|(old, s): &(u8, String)| (rank, name) < (*old, s.as_str()))
                && (kind == libc::DT_DIR || parent.join(name).is_dir())
            {
                best = Some((rank, name.to_owned()));
            }
            ControlFlow::Continue(())
        });
        path.push(best?.1);
        changed = true;
    }
    changed.then_some(path)
}

struct Directory<'a> {
    dir: std::borrow::Cow<'a, str>,
    base: &'a str,
    path: PathBuf,
    fold: Option<crate::session::CharacterLocale>,
    map_case: bool,
    show_hidden: bool,
    mark_dirs: bool,
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
        let (dir, path) = if session.shopt.dirspell
            && !dir.is_empty()
            && !path.is_dir()
            && let Some(corrected) = corrected_directory(&path)
        {
            let corrected = if session.shopt.direxpand {
                std::path::absolute(corrected).ok()?
            } else {
                corrected
            };
            (
                format!("{}/", corrected.to_str()?.trim_end_matches('/')).into(),
                corrected,
            )
        } else {
            (dir.into(), path)
        };
        Some(Self {
            dir,
            base,
            path,
            fold: session
                .bindings
                .ignore_case
                .then(|| session.character_locale()),
            map_case: session.bindings.map_case,
            show_hidden: !session.bindings.hide_dotfiles,
            mark_dirs: !session.bindings.unmarked_dirs,
        })
    }

    fn visit(&self, mut visitor: impl FnMut(&str, u8) -> ControlFlow<()>) {
        let mut entries = 0usize;
        let editing = crate::editor::preview::active();
        let prefix = if self.fold.is_some() {
            &[][..]
        } else {
            self.base.as_bytes()
        };
        let ascii_map = self.map_case
            && self.base.is_ascii()
            && self
                .fold
                .is_some_and(|locale| !matches!(locale, crate::session::CharacterLocale::Turkic));
        let base = self.base.as_bytes();
        let _ = scan(&self.path, prefix, &mut |raw, kind| {
            if editing && entries.is_multiple_of(256) {
                crate::editor::preview::scan_input();
            }
            entries += 1;
            let Some(raw) = raw else {
                return ControlFlow::Continue(());
            };
            let bytes = raw.to_bytes();
            // A folded scan cannot use the bytewise directory prefix filter.
            // Reject ASCII mismatches before decoding the rest of the filename.
            let mut matched = false;
            if ascii_map {
                let head = &bytes[..bytes.len().min(base.len())];
                if head.is_ascii() {
                    if head.len() != base.len()
                        || !base.iter().zip(head).all(|(&a, &b)| {
                            a.eq_ignore_ascii_case(&b)
                                || matches!((a, b), (b'-', b'_') | (b'_', b'-'))
                        })
                    {
                        return ControlFlow::Continue(());
                    }
                    matched = true;
                }
            }
            if !matches!(bytes, b"." | b"..")
                && (!bytes.starts_with(b".")
                    || self.base.starts_with('.')
                    || self.show_hidden && self.base.is_empty())
                && let Ok(name) = raw.to_str()
                && (matched
                    || self.fold.is_none_or(|locale| {
                        if !self.map_case {
                            return crate::pattern::folded_prefix(name, self.base, locale)
                                .is_some();
                        }
                        let mut chars = name.chars();
                        self.base.chars().all(|c| {
                            chars.next().is_some_and(|d| {
                                crate::pattern::same_letter(c, d, locale)
                                    || matches!((c, d), ('-', '_') | ('_', '-'))
                            })
                        })
                    }))
            {
                return visitor(name, kind);
            }
            ControlFlow::Continue(())
        });
    }

    fn collect(&self, directories_only: bool, mark_directories: bool, choices: &mut Vec<String>) {
        let dir = self.dir.as_ref();
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

fn filter_ignored(choices: &mut Vec<String>, suffixes: Option<&str>, force: bool) {
    if let Some(suffixes) = suffixes {
        let acceptable = |path: &String| {
            !suffixes
                .split(':')
                .any(|suffix| !suffix.is_empty() && path.trim_end_matches('/').ends_with(suffix))
        };
        if force || choices.iter().any(acceptable) {
            choices.retain(acceptable);
        }
    }
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
    let mut expanded = false;
    for part in parents.split('/').filter(|s| !s.is_empty()) {
        let mut next = Vec::new();
        for mut path in paths {
            path.push_str(part);
            let directory = Directory::open(&path, session)?;
            if directory.path.join(part).is_dir() {
                path.push('/');
                next.push(path);
            } else if !matches!(part, "." | "..") {
                expanded = true;
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
    if !expanded {
        return None;
    }
    for path in &mut paths {
        path.push_str(leaf);
    }
    let branches: Vec<_> = paths
        .iter()
        .filter_map(|p| Directory::open(p, session))
        .collect();
    let mut matches = branch_matches(&branches, dirs);
    if !dirs {
        filter_ignored(
            &mut matches,
            session.get("FIGNORE").as_deref(),
            session.shopt.force_fignore,
        );
    }
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

fn common_file(
    directory: &Directory<'_>,
    directories_only: bool,
    accept: impl Fn(&str) -> bool,
) -> Option<Common> {
    let mut first_kind = libc::DT_UNKNOWN;
    let mut common: Option<Common> = None;
    directory.visit(|name, kind| {
        if !accept(name) {
            return ControlFlow::Continue(());
        }
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
    if common.unique
        && directory.mark_dirs
        && (directories_only || directory.is_directory(&common.text, first_kind))
    {
        common.text.push('/');
    }
    common.text.insert_str(0, &directory.dir);
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
    let mut choices = crate::pattern::sorted_names(choices);
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
