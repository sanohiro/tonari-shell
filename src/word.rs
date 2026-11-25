pub mod brace;
mod modifier;
mod replace;
use crate::parser::Statement;
use crate::pattern::{Field, quote_into};
pub use modifier::{Modification, ReplaceMode};

#[derive(Clone, Debug)]
pub enum Word {
    Literal(String),
    Expanded(Vec<Part>),
    Declaration(Vec<Part>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alternative {
    Default,
    Alternate,
    Assign,
    Error,
}

#[derive(Clone, Debug)]
pub enum Part {
    BadSubstitution(String),
    Brace(&'static str),
    BraceParameter(String),
    BraceTilde(&'static str),
    Range(Box<brace::Range>),
    Text(String),
    Pattern(String),
    Nested(Word),
    Elements {
        values: Vec<String>,
        at: bool,
        quoted: bool,
    },
    Modify {
        name: String,
        operation: Box<Modification>,
        quoted: bool,
    },
    Trim {
        name: String,
        pattern: Box<Word>,
        prefix: bool,
        longest: bool,
        quoted: bool,
    },
    Arithmetic {
        expression: String,
        quoted: bool,
    },
    Parameter {
        name: String,
        quoted: bool,
    },
    Tilde(String),
    Unquoted(String),
    Process {
        body: Vec<Statement>,
        input: bool,
    },
    Substitution {
        body: Vec<Statement>,
        quoted: bool,
    },
    Alternative {
        name: String,
        check_empty: bool,
        op: Alternative,
        word: Box<Word>,
        quoted: bool,
    },
}

pub fn is_identifier(name: &str) -> bool {
    let mut chars = name.bytes();
    matches!(chars.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

impl Word {
    pub fn as_literal(&self) -> Option<&str> {
        match self {
            Self::Literal(s) => Some(s),
            _ => None,
        }
    }

    pub fn declaration(self) -> Self {
        match self {
            Self::Expanded(parts) => Self::Declaration(parts),
            word => word,
        }
    }

    pub fn remove_assignment_prefix(&mut self, length: usize) {
        let text = match self {
            Self::Literal(s) => s,
            Self::Expanded(parts) | Self::Declaration(parts) => {
                let Part::Text(s) = &mut parts[0] else {
                    unreachable!()
                };
                s
            }
        };
        text.drain(..length);
    }

    /// Resolve in lexical order so an arithmetic assignment is visible to the
    /// following expansion and a skipped parameter alternative has no effects.
    pub fn resolve(
        &mut self,
        env: &mut crate::expansion::Expansion<'_>,
    ) -> Result<(), crate::expansion::Error> {
        let parts = match self {
            Self::Literal(_) => return Ok(()),
            Self::Expanded(parts) | Self::Declaration(parts) => parts,
        };
        let literal = |text, quoted| {
            if quoted {
                Part::Text(text)
            } else {
                Part::Unquoted(text)
            }
        };
        for part in parts.iter_mut() {
            if let Part::BadSubstitution(text) = part {
                return Err(crate::expansion::Error::fatal(
                    format!("{text}: bad substitution"),
                    1,
                ));
            }
            if let Part::BraceParameter(name) = part {
                *part = Part::Parameter {
                    name: std::mem::take(name),
                    quoted: false,
                };
            }
            if let Part::Parameter { name, quoted } = part
                && let Some(reference) = name.strip_prefix('!')
                && env.session.is_nameref(reference)
            {
                let target = env.session.reference_name(reference)?.into_owned();
                *part = literal(target, *quoted);
                continue;
            }
            if let Part::Parameter { name, .. }
            | Part::Alternative { name, .. }
            | Part::Trim { name, .. }
            | Part::Modify { name, .. } = part
            {
                if let Some(reference) = name
                    .strip_prefix('!')
                    .filter(|s| !s.is_empty() && array_elements_name(name).is_none())
                {
                    let target = modifier::value(reference, env)?.ok_or_else(|| {
                        crate::expansion::Error::fatal(
                            format!("{reference}: invalid indirect expansion"),
                            1,
                        )
                    })?;
                    if !(is_identifier(&target)
                        || array_elements_name(&target).is_some()
                        || (!target.is_empty() && target.bytes().all(|b| b.is_ascii_digit()))
                        || matches!(target.as_str(), "@" | "*" | "#" | "?" | "-" | "$" | "!")
                        || env.session.valid_target(&target))
                    {
                        return Err(crate::expansion::Error::fatal(
                            format!("{target}: invalid variable name"),
                            1,
                        ));
                    }
                    *name = target;
                }
            }
            match part {
                Part::Modify {
                    name,
                    operation,
                    quoted,
                } => {
                    *part = operation.evaluate(name, *quoted, env)?;
                }
                Part::Trim {
                    name,
                    pattern,
                    prefix,
                    longest,
                    quoted,
                } => {
                    let values = modifier::Values::read(name, env)?;
                    let pattern = env.pattern(std::mem::replace(
                        pattern.as_mut(),
                        Word::Literal(String::new()),
                    ))?;
                    *part = values.map(*quoted, |v| {
                        crate::pattern::trim(
                            v,
                            &pattern,
                            *prefix,
                            *longest,
                            env.session.shopt.glob.extglob,
                        )
                    });
                }
                Part::Arithmetic { expression, quoted } => {
                    *part = literal(env.arithmetic(expression)?.to_string(), *quoted);
                }
                Part::Process { body, input } => {
                    *part = Part::Text(env.process(std::mem::take(body), *input)?);
                }
                Part::Substitution { body, quoted } => {
                    *part = literal(env.capture(std::mem::take(body))?, *quoted);
                }
                Part::Alternative {
                    name,
                    check_empty,
                    op,
                    word,
                    quoted,
                } => {
                    let value = modifier::value(name, env)?;
                    let set = value
                        .as_ref()
                        .is_some_and(|s| !*check_empty || !s.is_empty());
                    let use_word_if_set = *op == Alternative::Alternate;
                    if set == use_word_if_set {
                        word.resolve(env)?;
                        if matches!(*op, Alternative::Assign | Alternative::Error) {
                            let value = (**word)
                                .clone()
                                .scalar(&|k| env.lookup(k), env.session.positional.as_slice());
                            if *op == Alternative::Error {
                                let default = if *check_empty {
                                    "parameter null or not set"
                                } else {
                                    "parameter not set"
                                };
                                return Err(env.missing(
                                    name,
                                    if value.is_empty() { default } else { &value },
                                ));
                            }
                            env.assign_parameter(name, &value)?;
                            *part = literal(value, *quoted);
                            continue;
                        }
                        *part = Part::Nested(std::mem::replace(
                            word.as_mut(),
                            Word::Literal(String::new()),
                        ));
                    } else {
                        let text = if use_word_if_set { None } else { value };
                        *part = literal(text.unwrap_or_default(), *quoted);
                    }
                }
                Part::Parameter { name, quoted }
                    if !matches!(name.as_str(), "@" | "*")
                        && array_elements_name(name).is_none() =>
                {
                    let value = modifier::value(name, env)?;
                    if env.session.nounset && value.is_none() {
                        return Err(env.missing(name, "unbound variable"));
                    }
                    *part = literal(
                        value.unwrap_or_else(|| {
                            if name.starts_with('#') {
                                "0".into()
                            } else {
                                String::new()
                            }
                        }),
                        *quoted,
                    );
                }
                _ => {}
            }
        }
        if let [Part::Text(value)] = parts.as_mut_slice() {
            // A resolved quoted scalar already owns the complete field.
            *self = Self::Literal(std::mem::take(value));
        }
        Ok(())
    }

    pub fn scalar(
        mut self,
        lookup: &dyn Fn(&str) -> Option<String>,
        positional: &[String],
    ) -> String {
        if let Self::Literal(s) = self {
            return s;
        }
        if let Self::Expanded(parts) | Self::Declaration(parts) = &mut self
            && let [Part::Unquoted(value) | Part::Text(value)] = parts.as_mut_slice()
        {
            return std::mem::take(value);
        }
        let mut output = Output::Scalar(String::new());
        self.expand_into(&mut output, lookup, positional, "");
        let Output::Scalar(text) = output else {
            unreachable!()
        };
        text
    }

    #[cfg(test)]
    pub fn fields(
        self,
        lookup: &dyn Fn(&str) -> Option<String>,
        positional: &[String],
    ) -> Vec<String> {
        let mut words = Vec::new();
        self.append_fields(lookup, positional, Some(&Default::default()), &mut words)
            .unwrap();
        words
    }

    pub fn append_fields(
        mut self,
        lookup: &dyn Fn(&str) -> Option<String>,
        positional: &[String],
        globbing: Option<&crate::pattern::GlobOptions>,
        words: &mut Vec<String>,
    ) -> Result<(), crate::expansion::Error> {
        if let Self::Literal(s) = self {
            words.push(s);
            return Ok(());
        }
        // A resolved quoted scalar, including the empty string, is exactly one field.
        if let Self::Expanded(parts) | Self::Declaration(parts) = &mut self
            && let [Part::Text(text)] = parts.as_mut_slice()
        {
            words.push(std::mem::take(text));
            return Ok(());
        }
        let ifs = lookup("IFS");
        let mut output = Output::Fields(Fields {
            lookup,
            globbing,
            words,
            current: Field::default(),
            error: None,
            present: false,
            whitespace: false,
        });
        self.expand_into(
            &mut output,
            lookup,
            positional,
            ifs.as_deref().unwrap_or(" \t\n"),
        );
        let Output::Fields(mut fields) = output else {
            unreachable!()
        };
        fields.finish_field();
        fields.error.map_or(Ok(()), Err)
    }

    pub fn pattern(self, lookup: &dyn Fn(&str) -> Option<String>, positional: &[String]) -> String {
        let mut output = Output::Pattern(String::new(), PatternKind::Glob);
        self.expand_into(&mut output, lookup, positional, "");
        let Output::Pattern(text, _) = output else {
            unreachable!()
        };
        text
    }

    pub fn extended_pattern(
        self,
        lookup: &dyn Fn(&str) -> Option<String>,
        positional: &[String],
    ) -> String {
        let mut output = Output::Pattern(String::new(), PatternKind::Extended);
        self.expand_into(&mut output, lookup, positional, "");
        let Output::Pattern(text, _) = output else {
            unreachable!()
        };
        text
    }

    fn replacement(self, lookup: &dyn Fn(&str) -> Option<String>, positional: &[String]) -> String {
        let mut output = Output::Pattern(String::new(), PatternKind::Replacement);
        self.expand_into(&mut output, lookup, positional, "");
        let Output::Pattern(text, _) = output else {
            unreachable!()
        };
        text
    }
    pub fn regex(self, lookup: &dyn Fn(&str) -> Option<String>, positional: &[String]) -> String {
        let mut output = Output::Pattern(String::new(), PatternKind::Regex);
        self.expand_into(&mut output, lookup, positional, "");
        let Output::Pattern(text, _) = output else {
            unreachable!()
        };
        text
    }

    fn expand_into(
        self,
        output: &mut Output,
        lookup: &dyn Fn(&str) -> Option<String>,
        positional: &[String],
        ifs: &str,
    ) {
        let parts = match self {
            Self::Literal(s) => {
                output.append(&s, true, ifs);
                return;
            }
            Self::Declaration(_) if matches!(output, Output::Fields(_)) => {
                output.append(&self.scalar(lookup, positional), true, ifs);
                return;
            }
            Self::Expanded(parts) | Self::Declaration(parts) => parts,
        };
        for part in parts {
            let part = match part {
                Part::BraceParameter(name) => Part::Parameter {
                    name,
                    quoted: false,
                },
                other => other,
            };
            match part {
                Part::BraceParameter(_) => unreachable!(),
                Part::Modify { .. } => {
                    unreachable!("parameter modifiers resolve before field splitting")
                }
                Part::Elements { values, at, quoted } => {
                    if at || !quoted && matches!(output, Output::Fields(_)) {
                        output.elements(&values, quoted, ifs);
                    } else {
                        output.append(&values.join(&ifs_separator(lookup)), quoted, ifs);
                    }
                }
                Part::BadSubstitution(_)
                | Part::Substitution { .. }
                | Part::Process { .. }
                | Part::Arithmetic { .. }
                | Part::Trim { .. } => {
                    unreachable!(
                        "command and arithmetic substitutions are resolved before field splitting"
                    )
                }
                Part::Brace(text) | Part::BraceTilde(text) => output.append(text, true, ifs),
                Part::Range(range) => output.append(&range.spelling, true, ifs),
                Part::Text(s) => output.append(&s, true, ifs),
                Part::Pattern(s) => match output {
                    Output::Scalar(text) | Output::Pattern(text, _) => text.push_str(&s),
                    Output::Fields(fields) => fields.literal(&s, true),
                },
                Part::Nested(word) => word.expand_into(output, lookup, positional, ifs),
                Part::Unquoted(s) => output.append(&s, false, ifs),
                Part::Tilde(s) => output.append(&tilde(&s, lookup), true, ifs),
                Part::Parameter { name, quoted }
                    if name == "@"
                        || (name == "*" && !quoted && matches!(output, Output::Fields(_))) =>
                {
                    output.elements(positional, quoted, ifs);
                }
                Part::Parameter { name, quoted } if array_elements_name(&name).is_some() => {
                    let (base, at, keys) = array_elements_name(&name).unwrap();
                    let values = array_elements(base, keys, lookup).unwrap_or_default();
                    if at || (!quoted && matches!(output, Output::Fields(_))) {
                        output.elements(&values, quoted, ifs);
                    } else {
                        output.append(&values.join(&ifs_separator(lookup)), quoted, ifs);
                    }
                }
                Part::Parameter { name, quoted } => {
                    let value = parameter(&name, lookup, positional).unwrap_or_else(|| {
                        if name.starts_with('#') {
                            "0".into()
                        } else {
                            String::new()
                        }
                    });
                    output.append(&value, quoted, ifs);
                }
                Part::Alternative {
                    name,
                    check_empty,
                    op,
                    word,
                    quoted,
                } => {
                    let value = parameter(&name, lookup, positional);
                    let set = value
                        .as_ref()
                        .is_some_and(|s| !check_empty || !s.is_empty());
                    let use_word_if_set = op == Alternative::Alternate;
                    if set == use_word_if_set {
                        word.expand_into(output, lookup, positional, ifs);
                    } else {
                        let original = if use_word_if_set { None } else { value };
                        output.append(original.as_deref().unwrap_or(""), quoted, ifs);
                    }
                }
            }
        }
    }
}

enum PatternKind {
    Replacement,
    Glob,
    Extended,
    Regex,
}

enum Output<'a> {
    Scalar(String),
    Pattern(String, PatternKind),
    Fields(Fields<'a>),
}

impl Output<'_> {
    fn elements(&mut self, values: &[String], quoted: bool, ifs: &str) {
        for (i, value) in values.iter().enumerate() {
            if i != 0 {
                match self {
                    Self::Scalar(text) | Self::Pattern(text, _) => text.push(' '),
                    Self::Fields(fields) => fields.finish_field(),
                }
            }
            self.append(value, quoted, ifs);
        }
    }

    fn append(&mut self, text: &str, quoted: bool, ifs: &str) {
        match self {
            Self::Scalar(buffer) => buffer.push_str(text),
            Self::Pattern(buffer, PatternKind::Replacement) => {
                for c in text.chars() {
                    if c == '\\' || c == '&' && quoted {
                        buffer.push('\\');
                    }
                    buffer.push(c);
                }
            }
            Self::Pattern(buffer, _) if !quoted => buffer.push_str(text),
            Self::Pattern(buffer, PatternKind::Extended) => {
                crate::pattern::quote_pattern(buffer, text, true)
            }
            Self::Pattern(buffer, PatternKind::Glob) => quote_into(buffer, text),
            Self::Pattern(buffer, PatternKind::Regex) => {
                for c in text.chars() {
                    if matches!(
                        c,
                        '\\' | '.'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                            | '('
                            | ')'
                            | '*'
                            | '+'
                            | '?'
                            | '^'
                            | '$'
                            | '|'
                    ) {
                        buffer.push('\\');
                    }
                    buffer.push(c);
                }
            }
            Self::Fields(fields) => fields.expansion(text, quoted, ifs),
        }
    }
}

// The internal index query cannot collide with a Bash identifier. Sparse arrays
// enumerate their occupied indexes in ascending order, including empty values.
fn array_elements_name(name: &str) -> Option<(&str, bool, bool)> {
    let (name, keys) = name.strip_prefix('!').map_or((name, false), |s| (s, true));
    let base = name
        .strip_suffix("[@]")
        .or_else(|| name.strip_suffix("[*]"))?;
    is_identifier(base).then_some((base, name.ends_with("[@]"), keys))
}

fn array_elements(
    base: &str,
    keys: bool,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Option<Vec<String>> {
    let indexes = lookup(&format!("\u{1}{base}"))?;
    Some(
        indexes
            .split('\0')
            .filter(|i| !i.is_empty())
            .filter_map(|i| {
                if keys {
                    Some(i.to_owned())
                } else {
                    lookup(&format!("{base}[{i}]"))
                }
            })
            .collect(),
    )
}

fn parameter(
    name: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    positional: &[String],
) -> Option<String> {
    if name == "*" || name == "@" {
        return (!positional.is_empty()).then(|| positional.join(&ifs_separator(lookup)));
    }
    if name == "#" {
        return Some(positional.len().to_string());
    }
    if let Some(name) = name.strip_prefix('#') {
        let count = if matches!(name, "@" | "*") {
            positional.len()
        } else if let Some((base, _, _)) = array_elements_name(name) {
            lookup(&format!("\u{1}{base}"))?
                .split('\0')
                .filter(|i| !i.is_empty())
                .count()
        } else {
            parameter(name, lookup, positional)?.chars().count()
        };
        return Some(count.to_string());
    }
    if let Some((base, _, keys)) = array_elements_name(name) {
        return array_elements(base, keys, lookup)
            .map(|values| values.join(&ifs_separator(lookup)));
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) && name != "0" {
        return positional
            .get(name.parse::<usize>().ok()?.checked_sub(1)?)
            .cloned();
    }
    lookup(name)
}

fn tilde(s: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let key = match s {
        "~" => "HOME",
        "~+" => "PWD",
        "~-" => "OLDPWD",
        _ => return s.to_owned(),
    };
    lookup(key).unwrap_or_else(|| s.to_owned())
}

fn ifs_separator(lookup: &dyn Fn(&str) -> Option<String>) -> String {
    lookup("IFS").map_or_else(
        || " ".into(),
        |s| s.chars().next().map(String::from).unwrap_or_default(),
    )
}

// Visit separator spans in the original UTF-8 text. A single delimiter can
// use str's character search even when the delimiter is a multibyte scalar.
pub(crate) fn separators(text: &str, ifs: &str, mut visit: impl FnMut(usize, &str)) {
    let mut chars = ifs.chars();
    let Some(first) = chars.next() else { return };
    if chars.next().is_none() {
        for (i, sep) in text.match_indices(first) {
            visit(i, sep);
        }
    } else if ifs == " \t\n" {
        for (i, sep) in text.match_indices([' ', '\t', '\n']) {
            visit(i, sep);
        }
    } else {
        for (i, sep) in text.match_indices(|c| ifs.contains(c)) {
            visit(i, sep);
        }
    }
}

struct Fields<'a> {
    words: &'a mut Vec<String>,
    current: Field,
    globbing: Option<&'a crate::pattern::GlobOptions>,
    lookup: &'a dyn Fn(&str) -> Option<String>,
    error: Option<crate::expansion::Error>,
    present: bool,
    whitespace: bool,
}

impl Fields<'_> {
    fn emit(&mut self) {
        let current = std::mem::take(&mut self.current);
        if self.error.is_none() {
            self.error = current.finish(self.words, self.globbing, self.lookup).err();
        }
    }

    fn finish_field(&mut self) {
        if self.present {
            self.emit();
            self.present = false;
        }
        self.whitespace = false;
    }

    fn literal(&mut self, text: &str, active: bool) {
        if self.whitespace {
            self.finish_field();
        }
        self.present = true;
        self.current.push(text, active);
    }

    fn expansion(&mut self, text: &str, quoted: bool, ifs: &str) {
        if quoted {
            self.literal(text, false);
            return;
        }
        let mut start = 0;
        separators(text, ifs, |i, sep| {
            if start < i {
                self.literal(&text[start..i], true);
            }
            if matches!(sep, " " | "\t" | "\n") {
                self.whitespace = true;
            } else {
                self.emit();
                self.present = false;
                self.whitespace = false;
            }
            start = i + sep.len();
        });
        if start < text.len() {
            self.literal(&text[start..], true);
        }
    }
}

#[derive(Default)]
pub struct WordBuilder {
    parts: Vec<Part>,
    text: String,
    literal: bool,
}

impl WordBuilder {
    pub fn push(&mut self, c: char) {
        self.text.push(c);
        self.literal = true;
    }

    pub fn literal(&mut self, s: &str) {
        self.text.push_str(s);
        self.literal = true;
    }

    pub fn part(&mut self, part: Part) {
        // A parameter with adjacent literal text commonly needs only two parts.
        if self.parts.is_empty() {
            self.parts.reserve_exact(2);
        }
        self.flush();
        self.parts.push(part);
    }

    pub fn brace_parameter(&mut self) {
        if let Some(part @ Part::Parameter { .. }) = self.parts.last_mut()
            && let Part::Parameter { name, .. } = part
        {
            *part = Part::BraceParameter(std::mem::take(name));
        }
    }

    fn flush(&mut self) {
        if self.literal {
            self.parts.push(Part::Text(std::mem::take(&mut self.text)));
            self.literal = false;
        }
    }

    pub fn finish(mut self) -> Word {
        if self.parts.is_empty() {
            Word::Literal(self.text)
        } else {
            self.flush();
            Word::Expanded(self.parts)
        }
    }
}
