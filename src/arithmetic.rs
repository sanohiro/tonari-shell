use crate::{Flow, Session};

type Result<T> = std::result::Result<T, &'static str>;

pub(crate) trait Variables {
    fn get(&self, name: &str) -> Option<String>;
    fn set(&mut self, name: &str, value: i64) -> Result<()>;
    fn nounset(&self) -> bool {
        false
    }
}

impl Variables for Session {
    fn get(&self, name: &str) -> Option<String> {
        self.get(name)
    }
    fn set(&mut self, name: &str, value: i64) -> Result<()> {
        self.assign(name, &value.to_string());
        Ok(())
    }
    fn nounset(&self) -> bool {
        self.nounset
    }
}

struct ReadOnly<'a, F>(&'a F);
impl<F: Fn(&str) -> Option<String>> Variables for ReadOnly<'_, F> {
    fn get(&self, name: &str) -> Option<String> {
        (self.0)(name)
    }
    fn set(&mut self, _: &str, _: i64) -> Result<()> {
        Err("assignment in arithmetic expansion is not implemented yet")
    }
}

pub fn evaluate(source: &str, lookup: &impl Fn(&str) -> Option<String>) -> Result<i64> {
    calculate(source, &mut ReadOnly(lookup), 0)
}

pub(crate) fn with_variables(source: &str, vars: &mut dyn Variables) -> Result<i64> {
    calculate(source, vars, 0)
}

pub fn run(source: &str, session: &mut Session) -> Result<i64> {
    calculate(source, session, 0)
}

pub fn expanded(source: &str, session: &mut Session, status: u8) -> Result<i64> {
    if !source.contains(['$', '`']) {
        return run(source, session);
    }
    let captured = std::cell::Cell::new(None);
    crate::expansion::Expansion::new(session, status, &captured)
        .arithmetic(source)
        .map_err(|_| "invalid arithmetic expansion")
}

pub fn let_builtin(args: &[String], session: &mut Session) -> u8 {
    let args = if args.first().is_some_and(|arg| arg == "--") {
        &args[1..]
    } else {
        args
    };
    if args.is_empty() {
        eprintln!("tonari: let: expression expected");
        return 1;
    }
    let mut last = 0;
    for expr in args {
        match run(expr, session) {
            Ok(value) => last = value,
            Err(message) => {
                eprintln!("tonari: let: {message}");
                return 1;
            }
        }
    }
    u8::from(last == 0)
}

pub fn command(source: &str, session: &mut Session, status: u8) -> Flow {
    match expanded(source, session, status) {
        Ok(n) => Flow::Continue(u8::from(n == 0)),
        Err(e) => {
            eprintln!("tonari: {e}");
            Flow::Continue(1)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Token<'a> {
    End,
    Number(&'a str),
    Name(&'a str),
    Op(&'a str),
}

struct Parser<'a> {
    source: &'a str,
    token: Token<'a>,
}

impl<'a> Parser<'a> {
    fn advance(&mut self) -> Result<Token<'a>> {
        let old = self.token;
        let source = self.source.trim_start_matches([' ', '\t', '\n', '\r']);
        let length = |s: &str| {
            s.bytes()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_')
                .count()
        };
        let (token, consumed) = match source.as_bytes().first() {
            None => (Token::End, 0),
            Some(b'0'..=b'9') => {
                let n = source
                    .bytes()
                    .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'#' | b'@'))
                    .count();
                (Token::Number(&source[..n]), n)
            }
            Some(b'a'..=b'z' | b'A'..=b'Z' | b'_') => {
                let n = length(source);
                (Token::Name(&source[..n]), n)
            }
            Some(b'$') => {
                let braced = source.starts_with("${");
                let rest = &source[if braced { 2 } else { 1 }..];
                let n = length(rest);
                if n == 0 || (braced && !rest[n..].starts_with('}')) {
                    return Err("invalid arithmetic parameter");
                }
                (Token::Name(&rest[..n]), n + if braced { 3 } else { 1 })
            }
            _ => {
                let op = [
                    "<<=", ">>=", "++", "--", "**", "&&", "||", "<<", ">>", "<=", ">=", "==", "!=",
                    "+=", "-=", "*=", "/=", "%=", "&=", "^=", "|=",
                ]
                .into_iter()
                .find(|op| source.starts_with(op))
                .map_or_else(
                    || source.get(..1).filter(|s| "+-*/%()!:~&|^<>=?,".contains(s)),
                    Some,
                )
                .ok_or("invalid arithmetic token")?;
                (Token::Op(op), op.len())
            }
        };
        self.token = token;
        self.source = &source[consumed..];
        Ok(old)
    }

    fn expression(
        &mut self,
        variables: &mut dyn Variables,
        minimum: u8,
        active: bool,
        depth: u8,
    ) -> Result<Value<'a>> {
        if depth >= 96 {
            return Err("arithmetic nesting limit exceeded");
        }
        let mut left = self.atom(variables, active, depth + 1)?;
        while let Token::Op(op) = self.token {
            let Some((precedence, right_associative)) = precedence(op) else {
                break;
            };
            if precedence < minimum {
                break;
            }
            self.advance()?;
            if op == "?" {
                let test = left.read(variables, active, depth + 1)? != 0;
                let yes = self
                    .expression(variables, 1, active && test, depth + 1)?
                    .read(variables, active && test, depth + 1)?;
                if self.advance()? != Token::Op(":") {
                    return Err("missing colon in arithmetic conditional");
                }
                let no = self
                    .expression(variables, precedence, active && !test, depth + 1)?
                    .read(variables, active && !test, depth + 1)?;
                left = Value::Number(if test { yes } else { no });
                continue;
            }
            let a = if op == "=" {
                0
            } else {
                left.read(variables, active, depth + 1)?
            };
            let run_right = active && !(op == "&&" && a == 0 || op == "||" && a != 0);
            let b = self
                .expression(
                    variables,
                    precedence + u8::from(!right_associative),
                    run_right,
                    depth + 1,
                )?
                .read(variables, run_right, depth + 1)?;
            if precedence == 2 {
                let Value::Variable(name) = left else {
                    return Err("arithmetic assignment requires a variable");
                };
                let result = if !active || op == "=" {
                    b
                } else {
                    binary(&op[..op.len() - 1], a, b)?
                };
                if active {
                    variables.set(name, result)?;
                }
                left = Value::Number(result);
            } else {
                left = Value::Number(if active { binary(op, a, b)? } else { 0 });
            }
        }
        Ok(left)
    }

    fn atom(
        &mut self,
        variables: &mut dyn Variables,
        active: bool,
        depth: u8,
    ) -> Result<Value<'a>> {
        if depth >= 96 {
            return Err("arithmetic nesting limit exceeded");
        }
        let mut value = match self.advance()? {
            Token::Number(s) => Value::Number(number(s)?),
            Token::Name(n) => Value::Variable(n),
            Token::Op("(") => {
                let v = self.expression(variables, 1, active, depth + 1)?;
                if self.advance()? != Token::Op(")") {
                    return Err("missing arithmetic closing parenthesis");
                }
                v
            }
            Token::Op(op @ ("+" | "-" | "!" | "~" | "++" | "--")) => {
                let operand = self.atom(variables, active, depth + 1)?;
                let n = operand.read(variables, active, depth + 1)?;
                let result = match op {
                    "+" => n,
                    "-" => n.wrapping_neg(),
                    "!" => i64::from(n == 0),
                    "~" => !n,
                    _ => {
                        let Value::Variable(name) = operand else {
                            return Err("arithmetic increment requires a variable");
                        };
                        let next = n.wrapping_add(if op == "++" { 1 } else { -1 });
                        if active {
                            variables.set(name, next)?;
                        }
                        next
                    }
                };
                Value::Number(result)
            }
            _ => return Err("expected an arithmetic operand"),
        };
        if let Token::Op(op @ ("++" | "--")) = self.token {
            self.advance()?;
            let Value::Variable(name) = value else {
                return Err("arithmetic increment requires a variable");
            };
            let old = value.read(variables, active, depth + 1)?;
            if active {
                variables.set(name, old.wrapping_add(if op == "++" { 1 } else { -1 }))?;
            }
            value = Value::Number(old);
        }
        Ok(value)
    }
}

#[derive(Clone, Copy)]
enum Value<'a> {
    Number(i64),
    Variable(&'a str),
}
impl Value<'_> {
    fn read(self, variables: &mut dyn Variables, active: bool, depth: u8) -> Result<i64> {
        if !active {
            return Ok(0);
        }
        match self {
            Self::Number(n) => Ok(n),
            Self::Variable(name) => match variables.get(name) {
                Some(s) => calculate(&s, variables, depth + 1),
                None if variables.nounset() => Err("unbound arithmetic variable"),
                None => Ok(0),
            },
        }
    }
}

fn calculate(source: &str, variables: &mut dyn Variables, depth: u8) -> Result<i64> {
    if depth >= 96 {
        return Err("arithmetic nesting limit exceeded");
    }
    let mut p = Parser {
        source,
        token: Token::End,
    };
    p.advance()?;
    if p.token == Token::End {
        return Ok(0);
    }
    let value = p
        .expression(variables, 1, true, depth + 1)?
        .read(variables, true, depth + 1)?;
    if p.token != Token::End {
        return Err("unexpected arithmetic token");
    }
    Ok(value)
}

fn precedence(op: &str) -> Option<(u8, bool)> {
    let prec = match op {
        "," => 1,
        "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "<<=" | ">>=" | "&=" | "^=" | "|=" => 2,
        "?" => 3,
        "||" => 4,
        "&&" => 5,
        "|" => 6,
        "^" => 7,
        "&" => 8,
        "==" | "!=" => 9,
        "<" | ">" | "<=" | ">=" => 10,
        "<<" | ">>" => 11,
        "+" | "-" => 12,
        "*" | "/" | "%" => 13,
        "**" => 14,
        _ => return None,
    };
    Some((prec, matches!(prec, 2 | 3 | 14)))
}

fn binary(op: &str, a: i64, b: i64) -> Result<i64> {
    Ok(match op {
        "," => b,
        "||" => i64::from(a != 0 || b != 0),
        "&&" => i64::from(a != 0 && b != 0),
        "|" => a | b,
        "&" => a & b,
        "^" => a ^ b,
        "==" => i64::from(a == b),
        "!=" => i64::from(a != b),
        "<" => i64::from(a < b),
        ">" => i64::from(a > b),
        "<=" => i64::from(a <= b),
        ">=" => i64::from(a >= b),
        "<<" => a.wrapping_shl(b as u32),
        ">>" => a.wrapping_shr(b as u32),
        "+" => a.wrapping_add(b),
        "-" => a.wrapping_sub(b),
        "*" => a.wrapping_mul(b),
        "/" | "%" if b == 0 => return Err("division by zero"),
        "/" => a.wrapping_div(b),
        "%" => a.wrapping_rem(b),
        "**" if b < 0 => return Err("negative arithmetic exponent"),
        "**" => {
            let mut power = b as u64;
            let mut base = a;
            let mut result = 1i64;
            while power != 0 {
                if power & 1 != 0 {
                    result = result.wrapping_mul(base);
                }
                base = base.wrapping_mul(base);
                power >>= 1;
            }
            result
        }
        _ => return Err("unknown arithmetic operator"),
    })
}

fn number(s: &str) -> Result<i64> {
    let (base, digits) = if let Some((base, digits)) = s.split_once('#') {
        let base = base.parse::<u32>().map_err(|_| "invalid arithmetic base")?;
        if !(2..=64).contains(&base) {
            return Err("invalid arithmetic base");
        }
        (base, digits)
    } else if let Some(digits) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        (16, digits)
    } else if s.starts_with('0') {
        (8, s)
    } else {
        (10, s)
    };
    if digits.is_empty() {
        return Err("missing digits in arithmetic constant");
    }
    let mut value = 0i64;
    for c in digits.bytes() {
        let d = match c {
            b'0'..=b'9' => u32::from(c - b'0'),
            b'a'..=b'z' => 10 + u32::from(c - b'a'),
            b'A'..=b'Z' => (if base <= 36 { 10 } else { 36 }) + u32::from(c - b'A'),
            b'@' => 62,
            b'_' => 63,
            _ => return Err("invalid arithmetic digit"),
        };
        if d >= base {
            return Err("invalid arithmetic digit");
        }
        value = value
            .wrapping_mul(i64::from(base))
            .wrapping_add(i64::from(d));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precedence_numbers_and_lazy_branches() {
        let lookup = |name: &str| (name == "n").then(|| "3+1".to_owned());
        for (s, v) in [
            ("2+3*4", 14),
            ("2**3**2", 512),
            ("16#FF + 010 + 0x10", 279),
            ("n==4 ? 2*n : 1/0", 8),
            ("0 && (1/0)", 0),
            ("1 || (1/0)", 1),
            ("-1 >> 2", -1),
        ] {
            assert_eq!(evaluate(s, &lookup).unwrap(), v, "{s}");
        }
        assert!(evaluate("08", &lookup).is_err());
        assert!(evaluate("x", &|_| Some("x".into())).is_err());
    }
}
