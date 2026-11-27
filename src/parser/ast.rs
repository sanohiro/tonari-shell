use super::*;

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
    pub line: usize,
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
    pub keyword_args: Option<Rc<Vec<(usize, String)>>>,
}

impl<W> Default for SimpleCommand<W> {
    fn default() -> Self {
        Self {
            location: None,
            compound: None,
            words: Vec::new(),
            redirects: Vec::new(),
            assignments: Vec::new(),
            keyword_args: None,
        }
    }
}

impl<W> SimpleCommand<W> {
    pub(super) fn empty(&self) -> bool {
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
    pub(super) source: Rc<str>,
    pub(super) subscript_aliases: crate::alias::Aliases,
    pub(super) start: usize,
    pub(crate) end: usize,
    pub line: usize,
}
impl Location {
    pub(crate) fn label(source: String, line: usize) -> Self {
        Self {
            start: 0,
            end: source.len(),
            source: source.into(),
            subscript_aliases: Default::default(),
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

impl Statement {
    pub(crate) fn single_stage(&self) -> Option<&SimpleCommand<Word>> {
        let Command::Pipeline(stages) = &self.command else {
            return None;
        };
        match stages.as_slice() {
            [cmd] => Some(cmd),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum CasePattern {
    Fixed(Matcher<String>),
    Extended(crate::pattern::extended::Pattern, Matcher<String>),
    Dynamic(Word),
}

impl CasePattern {
    pub(super) fn new(word: Word) -> Self {
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
    pub declaration_line: usize,
    pub location: Location,
    pub body: wrapper::FunctionBody,
    pub rendered_body: std::cell::OnceCell<String>,
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
