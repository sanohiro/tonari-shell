use super::{CaseEnd, Error, Reader, Target, ansi, here_doc, parse};
use crate::word::{Part, Word, WordBuilder, is_identifier};
use std::str::Chars;

#[derive(Debug, Clone)]
pub(super) struct LexWord {
    pub(super) word: Word,
    pub(super) protected: bool,
    pub(super) assignment: Option<String>,
}

#[derive(Clone)]
pub(super) enum Token {
    Word(LexWord),
    ArrayStart,
    LeftParen,
    RightParen,
    Arithmetic(String),
    Redirect(i32, Target),
    BothRedirect(Target),
    Background,
    PipeBoth,
    Pipe,
    And,
    Or,
    End,
    EndArm(CaseEnd),
    Newline,
}

pub(super) fn descriptor(s: &str) -> Result<i32, Error> {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        return s
            .parse()
            .map_err(|_| Error::Invalid("invalid file descriptor"));
    }
    Err(Error::Invalid("invalid file descriptor"))
}

fn redirect(chars: &mut Chars<'_>, op: char, fd: i32) -> Result<Token, Error> {
    let target = match (op, chars.clone().next()) {
        ('<', Some('<')) => return Err(Error::Invalid("unsupported redirection")),
        ('<', Some('>')) => {
            chars.next();
            Target::ReadWrite
        }
        ('>', Some('|')) => {
            chars.next();
            Target::Clobber
        }
        (_, Some('&')) => {
            chars.next();
            if op == '>' && fd == 1 {
                Target::OutputWord
            } else {
                Target::Duplicate
            }
        }
        ('>', Some('>')) => {
            chars.next();
            Target::Append
        }
        ('>', _) => Target::Write,
        _ => Target::Read,
    };
    Ok(Token::Redirect(fd, target))
}

pub(super) fn dollar(
    chars: &mut Chars<'_>,
    word: &mut WordBuilder,
    quoted: bool,
) -> Result<(), Error> {
    let rest = chars.as_str();
    let name = match chars.clone().next() {
        Some('{') => {
            chars.next();
            let content = chars.as_str();
            let len = match content.as_bytes().first() {
                Some(b'a'..=b'z' | b'A'..=b'Z' | b'_') => content
                    .bytes()
                    .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                    .count(),
                Some(b'0'..=b'9') => content.bytes().take_while(|b| b.is_ascii_digit()).count(),
                Some(b'@' | b'*' | b'?' | b'$' | b'#' | b'!' | b'-') => 1,
                None => return Err(Error::Incomplete("unterminated parameter expansion")),
                _ => return Err(Error::Invalid("unsupported parameter expansion")),
            };
            let prefixed = content.starts_with(['#', '!'])
                && content
                    .as_bytes()
                    .get(1)
                    .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_');
            let len = if content.starts_with(['#', '!'])
                && content.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
            {
                1 + content[1..].bytes().take_while(u8::is_ascii_digit).count()
            } else if content.starts_with(['#', '!'])
                && content.as_bytes().get(2) == Some(&b'}')
                && matches!(
                    content.as_bytes().get(1),
                    Some(b'@' | b'*' | b'-' | b'#' | b'?' | b'$' | b'!')
                )
            {
                2
            } else if prefixed {
                1 + content[1..]
                    .bytes()
                    .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                    .count()
            } else {
                len
            };
            let len = if content[len..].starts_with('[') {
                len + super::modifier::index_end(&content[len..])?
            } else {
                len
            };
            let name = &content[..len];
            *chars = content[len..].chars();
            if chars.as_str().starts_with('}') {
                chars.next();
                name
            } else {
                if chars.as_str().starts_with('/') {
                    chars.next();
                    let mode = match chars.clone().next() {
                        Some('/') => crate::word::ReplaceMode::All,
                        Some('#') => crate::word::ReplaceMode::Prefix,
                        Some('%') => crate::word::ReplaceMode::Suffix,
                        _ => crate::word::ReplaceMode::First,
                    };
                    if mode != crate::word::ReplaceMode::First {
                        chars.next();
                    }
                    let (pattern, separator) = parameter_word(chars, false, true)?;
                    let replacement = if separator == '/' {
                        alternative_word(chars, false)?
                    } else {
                        Word::Literal(String::new())
                    };
                    word.part(Part::Modify {
                        name: name.into(),
                        operation: Box::new(crate::word::Modification::Replace {
                            pattern,
                            replacement,
                            mode,
                        }),
                        quoted,
                    });
                    return Ok(());
                }
                if let [b'@', op @ (b'U' | b'u' | b'L' | b'Q' | b'E'), b'}', ..] =
                    chars.as_str().as_bytes()
                {
                    let op = *op;
                    *chars = chars.as_str()[3..].chars();
                    let operation = match op {
                        b'Q' => crate::word::Modification::Quote,
                        b'E' => crate::word::Modification::Escapes,
                        _ => crate::word::Modification::Case {
                            upper: op != b'L',
                            all: op != b'u',
                            pattern: crate::word::Word::Literal(String::new()),
                        },
                    };
                    word.part(Part::Modify {
                        name: name.into(),
                        operation: Box::new(operation),
                        quoted,
                    });
                    return Ok(());
                }
                if chars.as_str().starts_with("@P}") {
                    *chars = chars.as_str()[3..].chars();
                    word.part(Part::Modify {
                        name: name.into(),
                        operation: Box::new(crate::word::Modification::Prompt),
                        quoted,
                    });
                    return Ok(());
                }
                if chars.as_str().starts_with(':')
                    && !chars.as_str()[1..].starts_with(['-', '+', '=', '?'])
                {
                    chars.next();
                    let (offset, length) = super::modifier::slice(chars)?;
                    word.part(Part::Modify {
                        name: name.into(),
                        operation: Box::new(crate::word::Modification::Slice { offset, length }),
                        quoted,
                    });
                    return Ok(());
                }
                if let Some(op @ ('^' | ',')) = chars.clone().next() {
                    chars.next();
                    let all = chars.as_str().starts_with(op);
                    if all {
                        chars.next();
                    }
                    let pattern = alternative_word(chars, false)?;
                    word.part(Part::Modify {
                        name: name.into(),
                        operation: Box::new(crate::word::Modification::Case {
                            upper: op == '^',
                            all,
                            pattern,
                        }),
                        quoted,
                    });
                    return Ok(());
                }
                if let Some(op @ ('%' | '#')) = chars.clone().next() {
                    chars.next();
                    let longest = chars.as_str().starts_with(op);
                    if longest {
                        chars.next();
                    }
                    let pattern = alternative_word(chars, false)?;
                    word.part(Part::Trim {
                        name: name.into(),
                        pattern: Box::new(pattern),
                        prefix: op == '#',
                        longest,
                        quoted,
                    });
                    return Ok(());
                }
                let check_empty = chars.as_str().starts_with(':');
                if check_empty {
                    chars.next();
                }
                let op = match chars.next() {
                    Some('+') => crate::word::Alternative::Alternate,
                    Some('-') => crate::word::Alternative::Default,
                    Some('=') => crate::word::Alternative::Assign,
                    Some('?') => crate::word::Alternative::Error,
                    None => return Err(Error::Incomplete("unterminated parameter expansion")),
                    _ => return Err(Error::Invalid("unsupported parameter expansion operator")),
                };
                let alternative = alternative_word(chars, quoted)?;
                word.part(Part::Alternative {
                    name: name.into(),
                    check_empty,
                    op,
                    word: Box::new(alternative),
                    quoted,
                });
                return Ok(());
            }
        }
        Some('a'..='z' | 'A'..='Z' | '_') => {
            let end = rest
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .count();
            *chars = rest[end..].chars();
            &rest[..end]
        }
        Some('0'..='9' | '@' | '*' | '?' | '$' | '#' | '!' | '-') => {
            chars.next();
            &rest[..1]
        }
        Some('(') => {
            if let Some(after) = rest.strip_prefix("((") {
                *chars = after.chars();
                let expression = arithmetic_source(chars)?;
                word.part(Part::Arithmetic { expression, quoted });
                return Ok(());
            }
            let (tokens, after) = lex(&rest[1..], true, false)?;
            let body = Reader::new(tokens).list(&[])?;
            *chars = after.chars();
            word.part(Part::Substitution { body, quoted });
            return Ok(());
        }
        Some('\'') if !quoted => {
            chars.next();
            word.literal(&ansi::quoted(chars)?);
            return Ok(());
        }
        Some('"') if !quoted => {
            chars.next();
            self::quoted(chars, '"', word)?;
            return Ok(());
        }
        _ => {
            word.push('$');
            return Ok(());
        }
    };
    word.part(Part::Parameter {
        name: name.into(),
        quoted,
    });
    Ok(())
}

pub(super) fn backquote(
    chars: &mut Chars<'_>,
    word: &mut WordBuilder,
    quoted: bool,
) -> Result<(), Error> {
    let mut source = String::new();
    while let Some(c) = chars.next() {
        match c {
            '`' => {
                let body = parse(&source)?;
                word.part(Part::Substitution { body, quoted });
                return Ok(());
            }
            '\\' => {
                let next = chars
                    .next()
                    .ok_or(Error::Incomplete("unterminated backquote"))?;
                if next != '\n' {
                    if !(matches!(next, '$' | '`' | '\\') || quoted && next == '"') {
                        source.push('\\');
                    }
                    source.push(next);
                }
            }
            _ => source.push(c),
        }
    }
    Err(Error::Incomplete("unterminated backquote"))
}

fn alternative_word(chars: &mut Chars<'_>, outer_quoted: bool) -> Result<Word, Error> {
    Ok(parameter_word(chars, outer_quoted, false)?.0)
}

fn parameter_word(
    chars: &mut Chars<'_>,
    outer_quoted: bool,
    stop_slash: bool,
) -> Result<(Word, char), Error> {
    let mut word = WordBuilder::default();
    let mut empty = true;
    while let Some(c) = chars.next() {
        match c {
            c if c == '}' || stop_slash && c == '/' => {
                if empty && !outer_quoted {
                    word.part(Part::Unquoted(String::new()));
                }
                return Ok((word.finish(), c));
            }
            '$' if outer_quoted && chars.as_str().starts_with('\'') => {
                chars.next();
                word.literal(&ansi::quoted(chars)?);
            }
            '$' => dollar(chars, &mut word, outer_quoted)?,
            '"' => quoted(chars, c, &mut word)?,
            '\'' if !outer_quoted => quoted(chars, c, &mut word)?,
            '`' => backquote(chars, &mut word, outer_quoted)?,
            '\\' => {
                let next = chars
                    .next()
                    .ok_or(Error::Incomplete("unterminated parameter expansion"))?;
                if next == '\n' {
                    continue;
                }
                if outer_quoted && !matches!(next, '"' | '$' | '`' | '\\' | '}') {
                    word.push('\\');
                }
                word.push(next);
            }
            _ => {
                let rest = chars.as_str();
                let end = rest
                    .find(|v| {
                        stop_slash && v == '/'
                            || matches!(v, '}' | '$' | '"' | '`' | '\\')
                            || (!outer_quoted && v == '\'')
                    })
                    .unwrap_or(rest.len());
                let mut text = String::from(c);
                text.push_str(&rest[..end]);
                *chars = rest[end..].chars();
                if outer_quoted {
                    word.literal(&text);
                } else {
                    word.part(Part::Unquoted(text));
                }
            }
        }
        empty = false;
    }
    Err(Error::Incomplete("unterminated parameter expansion"))
}

pub(super) fn quoted(
    chars: &mut Chars<'_>,
    quote: char,
    word: &mut WordBuilder,
) -> Result<(), Error> {
    if quote == '\'' {
        let (literal, rest) = chars
            .as_str()
            .split_once(quote)
            .ok_or(Error::Incomplete("unterminated quote"))?;
        word.literal(literal);
        *chars = rest.chars();
        return Ok(());
    }
    let mut empty = true;
    while let Some(c) = chars.next() {
        if c == '"' {
            if empty {
                word.literal("");
            }
            return Ok(());
        }
        match c {
            '\\' if matches!(chars.clone().next(), Some('"' | '$' | '`' | '\\' | '\n')) => {
                let escaped = chars.next().unwrap();
                if escaped == '\n' {
                    continue;
                }
                word.push(escaped);
            }
            '$' => dollar(chars, word, true)?,
            '`' => backquote(chars, word, true)?,
            _ => word.push(c),
        }
        empty = false;
    }
    Err(Error::Incomplete("unterminated quote"))
}

fn process_start(source: &str) -> bool {
    source.starts_with("<(") || source.starts_with(">(")
}

pub(super) fn separator(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\n' | ';' | '|' | '&' | '<' | '>' | '(' | ')'
    )
}

pub(super) fn arithmetic_clauses(source: &str) -> Result<[&str; 3], Error> {
    let mut chars = source.chars();
    let mut fields = [""; 3];
    let mut index = 0;
    let mut start = 0;
    let mut depth = 0usize;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '"' | '\'' => quoted(&mut chars, c, &mut WordBuilder::default())?,
            '`' => backquote(&mut chars, &mut WordBuilder::default(), true)?,
            '$' if chars.as_str().starts_with(['(', '{']) => {
                dollar(&mut chars, &mut WordBuilder::default(), true)?
            }
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => {
                if index >= 2 {
                    return Err(Error::Invalid("arithmetic for requires three expressions"));
                }
                let end = source.len() - chars.as_str().len();
                fields[index] = &source[start..end - 1];
                start = end;
                index += 1;
            }
            _ => {}
        }
    }
    if index != 2 {
        return Err(Error::Invalid("arithmetic for requires three expressions"));
    }
    fields[2] = &source[start..];
    Ok(fields)
}

pub(crate) fn arithmetic_word(source: &str) -> Result<crate::word::Word, Error> {
    subscript_word(source, false)
}

pub(crate) fn subscript_word(source: &str, associative: bool) -> Result<crate::word::Word, Error> {
    let mut chars = source.chars();
    let mut word = WordBuilder::default();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted(&mut chars, c, &mut word)?,
            '\'' if associative => quoted(&mut chars, c, &mut word)?,
            '$' => dollar(&mut chars, &mut word, true)?,
            '`' => backquote(&mut chars, &mut word, true)?,
            '\\' => {
                let next = chars
                    .next()
                    .ok_or(Error::Incomplete("trailing arithmetic escape"))?;
                if next != '\n' {
                    word.push(next);
                }
            }
            _ => word.push(c),
        }
    }
    Ok(word.finish())
}

fn arithmetic_source(chars: &mut Chars<'_>) -> Result<String, Error> {
    let source = chars.as_str();
    let mut depth = 0usize;
    while let Some(c) = chars.next() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            ')' if chars.as_str().starts_with(')') => {
                let end = source.len() - chars.as_str().len() - 1;
                chars.next();
                return Ok(source[..end].into());
            }
            ')' => {
                return Err(Error::Invalid(
                    "invalid closing parenthesis in arithmetic expression",
                ));
            }
            _ => {}
        }
    }
    Err(Error::Incomplete("unterminated arithmetic expression"))
}

fn pattern_parts(
    chars: &mut Chars<'_>,
    word: &mut WordBuilder,
    group: Option<char>,
) -> Result<(), Error> {
    let mut raw = group.map_or_else(String::new, |op| op.to_string());
    let mut depth = 0usize;
    while let Some(c) = chars.clone().next() {
        if group.is_none() && depth == 0 && matches!(c, ' ' | '\t' | '\n' | ';') {
            break;
        }
        chars.next();
        if matches!(c, '\'' | '"' | '\\' | '`' | '$') && !raw.is_empty() {
            word.part(Part::Pattern(std::mem::take(&mut raw)));
        }
        match c {
            '\'' | '"' => quoted(chars, c, word)?,
            '\\' => {
                let escaped = chars
                    .next()
                    .ok_or(Error::Incomplete("trailing regular expression escape"))?;
                if escaped != '\n' {
                    word.push(escaped);
                }
            }
            '$' if chars.clone().next().is_some_and(|next| {
                next.is_ascii_alphanumeric()
                    || matches!(
                        next,
                        '{' | '(' | '_' | '@' | '*' | '?' | '$' | '#' | '!' | '-'
                    )
            }) =>
            {
                dollar(chars, word, false)?
            }
            '`' => backquote(chars, word, false)?,
            '[' if group.is_some()
                && let Some(end) = crate::pattern::bracket_end(chars.as_str())
                && !chars.as_str()[..=end].contains(['$', '\u{60}', '\u{27}', '"', '\\']) =>
            {
                raw.push('[');
                raw.push_str(&chars.as_str()[..=end]);
                *chars = chars.as_str()[end + 1..].chars();
            }
            '(' => {
                depth += 1;
                raw.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                raw.push(c);
                if group.is_some() && depth == 0 {
                    break;
                }
            }
            _ => raw.push(c),
        }
    }
    if !raw.is_empty() {
        word.part(Part::Pattern(raw));
    }
    if group.is_some() && depth != 0 {
        return Err(Error::Incomplete("unterminated extended pattern"));
    }
    Ok(())
}

fn pattern_chunk(chars: &mut Chars<'_>, word: &mut WordBuilder, start: &str, extended: bool) {
    // Keep the spelling of ranges and classes after the first wildcard or
    // expansion. Quoted and escaped pieces take a separate, literal path.
    let rest = chars.as_str();
    let end = rest
        .find(|c| separator(c) || matches!(c, '\'' | '"' | '\\' | '$' | '`' | '{' | ',' | '}'))
        .unwrap_or(rest.len());
    // Leave the operator before the next opening parenthesis for the group reader.
    let end = if extended
        && rest[end..].starts_with('(')
        && rest[..end].ends_with(['?', '*', '+', '@', '!'])
    {
        end - 1
    } else {
        end
    };
    let len = start.len() - rest.len() + end;
    word.part(Part::Pattern(start[..len].into()));
    *chars = rest[end..].chars();
}

fn command_position(tokens: &[Token]) -> bool {
    for token in tokens.iter().rev() {
        match token {
            Token::Word(word)
                if !word.protected
                    && matches!(
                        word.word.as_literal(),
                        Some(
                            "if" | "while" | "until" | "elif" | "then" | "else" | "do" | "!" | "{"
                        )
                    ) =>
            {
                continue;
            }
            Token::Word(_)
            | Token::Arithmetic(_)
            | Token::Redirect(..)
            | Token::BothRedirect(_) => return false,
            _ => return true,
        }
    }
    true
}

pub(super) fn lex(
    source: &str,
    substitution: bool,
    final_input: bool,
) -> Result<(Vec<Token>, &str), Error> {
    let mut chars = source.chars();
    let mut parentheses = 0usize;
    let mut saw_case = false;
    let mut in_test = false;
    let mut pattern_next: Option<bool> = None;
    let mut pending_docs = Vec::new();
    let mut tokens = Vec::new();
    while let Some(c) = chars.clone().next() {
        if matches!(c, ' ' | '\t') {
            chars.next();
            continue;
        }
        let pattern = pattern_next.take();
        if pattern == Some(false)
            && !matches!(c, '\n' | ';')
            && chars
                .as_str()
                .strip_prefix("]]")
                .is_none_or(|rest| !rest.chars().next().is_none_or(separator))
        {
            let mut word = WordBuilder::default();
            pattern_parts(&mut chars, &mut word, None)?;
            tokens.push(Token::Word(LexWord {
                word: word.finish(),
                protected: false,
                assignment: None,
            }));
            continue;
        }
        let extended = pattern == Some(true) || super::EXTGLOB.get();
        if c == '#' {
            let rest = chars.as_str();
            chars = rest[rest.find('\n').unwrap_or(rest.len())..].chars();
            continue;
        }
        if separator(c) && !process_start(chars.as_str()) {
            chars.next();
            if substitution && c == ')' && parentheses == 0 {
                // A case pattern can end with an unmatched right parenthesis.
                // Ask the grammar only for substitutions containing a case word.
                let ends_pattern = saw_case
                    && matches!(
                        Reader::new(tokens.clone()).list(&[]),
                        Err(Error::Incomplete("expected ')' after case pattern"))
                    );
                if !ends_pattern {
                    return Ok((tokens, chars.as_str()));
                }
            }
            if c == '<' && chars.as_str().starts_with('<') {
                here_doc::redirect(&mut chars, 0, &mut tokens, &mut pending_docs)?;
                continue;
            }
            let token = match c {
                '\n' => {
                    here_doc::collect(&mut chars, &mut tokens, &mut pending_docs, final_input)?;
                    Token::Newline
                }
                ';' if chars.as_str().starts_with(';') => {
                    chars.next();
                    if chars.as_str().starts_with('&') {
                        chars.next();
                        Token::EndArm(CaseEnd::Retest)
                    } else {
                        Token::EndArm(CaseEnd::Stop)
                    }
                }
                ';' if chars.as_str().starts_with('&') => {
                    chars.next();
                    Token::EndArm(CaseEnd::FallThrough)
                }
                ';' => Token::End,
                '(' if chars.as_str().starts_with('(') && !in_test => {
                    chars.next();
                    Token::Arithmetic(arithmetic_source(&mut chars)?)
                }
                '(' => {
                    parentheses += 1;
                    Token::LeftParen
                }
                ')' => {
                    parentheses = parentheses.saturating_sub(1);
                    Token::RightParen
                }
                '|' if chars.as_str().starts_with('|') => {
                    chars.next();
                    Token::Or
                }
                '|' if chars.as_str().starts_with('&') => {
                    chars.next();
                    Token::PipeBoth
                }
                '|' => Token::Pipe,
                '&' if chars.as_str().starts_with('&') => {
                    chars.next();
                    Token::And
                }
                '&' if chars.as_str().starts_with('>') => {
                    chars.next();
                    if chars.as_str().starts_with('>') {
                        chars.next();
                        Token::BothRedirect(Target::Append)
                    } else {
                        Token::BothRedirect(Target::Write)
                    }
                }
                '&' => Token::Background,
                op => redirect(&mut chars, op, if op == '<' { 0 } else { 1 })?,
            };
            tokens.push(token);
            continue;
        }
        let mut word = WordBuilder::default();
        let mut protected = false;
        let mut started = false;
        let mut pattern_context = false;
        let mut braces = 0usize;
        let mut assignment = None;
        let start = chars.as_str();
        if let Some(prefix) = super::array::indexed_prefix(start) {
            word.literal(&start[..prefix.len() + 1]);
            assignment = Some(prefix.to_owned());
            chars = start[prefix.len() + 1..].chars();
            started = true;
        }
        while let Some(c) = chars.clone().next() {
            if separator(c) && !process_start(chars.as_str()) {
                break;
            }
            let rest = chars.as_str();
            chars.next();
            match c {
                '<' | '>' => {
                    chars.next();
                    let (tokens, after) = lex(chars.as_str(), true, false)?;
                    let body = Reader::new(tokens).list(&[])?;
                    chars = after.chars();
                    word.part(Part::Process {
                        body,
                        input: c == '<',
                    });
                }
                '\'' | '"' => {
                    protected = true;
                    quoted(&mut chars, c, &mut word)?;
                }
                '\\' => {
                    let escaped = chars
                        .next()
                        .ok_or(Error::Incomplete("trailing backslash"))?;
                    if escaped == '\n' {
                        if chars.as_str().is_empty() {
                            return Err(Error::Incomplete("line continuation"));
                        }
                        continue;
                    }
                    protected = true;
                    word.push(escaped);
                }
                '$' => {
                    protected |= chars.as_str().starts_with(['\'', '"']);
                    let named = chars
                        .as_str()
                        .as_bytes()
                        .first()
                        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_');
                    dollar(&mut chars, &mut word, false)?;
                    if named
                        && (chars.as_str().starts_with('{')
                            || braces > 0 && chars.as_str().starts_with([',', '}']))
                    {
                        word.brace_parameter();
                    }
                    pattern_context = true;
                }
                '[' | '{' if !started && chars.clone().next().is_none_or(separator) => word.push(c),
                '`' => {
                    backquote(&mut chars, &mut word, false)?;
                    pattern_context = true;
                }
                '[' if !started
                    && chars.as_str().starts_with('[')
                    && chars.as_str()[1..].chars().next().is_none_or(separator) =>
                {
                    word.literal("[[");
                    chars.next();
                }
                '{' => {
                    pattern_context = true;
                    let tail = chars.as_str();
                    if let Some((inner, after)) = tail.split_once('}')
                        && let Some(range) = crate::word::brace::Range::parse(inner)
                    {
                        word.part(Part::Range(Box::new(range)));
                        chars = after.chars();
                    } else {
                        word.part(Part::Brace("{"));
                        braces += 1;
                    }
                }
                '}' if braces > 0 => {
                    word.part(Part::Brace("}"));
                    braces -= 1;
                }
                ',' if braces > 0 => word.part(Part::Brace(",")),
                '@' | '?' | '*' | '+' | '!' if extended && chars.as_str().starts_with('(') => {
                    pattern_parts(&mut chars, &mut word, Some(c))?;
                    pattern_context = true;
                }
                '*' | '?' | '[' => {
                    pattern_context = true;
                    pattern_chunk(&mut chars, &mut word, rest, extended);
                }
                '=' if assignment.is_none() && !protected => {
                    let prefix = &start[..start.len() - rest.len()];
                    if is_identifier(prefix.strip_suffix('+').unwrap_or(prefix)) {
                        assignment = Some(prefix.to_owned());
                    }
                    word.push(c);
                }
                '~' if braces != 0 || chars.as_str().starts_with('{') => {
                    let tilde = if chars.as_str().starts_with('+') {
                        chars.next();
                        "~+"
                    } else if chars.as_str().starts_with('-') {
                        chars.next();
                        "~-"
                    } else {
                        "~"
                    };
                    word.part(Part::BraceTilde(tilde));
                }
                '~' if !started
                    || assignment
                        .as_ref()
                        .is_some_and(|name| start.len() - rest.len() == name.len() + 1) =>
                {
                    let length = chars
                        .as_str()
                        .find(|c| c == '/' || separator(c))
                        .unwrap_or(chars.as_str().len());
                    let suffix = &chars.as_str()[..length];
                    if matches!(suffix, "" | "+" | "-") {
                        word.part(Part::Tilde(format!("~{suffix}")));
                        chars = chars.as_str()[length..].chars();
                    } else {
                        word.push(c);
                    }
                }
                _ if pattern_context => pattern_chunk(&mut chars, &mut word, rest, extended),
                _ => word.push(c),
            }
            started = true;
        }
        if !started {
            continue;
        }
        let word = word.finish();
        if !protected && matches!(chars.clone().next(), Some('<' | '>')) {
            if let Some(text) = word
                .as_literal()
                .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            {
                let op = chars.next().unwrap();
                let fd = descriptor(text)?;
                if op == '<' && chars.as_str().starts_with('<') {
                    here_doc::redirect(&mut chars, fd, &mut tokens, &mut pending_docs)?;
                } else {
                    tokens.push(redirect(&mut chars, op, fd)?);
                }
                continue;
            }
        }
        if !protected {
            match word.as_literal() {
                Some("[[") if command_position(&tokens) => in_test = true,
                Some("]]") => in_test = false,
                Some("=~") if in_test => pattern_next = Some(false),
                Some("=" | "==" | "!=") if in_test => pattern_next = Some(true),
                _ => {}
            }
        }
        if substitution && !protected && word.as_literal() == Some("case") {
            saw_case = true;
        }
        let begins_array = assignment.as_ref().is_some_and(|name| {
            chars.as_str().starts_with('(') && start.len() - chars.as_str().len() == name.len() + 1
        });
        tokens.push(Token::Word(LexWord {
            word,
            protected,
            assignment,
        }));
        if begins_array {
            chars.next();
            parentheses += 1;
            tokens.push(Token::ArrayStart);
        }
    }
    here_doc::collect(&mut chars, &mut tokens, &mut pending_docs, final_input)?;
    if substitution {
        return Err(Error::Incomplete("unterminated command substitution"));
    }
    Ok((tokens, ""))
}
