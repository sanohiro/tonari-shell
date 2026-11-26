use super::lexer::{LexWord, Token};
use super::{Error, Reader, Word, is_identifier};

#[derive(Debug, Clone)]
pub struct Subscript<T> {
    pub index: T,
    pub append: bool,
}

#[derive(Debug, Clone)]
pub enum Assignment<W> {
    Scalar(W),
    Array(Option<usize>, Vec<(Option<Subscript<String>>, W)>),
}

// Keep subscripts intact while lexing the assignment name. They are arithmetic
// expressions, and information such as whitespace belongs to that expression.
pub(super) fn indexed_prefix(source: &str) -> Option<&str> {
    let n = source
        .bytes()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_')
        .count();
    if !source[n..].starts_with('[') || n != 0 && !is_identifier(&source[..n]) {
        return None;
    }
    let end = n + super::modifier::index_end(&source[n..]).ok()?;
    let plus = usize::from(source[end..].starts_with('+'));
    source[end + plus..]
        .starts_with('=')
        .then(|| &source[..end + plus])
}

pub(crate) fn quoted_initializer(
    source: &str,
) -> Result<Vec<(Option<Subscript<String>>, Word)>, Error> {
    let (tokens, spans, _) = super::lexer::lex_initializer(source)?;
    let mut reader = Reader::new(tokens, spans, source);
    if !matches!(reader.peek(), Some(Token::LeftParen)) {
        return Err(Error::Invalid("expected array initializer"));
    }
    let items = reader.array_items()?;
    if !reader.tokens.is_empty() {
        return Err(Error::Invalid("unexpected token after array initializer"));
    }
    Ok(items)
}

impl Reader {
    pub(super) fn array_items(&mut self) -> Result<Vec<(Option<Subscript<String>>, Word)>, Error> {
        self.tokens.pop_front();
        let mut items = Vec::new();
        loop {
            match self.operand() {
                Some(Token::RightParen) => return Ok(items),
                Some(Token::Newline) => {}
                Some(Token::Word(LexWord {
                    mut word,
                    assignment,
                    ..
                })) => {
                    let index = assignment
                        .filter(|k| k.starts_with('[') && (k.ends_with(']') || k.ends_with("]+")));
                    if let Some(name) = &index {
                        word.remove_assignment_prefix(name.len() + 1);
                    }
                    items.push((
                        index.map(|s| {
                            let append = s.ends_with('+');
                            Subscript {
                                index: s[1..s.len() - 1 - usize::from(append)].into(),
                                append,
                            }
                        }),
                        word,
                    ));
                }
                None => return Err(Error::Incomplete("unfinished array assignment")),
                _ => return Err(Error::Invalid("unexpected token in array assignment")),
            }
        }
    }
}
