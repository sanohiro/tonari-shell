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

// Ordinary keyword operands keep their name in the word itself. Only indexed
// syntax and operands beyond the inline bitset need an additional spelling.
#[derive(Default, Debug, Clone)]
pub struct KeywordArgs {
    plain: u64,
    other: Option<Rc<Vec<(usize, String)>>>,
}

impl KeywordArgs {
    pub(super) fn record(&mut self, index: usize, name: &str) {
        if index < u64::BITS as usize && !name.contains('[') {
            self.plain |= 1 << index;
        } else {
            Rc::make_mut(self.other.get_or_insert_with(Default::default))
                .push((index, name.into()));
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.plain == 0 && self.other.is_none()
    }

    pub(crate) fn key<'a>(&'a self, index: usize, word: &'a Word) -> Option<&'a str> {
        if index < u64::BITS as usize && self.plain & (1 << index) != 0 {
            let text = match word {
                Word::Literal(text) => text,
                Word::Expanded(parts) | Word::Declaration(parts) => {
                    let crate::word::Part::Text(text) = parts.first()? else {
                        return None;
                    };
                    text
                }
            };
            return Some(text.split_once('=').map_or(text.as_str(), |(name, _)| name));
        }
        let other = self.other.as_ref()?;
        let pos = other.binary_search_by_key(&index, |(i, _)| *i).ok()?;
        Some(&other[pos].1)
    }
}

#[derive(Debug, Clone)]
pub struct SimpleCommand<W = String> {
    pub location: Option<Box<Location>>,
    pub compound: Option<std::rc::Rc<Command>>,
    pub words: Vec<W>,
    pub redirects: Vec<Redirect<W>>,
    pub assignments: Vec<(String, Assignment<W>)>,
    pub keyword_args: KeywordArgs,
}

impl<W> Default for SimpleCommand<W> {
    fn default() -> Self {
        Self {
            location: None,
            compound: None,
            words: Vec::new(),
            redirects: Vec::new(),
            assignments: Vec::new(),
            keyword_args: KeywordArgs::default(),
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
    Extended(String),
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
                if READ_OPTIONS.get().extglob && parts.iter().any(|part| matches!(part, Part::Pattern(s) if crate::pattern::extended::has_group(s))) {
                    let text = word.extended_pattern(&|_| None, &[]);
                    return Self::Extended(text);
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
pub struct ForClause {
    pub select: bool,
    pub header: Location,
    pub name: String,
    pub words: Option<Vec<Word>>,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub struct ArithmeticForClause {
    pub init: String,
    pub condition: String,
    pub step: String,
    pub body: Vec<Statement>,
}

#[derive(Debug)]
pub struct CaseClause {
    pub header: Location,
    pub word: Word,
    pub arms: Vec<CaseArm>,
}

#[derive(Debug, Clone)]
pub enum Command {
    Scalar(Box<(String, Word)>),
    LiteralAssignment(String, std::num::NonZeroUsize),
    Pipeline(Pipeline<Word>),
    Group(Vec<Statement>),
    Timed {
        posix: bool,
        command: Box<Command>,
    },
    Test(Box<Expr>),
    Arithmetic(Box<crate::arithmetic::Expression>),
    ForArithmetic(Box<ArithmeticForClause>),
    Subshell(Vec<Statement>),
    Coprocess {
        name: String,
        body: Box<Command>,
    },
    Background(Vec<Statement>),
    Case(Rc<CaseClause>),
    If {
        branches: Vec<(Vec<Statement>, Vec<Statement>)>,
        otherwise: Vec<Statement>,
    },
    While {
        until: bool,
        condition: Vec<Statement>,
        body: Vec<Statement>,
    },
    For(Box<ForClause>),
    Function {
        name: String,
        definition: Rc<FunctionDefinition>,
    },
}
