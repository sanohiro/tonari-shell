mod aliases;
mod array;
use crate::test_expression::Expr;
pub use array::Assignment;
mod ansi;
pub(crate) mod completion_words;
mod here_doc;
mod lexer;
pub use crate::word::{Word, is_identifier};
pub(crate) use here_doc::{HereDocHint, expansion_string};
use lexer::{LexWord, Token, descriptor, lex};

// Keep text supplied by prompt escapes (including the working directory) in
// literal word segments. It must never be parsed as shell code.
pub(crate) fn prompt_word(
    source: &str,
    mut escape: impl FnMut(char) -> String,
) -> Result<Word, Error> {
    let mut builder = crate::word::WordBuilder::default();
    let mut chars = source.chars();
    while let Some(c) = chars.next() {
        match c {
            '$' => lexer::dollar(&mut chars, &mut builder, true)?,
            '`' => lexer::backquote(&mut chars, &mut builder, true)?,
            '\\' => match chars.next() {
                Some(code) => builder.literal(&escape(code)),
                None => builder.push('\\'),
            },
            c => builder.push(c),
        }
    }
    Ok(builder.finish())
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Target {
    Read,
    Write,
    Append,
    Duplicate,
    HereDoc { expand: bool },
    HereString,
}

#[derive(Debug, Clone)]
pub struct Redirect<W = String> {
    pub fd: i32,
    pub target: Target,
    pub word: W,
}

#[derive(Debug, Clone)]
pub struct SimpleCommand<W = String> {
    pub compound: Option<Box<Command>>,
    pub words: Vec<W>,
    pub redirects: Vec<Redirect<W>>,
    pub assignments: Vec<(String, Assignment<W>)>,
}

impl<W> Default for SimpleCommand<W> {
    fn default() -> Self {
        Self {
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
pub struct Statement {
    pub condition: Condition,
    pub negated: bool,
    pub command: Command,
}

#[derive(Debug, Clone)]
pub enum Command {
    Pipeline(Pipeline<Word>),
    Group(Vec<Statement>),
    Test(Expr),
    Arithmetic(String),
    ForArithmetic {
        init: String,
        condition: String,
        step: String,
        body: Vec<Statement>,
    },
    Subshell(Vec<Statement>),
    Background(Vec<Statement>),
    Case {
        word: Word,
        arms: Vec<(Vec<Word>, Vec<Statement>)>,
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
        name: String,
        words: Option<Vec<Word>>,
        body: Vec<Statement>,
    },
    Function {
        name: String,
        body: Vec<Statement>,
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

fn reserved(word: &str) -> bool {
    matches!(
        word,
        "if" | "then"
            | "else"
            | "elif"
            | "fi"
            | "case"
            | "esac"
            | "while"
            | "until"
            | "for"
            | "do"
            | "done"
            | "in"
            | "select"
            | "function"
            | "coproc"
            | "time"
            | "!"
            | "{"
            | "[["
            | "}"
    )
}

pub fn parse(source: &str) -> Result<Vec<Statement>, Error> {
    parse_source(source, false)
}

pub fn parse_final(source: &str) -> Result<Vec<Statement>, Error> {
    parse_source(source, true)
}

pub fn session_source(
    source: &str,
    eof: bool,
    session: &crate::Session,
) -> Result<Vec<Statement>, Error> {
    let read = || {
        if eof {
            parse_final(source)
        } else {
            parse(source)
        }
    };
    if session.shopt.expand_aliases && session.aliases.0.is_some() {
        aliases::with(session.aliases.clone(), read)
    } else {
        read()
    }
}

fn parse_source(source: &str, final_input: bool) -> Result<Vec<Statement>, Error> {
    if source.contains('\0') {
        return Err(Error::Invalid("NUL in command text"));
    }
    let (tokens, _) = lex(source, false, final_input)?;
    Reader::new(tokens).list(&[])
}

struct Reader {
    aliases: crate::alias::Aliases,
    alias_frames: Vec<aliases::Frame>,
    tokens: std::vec::IntoIter<Token>,
}

impl Reader {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens: tokens.into_iter(),
            aliases: aliases::current(),
            alias_frames: Vec::new(),
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.as_slice().first()
    }

    fn keyword(&self) -> Option<&str> {
        match self.peek()? {
            Token::Word(LexWord {
                word,
                protected: false,
                ..
            }) => word.as_literal(),
            _ => None,
        }
    }

    fn expect(&mut self, keyword: &str) -> Result<(), Error> {
        if self.keyword() == Some(keyword) {
            self.tokens.next();
            Ok(())
        } else if self.peek().is_none() {
            Err(Error::Incomplete("unfinished compound command"))
        } else {
            Err(Error::Invalid("unexpected token in compound command"))
        }
    }

    fn newlines(&mut self) {
        while matches!(self.peek(), Some(Token::Newline)) {
            self.tokens.next();
        }
    }

    fn at_stop(&self, stops: &[&str]) -> bool {
        match self.peek() {
            Some(Token::RightParen) => stops.contains(&")"),
            Some(Token::EndArm) => stops.contains(&";;"),
            _ => self.keyword().is_some_and(|s| stops.contains(&s)),
        }
    }

    fn list(&mut self, stops: &[&str]) -> Result<Vec<Statement>, Error> {
        let mut statements = Vec::new();
        let mut condition = Condition::Always;
        let mut and_or_start = 0;
        loop {
            self.newlines();
            if self.peek().is_none() {
                if condition != Condition::Always {
                    return Err(Error::Incomplete("expected a command after operator"));
                }
                return Ok(statements);
            }
            if self.at_stop(stops) {
                if (statements.is_empty() && !stops.contains(&";;"))
                    || condition != Condition::Always
                {
                    return Err(Error::Invalid("expected a command"));
                }
                return Ok(statements);
            }
            self.expand_alias(true)?;
            if self.peek().is_none() && condition == Condition::Always {
                return Ok(statements);
            }
            let mut inverted = None;
            while self.keyword() == Some("!") {
                self.tokens.next();
                inverted = Some(!inverted.unwrap_or(false));
                self.expand_alias(true)?;
            }
            let command = if inverted.is_some()
                && matches!(self.peek(), None | Some(Token::End | Token::Newline))
            {
                Command::Pipeline(vec![SimpleCommand::default()])
            } else {
                self.command()?
            };
            statements.push(Statement {
                condition,
                negated: inverted.unwrap_or(false),
                command,
            });
            condition = match self.peek() {
                None => return Ok(statements),
                Some(Token::EndArm | Token::RightParen) if self.at_stop(stops) => {
                    return Ok(statements);
                }
                Some(Token::End | Token::Newline) => {
                    and_or_start = statements.len();
                    Condition::Always
                }
                Some(Token::Background) => {
                    let group = statements.split_off(and_or_start);
                    statements.push(Statement {
                        condition: Condition::Always,
                        negated: false,
                        command: Command::Background(group),
                    });
                    and_or_start = statements.len();
                    Condition::Always
                }
                Some(Token::And) => Condition::Success,
                Some(Token::Or) => Condition::Failure,
                _ => return Err(Error::Invalid("expected a command separator")),
            };
            self.tokens.next();
        }
    }

    fn command(&mut self) -> Result<Command, Error> {
        let first = match self.compound()? {
            Some(compound) if !self.has_redirect() && !self.has_pipe() => return Ok(compound),
            Some(compound) => self.compound_stage(compound)?,
            None => self.simple()?,
        };
        let mut pipeline = vec![first];
        while self.has_pipe() {
            if matches!(self.tokens.next(), Some(Token::PipeBoth)) {
                pipeline.last_mut().unwrap().redirects.push(Redirect {
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
            let stage = match self.compound()? {
                Some(compound) => self.compound_stage(compound)?,
                None => self.simple()?,
            };
            pipeline.push(stage);
        }
        Ok(Command::Pipeline(pipeline))
    }

    fn has_redirect(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token::Redirect(..) | Token::BothRedirect(_))
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
            let token = self.tokens.next().unwrap();
            self.redirection(token, &mut stage.redirects)?;
        }
        Ok(stage)
    }

    fn compound(&mut self) -> Result<Option<Command>, Error> {
        if matches!(self.peek(), Some(Token::Arithmetic(_))) {
            let Some(Token::Arithmetic(expr)) = self.tokens.next() else {
                unreachable!()
            };
            return Ok(Some(Command::Arithmetic(expr)));
        }
        if matches!(self.peek(), Some(Token::LeftParen)) {
            self.tokens.next();
            let body = self.list(&[")"])?;
            if self.tokens.next().is_none() {
                return Err(Error::Incomplete("unfinished subshell"));
            }
            return Ok(Some(Command::Subshell(body)));
        }
        match self.keyword() {
            Some("{") => {
                self.tokens.next();
                let body = self.list(&["}"])?;
                self.expect("}")?;
                Ok(Command::Group(body))
            }
            Some("[[") => {
                self.tokens.next();
                let expr = self.test_or()?;
                self.newlines();
                self.expect("]]")?;
                Ok(Command::Test(expr))
            }
            Some("case") => self.case_clause(),
            Some("if") => self.if_clause(),
            Some("while" | "until") => self.while_clause(),
            Some("for") => self.for_clause(),
            Some("function") => self.function(true),
            Some(name)
                if is_identifier(name)
                    && matches!(self.tokens.as_slice().get(1), Some(Token::LeftParen)) =>
            {
                self.function(false)
            }
            _ => return Ok(None),
        }
        .map(Some)
    }

    fn test_or(&mut self) -> Result<Expr, Error> {
        let mut result = self.test_and()?;
        while matches!(self.peek(), Some(Token::Or)) {
            self.tokens.next();
            self.newlines();
            result = Expr::Or(Box::new(result), Box::new(self.test_and()?));
        }
        Ok(result)
    }

    fn test_and(&mut self) -> Result<Expr, Error> {
        let mut result = self.test_atom()?;
        while matches!(self.peek(), Some(Token::And)) {
            self.tokens.next();
            self.newlines();
            result = Expr::And(Box::new(result), Box::new(self.test_atom()?));
        }
        Ok(result)
    }

    fn test_word(&mut self) -> Result<Word, Error> {
        if self.keyword() == Some("]]") {
            return Err(Error::Invalid("missing conditional operand"));
        }
        match self.tokens.next() {
            Some(Token::Word(word)) => Ok(word.word),
            None => Err(Error::Incomplete("unfinished conditional expression")),
            _ => Err(Error::Invalid("expected a conditional operand")),
        }
    }

    fn test_atom(&mut self) -> Result<Expr, Error> {
        self.newlines();
        if self.keyword() == Some("!") {
            self.tokens.next();
            return Ok(Expr::Not(Box::new(self.test_atom()?)));
        }
        if matches!(self.peek(), Some(Token::LeftParen)) {
            self.tokens.next();
            let e = self.test_or()?;
            match self.tokens.next() {
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
            self.tokens.next();
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
        self.tokens.next();
        Ok(Expr::Binary(left, op, self.test_word()?))
    }

    fn case_clause(&mut self) -> Result<Command, Error> {
        self.tokens.next();
        let word = match self.tokens.next() {
            Some(Token::Word(word)) => word.word,
            None => return Err(Error::Incomplete("expected a case word")),
            _ => return Err(Error::Invalid("expected a case word")),
        };
        self.newlines();
        self.expect("in")?;
        self.newlines();
        let mut arms = Vec::new();
        while self.keyword() != Some("esac") {
            if matches!(self.peek(), Some(Token::LeftParen)) {
                self.tokens.next();
            }
            let mut patterns = Vec::new();
            loop {
                match self.tokens.next() {
                    Some(Token::Word(word)) => patterns.push(word.word),
                    None => return Err(Error::Incomplete("unfinished case command")),
                    _ => return Err(Error::Invalid("expected a case pattern")),
                }
                match self.tokens.next() {
                    Some(Token::RightParen) => break,
                    Some(Token::Pipe) => {}
                    None => return Err(Error::Incomplete("expected ')' after case pattern")),
                    _ => return Err(Error::Invalid("expected ')' after case pattern")),
                }
            }
            let body = self.list(&[";;", "esac"])?;
            arms.push((patterns, body));
            if matches!(self.peek(), Some(Token::EndArm)) {
                self.tokens.next();
            }
            self.newlines();
        }
        self.expect("esac")?;
        Ok(Command::Case { word, arms })
    }

    fn if_clause(&mut self) -> Result<Command, Error> {
        self.tokens.next();
        let mut branches = Vec::new();
        loop {
            let condition = self.list(&["then"])?;
            self.expect("then")?;
            let body = self.list(&["else", "elif", "fi"])?;
            branches.push((condition, body));
            if self.keyword() != Some("elif") {
                break;
            }
            self.tokens.next();
        }
        let otherwise = if self.keyword() == Some("else") {
            self.tokens.next();
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
        self.tokens.next();
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
        match self.tokens.next() {
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
        self.tokens.next();
        if matches!(self.peek(), Some(Token::Arithmetic(_))) {
            let Some(Token::Arithmetic(header)) = self.tokens.next() else {
                unreachable!()
            };
            let mut expressions = header.split(';');
            let (Some(init), Some(condition), Some(step), None) = (
                expressions.next(),
                expressions.next(),
                expressions.next(),
                expressions.next(),
            ) else {
                return Err(Error::Invalid("arithmetic for requires three expressions"));
            };
            if matches!(self.peek(), Some(Token::End)) {
                self.tokens.next();
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
            self.tokens.next();
            let mut words = Vec::new();
            while matches!(self.peek(), Some(Token::Word(_))) {
                if let Some(Token::Word(word)) = self.tokens.next() {
                    words.push(word.word);
                }
            }
            Some(words)
        } else {
            None
        };
        if matches!(self.peek(), Some(Token::End)) {
            self.tokens.next();
        }
        self.newlines();
        self.expect("do")?;
        let body = self.list(&["done"])?;
        self.expect("done")?;
        Ok(Command::For { name, words, body })
    }

    fn function(&mut self, keyword: bool) -> Result<Command, Error> {
        if keyword {
            self.tokens.next();
        }
        let name = self.name()?;
        if matches!(self.peek(), Some(Token::LeftParen)) {
            self.tokens.next();
            match self.tokens.next() {
                Some(Token::RightParen) => {}
                None => return Err(Error::Incomplete("unfinished function definition")),
                _ => return Err(Error::Invalid("expected empty function parentheses")),
            }
        }
        self.newlines();
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
                condition: Condition::Always,
                negated: false,
                command: Command::Pipeline(vec![self.compound_stage(compound)?]),
            }]
        } else if let Command::Group(body) = compound {
            body
        } else {
            vec![Statement {
                condition: Condition::Always,
                negated: false,
                command: compound,
            }]
        };
        Ok(Command::Function { name, body })
    }

    fn redirection(
        &mut self,
        token: Token,
        redirects: &mut Vec<Redirect<Word>>,
    ) -> Result<(), Error> {
        match token {
            Token::BothRedirect(target) => {
                let Some(Token::Word(LexWord { word, .. })) = self.tokens.next() else {
                    return Err(Error::Invalid("expected a redirection target"));
                };
                redirects.push(Redirect {
                    fd: 1,
                    target,
                    word,
                });
                redirects.push(Redirect {
                    fd: 2,
                    target: Target::Duplicate,
                    word: Word::Literal("1".into()),
                });
            }
            Token::Redirect(fd, target) => {
                let Some(Token::Word(LexWord { word, .. })) = self.tokens.next() else {
                    return Err(Error::Invalid("expected a redirection target"));
                };
                if target == Target::Duplicate {
                    if let Some(value) = word.as_literal().filter(|s| *s != "-") {
                        descriptor(value)?;
                    }
                }
                redirects.push(Redirect { fd, target, word });
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
                Some(Token::Word(_) | Token::Redirect(_, _) | Token::BothRedirect(_))
            ) {
                break;
            }
            match self.tokens.next().unwrap() {
                Token::Word(LexWord {
                    mut word,
                    protected,
                    assignment,
                }) => {
                    let declaration = matches!(
                        cmd.words.first().and_then(Word::as_literal),
                        Some("export" | "local" | "declare" | "typeset")
                    );
                    if matches!(self.peek(), Some(Token::ArrayStart)) {
                        let name = assignment.ok_or(Error::Invalid("invalid array assignment"))?;
                        if !is_identifier(name.strip_suffix('+').unwrap_or(&name))
                            || !(cmd.words.is_empty() || declaration)
                        {
                            return Err(Error::Invalid("invalid array assignment"));
                        }
                        let items = self.array_items()?;
                        if declaration {
                            cmd.words
                                .push(Word::Literal(name.trim_end_matches('+').into()));
                        }
                        cmd.assignments.push((name, Assignment::Array(items)));
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
                    } else if assignment.is_some()
                        && matches!(
                            cmd.words[0].as_literal(),
                            Some("export" | "local" | "declare" | "typeset")
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
        let Command::Function { name, body } = &program[0].command else {
            panic!()
        };
        assert_eq!(name, "f");
        let Command::For {
            words: Some(words),
            body,
            ..
        } = &body[0].command
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
        let Command::Function { name, body } = &program[1].command else {
            panic!()
        };
        assert_eq!(name, "g");
        assert!(matches!(
            body[0].command,
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
    fn conditional_syntax_and_arithmetic_commands() {
        let program=parse(r#"[[ -n "$x" && ( "$x" == a* || "$x" =~ ^(a|b)[0-9]+$ ) ]]; for ((i=0;i<3;i++)); do ((sum+=i)); done"#).unwrap();
        assert!(matches!(program[0].command, Command::Test(Expr::And(..))));
        assert!(matches!(program[1].command, Command::ForArithmetic { .. }));
        assert!(parse("[[ x && ]]").is_err());
        assert!(matches!(parse("[[ x =="), Err(Error::Incomplete(_))));
    }
}
