use std::rc::Rc;

use crate::parser::{self, Statement};
use crate::{Flow, Session};

pub(crate) fn builtin(session: &mut Session, args: &[String], status: u8) -> Flow {
    let args = match args.split_first() {
        Some((opt, rest)) if opt == "--" => rest,
        Some((opt, _)) if opt == "--help" => {
            crate::builtins::control::help(&[String::from("eval")]);
            return Flow::Continue(2);
        }
        Some((opt, _)) if opt.starts_with('-') && opt.len() > 1 => {
            let flag = opt[1..].chars().next().unwrap();
            diagnostic!("eval: -{flag}: invalid option\neval: usage: eval [arg ...]");
            return Flow::Continue(2);
        }
        _ => args,
    };
    if crate::stack_budget_exceeded() {
        diagnostic!("eval: nesting limit exceeded");
        return Flow::Unwind(1);
    }
    let joined;
    let source = match args {
        [] => return Flow::Continue(0),
        [one] => one.as_str(),
        many => {
            joined = many.join(" ");
            &joined
        }
    };
    // Bash 5.3 uses the calling line even when a quoted argument spans lines.
    let line = session
        .current_command
        .as_ref()
        .map_or(1, |location| location.line);
    let previous = std::mem::replace(&mut session.evaluating, true);
    session.trace_depth += 1;
    let flow = super::text(session, source, status, false, false, Some(line));
    session.trace_depth -= 1;
    session.evaluating = previous;
    flow
}

#[derive(Clone, Default)]
pub(crate) struct EvalCache {
    seen: Vec<Rc<str>>,
    parsed: Vec<Rc<Entry>>,
}

pub(super) struct Entry {
    source: Rc<str>,
    program: Vec<Statement>,
    line: usize,
    eof: bool,
    extglob: bool,
    extquote: bool,
    posix: bool,
    legacy_time: bool,
    comments: bool,
}

pub(super) enum Program {
    Fresh(Vec<Statement>),
    Cached(Rc<Entry>),
}

impl AsRef<[Statement]> for Program {
    fn as_ref(&self) -> &[Statement] {
        match self {
            Self::Fresh(statements) => statements,
            Self::Cached(entry) => &entry.program,
        }
    }
}

impl Program {
    pub(super) fn run(self, session: &mut Session, status: u8, tail: bool) -> Flow {
        match self {
            Self::Fresh(statements) => crate::execute_program(session, statements, status, tail),
            Self::Cached(entry) => {
                crate::eval::borrowed_program(session, &entry.program, status, tail)
            }
        }
    }
}

fn remember<T>(entries: &mut Vec<T>, value: T) {
    const CAPACITY: usize = 16;
    if entries.len() == CAPACITY {
        entries.remove(0);
    }
    entries.push(value);
}

pub(super) fn parse(
    session: &mut Session,
    source: &str,
    eof: bool,
    line: usize,
) -> Result<Program, parser::Error> {
    // Keep a bounded working set and promote only repeated source. A multiline function
    // or generated conditional can exceed a page without being a large script.
    const MAX_CACHED_SOURCE: usize = 16 * 1024;
    if source.len() > MAX_CACHED_SOURCE {
        return parser::session_source_at(source, eof, session, line).map(Program::Fresh);
    }
    if let Some(statement) = parser::literal_assignment(source, line) {
        return Ok(Program::Fresh(vec![statement]));
    }
    // Alias replacement can rewrite source locations and emit parse-time
    // diagnostics. An unfinished here-document can warn every time it is read.
    if session.reads_aliases() || source.contains('<') {
        return parser::session_source_at(source, eof, session, line).map(Program::Fresh);
    }
    if let Some(program) = cached(session, source, eof, line) {
        return Ok(program);
    }
    let program = parser::session_source_at(source, eof, session, line)?;
    if let Some(statement) = program.first() {
        let comments = session.read_comments();
        let cache = &mut session.eval_cache;
        let text = statement.location.full_source();
        if cache.seen.iter().rev().any(|seen| &**seen == source) {
            let entry = Rc::new(Entry {
                source: text.clone(),
                program,
                line,
                eof,
                extglob: session.shopt.glob.extglob,
                extquote: session.shopt.extquote,
                posix: session.posix,
                legacy_time: session.posix && session.shopt.compat <= 41,
                comments,
            });
            remember(&mut cache.parsed, entry.clone());
            return Ok(Program::Cached(entry));
        } else {
            // Generated strings often change on every call. Keep the parser's
            // source allocation and only save an AST after the text recurs.
            remember(&mut cache.seen, text.clone());
        }
    }
    Ok(Program::Fresh(program))
}

fn cached(session: &Session, source: &str, eof: bool, line: usize) -> Option<Program> {
    let extglob = session.shopt.glob.extglob;
    let comments = session.read_comments();
    session
        .eval_cache
        .parsed
        .iter()
        .rev()
        .find(|entry| {
            entry.line == line
                && entry.eof == eof
                && entry.extglob == extglob
                && entry.extquote == session.shopt.extquote
                && entry.posix == session.posix
                && entry.legacy_time == (session.posix && session.shopt.compat <= 41)
                && entry.comments == comments
                && &*entry.source == source
        })
        .cloned()
        .map(Program::Cached)
}

// A previously parsed single-line trap can execute its cached program directly.
// Changes to aliases, parser options or verbose input use the normal reader.
pub(crate) fn trap_text(session: &mut Session, source: &str, status: u8) -> Flow {
    if !session.verbose
        && !source.contains('\n')
        && !session.reads_aliases()
        && let Some(program) = cached(session, source, true, 1)
    {
        return match program.run(session, status, false) {
            Flow::Discard(code) => Flow::Continue(code),
            other => other,
        };
    }
    super::text(session, source, status, false, false, Some(1))
}
