use crate::arithmetic::{self, Variables};
use crate::parser::{Statement, Word};
use crate::{Session, execute};
use std::cell::Cell;

#[derive(Debug)]
enum Recovery {
    Control(crate::Flow),
    Command,
    Input,
    Exit,
}

#[derive(Debug)]
pub struct Error {
    message: std::borrow::Cow<'static, str>,
    recovery: Recovery,
    pub status: u8,
}

impl Error {
    pub fn fatal(message: impl Into<std::borrow::Cow<'static, str>>, status: u8) -> Self {
        Self {
            message: message.into(),
            recovery: Recovery::Exit,
            status,
        }
    }

    pub fn discard(message: impl Into<std::borrow::Cow<'static, str>>) -> Self {
        Self {
            message: message.into(),
            recovery: Recovery::Input,
            status: 1,
        }
    }

    pub fn readonly(name: &str) -> Self {
        Self::discard(format!("{name}: readonly variable"))
    }

    pub(crate) fn control(flow: crate::Flow) -> Self {
        Self {
            message: "".into(),
            status: match flow {
                crate::Flow::Discard(n) | crate::Flow::Exit(n) | crate::Flow::Break(_, n) => n,
                _ => 0,
            },
            recovery: Recovery::Control(flow),
        }
    }

    pub(crate) fn report(&self, context: &str) {
        if !matches!(self.recovery, Recovery::Control(_)) {
            eprintln!("tonari: {context}{self}");
        }
    }

    pub fn child_failure(&mut self) {
        self.recovery = Recovery::Command;
    }

    pub fn flow(&self, session: &Session) -> crate::Flow {
        match self.recovery {
            Recovery::Control(flow) => flow,
            Recovery::Input
                if session.posix && !session.interactive && session.shopt.compat > 43 =>
            {
                crate::Flow::Exit(self.status)
            }
            Recovery::Input => crate::Flow::Discard(self.status),
            Recovery::Exit if !session.interactive => crate::Flow::Exit(self.status),
            Recovery::Exit => crate::Flow::Discard(self.status),
            _ => crate::Flow::Continue(self.status),
        }
    }
}

impl From<&'static str> for Error {
    fn from(message: &'static str) -> Self {
        Self {
            message: message.into(),
            recovery: Recovery::Command,
            status: 1,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}

impl std::error::Error for Error {}

pub struct Expansion<'a> {
    pub session: &'a mut Session,
    assignments: &'a mut [(String, String)],
    persistent: bool,
    initial: u8,
    captured: &'a Cell<Option<u8>>,
}

impl<'a> Expansion<'a> {
    pub fn new(session: &'a mut Session, initial: u8, captured: &'a Cell<Option<u8>>) -> Self {
        Self::with_assignments(session, initial, captured, &mut [], false)
    }

    pub fn with_assignments(
        session: &'a mut Session,
        initial: u8,
        captured: &'a Cell<Option<u8>>,
        assignments: &'a mut [(String, String)],
        persistent: bool,
    ) -> Self {
        Self {
            session,
            assignments,
            persistent,
            initial,
            captured,
        }
    }

    pub(crate) fn borrowed_scalar(&self, name: &str) -> Option<&str> {
        if !crate::word::is_identifier(name) {
            return None;
        }
        self.assignments
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, value)| value.as_str())
            .or_else(|| self.session.stored_scalar(name))
    }

    pub fn lookup(&self, name: &str) -> Option<String> {
        self.assignments
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .or_else(|| {
                execute::parameter(
                    self.session,
                    self.captured.get().unwrap_or(self.initial),
                    name,
                )
            })
    }

    pub(super) fn shadows(&self, name: &str) -> bool {
        self.assignments.iter().any(|(key, _)| key == name)
    }

    pub fn assign(&mut self, name: &str, text: &str) {
        if let Some((_, v)) = self.assignments.iter_mut().rev().find(|(k, _)| k == name) {
            v.clear();
            v.push_str(text);
            if !self.persistent {
                return;
            }
        }
        self.session.assign(name, text);
    }

    pub fn missing(&self, name: &str, message: &str) -> Error {
        Error::fatal(
            format!("{name}: {message}"),
            if self.session.command_string && !self.session.interactive {
                127
            } else {
                1
            },
        )
    }

    pub fn assign_parameter(&mut self, name: &str, text: &str) -> Result<(), Error> {
        let base = crate::session::array::subscript(name).map_or(name, |(base, _)| base);
        if self.session.readonly(base) {
            return Err(Error::readonly(base));
        }
        if crate::parser::is_identifier(name) {
            self.assign(name, text);
            return Ok(());
        }
        if let Some((base, index)) = crate::session::array::subscript(name) {
            if !matches!(index, "@" | "*") {
                if self.session.is_associative(base) {
                    let key = self.key(index)?;
                    self.session.assign_key(base, key, text.into())?;
                    return Ok(());
                }
                let i = self.session.resolve_index(base, index)?;
                self.session.assign_element(base, i, text.to_owned());
                return Ok(());
            }
        }
        Err(Error::fatal(
            format!("${name}: cannot assign in this way"),
            1,
        ))
    }

    pub fn process(&mut self, body: Vec<Statement>, input: bool) -> Result<String, Error> {
        Ok(crate::execute::process_substitution::launch(
            self.session,
            body,
            input,
            self.captured.get().unwrap_or(self.initial),
            self.assignments,
        )?)
    }

    pub fn current_capture(&mut self, body: Vec<Statement>, reply: bool) -> Result<String, Error> {
        let last = self.status();
        let (text, status) =
            crate::subshell::current_substitution(self.session, body, last, reply)?;
        self.captured.set(Some(status));
        Ok(text)
    }

    pub fn capture(&mut self, body: Vec<Statement>) -> Result<String, &'static str> {
        let mut overridden;
        let session = if self.assignments.is_empty() {
            &mut *self.session
        } else {
            let mut copy = self.session.clone();
            copy.bind(self.assignments);
            overridden = copy;
            &mut overridden
        };
        let (value, status) = crate::capture_substitution(
            session,
            body,
            self.captured.get().unwrap_or(self.initial),
        )?;
        self.captured.set(Some(status));
        Ok(value)
    }

    pub fn status(&self) -> u8 {
        self.captured.get().unwrap_or(self.initial)
    }

    pub fn array_items(
        &mut self,
        parts: Vec<(Option<crate::parser::Subscript<String>>, Word)>,
        associative: bool,
    ) -> Result<crate::session::array::ArrayItems, Error> {
        use crate::session::array::{ArrayItems, Index};
        if associative {
            let pairs = parts.first().is_some_and(|(index, _)| index.is_none());
            let mut items = Vec::new();
            let mut parts = parts.into_iter();
            while let Some((index, word)) = parts.next() {
                let (key, value, append) = if pairs {
                    let key = self.scalar(word)?;
                    let value = match parts.next() {
                        Some((_, word)) => self.scalar(word)?,
                        None => String::new(),
                    };
                    (key, value, false)
                } else {
                    let index = index.ok_or("missing associative array subscript")?;
                    let key = self.key(&index.index)?;
                    (key, self.scalar(word)?, index.append)
                };
                if key.is_empty() {
                    return Err("bad array subscript".into());
                }
                items.push((key, value, append));
            }
            return Ok(ArrayItems::Associative(items));
        }
        let mut items = Vec::with_capacity(parts.len());
        for (index, word) in parts {
            if let Some(crate::parser::Subscript { index, append }) = index {
                // Shell expansions in a subscript precede its RHS, but arithmetic
                // in the resulting text runs when the initialized array is written.
                let index = if index.contains(['$', '`', '"', '\\']) {
                    let word = crate::parser::with_subscript_aliases(self.session, &index, || {
                        crate::parser::arithmetic_word(&index)
                    })
                    .map_err(|e| e.message())?;
                    self.scalar(word)?
                } else {
                    index
                };
                let index = Index::new(index);
                items.push((
                    Some(crate::parser::Subscript { index, append }),
                    self.scalar(word)?,
                ));
            } else {
                items.extend(self.fields(word)?.into_iter().map(|s| (None, s)));
            }
        }
        Ok(ArrayItems::Indexed(items))
    }

    pub fn key(&mut self, source: &str) -> Result<String, Error> {
        let key = if source.contains(['$', '`', '"', '\'', '\\']) {
            let word = crate::parser::with_subscript_aliases(self.session, source, || {
                crate::parser::subscript_word(source, true)
            })
            .map_err(|e| e.message())?;
            self.scalar(word)?
        } else {
            source.to_owned()
        };
        if key.is_empty() {
            return Err("bad array subscript".into());
        }
        Ok(key)
    }

    pub fn arithmetic(&mut self, expression: &str) -> Result<i64, Error> {
        let expanded;
        let expression = if expression.contains(['$', '`', '"', '\\']) {
            expanded = if expression.contains('[') {
                let (keys, tail) =
                    crate::parser::with_subscript_aliases(self.session, expression, || {
                        crate::parser::arithmetic_segments(expression, |name| {
                            self.session.is_associative(name)
                        })
                    })
                    .map_err(|e| e.message())?;
                if keys.is_empty() {
                    self.scalar(tail)?
                } else {
                    let mut source = String::with_capacity(expression.len());
                    for (prefix, key) in keys {
                        source.push_str(&self.scalar(prefix)?);
                        let key = self.scalar(key)?;
                        if key.contains(['[', ']', '\\', '\'', '"', '$', '`']) {
                            source.push_str(&crate::completion_spec::single_quote(&key));
                        } else {
                            source.push_str(&key);
                        }
                    }
                    source.push_str(&self.scalar(tail)?);
                    source
                }
            } else {
                let word = crate::parser::with_subscript_aliases(self.session, expression, || {
                    crate::parser::arithmetic_word(expression)
                })
                .map_err(|e| e.message())?;
                self.scalar(word)?
            };
            expanded.as_str()
        } else {
            expression
        };
        arithmetic::with_variables(expression, self).map_err(|message| match message {
            "unbound arithmetic variable" => Error::fatal(message, 1),
            _ => Error::discard(message),
        })
    }

    pub fn fields(&mut self, word: Word) -> Result<Vec<String>, Error> {
        self.fields_with_glob(word, !self.session.noglob)
    }

    pub fn fields_with_glob(&mut self, word: Word, glob: bool) -> Result<Vec<String>, Error> {
        let mut fields = Vec::new();
        self.append_fields_with_glob(word, glob, &mut fields)?;
        Ok(fields)
    }

    pub fn append_fields(&mut self, word: Word, fields: &mut Vec<String>) -> Result<(), Error> {
        self.append_fields_with_glob(word, !self.session.noglob, fields)
    }

    fn append_fields_with_glob(
        &mut self,
        word: Word,
        glob: bool,
        fields: &mut Vec<String>,
    ) -> Result<(), Error> {
        if self.session.shopt.braceexpand
            && let Word::Expanded(parts) = &word
            && parts.iter().any(|p| {
                matches!(
                    p,
                    crate::word::Part::Brace("{") | crate::word::Part::Range(_)
                )
            })
            && let Word::Expanded(parts) = word
        {
            return crate::word::brace::append_fields(parts, self, glob, fields);
        }
        self.append_resolved_fields(word, glob, fields)
    }

    pub(crate) fn append_resolved_fields(
        &mut self,
        mut word: Word,
        glob: bool,
        fields: &mut Vec<String>,
    ) -> Result<(), Error> {
        word.resolve(self)?;
        // Each element of a standalone quoted "${a[@]}" is already a complete field.
        if self.assignments.is_empty()
            && let Word::Expanded(parts) = &word
            && let [crate::word::Part::Parameter { name, quoted: true }] = parts.as_slice()
            && let Some(base) = name.strip_suffix("[@]")
            && self.session.append_indexed_fields(base, fields)
        {
            return Ok(());
        }
        word.append_fields(
            &|name| self.lookup(name),
            self.session.positional.as_slice(),
            glob.then_some(&self.session.shopt.glob),
            fields,
        )
    }

    pub fn scalar(&mut self, mut word: Word) -> Result<String, Error> {
        word.resolve(self)?;
        Ok(word.scalar(
            &|name| self.lookup(name),
            self.session.positional.as_slice(),
        ))
    }

    pub fn pattern(&mut self, mut word: Word) -> Result<String, Error> {
        word.resolve(self)?;
        Ok(if self.session.shopt.glob.extglob {
            word.extended_pattern(
                &|name| self.lookup(name),
                self.session.positional.as_slice(),
            )
        } else {
            word.pattern(
                &|name| self.lookup(name),
                self.session.positional.as_slice(),
            )
        })
    }

    pub fn regex(&mut self, mut word: Word) -> Result<String, Error> {
        word.resolve(self)?;
        Ok(word.regex(
            &|name| self.lookup(name),
            self.session.positional.as_slice(),
        ))
    }
}

impl Variables for Expansion<'_> {
    fn get(&self, name: &str) -> Option<String> {
        self.lookup(name)
    }
    fn set(&mut self, name: &str, value: i64) -> Result<(), &'static str> {
        if crate::session::array::subscript(name).is_some() {
            return Variables::set(self.session, name, value);
        }
        if self.session.readonly(name) {
            return Err("readonly variable");
        }
        self.assign(name, &value.to_string());
        Ok(())
    }
    fn associative_key(
        &mut self,
        name: &str,
        source: &str,
    ) -> Option<Result<String, &'static str>> {
        self.session
            .is_associative(name)
            .then(|| self.key(source).map_err(|_| "bad array subscript"))
    }
    fn nounset(&self) -> bool {
        self.session.nounset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(source: &str, env: &mut Expansion<'_>) -> Result<String, Error> {
        env.scalar(crate::parser::expansion_string(source).unwrap())
    }

    #[test]
    fn arithmetic_effects_follow_parameter_expansion_order() {
        let mut session = Session::default();
        session.assign("x", "3");
        let captured = Cell::new(None);
        let mut env = Expansion::new(&mut session, 0, &captured);
        assert_eq!(value("$x$((x=8))$x", &mut env).unwrap(), "388");
        assert_eq!(value("${x:-$((x=1))} $x", &mut env).unwrap(), "8 8");
        assert_eq!(value("${y:=$((x+1))} $y", &mut env).unwrap(), "9 9");
    }

    #[test]
    fn writes_to_an_environment_prefix_stay_with_that_command() {
        let mut session = Session::default();
        session.assign("x", "3");
        let captured = Cell::new(None);
        let mut prefix = [("x".into(), "4".into())];
        let mut env = Expansion::with_assignments(&mut session, 0, &captured, &mut prefix, false);
        assert_eq!(value("$((x++))$x", &mut env).unwrap(), "45");
        assert_eq!(session.get("x").as_deref(), Some("3"));
        assert_eq!(prefix[0].1, "5");
    }

    #[test]
    fn missing_parameter_has_mode_specific_exit_status_and_expanded_message() {
        let mut session = Session::default();
        session.command_string = true;
        session.assign("message", "missing input");
        let captured = Cell::new(None);
        let mut env = Expansion::new(&mut session, 0, &captured);
        let missing = value("${missing:?${message}}", &mut env).unwrap_err();
        assert_eq!(missing.to_string(), "missing: missing input");
        assert_eq!(missing.status, 127);
        env.session.command_string = false;
        assert_eq!(value("${missing?}", &mut env).unwrap_err().status, 1);
    }
}
