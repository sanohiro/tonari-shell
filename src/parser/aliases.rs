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
            let (mut replacement, spans, _) = with(nested, || lex(value, false, true))?;
            let index = self.position();
            let (start, end) = self.spans[index];
            let mut source = self.source.to_string();
            source.replace_range(start..end, value);
            self.newlines = source.match_indices('\n').map(|(n, _)| n).collect();
            self.source = source.into();
            for (a, b) in &mut self.spans[index + 1..] {
                *a = *a - (end - start) + value.len();
                *b = *b - (end - start) + value.len();
            }
            self.spans.splice(
                index..=index,
                spans.into_iter().map(|(a, b)| (start + a, start + b)),
            );
            self.tokens.pop_front();
            self.alias_frames.push(Frame {
                name,
                remaining: self.tokens.len(),
                next_word,
            });
            replacement.extend(self.tokens.drain(..));
            self.tokens = replacement.into();
        }
    }
}
