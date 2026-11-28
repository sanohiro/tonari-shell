use super::{Error, Reader, Token, lex, reserved};
use crate::alias::Aliases;
use std::cell::RefCell;

thread_local! {
    // Substitutions and backquotes start nested parsers while their outer
    // command is being read. They inherit that read's alias definitions.
    static CONTEXT: RefCell<Aliases> = const { RefCell::new(Aliases(None)) };
}

pub(super) fn resume(aliases: Aliases) {
    CONTEXT.replace(aliases);
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

    pub(super) fn span(&self, index: usize, reader: &Reader) -> (usize, usize) {
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
            subscript_aliases: reader.subscript_aliases(),
            source: text,
            line: reader.first_line + lines,
        }
    }
}

// Ordinary parameter expansions do not invoke another parser. A command
// substitution must keep the outer alias disabled while its body is read.
fn may_read_nested_command(value: &str) -> bool {
    if value.contains('`') {
        return true;
    }
    value.contains('$')
        && (value.contains('(')
            || memchr::memchr_iter(b'{', value.as_bytes()).any(|i| {
                super::lexer::starts_braced_command(super::lexer::joined(&value[i + 1..]))
            }))
}

impl Reader {
    pub(super) fn alias_newlines(
        &mut self,
        command_word: bool,
        protected: &[&str],
    ) -> Result<(), Error> {
        loop {
            self.newlines();
            self.alias_at(command_word, Some(protected))?;
            if !matches!(self.peek(), Some(Token::Newline)) {
                return Ok(());
            }
        }
    }

    fn replacement_aliases(&self, name: &str, mask_active: bool) -> Aliases {
        if !mask_active {
            return self.aliases.clone();
        }
        let original = self.aliases.0.as_ref().expect("an alias was found");
        let masked = || {
            let mut table = original.as_ref().clone();
            table.remove(name);
            for frame in &self.alias_frames {
                table.remove(&frame.name);
            }
            Aliases(Some(std::rc::Rc::new(table)))
        };
        if !self.alias_frames.is_empty() {
            return masked();
        }
        type WeakTable = std::rc::Weak<std::collections::BTreeMap<String, String>>;
        thread_local! { static LAST: RefCell<Option<(WeakTable,String,Aliases)>> =const {RefCell::new(None)}; }
        LAST.with_borrow_mut(|cache| {
            if let Some((table, key, replacement)) = cache
                && table.as_ptr() == std::rc::Rc::as_ptr(original)
                && key == name
            {
                return replacement.clone();
            }
            let replacement = masked();
            *cache = Some((
                std::rc::Rc::downgrade(original),
                name.into(),
                replacement.clone(),
            ));
            replacement
        })
    }

    // A replacement ending in shell syntax can take bytes from the following
    // input. Re-lex this uncommon suffix, keeping offsets for outer frames.
    pub(super) fn joined_alias(
        &mut self,
        name: String,
        value: &str,
        nested: Aliases,
    ) -> Result<(), Error> {
        let index = self.position();
        let count = index + self.tokens.len();
        let span = |i| {
            self.alias_source.as_ref().map_or_else(
                || {
                    self.spans
                        .get(i)
                        .copied()
                        .unwrap_or((self.source.len(), self.source.len()))
                },
                |s| s.span(i, self),
            )
        };
        let physical;
        if let Some(expanded) = &self.alias_source {
            let mut joined = expanded.text.clone();
            joined.push_str(&self.source[expanded.original_byte..]);
            physical = std::borrow::Cow::Owned(joined);
        } else {
            physical = std::borrow::Cow::Borrowed(self.source.as_ref());
        }
        let (start, end) = span(index);
        let mut suffix = &physical[end..];
        let mut source = String::from(&physical[..start]);
        source.push_str(value);
        // A trailing blank permits alias substitution on the delimiter word
        // before the lexer starts collecting its here-document.
        if value.ends_with([' ', '\t']) && value.trim_end_matches([' ', '\t']).ends_with("<<") {
            let rest = suffix.trim_start_matches([' ', '\t']);
            let n = rest.find(super::lexer::separator).unwrap_or(rest.len());
            if n != 0
                && let Some(alias) = nested.get(&rest[..n])
            {
                source.push_str(&suffix[..suffix.len() - rest.len()]);
                source.push_str(alias);
                suffix = &rest[n..];
            }
        }
        let boundary = source.len();
        let old_boundary = physical.len() - suffix.len();
        source.push_str(suffix);
        let (mut tokens, offsets, _) = with(nested, || {
            super::lexer::lex_joined(
                &source[start..],
                self.final_input,
                boundary - start,
                self.aliases.clone(),
            )
        })?;
        let mut spans: Vec<_> = (0..index).map(span).collect();
        let boundaries: Vec<_> = self
            .alias_frames
            .iter()
            .map(|f| span(count - f.remaining).0)
            .collect();
        spans.extend(offsets.iter().map(|(a, b)| (start + a, start + b)));
        let remaining = |byte| tokens.len() - offsets.partition_point(|(a, _)| start + a < byte);
        for (f, old) in self.alias_frames.iter_mut().zip(boundaries) {
            f.remaining = remaining(boundary + old.saturating_sub(old_boundary));
        }
        self.alias_frames.push(Frame {
            name,
            remaining: remaining(boundary),
            next_word: value.ends_with([' ', '\t']),
        });
        self.spans = spans;
        for token in &mut tokens {
            if let Token::HereBody(_, (begin, end)) = token {
                *begin += start;
                *end += start;
            }
        }
        self.tokens = tokens.into();
        self.alias_source = None;
        self.newlines = memchr::memchr_iter(b'\n', source.as_bytes()).collect();
        self.source = source.into();
        Ok(())
    }

    pub(super) fn alias_after_name(&mut self) -> Result<(), Error> {
        if self
            .alias_frames
            .last()
            .is_some_and(|f| f.next_word && f.remaining + 1 == self.tokens.len())
            && self.keyword().is_some_and(crate::word::is_identifier)
        {
            let name = self.tokens.pop_front().unwrap();
            self.alias_at(false, Some(&[]))?;
            self.tokens.push_front(name);
        }
        Ok(())
    }

    pub(super) fn expand_alias(&mut self, command_word: bool) -> Result<(), Error> {
        self.alias_at(command_word, if command_word { None } else { Some(&[]) })
    }

    // A trailing blank also enables substitution outside command position.
    // Only words reserved at the current grammar position are protected there.
    pub(super) fn alias_at(
        &mut self,
        command_word: bool,
        protected: Option<&[&str]>,
    ) -> Result<(), Error> {
        if self.aliases.0.is_none() {
            return Ok(());
        }
        let mut enabled = command_word;
        loop {
            let remaining = self.tokens.len();
            let mut after_blank = false;
            while self
                .alias_frames
                .last()
                .is_some_and(|f| remaining <= f.remaining)
            {
                let frame = self.alias_frames.pop().unwrap();
                after_blank |= frame.next_word && frame.remaining == remaining;
                enabled |= after_blank;
            }
            if !enabled {
                return Ok(());
            }
            let name = match self.peek() {
                Some(Token::TimeUtility) => "time",
                Some(Token::Reserved(name)) => *name,
                Some(Token::Word(word)) if !word.protected && word.assignment.is_none() => {
                    let Some(name) = word.word.as_literal() else {
                        return Ok(());
                    };
                    name
                }
                _ => return Ok(()),
            };
            let Some(value) = self.aliases.get(name) else {
                return Ok(());
            };
            if ((!after_blank || super::READ_OPTIONS.get().posix)
                && !matches!(self.peek(), Some(Token::TimeUtility))
                && protected.map_or_else(|| reserved(name), |words| words.contains(&name)))
                || self.alias_frames.iter().any(|f| f.name == name)
            {
                return Ok(());
            }
            let name = name.to_string();
            let next_word = value.ends_with([' ', '\t']);
            if value.ends_with(['<', '>', '\\'])
                || value.contains("<<")
                || value.contains('#')
                || value.ends_with("\\\n")
            {
                let nested = self.replacement_aliases(&name, value.contains(['$', '`']));
                let value = value.to_owned();
                self.joined_alias(name, &value, nested)?;
                continue;
            }
            let nested = self.replacement_aliases(&name, may_read_nested_command(value));
            let replacement = with(nested, || lex(value, false, true));
            let (replacement, spans, _) = match replacement {
                Err(Error::Incomplete(_)) => {
                    let nested = self.replacement_aliases(&name, value.contains(['$', '`']));
                    let value = value.to_owned();
                    self.joined_alias(name, &value, nested)?;
                    continue;
                }
                result => result?,
            };
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
