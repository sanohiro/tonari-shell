use crate::{Flow, Session};

type Result<T> = std::result::Result<T, &'static str>;

pub(crate) trait Variables {
    fn get(&self, name: &str) -> Option<String>;
    fn decimal(&self, _name: &str) -> Option<i64> {
        None
    }
    fn set(&mut self, name: &str, value: i64) -> Result<()>;
    fn associative_key(&mut self, _name: &str, _source: &str) -> Option<Result<String>> {
        None
    }
    fn nounset(&self) -> bool {
        false
    }
}

impl Variables for Session {
    fn decimal(&self, name: &str) -> Option<i64> {
        self.scalar_decimal(name)
    }
    fn get(&self, name: &str) -> Option<String> {
        self.get(name)
    }
    fn set(&mut self, name: &str, value: i64) -> Result<()> {
        if let Some((base, index)) = crate::session::array::subscript(name) {
            return if self.is_associative(base) {
                self.assign_key(base, index.into(), value.to_string())
            } else {
                self.assign_target(name, &value.to_string())
            };
        }
        if self.is_nameref(name) {
            return self.assign_target(name, &value.to_string());
        }
        if let Some(result) = self.write_scalar_decimal(name, value) {
            return result;
        }
        if self.readonly(name) {
            return Err("readonly variable");
        }
        self.assign(name, &value.to_string());
        Ok(())
    }
    fn associative_key(&mut self, name: &str, source: &str) -> Option<Result<String>> {
        self.is_associative(name)
            .then(|| self.subscript_key(source))
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

// A subscript obtained through indirection is data. Support pure parameter
// and arithmetic forms without dispatching commands or assigning variables.
pub(crate) fn index(source: &str, lookup: &impl Fn(&str) -> Option<String>) -> Result<i64> {
    if !source.contains("${") && !source.contains("$((") {
        return evaluate(source, lookup);
    }
    fn text(
        word: crate::word::Word,
        lookup: &impl Fn(&str) -> Option<String>,
        depth: u8,
    ) -> Result<String> {
        use crate::word::{Alternative, Part, Word, is_identifier};
        if depth >= 32 {
            return Err("array subscript nested too deeply");
        }
        let parts = match word {
            Word::Literal(value) => return Ok(value),
            Word::Expanded(parts) => parts,
            _ => return Err("invalid indirect array subscript"),
        };
        let mut output = String::new();
        for part in parts {
            match part {
                Part::Text(s) | Part::Pattern(s) => output.push_str(&s),
                Part::Parameter { name, .. } if is_identifier(&name) => {
                    if let Some(s) = lookup(&name) {
                        output.push_str(&s);
                    }
                }
                Part::Alternative {
                    name,
                    op: op @ (Alternative::Default | Alternative::Alternate),
                    check_empty,
                    word,
                    ..
                } if is_identifier(&name) => {
                    let value = lookup(&name).filter(|s| !check_empty || !s.is_empty());
                    if (op == Alternative::Default) == value.is_none() {
                        output.push_str(&text(*word, lookup, depth + 1)?);
                    } else if op == Alternative::Default {
                        output.push_str(value.as_deref().unwrap_or(""));
                    }
                }
                Part::Arithmetic { expression, .. } => {
                    let word =
                        crate::parser::arithmetic_word(&expression).map_err(|e| e.message())?;
                    output
                        .push_str(&evaluate(&text(word, lookup, depth + 1)?, lookup)?.to_string());
                }
                _ => return Err("invalid indirect array subscript"),
            }
        }
        Ok(output)
    }
    let word = crate::parser::arithmetic_word(source).map_err(|e| e.message())?;
    evaluate(&text(word, lookup, 0)?, lookup)
}

pub(crate) fn with_variables(source: &str, vars: &mut dyn Variables) -> Result<i64> {
    calculate(source, vars, 0)
}

pub fn run(source: &str, session: &mut Session) -> Result<i64> {
    calculate(source, session, 0)
}

pub fn expanded(source: &str, session: &mut Session, status: u8) -> Result<i64> {
    if !source.contains(['$', '`', '"', '\\']) {
        return run(source, session);
    }
    let captured = std::cell::Cell::new(None);
    crate::expansion::Expansion::new(session, status, &captured)
        .arithmetic(source)
        .map_err(|_| "invalid arithmetic expansion")
}

// Loop headers are immutable, but their operands can be reassigned on every
// iteration. Decode frequent forms once and consult the live shell values.
pub(crate) enum Repeated<'a> {
    Other(&'a str),
    Compare(&'a str, &'a str, i64, &'a str),
    Step(&'a str, i64, bool, &'a str),
}

impl<'a> Repeated<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        let generic = Self::Other(source);
        if source.contains(['$', '`', '"', '\\']) {
            return generic;
        }
        let mut parser = Parser {
            source,
            token: Token::End,
        };
        if parser.advance().is_err() {
            return generic;
        }
        let Ok(first) = parser.advance() else {
            return generic;
        };
        let Ok(second) = parser.advance() else {
            return generic;
        };
        match (first, second, parser.token) {
            (Token::Name(name), Token::Op(op @ ("++" | "--")), Token::End)
            | (Token::Op(op @ ("++" | "--")), Token::Name(name), Token::End)
                if crate::parser::is_identifier(name) =>
            {
                Self::Step(
                    name,
                    if op == "++" { 1 } else { -1 },
                    matches!(first, Token::Name(_)),
                    source,
                )
            }
            (
                Token::Name(name),
                Token::Op(op @ ("<" | "<=" | ">" | ">=" | "==" | "!=")),
                Token::Number(value),
            ) if crate::parser::is_identifier(name)
                && parser
                    .source
                    .trim_matches([' ', '\t', '\n', '\r'])
                    .is_empty() =>
            {
                number(value).map_or(generic, |n| Self::Compare(name, op, n, source))
            }
            _ => generic,
        }
    }

    pub(crate) fn run(&self, session: &mut Session, status: u8) -> Result<i64> {
        match *self {
            Self::Other(source) => expanded(source, session, status),
            Self::Compare(name, op, rhs, source) => match session.scalar_decimal(name) {
                Some(n) => binary(op, n, rhs),
                None => expanded(source, session, status),
            },
            Self::Step(name, delta, postfix, source) => {
                let Some(old) = session.scalar_decimal(name) else {
                    return expanded(source, session, status);
                };
                let next = old.wrapping_add(delta);
                Variables::set(session, name, next)?;
                Ok(if postfix { old } else { next })
            }
        }
    }
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

pub(crate) fn traced(
    source: &str,
    session: &mut Session,
    status: u8,
    compiled: Option<&Repeated>,
) -> Result<i64> {
    fn write_trace(text: &str, session: &mut Session, status: u8) {
        crate::execute::trace_line(session, status, |line| {
            line.push_str("(( ");
            line.push_str(text);
            line.push_str(" ))");
        });
    }
    if !source.contains(['$', '`', '"', '\\']) {
        write_trace(source, session, status);
        return match compiled {
            Some(header) => header.run(session, status),
            None => run(source, session),
        };
    }
    let captured = std::cell::Cell::new(None);
    let mut env = crate::expansion::Expansion::new(session, status, &captured);
    let text = env
        .arithmetic_text(source)
        .map_err(|_| "invalid arithmetic expansion")?;
    write_trace(&text, env.session, status);
    with_variables(&text, &mut env)
}

pub fn command(source: &str, session: &mut Session, status: u8) -> Flow {
    let result = if session.xtrace {
        traced(source, session, status, None)
    } else {
        expanded(source, session, status)
    };
    match result {
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
                let mut n = length(source);
                if source[n..].starts_with('[') {
                    n += crate::parser::index_end(&source[n..])
                        .map_err(|_| "missing array subscript")?;
                }
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
                let bytes = source.as_bytes();
                let len = match bytes {
                    [b'<', b'<', b'=', ..] | [b'>', b'>', b'=', ..] => 3,
                    [sign @ (b'+' | b'-'), next, rest @ ..] if sign == next => {
                        if matches!(old, Token::Name(_))
                            || rest
                                .trim_ascii_start()
                                .first()
                                .is_some_and(|&b| b.is_ascii_alphabetic() || b == b'_')
                        {
                            2
                        } else {
                            1
                        }
                    }
                    [first, second, ..] if first == second && b"*&|<>".contains(first) => 2,
                    [first, b'=', ..] if b"+-*/%&^|<>=!".contains(first) => 2,
                    [first, ..] if b"+-*/%()!:~&|^<>=?,".contains(first) => 1,
                    _ => return Err("invalid arithmetic token"),
                };
                let op = &source[..len];
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
        depth: u16,
    ) -> Result<Value<'a>> {
        if depth >= 96 && deep_expression_full(depth) {
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
                    variables.set(&name, result)?;
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
        depth: u16,
    ) -> Result<Value<'a>> {
        if depth >= 96 && deep_expression_full(depth) {
            return Err("arithmetic nesting limit exceeded");
        }
        let mut value = match self.advance()? {
            Token::Number(s) => Value::Number(number(s)?),
            Token::Name(n) => {
                let name =
                    if active && let Some((base, index)) = crate::session::array::subscript(n) {
                        if index.is_empty() {
                            return Err("empty array subscript");
                        }
                        let index = match variables.associative_key(base, index) {
                            Some(key) => key?,
                            None => calculate(index, variables, depth + 1)?.to_string(),
                        };
                        std::borrow::Cow::Owned(format!("{base}[{index}]"))
                    } else {
                        std::borrow::Cow::Borrowed(n)
                    };
                Value::Variable(name)
            }
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
                            variables.set(&name, next)?;
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
            let Value::Variable(ref name) = value else {
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

enum Value<'a> {
    Number(i64),
    Variable(std::borrow::Cow<'a, str>),
}
impl Value<'_> {
    fn read(&self, variables: &mut dyn Variables, active: bool, depth: u16) -> Result<i64> {
        if !active {
            return Ok(0);
        }
        match self {
            Self::Number(n) => Ok(*n),
            Self::Variable(name) => {
                if let Some(n) = variables.decimal(name) {
                    return Ok(n);
                }
                match variables.get(name) {
                    Some(s) => calculate(&s, variables, depth + 1),
                    None if variables.nounset() => Err("unbound arithmetic variable"),
                    None => Ok(0),
                }
            }
        }
    }
}

// Long chains of arithmetic variable references can implement terminating
// recursions. Respect the real stack budget on the process main thread.
#[cold]
fn deep_expression_full(depth: u16) -> bool {
    if depth >= 2048 {
        return true;
    }
    if depth % 16 != 0 {
        return false;
    }
    let Some(origin) = crate::MAIN_STACK_ORIGIN.get() else {
        return true;
    };
    let mut limit = std::mem::MaybeUninit::uninit();
    if unsafe { libc::getrlimit(libc::RLIMIT_STACK, limit.as_mut_ptr()) } != 0 {
        return true;
    }
    let used = origin.abs_diff(std::ptr::from_ref(&limit).addr()) as u64;
    used + 256 * 1024 >= unsafe { limit.assume_init() }.rlim_cur / 2
}

fn calculate(source: &str, variables: &mut dyn Variables, depth: u16) -> Result<i64> {
    if depth >= 96 && deep_expression_full(depth) {
        return Err("arithmetic nesting limit exceeded");
    }
    if let [digit @ b'0'..=b'9'] = source.as_bytes() {
        return Ok(i64::from(digit - b'0'));
    }
    if let Some(name) = source
        .strip_suffix("++")
        .or_else(|| source.strip_suffix("--"))
        && crate::parser::is_identifier(name)
        && let Some(old) = variables.decimal(name)
    {
        let delta = if source.ends_with("++") { 1 } else { -1 };
        variables.set(name, old.wrapping_add(delta))?;
        return Ok(old);
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
        if base.starts_with('0') {
            return Err("invalid arithmetic base");
        }
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
