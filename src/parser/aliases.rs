use super::{Error, Reader, Token, lex, reserved};
use crate::alias::Aliases;
use std::cell::RefCell;

thread_local! {
    // Substitutions and backquotes start nested parsers while their outer
    // command is being read. They inherit that read's alias definitions.
    static CONTEXT: RefCell<Aliases> = const { RefCell::new(Aliases(None)) };
}

pub(super) fn current() -> Aliases {
    CONTEXT.with_borrow(Clone::clone)
}

pub(super) fn with<T>(aliases: Aliases, work: impl FnOnce() -> T) -> T {
    struct Restore(Option<Aliases>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CONTEXT.replace(self.0.take().unwrap());
        }
    }
    let _restore = Restore(Some(CONTEXT.replace(aliases)));
    work()
}

pub(super) struct Frame {
    name: String,
    remaining: usize,
    next_word: bool,
}

// Alias substitutions proceed left-to-right. Keep the rewritten prefix and
// its token spans; untouched suffix bytes and offsets stay in the lexer input.
#[derive(Default)]
pub(super) struct Expanded {
    text: String,
    spans: Vec<(usize, usize)>,
    newlines: Vec<usize>,
    original_byte: usize,
    original_token: usize,
}

impl Expanded {
    pub(super) fn token_count(&self, original_count: usize) -> usize {
        self.spans.len() + (original_count - self.original_token)
    }

    fn replace(
        &mut self,
        index: usize,
        value: &str,
        replacement: Vec<(usize, usize)>,
        original: &str,
        tokens: &[(usize, usize)],
    ) {
        let rescan;
        if index >= self.spans.len() {
            let token = self.original_token + (index - self.spans.len());
            let (start, end) = tokens[token];
            let prefix = self.text.len();
            rescan = prefix;
            self.spans
                .extend(tokens[self.original_token..token].iter().map(|&(a, b)| {
                    (
                        prefix + (a - self.original_byte),
                        prefix + (b - self.original_byte),
                    )
                }));
            self.text.push_str(&original[self.original_byte..start]);
            let at = self.text.len();
            self.text.push_str(value);
            self.spans
                .extend(replacement.into_iter().map(|(a, b)| (at + a, at + b)));
            self.original_byte = end;
            self.original_token = token + 1;
        } else {
            // A nested alias rewrites a token in the replacement just inserted.
            let (start, end) = self.spans[index];
            self.text.replace_range(start..end, value);
            self.newlines
                .truncate(self.newlines.partition_point(|&n| n < start));
            rescan = start;
            if value.len() != end - start {
                for (a, b) in &mut self.spans[index + 1..] {
                    *a = *a - (end - start) + value.len();
                    *b = *b - (end - start) + value.len();
                }
            }
            self.spans.splice(
                index..=index,
                replacement.into_iter().map(|(a, b)| (start + a, start + b)),
            );
        }
        self.newlines.extend(
            memchr::memchr_iter(b'\n', &self.text.as_bytes()[rescan..]).map(|i| rescan + i),
        );
    }

    fn span(&self, index: usize, reader: &Reader) -> (usize, usize) {
        if let Some(span) = self.spans.get(index) {
            return *span;
        }
        let original = self.original_token + (index - self.spans.len());
        let (a, b) = reader
            .spans
            .get(original)
            .copied()
            .unwrap_or((reader.source.len(), reader.source.len()));
        (
            self.text.len() + (a - self.original_byte),
            self.text.len() + (b - self.original_byte),
        )
    }

    pub(super) fn location(
        &self,
        reader: &Reader,
        from: usize,
        position: usize,
    ) -> super::Location {
        let start = self.span(from, reader).0;
        let end = if position > from {
            self.span(position - 1, reader).1
        } else {
            start
        };
        let boundary = self.text.len();
        let original_at = |at: usize| self.original_byte + (at - boundary);
        let text: std::rc::Rc<str> = if end <= boundary {
            self.text[start..end].into()
        } else if start >= boundary {
            reader.source[original_at(start)..original_at(end)].into()
        } else {
            let mut command = String::with_capacity(end - start);
            command.push_str(&self.text[start..]);
            command.push_str(&reader.source[self.original_byte..original_at(end)]);
            command.into()
        };
        let lines = if start <= boundary {
            self.newlines.partition_point(|&n| n < start)
        } else {
            let a = reader.newlines.partition_point(|&n| n < self.original_byte);
            self.newlines.len() + (reader.newlines.partition_point(|&n| n < original_at(start)) - a)
        };
        super::Location {
            start: 0,
            end: text.len(),
            source: text,
            line: super::FIRST_LINE.get() + lines,
        }
    }
}

impl Reader {
    pub(super) fn expand_alias(&mut self, command_word: bool) -> Result<(), Error> {
        if self.aliases.0.is_none() {
            return Ok(());
        }
        let mut enabled = command_word;
        loop {
            let remaining = self.tokens.len();
            while self
                .alias_frames
                .last()
                .is_some_and(|f| remaining <= f.remaining)
            {
                let frame = self.alias_frames.pop().unwrap();
                enabled |= frame.next_word && frame.remaining == remaining;
            }
            if !enabled {
                return Ok(());
            }
            let Some(Token::Word(word)) = self.peek() else {
                return Ok(());
            };
            if word.protected || word.assignment.is_some() {
                return Ok(());
            }
            let Some(name) = word.word.as_literal() else {
                return Ok(());
            };
            let Some(value) = self.aliases.get(name) else {
                return Ok(());
            };
            if reserved(name) || self.alias_frames.iter().any(|f| f.name == name) {
                return Ok(());
            }
            if self.alias_frames.len() >= 128 {
                return Err(Error::Invalid("alias expansion nesting limit exceeded"));
            }
            let name = name.to_string();
            let next_word = value.ends_with([' ', '\t']);
            // A recursive reference inside a substitution in the replacement
            // must be disabled in that nested parser as well.
            let mut nested = self.aliases.clone();
            if value.contains(['$', '`']) {
                let disabled = std::rc::Rc::make_mut(nested.0.as_mut().unwrap());
                disabled.remove(&name);
                for frame in &self.alias_frames {
                    disabled.remove(&frame.name);
                }
            }
            let (replacement, spans, _) = with(nested, || lex(value, false, true))?;
            let index = self.position();
            self.alias_source
                .get_or_insert_with(Default::default)
                .replace(index, value, spans, &self.source, &self.spans);
            self.tokens.pop_front();
            self.alias_frames.push(Frame {
                name,
                remaining: self.tokens.len(),
                next_word,
            });
            for token in replacement.into_iter().rev() {
                self.tokens.push_front(token);
            }
        }
    }
}
