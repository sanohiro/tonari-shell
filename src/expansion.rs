use crate::arithmetic::{self, Variables};
use crate::parser::{Statement, Word};
use crate::{Session, execute};
use std::cell::Cell;

#[derive(Debug)]
pub struct Error {
    message: std::borrow::Cow<'static, str>,
    pub fatal: bool,
    pub status: u8,
}

impl Error {
    pub fn fatal(message: impl Into<std::borrow::Cow<'static, str>>, status: u8) -> Self {
        Self {
            message: message.into(),
            fatal: true,
            status,
        }
    }

    pub fn flow(&self, session: &Session) -> crate::Flow {
        if self.fatal && !session.interactive {
            crate::Flow::Exit(self.status)
        } else {
            crate::Flow::Continue(self.status)
        }
    }
}

impl From<&'static str> for Error {
    fn from(message: &'static str) -> Self {
        Self {
            message: message.into(),
            fatal: false,
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
        if crate::parser::is_identifier(name) {
            self.assign(name, text);
            return Ok(());
        }
        if let Some((base, index)) = crate::session::array::subscript(name) {
            if !matches!(index, "@" | "*") {
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

    pub fn capture(&mut self, body: Vec<Statement>) -> Result<String, &'static str> {
        let overridden;
        let session = if self.assignments.is_empty() {
            &*self.session
        } else {
            let mut copy = self.session.clone();
            copy.bind(self.assignments);
            overridden = copy;
            &overridden
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

    pub fn arithmetic(&mut self, expression: &str) -> Result<i64, Error> {
        let expanded;
        let expression = if expression.contains(['$', '`']) {
            let word = crate::parser::expansion_string(expression).map_err(|e| e.message())?;
            expanded = self.scalar(word)?;
            expanded.as_str()
        } else {
            expression
        };
        arithmetic::with_variables(expression, self).map_err(|e| Error::fatal(e, 1))
    }

    pub fn fields(&mut self, word: Word) -> Result<Vec<String>, Error> {
        self.fields_with_glob(word, !self.session.noglob)
    }

    pub fn fields_with_glob(&mut self, mut word: Word, glob: bool) -> Result<Vec<String>, Error> {
        word.resolve(self)?;
        Ok(word.fields_with_glob(
            &|name| self.lookup(name),
            &self.session.positional,
            glob.then_some(self.session.shopt.glob),
        ))
    }

    pub fn scalar(&mut self, mut word: Word) -> Result<String, Error> {
        word.resolve(self)?;
        Ok(word.scalar(&|name| self.lookup(name), &self.session.positional))
    }

    pub fn pattern(&mut self, mut word: Word) -> Result<String, Error> {
        word.resolve(self)?;
        Ok(word.pattern(&|name| self.lookup(name), &self.session.positional))
    }

    pub fn regex(&mut self, mut word: Word) -> Result<String, Error> {
        word.resolve(self)?;
        Ok(word.regex(&|name| self.lookup(name), &self.session.positional))
    }
}

impl Variables for Expansion<'_> {
    fn get(&self, name: &str) -> Option<String> {
        self.lookup(name)
    }
    fn set(&mut self, name: &str, value: i64) -> Result<(), &'static str> {
        self.assign(name, &value.to_string());
        Ok(())
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
