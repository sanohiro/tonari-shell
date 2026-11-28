use super::lexer::{LexWord, Token};
use super::{Error, Reader, Word, is_identifier};
use crate::word::name_chars_len;

#[derive(Debug, Clone)]
pub struct Subscript<T> {
    pub index: T,
    pub append: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ArrayItems<W> {
    pub words: Vec<(Option<Subscript<String>>, W)>,
    // A bare first word can introduce either indexed entries or associative
    // pairs. Only keep alternate spellings where those interpretations differ.
    pub pair_sources: Vec<(usize, std::rc::Rc<str>)>,
}

impl ArrayItems<Word> {
    pub(crate) fn indexed_words(self) -> impl Iterator<Item = (Option<Subscript<String>>, Word)> {
        let possible_pairs = self.words.first().is_some_and(|(index, _)| index.is_none());
        self.words.into_iter().map(move |(index, mut word)| {
            if possible_pairs && let Some(index) = &index {
                // The lexer retained the prefix for either interpretation.
                word.remove_assignment_prefix(index.index.len() + 3 + usize::from(index.append))
                    .expect("parsed array subscript");
            }
            (index, word)
        })
    }
}

#[derive(Debug, Clone)]
pub enum Assignment<W> {
    Scalar(W),
    Array(Option<usize>, ArrayItems<W>, super::Location),
}

// Keep subscripts intact while lexing the assignment name. They are arithmetic
// expressions, and information such as whitespace belongs to that expression.
pub(super) fn indexed_prefix(source: &str) -> Option<&str> {
    let n = name_chars_len(source);
    if !source[n..].starts_with('[') || n != 0 && !is_identifier(&source[..n]) {
        return None;
    }
    let end = n + super::modifier::index_end(&source[n..]).ok()?;
    let plus = usize::from(source[end..].starts_with('+'));
    source[end + plus..]
        .starts_with('=')
        .then(|| &source[..end + plus])
}

pub(super) fn assignment_prefix(source: &str) -> Option<(std::borrow::Cow<'_, str>, usize)> {
    if let Some(prefix) = indexed_prefix(source) {
        return Some((prefix.into(), prefix.len() + 1));
    }
    let mut cursor = source;
    loop {
        cursor = &cursor[name_chars_len(cursor)..];
        let rest = super::lexer::joined(cursor);
        if rest.len() == cursor.len() {
            break;
        }
        cursor = rest;
    }
    let name = &source[..source.len() - cursor.len()];
    let end = super::modifier::index_end(cursor).ok()?;
    let subscript = &cursor[..end];
    let rest = super::lexer::joined(&cursor[end..]);
    let plus = rest.starts_with('+');
    let rest = super::lexer::joined(&rest[usize::from(plus)..]);
    let tail = rest.strip_prefix('=')?;
    let mut prefix = name.replace("\\\n", "");
    if !prefix.is_empty() && !is_identifier(&prefix) {
        return None;
    }
    prefix.push_str(subscript);
    if plus {
        prefix.push('+');
    }
    Some((prefix.into(), source.len() - tail.len()))
}

pub(crate) fn quoted_initializer(source: &str) -> Result<ArrayItems<Word>, Error> {
    if let Some(body) = source.strip_prefix('(').and_then(|s| s.strip_suffix(')'))
        && body.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/' | b' ' | b'\t')
        })
    {
        return Ok(ArrayItems {
            words: body
                .split_ascii_whitespace()
                .map(|word| (None, Word::Literal(word.into())))
                .collect(),
            pair_sources: Vec::new(),
        });
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

fn pair_needs_source(index: Option<&str>, word: &Word) -> bool {
    use crate::word::Part;
    // The lexer kept subscript syntax verbatim and recognized RHS tildes and
    // process substitutions as assignment operands. Pair words use other rules.
    if index.is_some_and(|s| s.contains(['$', '`', '"', '\'', '\\', '<', '>'])) {
        return true;
    }
    match word {
        Word::Literal(s) => s.contains(":~") || s.starts_with('~') && s.contains(':'),
        Word::Expanded(parts) | Word::Declaration(parts) => parts.iter().any(|part| match part {
            Part::Process { .. } | Part::Tilde(_) => true,
            Part::Text(text) | Part::Pattern(text) => text.contains('~'),
            _ => false,
        }),
    }
}

impl Reader {
    pub(super) fn array_items<const COLLECT: bool>(&mut self) -> Result<ArrayItems<Word>, Error> {
        self.tokens.pop_front();
        let mut items = ArrayItems::default();
        let mut possible_pairs = true;
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
                        .map(|len| {
                            word.assignment_name(len)
                                .map(str::to_owned)
                                .ok_or(Error::Invalid("invalid assignment prefix"))
                        })
                        .transpose()?
                        .filter(|k| k.starts_with('[') && (k.ends_with(']') || k.ends_with("]+")));
                    if items.words.is_empty() {
                        possible_pairs = index.is_none();
                    }
                    if possible_pairs && pair_needs_source(index.as_deref(), &word) {
                        let source = self.location(self.position() - 1);
                        items
                            .pair_sources
                            .push((items.words.len(), source.text().into()));
                    }
                    if !possible_pairs && let Some(name) = &index {
                        word.remove_assignment_prefix(name.len() + 1)
                            .ok_or(Error::Invalid("invalid assignment prefix"))?;
                    }
                    items.words.push((
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
                    return Err(Error::incomplete_message("unfinished array assignment"));
                }
                Some(unexpected) => {
                    // Resume only after the matching closing parenthesis. A later
                    // physical line is still array text until that delimiter.
                    let mut depth = 1;
                    let mut token = Some(unexpected);
                    while let Some(current) = token {
                        match current {
                            Token::LeftParen | Token::ArrayStart => depth += 1,
                            Token::RightParen => {
                                depth -= 1;
                                if depth == 0 {
                                    self.invalid_array_end =
                                        Some(self.location(self.position() - 1).end);
                                    return Ok(items);
                                }
                            }
                            _ => {}
                        }
                        token = self.tokens.pop_front();
                    }
                    return Err(if self.final_input {
                        Error::ArrayToken(if self.depth > 1 { 2 } else { 1 })
                    } else {
                        Error::incomplete_message("unfinished array assignment")
                    });
                }
            }
        }
    }
}
