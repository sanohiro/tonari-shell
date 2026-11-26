use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::Path;
use std::rc::Rc;

use crate::{Session, complete, execute, parser, pattern};

const OPTIONS: &[&str] = &[
    "bashdefault",
    "default",
    "dirnames",
    "filenames",
    "fullquote",
    "noquote",
    "nosort",
    "nospace",
    "plusdirs",
];
pub(crate) const KEYWORDS: &[&str] = &[
    "!", "[[", "]]", "case", "coproc", "do", "done", "elif", "else", "esac", "fi", "for",
    "function", "if", "in", "select", "then", "time", "until", "while", "{", "}",
];
const ACTIONS: &[&str] = &[
    "alias",
    "binding",
    "arrayvar",
    "builtin",
    "enabled",
    "disabled",
    "helptopic",
    "command",
    "directory",
    "export",
    "file",
    "function",
    "keyword",
    "job",
    "running",
    "stopped",
    "shopt",
    "setopt",
    "user",
    "hostname",
    "group",
    "service",
    "signal",
    "variable",
];

#[derive(Clone, Default)]
pub struct Registry {
    specs: BTreeMap<Target, Spec>,
    active: Option<BTreeSet<String>>,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct Spec {
    actions: Vec<String>,
    options: BTreeSet<String>,
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
        let default_ifs = ifs == " \t\n";
        self.source
            .split(move |c| {
                if default_ifs {
                    matches!(c, ' ' | '\t' | '\n')
                } else {
                    ifs.contains(c)
                }
            })
            .filter(move |word| !word.is_empty() && word.starts_with(prefix))
    }

    // Delay indexing until a large immutable list is reused. compgen still
    // traverses the original words in declaration order and observes live IFS.
    pub(crate) fn indexed(&self, prefix: &str, ifs: &str) -> Option<&[(usize, usize)]> {
        if ifs != " \t\n" || self.source.len() < 8192 || !self.queried.replace(true) {
            return None;
        }
        let index = self.offsets.get_or_init(|| {
            let base = self.source.as_ptr() as usize;
            let mut words: Vec<_> = self
                .literal_matches("", ifs)
                .map(|w| (w.as_ptr() as usize - base, w.len()))
                .collect();
            words.sort_unstable_by_key(|&r| self.word(r));
            words.dedup_by_key(|&mut r| self.word(r));
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
        let ifs = session.get("IFS").unwrap_or_else(|| " \t\n".into());
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
enum Target {
    Command(String),
    Default,
    Empty,
    Initial,
}

impl Target {
    fn argument(&self) -> String {
        match self {
            Self::Command(name) => shell_quote(name),
            Self::Default => "-D".into(),
            Self::Empty => "-E".into(),
            Self::Initial => "-I".into(),
        }
    }
}

pub(crate) fn single_quote(text: &str) -> String {
    let mut output = String::with_capacity(text.len() + 2);
    push_single_quote(&mut output, text);
    output
}

pub(crate) fn push_single_quote(output: &mut String, text: &str) {
    output.push('\'');
    let mut tail = text;
    while let Some((before, after)) = tail.split_once('\'') {
        output.push_str(before);
        output.push_str("'\\''");
        tail = after;
    }
    output.push_str(tail);
    output.push('\'');
}

fn shell_quote(text: &str) -> String {
    if !text.is_empty()
        && text
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_./-".contains(&c))
    {
        text.to_owned()
    } else {
        single_quote(text)
    }
}

#[derive(Default)]
struct Request {
    spec: Spec,
    options: Vec<(bool, String)>,
    operands: Vec<String>,
    special: Option<Target>,
    destination: Option<String>,
    print: bool,
    remove: bool,
}

impl Request {
    fn parse(builtin: &str, args: &[String]) -> Result<Self, String> {
        let mut result = Self::default();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            if arg == "--" {
                result.operands.extend(args.cloned());
                break;
            }
            let plus = builtin == "compopt" && arg.starts_with('+');
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
                            .cloned()
                            .ok_or_else(|| format!("-{flag} requires an argument"))
                    } else {
                        let text = flags.as_str().to_owned();
                        flags = "".chars();
                        Ok(text)
                    }
                };
                match flag {
                    'D' | 'E' | 'I' if builtin != "compgen" => {
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
                    'p' if builtin == "complete" => result.print = true,
                    'r' if builtin == "complete" => result.remove = true,
                    'o' => {
                        let option = value()?;
                        if !OPTIONS.contains(&option.as_str()) && option != "typeahead" {
                            return Err(format!("unknown completion option: {option}"));
                        }
                        if !plus {
                            result.spec.options.insert(option.clone());
                        }
                        result.options.push((!plus, option));
                    }
                    _ if builtin == "compopt" => return Err(format!("invalid option: {flag}")),
                    'A' => {
                        let action = value()?;
                        if !ACTIONS.contains(&action.as_str()) {
                            return Err(format!("unsupported completion action: {action}"));
                        }
                        result.spec.actions.push(action);
                    }
                    'a' | 'b' | 'c' | 'd' | 'e' | 'f' | 'k' | 'v' | 'j' | 'u' | 'g' | 's' => {
                        let action = match flag {
                            'a' => "alias",
                            'b' => "builtin",
                            'c' => "command",
                            'd' => "directory",
                            'e' => "export",
                            'f' => "file",
                            'k' => "keyword",
                            'g' => "group",
                            's' => "service",
                            'j' => "job",
                            'u' => "user",
                            _ => "variable",
                        };
                        result.spec.actions.push(action.to_owned());
                    }
                    'W' => result.spec.words = Some(Rc::new(WordList::new(value()?))),
                    'V' if builtin == "compgen" => {
                        let name = value()?;
                        if !crate::word::is_identifier(&name) {
                            return Err(format!("{name}: invalid identifier"));
                        }
                        result.destination = Some(name);
                    }
                    'G' => result.spec.glob = Some(value()?),
                    'P' => result.spec.prefix = Some(value()?),
                    'S' => result.spec.suffix = Some(value()?),
                    'X' => result.spec.filter = Some(value()?),
                    'F' => result.spec.function = Some(value()?),
                    'C' => result.spec.command = Some(value()?),
                    _ => return Err(format!("invalid option: -{flag}")),
                }
            }
        }
        Ok(result)
    }

    fn targets(&self) -> Vec<Target> {
        self.special.clone().map_or_else(
            || self.operands.iter().cloned().map(Target::Command).collect(),
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
        let file_options = ["filenames", "dirnames", "plusdirs"]
            .iter()
            .any(|o| self.options.contains(*o));
        let source = match (self.actions.as_slice(), self.words.as_ref()) {
            ([], Some(list))
                if list.literal && !file_options && !self.options.contains("nosort") =>
            {
                complete::Source::LiteralWords(Rc::clone(list))
            }
            ([action], None) if action == "file" => complete::Source::Files,
            ([action], None) if action == "directory" => complete::Source::Directories,
            ([action], None) if action == "command" && !file_options => complete::Source::Commands,
            _ => return None,
        };
        Some(Generated {
            source,
            noquote: self.options.contains("noquote"),
            nospace: self.options.contains("nospace"),
            fallback: self.options.contains("default") || self.options.contains("bashdefault"),
        })
    }

    fn definition(&self, target: &Target) -> String {
        let mut line = "complete".to_owned();
        for option in &self.options {
            line.push_str(&format!(" -o {option}"));
        }
        for (action, flag) in [
            ("alias", "-a"),
            ("builtin", "-b"),
            ("command", "-c"),
            ("directory", "-d"),
            ("export", "-e"),
            ("file", "-f"),
            ("group", "-g"),
            ("job", "-j"),
            ("keyword", "-k"),
            ("service", "-s"),
            ("user", "-u"),
            ("variable", "-v"),
            ("arrayvar", "-A arrayvar"),
            ("binding", "-A binding"),
            ("disabled", "-A disabled"),
            ("enabled", "-A enabled"),
            ("function", "-A function"),
            ("helptopic", "-A helptopic"),
            ("hostname", "-A hostname"),
            ("running", "-A running"),
            ("setopt", "-A setopt"),
            ("shopt", "-A shopt"),
            ("signal", "-A signal"),
            ("stopped", "-A stopped"),
        ] {
            if self.actions.iter().any(|value| value == action) {
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
                let quoted = if flag == 'F' {
                    shell_quote(arg)
                } else {
                    single_quote(arg)
                };
                line.push_str(&format!(" -{flag} {quoted}"));
            }
        }
        line.push(' ');
        line.push_str(&target.argument());
        line
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
        for action in &self.actions {
            match action.as_str() {
                "alias" => words.extend(
                    session
                        .aliases
                        .names()
                        .filter(|n| n.starts_with(prefix))
                        .cloned(),
                ),
                "arrayvar" => words.extend(
                    session
                        .array_names()
                        .into_iter()
                        .filter(|n| n.starts_with(prefix)),
                ),
                "shopt" | "setopt" => words.extend(
                    session
                        .options(action == "setopt")
                        .map(|(k, _)| k)
                        .filter(|k| k.starts_with(prefix))
                        .map(str::to_owned),
                ),
                "job" | "running" | "stopped" => {
                    words.extend(session.jobs.completion_names(action, prefix))
                }
                "user" => words.extend(complete::user_names(prefix)),
                "group" => words.extend(complete::group_names(prefix)),
                "service" => words.extend(complete::service_names(prefix)),
                "signal" => words.extend(complete::signal_names(prefix)),
                "hostname" => words.extend(complete::host_names(prefix, session)),
                "command" => words.extend(complete::command_names(prefix, session)),
                "directory" | "file" => words.extend(complete::file_names(
                    prefix,
                    action == "directory",
                    false,
                    session,
                )),
                "enabled" | "disabled" | "helptopic" => {
                    words.extend(
                        execute::builtin_names()
                            .iter()
                            .copied()
                            .filter(|n| {
                                n.starts_with(prefix)
                                    && (action == "helptopic"
                                        || (action == "disabled")
                                            == session.disabled_builtins.contains(*n))
                            })
                            .map(str::to_owned),
                    );
                }
                "binding" => words.extend(
                    crate::editor::bindings::names()
                        .filter(|n| n.starts_with(prefix))
                        .map(str::to_owned),
                ),
                "builtin" | "keyword" => {
                    let names = if action == "builtin" {
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
                "function" => words.extend(
                    session
                        .functions
                        .keys()
                        .filter(|n| n.starts_with(prefix))
                        .cloned(),
                ),
                "variable" => words.extend(session.variable_names(prefix)),
                "export" => {
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
                _ => unreachable!(),
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
            let old_options = session.completions.active.replace(self.options.clone());
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

fn command_callback(
    session: &mut Session,
    command: &str,
    prefix: &str,
    last: u8,
    context: Option<&complete::Context<'_>>,
) -> Result<Vec<String>, crate::expansion::Error> {
    let (name, previous, line, point, key) = context.map_or(("compgen", "", "", 0, "0"), |c| {
        let previous = c.current.checked_sub(1).map_or("", |i| c.words[i].as_str());
        (
            c.words[0].as_str(),
            previous,
            c.line,
            c.line[..c.cursor].chars().count(),
            "9",
        )
    });
    // Only the registered command is shell source. Append the three arguments as literals.
    let mut source = command.to_owned();
    for arg in [name, prefix, previous] {
        source.push(' ');
        source.push_str(&single_quote(arg));
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
    let mut args = vec![
        func.to_owned(),
        "compgen".to_owned(),
        prefix.to_owned(),
        String::new(),
    ];
    if let Some(c) = context {
        args[1] = c.words[0].clone();
        if c.current > 0 {
            args[3] = c.words[c.current - 1].clone();
        }
        session.assign("COMP_LINE", c.line);
        session.assign(
            "COMP_POINT",
            &c.line[..c.cursor].chars().count().to_string(),
        );
        session.assign("COMP_CWORD", &c.current.to_string());
        session.assign("COMP_TYPE", "9");
        session.assign("COMP_KEY", "9");
        session.replace_array("COMP_WORDS", c.words.clone());
    } else {
        session.assign("COMP_LINE", "");
        session.assign("COMP_POINT", "0");
        session.assign("COMP_CWORD", "-1");
        session.assign("COMP_TYPE", "0");
        session.assign("COMP_KEY", "0");
        session.replace_array("COMP_WORDS", Vec::new());
    }
    session.replace_array("COMPREPLY", Vec::new());
    let flow = crate::call_function(session, args, last, false);
    let values = if matches!(flow, crate::Flow::Discard(_) | crate::Flow::Exit(_)) {
        Vec::new()
    } else {
        session.array_values("COMPREPLY")
    };
    session.restore(old);
    (values, matches!(flow, crate::Flow::Continue(124)))
}

fn set_options(
    options: &mut BTreeSet<String>,
    changes: &[(bool, String)],
    target: &str,
    out: &mut dyn Write,
) -> io::Result<()> {
    if changes.is_empty() {
        write!(out, "compopt")?;
        for &opt in OPTIONS
            .iter()
            .chain(options.contains("typeahead").then_some(&"typeahead"))
        {
            write!(
                out,
                " {}o {opt}",
                if options.contains(opt) { "-" } else { "+" }
            )?;
        }
        writeln!(out, " {target}")?;
    } else {
        for (enable, opt) in changes {
            if *enable {
                options.insert(opt.clone());
            } else {
                options.remove(opt);
            }
        }
    }
    Ok(())
}

fn apply(
    mut request: Request,
    name: &str,
    session: &mut Session,
    last: u8,
    out: &mut dyn Write,
) -> io::Result<u8> {
    if name == "compgen" {
        let prefix = request.operands.first().map_or("", String::as_str);
        let mut words = match request.spec.candidates(session, prefix, last, None) {
            Ok((words, _)) => words,
            Err(error) => {
                eprintln!("tonari: compgen: {error}");
                return Ok(1);
            }
        };
        let options = &request.spec.options;
        if options.contains("plusdirs") || words.is_empty() && options.contains("dirnames") {
            words.extend(complete::file_names(prefix, true, false, session));
        }
        if words.is_empty() && options.contains("default") {
            words = complete::file_names(prefix, false, false, session);
        }
        if let Some(name) = request.destination {
            let status = u8::from(words.is_empty());
            if let Err(error) = session.store_completions(&name, words) {
                eprintln!("tonari: compgen: {name}: {error}");
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
    if name == "complete" && request.remove && targets.is_empty() {
        registry.clear();
        return Ok(0);
    }
    if name == "complete" && targets.is_empty() {
        if !request.print && request.spec != Spec::default() {
            eprintln!("tonari: complete: a command name or a default target is required");
            return Ok(2);
        }
        for (key, spec) in registry.iter() {
            writeln!(out, "{}", spec.definition(key))?;
        }
        return Ok(0);
    }
    if name == "compopt" && targets.is_empty() {
        if let Some(options) = session.completions.active.as_mut() {
            if request
                .options
                .iter()
                .any(|(_, option)| option == "typeahead")
            {
                eprintln!("tonari: compopt: register typeahead before the callback starts");
                return Ok(1);
            }
            set_options(options, &request.options, "", out)?;
            return Ok(0);
        }
        eprintln!("tonari: compopt: no completion specification is active");
        return Ok(1);
    }
    let mut status = 0;
    for key in targets {
        if name == "complete" && !(request.print || request.remove) {
            registry.insert(key, request.spec.clone());
            continue;
        }
        let Some(spec) = registry.get_mut(&key) else {
            eprintln!(
                "tonari: {name}: no completion specification for {}",
                key.argument()
            );
            status = 1;
            continue;
        };
        if request.remove {
            registry.remove(&key);
        } else if name == "complete" {
            writeln!(out, "{}", spec.definition(&key))?;
        } else {
            set_options(&mut spec.options, &request.options, &key.argument(), out)?;
        }
    }
    Ok(status)
}

pub fn run(name: &str, args: &[String], last: u8, session: &mut Session) -> u8 {
    if name == "compgen" && args.is_empty() {
        return 0;
    }
    let request = match Request::parse(name, args) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("tonari: {name}: {error}");
            return 2;
        }
    };
    apply(request, name, session, last, &mut io::stdout().lock()).unwrap_or(1)
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

    fn lookup(&self, command: Option<&str>, prefix: &str) -> Option<&Spec> {
        let registry = &self.specs;
        match command {
            Some(cmd) => registry
                .get(&Target::Command(cmd.to_owned()))
                .or_else(|| {
                    cmd.rsplit_once('/')
                        .and_then(|(_, base)| registry.get(&Target::Command(base.to_owned())))
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
            !scripted || spec.options.contains("typeahead"),
            || spec.candidates(session, prefix, 0, context),
        )
        .unwrap_or_else(|error| {
            eprintln!("tonari: completion: {error}");
            (Vec::new(), false)
        });
        if !reload {
            break (spec, words);
        }
        // Bash completion loaders return 124 after installing the real spec.
        seen.push(original);
        if seen.len() > 32 {
            eprintln!("tonari: completion: possible retry loop");
            break (spec, words);
        }
        original = session.completions.lookup(command, prefix)?.clone();
        if seen.contains(&original) {
            break (spec, words);
        }
    };
    let allows_default = spec.options.contains("default") || spec.options.contains("bashdefault");
    if spec.options.contains("plusdirs") || (words.is_empty() && spec.options.contains("dirnames"))
    {
        words.extend(complete::file_names(prefix, true, true, session));
    }
    if words.is_empty() && allows_default {
        return None;
    }
    let filenames = spec.options.contains("filenames")
        || spec.actions.iter().any(|a| a == "directory" || a == "file");
    if filenames {
        for word in &mut words {
            if !word.ends_with('/') && Path::new(word).is_dir() {
                word.push('/');
            }
        }
    }
    if spec.options.contains("nosort") {
        let mut seen = BTreeSet::new();
        words.retain(|w| seen.insert(w.clone()));
    } else {
        words.sort();
        words.dedup();
    }
    Some(Generated {
        source: complete::Source::Words(words),
        fallback: false,
        // Bash callbacks may supply shell syntax, including their own trailing separator.
        noquote: spec.options.contains("noquote")
            || (!filenames && (spec.function.is_some() || spec.command.is_some())),
        nospace: spec.options.contains("nospace"),
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
        let request = Request::parse(name, &args).unwrap();
        let mut out = Vec::new();
        let status = apply(request, name, session, 0, &mut out).unwrap();
        (status, String::from_utf8(out).unwrap())
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
