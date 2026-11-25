mod aliases;
mod array;
use crate::pattern::Matcher;
use crate::test_expression::Expr;
pub use array::Assignment;
mod ansi;
pub(crate) use ansi::decode as ansi_string;
pub(crate) mod completion_words;
mod here_doc;
pub(crate) use lexer::{arithmetic_word, subscript_word};
mod lexer;
mod modifier;
mod wrapper;
pub use crate::word::{Word, is_identifier};
pub(crate) use wrapper::opens_function;

pub(crate) fn history_word_spans(s: &str) -> Vec<(usize, usize)> {
    let mut words = Vec::new();
    let mut start = None;
    let mut quote = None;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if !escaped && quote.is_none() && c.is_whitespace() {
            if let Some(a) = start.take() {
                words.push((a, i));
            }
            continue;
        }
        start.get_or_insert(i);
        if escaped {
            escaped = false;
        } else if c == '\\' && quote != Some('\'') {
            escaped = true;
        } else if quote == Some(c) {
            quote = None;
        } else if quote.is_none() && matches!(c, '\'' | '"') {
            quote = Some(c);
        }
    }
    if let Some(a) = start {
        words.push((a, s.len()));
    }
    words
}

/// Keep the original shell spelling, including quotes and substitutions.
pub(crate) fn last_history_argument(line: &str) -> Option<&str> {
    let blank = [' ', '\t', '\n'];
    let trimmed = line.trim_end_matches(blank);
    if trimmed.is_empty() {
        return None;
    }

    // A bare prefix cannot leave the last argument inside an open quote or expansion.
    let first_special = trimmed.bytes().position(|b| {
        !(lexer::plain_byte(b)
            || b >= 128
            || matches!(b, b' ' | b'\t' | b'\n' | b'=' | b',' | b'@' | b'%'))
    });
    let prefix = match first_special {
        None => return trimmed.rsplit(blank).next(),
        Some(i) => &trimmed[..i],
    };
    let tail = &trimmed.as_bytes()[prefix.len()..];
    if tail.len() >= 2
        && tail[0] == b'\''
        && memchr::memchr(b'\'', &tail[1..]) == Some(tail.len() - 2)
    {
        let start = prefix.rfind(blank).map_or(0, |i| i + 1);
        return Some(&trimmed[start..]);
    }
    // Pass the untrimmed line to the lexer: an escaped trailing blank belongs to the word.
    let (tokens, spans, _) = match lex(line, false, true) {
        Ok(parts) => parts,
        Err(_) => return history_word_spans(line).last().map(|&(a, b)| &line[a..b]),
    };
    let index = tokens
        .iter()
        .zip(&spans)
        .rposition(|(t, &(a, b))| !matches!(t, Token::Newline) || b - a > 1)?;
    if matches!(tokens[index], Token::Newline) {
        // A newline spanning a heredoc includes its closing delimiter.
        let (a, b) = spans[index];
        let closing = line[a..b].trim_end_matches('\n').rsplit('\n').next()?;
        return last_history_argument(closing);
    }
    let (mut start, end) = spans[index];
    // The history word for an adjacent fd duplication includes its operator.
    if index > 0
        && spans[index - 1].1 == start
        && matches!(
            tokens[index - 1],
            Token::Redirect(_, Target::Duplicate | Target::OutputWord)
        )
        && line[start..end]
            .trim_end_matches('-')
            .bytes()
            .all(|b| b.is_ascii_digit())
    {
        start = spans[index - 1].0;
    }
    Some(&line[start..end])
}

/// Byte end of the next shell token in the displayed command, including any
/// whitespace before it. Lexing identifies quotes and substitutions without running them.
pub(crate) fn suggestion_end(line: &str, cursor: usize) -> usize {
    // A plain prefix cannot leave us inside quotes or a substitution. Stop
    // at the next blank without lexing later arguments of a long suggestion.
    let bare = |b: u8| b.is_ascii_alphanumeric() || b >= 128 || b"-_./:=,@%+".contains(&b);
    let tail = line[cursor..].trim_start_matches([' ', '\t']);
    let word = tail.bytes().take_while(|&b| bare(b)).count();
    if word > 0
        && tail
            .as_bytes()
            .get(word)
            .is_none_or(|b| matches!(b, b' ' | b'\t' | b'\n'))
        && line[..cursor]
            .bytes()
            .all(|b| bare(b) || matches!(b, b' ' | b'\t'))
    {
        return line.len() - tail.len() + word;
    }
    let next_end = |source| {
        lex(source, false, true)
            .ok()?
            .1
            .into_iter()
            .find(|&(_, end)| end > cursor)
            .map(|(_, end)| end)
    };
    next_end(line)
        .or_else(|| {
            // History can contain syntax errors beyond this argument. Confirm a
            // shorter prefix with the lexer before copying any of it into the draft.
            let (_, end) = history_word_spans(line)
                .into_iter()
                .find(|&(_, end)| end > cursor)?;
            next_end(&line[..end])
        })
        .unwrap_or(cursor)
}

pub(crate) fn is_function_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || "_-.+:@/".contains(c))
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
    mut escape: impl FnMut(char, &mut std::str::Chars) -> String,
) -> Result<Word, Error> {
    let mut builder = crate::word::WordBuilder::default();
    let mut chars = source.chars();
    while let Some(c) = chars.next() {
        match c {
            '$' if expand => lexer::dollar(&mut chars, &mut builder, true)?,
            '`' if expand => lexer::backquote(&mut chars, &mut builder, true)?,
            '\\' => match chars.next() {
                Some(code) => builder.literal(&escape(code, &mut chars)),
                None => builder.push('\\'),
            },
            c => builder.push(c),
        }
    }
    Ok(builder.finish())
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Target {
    ReadWrite,
    Clobber,
    NoClobber,
    CloseMoved,
    Read,
    Write,
    Append,
    Duplicate,
    OutputWord,
    HereDoc { expand: bool },
    HereString,
}

#[derive(Debug, Clone)]
pub struct Redirect<W = String> {
    pub variable: Option<Box<str>>,
    pub fd: i32,
    pub target: Target,
    pub word: W,
}

#[derive(Debug, Clone)]
pub struct SimpleCommand<W = String> {
    pub location: Option<Location>,
    pub compound: Option<Box<Command>>,
    pub words: Vec<W>,
    pub redirects: Vec<Redirect<W>>,
    pub assignments: Vec<(String, Assignment<W>)>,
}

impl<W> Default for SimpleCommand<W> {
    fn default() -> Self {
        Self {
            location: None,
            compound: None,
            words: Vec::new(),
            redirects: Vec::new(),
            assignments: Vec::new(),
        }
    }
}

impl<W> SimpleCommand<W> {
    fn empty(&self) -> bool {
        self.compound.is_none()
            && self.words.is_empty()
            && self.redirects.is_empty()
            && self.assignments.is_empty()
    }
}

pub type Pipeline<W = String> = Vec<SimpleCommand<W>>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Condition {
    Always,
    Success,
    Failure,
}

impl Condition {
    pub fn allows(self, status: u8) -> bool {
        match self {
            Self::Always => true,
            Self::Success => status == 0,
            Self::Failure => status != 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Location {
    source: Rc<str>,
    start: usize,
    pub(crate) end: usize,
    pub line: usize,
}
impl Location {
    pub(crate) fn label(source: String, line: usize) -> Self {
        Self {
            start: 0,
            end: source.len(),
            source: source.into(),
            line,
        }
    }
    pub(crate) fn full_source(&self) -> &Rc<str> {
        &self.source
    }
    pub fn text(&self) -> &str {
        &self.source[self.start..self.end]
    }
}

#[derive(Debug, Clone)]
pub struct Statement {
    pub location: Location,
    pub condition: Condition,
    pub negated: bool,
    pub command: Command,
}

#[derive(Debug)]
pub enum CasePattern {
    Fixed(Matcher<String>),
    Extended(crate::pattern::extended::Pattern, Matcher<String>),
    Dynamic(Word),
}

impl CasePattern {
    fn new(word: Word) -> Self {
        use crate::word::Part;
        match word {
            Word::Literal(text) => Self::Fixed(Matcher::Literal(text)),
            Word::Expanded(ref parts)
                if parts
                    .iter()
                    .all(|p| matches!(p, Part::Text(_) | Part::Pattern(_))) =>
            {
                if EXTGLOB.get() && parts.iter().any(|part| matches!(part, Part::Pattern(s) if crate::pattern::extended::has_group(s))) {
                    let text = word.extended_pattern(&|_| None, &[]);
                    let extended = crate::pattern::extended::Pattern::new(&text);
                    let basic = Matcher::new(text);
                    return match extended {
                        Some(group) => Self::Extended(group, basic),
                        None => Self::Fixed(basic),
                    };
                }
                Self::Fixed(Matcher::new(word.pattern(&|_| None, &[])))
            }
            word => Self::Dynamic(word),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseEnd {
    Stop,
    FallThrough,
    Retest,
}

pub type CaseArm = (Vec<CasePattern>, Vec<Statement>, CaseEnd);

#[derive(Debug)]
pub struct FunctionDefinition {
    pub location: Location,
    pub body: wrapper::FunctionBody,
}

#[derive(Debug, Clone)]
pub enum Command {
    Pipeline(Pipeline<Word>),
    Group(Vec<Statement>),
    Timed {
        posix: bool,
        command: Box<Command>,
    },
    Test(Expr),
    Arithmetic(String),
    ForArithmetic {
        init: String,
        condition: String,
        step: String,
        body: Vec<Statement>,
    },
    Subshell(Vec<Statement>),
    Coprocess {
        name: String,
        body: Box<Command>,
    },
    Background(Vec<Statement>),
    Case {
        header: Location,
        word: Word,
        arms: Rc<Vec<CaseArm>>,
    },
    If {
        branches: Vec<(Vec<Statement>, Vec<Statement>)>,
        otherwise: Vec<Statement>,
    },
    While {
        until: bool,
        condition: Vec<Statement>,
        body: Vec<Statement>,
    },
    For {
        select: bool,
        header: Location,
        name: String,
        words: Option<Vec<Word>>,
        body: Vec<Statement>,
    },
    Function {
        name: String,
        definition: Rc<FunctionDefinition>,
    },
}

#[derive(Debug)]
pub enum Error {
    Incomplete(&'static str),
    HereDoc(HereDocHint),
    Invalid(&'static str),
}

impl Error {
    pub fn incomplete(&self) -> bool {
        matches!(self, Self::Incomplete(_) | Self::HereDoc(_))
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::Incomplete(s) | Self::Invalid(s) => s,
            Self::HereDoc(_) => "unterminated here-document",
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

fn with_read_options<T>(session: &crate::Session, line: usize, parse: impl FnOnce() -> T) -> T {
    let read = || {
        let old_line = FIRST_LINE.replace(line);
        let before = EXTGLOB.replace(session.shopt.glob.extglob);
        let parsed = parse();
        EXTGLOB.set(before);
        FIRST_LINE.set(old_line);
        parsed
    };
    if session.shopt.expand_aliases && session.aliases.0.is_some() {
        aliases::with(session.aliases.clone(), read)
    } else {
        read()
    }
}

// Generated eval assignments often change just the value. Construct the same
// simple-command AST; assignment attributes, traps and exit status stay live.
pub(crate) fn literal_assignment(source: &str, line: usize) -> Option<Statement> {
    let command = source.strip_suffix('\n').unwrap_or(source);
    let (name, value) = command.split_once('=')?;
    if !crate::word::is_identifier(name) || !value.bytes().all(lexer::plain_byte) {
        return None;
    }
    let location = Location {
        source: source.into(),
        start: 0,
        end: command.len(),
        line,
    };
    Some(Statement {
        location,
        condition: Condition::Always,
        negated: false,
        command: Command::Pipeline(vec![SimpleCommand {
            assignments: vec![(
                name.to_owned(),
                Assignment::Scalar(Word::Literal(value.to_owned())),
            )],
            ..Default::default()
        }]),
    })
}

fn parse_source(source: &str, final_input: bool) -> Result<Vec<Statement>, Error> {
    if source.contains('\0') {
        return Err(Error::Invalid("NUL in command text"));
    }
    if let Some(function) = wrapper::parse(source) {
        return Ok(vec![function]);
    }
    let (tokens, spans, _) = lex(source, false, final_input)?;
    Reader::new(tokens, spans, source).list(&[])
}

// Most shell reads are a few lines. Reuse their token and span allocations;
// nested parses take another pair while the outer reader owns its buffers.
type TokenBuffers = (Vec<Token>, Vec<(usize, usize)>);
thread_local! {
    static TOKEN_BUFFERS: std::cell::Cell<TokenBuffers> = const {
        std::cell::Cell::new((Vec::new(), Vec::new()))
    };
}

struct Reader {
    source: Rc<str>,
    spans: Vec<(usize, usize)>,
    newlines: Vec<usize>,
    aliases: crate::alias::Aliases,
    alias_frames: Vec<aliases::Frame>,
    tokens: std::collections::VecDeque<Token>,
}

impl Drop for Reader {
    fn drop(&mut self) {
        if self.tokens.capacity() <= 256 && self.spans.capacity() <= 256 {
            self.tokens.clear();
            self.spans.clear();
            TOKEN_BUFFERS.set((
                std::mem::take(&mut self.tokens).into(),
                std::mem::take(&mut self.spans),
            ));
        }
    }
}

impl Reader {
    fn new(tokens: Vec<Token>, spans: Vec<(usize, usize)>, source: &str) -> Self {
        Self {
            spans,
            newlines: memchr::memchr_iter(b'\n', source.as_bytes()).collect(),
            source: source.into(),
            tokens: tokens.into(),
            aliases: aliases::current(),
            alias_frames: Vec::new(),
        }
    }

    fn position(&self) -> usize {
        self.spans.len() - self.tokens.len()
    }
    fn location(&self, from: usize) -> Location {
        let start = self.spans.get(from).map_or(self.source.len(), |s| s.0);
        let end = if self.position() > from {
            self.spans[self.position() - 1].1
        } else {
            start
        };
        Location {
            source: self.source.clone(),
            start,
            end,
            line: FIRST_LINE.get() + self.newlines.partition_point(|&n| n < start),
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
            Err(Error::Incomplete("unfinished compound command"))
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
        self.read_list(stops, false)
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
                    return Err(Error::Incomplete("expected a command after operator"));
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
                && matches!(self.peek(), None | Some(Token::End | Token::Newline))
            {
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
            // Nested lists often contain one statement. Its enum is large.
            if statements.is_empty() {
                statements.reserve_exact(1);
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
        let start = self.position();
        let mut first = match self.compound()? {
            Some(compound) if !self.has_redirect() && !self.has_pipe() => return Ok(compound),
            Some(compound) => self.compound_stage(compound)?,
            None => self.simple()?,
        };
        if self.has_pipe() {
            first.location = Some(self.location(start));
        }
        let mut pipeline = vec![first];
        while self.has_pipe() {
            if matches!(self.tokens.pop_front(), Some(Token::PipeBoth)) {
                pipeline.last_mut().unwrap().redirects.push(Redirect {
                    variable: None,
                    fd: 2,
                    target: Target::Duplicate,
                    word: Word::Literal("1".into()),
                });
            }
            self.newlines();
            if self.peek().is_none() {
                return Err(Error::Incomplete("expected a command after pipe"));
            }
            self.expand_alias(true)?;
            let start = self.position();
            let mut stage = match self.compound()? {
                Some(compound) => self.compound_stage(compound)?,
                None => self.simple()?,
            };
            stage.location = Some(self.location(start));
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
            return Ok(Some(Command::Arithmetic(expr)));
        }
        if matches!(self.peek(), Some(Token::LeftParen)) {
            self.tokens.pop_front();
            let body = self.list(&[")"])?;
            if self.tokens.pop_front().is_none() {
                return Err(Error::Incomplete("unfinished subshell"));
            }
            return Ok(Some(Command::Subshell(body)));
        }
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
                Ok(Command::Test(expr))
            }
            Some("coproc") => self.coproc(),
            Some("case") => self.case_clause(),
            Some("if") => self.if_clause(),
            Some("while" | "until") => self.while_clause(),
            Some("for" | "select") => self.for_clause(),
            Some("function") => self.function(true),
            Some(name)
                if matches!(self.tokens.get(1), Some(Token::LeftParen))
                    && is_function_name(name) =>
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
            return Err(Error::Incomplete("expected a coprocess command"));
        }
        let command = if let Some(compound) = self.compound()? {
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

    fn test_or(&mut self) -> Result<Expr, Error> {
        let mut result = self.test_and()?;
        while matches!(self.peek(), Some(Token::Or)) {
            self.tokens.pop_front();
            self.newlines();
            result = Expr::Or(Box::new(result), Box::new(self.test_and()?));
        }
        Ok(result)
    }

    fn test_and(&mut self) -> Result<Expr, Error> {
        let mut result = self.test_atom()?;
        while matches!(self.peek(), Some(Token::And)) {
            self.tokens.pop_front();
            self.newlines();
            result = Expr::And(Box::new(result), Box::new(self.test_atom()?));
        }
        Ok(result)
    }

    fn test_word(&mut self) -> Result<Word, Error> {
        if self.keyword() == Some("]]") {
            return Err(Error::Invalid("missing conditional operand"));
        }
        match self.operand() {
            Some(Token::Word(word)) => Ok(word.word),
            None => Err(Error::Incomplete("unfinished conditional expression")),
            _ => Err(Error::Invalid("expected a conditional operand")),
        }
    }

    fn test_atom(&mut self) -> Result<Expr, Error> {
        self.newlines();
        if self.keyword() == Some("!") {
            self.tokens.pop_front();
            return Ok(Expr::Not(Box::new(self.test_atom()?)));
        }
        if matches!(self.peek(), Some(Token::LeftParen)) {
            self.tokens.pop_front();
            let e = self.test_or()?;
            match self.tokens.pop_front() {
                Some(Token::RightParen) => return Ok(e),
                None => return Err(Error::Incomplete("unclosed conditional grouping")),
                _ => return Err(Error::Invalid("expected ')' in conditional expression")),
            }
        }
        let unary = self.keyword().is_some_and(|op| {
            matches!(
                op,
                "-a" | "-b"
                    | "-c"
                    | "-d"
                    | "-e"
                    | "-f"
                    | "-g"
                    | "-h"
                    | "-k"
                    | "-n"
                    | "-o"
                    | "-p"
                    | "-r"
                    | "-s"
                    | "-t"
                    | "-u"
                    | "-v"
                    | "-w"
                    | "-x"
                    | "-z"
                    | "-G"
                    | "-L"
                    | "-N"
                    | "-O"
                    | "-R"
                    | "-S"
            )
        });
        if unary {
            let op = self.keyword().unwrap().to_owned();
            self.tokens.pop_front();
            return Ok(Expr::Unary(op, self.test_word()?));
        }
        let left = self.test_word()?;
        let op = match self.peek() {
            Some(Token::Redirect(0, Target::Read)) => "<",
            Some(Token::Redirect(1, Target::Write)) => ">",
            _ => match self.keyword() {
                Some(
                    op @ ("=" | "==" | "!=" | "=~" | "-eq" | "-ne" | "-lt" | "-gt" | "-le" | "-ge"
                    | "-ef" | "-nt" | "-ot"),
                ) => op,
                _ => return Ok(Expr::Word(left)),
            },
        }
        .to_owned();
        self.tokens.pop_front();
        Ok(Expr::Binary(left, op, self.test_word()?))
    }

    fn case_clause(&mut self) -> Result<Command, Error> {
        let start = self.position();
        self.tokens.pop_front();
        let word = match self.operand() {
            Some(Token::Word(word)) => word.word,
            None => return Err(Error::Incomplete("expected a case word")),
            _ => return Err(Error::Invalid("expected a case word")),
        };
        self.newlines();
        self.expect("in")?;
        let header = self.location(start);
        self.newlines();
        let mut arms = Vec::new();
        while self.keyword() != Some("esac") {
            if matches!(self.peek(), Some(Token::LeftParen)) {
                self.tokens.pop_front();
            }
            let mut patterns = Vec::new();
            loop {
                match self.operand() {
                    Some(Token::Word(word)) => patterns.push(CasePattern::new(word.word)),
                    None => return Err(Error::Incomplete("unfinished case command")),
                    _ => return Err(Error::Invalid("expected a case pattern")),
                }
                match self.tokens.pop_front() {
                    Some(Token::RightParen) => break,
                    Some(Token::Pipe) => {}
                    None => return Err(Error::Incomplete("expected ')' after case pattern")),
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
        Ok(Command::Case {
            header,
            word,
            arms: Rc::new(arms),
        })
    }

    fn if_clause(&mut self) -> Result<Command, Error> {
        self.tokens.pop_front();
        let mut branches = Vec::new();
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
            None => Err(Error::Incomplete("expected an identifier")),
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
            self.expect("do")?;
            let body = self.list(&["done"])?;
            self.expect("done")?;
            return Ok(Command::ForArithmetic {
                init: init.into(),
                condition: condition.into(),
                step: step.into(),
                body,
            });
        }
        let name = self.name()?;
        self.newlines();
        let words = if self.keyword() == Some("in") {
            self.tokens.pop_front();
            let mut words = Vec::new();
            while matches!(self.peek(), Some(Token::Word(_) | Token::Reserved(_))) {
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
        self.newlines();
        self.expect("do")?;
        let body = self.list(&["done"])?;
        self.expect("done")?;
        Ok(Command::For {
            select,
            header,
            name,
            words,
            body,
        })
    }

    fn function(&mut self, keyword: bool) -> Result<Command, Error> {
        if keyword {
            self.tokens.pop_front();
        }
        let name = match self.operand() {
            Some(Token::Word(LexWord {
                word: Word::Literal(name),
                protected: false,
                ..
            })) if is_function_name(&name) => name,
            None => return Err(Error::Incomplete("expected a function name")),
            _ => return Err(Error::Invalid("invalid function name")),
        };
        if matches!(self.peek(), Some(Token::LeftParen)) {
            self.tokens.pop_front();
            match self.tokens.pop_front() {
                Some(Token::RightParen) => {}
                None => return Err(Error::Incomplete("unfinished function definition")),
                _ => return Err(Error::Invalid("expected empty function parentheses")),
            }
        }
        self.newlines();
        let start = self.position();
        let compound = self.compound()?.ok_or_else(|| {
            if self.peek().is_none() {
                Error::Incomplete("expected a function body")
            } else {
                Error::Invalid("expected a compound function body")
            }
        })?;
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
        Ok(Command::Function {
            name,
            definition: Rc::new(FunctionDefinition {
                body: wrapper::FunctionBody::Parsed(body),
                location: self.location(start),
            }),
        })
    }

    fn redirection(
        &mut self,
        token: Token,
        redirects: &mut Vec<Redirect<Word>>,
    ) -> Result<(), Error> {
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
                    variable: None,
                    fd: 1,
                    target,
                    word,
                });
                redirects.push(Redirect {
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

    fn simple(&mut self) -> Result<SimpleCommand<Word>, Error> {
        let mut cmd = SimpleCommand::default();
        loop {
            self.expand_alias(cmd.words.is_empty())?;
            if !matches!(
                self.peek(),
                Some(
                    Token::Word(_)
                        | Token::Reserved(_)
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
                    let declaration = matches!(
                        cmd.words.first().and_then(Word::as_literal),
                        Some("export" | "local" | "declare" | "typeset" | "readonly")
                    );
                    if matches!(self.peek(), Some(Token::ArrayStart)) {
                        let name = assignment.ok_or(Error::Invalid("invalid array assignment"))?;
                        if !is_identifier(name.strip_suffix('+').unwrap_or(&name))
                            || !(cmd.words.is_empty() || declaration)
                        {
                            return Err(Error::Invalid("invalid array assignment"));
                        }
                        let items = self.array_items()?;
                        let operand = declaration.then_some(cmd.words.len());
                        if declaration {
                            cmd.words
                                .push(Word::Literal(name.trim_end_matches('+').into()));
                        }
                        cmd.assignments
                            .push((name, Assignment::Array(operand, items)));
                        continue;
                    }
                    if cmd.words.is_empty() {
                        if let Some(name) = assignment.filter(|name| !name.starts_with('[')) {
                            word.remove_assignment_prefix(name.len() + 1);
                            cmd.assignments.push((name, Assignment::Scalar(word)));
                            continue;
                        }
                        if !protected && word.as_literal().is_some_and(reserved) {
                            return Err(Error::Invalid("unexpected reserved word"));
                        }
                        // Most simple commands have only the name and one operand.
                        cmd.words.reserve_exact(2);
                    } else if assignment.is_some()
                        && matches!(
                            cmd.words[0].as_literal(),
                            Some("export" | "local" | "declare" | "typeset" | "readonly")
                        )
                    {
                        word = word.declaration();
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
    }

    #[test]
    fn distinguishes_continuation_from_invalid_input() {
        for source in ["cmd |\n", "cmd '", "cmd \\\n"] {
            assert!(matches!(parse(source), Err(Error::Incomplete(_))));
        }
        for source in ["| cmd", "cmd || ; cmd", "cmd > ", "cmd 2147483648>&2"] {
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
        let Command::For {
            words: Some(words),
            body,
            ..
        } = &definition.body.statements()[0].command
        else {
            panic!()
        };
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
        let Command::Case { arms, .. } = &program[1].command else {
            panic!()
        };
        assert_eq!(arms[0].0.len(), 2);
        assert!(arms[1].1.is_empty());
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
        assert!(matches!(program[0].command, Command::Test(Expr::And(..))));
        assert!(matches!(program[1].command, Command::ForArithmetic { .. }));
        assert!(parse("[[ x && ]]").is_err());
        assert!(matches!(parse("[[ x =="), Err(Error::Incomplete(_))));
    }
}
