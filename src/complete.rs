mod command_cache;

use crate::pattern::directory::scan;
use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use crate::Session;

pub struct Completion {
    pub start: usize,
    pub replacement: String,
    pub choices: Vec<String>,
    quoting: ChoiceQuoting,
    append_space: bool,
    unmarked: Vec<usize>,
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
    pub command_end: usize,
    quoted_command: bool,
}

#[derive(Default)]
enum Quoting {
    #[default]
    Unquoted,
    Single,
    Double,
    Escape,
    DoubleEscape,
}

#[derive(PartialEq, Eq)]
enum WordChar {
    Unquoted,
    Quoted,
    Delimiter,
}

impl Quoting {
    fn consume(&mut self, ch: char) -> WordChar {
        use Quoting::*;
        let (next, kind) = match (&*self, ch) {
            (Escape, _) => (Unquoted, WordChar::Quoted),
            (DoubleEscape, _) => (Double, WordChar::Quoted),
            (Single, '\'') | (Double, '"') => (Unquoted, WordChar::Delimiter),
            (Double, '\\') => (DoubleEscape, WordChar::Delimiter),
            (Single | Double, _) => return WordChar::Quoted,
            (Unquoted, '\'') => (Single, WordChar::Delimiter),
            (Unquoted, '"') => (Double, WordChar::Delimiter),
            (Unquoted, '\\') => (Escape, WordChar::Delimiter),
            (Unquoted, _) => return WordChar::Unquoted,
        };
        *self = next;
        kind
    }
}

// A Tab inside a group or command substitution completes its own command.
// Keep closed substitutions outside the caret in their surrounding command.
fn command_span(line: &str, cursor: usize) -> (std::ops::Range<usize>, bool) {
    let mut span = 0..line.len();
    let mut quoted = false;
    if memchr::memchr2(b'(', b'`', line.as_bytes()).is_none() {
        return (span, quoted);
    }
    struct Group {
        start: usize,
        close: char,
        command: bool,
        quoted: bool,
        outer: Quoting,
    }

    let mut groups: Vec<Group> = Vec::new();
    let mut quoting = Quoting::default();
    let mut dollar = false;
    for (i, c) in line.char_indices() {
        let unquoted = matches!(quoting, Quoting::Unquoted);
        let expandable = unquoted || matches!(quoting, Quoting::Double);
        let close = groups.last().is_some_and(|g| g.close == c);
        if close && (unquoted || c == '`' && expandable) {
            let group = groups.pop().unwrap();
            if group.command && group.start <= cursor && cursor <= i && group.start > span.start {
                span = group.start..i;
                quoted = group.quoted;
            }
            quoting = group.outer;
        } else if c == '`' && expandable || c == '(' && (unquoted || dollar) {
            let previous = i.checked_sub(1).map(|p| line.as_bytes()[p]);
            let at_boundary =
                previous.is_none_or(|b| b.is_ascii_whitespace() || b";|&".contains(&b));
            let command =
                c == '`' || line.as_bytes().get(i + 1) != Some(&b'(') && (dollar || at_boundary);
            groups.push(Group {
                start: i + 1,
                close: if c == '`' { '`' } else { ')' },
                command,
                quoted: c == '`' || !unquoted,
                outer: std::mem::take(&mut quoting),
            });
        } else {
            quoting.consume(c);
        }
        dollar = c == '$' && expandable;
    }
    for group in groups {
        if group.command && group.start <= cursor && group.start > span.start {
            span = group.start..line.len();
            quoted = group.quoted;
        }
    }
    (span, quoted)
}

impl<'a> Context<'a> {
    // The callback receives source spelling, including quotes and backslashes.
    // Readline keeps a run of punctuation as one COMP_WORDS entry, while the
    // replacement starts after that punctuation.
    pub(crate) fn programmable(&self, breaks: &str, hostname: bool) -> Self {
        let mut words = Vec::new();
        let mut token = self.command_start;
        let mut punctuation = false;
        let mut quoting = Quoting::default();
        let mut end = self.command_end;
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
        for (off, c) in self.line[self.command_start..self.command_end].char_indices() {
            let i = self.command_start + off;
            let protected = quoting.consume(c) != WordChar::Unquoted;
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
            command_end: end,
            quoted_command: self.quoted_command,
        }
    }

    /// Tokenize the current simple command for completion without evaluating it.
    pub(crate) fn scan(line: &'a str, cursor: usize) -> Option<Self> {
        line.get(..cursor)?;
        let mut words = Vec::new();
        let mut word = String::new();
        let mut quoting = Quoting::default();
        let mut in_word = false;
        let (span, quoted_command) = command_span(line, cursor);
        let mut start = span.start;
        let mut command_start = start;
        let mut command_end = span.end;
        let mut current = None;
        for (offset, c) in line[span.clone()].char_indices() {
            let i = span.start + offset;
            let kind = quoting.consume(c);
            let separator = kind == WordChar::Unquoted && (c.is_whitespace() || "|;&".contains(c));
            if i == cursor {
                current = Some(words.len());
                if !in_word && separator {
                    words.push(String::new());
                }
            }
            if separator {
                if in_word {
                    if i < cursor
                        && words.is_empty()
                        && (assignment_prefix(&line[start..i]).is_some() || &line[start..i] == "{")
                    {
                        word.clear();
                    } else {
                        words.push(std::mem::take(&mut word));
                    }
                }
                if c == '\n' || "|;&".contains(c) {
                    if i >= cursor {
                        command_end = i;
                        in_word = false;
                        break;
                    }
                    words.clear();
                    command_start = i + c.len_utf8();
                }
                if i < cursor {
                    start = i + c.len_utf8();
                    if words.is_empty() {
                        command_start = start;
                    }
                }
                in_word = false;
            } else {
                if !in_word && words.is_empty() && i < cursor {
                    command_start = i;
                }
                in_word = true;
                if kind != WordChar::Delimiter {
                    word.push(c);
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
            command_end,
            quoted_command,
        })
    }
}

// Inspect the source spelling so a quoted name or escaped equals remains a command.
fn assignment_prefix(word: &str) -> Option<usize> {
    let name = crate::word::name_chars_len(word);
    if !crate::parser::is_identifier(&word[..name]) {
        return None;
    }
    let mut rest = &word[name..];
    if let Some(subscript) = rest.strip_prefix('[') {
        let mut quote = Quoting::default();
        let mut depth = 1;
        let end = subscript
            .char_indices()
            .find(|&(_, c)| {
                if quote.consume(c) == WordChar::Unquoted {
                    match c {
                        '[' => depth += 1,
                        ']' => depth -= 1,
                        _ => {}
                    }
                }
                depth == 0
            })?
            .0;
        rest = &subscript[end + 1..];
    }
    let value = rest.strip_prefix('+').unwrap_or(rest).strip_prefix('=')?;
    Some(word.len() - value.len())
}

fn dequote(raw: &str) -> std::borrow::Cow<'_, str> {
    if !raw.contains(['\\', '\'', '"']) {
        return raw.into();
    }
    let mut quote = Quoting::default();
    raw.chars()
        .filter(|&c| quote.consume(c) != WordChar::Delimiter)
        .collect::<String>()
        .into()
}

struct Common {
    text: String,
    unique: bool,
    unmark_dirs: bool,
}

impl Common {
    fn new(text: String) -> Self {
        Self {
            text,
            unique: true,
            unmark_dirs: false,
        }
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

    fn endpoints(first: &str, last: &str, multiple: bool, lower_bound: &str) -> Self {
        let mut common = Self::new(first.to_owned());
        if multiple {
            let _ = common.narrow(last, lower_bound);
        }
        common
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
    LiteralWords {
        source: std::rc::Rc<crate::completion_spec::WordList>,
        keep_order: bool,
    },
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
            Self::LiteralWords { source, keep_order } => {
                let ifs = session.ifs();
                if keep_order {
                    let names = source.ordered_matches(prefix, &ifs);
                    if !list {
                        return Some((Common::from_names(names, prefix)?, Vec::new()));
                    }
                    let mut seen = foldhash::HashSet::default();
                    let words: Vec<_> = names
                        .filter(|name| seen.insert(*name))
                        .map(str::to_owned)
                        .collect();
                    return Some((
                        Common::from_names(words.iter().map(String::as_str), prefix)?,
                        words,
                    ));
                }
                if let Some(matches) = source.indexed(prefix, &ifs) {
                    let common = Common::endpoints(
                        source.word(*matches.first()?),
                        source.word(*matches.last()?),
                        matches.len() > 1,
                        prefix,
                    );
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
            Self::Commands if !prefix.contains('/') => {
                if !list {
                    return Some((common_command(prefix, session)?, Vec::new()));
                }
                let mut names = command_names(prefix, session, false);
                names.sort_unstable();
                names.dedup();
                names
            }
            Self::Files | Self::Directories | Self::Commands => {
                let filter = match self {
                    Self::Directories => FileFilter::Directories,
                    Self::Commands => {
                        FileFilter::Commands(crate::execute::IgnoredExecutables::new(session))
                    }
                    _ => FileFilter::Any,
                };
                let directory = Directory::open(prefix, session)?;
                let ignored = matches!(filter, FileFilter::Any)
                    .then(|| session.get("FIGNORE"))
                    .flatten()
                    .filter(|s| !s.is_empty());
                let (mut common, choices) = if list {
                    let mut choices = Vec::new();
                    directory.collect(&filter, true, false, &mut choices);
                    filter_ignored(
                        &mut choices,
                        ignored.as_deref(),
                        session.shopt.force_fignore,
                    );
                    let choices = crate::pattern::sorted_names(choices);
                    let common = if directory.fold.is_some() {
                        let mut common = Common::new(choices.first()?.clone());
                        for name in &choices[1..] {
                            if directory
                                .narrow_folded(&mut common, name, prefix)
                                .is_break()
                            {
                                break;
                            }
                        }
                        common
                    } else {
                        Common::from_names(choices.iter().map(String::as_str), prefix)?
                    };
                    (common, choices)
                } else {
                    let common = if let Some(patterns) = ignored.as_deref() {
                        let suffixes: Vec<_> =
                            patterns.split(':').filter(|s| !s.is_empty()).collect();
                        common_file(&directory, &filter, |name| {
                            !suffixes
                                .iter()
                                .any(|s| ignored_suffix(&directory.dir, name, s))
                        })
                        .or_else(|| {
                            (!session.shopt.force_fignore)
                                .then(|| common_file(&directory, &filter, |_| true))
                                .flatten()
                        })
                    } else {
                        common_file(&directory, &filter, |_| true)
                    };
                    (common?, Vec::new())
                };
                common.unmark_dirs = session.bindings.unmarked_dirs;
                if common.unmark_dirs
                    && !common.unique
                    && common.text.len() > directory.dir.len()
                    && common.text.ends_with('/')
                {
                    common.text.pop();
                }
                return Some((common, choices));
            }
        };
        // These generators return sorted choices. The first and last determine
        // the common prefix of the whole list, including multibyte boundaries.
        let common = Common::endpoints(words.first()?, words.last()?, words.len() > 1, prefix);
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
    // COMP_WORDS includes the untouched suffix, but Tab replaces only up to point.
    let partial;
    let prefix = if cursor == line.len() {
        context.words.get(context.current)?.as_str()
    } else {
        partial = dequote(&line[start..cursor]);
        partial.as_ref()
    };
    let command = (!first).then(|| context.words[0].as_str());
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
        crate::completion_spec::for_tab(session, command, callback_prefix, Some(callback), list)
    } else {
        None
    };
    if let Some(generated) = generated {
        let paths = matches!(generated.source, Source::Directories | Source::Files);
        let dirs = matches!(generated.source, Source::Directories);
        let matches = generated
            .source
            .matches(callback_prefix, session, list)
            .or_else(|| {
                paths
                    .then(|| abbreviated(callback_prefix, dirs, false, session))
                    .flatten()
            });
        if matches.is_some() || !generated.fallback {
            let (common, choices) = matches?;
            let mut completion = context.completion(
                callback.start,
                common,
                choices,
                generated.noquote,
                !generated.nospace && cursor == line.len(),
                0,
            );
            if !generated.unmarked.is_empty() {
                completion.unmarked = generated.unmarked;
            }
            return Some(completion);
        }
    }
    if !files_only {
        let raw = &line[start..cursor];
        let mut quoting = Quoting::default();
        let mut file_start = assignment_prefix(raw);
        for (offset, c) in raw.char_indices() {
            if quoting.consume(c) == WordChar::Unquoted && matches!(c, '>' | '<') {
                file_start = Some(offset + 1);
            }
        }
        if let Some(skip) = file_start {
            let prefix = dequote(&raw[skip..]);
            let (common, choices) = Source::Files.matches(&prefix, session, list)?;
            return Some(context.completion(
                start + skip,
                common,
                choices,
                false,
                cursor == line.len(),
                0,
            ));
        }
        if let Some(user) = raw.strip_prefix('~')
            && !user.is_empty()
            && !user.contains('/')
        {
            let choices: Vec<_> = user_names(user)
                .into_iter()
                .map(|s| format!("~{s}/"))
                .collect();
            let common = Common::from_names(choices.iter().map(String::as_str), raw)?;
            return Some(context.completion(start, common, choices, false, false, 0));
        }
    }
    if let Some(result) = expanded_path(&context, prefix, session, command, list, files_only) {
        return Some(result);
    }
    if !files_only && let Some(variable) = variable_completion(&context, session, list) {
        return Some(variable);
    }
    let (common, choices) = ordinary(&context, prefix, session, command, list, files_only)?;
    Some(context.completion(start, common, choices, false, cursor == line.len(), 0))
}

fn ordinary(
    context: &Context<'_>,
    prefix: &str,
    session: &Session,
    command: Option<&str>,
    list: bool,
    files_only: bool,
) -> Option<(Common, Vec<String>)> {
    let first = context.current == 0;
    if first && prefix.is_empty() && !files_only && session.shopt.no_empty_cmd_completion {
        return None;
    }
    let dirs = matches!(command, Some("cd" | "pushd")) && !files_only;
    if prefix.is_empty()
        && !files_only
        && context.current > 1
        && context.words[context.current - 1].starts_with('-')
    {
        let names =
            crate::history::arguments::candidates(session, &context.words, context.current, dirs);
        if !names.is_empty() {
            return alternatives(names, prefix);
        }
    }
    let default_source = if files_only {
        Source::Files
    } else if session.shopt.hostcomplete
        && let Some((user, host)) = prefix.rsplit_once('@')
    {
        let mut names: Vec<_> = host_names(host, session)
            .into_iter()
            .map(|s| format!("{user}@{s}"))
            .collect();
        names.sort_unstable();
        names.dedup();
        if names.is_empty() {
            Source::Files
        } else {
            Source::Words(names)
        }
    } else if first {
        Source::Commands
    } else {
        Source::Files
    };
    let files = matches!(default_source, Source::Files | Source::Directories);
    let source = if dirs && files {
        Source::Directories
    } else {
        default_source
    };
    let command_path = matches!(source, Source::Commands) && prefix.contains('/');
    source
        .matches(prefix, session, list)
        .or_else(|| {
            (files || command_path)
                .then(|| abbreviated(prefix, dirs, command_path, session))
                .flatten()
        })
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
}

fn expanded_path(
    context: &Context<'_>,
    prefix: &str,
    session: &Session,
    command: Option<&str>,
    list: bool,
    files_only: bool,
) -> Option<Completion> {
    let Context {
        line,
        start,
        cursor,
        ..
    } = *context;
    if let Some((head, rest)) = line[start..cursor].split_once('/')
        && (head.starts_with('$') || head == "~" && session.shopt.direxpand)
    {
        let value = if head == "~" {
            session.tilde_home().map(|home| {
                if home.is_empty() {
                    "/".into()
                } else {
                    home.into_owned()
                }
            })
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
            let rest = prefix.split_once('/').map_or(rest, |(_, tail)| tail);
            let expanded = format!("{parent}{rest}");
            let source = if command.is_none() && !files_only {
                Source::Commands
            } else if matches!(command, Some("cd" | "pushd")) && !files_only {
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
                return Some(context.completion(
                    start,
                    common,
                    choices,
                    false,
                    cursor == line.len(),
                    offset,
                ));
            }
        }
    }
    None
}

fn variable_completion(context: &Context<'_>, session: &Session, list: bool) -> Option<Completion> {
    let cursor = context.cursor;
    let end = context.line.len();
    let raw = &context.line[context.start..cursor];
    let (quote, raw) = match raw.as_bytes().first() {
        Some(&q @ (b'\'' | b'"')) => (Some(char::from(q)), &raw[1..]),
        _ => (None, raw),
    };
    let dollar = raw.rfind('$')?;
    if raw.as_bytes()[..dollar]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
        % 2
        == 1
    {
        return None;
    }
    let tail = &raw[dollar + 1..];
    let (braced, prefix) = tail.strip_prefix('{').map_or((false, tail), |p| (true, p));
    if !prefix.is_empty() && !crate::parser::is_identifier(prefix) {
        return None;
    }
    let names = session.variable_names(prefix);
    let mut common = Common::endpoints(names.first()?, names.last()?, names.len() > 1, prefix);
    let close = |name: &mut String| {
        if braced && !context.line[cursor..].starts_with('}') {
            name.push('}');
        }
        if cursor == end
            && let Some(q) = quote
        {
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
    Some(context.completion(
        cursor - prefix.len(),
        common,
        choices,
        true,
        cursor == end,
        0,
    ))
}

impl Context<'_> {
    fn completion(
        &self,
        start: usize,
        common: Common,
        mut choices: Vec<String>,
        noquote: bool,
        append_space: bool,
        source_prefix: usize,
    ) -> Completion {
        let append_space = append_space && !self.quoted_command;
        let mut continued = None;
        if self.cursor < self.line.len() {
            let mut quoting = Quoting::default();
            for c in self.line[start..self.cursor].chars() {
                quoting.consume(c);
            }
            continued = match quoting {
                Quoting::Single => Some('\''),
                Quoting::Double | Quoting::DoubleEscape => Some('"'),
                _ => None,
            };
        }
        let quoting = ChoiceQuoting {
            noquote,
            source_prefix,
            continued,
        };
        let mut unmarked = Vec::new();
        let mut replacement = quoting.spell(&common.text, append_space && common.unique);
        if common.unmark_dirs {
            if common.unique && replacement.ends_with('/') {
                replacement.pop();
            }
            // Keep the directory bit for picker insertion after removing its display mark.
            for (i, name) in choices.iter_mut().enumerate() {
                if name.ends_with('/') {
                    name.pop();
                    unmarked.push(i);
                }
            }
        }
        Completion {
            start,
            replacement,
            choices,
            quoting,
            append_space,
            unmarked,
        }
    }
}

impl Completion {
    pub fn select(&self, index: usize) -> String {
        self.quoting.spell(
            &self.choices[index],
            self.append_space && self.unmarked.binary_search(&index).is_err(),
        )
    }
}

struct ChoiceQuoting {
    noquote: bool,
    source_prefix: usize,
    continued: Option<char>,
}

impl ChoiceQuoting {
    fn spell(&self, common: &str, append_space: bool) -> String {
        // Keep the suffix in its original quote context when inserting at point.
        if let Some(q) = self.continued {
            let mut replacement = common[..self.source_prefix].to_owned();
            replacement.push(q);
            for c in common[self.source_prefix..].chars() {
                if !self.noquote && q == '\'' && c == '\'' {
                    replacement.push_str("'\\''");
                } else {
                    if !self.noquote && q == '"' && matches!(c, '"' | '$' | '`' | '\\') {
                        replacement.push('\\');
                    }
                    replacement.push(c);
                }
            }
            return replacement;
        }
        // Re-quote the complete word. A unique file followed by Enter must remain
        // a single literal argument even when it contains shell metacharacters.
        let mut replacement = String::new();
        for (i, c) in common.char_indices() {
            if self.noquote || i < self.source_prefix {
                replacement.push(c);
                continue;
            }
            if c == '\n' {
                replacement.push_str("'\n'");
                continue;
            }
            let leading_tilde = i == 0 && c == '~' && common.contains('/');
            let ordinary = c.is_alphanumeric()
                || "/._-+,%^]}@".contains(c)
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
}

type CommandVisitor<'a> = dyn FnMut(&str, Option<(&Path, u8)>) -> ControlFlow<()> + 'a;

fn visit_commands(
    prefix: &str,
    session: &Session,
    common_only: bool,
    preserve_duplicates: bool,
    visit: &mut CommandVisitor<'_>,
) -> ControlFlow<()> {
    static SORTED_BUILTINS: std::sync::OnceLock<Vec<&str>> = std::sync::OnceLock::new();
    let builtins = SORTED_BUILTINS.get_or_init(|| {
        let mut names = crate::execute::builtin_names().to_vec();
        names.sort_unstable();
        names
    });
    for name in session
        .aliases
        .names()
        .map(String::as_str)
        .chain(crate::completion_spec::KEYWORDS.iter().copied())
        .chain(session.functions.names(prefix).map(String::as_str))
        .chain(
            builtins
                .iter()
                .copied()
                .filter(|n| !session.disabled_builtins.contains(*n)),
        )
    {
        if name.starts_with(prefix) {
            visit(name, None)?;
        }
    }
    let editing = crate::editor::preview::active();
    let mut entries = 0usize;
    let mut checked_fd = false;
    let ignored = crate::execute::IgnoredExecutables::new(session);
    let search = session.search_path(&[]);
    use std::os::unix::ffi::OsStrExt;
    let mut seen = foldhash::HashSet::default();
    for (position, component) in search.as_bytes().split(|&b| b == b':').enumerate() {
        let path = Path::new(std::ffi::OsStr::from_bytes(if component.is_empty() {
            b"."
        } else {
            component
        }));
        if !preserve_duplicates && !seen.insert(path.as_os_str().as_bytes()) {
            continue;
        }
        let mut candidate = |raw: Option<&std::ffi::CStr>, kind| {
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
        };
        if position < command_cache::MAX_DIRECTORIES {
            command_cache::scan(
                path,
                prefix.as_bytes(),
                &mut checked_fd,
                common_only,
                editing,
                &mut candidate,
            )?;
        } else {
            scan(path, prefix.as_bytes(), &mut candidate)?;
        }
    }
    ControlFlow::Continue(())
}

fn executable(
    candidate: &mut crate::execute::Candidate,
    name: &str,
    entry: Option<(&Path, u8)>,
) -> bool {
    entry.is_none_or(|(path, kind)| {
        matches!(kind, libc::DT_REG | libc::DT_LNK | libc::DT_UNKNOWN)
            && candidate.join(path, name).is_some_and(|path| {
                #[cfg(target_os = "linux")]
                if kind == libc::DT_REG {
                    return unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 };
                }
                crate::execute::check_candidate(path).unwrap_or(false)
            })
    })
}

fn common_command(prefix: &str, session: &Session) -> Option<Common> {
    let mut common: Option<Common> = None;
    let mut candidate = crate::execute::Candidate::new();
    let _ = visit_commands(prefix, session, true, false, &mut |name, entry| {
        if let Some(known) = &mut common {
            // Only a name that could shorten the prefix or change uniqueness
            // needs a filesystem permission check.
            if known.could_change(name) && executable(&mut candidate, name, entry) {
                return known.narrow(name, prefix);
            }
        } else if executable(&mut candidate, name, entry) {
            common = Some(Common::new(name.to_owned()));
        }
        ControlFlow::Continue(())
    });
    common
}

fn collect_name(raw: &std::ffi::CStr, prefix: &str, names: &mut Vec<String>) {
    if raw.to_bytes().starts_with(prefix.as_bytes())
        && let Ok(name) = raw.to_str()
    {
        names.push(name.to_owned());
    }
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
            collect_name(
                std::ffi::CStr::from_ptr((*entry).pw_name),
                prefix,
                &mut names,
            );
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
            collect_name(std::ffi::CStr::from_ptr(group.gr_name), prefix, &mut names);
        }
        libc::endgrent();
    }
    names
}

pub(crate) fn service_names(prefix: &str) -> Vec<String> {
    use std::os::unix::fs::MetadataExt;
    struct Previous {
        file: (u64, u64, i64, i64, u64),
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
        {
            return entry
                .names
                .iter()
                .filter(|name| name.starts_with(prefix))
                .cloned()
                .collect();
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
            .map(str::to_owned)
            .collect();
        let matching = names
            .iter()
            .filter(|name| name.starts_with(prefix))
            .cloned()
            .collect();
        *last = Some(Previous { file: key, names });
        matching
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

pub fn command_names(prefix: &str, session: &Session, preserve_duplicates: bool) -> Vec<String> {
    let mut names = Vec::new();
    if prefix.contains('/') {
        if let Some(dir) = Directory::open(prefix, session) {
            let filter = FileFilter::Commands(crate::execute::IgnoredExecutables::new(session));
            dir.collect(&filter, false, true, &mut names);
        }
        return names;
    }
    let mut candidate = crate::execute::Candidate::new();
    let _ = visit_commands(
        prefix,
        session,
        false,
        preserve_duplicates,
        &mut |name, entry| {
            if executable(&mut candidate, name, entry) {
                names.push(name.to_owned());
            }
            ControlFlow::Continue(())
        },
    );
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
                crate::editor::preview::poll_typeahead();
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
}

enum FileFilter {
    Any,
    Directories,
    Commands(crate::execute::IgnoredExecutables),
}

impl FileFilter {
    fn files(directories_only: bool) -> Self {
        if directories_only {
            Self::Directories
        } else {
            Self::Any
        }
    }

    fn accepts(
        &self,
        dir: &Directory<'_>,
        name: &str,
        kind: u8,
        is_dir: bool,
        candidate: &mut crate::execute::Candidate,
    ) -> bool {
        match self {
            Self::Any => true,
            Self::Directories => is_dir,
            Self::Commands(ignore) => {
                is_dir
                    || ((!ignore.active() || !ignore.contains(&dir.path.join(name)))
                        && executable(candidate, name, Some((&dir.path, kind))))
            }
        }
    }
}

impl<'a> Directory<'a> {
    fn open(prefix: &'a str, session: &Session) -> Option<Self> {
        let (dir, base) = prefix
            .rfind('/')
            .map_or(("", prefix), |i| prefix.split_at(i + 1));
        let path = match dir.strip_prefix("~/") {
            Some(rest) => session.tilde_path(rest)?,
            None if let Some(name) = dir.strip_prefix('~') => {
                let (name, rest) = name.split_once('/')?;
                Path::new(&crate::word::named_home(name)?).join(rest.trim_start_matches('/'))
            }
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
        })
    }

    fn visit(&self, dot_entries: bool, mut visitor: impl FnMut(&str, u8) -> ControlFlow<()>) {
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
            if (dot_entries && self.base.starts_with('.') || !matches!(bytes, b"." | b".."))
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

    // Match the spelling that the sorted list would choose: prefer an exact
    // typed prefix, then byte order. Different folded scalars may have different widths.
    fn narrow_folded(&self, common: &mut Common, name: &str, prefix: &str) -> ControlFlow<()> {
        let locale = self.fold.unwrap();
        let mut old_len = 0;
        let mut new_len = 0;
        for (a, b) in common.text.chars().zip(name.chars()) {
            if !(crate::pattern::same_letter(a, b, locale)
                || self.map_case && matches!((a, b), ('-', '_') | ('_', '-')))
            {
                break;
            }
            old_len += a.len_utf8();
            new_len += b.len_utf8();
        }
        let old = &common.text[..old_len];
        let new = &name[..new_len];
        let new_exact = new.starts_with(prefix);
        let old_exact = old.starts_with(prefix);
        if new_exact && !old_exact || new_exact == old_exact && new < old {
            common.text.clear();
            common.text.push_str(new);
        } else {
            common.text.truncate(old_len);
        }
        common.unique = false;
        if common.text == prefix {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    fn collect(
        &self,
        filter: &FileFilter,
        mark_directories: bool,
        dot_entries: bool,
        choices: &mut Vec<String>,
    ) {
        let dir = self.dir.as_ref();
        let mut candidate = crate::execute::Candidate::new();
        self.visit(dot_entries, |name, kind| {
            let is_dir = (mark_directories || !matches!(filter, FileFilter::Any))
                && self.is_directory(name, kind);
            if !filter.accepts(self, name, kind, is_dir, &mut candidate) {
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

fn ignored_suffix(dir: &str, name: &str, suffix: &str) -> bool {
    !suffix.is_empty()
        && (name.ends_with(suffix)
            || suffix
                .strip_suffix(name)
                .is_some_and(|rest| dir.ends_with(rest)))
}

fn filter_ignored(choices: &mut Vec<String>, suffixes: Option<&str>, force: bool) {
    if let Some(suffixes) = suffixes {
        let acceptable = |path: &String| {
            !suffixes
                .split(':')
                .any(|suffix| ignored_suffix("", path.trim_end_matches('/'), suffix))
        };
        if force || choices.iter().any(acceptable) {
            choices.retain(acceptable);
        }
    }
}

fn abbreviated(
    prefix: &str,
    dirs: bool,
    command_path: bool,
    session: &Session,
) -> Option<(Common, Vec<String>)> {
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
                directory.visit(false, |name, kind| {
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
    let mut matches = if command_path {
        let filter = FileFilter::Commands(crate::execute::IgnoredExecutables::new(session));
        let mut choices = Vec::new();
        for branch in &branches {
            branch.collect(&filter, true, false, &mut choices);
        }
        choices
    } else {
        branch_matches(&branches, dirs)
    };
    if !dirs && !command_path {
        filter_ignored(
            &mut matches,
            session.get("FIGNORE").as_deref(),
            session.shopt.force_fignore,
        );
    }
    matches.sort_unstable();
    matches.dedup();
    let (mut common, matches) = alternatives(matches, prefix)?;
    common.unmark_dirs = session.bindings.unmarked_dirs;
    Some((common, matches))
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
            d.collect(&FileFilter::files(dirs), true, false, &mut choices);
        }
        return choices;
    }
    let next = AtomicUsize::new(0);
    let scan = || {
        let mut choices = Vec::new();
        while let Some(d) = branches.get(next.fetch_add(1, Ordering::Relaxed)) {
            d.collect(&FileFilter::files(dirs), true, false, &mut choices);
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
                Err(mpsc::RecvTimeoutError::Timeout) => crate::editor::preview::poll_typeahead(),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        matches
    })
}

fn common_file(
    directory: &Directory<'_>,
    filter: &FileFilter,
    accept: impl Fn(&str) -> bool,
) -> Option<Common> {
    let mut first_kind = libc::DT_UNKNOWN;
    let mut common: Option<Common> = None;
    let mut candidate = crate::execute::Candidate::new();
    let mut marked = String::new();
    directory.visit(false, |name, kind| {
        if !accept(name) {
            return ControlFlow::Continue(());
        }
        if directory.fold.is_none() && common.as_ref().is_some_and(|c| !c.could_change(name)) {
            return ControlFlow::Continue(());
        }
        let is_dir = (directory.fold.is_some() || !matches!(filter, FileFilter::Any))
            && directory.is_directory(name, kind);
        if !filter.accepts(directory, name, kind, is_dir, &mut candidate) {
            return ControlFlow::Continue(());
        }
        // A folded common prefix can span distinct names, including their final slash.
        let name = if directory.fold.is_some() && is_dir {
            marked.clear();
            marked.push_str(name);
            marked.push('/');
            marked.as_str()
        } else {
            name
        };
        if let Some(known) = &mut common {
            if directory.fold.is_some() {
                return directory.narrow_folded(known, name, directory.base);
            }
            return known.narrow(name, directory.base);
        }
        first_kind = kind;
        common = Some(Common::new(name.to_owned()));
        ControlFlow::Continue(())
    });
    let mut common = common?;
    if common.unique && directory.fold.is_none() && directory.is_directory(&common.text, first_kind)
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
    let mut choices = generated_files(prefix, directories_only, mark_directories, false, session);
    choices = crate::pattern::sorted_names(choices);
    choices.dedup();
    choices
}

pub(crate) fn generated_files(
    prefix: &str,
    directories_only: bool,
    mark_directories: bool,
    dot_entries: bool,
    session: &Session,
) -> Vec<String> {
    let Some(directory) = Directory::open(prefix, session) else {
        return Vec::new();
    };
    let mut choices = Vec::new();
    directory.collect(
        &FileFilter::files(directories_only),
        mark_directories,
        dot_entries,
        &mut choices,
    );
    choices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_scanners_share_quoted_word_boundaries() {
        let line = r#"ignore; cmd '' 'x|y' "a\" b" one\ two p\:q end"#;
        let context = Context::scan(line, line.len()).unwrap();
        assert_eq!(
            context.words,
            ["cmd", "", "x|y", "a\" b", "one two", "p:q", "end"]
        );
        let callback = context.programmable(default_word_breaks(true), true);
        assert_eq!(
            callback.words,
            [
                "cmd",
                "''",
                "'x|y'",
                r#""a\" b""#,
                r"one\ two",
                r"p\:q",
                "end"
            ]
        );
        assert_eq!(
            (context.current, context.start),
            (callback.current, callback.start)
        );
        assert_eq!(&line[callback.start..], "end");
    }

    #[test]
    fn common_prefix_respects_utf8_and_duplicate_names() {
        let same = Common::from_names(["雪", "雪"].into_iter(), "").unwrap();
        assert!(same.unique);
        let split = Common::from_names(["x雪", "x雲"].into_iter(), "").unwrap();
        assert!(!split.unique);
        assert_eq!(split.text, "x");
    }
}
