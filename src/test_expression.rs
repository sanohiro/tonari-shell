use crate::{Flow, Session, arithmetic, builtins, pattern, word::Word};
use std::ffi::CString;

#[derive(Clone, Debug)]
pub enum Expr {
    Word(Word),
    Unary(String, Word),
    Binary(Word, String, Word),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

pub fn evaluate(expr: &Expr, session: &mut Session, status: u8) -> Flow {
    match condition(expr, session, status) {
        Ok(value) => Flow::Continue(u8::from(!value)),
        Err(flow) => flow,
    }
}

fn condition(expr: &Expr, session: &mut Session, status: u8) -> Result<bool, Flow> {
    use Expr::*;
    fn operand<'a>(
        s: &mut Session,
        word: &'a crate::word::Word,
        status: u8,
    ) -> Result<std::borrow::Cow<'a, str>, Flow> {
        if let Some(text) = word.as_literal() {
            return Ok(std::borrow::Cow::Borrowed(text));
        }
        crate::eval::expand_case(s, word.clone(), status, false).map(std::borrow::Cow::Owned)
    }
    let error = |message: &'static str| {
        eprintln!("tonari: [[: {message}");
        Flow::Continue(2)
    };
    match expr {
        Not(e) => Ok(!condition(e, session, status)?),
        And(a, b) => Ok(condition(a, session, status)? && condition(b, session, status)?),
        Or(a, b) => Ok(condition(a, session, status)? || condition(b, session, status)?),
        Word(word) => Ok(!operand(session, word, status)?.is_empty()),
        Unary(op, word) => {
            builtins::unary(op, &operand(session, word, status)?, session).map_err(error)
        }
        Binary(left, op, right) => {
            let left = operand(session, left, status)?;
            match op.as_str() {
                "=" | "==" | "!=" => {
                    if !session.shopt.nocasematch
                        && let Some(text) = right.as_literal()
                    {
                        return Ok((left == text) != (op == "!="));
                    }
                    let captured = std::cell::Cell::new(None);
                    let mut env = crate::expansion::Expansion::new(session, status, &captured);
                    let mut right = right.clone();
                    right.resolve(&mut env).map_err(|e| {
                        eprintln!("tonari: [[: {e}");
                        e.flow(env.session)
                    })?;
                    let right = right.extended_pattern(
                        &|name| env.lookup(name),
                        env.session.positional.as_slice(),
                    );
                    let matched = if env.session.shopt.nocasematch {
                        pattern::extended::insensitive_match(
                            &left,
                            &right,
                            true,
                            env.session.character_locale(),
                        )
                    } else {
                        pattern::extended::matches(&left, &right)
                    };
                    Ok(matched != (op == "!="))
                }
                "=~" => {
                    let captured = std::cell::Cell::new(None);
                    let expression = crate::expansion::Expansion::new(session, status, &captured)
                        .regex(right.clone())
                        .map_err(|e| {
                            eprintln!("tonari: [[: {e}");
                            e.flow(session)
                        })?;
                    let result = pattern::localized(|| {
                        let ignore_case = session.shopt.nocasematch;
                        let slot = &mut session.regex[usize::from(ignore_case)];
                        if slot.as_ref().is_none_or(|r| r.pattern != expression) {
                            *slot = Some(std::rc::Rc::new(Compiled::new(expression, ignore_case)?));
                        }
                        slot.as_ref().unwrap().search(&left)
                    })
                    .map_err(|()| Flow::Continue(2))?;
                    let matched = result.is_some();
                    session.replace_array("BASH_REMATCH", result.unwrap_or_default());
                    Ok(matched)
                }
                "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge" => {
                    let right = operand(session, right, status)?;
                    let a = arithmetic::run(&left, session).map_err(error)?;
                    let b = arithmetic::run(&right, session).map_err(error)?;
                    Ok(match op.as_str() {
                        "-eq" => a == b,
                        "-ne" => a != b,
                        "-lt" => a < b,
                        "-le" => a <= b,
                        "-gt" => a > b,
                        _ => a >= b,
                    })
                }
                _ => builtins::binary(&left, op, &operand(session, right, status)?).map_err(error),
            }
        }
    }
}

pub struct Compiled {
    pattern: String,
    regex: Box<libc::regex_t>,
    captures: usize,
}

impl Compiled {
    fn new(pattern: String, ignore_case: bool) -> Result<Self, ()> {
        let encoded = CString::new(pattern.as_str()).map_err(|_| ())?;
        let mut regex = Box::<libc::regex_t>::new_uninit();
        let flags = if ignore_case {
            libc::REG_EXTENDED | libc::REG_ICASE
        } else {
            libc::REG_EXTENDED
        };
        if unsafe { libc::regcomp(regex.as_mut_ptr(), encoded.as_ptr(), flags) } != 0 {
            return Err(());
        }
        let captures = capture_count(&pattern) + 1;
        Ok(Self {
            pattern,
            regex: unsafe { regex.assume_init() },
            captures,
        })
    }

    fn search(&self, text: &str) -> Result<Option<Vec<String>>, ()> {
        let encoded = CString::new(text).map_err(|_| ())?;
        let mut matched = vec![
            libc::regmatch_t {
                rm_so: -1,
                rm_eo: -1
            };
            self.captures
        ];
        match unsafe {
            libc::regexec(
                &*self.regex,
                encoded.as_ptr(),
                matched.len(),
                matched.as_mut_ptr(),
                0,
            )
        } {
            0 => Ok(Some(
                matched
                    .iter()
                    .map(|m| {
                        text.get(m.rm_so as usize..m.rm_eo as usize)
                            .unwrap_or_default()
                            .to_owned()
                    })
                    .collect(),
            )),
            libc::REG_NOMATCH => Ok(None),
            _ => Err(()),
        }
    }
}

impl Drop for Compiled {
    fn drop(&mut self) {
        unsafe { libc::regfree(&mut *self.regex) };
    }
}

// libc exposes regex_t as opaque on our targets. Count ERE capture groups
// so trailing unmatched groups are still published as empty Bash array elements.
fn capture_count(expression: &str) -> usize {
    let mut bytes = expression.as_bytes();
    let mut groups = 0;
    while let Some((&byte, rest)) = bytes.split_first() {
        bytes = rest;
        match byte {
            b'\\' => bytes = bytes.get(1..).unwrap_or_default(),
            b'(' => groups += 1,
            b'[' => {
                bytes = bytes.strip_prefix(b"^").unwrap_or(bytes);
                bytes = bytes.strip_prefix(b"]").unwrap_or(bytes);
                while let Some((&c, after)) = bytes.split_first() {
                    bytes = after;
                    if c == b']' {
                        break;
                    }
                    if c == b'[' {
                        if let Some(&marker @ (b':' | b'=' | b'.')) = after.first() {
                            if let Some(end) = after.windows(2).position(|s| s == [marker, b']']) {
                                bytes = &after[end + 2..];
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    groups
}
