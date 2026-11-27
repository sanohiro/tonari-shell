use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::Path;
use std::rc::Rc;

use crate::{Session, complete, execute, parser, pattern};

pub(crate) const KEYWORDS: &[&str] = &[
    "!", "[[", "]]", "case", "coproc", "do", "done", "elif", "else", "esac", "fi", "for",
    "function", "if", "in", "select", "then", "time", "until", "while", "{", "}",
];
macro_rules! actions {
    ($( $kind:ident => ($name:literal, $flag:literal $(, $short:literal)?) ),+ $(,)?) => {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum CompAction { $( $kind ),+ }
        impl CompAction {
            const ALL: &[(Self, &'static str)] = &[$((Self::$kind, $flag)),+];
            fn parse(name: &str) -> Option<Self> {
                match name { $($name => Some(Self::$kind),)+ _ => None }
            }
            fn short(flag: char) -> Option<Self> {
                match flag { $($( $short => Some(Self::$kind), )?)+ _ => None }
            }
        }
    }
}

actions! {
    Alias => ("alias", "-a", 'a'),
    Builtin => ("builtin", "-b", 'b'),
    Command => ("command", "-c", 'c'),
    Directory => ("directory", "-d", 'd'),
    Export => ("export", "-e", 'e'),
    File => ("file", "-f", 'f'),
    Group => ("group", "-g", 'g'),
    Job => ("job", "-j", 'j'),
    Keyword => ("keyword", "-k", 'k'),
    Service => ("service", "-s", 's'),
    User => ("user", "-u", 'u'),
    Variable => ("variable", "-v", 'v'),
    Arrayvar => ("arrayvar", "-A arrayvar"),
    Binding => ("binding", "-A binding"),
    Disabled => ("disabled", "-A disabled"),
    Enabled => ("enabled", "-A enabled"),
    Function => ("function", "-A function"),
    Helptopic => ("helptopic", "-A helptopic"),
    Hostname => ("hostname", "-A hostname"),
    Running => ("running", "-A running"),
    Setopt => ("setopt", "-A setopt"),
    Shopt => ("shopt", "-A shopt"),
    Signal => ("signal", "-A signal"),
    Stopped => ("stopped", "-A stopped"),
}

macro_rules! options {
    ($( $kind:ident => $name:literal ),+ $(,)?) => {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum CompOption { $( $kind ),+ }
        impl CompOption {
            const ALL: &[Self] = &[$(Self::$kind),+];
            fn parse(name: &str) -> Option<Self> {
                match name { $($name => Some(Self::$kind),)+ _ => None }
            }
            fn name(self) -> &'static str { match self { $(Self::$kind => $name),+ } }
        }
    }
}

options! {
    Bashdefault => "bashdefault",
    Default => "default",
    Dirnames => "dirnames",
    Filenames => "filenames",
    Fullquote => "fullquote",
    Noquote => "noquote",
    Nosort => "nosort",
    Nospace => "nospace",
    Plusdirs => "plusdirs",
    Typeahead => "typeahead",
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct CompOptions(u16);

impl CompOptions {
    fn contains(self, option: CompOption) -> bool {
        self.0 & (1 << option as u8) != 0
    }
    fn set(&mut self, option: CompOption, enabled: bool) {
        let bit = 1 << option as u8;
        if enabled {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }
}

#[derive(Clone, Default)]
pub struct Registry {
    specs: BTreeMap<Target<'static>, Spec>,
    active: Option<CompOptions>,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct Spec {
    actions: Vec<CompAction>,
    options: CompOptions,
    words: Option<Rc<WordList>>,
    glob: Option<String>,
    function: Option<String>,
    command: Option<String>,
    prefix: Option<String>,
    suffix: Option<String>,
    filter: Option<String>,
}

pub(crate) struct WordList {
    source: String,
    literal: bool,
    queried: std::cell::Cell<bool>,
    offsets: std::cell::OnceCell<Vec<(usize, usize)>>,
}

impl PartialEq for WordList {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}
impl Eq for WordList {}

impl WordList {
    fn new(source: String) -> Self {
        let literal = !source.contains(['\'', '"', '\\', '$', '`', '~', '{', '}']);
        Self {
            source,
            literal,
            queried: Default::default(),
            offsets: Default::default(),
        }
    }

    pub(crate) fn literal_matches<'a>(
        &'a self,
        prefix: &'a str,
        ifs: &'a str,
    ) -> impl Iterator<Item = &'a str> {
        let default = ifs == crate::word::DEFAULT_IFS;
        let mut ordinary = default.then(|| crate::word::default_fields(&self.source));
        let mut custom = self
            .source
            .split(move |c| ifs.contains(c))
            .filter(|w| !w.is_empty());
        std::iter::from_fn(move || match &mut ordinary {
            Some(words) => words.next(),
            None => custom.next(),
        })
        .filter(move |word| word.starts_with(prefix))
    }

    // Search selective prefixes directly in declaration order. A hit is a
    // candidate only at a field boundary. A separator-free needle cannot
    // overlap a valid match in the following field.
    pub(crate) fn ordered_matches<'a>(
        &'a self,
        prefix: &'a str,
        ifs: &'a str,
    ) -> impl Iterator<Item = &'a str> {
        let separator = |b| matches!(b, b' ' | b'\t' | b'\n');
        let mut hits = (ifs == crate::word::DEFAULT_IFS
            && prefix.len() >= 4
            && self.source.len() >= 8192
            && !prefix.bytes().any(separator))
        .then(|| memchr::memmem::find_iter(self.source.as_bytes(), prefix.as_bytes()));
        let mut fields = self.literal_matches(prefix, ifs);
        std::iter::from_fn(move || {
            let Some(hits) = &mut hits else {
                return fields.next();
            };
            let start = hits.find(|&i| i == 0 || separator(self.source.as_bytes()[i - 1]))?;
            let tail = &self.source[start + prefix.len()..];
            let len = memchr::memchr3(b' ', b'\t', b'\n', tail.as_bytes()).unwrap_or(tail.len());
            Some(&self.source[start..start + prefix.len() + len])
        })
    }

    // Delay indexing until a large immutable list is reused. compgen still
    // traverses the original words in declaration order and observes live IFS.
    pub(crate) fn indexed(&self, prefix: &str, ifs: &str) -> Option<&[(usize, usize)]> {
        if ifs != crate::word::DEFAULT_IFS
            || self.source.len() < 8192
            || !self.queried.replace(true)
        {
            return None;
        }
        let index = self.offsets.get_or_init(|| {
            let base = self.source.as_ptr() as usize;
            let mut words = Vec::new();
            let mut sorted = true;
            let mut previous = "";
            for word in self.literal_matches("", ifs) {
                if sorted {
                    match previous.cmp(word) {
                        std::cmp::Ordering::Equal => continue,
                        std::cmp::Ordering::Greater => sorted = false,
                        std::cmp::Ordering::Less => {}
                    }
                    previous = word;
                }
                words.push((word.as_ptr() as usize - base, word.len()));
            }
            if !sorted {
                words.sort_unstable_by_key(|&r| self.word(r));
                words.dedup_by_key(|&mut r| self.word(r));
            }
            words
        });
        let start = index.partition_point(|&r| self.word(r) < prefix);
        let matches = &index[start..];
        Some(&matches[..matches.partition_point(|&r| self.word(r).starts_with(prefix))])
    }

    pub(crate) fn word(&self, (start, len): (usize, usize)) -> &str {
        &self.source[start..start + len]
    }

    fn append_matches(
        &self,
        out: &mut Vec<String>,
        prefix: &str,
        last: u8,
        session: &mut Session,
    ) -> Result<(), crate::expansion::Error> {
        let ifs = session
            .get("IFS")
            .unwrap_or_else(|| crate::word::DEFAULT_IFS.into());
        if self.literal {
            out.extend(self.literal_matches(prefix, &ifs).map(str::to_owned));
        } else {
            let items = parser::completion_words::parse(&self.source, &ifs)
                .map_err(|_| "invalid completion word list")?;
            let captured = std::cell::Cell::new(None);
            let mut env = crate::expansion::Expansion::new(session, last, &captured);
            for word in items {
                out.extend(
                    env.fields_with_glob(word, false)?
                        .into_iter()
                        .filter(|w| w.starts_with(prefix)),
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Target<'a> {
    Command(std::borrow::Cow<'a, str>),
    Default,
    Empty,
    Initial,
}

impl Target<'_> {
    fn argument(&self) -> String {
        match self {
            Self::Command(name) if name.starts_with('-') => format!("-- {}", shell_quote(name)),
            Self::Command(name) => shell_quote(name),
            Self::Default => "-D".into(),
            Self::Empty => "-E".into(),
            Self::Initial => "-I".into(),
        }
    }
}

use crate::word::quote::shell_quote;

#[derive(Default)]
struct Request {
    spec: Spec,
    options: Vec<(bool, CompOption)>,
    operands: Vec<String>,
    special: Option<Target<'static>>,
    destination: Option<String>,
    print: bool,
    remove: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CompBuiltin {
    Complete,
    Compgen,
    Compopt,
}

impl CompBuiltin {
    fn from_name(name: &str) -> Self {
        match name {
            "complete" => Self::Complete,
            "compgen" => Self::Compgen,
            "compopt" => Self::Compopt,
            _ => unreachable!("registered completion builtin"),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Compgen => "compgen",
            Self::Compopt => "compopt",
        }
    }
}

impl Request {
    fn parse(builtin: CompBuiltin, args: &[String]) -> Result<Self, String> {
        let mut result = Self::default();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            if arg == "--" {
                result.operands.extend(args.cloned());
                break;
            }
            let plus = builtin == CompBuiltin::Compopt && arg.starts_with('+');
            if !(arg.starts_with('-') || plus) || arg.len() < 2 {
                result.operands.push(arg.clone());
                result.operands.extend(args.cloned());
                break;
            }
            let mut flags = arg[1..].chars();
            while let Some(flag) = flags.next() {
                let mut value = || {
                    if flags.as_str().is_empty() {
                        args.next()
                            .map(String::as_str)
                            .ok_or_else(|| format!("-{flag} requires an argument"))
                    } else {
                        let text = flags.as_str();
                        flags = "".chars();
                        Ok(text)
                    }
                };
                match flag {
                    'D' | 'E' | 'I' if builtin != CompBuiltin::Compgen => {
                        // Bash gives -D precedence over -E, and -E over -I.
                        let target = match flag {
                            'D' => Target::Default,
                            'E' => Target::Empty,
                            _ => Target::Initial,
                        };
                        if result.special.as_ref().is_none_or(|old| old > &target) {
                            result.special = Some(target);
                        }
                    }
                    'p' if builtin == CompBuiltin::Complete => result.print = true,
                    'r' if builtin == CompBuiltin::Complete => result.remove = true,
                    'o' => {
                        let option = value()?;
                        let option = CompOption::parse(option)
                            .ok_or_else(|| format!("unknown completion option: {option}"))?;
                        if !plus {
                            result.spec.options.set(option, true);
                        }
                        result.options.push((!plus, option));
                    }
                    _ if builtin == CompBuiltin::Compopt => {
                        return Err(format!("invalid option: {flag}"));
                    }
                    'A' => {
                        let action = value()?;
                        result.spec.actions.push(
                            CompAction::parse(action).ok_or_else(|| {
                                format!("unsupported completion action: {action}")
                            })?,
                        );
                    }
                    'W' => result.spec.words = Some(Rc::new(WordList::new(value()?.to_owned()))),
                    'V' if builtin == CompBuiltin::Compgen => {
                        let name = value()?;
                        if !crate::word::is_identifier(name) {
                            return Err(format!("{name}: invalid identifier"));
                        }
                        result.destination = Some(name.to_owned());
                    }
                    'G' => result.spec.glob = Some(value()?.to_owned()),
                    'P' => result.spec.prefix = Some(value()?.to_owned()),
                    'S' => result.spec.suffix = Some(value()?.to_owned()),
                    'X' => result.spec.filter = Some(value()?.to_owned()),
                    'F' => result.spec.function = Some(value()?.to_owned()),
                    'C' => result.spec.command = Some(value()?.to_owned()),
                    _ => result.spec.actions.push(
                        CompAction::short(flag)
                            .ok_or_else(|| format!("invalid option: -{flag}"))?,
                    ),
                }
            }
        }
        Ok(result)
    }

    fn targets(&self) -> Vec<Target<'static>> {
        self.special.clone().map_or_else(
            || {
                self.operands
                    .iter()
                    .cloned()
                    .map(|s| Target::Command(s.into()))
                    .collect()
            },
            |target| vec![target],
        )
    }
}

impl Spec {
    fn native_generator(&self) -> Option<Generated> {
        // Shell code, filters and affixes can change the words or their order.
        if self.glob.is_some()
            || self.function.is_some()
            || self.command.is_some()
            || self.prefix.is_some()
            || self.suffix.is_some()
            || self.filter.is_some()
        {
            return None;
        }
        let file_options = [
            CompOption::Filenames,
            CompOption::Dirnames,
            CompOption::Plusdirs,
        ]
        .iter()
        .any(|o| self.options.contains(*o));
        let source = match (self.actions.as_slice(), self.words.as_ref()) {
            ([], Some(list)) if list.literal && !file_options => complete::Source::LiteralWords {
                source: Rc::clone(list),
                keep_order: self.options.contains(CompOption::Nosort),
            },
            ([CompAction::File], None) => complete::Source::Files,
            ([CompAction::Directory], None) => complete::Source::Directories,
            ([CompAction::Command], None) if !file_options => complete::Source::Commands,
            _ => return None,
        };
        Some(Generated {
            source,
            noquote: self.options.contains(CompOption::Noquote),
            nospace: self.options.contains(CompOption::Nospace),
            fallback: self.options.contains(CompOption::Default)
                || self.options.contains(CompOption::Bashdefault),
        })
    }

    fn write_definition(
        &self,
        target: &Target<'_>,
        line: &mut String,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        line.clear();
        line.push_str("complete");
        for option in CompOption::ALL {
            if self.options.contains(*option) {
                line.push_str(" -o ");
                line.push_str(option.name());
            }
        }
        for &(action, flag) in CompAction::ALL {
            if self.actions.contains(&action) {
                line.push(' ');
                line.push_str(flag);
            }
        }
        for (flag, arg) in [
            ('G', self.glob.as_deref()),
            ('W', self.words.as_ref().map(|list| list.source.as_str())),
            ('P', self.prefix.as_deref()),
            ('S', self.suffix.as_deref()),
            ('X', self.filter.as_deref()),
            ('C', self.command.as_deref()),
            ('F', self.function.as_deref()),
        ] {
            if let Some(arg) = arg {
                line.push_str(" -");
                line.push(flag);
                line.push(' ');
                if flag == 'F' {
                    line.push_str(&shell_quote(arg));
                } else {
                    crate::word::quote::push_single_quote(line, arg);
                }
            }
        }
        line.push(' ');
        line.push_str(&target.argument());
        line.push('\n');
        out.write_all(line.as_bytes())
    }

    fn candidates(
        &mut self,
        session: &mut Session,
        prefix: &str,
        last: u8,
        context: Option<&complete::Context<'_>>,
    ) -> Result<(Vec<String>, bool), crate::expansion::Error> {
        let mut reload = false;
        let mut words = Vec::new();
        for &action in &self.actions {
            match action {
                CompAction::Alias => words.extend(
                    session
                        .aliases
                        .names()
                        .filter(|n| n.starts_with(prefix))
                        .cloned(),
                ),
                CompAction::Arrayvar => words.extend(
                    session
                        .array_names()
                        .into_iter()
                        .filter(|n| n.starts_with(prefix)),
                ),
                CompAction::Shopt | CompAction::Setopt => words.extend(
                    session
                        .options(action == CompAction::Setopt)
                        .map(|(k, _)| k)
                        .filter(|k| k.starts_with(prefix))
                        .map(str::to_owned),
                ),
                CompAction::Job | CompAction::Running | CompAction::Stopped => {
                    words.extend(session.jobs.completion_names(
                        match action {
                            CompAction::Job => "job",
                            CompAction::Running => "running",
                            _ => "stopped",
                        },
                        prefix,
                    ))
                }
                CompAction::User => words.extend(complete::user_names(prefix)),
                CompAction::Group => words.extend(complete::group_names(prefix)),
                CompAction::Service => words.extend(complete::service_names(prefix)),
                CompAction::Signal => words.extend(complete::signal_names(prefix)),
                CompAction::Hostname => words.extend(complete::host_names(prefix, session)),
                CompAction::Command => words.extend(complete::command_names(prefix, session)),
                CompAction::Directory | CompAction::File => words.extend(complete::file_names(
                    prefix,
                    action == CompAction::Directory,
                    false,
                    session,
                )),
                CompAction::Enabled | CompAction::Disabled | CompAction::Helptopic => {
                    words.extend(
                        execute::builtin_names()
                            .iter()
                            .copied()
                            .filter(|n| {
                                n.starts_with(prefix)
                                    && (action == CompAction::Helptopic
                                        || (action == CompAction::Disabled)
                                            == session.disabled_builtins.contains(*n))
                            })
                            .map(str::to_owned),
                    );
                }
                CompAction::Binding => words.extend(
                    crate::editor::bindings::names()
                        .filter(|n| n.starts_with(prefix))
                        .map(str::to_owned),
                ),
                CompAction::Builtin | CompAction::Keyword => {
                    let names = if action == CompAction::Builtin {
                        execute::builtin_names()
                    } else {
                        KEYWORDS
                    };
                    words.extend(
                        names
                            .iter()
                            .filter(|n| n.starts_with(prefix))
                            .map(|s| (*s).to_owned()),
                    );
                }
                CompAction::Function => words.extend(
                    session
                        .functions
                        .keys()
                        .filter(|n| n.starts_with(prefix))
                        .cloned(),
                ),
                CompAction::Variable => words.extend(session.variable_names(prefix)),
                CompAction::Export => {
                    let mut names: BTreeSet<_> = std::env::vars_os()
                        .filter_map(|(n, _)| n.into_string().ok())
                        .collect();
                    for (key, value) in session.environment_changes(&[]) {
                        if value.is_some() {
                            names.insert(key.to_owned());
                        } else {
                            names.remove(key);
                        }
                    }
                    words.extend(names.into_iter().filter(|n| n.starts_with(prefix)));
                }
            }
        }
        if let Some(glob) = &self.glob {
            words.extend(pattern::pathnames(glob, &session.shopt.glob, &|k| {
                session.get(k)
            }));
        }
        if let Some(list) = &self.words {
            list.append_matches(&mut words, prefix, last, session)?;
        }
        if let Some(func) = &self.function {
            if !session.functions.contains_key(func) {
                return Err("completion function not found".into());
            }
            let old_options = session.completions.active.replace(self.options);
            let (reply, retry) = callback(session, func, prefix, last, context);
            reload = retry;
            words.extend(reply);
            self.options = std::mem::replace(&mut session.completions.active, old_options).unwrap();
        }
        if let Some(command) = &self.command {
            words.extend(command_callback(session, command, prefix, last, context)?);
        }
        if let Some(filter) = &self.filter {
            let (invert, filter) = filter
                .strip_prefix('!')
                .map_or((false, filter.as_str()), |s| (true, s));
            let mut expanded = String::new();
            let mut chars = filter.chars();
            while let Some(c) = chars.next() {
                match c {
                    '&' => expanded.push_str(prefix),
                    '\\' => {
                        expanded.push(c);
                        if let Some(c) = chars.next() {
                            expanded.push(c);
                        }
                    }
                    _ => expanded.push(c),
                }
            }
            let extended = session.shopt.glob.extglob;
            if session.shopt.nocasematch {
                let matcher = pattern::extended::Pattern::insensitive(
                    &expanded,
                    extended,
                    session.character_locale(),
                );
                words.retain(|w| matcher.accepts(w, false) == invert);
            } else if let Some(matcher) = extended
                .then(|| pattern::extended::Pattern::new(&expanded))
                .flatten()
            {
                words.retain(|w| matcher.accepts(w, false) == invert);
            } else {
                let matcher = pattern::Matcher::new(expanded);
                words.retain(|w| matcher.matches(w) == invert);
            }
        }
        if self.prefix.is_some() || self.suffix.is_some() {
            for w in &mut words {
                if let Some(p) = &self.prefix {
                    w.insert_str(0, p);
                }
                if let Some(s) = &self.suffix {
                    w.push_str(s);
                }
            }
        }
        Ok((words, reload))
    }
}

struct CallbackContext<'a> {
    name: &'a str,
    previous: &'a str,
    line: &'a str,
    point: usize,
    key: &'static str,
}

impl<'a> CallbackContext<'a> {
    fn new(context: Option<&'a complete::Context<'a>>) -> Self {
        match context {
            Some(c) => Self {
                name: &c.words[0],
                previous: c.current.checked_sub(1).map_or("", |i| c.words[i].as_str()),
                line: c.line,
                point: c.line[..c.cursor].chars().count(),
                key: "9",
            },
            None => Self {
                name: "compgen",
                previous: "",
                line: "",
                point: 0,
                key: "0",
            },
        }
    }
}

fn command_callback(
    session: &mut Session,
    command: &str,
    prefix: &str,
    last: u8,
    context: Option<&complete::Context<'_>>,
) -> Result<Vec<String>, crate::expansion::Error> {
    let CallbackContext {
        name,
        previous,
        line,
        point,
        key,
    } = CallbackContext::new(context);
    // Only the registered command is shell source. Append the three arguments as literals.
    let mut source = command.to_owned();
    for arg in [name, prefix, previous] {
        source.push(' ');
        crate::word::quote::push_single_quote(&mut source, arg);
    }
    let program = parser::parse_final(&source).map_err(|_| "invalid completion command")?;
    let old = session.bind(&[
        ("COMP_LINE".into(), line.into()),
        ("COMP_POINT".into(), point.to_string()),
        ("COMP_TYPE".into(), key.into()),
        ("COMP_KEY".into(), key.into()),
    ]);
    let result = crate::capture_substitution(session, program, last);
    session.restore(old);
    let (text, _) = result?;
    let mut values = Vec::new();
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if c == '\n' && !text[..i].ends_with('\\') {
            if start < i {
                values.push(text[start..i].to_owned());
            }
            start = i + 1;
        }
    }
    if start < text.len() {
        values.push(text[start..].to_owned());
    }
    Ok(values)
}

fn callback(
    session: &mut Session,
    func: &str,
    prefix: &str,
    last: u8,
    context: Option<&complete::Context<'_>>,
) -> (Vec<String>, bool) {
    let old = session.save_variables(&[
        "COMP_LINE",
        "COMP_POINT",
        "COMP_WORDS",
        "COMP_CWORD",
        "COMP_TYPE",
        "COMP_KEY",
        "COMPREPLY",
    ]);
    let fields = CallbackContext::new(context);
    let args = vec![
        func.to_owned(),
        fields.name.to_owned(),
        prefix.to_owned(),
        fields.previous.to_owned(),
    ];
    session.assign("COMP_LINE", fields.line);
    session.assign("COMP_POINT", &fields.point.to_string());
    session.assign("COMP_TYPE", fields.key);
    session.assign("COMP_KEY", fields.key);
    if let Some(c) = context {
        session.assign("COMP_CWORD", &c.current.to_string());
        session.replace_array("COMP_WORDS", c.words.clone());
    } else {
        session.assign("COMP_CWORD", "-1");
        session.replace_array("COMP_WORDS", Vec::new());
    }
    session.replace_array("COMPREPLY", Vec::new());
    let flow = crate::call_function(session, args, last, false);
    let values = if matches!(flow, crate::Flow::Discard(_) | crate::Flow::Exit(_)) {
        Vec::new()
    } else {
        session.take_completion_reply()
    };
    session.restore(old);
    (values, matches!(flow, crate::Flow::Continue(124)))
}

fn set_options(
    options: &mut CompOptions,
    changes: &[(bool, CompOption)],
    target: &str,
    out: &mut dyn Write,
) -> io::Result<()> {
    if changes.is_empty() {
        write!(out, "compopt")?;
        for &opt in CompOption::ALL {
            if opt == CompOption::Typeahead && !options.contains(opt) {
                continue;
            }
            write!(
                out,
                " {}o {}",
                if options.contains(opt) { "-" } else { "+" },
                opt.name()
            )?;
        }
        writeln!(out, " {target}")?;
    } else {
        for (enable, opt) in changes {
            options.set(*opt, *enable);
        }
    }
    Ok(())
}

fn apply(
    mut request: Request,
    builtin: CompBuiltin,
    session: &mut Session,
    last: u8,
    out: &mut dyn Write,
) -> io::Result<u8> {
    if builtin == CompBuiltin::Compgen {
        let prefix = request.operands.first().map_or("", String::as_str);
        let mut words = match request.spec.candidates(session, prefix, last, None) {
            Ok((words, _)) => words,
            Err(error) => {
                diagnostic!("compgen: {error}");
                return Ok(1);
            }
        };
        let options = &request.spec.options;
        if options.contains(CompOption::Plusdirs)
            || words.is_empty() && options.contains(CompOption::Dirnames)
        {
            words.extend(complete::file_names(prefix, true, false, session));
        }
        if words.is_empty() && options.contains(CompOption::Default) {
            words = complete::file_names(prefix, false, false, session);
        }
        if let Some(name) = request.destination {
            let status = u8::from(words.is_empty());
            if let Err(error) = session.store_completions(&name, words) {
                diagnostic!("compgen: {name}: {error}");
                return Ok(1);
            }
            return Ok(status);
        }
        let capacity = match words.len() {
            0 => 0,
            1..=8 => 256,
            _ => 8192,
        };
        let mut output = io::BufWriter::with_capacity(capacity, out);
        for word in &words {
            writeln!(output, "{word}")?;
        }
        output.flush()?;
        return Ok(u8::from(words.is_empty()));
    }
    let registry = &mut session.completions.specs;
    let targets = request.targets();
    if builtin == CompBuiltin::Complete && request.remove && targets.is_empty() {
        registry.clear();
        return Ok(0);
    }
    if builtin == CompBuiltin::Complete && targets.is_empty() {
        if !request.print && request.spec != Spec::default() {
            diagnostic!("complete: a command name or a default target is required");
            return Ok(2);
        }
        let mut line = String::new();
        for (key, spec) in registry.iter() {
            spec.write_definition(key, &mut line, out)?;
        }
        return Ok(0);
    }
    if builtin == CompBuiltin::Compopt && targets.is_empty() {
        if let Some(options) = session.completions.active.as_mut() {
            if request
                .options
                .iter()
                .any(|(_, option)| *option == CompOption::Typeahead)
            {
                diagnostic!("compopt: register typeahead before the callback starts");
                return Ok(1);
            }
            set_options(options, &request.options, "", out)?;
            return Ok(0);
        }
        diagnostic!("compopt: no completion specification is active");
        return Ok(1);
    }
    let mut status = 0;
    let mut line = String::new();
    for key in targets {
        if builtin == CompBuiltin::Complete && !(request.print || request.remove) {
            registry.insert(key, request.spec.clone());
            continue;
        }
        let Some(spec) = registry.get_mut(&key) else {
            let name = builtin.name();
            diagnostic!("{name}: no completion specification for {}", key.argument());
            status = 1;
            continue;
        };
        if request.remove {
            registry.remove(&key);
        } else if builtin == CompBuiltin::Complete {
            spec.write_definition(&key, &mut line, out)?;
        } else {
            set_options(&mut spec.options, &request.options, &key.argument(), out)?;
        }
    }
    Ok(status)
}

pub fn run(name: &str, args: &[String], last: u8, session: &mut Session) -> u8 {
    let builtin = CompBuiltin::from_name(name);
    if builtin == CompBuiltin::Compgen && args.is_empty() {
        return 0;
    }
    let request = match Request::parse(builtin, args) {
        Ok(request) => request,
        Err(error) => {
            diagnostic!("{name}: {error}");
            return 2;
        }
    };
    if builtin == CompBuiltin::Complete
        && request.operands.is_empty()
        && request.special.is_none()
        && !request.remove
    {
        let mut out = crate::builtins::listing_stdout(name);
        return apply(request, builtin, session, last, &mut out)
            .and_then(|status| out.flush().map(|()| status))
            .unwrap_or(1);
    }
    apply(
        request,
        builtin,
        session,
        last,
        &mut crate::builtins::stdout_for(name),
    )
    .unwrap_or(1)
}

pub struct Generated {
    pub source: complete::Source,
    pub fallback: bool,
    pub noquote: bool,
    pub nospace: bool,
}

impl Registry {
    pub(crate) fn configured(
        &self,
        command: Option<&str>,
        prefix: &str,
        session: &Session,
    ) -> bool {
        !self.specs.is_empty()
            && (self.lookup(command, prefix).is_some()
                || (session.shopt.progcomp_alias
                    && command.is_some_and(|cmd| session.aliases.get(cmd).is_some())))
    }

    pub(crate) fn in_callback(&self) -> bool {
        self.active.is_some()
    }

    fn lookup<'a>(&'a self, command: Option<&'a str>, prefix: &str) -> Option<&'a Spec> {
        let registry = &self.specs;
        match command {
            Some(cmd) => registry
                .get(&Target::Command(cmd.into()))
                .or_else(|| {
                    cmd.rsplit_once('/')
                        .and_then(|(_, base)| registry.get(&Target::Command(base.into())))
                })
                .or_else(|| registry.get(&Target::Default)),
            None if prefix.is_empty() => registry
                .get(&Target::Empty)
                .or_else(|| registry.get(&Target::Initial)),
            None => registry.get(&Target::Initial),
        }
    }
}

pub fn for_tab(
    session: &mut Session,
    command: Option<&str>,
    prefix: &str,
    context: Option<&complete::Context<'_>>,
) -> Option<Generated> {
    if !session.shopt.progcomp || session.completions.specs.is_empty() {
        return None;
    }
    if session.completions.lookup(command, prefix).is_none()
        && session.shopt.progcomp_alias
        && let (Some(command), Some(ctx)) = (command, context)
        && session.aliases.get(command).is_some()
    {
        let mut line = ctx.line.to_owned();
        let mut cursor = ctx.cursor;
        let mut name = command.to_owned();
        let mut at = ctx.command_start;
        let mut visited = BTreeSet::new();
        while visited.insert(name.clone()) && visited.len() <= 32 {
            let alias = session.aliases.get(&name)?;
            if !line[at..].starts_with(&name) {
                return None;
            }
            line.replace_range(at..at + name.len(), alias);
            cursor = cursor - name.len() + alias.len();
            let expanded = complete::Context::scan(&line, cursor)?;
            name = expanded.words.first()?.clone();
            at = expanded.command_start;
            if session.completions.lookup(Some(&name), prefix).is_some() {
                let expanded =
                    expanded.programmable(&session.completion_breaks(), session.shopt.hostcomplete);
                return generate_for_tab(session, Some(&name), prefix, Some(&expanded));
            }
        }
        return None;
    }
    generate_for_tab(session, command, prefix, context)
}

fn generate_for_tab(
    session: &mut Session,
    command: Option<&str>,
    prefix: &str,
    context: Option<&complete::Context<'_>>,
) -> Option<Generated> {
    let initial = session.completions.lookup(command, prefix)?;
    if let Some(generated) = initial.native_generator() {
        return Some(generated);
    }
    let mut original = initial.clone();
    let mut seen = Vec::new();
    let (spec, mut words) = loop {
        let mut spec = original.clone();
        let scripted = spec.function.is_some()
            || spec.command.is_some()
            || spec.words.as_ref().is_some_and(|w| !w.literal);
        let (words, reload) = crate::editor::preview::with_input(
            !scripted || spec.options.contains(CompOption::Typeahead),
            || spec.candidates(session, prefix, 0, context),
        )
        .unwrap_or_else(|error| {
            diagnostic!("completion: {error}");
            (Vec::new(), false)
        });
        if !reload {
            break (spec, words);
        }
        // Bash completion loaders return 124 after installing the real spec.
        seen.push(original);
        if seen.len() > 32 {
            diagnostic!("completion: possible retry loop");
            break (spec, words);
        }
        original = session.completions.lookup(command, prefix)?.clone();
        if seen.contains(&original) {
            break (spec, words);
        }
    };
    let allows_default = spec.options.contains(CompOption::Default)
        || spec.options.contains(CompOption::Bashdefault);
    if spec.options.contains(CompOption::Plusdirs)
        || (words.is_empty() && spec.options.contains(CompOption::Dirnames))
    {
        words.extend(complete::file_names(prefix, true, true, session));
    }
    if words.is_empty() && allows_default {
        return None;
    }
    let filenames = spec.options.contains(CompOption::Filenames)
        || spec
            .actions
            .iter()
            .any(|a| matches!(a, CompAction::Directory | CompAction::File));
    if filenames && !session.bindings.unmarked_dirs {
        for word in &mut words {
            let directory = word
                .strip_prefix("~/")
                .and_then(|rest| session.get("HOME").map(|h| Path::new(&h).join(rest)));
            if !word.ends_with('/')
                && directory
                    .as_deref()
                    .unwrap_or_else(|| Path::new(word))
                    .is_dir()
            {
                word.push('/');
            }
        }
    }
    if spec.options.contains(CompOption::Nosort) {
        // Borrow the candidates while recording first occurrences, then compact in place.
        if words.len() > 1 {
            let mut seen = std::collections::HashSet::with_capacity(words.len());
            let keep: Vec<_> = words
                .iter()
                .map(|word| seen.insert(word.as_str()))
                .collect();
            let mut keep = keep.into_iter();
            words.retain(|_| keep.next().unwrap());
        }
    } else {
        words.sort_unstable();
        words.dedup();
    }
    Some(Generated {
        source: complete::Source::Words(words),
        fallback: false,
        // Bash callbacks may supply shell syntax, including their own trailing separator.
        noquote: spec.options.contains(CompOption::Noquote)
            || (!filenames && (spec.function.is_some() || spec.command.is_some())),
        nospace: spec.options.contains(CompOption::Nospace),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invoke(session: &mut Session, name: &str, args: &[&str]) -> (u8, String) {
        let args = args
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>();
        let builtin = CompBuiltin::from_name(name);
        let request = Request::parse(builtin, &args).unwrap();
        let mut out = Vec::new();
        let status = apply(request, builtin, session, 0, &mut out).unwrap();
        (status, String::from_utf8(out).unwrap())
    }

    #[test]
    fn ordered_prefixes_only_match_complete_fields() {
        let source = "pre雪火山猫 target-targeted\r\x0b 雪火山猫A\t雪火山猫B\n".repeat(256);
        let list = WordList::new(source);
        for ifs in [crate::word::DEFAULT_IFS, "猫", ""] {
            for prefix in ["", "target", "arget", "雪火山猫", "target-", "猫 ", "猫A\t"] {
                let ordered: Vec<_> = list.ordered_matches(prefix, ifs).collect();
                assert_eq!(
                    ordered,
                    list.literal_matches(prefix, ifs).collect::<Vec<_>>()
                );
            }
        }
    }

    #[test]
    fn fixed_word_lists_read_current_ifs_and_keep_listing_order() {
        let mut session = Session::default();
        let words = "seat😀sea😀seat😀雪";
        invoke(&mut session, "complete", &["-W", words, "demo"]);
        session.assign("IFS", "😀");
        let line = "demo se";
        let first = complete::at(line, line.len(), &mut session, false, false).unwrap();
        assert_eq!(first.replacement, "sea");
        assert!(first.choices.is_empty());
        let list = complete::at(line, line.len(), &mut session, true, false).unwrap();
        assert_eq!(list.choices, ["sea", "seat"]);
        invoke(&mut session, "compopt", &["-o", "nosort", "demo"]);
        let list = complete::at(line, line.len(), &mut session, true, false).unwrap();
        assert_eq!(list.choices, ["seat", "sea"]);
        invoke(&mut session, "compopt", &["+o", "nosort", "demo"]);
        session.assign("IFS", "");
        let list = complete::at(line, line.len(), &mut session, true, false).unwrap();
        assert_eq!(list.choices, [words]);
        session.assign("IFS", " ");
        invoke(&mut session, "complete", &["-W", "same same", "demo"]);
        let line = "demo sa";
        assert_eq!(
            complete::at(line, line.len(), &mut session, false, false)
                .unwrap()
                .replacement,
            "same "
        );
        assert_eq!(
            invoke(&mut session, "compgen", &["-W", "a a ab", "a"]).1,
            "a\na\nab\n"
        );
    }

    #[test]
    fn registered_specs_apply_on_tab_and_can_be_changed_and_removed() {
        let mut session = Session::default();
        invoke(
            &mut session,
            "complete",
            &["-W", "'two words' test", "demo"],
        );
        let line = "printf x | demo tw";
        let c = complete::at(line, line.len(), &mut session, false, false).unwrap();
        assert_eq!(&line[c.start..], "tw");
        assert_eq!(c.replacement, "two\\ words ");
        invoke(&mut session, "compopt", &["-o", "nospace", "demo"]);
        assert_eq!(
            complete::at(line, line.len(), &mut session, false, false)
                .unwrap()
                .replacement,
            "two\\ words"
        );
        assert!(
            invoke(&mut session, "complete", &["-p", "demo"])
                .1
                .contains(" -o nospace ")
        );
        invoke(&mut session, "complete", &["-r", "demo"]);
        assert!(for_tab(&mut session, Some("demo"), "tw", None).is_none());
    }

    #[test]
    fn function_completion_sees_input_and_restores_callback_variables() {
        let mut session = Session::default();
        let source = r#"choose() {
            COMPREPLY=("$1:${COMP_WORDS[1]}:$2:$3:${COMP_WORDS[3]}:$COMP_CWORD:$COMP_POINT")
            compopt -o nospace
        }"#;
        let program = crate::parser::parse(source).unwrap();
        crate::execute_program(&mut session, program, 0, false);
        session.assign("COMP_LINE", "saved");
        session.replace_array("COMPREPLY", vec!["saved".into()]);
        invoke(&mut session, "complete", &["-F", "choose", "demo"]);
        let line = "demo α be later";
        let cursor = line.find(" later").unwrap();
        let result = complete::at(line, cursor, &mut session, true, false).unwrap();
        assert_eq!(result.choices, ["demo:α:be:α:later:2:9"]);
        assert!(!result.replacement.ends_with(' '));
        assert_eq!(session.get("COMP_LINE").as_deref(), Some("saved"));
        assert_eq!(session.array_values("COMPREPLY"), ["saved"]);
    }

    #[test]
    fn callback_keeps_order_and_restores_dense_and_sparse_replies() {
        for values in [
            "(zeta alpha zeta amber)",
            "([2]=zeta [6]=alpha [8]=zeta [9]=amber)",
        ] {
            let mut session = Session::default();
            let code = format!("COMPREPLY=(saved); choose() {{ COMPREPLY={values}; }}");
            crate::execute_program(&mut session, crate::parser::parse(&code).unwrap(), 0, false);
            invoke(
                &mut session,
                "complete",
                &["-o", "nosort", "-F", "choose", "demo"],
            );
            let result = complete::at("demo ", 5, &mut session, true, false).unwrap();
            assert_eq!(result.choices, ["zeta", "alpha", "amber"]);
            assert_eq!(session.array_values("COMPREPLY"), ["saved"]);
        }
    }

    #[test]
    fn a_default_function_can_install_a_command_spec_and_retry() {
        let mut session = Session::default();
        let loader = r#"load() { complete -W 'start stop' "$1"; return 124; }"#;
        crate::execute_program(
            &mut session,
            crate::parser::parse(loader).unwrap(),
            0,
            false,
        );
        invoke(&mut session, "complete", &["-D", "-F", "load"]);
        let line = "demo sta";
        let completion = complete::at(line, line.len(), &mut session, false, false).unwrap();
        assert_eq!(completion.replacement, "start ");
        session.shopt.progcomp = false;
        assert!(for_tab(&mut session, Some("demo"), "sta", None).is_none());
    }

    #[test]
    fn wordlists_expand_at_completion_and_input_remains_data() {
        let mut session = Session::default();
        invoke(
            &mut session,
            "complete",
            &["-W", "\"$items\" $items ab;cd #hi /x*", "demo"],
        );
        session.assign("items", "two words");
        let complete::Source::Words(known) = for_tab(&mut session, Some("demo"), "", None)
            .unwrap()
            .source
        else {
            panic!("expected a word list")
        };
        assert_eq!(known, ["#hi", "/x*", "ab;cd", "two", "two words", "words"]);
        assert!(
            matches!(for_tab(&mut session, Some("demo"), "$(echo two)", None).unwrap().source,
                         complete::Source::Words(words) if words.is_empty())
        );
        assert_eq!(
            invoke(
                &mut session,
                "compgen",
                &["-W", "abc abc abd", "-X", "*d", "-P", "p", "--", "ab"]
            ),
            (0, "pabc\npabc\n".into())
        );
    }

    #[test]
    fn glob_action_and_generated_lists_preserve_their_own_matching_rules() {
        let mut session = Session::default();
        let glob = concat!(env!("CARGO_MANIFEST_DIR"), "/src/patter?.rs");
        let name = concat!(env!("CARGO_MANIFEST_DIR"), "/src/pattern.rs\n");
        assert_eq!(
            invoke(&mut session, "compgen", &["-G", glob, "different-prefix"]),
            (0, name.into())
        );
        assert_eq!(
            invoke(&mut session, "compgen", &["-G", "/absent/tonari/*.missing"]),
            (1, String::new())
        );
        assert_eq!(
            invoke(&mut session, "compgen", &["-A", "builtin", "comp"]).1,
            "compgen\ncomplete\ncompopt\n"
        );
    }
}
