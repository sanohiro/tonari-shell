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

pub fn evaluate(expr: Expr, session: &mut Session, status: u8) -> Flow {
    match condition(expr, session, status) {
        Ok(value) => Flow::Continue(u8::from(!value)),
        Err(flow) => flow,
    }
}

fn condition(expr: Expr, session: &mut Session, status: u8) -> Result<bool, Flow> {
    use Expr::*;
    let operand = |s: &mut Session, word| crate::eval::expand_case(s, word, status, false);
    let error = |message: &'static str| {
        eprintln!("tonari: [[: {message}");
        Flow::Continue(2)
    };
    match expr {
        Not(e) => Ok(!condition(*e, session, status)?),
        And(a, b) => Ok(condition(*a, session, status)? && condition(*b, session, status)?),
        Or(a, b) => Ok(condition(*a, session, status)? || condition(*b, session, status)?),
        Word(word) => Ok(!operand(session, word)?.is_empty()),
        Unary(op, word) => builtins::unary(&op, &operand(session, word)?, session).map_err(error),
        Binary(left, op, right) => {
            let left = operand(session, left)?;
            match op.as_str() {
                "=" | "==" | "!=" => {
                    let right = crate::eval::expand_case(session, right, status, true)?;
                    Ok(pattern::matches(&left, &right) != (op == "!="))
                }
                "=~" => {
                    let captured = std::cell::Cell::new(None);
                    let expression = crate::expansion::Expansion::new(session, status, &captured)
                        .regex(right)
                        .map_err(|e| {
                            eprintln!("tonari: [[: {e}");
                            e.flow(session)
                        })?;
                    let result = regex(&left, &expression).map_err(error)?;
                    let matched = result.is_some();
                    session.replace_array("BASH_REMATCH", result.unwrap_or_default());
                    Ok(matched)
                }
                "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge" => {
                    let right = operand(session, right)?;
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
                _ => builtins::binary(&left, &op, &operand(session, right)?).map_err(error),
            }
        }
    }
}

fn regex(text: &str, expression: &str) -> Result<Option<Vec<String>>, &'static str> {
    crate::pattern::localized(|| regex_local(text, expression))
}

fn regex_local(text: &str, expression: &str) -> Result<Option<Vec<String>>, &'static str> {
    let captures = capture_count(expression) + 1;
    let expression = CString::new(expression).map_err(|_| "NUL in regular expression")?;
    let encoded = CString::new(text).map_err(|_| "NUL in regular expression input")?;
    let mut regex = std::mem::MaybeUninit::uninit();
    if unsafe { libc::regcomp(regex.as_mut_ptr(), expression.as_ptr(), libc::REG_EXTENDED) } != 0 {
        return Err("invalid regular expression");
    }
    let mut regex = unsafe { regex.assume_init() };
    let mut matched = vec![
        libc::regmatch_t {
            rm_so: -1,
            rm_eo: -1
        };
        captures
    ];
    let result =
        unsafe { libc::regexec(&regex, encoded.as_ptr(), captures, matched.as_mut_ptr(), 0) };
    unsafe { libc::regfree(&mut regex) };
    match result {
        0 => Ok(Some(
            matched
                .iter()
                .map(|m| {
                    text.get(m.rm_so as usize..m.rm_eo as usize)
                        .unwrap_or_default()
                        .to_string()
                })
                .collect(),
        )),
        libc::REG_NOMATCH => Ok(None),
        _ => Err("regular expression match failed"),
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
