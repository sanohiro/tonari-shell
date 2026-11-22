use super::lexer::{LexWord, Token};
use super::{Error, Reader, Word, is_identifier};

#[derive(Debug, Clone)]
pub enum Assignment<W> {
    Scalar(W),
    Array(Vec<(Option<String>, W)>),
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
    let end = source[n + 1..].find(']')? + n + 2;
    let plus = usize::from(source[end..].starts_with('+'));
    source[end + plus..]
        .starts_with('=')
        .then(|| &source[..end + plus])
}

impl Reader {
    pub(super) fn array_items(&mut self) -> Result<Vec<(Option<String>, Word)>, Error> {
        self.tokens.next();
        let mut items = Vec::new();
        loop {
            match self.tokens.next() {
                Some(Token::RightParen) => return Ok(items),
                Some(Token::Newline) => {}
                Some(Token::Word(LexWord {
                    mut word,
                    assignment,
                    ..
                })) => {
                    let index = assignment.filter(|k| k.starts_with('[') && k.ends_with(']'));
                    if let Some(name) = &index {
                        word.remove_assignment_prefix(name.len() + 1);
                    }
                    items.push((index.map(|s| s[1..s.len() - 1].into()), word));
                }
                None => return Err(Error::Incomplete("unfinished array assignment")),
                _ => return Err(Error::Invalid("unexpected token in array assignment")),
            }
        }
    }
}
