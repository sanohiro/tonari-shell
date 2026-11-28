use crate::arithmetic::{self, Variables};
use crate::parser::{Statement, Word};
use crate::{Session, execute};
use std::cell::Cell;

#[derive(Debug)]
enum Recovery {
    Control(crate::Flow),
    Command,
    Input,
    Line,
    Exit,
}

#[derive(Debug)]
pub struct Error {
    message: std::borrow::Cow<'static, str>,
    recovery: Recovery,
    pub status: u8,
}

impl Error {
    pub(crate) fn in_expression(mut self, text: &str) -> Self {
        self.message = format!("{text}: {}", self.message).into();
        self
    }

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

    pub fn discard_line(message: &'static str) -> Self {
        Self {
            message: message.into(),
            recovery: Recovery::Line,
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
                crate::Flow::Unwind(n)
                | crate::Flow::Discard(n)
                | crate::Flow::Exit(n)
                | crate::Flow::Break(_, n) => n,
                _ => 0,
            },
            recovery: Recovery::Control(flow),
        }
    }

    pub(crate) fn report(&self, context: &str) {
        if !matches!(self.recovery, Recovery::Control(_)) {
            diagnostic!("{context}{self}");
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
            Recovery::Line | Recovery::Input => crate::Flow::Discard(self.status),
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

pub(crate) enum AssignmentElement {
    Index(String, i64),
    Key(String, String),
}

impl AssignmentElement {
    pub(crate) fn value(&self, session: &Session) -> Option<String> {
        match self {
            Self::Index(base, index) => session.element(base, *index),
            Self::Key(base, key) => session.assoc_value(base, key).map(str::to_owned),
        }
    }
}

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
            .or_else(|| {
                if self.assignments.is_empty() {
                    self.session.referenced_scalar(name)
                } else {
                    self.session.stored_scalar(name)
                }
            })
    }

    pub fn lookup(&self, name: &str) -> Option<String> {
        self.assignments
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .or_else(|| execute::parameter(self.session, self.status(), name))
    }

    pub(crate) fn ifs_separator(&self) -> std::borrow::Cow<'_, str> {
        let ifs = self
            .assignments
            .iter()
            .rev()
            .find(|(key, _)| key == "IFS")
            .map_or_else(|| self.session.ifs(), |(_, value)| value.as_str().into());
        let end = ifs.chars().next().map_or(0, char::len_utf8);
        match ifs {
            std::borrow::Cow::Borrowed(value) => (&value[..end]).into(),
            std::borrow::Cow::Owned(mut value) => {
                value.truncate(end);
                value.into()
            }
        }
    }

    pub(crate) fn has_pending_assignments(&self) -> bool {
        !self.assignments.is_empty()
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

    pub(crate) fn assignment_element(
        &mut self,
        name: &str,
    ) -> Result<Option<AssignmentElement>, Error> {
        let resolved = self.session.reference_name(name)?;
        let Some((base, expr)) = crate::session::array::subscript(&resolved) else {
            return Ok(None);
        };
        if matches!(expr, "@" | "*") {
            return Ok(None);
        }
        let stored = name != resolved && !name.contains('[');
        let base = base.to_owned();
        Ok(Some(if self.session.is_associative(&base) {
            let key = if stored {
                arithmetic::data_key(expr, self)?
            } else {
                self.key(expr)?
            };
            AssignmentElement::Key(base, key)
        } else {
            let n = if stored {
                arithmetic::data_index(expr, self)?
            } else {
                self.arithmetic(expr)?
            };
            let index = self.session.relative_index(&base, n)?;
            AssignmentElement::Index(base, index)
        }))
    }

    pub(crate) fn assign_parameter(
        &mut self,
        name: &str,
        element: Option<AssignmentElement>,
        mut value: String,
    ) -> Result<String, Error> {
        let resolved;
        let base = match &element {
            Some(AssignmentElement::Index(base, _) | AssignmentElement::Key(base, _)) => {
                base.as_str()
            }
            None if crate::parser::is_identifier(name) => {
                resolved = self.session.reference_name(name)?;
                resolved.as_ref()
            }
            None => {
                return Err(Error::discard(format!(
                    "${name}: cannot assign in this way"
                )));
            }
        };
        if self.session.readonly(base) {
            return Err(Error::readonly(base));
        }
        let (integer, changes_case) = self.session.assignment_attributes(base);
        if integer {
            value = self.arithmetic_value(&value)?.to_string();
        }
        match &element {
            Some(AssignmentElement::Index(base, index)) => {
                self.session.assign_element(base, *index, value.clone())
            }
            Some(AssignmentElement::Key(base, key)) => {
                self.session.assign_key(base, key.clone(), value.clone())?
            }
            None => self.assign(base, &value),
        }
        if changes_case {
            value = element
                .as_ref()
                .map_or_else(|| self.lookup(base), |e| e.value(self.session))
                .unwrap_or_default();
        }
        Ok(value)
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

    pub fn current_capture(&mut self, body: &[Statement], reply: bool) -> Result<String, Error> {
        let last = self.status();
        let (text, status) =
            crate::subshell::current_substitution(self.session, body, last, reply)?;
        self.captured.set(Some(status));
        Ok(text)
    }

    pub fn capture(&mut self, body: &[Statement]) -> Result<String, &'static str> {
        let status = self.captured.get().unwrap_or(self.initial);
        let bindings = (!self.assignments.is_empty()).then(|| self.session.bind(self.assignments));
        let result = crate::capture_substitution(self.session, body, status);
        if let Some(bindings) = bindings {
            self.session.restore(bindings);
        }
        let (value, status) = result?;
        self.captured.set(Some(status));
        Ok(value)
    }

    pub fn status(&self) -> u8 {
        if self.session.posix {
            self.initial
        } else {
            self.captured.get().unwrap_or(self.initial)
        }
    }

    pub fn array_items(
        &mut self,
        parts: crate::parser::ArrayItems<Word>,
        associative: bool,
        declaration_operand: bool,
    ) -> Result<crate::session::array::ArrayItems, Error> {
        use crate::session::array::{ArrayItems, Index};
        if associative {
            let pairs = parts
                .words
                .first()
                .is_some_and(|(index, _)| index.is_none());
            if pairs {
                let mut sources = parts.pair_sources.into_iter().peekable();
                let mut words = parts.words.into_iter().enumerate().map(|(i, (_, word))| {
                    let source = sources
                        .next_if(|(position, _)| *position == i)
                        .map(|(_, s)| s);
                    (word, source)
                });
                let mut expand = |(word, source): (Word, Option<std::rc::Rc<str>>),
                                  assignment_value| {
                    let word = if let Some(source) = source {
                        crate::parser::with_subscript_aliases(self.session, &source, || {
                            crate::parser::associative_pair_word(&source, assignment_value)
                        })
                        .map_err(|e| e.message())?
                    } else {
                        word
                    };
                    self.scalar(word)
                };
                let mut items = Vec::with_capacity(words.len().div_ceil(2));
                while let Some(key) = words.next() {
                    let key = expand(key, false)?;
                    let next = words.next();
                    // A declaration expands its operands before the builtin runs.
                    // A direct assignment expands a value only after a valid key.
                    if key.is_empty() && !declaration_operand {
                        diagnostic!("bad array subscript");
                        continue;
                    }
                    let value = next
                        .map(|word| expand(word, !declaration_operand))
                        .transpose()?
                        .unwrap_or_default();
                    if key.is_empty() {
                        diagnostic!("bad array subscript");
                    } else {
                        items.push((key, value, false));
                    }
                }
                return Ok(ArrayItems::Associative(items, None));
            }
            let mut items = Vec::with_capacity(parts.words.len());
            let mut error = None;
            for (index, word) in parts.words {
                let Some(index) = index else {
                    error = Some("missing associative array subscript");
                    break;
                };
                let key = self.expanded_key(&index.index)?;
                let value = self.scalar(word)?;
                if key.is_empty() {
                    error = Some("bad array subscript");
                    break;
                }
                items.push((key, value, index.append));
            }
            return Ok(ArrayItems::Associative(items, error));
        }
        let mut items = Vec::with_capacity(parts.words.len());
        for (index, word) in parts.indexed_words() {
            if let Some(crate::parser::Subscript { index, append }) = index {
                // Shell expansions in a subscript precede its RHS, but arithmetic
                // in the resulting text runs when the initialized array is written.
                let index = if index.contains(['$', '`', '"', '\'', '\\']) {
                    let word = crate::parser::with_subscript_aliases(self.session, &index, || {
                        crate::parser::initializer_index_word(&index)
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

    pub(crate) fn expanded_key(&mut self, source: &str) -> Result<String, Error> {
        if let Some(rest) = source.strip_prefix("~/")
            && !rest.contains(['$', '`', '"', '\'', '\\'])
            && let Some(mut home) = self.lookup("HOME")
        {
            home.push('/');
            home.push_str(rest);
            return Ok(home);
        }
        if let Some((prefix, name)) = source.split_once('$')
            && !prefix.starts_with('~')
            && prefix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            && let Some(value) = self.borrowed_scalar(name)
        {
            return Ok([prefix, value].concat());
        }
        if source.starts_with('~') || source.contains(['$', '`', '"', '\'', '\\']) {
            let word = crate::parser::with_subscript_aliases(self.session, source, || {
                crate::parser::subscript_word(source, true)
            })
            .map_err(|e| e.message())?;
            self.scalar(word)
        } else {
            Ok(source.to_owned())
        }
    }

    pub fn key(&mut self, source: &str) -> Result<String, Error> {
        let key = self.expanded_key(source)?;
        if key.is_empty() {
            return Err("bad array subscript".into());
        }
        Ok(key)
    }

    pub fn arithmetic(&mut self, expression: &str) -> Result<i64, Error> {
        let text = self.arithmetic_text(expression)?;
        self.arithmetic_value(&text)
    }

    /// Operands that already went through shell expansion must only be parsed as arithmetic.
    pub(crate) fn arithmetic_value(&mut self, text: &str) -> Result<i64, Error> {
        arithmetic::with_variables(text, self).map_err(|e| e.error(text, self.session, true))
    }

    pub(crate) fn arithmetic_text<'s>(
        &mut self,
        expression: &'s str,
    ) -> Result<std::borrow::Cow<'s, str>, Error> {
        if expression.contains(['$', '`', '"', '\\']) {
            let text = if expression.contains('[') {
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
                            source.push_str(&crate::word::quote::single_quote(&key));
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
            Ok(text.into())
        } else {
            Ok(expression.into())
        }
    }

    pub(crate) fn redirect_path(&mut self, word: Word) -> Result<String, Error> {
        if self.session.posix && !self.session.interactive {
            return self.scalar(word);
        }
        let mut fields = self.fields(word)?.into_iter();
        let path = fields.next().ok_or("ambiguous redirect")?;
        if fields.next().is_some() {
            return Err("ambiguous redirect".into());
        }
        Ok(path)
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
        if !self.has_pending_assignments()
            && let Word::Expanded(parts) = &mut word
            && let [part] = parts.as_mut_slice()
        {
            use crate::word::Part;
            let parameter = match &*part {
                Part::Parameter { name, quoted } => Some((name.as_ref(), *quoted)),
                _ => None,
            };
            if let Some((name, quoted)) = parameter
                && let Some((base, at, keys)) = crate::word::array_elements_name(name)
            {
                if quoted && at && self.session.append_array_fields(base, keys, fields) {
                    return Ok(());
                }
                if !quoted && self.session.is_associative(base) {
                    // Feed the stored elements to ordinary IFS splitting and globbing.
                    let mut values = Vec::new();
                    if self.session.append_array_fields(base, keys, &mut values) {
                        *part = Part::Elements { values, at, quoted };
                    }
                }
            }
        }
        word.append_fields(
            &|name| self.lookup(name),
            self.session.positional.as_slice(),
            || {
                self.borrowed_scalar("IFS")
                    .map(Into::into)
                    .or_else(|| self.lookup("IFS").map(Into::into))
            },
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
    fn session(&self) -> &Session {
        self.session
    }
    fn visible_nameref(&self, name: &str) -> bool {
        self.session.is_nameref(name) && !self.shadows(name)
    }
    fn scalar_text(&self, name: &str) -> Option<&str> {
        self.borrowed_scalar(name)
    }
    fn get(&self, name: &str) -> Result<Option<String>, &'static str> {
        if self.session.is_nameref(name) && !self.shadows(name) {
            self.session.arithmetic_get(name)
        } else {
            Ok(self.lookup(name))
        }
    }
    fn indexed_decimal(&self, name: &str, index: i64) -> Option<i64> {
        if self.shadows(name) {
            None
        } else {
            self.session.indexed_scalar_decimal(name, index)
        }
    }
    fn set_indexed(&mut self, name: &str, index: i64, value: i64) -> Result<(), &'static str> {
        Variables::set_indexed(self.session, name, index, value)
    }
    fn set(&mut self, name: &str, value: i64) -> Result<(), &'static str> {
        if !self.shadows(name)
            || crate::session::array::subscript(name).is_some()
            || self.session.is_nameref(name)
        {
            return Variables::set(self.session, name, value);
        }
        if self.session.readonly(name) {
            return Err(crate::session::READONLY);
        }
        self.assign(name, &value.to_string());
        Ok(())
    }
    fn set_key(&mut self, name: &str, key: String, value: i64) -> Result<(), &'static str> {
        Variables::set_key(self.session, name, key, value)
    }
    fn is_associative(&self, name: &str) -> bool {
        self.session.is_associative(name)
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
