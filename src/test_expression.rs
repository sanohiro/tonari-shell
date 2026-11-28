use crate::{Flow, Session, arithmetic, builtins, pattern, word::Word};
use std::ffi::CString;

#[derive(Clone, Debug)]
pub enum Expr {
    Word(Word),
    Unary(&'static str, Word),
    Binary(Word, &'static str, Word),
    StaticGlob(Word, &'static str, std::rc::Rc<StaticGlob>),
    StaticRegex(Word, String, Option<String>),
    Not(Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
}

#[derive(Debug)]
pub struct StaticGlob {
    word: Word,
    text: std::cell::OnceCell<String>,
    compiled: std::cell::RefCell<Option<Box<CompiledGlob>>>,
}

#[derive(Debug)]
struct CompiledGlob {
    locale: Option<std::rc::Rc<pattern::PatternLocale>>,
    ascii_ranges: bool,
    matcher: pattern::Matcher<String>,
    group: Option<pattern::extended::Pattern>,
}

impl StaticGlob {
    fn matches(&self, left: &str, source: &str, session: &Session) -> bool {
        if session.shopt.nocasematch {
            pattern::extended::insensitive_session(left, source, true, session)
        } else {
            let locale = source
                .contains(['[', '?'])
                .then(|| pattern::session_locale(session))
                .flatten();
            let _guard = locale.as_ref().and_then(|l| l.enter());
            let mut entry = self.compiled.borrow_mut();
            // Bracket ranges and their per-character results belong to the
            // active locale. Keep its handle alive until recompilation.
            let ascii_ranges = session.shopt.globasciiranges;
            let current = entry.as_ref().is_some_and(|cached| {
                cached.ascii_ranges == ascii_ranges
                    && cached.locale.as_ref().map(std::rc::Rc::as_ptr)
                        == locale.as_ref().map(std::rc::Rc::as_ptr)
            });
            if !current {
                *entry = Some(Box::new(CompiledGlob {
                    matcher: pattern::Matcher::new(source.to_owned()),
                    group: pattern::extended::Pattern::new(source),
                    locale,
                    ascii_ranges,
                }));
            }
            let compiled = entry.as_ref().unwrap();
            compiled.group.as_ref().map_or_else(
                || compiled.matcher.matches(left),
                |group| group.accepts(left, false),
            )
        }
    }
}

impl Expr {
    pub(crate) fn binary(left: Word, operator: &'static str, right: Word) -> Self {
        use crate::word::Part;
        let fixed = match &right {
            Word::Literal(_) => true,
            Word::Expanded(parts) => parts
                .iter()
                .all(|p| matches!(p, Part::Pattern(_) | Part::Unquoted(_) | Part::Text(_))),
            Word::Declaration(_) => false,
        };
        if matches!(operator, "=" | "==" | "!=") && fixed && right.as_literal().is_none() {
            return Self::StaticGlob(
                left,
                operator,
                std::rc::Rc::new(StaticGlob {
                    word: right,
                    text: Default::default(),
                    compiled: Default::default(),
                }),
            );
        }
        if operator == "=~" && fixed {
            let quoted = right.clone().regex(&|_| None, &[]);
            let plain = right.scalar(&|_| None, &[]);
            let plain = (plain != quoted).then_some(plain);
            return Self::StaticRegex(left, quoted, plain);
        }
        Self::Binary(left, operator, right)
    }
}

pub fn evaluate(expr: &Expr, session: &mut Session, status: u8) -> Flow {
    match condition(expr, session, status) {
        Ok(value) => Flow::Continue(u8::from(!value)),
        Err(flow) => flow,
    }
}

fn trace(session: &mut Session, status: u8, words: &[&str]) {
    if session.xtrace {
        crate::execute::trace_line(session, status, |line| {
            line.push_str("[[");
            for (index, word) in words.iter().enumerate() {
                line.push(' ');
                if index == 1 && words.len() == 3 {
                    line.push_str(word);
                } else {
                    line.push_str(&crate::execute::trace_word(word));
                }
            }
            line.push_str(" ]]");
        });
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
        if let Some(text) = word.stored_parameter(s) {
            return Ok(std::borrow::Cow::Owned(text.to_owned()));
        }
        crate::eval::expand_case(s, word.clone(), status, false).map(std::borrow::Cow::Owned)
    }
    let error = |message: &'static str| {
        diagnostic!("[[: {message}");
        Flow::Continue(2)
    };
    if !session.xtrace {
        match expr {
            Word(w) | Unary("-n" | "-z", w) if let Some(v) = w.stored_parameter(session) => {
                return Ok(v.is_empty() == matches!(expr, Unary("-z", _)));
            }
            Binary(w, op @ ("<" | ">" | "=" | "==" | "!="), right)
                if (!session.shopt.nocasematch || matches!(*op, "<" | ">"))
                    && let Some(right) = right.as_literal()
                    && let Some(v) = w.stored_parameter(session) =>
            {
                return builtins::binary(v, op, right).map_err(error);
            }
            _ => {}
        }
    }
    match expr {
        Not(e) => match condition(e, session, status) {
            Ok(value) => Ok(!value),
            Err(Flow::Continue(code)) => Ok(code != 0),
            other => other,
        },
        And(terms) => {
            for term in terms {
                if !condition(term, session, status)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Or(terms) => {
            let (last, prefix) = terms.split_last().expect("empty conditional disjunction");
            for term in prefix {
                match condition(term, session, status) {
                    Ok(false) | Err(Flow::Continue(1..)) => {}
                    other => return other,
                }
            }
            condition(last, session, status)
        }
        Word(word) => {
            let value = operand(session, word, status)?;
            trace(session, status, &["-n", &value]);
            Ok(!value.is_empty())
        }
        Unary(op, word) => {
            let op = *op;
            let value = operand(session, word, status)?;
            trace(session, status, &[op, &value]);
            if op == "-v" {
                return builtins::variable_set(session, &value, word.as_literal().is_none())
                    .map_err(error);
            }
            builtins::unary(op, &value, session).map_err(error)
        }
        StaticGlob(left, op, glob) => {
            let source = glob
                .text
                .get_or_init(|| glob.word.clone().extended_pattern(&|_| None, &[]));
            if !session.xtrace
                && let Some(value) = left.stored_parameter(session)
            {
                return Ok(glob.matches(value, source, session) != (*op == "!="));
            }
            let left = operand(session, left, status)?;
            trace(session, status, &[&left, op, source]);
            let matched = glob.matches(&left, source, session);
            Ok(matched != (*op == "!="))
        }
        StaticRegex(left, quoted, plain) => {
            let pattern = if session.shopt.compat <= 31 {
                plain.as_ref().unwrap_or(quoted)
            } else {
                quoted
            };
            if !session.xtrace && left.stored_parameter(session).is_some() {
                return regex_search(session, pattern.as_str().into(), |compiled, session| {
                    // Compiling the expression updates only the regex cache.
                    compiled.search(left.stored_parameter(session).unwrap())
                });
            }
            let left = operand(session, left, status)?;
            regex_match(session, status, &left, pattern.as_str().into())
        }
        Binary(left, op, right) => {
            let op = *op;
            let left = operand(session, left, status)?;
            match op {
                "=" | "==" | "!=" => {
                    if !session.shopt.nocasematch
                        && let Some(text) = right.as_literal()
                    {
                        trace(session, status, &[&left, op, text]);
                        return Ok((left == text) != (op == "!="));
                    }
                    let captured = std::cell::Cell::new(None);
                    let mut env = crate::expansion::Expansion::new(session, status, &captured);
                    let mut right = right.clone();
                    right.resolve(&mut env).map_err(|e| {
                        e.report("[[: ");
                        e.flow(env.session)
                    })?;
                    let right = right.extended_pattern(
                        &|name| env.lookup(name),
                        env.session.positional.as_slice(),
                    );
                    trace(env.session, status, &[&left, op, &right]);
                    let locale = right
                        .contains(['[', '?'])
                        .then(|| pattern::session_locale(env.session))
                        .flatten();
                    let _guard = locale.as_ref().and_then(|l| l.enter());
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
                    let expression = if session.shopt.compat <= 31 {
                        operand(session, right, status)?
                    } else {
                        crate::expansion::Expansion::new(session, status, &captured)
                            .regex(right.clone())
                            .map_err(|e| {
                                e.report("[[: ");
                                e.flow(session)
                            })?
                            .into()
                    };
                    regex_match(session, status, &left, expression)
                }
                "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge" => {
                    let right = operand(session, right, status)?;
                    trace(session, status, &[&left, op, &right]);
                    let arith_error = |message| {
                        diagnostic!("[[: {message}");
                        Flow::Continue(1)
                    };
                    let a = arithmetic::run(&left, session).map_err(arith_error)?;
                    let b = arithmetic::run(&right, session).map_err(arith_error)?;
                    Ok(match op {
                        "-eq" => a == b,
                        "-ne" => a != b,
                        "-lt" => a < b,
                        "-le" => a <= b,
                        "-gt" => a > b,
                        _ => a >= b,
                    })
                }
                _ => {
                    let right = operand(session, right, status)?;
                    trace(session, status, &[&left, op, &right]);
                    builtins::binary(&left, op, &right).map_err(error)
                }
            }
        }
    }
}

fn regex_match(
    session: &mut Session,
    status: u8,
    left: &str,
    expression: std::borrow::Cow<'_, str>,
) -> Result<bool, Flow> {
    trace(session, status, &[left, "=~", &expression]);
    regex_search(session, expression, |compiled, _| compiled.search(left))
}

fn regex_search(
    session: &mut Session,
    expression: std::borrow::Cow<'_, str>,
    search: impl FnOnce(&Compiled, &Session) -> Result<Option<Vec<String>>, ()>,
) -> Result<bool, Flow> {
    let locale = pattern::session_locale(session);
    let _guard = locale.as_ref().and_then(|l| l.enter());
    let ignore_case = session.shopt.nocasematch;
    let bank = usize::from(ignore_case);
    let recent = &mut session.regex[bank];
    let index = if let Some(index) = recent.iter().rposition(|r| {
        r.pattern == expression
            && r.locale.as_ref().map(std::rc::Rc::as_ptr)
                == locale.as_ref().map(std::rc::Rc::as_ptr)
    }) {
        index
    } else {
        let compiled = Compiled::new(expression.into_owned(), ignore_case, locale)
            .map_err(|()| Flow::Continue(2))?;
        const CAPACITY: usize = 64;
        if recent.len() == CAPACITY {
            recent.remove(0);
        }
        recent.push(std::rc::Rc::new(compiled));
        recent.len() - 1
    };
    let result = search(&session.regex[bank][index], session).map_err(|()| Flow::Continue(2))?;
    let matched = result.is_some();
    session.replace_array("BASH_REMATCH", result.unwrap_or_default());
    Ok(matched)
}

// Return the end of a POSIX ERE list, measured from just after its opening '['.
fn ere_list_end(body: &[u8]) -> Option<usize> {
    let mut i = usize::from(body.first() == Some(&b'^'));
    i += usize::from(body.get(i) == Some(&b']'));
    while let Some(&byte) = body.get(i) {
        if byte == b']' {
            return Some(i);
        }
        if byte == b'['
            && let Some((length, _)) = list_item(&body[i..])
        {
            i += length;
        } else {
            i += 1;
        }
    }
    None
}

fn list_item(input: &[u8]) -> Option<(usize, u8)> {
    let [b'[', mark @ (b'=' | b'.' | b':'), tail @ ..] = input else {
        return None;
    };
    let end = tail.windows(2).position(|w| w == [*mark, b']'])?;
    Some((end + 4, *mark))
}

fn open_list(pattern: &str) -> Option<usize> {
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'[' => match ere_list_end(&bytes[i + 1..]) {
                Some(n) => i += n + 2,
                None => return Some(i),
            },
            _ => i += 1,
        }
    }
    None
}

pub(crate) fn quote_regex(buffer: &mut String, mut text: &str) {
    if let Some(start) = open_list(buffer) {
        let before = buffer.len();
        buffer.push_str(text);
        let Some(close) = ere_list_end(&buffer.as_bytes()[start + 1..]) else {
            return;
        };
        let end = start + close + 2;
        buffer.truncate(end);
        text = &text[end - before..];
    }
    for c in text.chars() {
        if "\\.[]{}()*+?^$|".contains(c) {
            buffer.push('\\');
        }
        buffer.push(c);
    }
}

// Some POSIX regex implementations reject even the single-character
// collating elements in the portable character set. Preserve native
// equivalence classes whenever the host can compile the original expression.
fn portable_ere(pattern: &str) -> Option<String> {
    let bytes = pattern.as_bytes();
    let mut out = String::new();
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] != b'[' {
            i += 1;
            continue;
        }
        let end = i + 1 + ere_list_end(&bytes[i + 1..])?;
        let body = &pattern[i + 1..end];
        if body.contains("[.") || body.contains("[=") {
            let list = portable_list(body)?;
            out.push_str(&pattern[copied..i]);
            out.push_str(&list);
            copied = end + 1;
        }
        i = end + 1;
    }
    if copied == 0 {
        return None;
    }
    out.push_str(&pattern[copied..]);
    Some(out)
}

fn portable_list(body: &str) -> Option<String> {
    #[derive(Clone, Copy)]
    enum Member<'a> {
        Scalar(char),
        Dash,
        Class(&'a str),
    }
    use Member::*;
    let (negative, mut rest) = body.strip_prefix('^').map_or((false, body), |s| (true, s));
    if !negative
        && let Some((n, mark)) = list_item(rest.as_bytes())
        && n == rest.len()
        && matches!(mark, b'.' | b'=')
    {
        let name = &rest[2..n - 2];
        let mut chars = name.chars();
        let c = match (chars.next(), chars.next()) {
            (Some(c), None) => c,
            _ if mark == b'.' => char::from(crate::pattern::portable_symbol(name.as_bytes())?),
            _ => return None,
        };
        if c == '\0' {
            return None;
        }
        let mut quoted = String::new();
        if "\\.[]{}()*+?^$|".contains(c) {
            quoted.push('\\');
        }
        quoted.push(c);
        return Some(quoted);
    }
    let mut members = Vec::new();
    while !rest.is_empty() {
        if let Some((n, mark)) = list_item(rest.as_bytes()) {
            let name = &rest[2..n - 2];
            let item = if mark == b':' {
                Class(&rest[..n])
            } else {
                let c = if name.chars().count() == 1 {
                    name.chars().next()?
                } else if mark == b'.' {
                    char::from(crate::pattern::portable_symbol(name.as_bytes())?)
                } else {
                    return None;
                };
                if c == '\0' {
                    return None;
                }
                Scalar(c)
            };
            members.push(item);
            rest = &rest[n..];
        } else {
            let c = rest.chars().next()?;
            members.push(if c == '-' { Dash } else { Scalar(c) });
            rest = &rest[c.len_utf8()..];
        }
    }
    let mut ordinary = String::new();
    let mut punctuation = [false; 4];
    fn literal(c: char, ordinary: &mut String, punctuation: &mut [bool; 4]) {
        match c {
            ']' => punctuation[0] = true,
            '^' => punctuation[1] = true,
            '[' => punctuation[2] = true,
            '-' => punctuation[3] = true,
            c => ordinary.push(c),
        }
    }
    let mut input = members.as_slice();
    while let [first, tail @ ..] = input {
        if let (Scalar(a), [Dash, Scalar(b), tail @ ..]) = (*first, tail) {
            if a > *b {
                return None;
            }
            if a.is_ascii() && b.is_ascii() {
                for byte in (a as u8)..=(*b as u8) {
                    literal(char::from(byte), &mut ordinary, &mut punctuation);
                }
            } else {
                if matches!(a, ']' | '[' | '^' | '-') || matches!(b, ']' | '[' | '^' | '-') {
                    return None;
                }
                ordinary.push(a);
                ordinary.push('-');
                ordinary.push(*b);
            }
            input = tail;
        } else {
            match *first {
                Scalar(c) => literal(c, &mut ordinary, &mut punctuation),
                Dash if tail.is_empty() || input.len() == members.len() => punctuation[3] = true,
                Dash => return None,
                Class(s) => ordinary.push_str(s),
            }
            input = tail;
        }
    }
    let [closing, caret, opening, dash] = punctuation;
    if !negative && caret && !closing && ordinary.is_empty() && !opening && !dash {
        return Some(r"\^".into());
    }
    let mut result = String::from(if negative { "[^" } else { "[" });
    if closing {
        result.push(']');
    }
    result.push_str(&ordinary);
    if opening {
        result.push('[');
    }
    let dash_first = !negative && caret && !opening && !closing && ordinary.is_empty();
    if dash && dash_first {
        result.push('-');
    }
    if caret {
        result.push('^');
    }
    if dash && !dash_first {
        result.push('-');
    }
    result.push(']');
    Some(result)
}

pub struct Compiled {
    locale: Option<std::rc::Rc<pattern::PatternLocale>>,
    pattern: String,
    regex: Option<Box<libc::regex_t>>,
    captures: usize,
}

impl Compiled {
    fn new(
        pattern: String,
        ignore_case: bool,
        locale: Option<std::rc::Rc<pattern::PatternLocale>>,
    ) -> Result<Self, ()> {
        let encoded = CString::new(pattern.as_str()).map_err(|_| ())?;
        let mut regex = Box::<libc::regex_t>::new_uninit();
        let flags = if ignore_case {
            libc::REG_EXTENDED | libc::REG_ICASE
        } else {
            libc::REG_EXTENDED
        };
        let compile = |compiled: *mut libc::regex_t, bytes: &CString| unsafe {
            libc::regcomp(compiled, bytes.as_ptr(), flags) == 0
        };
        if !compile(regex.as_mut_ptr(), &encoded)
            && !portable_ere(&pattern)
                .and_then(|p| CString::new(p).ok())
                .is_some_and(|p| compile(regex.as_mut_ptr(), &p))
        {
            return Ok(Self {
                pattern,
                locale,
                regex: None,
                captures: 0,
            });
        }
        let captures = capture_count(&pattern) + 1;
        Ok(Self {
            pattern,
            locale,
            regex: Some(unsafe { regex.assume_init() }),
            captures,
        })
    }

    fn search(&self, text: &str) -> Result<Option<Vec<String>>, ()> {
        let Some(regex) = self.regex.as_deref() else {
            diagnostic!("[[: invalid regular expression: {}", self.pattern);
            return Err(());
        };
        let mut small_text = [0u8; 256];
        let allocated;
        let encoded = if text.len() < small_text.len() {
            small_text[..text.len()].copy_from_slice(text.as_bytes());
            std::ffi::CStr::from_bytes_with_nul(&small_text[..=text.len()]).map_err(|_| ())?
        } else {
            allocated = CString::new(text).map_err(|_| ())?;
            allocated.as_c_str()
        };
        let empty = libc::regmatch_t {
            rm_so: -1,
            rm_eo: -1,
        };
        let mut small_matches = [empty; 16];
        let mut large_matches;
        let matched = if self.captures <= small_matches.len() {
            &mut small_matches[..self.captures]
        } else {
            large_matches = vec![empty; self.captures];
            &mut large_matches[..]
        };
        match unsafe {
            libc::regexec(
                regex,
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
        if let Some(regex) = self.regex.as_deref_mut() {
            unsafe { libc::regfree(regex) };
        }
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
                    if c == b'['
                        && let Some(&marker @ (b':' | b'=' | b'.')) = after.first()
                        && let Some(end) = after[1..].windows(2).position(|s| s == [marker, b']'])
                    {
                        bytes = &after[end + 3..];
                    }
                }
            }
            _ => {}
        }
    }
    groups
}
