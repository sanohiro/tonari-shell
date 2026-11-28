use super::Error;
use super::lexer::{backquote, dollar, quoted};
use crate::word::{Word, WordBuilder};
use std::str::Chars;

/// The colon separating offset from length is outside arithmetic parentheses,
/// nested shell expansions, and the arithmetic conditional operator.
pub(super) fn slice(chars: &mut Chars<'_>) -> Result<(Word, Option<Word>), Error> {
    let mut word = WordBuilder::default();
    let mut offset = None;
    let mut parens = 0usize;
    let mut conditional = 0usize;
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted(chars, c, &mut word)?,
            '$' => dollar(chars, &mut word, true)?,
            '`' => backquote(chars, &mut word, true)?,
            '}' => {
                let last = word.finish();
                return Ok(match offset {
                    Some(start) => (start, Some(last)),
                    None => (last, None),
                });
            }
            ':' if parens == 0 && conditional == 0 && offset.is_none() => {
                offset = Some(std::mem::take(&mut word).finish());
            }
            _ => {
                match c {
                    '(' => parens += 1,
                    ')' => parens = parens.saturating_sub(1),
                    '?' if parens == 0 => conditional += 1,
                    ':' if parens == 0 => conditional = conditional.saturating_sub(1),
                    _ => {}
                }
                word.push(c);
            }
        }
    }
    Err(Error::incomplete_message("unterminated parameter slice"))
}

pub(crate) fn index_end(s: &str) -> Result<usize, Error> {
    let body = s
        .strip_prefix('[')
        .ok_or(Error::Invalid("expected array subscript"))?;
    if let Some(end) = body
        .bytes()
        .position(|b| matches!(b, b']' | b'[' | b'$' | b'`' | b'\'' | b'"' | b'\\'))
        && body.as_bytes()[end] == b']'
    {
        return Ok(end + 2);
    }
    let mut brackets = 0usize;
    let mut quote = None;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' && quote != Some('\'') {
            chars.next();
            continue;
        }
        if c == '$' && matches!(quote, None | Some('"')) && chars.as_str().starts_with(['(', '{']) {
            dollar(&mut chars, &mut WordBuilder::default(), quote.is_some())?;
            continue;
        }
        if quote == Some(c) {
            quote = None;
            continue;
        }
        if quote.is_some() {
            continue;
        }
        match c {
            '\'' | '"' | '`' => quote = Some(c),
            '[' => brackets += 1,
            ']' => {
                brackets -= 1;
                if brackets == 0 {
                    return Ok(s.len() - chars.as_str().len());
                }
            }
            _ => {}
        }
    }
    Err(Error::incomplete_message("unterminated array subscript"))
}
