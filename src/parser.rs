mod ast;
pub use ast::*;
mod history;
pub(crate) use history::{
    history_command, history_designator_spans, last_history_argument, suggestion_end,
};
mod aliases;
mod array;
pub(crate) mod pretty;
mod test_expr;
use crate::pattern::Matcher;
use crate::test_expression::Expr;
pub(crate) use array::quoted_initializer;
pub use array::{Assignment, Subscript};
mod ansi;
pub(crate) use ansi::decode as ansi_string;
pub(crate) mod completion_words;
mod here_doc;
pub(crate) use lexer::{arithmetic_segments, arithmetic_word, subscript_word};
mod lexer;
mod modifier;
mod wrapper;
pub use crate::word::{Word, is_identifier};
pub(crate) use wrapper::opens_function;

pub(crate) fn is_function_name(name: &str) -> bool {
    !name.is_empty()
        && !name.bytes().any(|b| {
            matches!(
                b,
                0 | b' '
                    | b'\t'
                    | b'\n'
                    | b'$'
                    | b'\''
                    | b'"'
                    | b'\\'
                    | b'\x60'
                    | b'('
                    | b')'
                    | b'<'
                    | b'>'
                    | b';'
                    | b'|'
                    | b'&'
            )
        })
}
pub(crate) use here_doc::{HereDocHint, expansion_string};
use lexer::{LexWord, Token, lex};
pub(crate) use modifier::index_end;
use std::rc::Rc;

// Keep text supplied by prompt escapes (including the working directory) in
// literal word segments. It must never be parsed as shell code.
pub(crate) fn prompt_word(
    source: &str,
    expand: bool,
    posix: bool,
    mut escape: impl FnMut(char, &mut std::str::Chars, &mut String),
) -> Result<Word, Error> {
    let mut builder = crate::word::WordBuilder::default();
    let mut chars = source.chars();
    let mut escaped = String::new();
    let mut backslash = false;
    while let Some(c) = chars.next() {
        if backslash && c != '\\' {
            backslash = false;
            if matches!(c, '$' | '\x60' | '"') {
                builder.push(c);
                continue;
            }
            builder.push('\\');
        }
        match c {
            '!' if posix => {
                if chars.as_str().starts_with('!') {
                    chars.next();
                    builder.push('!');
                } else {
                    escaped.clear();
                    escape('!', &mut chars, &mut escaped);
                    builder.literal(&escaped);
                }
            }
            '$' if expand => lexer::dollar(&mut chars, &mut builder, true)?,
            '\x60' if expand => lexer::backquote(&mut chars, &mut builder, true)?,
            '\\' => {
                let Some(code) = chars.next() else {
                    builder.push('\\');
                    break;
                };
                if expand && code == '\\' {
                    if backslash {
                        builder.push('\\');
                    }
                    backslash = !backslash;
                } else {
                    if backslash {
                        builder.push('\\');
                        backslash = false;
                    }
                    escaped.clear();
                    escape(code, &mut chars, &mut escaped);
                    builder.literal(&escaped);
                }
            }
            c => builder.push(c),
        }
    }
    if backslash {
        builder.push('\\');
    }
    Ok(builder.finish())
}

#[derive(Debug)]
pub enum Incomplete {
    Message(&'static str),
    SingleQuote,
    DoubleQuote,
    AnsiQuote,
    CasePattern,
    Subshell,
}

#[derive(Debug)]
pub enum Error {
    Incomplete(Incomplete),
    NulByte,
    ArithmeticClose,
    HereDoc(HereDocHint),
    Invalid(&'static str),
    ArrayToken,
}

impl Error {
    pub(crate) fn unfinished_function() -> Self {
        Self::Incomplete(Incomplete::Message("unfinished function definition"))
    }

    pub(crate) fn pending_quote(&self) -> Option<char> {
        match self {
            Self::Incomplete(Incomplete::SingleQuote) => Some('\''),
            Self::Incomplete(Incomplete::AnsiQuote) => Some('$'),
            Self::Incomplete(Incomplete::DoubleQuote) => Some('"'),
            _ => None,
        }
    }

    pub fn incomplete(&self) -> bool {
        matches!(self, Self::Incomplete(_) | Self::HereDoc(_))
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::Incomplete(Incomplete::Message(s)) | Self::Invalid(s) => s,
            Self::Incomplete(Incomplete::SingleQuote) => "unterminated single quote",
            Self::Incomplete(Incomplete::DoubleQuote) => "unterminated double quote",
            Self::Incomplete(Incomplete::AnsiQuote) => "unterminated dollar-single-quoted string",
            Self::Incomplete(Incomplete::CasePattern) => "expected ')' after case pattern",
            Self::Incomplete(Incomplete::Subshell) => "unfinished subshell",
            Self::NulByte => "NUL in command text",
            Self::ArithmeticClose => "invalid closing parenthesis in arithmetic expression",
            Self::HereDoc(_) => "unterminated here-document",
            Self::ArrayToken => "unexpected token in array assignment",
        }
    }
}

pub(crate) fn reserved(word: &str) -> bool {
    reserved_spelling(word).is_some()
}

fn reserved_spelling(word: &str) -> Option<&'static str> {
    Some(match word {
        "if" => "if",
        "then" => "then",
        "else" => "else",
        "elif" => "elif",
        "fi" => "fi",
        "case" => "case",
        "esac" => "esac",
        "while" => "while",
        "until" => "until",
        "for" => "for",
        "do" => "do",
        "done" => "done",
        "in" => "in",
        "select" => "select",
        "function" => "function",
        "coproc" => "coproc",
        "time" => "time",
        "!" => "!",
        "{" => "{",
        "[[" => "[[",
        "}" => "}",
        _ => return None,
    })
}

pub fn parse(source: &str) -> Result<Vec<Statement>, Error> {
    parse_source(source, false)
}

pub fn parse_final(source: &str) -> Result<Vec<Statement>, Error> {
    parse_source(source, true)
}

thread_local! {
    // Lexers invoked recursively by a substitution inherit the outer read's options.
    static FIRST_LINE: std::cell::Cell<usize> = const { std::cell::Cell::new(1) };
    static COMMENTS: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    static POSIX: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TIME_OPTIONS_EXTERNAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static EXTQUOTE: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    static EXTGLOB: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn session_source(
    source: &str,
    eof: bool,
    session: &crate::Session,
) -> Result<Vec<Statement>, Error> {
    session_source_at(source, eof, session, 1)
}

pub fn session_source_at(
    source: &str,
    eof: bool,
    session: &crate::Session,
    line: usize,
) -> Result<Vec<Statement>, Error> {
    if source.len() <= 4096
        && !session.reads_aliases()
        && let Some(statement) = literal_assignment(source, line)
    {
        return Ok(vec![statement]);
    }
    with_read_options(session, line, || {
        if eof {
            parse_final(source)
        } else {
            parse(source)
        }
    })
}

pub(crate) fn validate_tail(source: &str, session: &crate::Session, line: usize) -> bool {
    with_read_options(session, line, || {
        if source.contains('\0') {
            return false;
        }
        let Ok((tokens, spans, _)) = lex(source, false, false) else {
            return false;
        };
        Reader::new(tokens, spans, source)
            .read_list(&[], true)
            .is_ok()
    })
}

pub(crate) fn here_word(session: &crate::Session, source: &str) -> Result<Word, Error> {
    with_read_options(session, session.line(), || expansion_string(source))
}

// A subscript written in the script can contain its own command substitution.
// POSIX reads aliases with the surrounding command. Bash mode consults the
// live aliases when it expands the subscript.
pub(crate) fn with_subscript_aliases<T>(
    session: &crate::Session,
    source: &str,
    parse: impl FnOnce() -> T,
) -> T {
    let frozen = session
        .current_command
        .as_ref()
        .and_then(|s| s.subscript_aliases.0.as_ref());
    let live = if !session.posix && session.shopt.expand_aliases {
        session.aliases.0.as_ref()
    } else {
        None
    };
    let Some(table) = frozen.or(live) else {
        return parse();
    };
    let bytes = source.as_bytes();
    if !memchr::memchr2_iter(b'$', b'\x60', bytes).any(|i| {
        bytes[i] == b'\x60'
            || bytes.get(i + 1) == Some(&b'(')
            || (bytes.get(i + 1) == Some(&b'{') && lexer::starts_braced_command(&source[i + 2..]))
    }) {
        return parse();
    }
    // A function may be read in POSIX mode and called after switching back.
    // Names resolved by the earlier read take precedence over later aliases.
    let table = match (frozen, live) {
        (Some(first), Some(second)) if !Rc::ptr_eq(first, second) => {
            let mut combined = second.as_ref().clone();
            combined.extend(
                first
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
            Rc::new(combined)
        }
        _ => Rc::clone(table),
    };
    aliases::with(crate::alias::Aliases(Some(table)), parse)
}

fn with_read_options<T>(session: &crate::Session, line: usize, parse: impl FnOnce() -> T) -> T {
    let read = || {
        let _first_line = crate::ScopedCell::enter(&FIRST_LINE, line);
        let _extglob = crate::ScopedCell::enter(&EXTGLOB, session.shopt.glob.extglob);
        let _extquote = crate::ScopedCell::enter(&EXTQUOTE, session.shopt.extquote);
        let _posix = crate::ScopedCell::enter(&POSIX, session.posix);
        let _time_options_external = crate::ScopedCell::enter(
            &TIME_OPTIONS_EXTERNAL,
            session.posix && session.shopt.compat > 41,
        );
        let _comments = crate::ScopedCell::enter(&COMMENTS, session.read_comments());
        parse()
    };
    if session.reads_aliases() {
        aliases::with(session.aliases.clone(), read)
    } else {
        read()
    }
}

pub(crate) fn plain_assignment(command: &str) -> Option<(&str, &str)> {
    let (name, value) = command.split_once('=')?;
    (crate::word::is_identifier(name) && value.bytes().all(lexer::plain_byte))
        .then_some((name, value))
}

// Generated eval assignments often change just the value. Construct the same
// literal assignment as the full parser. Attributes, traps and status stay live.
pub(crate) fn literal_assignment(source: &str, line: usize) -> Option<Statement> {
    let command = source.strip_suffix('\n').unwrap_or(source);
    let (name, raw) = command.split_once('=')?;
    if !is_identifier(name) {
        return None;
    }
    let spelling = if raw.bytes().all(lexer::plain_byte) {
        command.to_owned()
    } else if !raw.contains(['\n', '\0']) {
        let value = if let Some(rest) = raw.strip_prefix("$'") {
            let mut chars = rest.chars();
            let decoded = ansi::quoted(&mut chars).ok()?;
            if !chars.as_str().is_empty() {
                return None;
            }
            decoded
        } else {
            let quoted = raw.strip_prefix('\'')?.strip_suffix('\'')?;
            if quoted.contains('\'') {
                return None;
            }
            quoted.to_owned()
        };
        [name, "=", &value].concat()
    } else {
        return None;
    };
    let location = Location {
        source: source.into(),
        subscript_aliases: Default::default(),
        start: 0,
        end: command.len(),
        line,
    };
    Some(Statement {
        location,
        condition: Condition::Always,
        negated: false,
        command: Command::LiteralAssignment(spelling, std::num::NonZeroUsize::new(name.len())?),
    })
}

fn parse_source(source: &str, final_input: bool) -> Result<Vec<Statement>, Error> {
    if source.contains('\0') {
        return Err(Error::NulByte);
    }
    if let Some(function) = wrapper::parse(source) {
        return Ok(vec![function]);
    }
    let (tokens, spans, _) = match lex(source, false, final_input) {
        Ok(tokens) => tokens,
        Err(error @ Error::Incomplete(_)) => {
            // An alias opening a quote can supply the missing left delimiter
            // for the following source. Only retry a literal command name.
            let aliases = aliases::current();
            let trimmed = source.trim_start_matches([' ', '\t']);
            let name = &trimmed[..lexer::plain_prefix(trimmed)];
            let Some(value) = aliases.get(name).filter(|_| {
                !name.is_empty() && trimmed[name.len()..].starts_with(lexer::separator)
            }) else {
                return Err(error);
            };
            let mut nested = aliases.clone();
            Rc::make_mut(nested.0.as_mut().unwrap()).remove(name);
            let replacement = aliases::with(nested.clone(), || lex(value, false, final_input));
            if !matches!(replacement, Err(Error::Incomplete(_))) {
                return Err(error);
            }
            let end = source.len() - trimmed.len() + name.len();
            let (tokens, spans, _) = lex(&source[..end], false, true)?;
            let mut reader = Reader::new(tokens, spans, source);
            reader.final_input = final_input;
            reader.joined_alias(name.into(), value, nested)?;
            return reader.list(&[]);
        }
        Err(error) => return Err(error),
    };
    let mut reader = Reader::new(tokens, spans, source);
    reader.final_input = final_input;
    reader.list(&[])
}

// Most shell reads are a few lines. Reuse their token and span allocations;
// nested parses take another pair while the outer reader owns its buffers.
const MAX_CACHED_TOKENS: usize = 4096;
type TokenBuffers = (Vec<Token>, Vec<(usize, usize)>);
thread_local! {
    static TOKEN_BUFFERS: std::cell::Cell<TokenBuffers> = const {
        std::cell::Cell::new((Vec::new(), Vec::new()))
    };
}

// During recursive lexing, a nested AST can start after the first line of
// the outer input. Store byte offsets only. No reference outlives the lex.
struct SourceLines {
    start: usize,
    end: usize,
    first: usize,
    newlines: Vec<usize>,
}

thread_local! {
    static READER_NEWLINES: std::cell::Cell<Vec<usize>> = const { std::cell::Cell::new(Vec::new()) };
    static SOURCE_LINES: std::cell::RefCell<Option<SourceLines>> = const { std::cell::RefCell::new(None) };
}

impl SourceLines {
    fn contains(&self, source: &str) -> bool {
        let start = source.as_ptr() as usize;
        self.start <= start && start + source.len() <= self.end
    }
}

fn source_line(source: &str) -> usize {
    SOURCE_LINES.with_borrow(|map| {
        map.as_ref().filter(|m| m.contains(source)).map_or_else(
            || FIRST_LINE.get(),
            |m| {
                let offset = source.as_ptr() as usize - m.start;
                m.first + m.newlines.partition_point(|&n| n < offset)
            },
        )
    })
}

fn with_source_lines<T>(source: &str, lex: impl FnOnce() -> T) -> T {
    // A trailing newline cannot move any nested command to another line.
    let text = source.trim_end_matches('\n');
    if !text.contains('\n')
        || SOURCE_LINES.with_borrow(|m| m.as_ref().is_some_and(|m| m.contains(source)))
        || !memchr::memchr3_iter(b'(', b'{', b'`', text.as_bytes()).any(|i| {
            // Nested lexing starts at a substitution. A physical newline can
            // also separate its introducer from the opener via a continuation.
            let prev = i.checked_sub(1).map(|p| text.as_bytes()[p]);
            match text.as_bytes()[i] {
                b'(' => matches!(prev, Some(b'$' | b'<' | b'>' | b'\n')),
                b'{' => matches!(prev, Some(b'$' | b'\n')),
                _ => true,
            }
        })
    {
        return lex();
    }
    let start = source.as_ptr() as usize;
    let previous = SOURCE_LINES.replace(Some(SourceLines {
        start,
        end: start + source.len(),
        first: FIRST_LINE.get(),
        newlines: memchr::memchr_iter(b'\n', source.as_bytes()).collect(),
    }));
    let result = lex();
    SOURCE_LINES.replace(previous);
    result
}

struct Reader {
    depth: usize,
    first_line: usize,
    source: Rc<str>,
    alias_source: Option<Box<aliases::Expanded>>,
    spans: Vec<(usize, usize)>,
    newlines: Vec<usize>,
    line_index: std::cell::Cell<usize>,
    aliases: crate::alias::Aliases,
    alias_frames: Vec<aliases::Frame>,
    final_input: bool,
    tokens: std::collections::VecDeque<Token>,
}

impl Drop for Reader {
    fn drop(&mut self) {
        if self.newlines.capacity() <= MAX_CACHED_TOKENS && self.newlines.capacity() != 0 {
            self.newlines.clear();
            READER_NEWLINES.set(std::mem::take(&mut self.newlines));
        }
        if self.tokens.capacity().max(self.spans.capacity()) <= MAX_CACHED_TOKENS {
            self.tokens.clear();
            self.spans.clear();
            TOKEN_BUFFERS.set((
                std::mem::take(&mut self.tokens).into(),
                std::mem::take(&mut self.spans),
            ));
        }
    }
}

fn scalar_assignment(name: String, mut word: Word) -> (String, Assignment<Word>) {
    word.remove_assignment_prefix(name.len() + 1);
    (name, Assignment::Scalar(word))
}

impl Reader {
    fn new(tokens: Vec<Token>, spans: Vec<(usize, usize)>, source: &str) -> Self {
        let mut newlines = READER_NEWLINES.take();
        newlines.extend(memchr::memchr_iter(b'\n', source.as_bytes()));
        Self {
            first_line: source_line(source),
            line_index: std::cell::Cell::new(0),
            depth: 0,
            spans,
            alias_source: None,
            newlines,
            source: source.into(),
            tokens: tokens.into(),
            aliases: aliases::current(),
            alias_frames: Vec::new(),
            final_input: false,
        }
    }

    fn nested<T>(&mut self, read: impl FnOnce(&mut Self) -> Result<T, Error>) -> Result<T, Error> {
        // Generated configure and eval scripts can legitimately nest hundreds of lists.
        if self.depth >= 512
            || self.depth >= 8 && self.depth.is_multiple_of(8) && crate::stack_budget_exceeded()
        {
            return Err(Error::Invalid("shell syntax nesting limit exceeded"));
        }
        self.depth += 1;
        let result = read(self);
        self.depth -= 1;
        result
    }

    fn position(&self) -> usize {
        let count = self
            .alias_source
            .as_ref()
            .map_or(self.spans.len(), |s| s.token_count(self.spans.len()));
        count - self.tokens.len()
    }
    fn subscript_aliases(&self) -> crate::alias::Aliases {
        if POSIX.get() {
            self.aliases.clone()
        } else {
            Default::default()
        }
    }

    fn location(&self, from: usize) -> Location {
        if let Some(expanded) = &self.alias_source {
            return expanded.location(self, from, self.position());
        }
        let start = self.spans.get(from).map_or(self.source.len(), |s| s.0);
        let position = self.spans.len() - self.tokens.len();
        let end = if position > from {
            self.spans[position - 1].1
        } else {
            start
        };
        let mut line = self.line_index.get();
        if line > 0 && self.newlines[line - 1] >= start {
            line = self.newlines[..line].partition_point(|&n| n < start);
        } else if self.newlines.get(line).is_some_and(|&n| n < start) {
            line += 1;
            if self.newlines.get(line).is_some_and(|&n| n < start) {
                line += self.newlines[line..].partition_point(|&n| n < start);
            }
        }
        self.line_index.set(line);
        Location {
            source: self.source.clone(),
            subscript_aliases: self.subscript_aliases(),
            start,
            end,
            line: self.first_line + line,
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.front()
    }

    fn keyword(&self) -> Option<&str> {
        self.peek()?.keyword()
    }

    fn operand(&mut self) -> Option<Token> {
        self.tokens.pop_front().map(Token::into_operand)
    }

    fn expect(&mut self, keyword: &str) -> Result<(), Error> {
        if self.keyword() == Some(keyword) {
            self.tokens.pop_front();
            Ok(())
        } else if self.peek().is_none() {
            Err(Error::Incomplete(Incomplete::Message(
                "unfinished compound command",
            )))
        } else {
            Err(Error::Invalid("unexpected token in compound command"))
        }
    }

    fn newlines(&mut self) {
        while matches!(self.peek(), Some(Token::Newline)) {
            self.tokens.pop_front();
        }
    }

    fn at_stop(&self, stops: &[&str]) -> bool {
        match self.peek() {
            Some(Token::RightParen) => stops.contains(&")"),
            Some(Token::EndArm(_)) => stops.contains(&";;"),
            _ => self.keyword().is_some_and(|s| stops.contains(&s)),
        }
    }

    fn list(&mut self, stops: &[&str]) -> Result<Vec<Statement>, Error> {
        self.nested(|r| r.read_list(stops, false)).map_err(|error| {
            // Never resume in the body of an invalid loop, conditional or function.
            if !stops.is_empty() && matches!(error, Error::ArrayToken) {
                Error::Invalid(error.message())
            } else {
                error
            }
        })
    }

    fn read_list(
        &mut self,
        stops: &[&str],
        discard_complete: bool,
    ) -> Result<Vec<Statement>, Error> {
        let mut statements = Vec::new();
        let mut condition = Condition::Always;
        let mut and_or_start = 0;
        loop {
            // A syntax-only top-level read needs the pending and/or chain for
            // a possible trailing ampersand. Every earlier command can drop.
            if discard_complete && condition == Condition::Always {
                statements.clear();
                and_or_start = 0;
            }
            self.newlines();
            if self.peek().is_none() {
                if condition != Condition::Always {
                    return Err(Error::Incomplete(Incomplete::Message(
                        "expected a command after operator",
                    )));
                }
                return Ok(statements);
            }
            self.expand_alias(true)?;
            if self.at_stop(stops) {
                if (statements.is_empty() && !stops.contains(&";;"))
                    || condition != Condition::Always
                {
                    return Err(Error::Invalid("expected a command"));
                }
                return Ok(statements);
            }
            if self.peek().is_none() && condition == Condition::Always {
                return Ok(statements);
            }
            if matches!(self.peek(), Some(Token::Newline)) {
                continue;
            }
            let mut inverted = None;
            let mut timing = None;
            while matches!(self.keyword(), Some("!" | "time")) {
                if self.keyword() == Some("time") {
                    self.tokens.pop_front();
                    let posix = matches!(self.keyword(), Some("-p" | "--"));
                    if self.keyword() == Some("-p") {
                        self.tokens.pop_front();
                    }
                    if self.keyword() == Some("--") {
                        self.tokens.pop_front();
                    }
                    timing = Some(posix);
                } else {
                    self.tokens.pop_front();
                    inverted = Some(!inverted.unwrap_or(false));
                }
                self.expand_alias(true)?;
            }
            let start = self.position();
            let command = if (inverted.is_some() || timing.is_some())
                && matches!(
                    self.peek(),
                    None | Some(Token::End | Token::Newline | Token::Background)
                ) {
                Command::Pipeline(vec![SimpleCommand::default()])
            } else {
                self.command()?
            };
            let command = if let Some(posix) = timing {
                Command::Timed {
                    posix,
                    command: Box::new(command),
                }
            } else {
                command
            };
            if statements.is_empty() {
                // Size a multi-command body once. Do not allocate several large
                // statements for the usual one-command condition or case arm.
                let mut rest = self.tokens.iter();
                let mut token = rest.next();
                while matches!(token, Some(Token::End | Token::Newline)) {
                    token = rest.next();
                }
                let more = token.is_some_and(|t| {
                    !matches!(t, Token::RightParen | Token::EndArm(_))
                        && !t.keyword().is_some_and(|s| stops.contains(&s))
                });
                statements.reserve_exact(if more { 3 } else { 1 });
            }
            statements.push(Statement {
                location: self.location(start),
                condition,
                negated: inverted.unwrap_or(false),
                command,
            });
            condition = match self.peek() {
                None => return Ok(statements),
                Some(Token::EndArm(_) | Token::RightParen) if self.at_stop(stops) => {
                    return Ok(statements);
                }
                Some(Token::End | Token::Newline) => {
                    and_or_start = statements.len();
                    Condition::Always
                }
                Some(Token::Background) => {
                    let group = statements.split_off(and_or_start);
                    statements.push(Statement {
                        location: group[0].location.clone(),
                        condition: Condition::Always,
                        negated: false,
                        command: Command::Background(group),
                    });
                    and_or_start = statements.len();
                    Condition::Always
                }
                Some(Token::And) => Condition::Success,
                Some(Token::Or) => Condition::Failure,
                _ if self.at_stop(stops) => return Ok(statements),
                _ => return Err(Error::Invalid("expected a command separator")),
            };
            self.tokens.pop_front();
        }
    }

    fn command(&mut self) -> Result<Command, Error> {
        // The lexer already identified the assignment. A following separator
        // rules out a command, redirection or compound array initializer.
        if matches!(self.peek(), Some(Token::Word(w)) if w.assignment.is_some_and(|len| !w.word.assignment_name(len).starts_with('[')))
            && matches!(self.tokens.get(1), None | Some(Token::Newline | Token::End))
        {
            let Some(Token::Word(LexWord {
                assignment: Some(name),
                word,
                ..
            })) = self.tokens.pop_front()
            else {
                unreachable!()
            };
            if let Word::Literal(text) = word {
                return Ok(Command::LiteralAssignment(text, name));
            }
            let mut word = word;
            let key = word.take_assignment_name(name);
            return Ok(Command::Scalar(Box::new((key, word))));
        }
        let start = self.position();
        let mut first = match self.compound()? {
            Some(compound) if !self.has_redirect() && !self.has_pipe() => return Ok(compound),
            Some(compound) => self.compound_stage(compound)?,
            None => self.simple()?,
        };
        if self.has_pipe() {
            first.location = Some(Box::new(self.location(start)));
        }
        let mut pipeline = vec![first];
        while self.has_pipe() {
            if matches!(self.tokens.pop_front(), Some(Token::PipeBoth)) {
                pipeline.last_mut().unwrap().redirects.push(Redirect {
                    line: self.location(self.position().saturating_sub(1)).line,
                    variable: None,
                    fd: 2,
                    target: Target::Duplicate,
                    word: Word::Literal("1".into()),
                });
            }
            loop {
                self.newlines();
                self.expand_alias(true)?;
                if !matches!(self.peek(), Some(Token::Newline)) {
                    break;
                }
            }
            if self.peek().is_none() {
                return Err(Error::Incomplete(Incomplete::Message(
                    "expected a command after pipe",
                )));
            }
            let start = self.position();
            let mut stage = match self.compound()? {
                Some(compound) => self.compound_stage(compound)?,
                None => self.simple()?,
            };
            stage.location = Some(Box::new(self.location(start)));
            pipeline.push(stage);
        }
        Ok(Command::Pipeline(pipeline))
    }

    fn has_redirect(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token::DescriptorName(_) | Token::Redirect(..) | Token::BothRedirect(_))
        )
    }

    fn has_pipe(&self) -> bool {
        matches!(self.peek(), Some(Token::Pipe | Token::PipeBoth))
    }

    fn compound_stage(&mut self, compound: Command) -> Result<SimpleCommand<Word>, Error> {
        let mut stage = SimpleCommand {
            compound: Some(Box::new(compound)),
            ..Default::default()
        };
        while self.has_redirect() {
            let token = self.tokens.pop_front().unwrap();
            self.redirection(token, &mut stage.redirects)?;
        }
        Ok(stage)
    }

    fn compound(&mut self) -> Result<Option<Command>, Error> {
        if matches!(self.peek(), Some(Token::Arithmetic(_))) {
            let Some(Token::Arithmetic(expr)) = self.tokens.pop_front() else {
                unreachable!()
            };
            return Ok(Some(Command::Arithmetic(Box::new(expr.into()))));
        }
        if matches!(self.peek(), Some(Token::LeftParen)) {
            self.tokens.pop_front();
            let body = self.list(&[")"])?;
            if self.tokens.pop_front().is_none() {
                return Err(Error::Incomplete(Incomplete::Subshell));
            }
            return Ok(Some(Command::Subshell(body)));
        }
        self.alias_after_name()?;
        match self.keyword() {
            Some("{") => {
                self.tokens.pop_front();
                let body = self.list(&["}"])?;
                self.expect("}")?;
                Ok(Command::Group(body))
            }
            Some("[[") => {
                self.tokens.pop_front();
                let expr = self.test_or()?;
                self.newlines();
                self.expect("]]")?;
                Ok(Command::Test(Box::new(expr)))
            }
            Some("coproc") => self.coproc(),
            Some("case") => self.case_clause(),
            Some("if") => self.if_clause(),
            Some("while" | "until") => self.while_clause(),
            Some("for" | "select") => self.for_clause(),
            Some("function") => self.function(true),
            _ if matches!(self.peek(), Some(Token::Word(_)))
                && matches!(self.tokens.get(1), Some(Token::LeftParen)) =>
            {
                self.function(false)
            }
            _ => return Ok(None),
        }
        .map(Some)
    }

    fn coproc(&mut self) -> Result<Command, Error> {
        self.tokens.pop_front();
        let named = match self.tokens.get(1) {
            Some(Token::LeftParen | Token::Arithmetic(_)) => true,
            Some(w) => matches!(
                w.keyword(),
                Some("{" | "[[" | "if" | "case" | "while" | "until" | "for" | "select")
            ),
            _ => false,
        };
        let name = if named && self.keyword().is_some_and(is_identifier) {
            self.name()?
        } else {
            String::from("COPROC")
        };
        if self.peek().is_none() {
            return Err(Error::Incomplete(Incomplete::Message(
                "expected a coprocess command",
            )));
        }
        let command = if let Some(compound) = self.nested(Self::compound)? {
            if self.has_redirect() {
                Command::Pipeline(vec![self.compound_stage(compound)?])
            } else {
                compound
            }
        } else {
            Command::Pipeline(vec![self.simple()?])
        };
        Ok(Command::Coprocess {
            name,
            body: Box::new(command),
        })
    }

    fn case_clause(&mut self) -> Result<Command, Error> {
        let start = self.position();
        self.tokens.pop_front();
        self.alias_at(false, Some(&[]))?;
        let word = match self.operand() {
            Some(Token::Word(word)) => word.word,
            None => {
                return Err(Error::Incomplete(Incomplete::Message(
                    "expected a case word",
                )));
            }
            _ => return Err(Error::Invalid("expected a case word")),
        };
        self.alias_newlines(&["in"])?;
        self.expect("in")?;
        let header = self.location(start);
        self.newlines();
        let mut arms = Vec::new();
        loop {
            self.alias_newlines(&["esac"])?;
            if self.keyword() == Some("esac") {
                break;
            }
            if matches!(self.peek(), Some(Token::LeftParen)) {
                self.tokens.pop_front();
            }
            let mut patterns = Vec::new();
            loop {
                self.alias_at(false, Some(&[]))?;
                match self.operand() {
                    Some(Token::Word(word)) => patterns.push(CasePattern::new(word.word)),
                    None => {
                        return Err(Error::Incomplete(Incomplete::Message(
                            "unfinished case command",
                        )));
                    }
                    _ => return Err(Error::Invalid("expected a case pattern")),
                }
                self.alias_at(false, Some(&[]))?;
                match self.tokens.pop_front() {
                    Some(Token::RightParen) => break,
                    Some(Token::Pipe) => {}
                    None => return Err(Error::Incomplete(Incomplete::CasePattern)),
                    _ => return Err(Error::Invalid("expected ')' after case pattern")),
                }
            }
            let body = self.list(&[";;", "esac"])?;
            let end = if let Some(Token::EndArm(end)) = self.peek() {
                let end = *end;
                self.tokens.pop_front();
                end
            } else {
                CaseEnd::Stop
            };
            arms.push((patterns, body, end));
            self.newlines();
        }
        self.expect("esac")?;
        Ok(Command::Case(Rc::new(CaseClause { header, word, arms })))
    }

    fn if_clause(&mut self) -> Result<Command, Error> {
        self.tokens.pop_front();
        let mut branches = Vec::with_capacity(1);
        loop {
            let condition = self.list(&["then"])?;
            self.expect("then")?;
            let body = self.list(&["else", "elif", "fi"])?;
            branches.push((condition, body));
            if self.keyword() != Some("elif") {
                break;
            }
            self.tokens.pop_front();
        }
        let otherwise = if self.keyword() == Some("else") {
            self.tokens.pop_front();
            self.list(&["fi"])?
        } else {
            Vec::new()
        };
        self.expect("fi")?;
        Ok(Command::If {
            branches,
            otherwise,
        })
    }

    fn while_clause(&mut self) -> Result<Command, Error> {
        let until = self.keyword() == Some("until");
        self.tokens.pop_front();
        let condition = self.list(&["do"])?;
        self.expect("do")?;
        let body = self.list(&["done"])?;
        self.expect("done")?;
        Ok(Command::While {
            until,
            condition,
            body,
        })
    }

    fn name(&mut self) -> Result<String, Error> {
        match self.operand() {
            Some(Token::Word(LexWord {
                word: Word::Literal(name),
                protected: false,
                ..
            })) if is_identifier(&name) => Ok(name),
            None => Err(Error::Incomplete(Incomplete::Message(
                "expected an identifier",
            ))),
            _ => Err(Error::Invalid("expected an identifier")),
        }
    }

    fn for_clause(&mut self) -> Result<Command, Error> {
        let select = self.keyword() == Some("select");
        let start = self.position();
        self.tokens.pop_front();
        if !select && matches!(self.peek(), Some(Token::Arithmetic(_))) {
            let Some(Token::Arithmetic(header)) = self.tokens.pop_front() else {
                unreachable!()
            };
            let [init, condition, step] = lexer::arithmetic_clauses(&header)?;
            if matches!(self.peek(), Some(Token::End)) {
                self.tokens.pop_front();
            }
            self.newlines();
            let (open, close) = if self.keyword() == Some("{") {
                ("{", "}")
            } else {
                ("do", "done")
            };
            self.expect(open)?;
            let body = self.list(&[close])?;
            self.expect(close)?;
            return Ok(Command::ForArithmetic(Box::new(ArithmeticForClause {
                init: init.into(),
                condition: condition.into(),
                step: step.into(),
                body,
            })));
        }
        self.alias_at(false, Some(&[]))?;
        let name = self.declaration_name(
            self.position(),
            "expected a loop variable",
            "expected a loop variable",
        )?;
        self.alias_newlines(&["in", "do"])?;
        let words = if self.keyword() == Some("in") {
            self.tokens.pop_front();
            let mut words = Vec::new();
            loop {
                self.expand_alias(false)?;
                if !matches!(
                    self.peek(),
                    Some(Token::Word(_) | Token::Reserved(_) | Token::TimeUtility)
                ) {
                    break;
                }
                if let Some(Token::Word(word)) = self.operand() {
                    words.push(word.word);
                }
            }
            Some(words)
        } else {
            None
        };
        let header = self.location(start);
        if matches!(self.peek(), Some(Token::End)) {
            self.tokens.pop_front();
        }
        self.alias_newlines(&["do"])?;
        self.expect("do")?;
        let body = self.list(&["done"])?;
        self.expect("done")?;
        Ok(Command::For(Box::new(ForClause {
            select,
            header,
            name,
            words,
            body,
        })))
    }

    // Quotes in a declaration name retain their source spelling. A plain word
    // already owns the exact string the AST needs.
    fn declaration_name(
        &mut self,
        start: usize,
        missing: &'static str,
        invalid: &'static str,
    ) -> Result<String, Error> {
        match self.operand() {
            Some(Token::Word(LexWord {
                word: Word::Literal(name),
                protected: false,
                ..
            })) => Ok(name),
            Some(Token::Word(_)) => Ok(self.location(start).text().into()),
            None => Err(Error::Incomplete(Incomplete::Message(missing))),
            _ => Err(Error::Invalid(invalid)),
        }
    }

    fn function(&mut self, keyword: bool) -> Result<Command, Error> {
        if keyword {
            self.tokens.pop_front();
        }
        let start_name = self.position();
        let name = self.declaration_name(
            start_name,
            "expected a function name",
            "invalid function name",
        )?;
        if matches!(self.peek(), Some(Token::LeftParen)) {
            self.tokens.pop_front();
            self.alias_at(false, Some(&[]))?;
            match self.tokens.pop_front() {
                Some(Token::RightParen) => {}
                None => {
                    return Err(Error::unfinished_function());
                }
                _ => return Err(Error::Invalid("expected empty function parentheses")),
            }
        }
        self.alias_newlines(&["{", "if", "while", "until", "for", "case"])?;
        let start = self.position();
        let compound = self.nested(Self::compound)?.ok_or_else(|| {
            if self.peek().is_none() {
                Error::Incomplete(Incomplete::Message("expected a function body"))
            } else {
                Error::Invalid("expected a compound function body")
            }
        })?;
        if matches!(compound, Command::Function { .. }) {
            return Err(Error::Invalid("expected a compound function body"));
        }
        let body = if self.has_redirect() {
            // Function redirections are applied when the function is invoked.
            vec![Statement {
                location: self.location(start),
                condition: Condition::Always,
                negated: false,
                command: Command::Pipeline(vec![self.compound_stage(compound)?]),
            }]
        } else if let Command::Group(body) = compound {
            body
        } else {
            vec![Statement {
                location: self.location(start),
                condition: Condition::Always,
                negated: false,
                command: compound,
            }]
        };
        let location = self.location(start);
        let declaration_line = if self.alias_source.is_none() {
            let header = &self.source.as_bytes()[self.spans[start_name].0..location.start];
            location.line - memchr::memchr_iter(b'\n', header).count()
        } else {
            self.location(start_name).line
        };
        Ok(Command::Function {
            name,
            definition: Rc::new(FunctionDefinition {
                rendered_body: Default::default(),
                declaration_line,
                body: wrapper::FunctionBody::Parsed(body),
                location,
            }),
        })
    }

    fn redirection(
        &mut self,
        token: Token,
        redirects: &mut Vec<Redirect<Word>>,
    ) -> Result<(), Error> {
        let line = self.location(self.position().saturating_sub(1)).line;
        if matches!(token, Token::Redirect(..) | Token::BothRedirect(_)) {
            self.alias_at(false, Some(&[]))?;
        }
        match token {
            Token::DescriptorName(name) => {
                let first = redirects.len();
                let token = self
                    .tokens
                    .pop_front()
                    .ok_or(Error::Invalid("expected a redirection"))?;
                self.redirection(token, redirects)?;
                redirects[first].variable = Some(name.into());
            }
            Token::BothRedirect(target) => {
                let Some(Token::Word(LexWord { word, .. })) = self.operand() else {
                    return Err(Error::Invalid("expected a redirection target"));
                };
                redirects.push(Redirect {
                    line,
                    variable: None,
                    fd: 1,
                    target,
                    word,
                });
                redirects.push(Redirect {
                    line,
                    variable: None,
                    fd: 2,
                    target: Target::Duplicate,
                    word: Word::Literal("1".into()),
                });
            }
            Token::Redirect(fd, target) => {
                let Some(Token::Word(LexWord { word, .. })) = self.operand() else {
                    return Err(Error::Invalid("expected a redirection target"));
                };
                let target = if target == Target::OutputWord
                    && word.as_literal().is_some_and(|v| {
                        v == "-" || lexer::descriptor(v.strip_suffix('-').unwrap_or(v)).is_ok()
                    }) {
                    Target::Duplicate
                } else {
                    target
                };
                redirects.push(Redirect {
                    line,
                    variable: None,
                    fd,
                    target,
                    word,
                });
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    fn array_word_suffix(&mut self) -> bool {
        if !matches!(
            self.peek(),
            Some(Token::Word(_) | Token::Reserved(_) | Token::TimeUtility)
        ) {
            return false;
        }
        let end = self.position();
        let adjacent = if let Some(expanded) = &self.alias_source {
            expanded.span(end - 1, self).1 == expanded.span(end, self).0
        } else {
            self.spans[end - 1].1 == self.spans[end].0
        };
        if adjacent {
            self.tokens.pop_front();
        }
        adjacent
    }

    fn simple(&mut self) -> Result<SimpleCommand<Word>, Error> {
        let mut cmd = SimpleCommand::default();
        loop {
            self.expand_alias(cmd.words.is_empty())?;
            if !matches!(
                self.peek(),
                Some(
                    Token::Word(_)
                        | Token::Reserved(_)
                        | Token::TimeUtility
                        | Token::DescriptorName(_)
                        | Token::Redirect(_, _)
                        | Token::BothRedirect(_)
                )
            ) {
                break;
            }
            match self.operand().unwrap() {
                Token::Word(LexWord {
                    mut word,
                    protected,
                    assignment,
                }) => {
                    let assignment = assignment.map(|length| word.assignment_name(length));
                    let declaration = assignment.is_some()
                        && matches!(
                            cmd.words
                                .iter()
                                .map(Word::as_literal)
                                .find(|w| *w != Some("command"))
                                .flatten(),
                            Some("export" | "local" | "declare" | "typeset" | "readonly")
                        );
                    if matches!(self.peek(), Some(Token::ArrayStart)) {
                        let name = assignment
                            .ok_or(Error::Invalid("invalid array assignment"))?
                            .to_owned();
                        let base = name.trim_end_matches('+');
                        if let Some(builtin @ ("let" | "eval")) =
                            cmd.words.first().and_then(Word::as_literal)
                        {
                            let start = self.position() - 1;
                            self.array_items::<false>()?;
                            self.array_word_suffix();
                            let expr = self.location(start);
                            cmd.words.push(
                                lexer::subscript_word(expr.text(), builtin == "eval")?
                                    .declaration(),
                            );
                            continue;
                        }
                        if !(is_identifier(base)
                            || crate::session::array::subscript(base).is_some())
                            || !(cmd.words.is_empty() || declaration)
                            || cmd.words.first().and_then(Word::as_literal) == Some("command")
                        {
                            return Err(Error::Invalid("invalid array assignment"));
                        }
                        let source_start = self.position() - 1;
                        let items = self.array_items::<true>()?;
                        if self.array_word_suffix() {
                            let expr = self.location(source_start);
                            if declaration {
                                cmd.words
                                    .push(lexer::subscript_word(expr.text(), true)?.declaration());
                            } else {
                                let value = &expr.text()[name.len() + 1..];
                                cmd.assignments.push((
                                    name,
                                    Assignment::Scalar(lexer::subscript_word(value, true)?),
                                ));
                            }
                            continue;
                        }
                        let operand = declaration.then_some(cmd.words.len());
                        if declaration {
                            cmd.words
                                .push(Word::Literal(name.trim_end_matches('+').into()));
                        }
                        cmd.assignments.push((
                            name,
                            Assignment::Array(operand, items, self.location(source_start)),
                        ));
                        continue;
                    }
                    if cmd.words.is_empty() {
                        if let Some(name) = assignment.filter(|name| !name.starts_with('[')) {
                            if cmd.assignments.is_empty() {
                                cmd.assignments.reserve_exact(1);
                            }
                            cmd.assignments
                                .push(scalar_assignment(name.to_owned(), word));
                            continue;
                        }
                        if cmd.assignments.is_empty()
                            && !protected
                            && word.as_literal().is_some_and(reserved)
                        {
                            return Err(Error::Invalid("unexpected reserved word"));
                        }
                        // A bounded lookahead sizes the argument vector for short commands.
                        let args = self
                            .tokens
                            .iter()
                            .take(8)
                            .take_while(|t| matches!(t, Token::Word(_)))
                            .count();
                        cmd.words.reserve_exact(1 + args);
                    } else if let Some(name) = assignment {
                        cmd.keyword_args.record(cmd.words.len(), name);
                        if declaration {
                            word = word.declaration();
                        }
                    } else if let Word::Expanded(parts) = &word
                        && matches!(parts.first(), Some(crate::word::Part::Text(name)) if crate::word::is_identifier(name))
                        && matches!(parts.get(1), Some(crate::word::Part::Pattern(p)) if p.starts_with('['))
                        && let Some(name) =
                            array::indexed_prefix(self.location(self.position() - 1).text())
                    {
                        cmd.keyword_args.record(cmd.words.len(), name);
                    }
                    cmd.words.push(word);
                }
                token => self.redirection(token, &mut cmd.redirects)?,
            }
        }
        if cmd.empty() {
            Err(Error::Invalid("expected a command"))
        } else {
            Ok(cmd)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Statement {
        fn pipeline(self) -> Pipeline<Word> {
            let Command::Pipeline(stages) = self.command else {
                panic!("expected pipeline")
            };
            stages
        }
    }

    fn literals(command: &SimpleCommand<Word>) -> Vec<&str> {
        command
            .words
            .iter()
            .map(|w| w.as_literal().unwrap())
            .collect()
    }

    #[test]
    fn carries_quotes_empty_arguments_and_utf8_through_lexing() {
        let parsed = parse(r#"printf '' "こ"'ん'\ にちは "#).unwrap();
        assert_eq!(
            literals(&parsed[0].clone().pipeline()[0]),
            ["printf", "", "こん にちは"]
        );
    }

    #[test]
    fn single_quoted_spans_keep_metacharacters_and_newlines_literal() {
        let source = "run pre'世界\n# $HOME \\` *'post # ignored\nnext";
        let parsed = parse(source).unwrap();
        assert_eq!(
            literals(&parsed[0].clone().pipeline()[0]),
            ["run", "pre世界\n# $HOME \\` *post"]
        );
        assert_eq!(literals(&parsed[1].clone().pipeline()[0]), ["next"]);
    }

    #[test]
    fn recognizes_only_unquoted_adjacent_io_numbers() {
        let parsed = parse(r#"cmd 2>log 2 >out '2'>last 1>&2 9>&21"#).unwrap();
        let cmd = &parsed[0].clone().pipeline()[0];
        assert_eq!(literals(cmd), ["cmd", "2", "2"]);
        let fds: Vec<_> = cmd.redirects.iter().map(|r| r.fd).collect();
        assert_eq!(fds, [2, 1, 1, 1, 9]);
        assert_eq!(cmd.redirects[3].target, Target::Duplicate);
        let parsed = parse("cmd 2147483648>&2").unwrap();
        let cmd = &parsed[0].clone().pipeline()[0];
        assert_eq!(literals(cmd), ["cmd", "2147483648"]);
        assert_eq!(cmd.redirects[0].fd, 1);
        assert_eq!(cmd.redirects[0].target, Target::Duplicate);
    }

    #[test]
    fn distinguishes_continuation_from_invalid_input() {
        for source in ["cmd |\n", "cmd '", "cmd \\\n"] {
            assert!(matches!(parse(source), Err(Error::Incomplete(_))));
        }
        for source in ["| cmd", "cmd || ; cmd", "cmd > "] {
            assert!(matches!(parse(source), Err(Error::Invalid(_))));
        }
        let parsed = parse("cmd |\n# comment\ncat; next\n").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].clone().pipeline().len(), 2);
    }

    #[test]
    fn keeps_equal_precedence_for_conditional_lists_and_accepts_continuations() {
        let program = parse("false &&\n echo skip || echo yes; echo done").unwrap();
        let guards: Vec<_> = program.into_iter().map(|s| s.condition).collect();
        assert_eq!(
            guards,
            [
                Condition::Always,
                Condition::Success,
                Condition::Failure,
                Condition::Always
            ]
        );
        assert!(matches!(parse("true ||\n"), Err(Error::Incomplete(_))));
        assert!(matches!(parse("true && || false"), Err(Error::Invalid(_))));
    }

    #[test]
    fn assignment_markers_require_unquoted_names_and_equal_signs() {
        let stmt = parse(r#"x="$val" 'x'=other x=tail"#).unwrap().remove(0);
        let cmd = stmt.pipeline().remove(0);
        let (name, value) = cmd.assignments.into_iter().next().unwrap();
        assert_eq!(name, "x");
        let Assignment::Scalar(value) = value else {
            panic!("expected scalar")
        };
        assert_eq!(value.scalar(&|_| Some("a b".into()), &[]), "a b");
        assert_eq!(cmd.words[0].as_literal(), Some("x=other"));
        assert_eq!(cmd.words[1].as_literal(), Some("x=tail"));
    }

    #[test]
    fn preserves_field_boundaries_only_for_unquoted_expansion_delimiters() {
        let cmd = parse(r#"emit lhs${x}rhs "$x" '$x' $missing ""$missing $x"" "#)
            .unwrap()
            .pop()
            .unwrap()
            .pipeline()
            .pop()
            .unwrap();
        let lookup = |key: &str| match key {
            "x" => Some(" a  b ".into()),
            _ => None,
        };
        let fields: Vec<_> = cmd
            .words
            .into_iter()
            .flat_map(|w| w.fields(&lookup, &[]))
            .collect();
        assert_eq!(
            fields,
            [
                "emit", "lhs", "a", "b", "rhs", " a  b ", "$x", "", "a", "b", ""
            ]
        );
    }

    #[test]
    fn nonwhitespace_ifs_and_quoted_positional_arguments() {
        let cmd = parse(r#"emit $x "pre$@post" "$*" "$#" "${10}""#)
            .unwrap()
            .pop()
            .unwrap()
            .pipeline()
            .pop()
            .unwrap();
        let lookup = |key: &str| match key {
            "x" => Some(" :a :: b: ".into()),
            "IFS" => Some(": ".into()),
            _ => None,
        };
        let args = ["a".into(), "".into(), "b c".into()];
        let fields: Vec<_> = cmd
            .words
            .into_iter()
            .flat_map(|w| w.fields(&lookup, &args))
            .collect();
        assert_eq!(
            fields,
            [
                "emit", "", "a", "", "b", "prea", "", "b cpost", "a::b c", "3", ""
            ]
        );
        let command = parse(r#"emit "$@" "q$@""#)
            .unwrap()
            .pop()
            .unwrap()
            .pipeline()
            .pop()
            .unwrap();
        let empty: Vec<_> = command
            .words
            .into_iter()
            .flat_map(|w| w.fields(&lookup, &[]))
            .collect();
        assert_eq!(empty, ["emit", "q"]);
    }

    #[test]
    fn alternatives_distinguish_unset_empty_and_quoted_default_words() {
        let source = r#"emit "${missing:-${x:+used}}" "${empty-default}" "${empty:-default}" ${missing:-a b} ${missing:-"a b"} ${x:+} "${missing:+word}""#;
        let words = parse(source)
            .unwrap()
            .pop()
            .unwrap()
            .pipeline()
            .pop()
            .unwrap()
            .words;
        let lookup = |name: &str| match name {
            "x" => Some("y".into()),
            "empty" => Some(String::new()),
            _ => None,
        };
        let values: Vec<_> = words
            .into_iter()
            .flat_map(|w| w.fields(&lookup, &[]))
            .collect();
        assert_eq!(values, ["emit", "used", "", "default", "a", "b", "a b", ""]);
    }

    #[test]
    fn compounds_nest_and_preserve_commands_that_look_like_keywords() {
        let source = "f() { for x in a 'do'; do if false; then echo else; elif true; then echo \"$x\"; else :; fi; done; }; function g { while true; do :; done; }";
        let program = parse(source).unwrap();
        let Command::Function { name, definition } = &program[0].command else {
            panic!()
        };
        assert_eq!(name, "f");
        let Command::For(clause) = &definition.body.statements()[0].command else {
            panic!()
        };
        let Some(words) = &clause.words else { panic!() };
        let body = &clause.body;
        assert_eq!(words[1].as_literal(), Some("do"));
        let Command::If {
            branches,
            otherwise,
        } = &body[0].command
        else {
            panic!()
        };
        assert_eq!((branches.len(), otherwise.len()), (2, 1));
        let Command::Function { name, definition } = &program[1].command else {
            panic!()
        };
        assert_eq!(name, "g");
        assert!(matches!(
            definition.body.statements()[0].command,
            Command::While { until: false, .. }
        ));
        for incomplete in ["if true; then echo ok\n", "for i in a; do\n", "fn() {\n"] {
            assert!(matches!(parse(incomplete), Err(Error::Incomplete(_))));
        }
    }

    #[test]
    fn differentiates_substitutions_from_literal_parentheses_and_skips_inactive_branches() {
        let source = r#"cmd "$(printf ')')" `printf x` "${set:-$(unused)}" '$(literal)' "#;
        let command = parse(source).unwrap().remove(0).pipeline().remove(0);
        let mut session = crate::Session::default();
        session.assign("set", "ok");
        let captured = std::cell::Cell::new(None);
        let mut env = crate::expansion::Expansion::new(&mut session, 0, &captured);
        let args: Vec<_> = command
            .words
            .into_iter()
            .flat_map(|word| env.fields(word).unwrap())
            .collect();
        assert_eq!(args, ["cmd", ")", "x", "ok", "$(literal)"]);
        assert!(matches!(parse("cmd $(echo x\n"), Err(Error::Incomplete(_))));
        assert!(parse("cmd $(echo x | | cat)").is_err());
    }

    #[test]
    fn case_arms_grouping_negation_and_nested_substitutions() {
        let script = r#"! (false || { :; }); case "$x" in (a|b) echo one;; 'esac') ;; *) :; esac"#;
        let program = parse(script).unwrap();
        assert!(program[0].negated);
        assert!(matches!(program[0].command, Command::Subshell(_)));
        let Command::Case(clause) = &program[1].command else {
            panic!()
        };
        assert_eq!(clause.arms[0].0.len(), 2);
        assert!(clause.arms[1].1.is_empty());
        assert!(parse(r#"echo "$(case a in a) echo ok;; esac)" "#).is_ok());
        assert!(matches!(parse("case x in name"), Err(Error::Incomplete(_))));
        assert!(parse("{ :; } (echo invalid)").is_err());
    }

    #[test]
    fn background_groups_an_entire_and_or_list() {
        let program = parse("echo start; false && echo skip || echo ok & echo foreground").unwrap();
        assert_eq!(program.len(), 3);
        let Command::Background(body) = &program[1].command else {
            panic!()
        };
        assert_eq!(body.len(), 3);
        assert_eq!(body[2].condition, Condition::Failure);
        let stages = parse("emit 2> prior |& cat &>>both")
            .unwrap()
            .remove(0)
            .pipeline();
        assert_eq!(stages[0].redirects[0].target, Target::Write);
        assert_eq!(stages[0].redirects[1].target, Target::Duplicate);
        assert_eq!(stages[1].redirects[0].target, Target::Append);
    }

    #[test]
    fn process_substitutions_are_words_and_preserve_adjacent_text() {
        use crate::word::Part;
        let Command::Pipeline(mut pipeline) =
            parse(r#"emit pre<(printf data)post ">(literal)" > >(read x)"#)
                .unwrap()
                .remove(0)
                .command
        else {
            panic!()
        };
        let cmd = pipeline.remove(0);
        let Word::Expanded(parts) = &cmd.words[1] else {
            panic!()
        };
        assert!(
            matches!(parts.as_slice(),[Part::Text(a),Part::Process{input:true,..},Part::Text(b)] if a=="pre" && b=="post")
        );
        assert_eq!(cmd.words[2].as_literal(), Some(">(literal)"));
        let Word::Expanded(parts) = &cmd.redirects[0].word else {
            panic!()
        };
        assert!(matches!(
            parts.as_slice(),
            [Part::Process { input: false, .. }]
        ));
        assert!(matches!(
            parse("emit <(unfinished"),
            Err(Error::Incomplete(_))
        ));
    }

    #[test]
    fn conditional_syntax_and_arithmetic_commands() {
        let program=parse(r#"[[ -n "$x" && ( "$x" == a* || "$x" =~ ^(a|b)[0-9]+$ ) ]]; for ((i=0;i<3;i++)); do ((sum+=i)); done"#).unwrap();
        assert!(
            matches!(&program[0].command, Command::Test(expr) if matches!(**expr, Expr::And(..)))
        );
        assert!(matches!(program[1].command, Command::ForArithmetic(_)));
        assert!(parse("[[ x && ]]").is_err());
        assert!(matches!(parse("[[ x =="), Err(Error::Incomplete(_))));
    }
}
