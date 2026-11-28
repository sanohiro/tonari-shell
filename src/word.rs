pub mod brace;
mod tilde;
use tilde::tilde;
pub(crate) use tilde::{account_home, account_shell, effective_user, named_home};
mod modifier;
pub(crate) mod quote;
mod replace;
use crate::parser::Statement;
use crate::pattern::{Field, quote_into};
pub(crate) use modifier::character_count;
pub use modifier::{Modification, ReplaceMode};

pub(crate) const ARRAY_INDEXES_PREFIX: char = '\u{1}';
fn array_indexes_key(base: &str) -> String {
    format!("{ARRAY_INDEXES_PREFIX}{base}")
}

#[derive(Clone, Debug)]
pub enum Word {
    Literal(String),
    Expanded(Vec<Part>),
    Declaration(Vec<Part>),
}

fn indirect_target(
    mut target: String,
    env: &mut crate::expansion::Expansion<'_>,
) -> Result<String, crate::expansion::Error> {
    // Resolve a computed numeric index once. Later parameter
    // operations consume the index without reinterpreting its source.
    if let Some((base, expr)) = crate::session::array::subscript(&target)
        && is_identifier(base)
        && !matches!(expr, "@" | "*")
        && !env.session.is_associative(base)
        && !expr.bytes().all(|b| b.is_ascii_digit())
    {
        let index = env.session.resolve_index(base, expr).map_err(|_| {
            crate::expansion::Error::discard(format!("{target}: invalid variable name"))
        })?;
        target = format!("{base}[{index}]");
    } else if !(is_identifier(&target)
        || array_elements_name(&target).is_some()
        || (!target.is_empty() && target.bytes().all(|b| b.is_ascii_digit()))
        || matches!(target.as_str(), "@" | "*" | "#" | "?" | "-" | "$" | "!")
        || env.session.valid_target(&target))
    {
        return Err(crate::expansion::Error::discard(format!(
            "{target}: invalid variable name"
        )));
    }
    Ok(target)
}

impl Default for Word {
    fn default() -> Self {
        Self::Literal(String::new())
    }
}

pub(crate) const DEFAULT_IFS: &str = " \t\n";

pub(crate) fn is_ifs_whitespace(sep: &str) -> bool {
    matches!(sep, " " | "\t" | "\n" | "\r" | "\x0b" | "\x0c")
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
    InvalidCommandSubstitution,
    PrefixNames {
        prefix: String,
        at: bool,
        quoted: bool,
    },
    Brace(&'static str),
    BraceParameter(std::borrow::Cow<'static, str>),
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
        name: std::borrow::Cow<'static, str>,
        operation: Box<Modification>,
        quoted: bool,
    },
    Trim {
        name: std::borrow::Cow<'static, str>,
        pattern: Box<Word>,
        prefix: bool,
        longest: bool,
        quoted: bool,
    },
    Arithmetic {
        expression: std::rc::Rc<crate::arithmetic::Expression>,
        quoted: bool,
    },
    Parameter {
        name: std::borrow::Cow<'static, str>,
        quoted: bool,
    },
    Tilde(String),
    Unquoted(String),
    Process {
        body: Vec<Statement>,
        input: bool,
    },
    CurrentSubstitution {
        body: std::rc::Rc<[Statement]>,
        quoted: bool,
        reply: bool,
    },
    Substitution {
        body: std::rc::Rc<[Statement]>,
        quoted: bool,
    },
    Alternative {
        name: std::borrow::Cow<'static, str>,
        check_empty: bool,
        op: Alternative,
        word: Box<Word>,
        quoted: bool,
    },
}

pub(crate) fn is_name_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

pub(crate) fn is_name_char(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

// Continuations can start with a digit, e.g. after a backslash-newline.
pub(crate) fn name_chars_len(source: &str) -> usize {
    source
        .bytes()
        .take_while(|&byte| is_name_char(byte))
        .count()
}

pub fn is_identifier(name: &str) -> bool {
    let mut chars = name.bytes();
    chars.next().is_some_and(is_name_start) && chars.all(is_name_char)
}

pub(crate) fn valid_tilde_user(user: &str) -> bool {
    user.chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '+' | '.' | '@'))
}

impl Word {
    pub fn as_literal(&self) -> Option<&str> {
        match self {
            Self::Literal(s) => Some(s),
            _ => None,
        }
    }

    // Assignment words can retain an empty literal where their name was removed.
    pub(crate) fn stored_parameter<'s>(&self, session: &'s crate::Session) -> Option<&'s str> {
        let Self::Expanded(parts) = self else {
            return None;
        };
        let parts = match parts.as_slice() {
            [Part::Text(prefix), rest @ ..] if prefix.is_empty() => rest,
            other => other,
        };
        let [Part::Parameter { name, .. } | Part::BraceParameter(name)] = parts else {
            return None;
        };
        match name.as_bytes() {
            [digit @ b'1'..=b'9'] => session
                .positional
                .as_slice()
                .get(usize::from(digit - b'1'))
                .map(String::as_str),
            _ if is_identifier(name) => session.referenced_scalar(name),
            _ => None,
        }
    }

    fn fixed_parts(&self) -> bool {
        matches!(self, Self::Literal(_))
            || matches!(self, Self::Expanded(parts) if parts.iter().all(|p| matches!(p, Part::Text(_) | Part::Unquoted(_) | Part::Pattern(_))))
    }

    fn fixed_pattern(self, extended: bool) -> String {
        if extended {
            self.extended_pattern(&|_| None, &[])
        } else {
            self.pattern(&|_| None, &[])
        }
    }

    pub fn declaration(self) -> Self {
        match self {
            Self::Expanded(parts) => Self::Declaration(parts),
            word => word,
        }
    }

    pub(crate) fn assignment_name(&self, length: std::num::NonZeroUsize) -> Option<&str> {
        let text = match self {
            Self::Literal(s) => s,
            Self::Expanded(parts) | Self::Declaration(parts) => match parts.first()? {
                Part::Text(s) => s,
                _ => return None,
            },
        };
        (text.as_bytes().get(length.get()) == Some(&b'='))
            .then(|| text.get(..length.get()))
            .flatten()
    }

    pub(crate) fn take_assignment_name(
        &mut self,
        length: std::num::NonZeroUsize,
    ) -> Option<String> {
        if let Self::Expanded(parts) = self
            && let Some(Part::Text(prefix)) = parts.first_mut()
            && prefix.len() == length.get() + 1
            && prefix.ends_with('=')
        {
            let mut name = std::mem::take(prefix);
            name.pop();
            return Some(name);
        }
        let name = self.assignment_name(length)?.to_owned();
        self.remove_assignment_prefix(name.len() + 1)?;
        Some(name)
    }

    pub fn remove_assignment_prefix(&mut self, length: usize) -> Option<()> {
        let text = match self {
            Self::Literal(s) => s,
            Self::Expanded(parts) | Self::Declaration(parts) => match parts.first_mut()? {
                Part::Text(s) => s,
                _ => return None,
            },
        };
        if length == 0 || text.as_bytes().get(length - 1) != Some(&b'=') {
            return None;
        }
        text.drain(..length);
        Some(())
    }

    pub(crate) fn literal_element_suffix(&self, key: &str) -> Option<&str> {
        let Self::Expanded(parts) = self else {
            return None;
        };
        let parts = match parts.as_slice() {
            [Part::Text(empty), rest @ ..] if empty.is_empty() => rest,
            rest => rest,
        };
        match parts {
            [
                Part::Parameter { name, .. } | Part::BraceParameter(name),
                Part::Text(suffix) | Part::Pattern(suffix),
            ] if name == key => Some(suffix),
            _ => None,
        }
    }

    /// A plain scalar can keep its allocation for a self-prefixed assignment.
    /// Evaluate the suffix normally. Expressions capable of changing the old
    /// value (including indexed name references) require the original snapshot.
    pub(crate) fn take_append_suffix(&mut self, target: &str, session: &crate::Session) -> bool {
        let Self::Expanded(parts) = self else {
            return false;
        };
        let parts = match parts.as_mut_slice() {
            [Part::Text(empty), rest @ ..] if empty.is_empty() => rest,
            other => other,
        };
        let Some((head, tail)) = parts.split_first_mut() else {
            return false;
        };
        if !matches!(head, Part::Parameter { name, .. } | Part::BraceParameter(name) if name == target)
            || !session.can_append_scalar(target)
            || !tail.iter().all(|p| match p {
                Part::Text(_) | Part::Pattern(_) => true,
                Part::Parameter { name, .. } | Part::BraceParameter(name) => {
                    is_identifier(name) && !session.is_nameref(name)
                }
                _ => false,
            })
        {
            return false;
        }
        *head = Part::Text(String::new());
        true
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
        let _nesting = crate::parser::enter_expansion()
            .map_err(|error| crate::expansion::Error::discard(error.message()))?;
        let literal = modifier::scalar_part;
        let mixed = parts.len() > 1;
        for part in parts.iter_mut() {
            if let Part::PrefixNames { prefix, at, quoted } = part {
                *part = Part::Elements {
                    values: env.session.variable_names(prefix),
                    at: *at,
                    quoted: *quoted,
                };
                continue;
            }
            if matches!(part, Part::InvalidCommandSubstitution) {
                return Err(crate::expansion::Error::discard_line(
                    "unterminated command substitution",
                ));
            }
            if let Part::BadSubstitution(text) = part {
                let message = format!("{text}: bad substitution");
                return Err(if env.session.errexit && !env.session.interactive {
                    crate::expansion::Error::fatal(message, 1)
                } else {
                    crate::expansion::Error::discard(message)
                });
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
                let target = env
                    .session
                    .read_reference_name(reference)
                    .unwrap_or_default()
                    .into_owned();
                *part = literal(target, *quoted);
                continue;
            }
            if let Part::Parameter { name, .. } = part
                && env.session.is_nameref(name)
            {
                let Some(target) = env.session.read_reference_name(name) else {
                    if env.session.nounset {
                        return Err(env.missing(name, "unbound variable"));
                    }
                    let quoted = matches!(part, Part::Parameter { quoted: true, .. });
                    *part = literal(String::new(), quoted);
                    continue;
                };
                if array_elements_name(&target).is_some() {
                    *name = target.into_owned().into();
                }
            }
            let plain_quote = match part {
                Part::Parameter { quoted, .. } => Some(*quoted),
                _ => None,
            };
            if let Part::Parameter { name, .. }
            | Part::Alternative { name, .. }
            | Part::Trim { name, .. }
            | Part::Modify { name, .. } = part
            {
                if name.ends_with("[]") {
                    return Err(crate::expansion::Error::discard(format!(
                        "{name}: bad array subscript"
                    )));
                }
                if let Some(reference) = name.strip_prefix('!')
                    && (!env.session.nounset || plain_quote.is_none())
                    && (matches!(reference, "@" | "*")
                        && env.session.positional.as_slice().is_empty()
                        || reference.as_bytes().first().is_some_and(u8::is_ascii_digit)
                            && reference
                                .parse::<usize>()
                                .is_ok_and(|i| i > env.session.positional.len()))
                {
                    *name = "".into();
                }
                if let Some(reference) = name.strip_prefix('!').filter(|s| {
                    !s.is_empty() && (plain_quote.is_none() || array_elements_name(name).is_none())
                }) {
                    let Some(target) = modifier::value(reference, env)? else {
                        if let Some(quoted) = plain_quote
                            && !env.session.nounset
                            && env.session.stored_array_len(reference).is_some()
                        {
                            *part = literal(String::new(), quoted);
                            continue;
                        }
                        let message = format!("{reference}: invalid indirect expansion");
                        return Err(if env.session.nounset {
                            crate::expansion::Error::fatal(message, 1)
                        } else {
                            crate::expansion::Error::discard(message)
                        });
                    };
                    *name = indirect_target(target, env)?.into();
                }
            }
            if let Part::Parameter { name, quoted } = part
                && (mixed || !*quoted)
                && let Some((base, at, keys)) = array_elements_name(name)
                && !env.shadows(base)
                && let Some(value) = snapshot_array(base, at, keys, *quoted, env)
            {
                *part = value;
                continue;
            }
            match part {
                Part::Tilde(s) => *part = Part::Text(tilde(s, env)),
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
                    if pattern.fixed_parts()
                        && let Some(text) = env.borrowed_scalar(name)
                    {
                        let extended = env.session.shopt.glob.extglob;
                        let source = std::mem::take(pattern.as_mut()).fixed_pattern(extended);
                        let compiled =
                            crate::pattern::ParameterPattern::new(&source, extended, None);
                        let cut = crate::pattern::trim_boundary(text, &compiled, *prefix, *longest);
                        let result = match cut {
                            Some(index) if *prefix => &text[index..],
                            Some(index) => &text[..index],
                            None => text,
                        };
                        *part = literal(result.to_owned(), *quoted);
                        continue;
                    }
                    let values = modifier::Values::read(name, env)?;
                    let pattern = env.pattern(std::mem::take(pattern.as_mut()))?;
                    let compiled = crate::pattern::ParameterPattern::new(
                        &pattern,
                        env.session.shopt.glob.extglob,
                        None,
                    );
                    *part = values.map(*quoted, |v| {
                        crate::pattern::trim(v, &compiled, *prefix, *longest)
                    });
                }
                Part::Arithmetic { expression, quoted } => {
                    let value = match expression.compiled(env, true) {
                        Some(Ok(n)) => n,
                        Some(Err(e)) if e == "unbound arithmetic variable" => {
                            return Err(crate::arithmetic::shell_error(e, expression, env.session));
                        }
                        Some(Err(e)) => return Err(crate::expansion::Error::discard(e)),
                        None => env.arithmetic(expression)?,
                    };
                    *part = literal(value.to_string(), *quoted);
                }
                Part::Process { body, input } => {
                    *part = Part::Text(env.process(std::mem::take(body), *input)?);
                }
                Part::CurrentSubstitution {
                    body,
                    quoted,
                    reply,
                } => {
                    *part = literal(env.current_capture(body, *reply)?, *quoted);
                }
                Part::Substitution { body, quoted } => {
                    *part = literal(env.capture(body)?, *quoted);
                }
                Part::Alternative {
                    name,
                    check_empty,
                    op,
                    word,
                    quoted,
                } => {
                    let element = if *op == Alternative::Assign {
                        env.assignment_element(name)?
                    } else {
                        None
                    };
                    if *op == Alternative::Alternate
                        && let Some(value) = env.borrowed_scalar(name)
                    {
                        if !*check_empty || !value.is_empty() {
                            word.resolve(env)?;
                            *part = resolved_operand(word);
                        } else {
                            *part = literal(String::new(), *quoted);
                        }
                        continue;
                    }
                    let elements = array_elements_name(name);
                    // A successful + alternative never reads the elements.
                    // Multiple quoted @ fields stay distinct even if all are empty.
                    if *op == Alternative::Alternate
                        && let Some((base, at, false)) = elements
                        && !env.shadows(base)
                        && let Some(len) = env.session.stored_array_len(base)
                        && len > 0
                        && (!*check_empty || len > 1 && (at || !*quoted))
                    {
                        word.resolve(env)?;
                        *part = resolved_operand(word);
                        continue;
                    }
                    let (set, original) =
                        if matches!(name.as_ref(), "@" | "*") || elements.is_some() {
                            let modifier::Values::Elements(values, at) =
                                modifier::Values::read(name, env)?
                            else {
                                unreachable!()
                            };
                            let separator = ifs_separator(&|key| env.lookup(key));
                            let nonempty = values.iter().any(|s| !s.is_empty())
                                || values.len() > 1 && (at || !*quoted || !separator.is_empty());
                            let set = !values.is_empty() && (!*check_empty || nonempty);
                            let part = if !*quoted && !separator.is_empty() {
                                Part::Unquoted(values.join(&separator))
                            } else {
                                Part::Elements {
                                    values,
                                    at,
                                    quoted: *quoted,
                                }
                            };
                            (set, part)
                        } else {
                            let value = if let Some(element) = &element {
                                element.value(env.session)
                            } else {
                                modifier::value(name, env)?
                            };
                            let set = value
                                .as_ref()
                                .is_some_and(|s| !*check_empty || !s.is_empty());
                            (set, literal(value.unwrap_or_default(), *quoted))
                        };
                    let use_word_if_set = *op == Alternative::Alternate;
                    if set == use_word_if_set {
                        word.resolve(env)?;
                        if matches!(*op, Alternative::Assign | Alternative::Error) {
                            let operand = std::mem::take(word.as_mut());
                            let value = operand
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
                            let value = env.assign_parameter(name, element, value)?;
                            *part = literal(value, *quoted);
                            continue;
                        }
                        *part = resolved_operand(word);
                    } else {
                        *part = if use_word_if_set {
                            match original {
                                Part::Elements {
                                    ref values,
                                    at: true,
                                    ..
                                } if values.is_empty() => original,
                                _ => literal(String::new(), *quoted),
                            }
                        } else {
                            original
                        };
                    }
                }
                Part::Parameter { name, quoted: true } if name == "*" => {
                    // A later expansion in this word can change IFS or the arguments.
                    *part = Part::Text(
                        env.session
                            .positional
                            .as_slice()
                            .join(&ifs_separator(&|k| env.lookup(k))),
                    );
                }
                Part::Parameter { name, quoted: true }
                    if name.ends_with("[*]") && array_elements_name(name).is_some() =>
                {
                    let separator = ifs_separator(&|k| env.lookup(k));
                    if let Some((base, _, false)) = array_elements_name(name)
                        && !env.shadows(base)
                        && let Some(joined) = env.session.join_array(base, &separator)
                    {
                        *part = Part::Text(joined);
                        continue;
                    }
                    let modifier::Values::Elements(values, _) = modifier::Values::read(name, env)?
                    else {
                        unreachable!()
                    };
                    *part = Part::Text(values.join(&separator));
                }
                Part::Parameter { name, quoted }
                    if !matches!(name.as_ref(), "@" | "*")
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
        let mut text = String::new();
        self.expand_into(&mut Output::Scalar(&mut text), lookup, positional, "");
        text
    }

    #[cfg(test)]
    pub fn fields(
        self,
        lookup: &dyn Fn(&str) -> Option<String>,
        positional: &[String],
    ) -> Vec<String> {
        let mut words = Vec::new();
        self.append_fields(
            lookup,
            positional,
            || lookup("IFS").map(Into::into),
            Some(&Default::default()),
            &mut words,
        )
        .unwrap();
        words
    }

    pub fn append_fields<'s>(
        mut self,
        lookup: &dyn Fn(&str) -> Option<String>,
        positional: &[String],
        get_ifs: impl FnOnce() -> Option<std::borrow::Cow<'s, str>>,
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
        if let Self::Expanded(parts) = &self
            && matches!(parts.as_slice(), [Part::Parameter { name, quoted: true }] if name == "@")
        {
            words.extend_from_slice(positional);
            return Ok(());
        }
        let ifs = get_ifs();
        // A standalone, already-resolved expansion can write complete fields
        // directly when no field can be a pathname pattern. Keep small words
        // on the ordinary path and recognize only Bash's three IFS spaces.
        if let Self::Expanded(parts) = &self
            && let [Part::Unquoted(text)] = parts.as_slice()
            && text.len() >= 256
            && ifs
                .as_deref()
                .is_none_or(|ifs| ifs == crate::word::DEFAULT_IFS)
            && globbing.is_none_or(|glob| {
                memchr::memchr3(b'*', b'?', b'[', text.as_bytes()).is_none()
                    && !(glob.extglob && text.contains('('))
            })
        {
            words.extend(default_fields(text).map(str::to_owned));
            return Ok(());
        }
        let mut fields = Fields {
            lookup,
            globbing,
            words,
            current: Field::default(),
            error: None,
            present: false,
            whitespace: false,
        };
        self.expand_into(
            &mut Output::Fields(&mut fields),
            lookup,
            positional,
            ifs.as_deref().unwrap_or(crate::word::DEFAULT_IFS),
        );
        fields.finish_field();
        fields.error.map_or(Ok(()), Err)
    }

    pub fn pattern(self, lookup: &dyn Fn(&str) -> Option<String>, positional: &[String]) -> String {
        self.expand_pattern(PatternKind::Glob, lookup, positional)
    }

    pub fn extended_pattern(
        self,
        lookup: &dyn Fn(&str) -> Option<String>,
        positional: &[String],
    ) -> String {
        self.expand_pattern(PatternKind::Extended, lookup, positional)
    }

    fn replacement(self, lookup: &dyn Fn(&str) -> Option<String>, positional: &[String]) -> String {
        self.expand_pattern(PatternKind::Replacement, lookup, positional)
    }
    pub fn regex(self, lookup: &dyn Fn(&str) -> Option<String>, positional: &[String]) -> String {
        self.expand_pattern(PatternKind::Regex, lookup, positional)
    }

    fn expand_pattern(
        self,
        kind: PatternKind,
        lookup: &dyn Fn(&str) -> Option<String>,
        positional: &[String],
    ) -> String {
        let mut text = String::new();
        self.expand_into(
            &mut Output::Pattern(&mut text, kind),
            lookup,
            positional,
            "",
        );
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
                Part::PrefixNames { .. }
                | Part::InvalidCommandSubstitution
                | Part::BadSubstitution(_)
                | Part::CurrentSubstitution { .. }
                | Part::Substitution { .. }
                | Part::Process { .. }
                | Part::Arithmetic { .. }
                | Part::Trim { .. }
                | Part::Tilde(_) => {
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

enum Output<'out, 'fields> {
    Scalar(&'out mut String),
    Pattern(&'out mut String, PatternKind),
    Fields(&'out mut Fields<'fields>),
}

impl Output<'_, '_> {
    fn elements(&mut self, values: &[String], quoted: bool, ifs: &str) {
        for (i, value) in values.iter().enumerate() {
            if i != 0 {
                match self {
                    Self::Scalar(text) | Self::Pattern(text, _) => text.push(' '),
                    Self::Fields(fields)
                        if !quoted
                            && ifs.as_bytes().first().is_some_and(|b| {
                                !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 11 | 12)
                            }) =>
                    {
                        // An implicit non-whitespace IFS separator emits even an
                        // empty field. The final field still needs actual text.
                        fields.emit();
                        fields.present = false;
                        fields.whitespace = false;
                    }
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
                if !quoted || !text.contains(['\\', '&']) {
                    buffer.push_str(text);
                    return;
                }
                for c in text.chars() {
                    if quoted && matches!(c, '\\' | '&') {
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
                crate::test_expression::quote_regex(buffer, text)
            }
            Self::Fields(fields) => fields.expansion(text, quoted, ifs),
        }
    }
}

// The internal index query cannot collide with a Bash identifier. Sparse arrays
// enumerate their occupied indexes in ascending order, including empty values.
pub(crate) fn array_elements_name(name: &str) -> Option<(&str, bool, bool)> {
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
    let indexes = lookup(&array_indexes_key(base))?;
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
            lookup(&array_indexes_key(base))?
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

fn snapshot_array(
    base: &str,
    at: bool,
    keys: bool,
    quoted: bool,
    env: &crate::expansion::Expansion<'_>,
) -> Option<Part> {
    let mut values = Vec::new();
    env.session
        .append_array_fields(base, keys, &mut values)
        .then(|| {
            if !at && quoted {
                // The separator belongs to this expansion, even if a later
                // parameter assignment in the same word changes IFS.
                Part::Text(values.join(&ifs_separator(&|k| env.lookup(k))))
            } else {
                Part::Elements { values, at, quoted }
            }
        })
}

fn ifs_separator(lookup: &dyn Fn(&str) -> Option<String>) -> std::borrow::Cow<'static, str> {
    let Some(mut ifs) = lookup("IFS") else {
        return " ".into();
    };
    ifs.truncate(ifs.chars().next().map_or(0, char::len_utf8));
    ifs.into()
}

pub(crate) fn default_fields(text: &str) -> impl Iterator<Item = &str> {
    let mut start = 0;
    memchr::memchr3_iter(b' ', b'\t', b'\n', text.as_bytes())
        .chain(std::iter::once(text.len()))
        .filter_map(move |end| {
            let field = &text[start..end];
            start = end + 1;
            (!field.is_empty()).then_some(field)
        })
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
    } else if ifs == crate::word::DEFAULT_IFS {
        for i in memchr::memchr3_iter(b' ', b'\t', b'\n', text.as_bytes()) {
            visit(i, &text[i..i + 1]);
        }
    } else if let [a, b] = ifs.as_bytes()
        && ifs.is_ascii()
    {
        for i in memchr::memchr2_iter(*a, *b, text.as_bytes()) {
            visit(i, &text[i..i + 1]);
        }
    } else if let [a, b, c] = ifs.as_bytes()
        && ifs.is_ascii()
    {
        for i in memchr::memchr3_iter(*a, *b, *c, text.as_bytes()) {
            visit(i, &text[i..i + 1]);
        }
    } else if ifs.is_ascii() {
        let mask = ifs.bytes().fold(0u128, |mask, b| mask | (1u128 << b));
        for (i, b) in text.bytes().enumerate() {
            if b.is_ascii() && mask & (1u128 << b) != 0 {
                visit(i, &text[i..i + 1]);
            }
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
            if is_ifs_whitespace(sep) {
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

fn resolved_operand(word: &mut Word) -> Part {
    match std::mem::take(word) {
        Word::Literal(text) => Part::Text(text),
        other => Part::Nested(other),
    }
}
