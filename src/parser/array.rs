use super::Incomplete;
use super::lexer::{LexWord, Token};
use super::{Error, Reader, Word, is_identifier};

#[derive(Debug, Clone)]
pub struct Subscript<T> {
    pub index: T,
    pub append: bool,
}

type ArrayItems<W> = Vec<(Option<Subscript<String>>, W)>;

#[derive(Debug, Clone)]
pub enum Assignment<W> {
    Scalar(W),
    Array(Option<usize>, ArrayItems<W>, super::Location),
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

pub(crate) fn quoted_initializer(source: &str) -> Result<ArrayItems<Word>, Error> {
    if let Some(body) = source.strip_prefix('(').and_then(|s| s.strip_suffix(')'))
        && body.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/' | b' ' | b'\t')
        })
    {
        return Ok(body
            .split_ascii_whitespace()
            .map(|word| (None, Word::Literal(word.into())))
            .collect());
    }
    let (tokens, spans, _) = super::lexer::lex_initializer(source)?;
    let mut reader = Reader::new(tokens, spans, source);
    if !matches!(reader.peek(), Some(Token::LeftParen)) {
        return Err(Error::Invalid("expected array initializer"));
    }
    let items = reader.array_items::<true>()?;
    if !reader.tokens.is_empty() {
        return Err(Error::Invalid("unexpected token after array initializer"));
    }
    Ok(items)
}

impl Reader {
    pub(super) fn array_items<const COLLECT: bool>(&mut self) -> Result<ArrayItems<Word>, Error> {
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
                    if !COLLECT {
                        continue;
                    }
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
                None => {
                    return Err(Error::Incomplete(Incomplete::Message(
                        "unfinished array assignment",
                    )));
                }
                Some(unexpected) => {
                    // Resume only after the matching closing parenthesis. A later
                    // physical line is still array text until that delimiter.
                    let mut depth = 1;
                    for token in std::iter::once(&unexpected).chain(&self.tokens) {
                        match token {
                            Token::LeftParen | Token::ArrayStart => depth += 1,
                            Token::RightParen => {
                                depth -= 1;
                                if depth == 0 {
                                    return Err(Error::ArrayToken);
                                }
                            }
                            _ => {}
                        }
                    }
                    return Err(if self.final_input {
                        Error::Invalid("unfinished invalid array assignment")
                    } else {
                        Error::Incomplete(Incomplete::Message("unfinished array assignment"))
                    });
                }
            }
        }
    }
}
