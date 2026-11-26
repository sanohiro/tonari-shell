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
            eprintln!("tonari: eval: -{flag}: invalid option\neval: usage: eval [arg ...]");
            return Flow::Continue(2);
        }
        _ => args,
    };
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
    seen: [Option<Rc<str>>; 2],
    parsed: [Option<Rc<Entry>>; 2],
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

pub(super) fn parse(
    session: &mut Session,
    source: &str,
    eof: bool,
    line: usize,
) -> Result<Program, parser::Error> {
    if source.len() > 4096 {
        return parser::session_source_at(source, eof, session, line).map(Program::Fresh);
    }
    if let Some(statement) = parser::literal_assignment(source, line) {
        return Ok(Program::Fresh(vec![statement]));
    }
    // Alias replacement can rewrite source locations and emit parse-time
    // diagnostics. An unfinished here-document can warn every time it is read.
    if (session.shopt.expand_aliases && session.aliases.0.is_some()) || source.contains('<') {
        return parser::session_source_at(source, eof, session, line).map(Program::Fresh);
    }
    let extglob = session.shopt.glob.extglob;
    let comments = session.read_comments();
    if let Some(entry) = session.eval_cache.parsed.iter().flatten().find(|entry| {
        entry.line == line
            && entry.eof == eof
            && entry.extglob == extglob
            && entry.extquote == session.shopt.extquote
            && entry.posix == session.posix
            && entry.legacy_time == (session.posix && session.shopt.compat <= 41)
            && entry.comments == comments
            && &*entry.source == source
    }) {
        return Ok(Program::Cached(entry.clone()));
    }
    let program = parser::session_source_at(source, eof, session, line)?;
    if let Some(statement) = program.first() {
        let cache = &mut session.eval_cache;
        let text = statement.location.full_source();
        if cache.seen.iter().flatten().any(|seen| &**seen == source) {
            cache.parsed.rotate_right(1);
            let entry = Rc::new(Entry {
                source: text.clone(),
                program,
                line,
                eof,
                extglob,
                extquote: session.shopt.extquote,
                posix: session.posix,
                legacy_time: session.posix && session.shopt.compat <= 41,
                comments,
            });
            cache.parsed[0] = Some(entry.clone());
            return Ok(Program::Cached(entry));
        } else {
            // Generated strings often change on every call. Keep the parser's
            // source allocation and only save an AST after the text recurs.
            cache.seen.rotate_right(1);
            cache.seen[0] = Some(text.clone());
        }
    }
    Ok(Program::Fresh(program))
}
