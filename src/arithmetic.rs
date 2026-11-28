use crate::{Flow, Session};

type Result<T> = std::result::Result<T, &'static str>;
type ShellResult<T> = std::result::Result<T, crate::expansion::Error>;

#[derive(Debug, PartialEq)]
pub(crate) enum Failure {
    Message(&'static str),
    Unbound(String),
    Readonly(String),
}

type Evaluation<T> = std::result::Result<T, Failure>;

impl From<&'static str> for Failure {
    fn from(text: &'static str) -> Self {
        Self::Message(text)
    }
}

impl Failure {
    fn write(name: &str, message: &'static str) -> Self {
        if message == crate::session::READONLY {
            Self::Readonly(name.to_owned())
        } else {
            Self::Message(message)
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Unbound(_) => "unbound arithmetic variable",
            Self::Readonly(_) => crate::session::READONLY,
            Self::Message(text) => text,
        }
    }

    pub(crate) fn error(
        self,
        expression: &str,
        session: &Session,
        expansion: bool,
    ) -> crate::expansion::Error {
        use crate::expansion::Error;
        match self {
            Self::Unbound(name) => Error::fatal(
                format!("{name}: unbound variable"),
                if session.command_string && !session.interactive {
                    127
                } else {
                    1
                },
            ),
            Self::Readonly(name) if expansion => Error::readonly(&name),
            Self::Readonly(name) => Error::from(crate::session::READONLY).in_expression(&name),
            Self::Message(text) if expansion => {
                Error::discard(text).in_expression(expression.trim())
            }
            Self::Message(text) => Error::from(text).in_expression(expression.trim()),
        }
    }
}

fn shell_error(
    error: impl Into<Failure>,
    expression: &str,
    session: &Session,
) -> crate::expansion::Error {
    error.into().error(expression, session, false)
}

pub(crate) trait Variables {
    fn session(&self) -> &Session;
    fn visible_nameref(&self, name: &str) -> bool {
        self.session().is_nameref(name)
    }
    fn get(&self, name: &str) -> Result<Option<String>>;
    fn scalar_text(&self, _name: &str) -> Option<&str> {
        None
    }
    fn decimal(&self, name: &str) -> Option<i64> {
        self.scalar_text(name).and_then(Session::plain_decimal)
    }
    fn set(&mut self, name: &str, value: i64) -> Result<()>;
    fn indexed_decimal(&self, _name: &str, _index: i64) -> Option<i64> {
        None
    }
    fn set_indexed(&mut self, name: &str, index: i64, value: i64) -> Result<()> {
        self.set(&format!("{name}[{index}]"), value)
    }
    fn set_key(&mut self, name: &str, key: String, value: i64) -> Result<()>;
    fn is_associative(&self, name: &str) -> bool;
    fn nounset(&self) -> bool {
        false
    }
}

impl Variables for Session {
    fn session(&self) -> &Session {
        self
    }
    fn scalar_text(&self, name: &str) -> Option<&str> {
        self.stored_scalar(name)
    }
    fn decimal(&self, name: &str) -> Option<i64> {
        self.scalar_decimal(name)
    }
    fn get(&self, name: &str) -> Result<Option<String>> {
        self.arithmetic_get(name)
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
            return Err(crate::session::READONLY);
        }
        self.assign(name, &value.to_string());
        Ok(())
    }
    fn indexed_decimal(&self, name: &str, index: i64) -> Option<i64> {
        self.indexed_scalar_decimal(name, index)
    }
    fn set_indexed(&mut self, name: &str, index: i64, value: i64) -> Result<()> {
        if !self.replace_indexed_decimal(name, index, value) {
            Variables::set(self, &format!("{name}[{index}]"), value)?;
        }
        Ok(())
    }
    fn set_key(&mut self, name: &str, key: String, value: i64) -> Result<()> {
        self.assign_key(name, key, value.to_string())
    }
    fn is_associative(&self, name: &str) -> bool {
        self.is_associative(name)
    }
    fn nounset(&self) -> bool {
        self.nounset
    }
}

mod data;
pub(crate) use data::{data_index, data_key};
use data::{data_index_at, data_key_at};

pub(crate) struct ReadOnly<'a>(pub(crate) &'a Session);
impl Variables for ReadOnly<'_> {
    fn session(&self) -> &Session {
        self.0
    }
    fn nounset(&self) -> bool {
        self.0.nounset
    }
    fn get(&self, name: &str) -> Result<Option<String>> {
        self.0.arithmetic_get(name)
    }
    fn scalar_text(&self, name: &str) -> Option<&str> {
        self.0.stored_scalar(name)
    }
    fn set_key(&mut self, _: &str, _: String, _: i64) -> Result<()> {
        Err("assignment is unavailable in read-only arithmetic")
    }
    fn is_associative(&self, name: &str) -> bool {
        self.0.is_associative(name)
    }
    fn set(&mut self, _: &str, _: i64) -> Result<()> {
        Err("assignment is unavailable in read-only arithmetic")
    }
}

pub(crate) fn with_variables(source: &str, vars: &mut dyn Variables) -> Evaluation<i64> {
    calculate(source, vars, 0)
}

pub(crate) fn numeric_subscript(source: &str) -> &str {
    source
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(source)
}

// Arithmetic names can contain expressions themselves. Only stored decimal
// scalars are leaves; a string such as "x++" must be evaluated in the child.
pub(crate) fn inert(source: &str, session: &Session) -> bool {
    if source.contains(['$', '`', '"', '\\', '[']) {
        return false;
    }
    let mut parser = Parser::new(source);
    loop {
        if parser.advance().is_err() {
            return false;
        }
        match parser.token {
            Token::End => return true,
            Token::Number(_) => {}
            Token::Name(name) if session.scalar_decimal(name).is_some() => {}
            Token::Op(
                "+" | "-" | "*" | "/" | "%" | "**" | "~" | "!" | "(" | ")" | "<<" | ">>" | "<"
                | ">" | "<=" | ">=" | "==" | "!=" | "&" | "|" | "^" | "&&" | "||" | "?" | ":",
            ) => {}
            _ => return false,
        }
    }
}

pub fn run(source: &str, session: &mut Session) -> Result<i64> {
    calculate(source, session, 0).map_err(Failure::message)
}

fn expanded_operand(source: &str, session: &mut Session, status: u8) -> ShellResult<i64> {
    if !source.contains(['$', '`', '"', '\\']) {
        return with_variables(source, session).map_err(|e| shell_error(e, source, session));
    }
    let captured = std::cell::Cell::new(None);
    crate::expansion::Expansion::new(session, status, &captured)
        .arithmetic_text(source)
        .and_then(|text| with_variables(&text, session).map_err(|e| shell_error(e, &text, session)))
}

// Loop headers are immutable, but their operands can be reassigned on every
// iteration. Decode frequent forms once and consult the live shell values.
pub(crate) enum Repeated<'a> {
    Other(&'a str, std::cell::OnceCell<Option<plan::Program>>),
    Compare(&'a str, &'a str, i64, &'a str),
    CompareArrayLength(&'a str, &'a str, &'a str, &'a str),
    CompareNames(&'a str, &'a str, &'a str, &'a str),
    Step(&'a str, i64, bool, &'a str),
}

impl<'a> Repeated<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        let generic = Self::Other(source, Default::default());
        if source.contains(['$', '`', '"', '\\']) {
            let text = source.trim();
            let (name, rest) = text.split_at(crate::word::name_chars_len(text));
            if crate::word::is_identifier(name) {
                let rest = rest.trim_start();
                if let Some(op) = ["<=", ">=", "==", "!=", "<", ">"]
                    .into_iter()
                    .find(|op| rest.starts_with(op))
                {
                    let rhs = rest[op.len()..].trim_start();
                    if let Some(param) = rhs.strip_prefix('$') {
                        let param = param
                            .strip_prefix('{')
                            .and_then(|s| s.strip_suffix('}'))
                            .unwrap_or(param);
                        if crate::word::is_identifier(param) {
                            return Self::CompareNames(name, op, param, source);
                        }
                    }
                    if let Some(operand) = rhs.strip_prefix("${#").and_then(|s| s.strip_suffix('}'))
                        && let Some((base, _, false)) = crate::word::array_elements_name(operand)
                        && crate::word::is_identifier(base)
                    {
                        return Self::CompareArrayLength(name, op, base, source);
                    }
                }
            }
            return generic;
        }
        let mut parser = Parser::new(source);
        if parser.advance().is_err() {
            return generic;
        }
        let Ok(first) = parser.advance() else {
            return generic;
        };
        let Ok(second) = parser.advance() else {
            return generic;
        };
        if !parser
            .source
            .trim_matches([' ', '\t', '\n', '\r'])
            .is_empty()
        {
            return generic;
        }
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
            ) if crate::parser::is_identifier(name) => {
                number(value).map_or(generic, |n| Self::Compare(name, op, n, source))
            }
            (
                Token::Name(name),
                Token::Op(op @ ("<" | "<=" | ">" | ">=" | "==" | "!=")),
                Token::Name(rhs),
            ) if crate::parser::is_identifier(name) && crate::parser::is_identifier(rhs) => {
                Self::CompareNames(name, op, rhs, source)
            }
            (Token::Name(name), Token::Op(op @ ("+=" | "-=")), Token::Number(value))
                if crate::parser::is_identifier(name) =>
            {
                number(value).map_or(generic, |n| {
                    Self::Step(
                        name,
                        if op == "+=" { n } else { n.wrapping_neg() },
                        false,
                        source,
                    )
                })
            }
            _ => generic,
        }
    }

    pub(crate) fn run(&self, session: &mut Session, status: u8) -> ShellResult<i64> {
        match *self {
            Self::Other(source, ref plan) => plan
                .get_or_init(|| plan::Program::compile(source))
                .as_ref()
                .and_then(|p| p.run(session, true))
                .map(|r| r.map_err(|e| shell_error(e, source, session)))
                .unwrap_or_else(|| expanded_operand(source, session, status)),
            Self::Compare(name, op, rhs, source) => match session.scalar_decimal(name) {
                Some(n) => binary(op, n, rhs).map_err(Into::into),
                None => expanded_operand(source, session, status),
            },
            Self::CompareArrayLength(name, op, array, source) => {
                match (
                    session.scalar_decimal(name),
                    session.referenced_array_len(array),
                ) {
                    (Some(n), Some(len)) if !session.nounset || len != 0 => {
                        binary(op, n, len as i64).map_err(Into::into)
                    }
                    _ => expanded_operand(source, session, status),
                }
            }
            Self::CompareNames(name, op, other, source) => {
                match (session.scalar_decimal(name), session.scalar_decimal(other)) {
                    (Some(left), Some(right)) => binary(op, left, right).map_err(Into::into),
                    _ => expanded_operand(source, session, status),
                }
            }
            Self::Step(name, delta, postfix, source) => {
                let Some(old) = session.scalar_decimal(name) else {
                    return expanded_operand(source, session, status);
                };
                let next = old.wrapping_add(delta);
                Variables::set(session, name, next)
                    .map_err(|e| shell_error(Failure::write(name, e), source, session))?;
                Ok(if postfix { old } else { next })
            }
        }
    }
}

pub fn let_builtin(args: &[String], session: &mut Session) -> Flow {
    let args = if args.first().is_some_and(|arg| arg == "--") {
        &args[1..]
    } else {
        args
    };
    if args.is_empty() {
        diagnostic!("let: expression expected");
        return Flow::Continue(1);
    }
    let mut last = 0;
    for expr in args {
        match with_variables(expr, session) {
            Ok(value) => last = value,
            Err(message) => {
                let context = if matches!(message, Failure::Unbound(_) | Failure::Readonly(_)) {
                    ""
                } else {
                    "let: "
                };
                let error = shell_error(message, expr, session);
                error.report(context);
                return error.flow(session);
            }
        }
    }
    Flow::Continue(u8::from(last == 0))
}

pub(crate) fn traced(
    source: &str,
    session: &mut Session,
    status: u8,
    compiled: Option<&Repeated>,
) -> ShellResult<i64> {
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
            None => with_variables(source, session).map_err(|e| shell_error(e, source, session)),
        };
    }
    let captured = std::cell::Cell::new(None);
    let mut env = crate::expansion::Expansion::new(session, status, &captured);
    let text = env.arithmetic_text(source)?;
    write_trace(&text, env.session, status);
    with_variables(&text, &mut env).map_err(|e| shell_error(e, &text, env.session))
}

#[derive(Debug, Clone)]
pub(crate) struct Expression {
    source: String,
    plan: std::cell::OnceCell<Option<plan::Program>>,
    tokens: std::cell::OnceCell<Option<Box<[Lexeme]>>>,
}

impl From<String> for Expression {
    fn from(source: String) -> Self {
        Self {
            source,
            plan: Default::default(),
            tokens: Default::default(),
        }
    }
}

// Offsets refer to this command's immutable source. Variables, array subscripts
// and short-circuit branches are still evaluated by the ordinary parser.
#[derive(Debug, Clone, Copy)]
enum Lexeme {
    Number(u32, u32),
    Name(u32, u32),
    Op(u32, u32),
}

mod plan;

impl std::ops::Deref for Expression {
    type Target = str;
    fn deref(&self) -> &str {
        &self.source
    }
}

impl Expression {
    pub(crate) fn compiled(
        &self,
        vars: &mut dyn Variables,
        permit_store: bool,
    ) -> Option<Evaluation<i64>> {
        self.plan
            .get_or_init(|| plan::Program::compile(&self.source))
            .as_ref()?
            .run(vars, permit_store)
    }

    fn lexemes(&self) -> Option<Box<[Lexeme]>> {
        if self.source.len() > 64 * 1024 || self.source.contains(['$', '`', '"', '\\']) {
            return None;
        }
        let mut scanner = Parser::new(&self.source);
        let mut out = Vec::new();
        loop {
            scanner.advance().ok()?;
            let end = (self.source.len() - scanner.source.len()) as u32;
            out.push(match scanner.token {
                Token::Number(t) => Lexeme::Number(end - t.len() as u32, end),
                Token::Name(t) => Lexeme::Name(end - t.len() as u32, end),
                Token::Op(t) => Lexeme::Op(end - t.len() as u32, end),
                Token::End => return Some(out.into_boxed_slice()),
            });
        }
    }
}

pub(crate) fn command(
    expr: &Expression,
    session: &mut Session,
    status: u8,
    repeated: bool,
) -> Flow {
    let source = expr.source.as_str();
    let result = if session.xtrace {
        traced(source, session, status, None)
    } else if repeated && let Some(result) = expr.compiled(session, true) {
        result.map_err(|e| shell_error(e, source, session))
    } else if repeated
        && !(source.ends_with("++") || source.ends_with("--"))
        && let Some(tokens) = expr.tokens.get_or_init(|| expr.lexemes())
    {
        Parser {
            source,
            tokens: Some(tokens),
            token: Token::End,
        }
        .evaluate(session, 0)
        .map_err(|e| shell_error(e, source, session))
    } else {
        expanded_operand(source, session, status)
    };
    match result {
        Ok(n) => Flow::Continue(u8::from(n == 0)),
        Err(e) => {
            e.report("((: ");
            e.flow(session)
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
    tokens: Option<&'a [Lexeme]>,
    source: &'a str,
    token: Token<'a>,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            tokens: None,
            token: Token::End,
        }
    }
    fn advance(&mut self) -> Result<Token<'a>> {
        let old = self.token;
        if let Some(tokens) = self.tokens {
            self.token = if let Some((&token, rest)) = tokens.split_first() {
                self.tokens = Some(rest);
                match token {
                    Lexeme::Number(a, b) => Token::Number(&self.source[a as usize..b as usize]),
                    Lexeme::Name(a, b) => Token::Name(&self.source[a as usize..b as usize]),
                    Lexeme::Op(a, b) => Token::Op(&self.source[a as usize..b as usize]),
                }
            } else {
                Token::End
            };
            return Ok(old);
        }
        let source = self.source.trim_start_matches([' ', '\t', '\n', '\r']);
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
                let mut n = crate::word::name_chars_len(source);
                if source[n..].starts_with('[') {
                    n += crate::parser::index_end(&source[n..])
                        .map_err(|_| "missing array subscript")?;
                }
                (Token::Name(&source[..n]), n)
            }
            Some(b'$') => return Err("invalid arithmetic operand"),
            _ => {
                let bytes = source.as_bytes();
                let len = match bytes {
                    [b'<', b'<', b'=', ..] | [b'>', b'>', b'=', ..] => 3,
                    [sign @ (b'+' | b'-'), next, rest @ ..] if sign == next => {
                        if matches!(old, Token::Name(_))
                            || rest
                                .trim_ascii_start()
                                .first()
                                .copied()
                                .is_some_and(crate::word::is_name_start)
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
    ) -> Evaluation<Value<'a>> {
        check_depth(depth)?;
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
                    return Err("missing colon in arithmetic conditional".into());
                }
                let no = self
                    .expression(variables, precedence, active && !test, depth + 1)?
                    .read(variables, active && !test, depth + 1)?;
                left = Value::Number(if test { yes } else { no });
                continue;
            }
            if precedence == 2 && matches!(left, Value::Number(_)) {
                return Err("attempted assignment to non-variable".into());
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
                let name = match left {
                    Value::Variable(name) => name,
                    Value::AssignmentTarget(source) => {
                        let (base, index) = source;
                        element_name(base, index, variables, depth)?
                    }
                    Value::Number(_) => {
                        return Err("attempted assignment to non-variable".into());
                    }
                };
                let result = if !active || op == "=" {
                    b
                } else {
                    binary(&op[..op.len() - 1], a, b)?
                };
                if active {
                    name.set(variables, result)?;
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
    ) -> Evaluation<Value<'a>> {
        check_depth(depth)?;
        let mut value = match self.advance()? {
            Token::Number(s) => Value::Number(number(s)?),
            Token::Name(n) => {
                if active && let Some((base, index)) = crate::session::array::subscript(n) {
                    if self.token == Token::Op("=") {
                        Value::AssignmentTarget((base, index))
                    } else {
                        Value::Variable(element_name(base, index, variables, depth)?)
                    }
                } else {
                    Value::Variable(Target::Name(n))
                }
            }
            Token::Op("(") => {
                let v = self.expression(variables, 1, active, depth + 1)?;
                if self.advance()? != Token::Op(")") {
                    return Err("missing arithmetic closing parenthesis".into());
                }
                Value::Number(v.read(variables, active, depth + 1)?)
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
                            return Err("arithmetic increment requires a variable".into());
                        };
                        let next = n.wrapping_add(if op == "++" { 1 } else { -1 });
                        if active {
                            name.set(variables, next)?;
                        }
                        next
                    }
                };
                Value::Number(result)
            }
            _ => return Err("expected an arithmetic operand".into()),
        };
        if let Token::Op(op @ ("++" | "--")) = self.token {
            self.advance()?;
            let Value::Variable(ref name) = value else {
                return Err("arithmetic increment requires a variable".into());
            };
            let old = value.read(variables, active, depth + 1)?;
            if active {
                name.set(variables, old.wrapping_add(if op == "++" { 1 } else { -1 }))?;
            }
            value = Value::Number(old);
        }
        Ok(value)
    }
}

fn element_name<'a>(
    base: &'a str,
    index: &str,
    variables: &mut dyn Variables,
    depth: u16,
) -> Evaluation<Target<'a>> {
    if index.is_empty() {
        return Err("empty array subscript".into());
    }
    if variables.is_associative(base) {
        let key = data_key_at(index, variables, depth + 1)?;
        return Ok(Target::Key(base, key));
    }
    if matches!(index, "@" | "*") {
        diagnostic!("{base}: bad array subscript");
        return Ok(Target::Unavailable);
    }
    Ok(Target::Indexed(
        base,
        data_index_at(numeric_subscript(index), variables, depth + 1)?,
    ))
}

enum Target<'a> {
    Unavailable,
    Name(&'a str),
    Key(&'a str, String),
    Indexed(&'a str, i64),
}

impl Target<'_> {
    fn set(&self, vars: &mut dyn Variables, value: i64) -> Evaluation<()> {
        match self {
            Self::Unavailable => Ok(()),
            Self::Name(name) => vars.set(name, value).map_err(|e| Failure::write(name, e)),
            Self::Key(name, key) => vars
                .set_key(name, key.clone(), value)
                .map_err(|e| Failure::write(name, e)),
            Self::Indexed(name, index) => vars
                .set_indexed(name, *index, value)
                .map_err(|e| Failure::write(name, e)),
        }
    }
}

enum Value<'a> {
    AssignmentTarget((&'a str, &'a str)),
    Number(i64),
    Variable(Target<'a>),
}
impl Value<'_> {
    fn read(&self, variables: &mut dyn Variables, active: bool, depth: u16) -> Evaluation<i64> {
        if !active {
            return Ok(0);
        }
        match self {
            Self::AssignmentTarget(_) => Err("expected arithmetic assignment".into()),
            Self::Number(n) => Ok(*n),
            Self::Variable(target) => {
                let text = match target {
                    Target::Unavailable => return Ok(0),
                    Target::Name(name) => {
                        if let Some(raw) = variables.scalar_text(name) {
                            if let Some(n) = Session::plain_decimal(raw) {
                                return Ok(n);
                            }
                            // Recursive arithmetic evaluation can mutate this variable.
                            Some(raw.to_owned())
                        } else {
                            data::read_name(name, variables, depth + 1)?
                        }
                    }
                    Target::Key(name, key) => {
                        let text = variables.session().assoc_value(name, key);
                        if let Some(n) = text.and_then(Session::plain_decimal) {
                            return Ok(n);
                        }
                        text.map(str::to_owned)
                    }
                    Target::Indexed(name, index) => {
                        if let Some(n) = variables.indexed_decimal(name, *index) {
                            return Ok(n);
                        }
                        variables.get(&format!("{name}[{index}]"))?
                    }
                };
                match text {
                    Some(s) if s.is_empty() => Ok(0),
                    Some(s) => calculate(&s, variables, depth + 1),
                    None if variables.nounset() => Err(Failure::Unbound(match target {
                        Target::Indexed(name, index) => format!("{name}[{index}]"),
                        Target::Name(name) => name.to_string(),
                        Target::Key(name, key) => format!("{name}[{key}]"),
                        Target::Unavailable => unreachable!(),
                    })),
                    None => Ok(0),
                }
            }
        }
    }
}

fn check_depth(depth: u16) -> Result<()> {
    const FIRST_STACK_CHECK: u16 = 96;
    if depth >= FIRST_STACK_CHECK && deep_expression_full(depth) {
        Err("arithmetic nesting limit exceeded")
    } else {
        Ok(())
    }
}

// Long chains of arithmetic variable references can implement terminating
// recursions. Respect the real stack budget on the process main thread.
#[cold]
fn deep_expression_full(depth: u16) -> bool {
    if depth >= 2048 {
        return true;
    }
    if !depth.is_multiple_of(16) {
        return false;
    }
    crate::stack_budget_exceeded()
}

fn calculate(source: &str, variables: &mut dyn Variables, depth: u16) -> Evaluation<i64> {
    check_depth(depth)?;
    if let [digit @ b'0'..=b'9'] = source.as_bytes() {
        return Ok(i64::from(digit - b'0'));
    }
    if crate::word::is_identifier(source)
        && let Some(n) = variables.decimal(source)
    {
        return Ok(n);
    }
    if let Some(name) = source
        .strip_suffix("++")
        .or_else(|| source.strip_suffix("--"))
        && crate::parser::is_identifier(name)
        && let Some(old) = variables.decimal(name)
    {
        let delta = if source.ends_with("++") { 1 } else { -1 };
        variables
            .set(name, old.wrapping_add(delta))
            .map_err(|e| Failure::write(name, e))?;
        return Ok(old);
    }
    Parser::new(source).evaluate(variables, depth)
}

impl Parser<'_> {
    fn evaluate(&mut self, variables: &mut dyn Variables, depth: u16) -> Evaluation<i64> {
        self.advance()?;
        if self.token == Token::End {
            return Ok(0);
        }
        let value =
            self.expression(variables, 1, true, depth + 1)?
                .read(variables, true, depth + 1)?;
        if self.token != Token::End {
            return Err("arithmetic syntax error in expression".into());
        }
        Ok(value)
    }
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

#[derive(Debug, Clone, Copy)]
enum Binary {
    Comma,
    LogicalOr,
    LogicalAnd,
    Or,
    And,
    Xor,
    Equal,
    Unequal,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    ShiftLeft,
    ShiftRight,
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Power,
}

impl Binary {
    fn parse(op: &str) -> Option<Self> {
        Some(match op {
            "," => Self::Comma,
            "||" => Self::LogicalOr,
            "&&" => Self::LogicalAnd,
            "|" => Self::Or,
            "&" => Self::And,
            "^" => Self::Xor,
            "==" => Self::Equal,
            "!=" => Self::Unequal,
            "<" => Self::Less,
            ">" => Self::Greater,
            "<=" => Self::LessEqual,
            ">=" => Self::GreaterEqual,
            "<<" => Self::ShiftLeft,
            ">>" => Self::ShiftRight,
            "+" => Self::Add,
            "-" => Self::Subtract,
            "*" => Self::Multiply,
            "/" => Self::Divide,
            "%" => Self::Remainder,
            "**" => Self::Power,
            _ => return None,
        })
    }

    fn apply(self, a: i64, b: i64) -> Result<i64> {
        Ok(match self {
            Self::Comma => b,
            Self::LogicalOr => i64::from(a != 0 || b != 0),
            Self::LogicalAnd => i64::from(a != 0 && b != 0),
            Self::Or => a | b,
            Self::And => a & b,
            Self::Xor => a ^ b,
            Self::Equal => i64::from(a == b),
            Self::Unequal => i64::from(a != b),
            Self::Less => i64::from(a < b),
            Self::Greater => i64::from(a > b),
            Self::LessEqual => i64::from(a <= b),
            Self::GreaterEqual => i64::from(a >= b),
            Self::ShiftLeft => a.wrapping_shl(b as u32),
            Self::ShiftRight => a.wrapping_shr(b as u32),
            Self::Add => a.wrapping_add(b),
            Self::Subtract => a.wrapping_sub(b),
            Self::Multiply => a.wrapping_mul(b),
            Self::Divide | Self::Remainder if b == 0 => return Err("division by 0"),
            Self::Divide => a.wrapping_div(b),
            Self::Remainder => a.wrapping_rem(b),
            Self::Power if b < 0 => return Err("exponent less than 0"),
            Self::Power => {
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
        })
    }
}

fn binary(op: &str, a: i64, b: i64) -> Result<i64> {
    Binary::parse(op)
        .ok_or("unknown arithmetic operator")?
        .apply(a, b)
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
        if digits.is_empty() {
            return Ok(0);
        }
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
            _ => return Err("value too great for base"),
        };
        if d >= base {
            return Err("value too great for base");
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
    fn read_only_subscript_observes_nounset() {
        let mut session = Session::default();
        assert_eq!(data_index("$missing", &mut ReadOnly(&session)), Ok(0));
        session.nounset = true;
        assert_eq!(
            data_index("$missing", &mut ReadOnly(&session)),
            Err("unbound arithmetic variable")
        );
        assert_eq!(data_index("${missing:-7}", &mut ReadOnly(&session)), Ok(7));
    }

    #[test]
    fn precedence_numbers_and_lazy_branches() {
        let mut session = Session::default();
        session.assign("n", "3+1");
        session.assign("x", "x");
        let mut vars = ReadOnly(&session);
        for (s, v) in [
            ("2+3*4", 14),
            ("2**3**2", 512),
            ("16#FF + 010 + 0x10", 279),
            ("n==4 ? 2*n : 1/0", 8),
            ("0 && (1/0)", 0),
            ("1 || (1/0)", 1),
            ("-1 >> 2", -1),
        ] {
            assert_eq!(data_index(s, &mut vars).unwrap(), v, "{s}");
        }
        assert!(data_index("08", &mut vars).is_err());
        assert!(data_index("x", &mut vars).is_err());
    }
}
