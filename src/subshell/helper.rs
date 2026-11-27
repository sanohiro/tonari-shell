use crate::Session;
use crate::expansion::Expansion;
use crate::parser::{CaseEnd, CasePattern, Command, Statement};
use crate::word::{Part, Word};
use std::cell::Cell;

// Only read positional scalars and previously established locals. The overlay
// never installs a shell variable, so abandoning a branch before output is safe.
fn scalar(word: &Word, known_local: &impl Fn(&str) -> bool, quoted: bool) -> bool {
    let known = |name: &str| {
        (!name.is_empty() && name.bytes().all(|b| b.is_ascii_digit())) || known_local(name)
    };
    match word {
        Word::Literal(_) => true,
        Word::Expanded(parts) | Word::Declaration(parts) => parts.iter().all(|part| match part {
            Part::Text(_) => true,
            Part::Parameter { name, quoted: q } => (!quoted || *q) && known(name),
            Part::Trim {
                name,
                quoted: q,
                pattern,
                ..
            } => (!quoted || *q) && known(name) && (pattern.as_literal().is_some() || matches!(pattern.as_ref(), Word::Expanded(parts) if parts.iter().all(|p| matches!(p, Part::Text(_) | Part::Unquoted(_) | Part::Pattern(_))))),
            _ => false,
        }),
    }
}

fn builtin<'a>(body: &'a [Statement], session: &Session) -> Option<(&'a str, &'a [Word])> {
    let cmd = super::sole_plain_command(body)?;
    let (name, args) = cmd.words.split_first()?;
    let name = name.as_literal()?;
    (!session.disabled_builtins.contains(name) && !session.functions.contains_key(name))
        .then_some((name, args))
}

fn output_arm<'a>(statement: &'a Statement, env: &mut Expansion<'_>) -> Option<&'a [Statement]> {
    use crate::parser::Condition;
    if statement.negated || statement.condition != Condition::Always {
        return None;
    }
    match &statement.command {
        Command::Case(case) if scalar(&case.word, &|key| env.shadows(key), false) => {
            let value = env.scalar(case.word.clone()).ok()?;
            for (patterns, body, end) in case.arms.iter() {
                if *end != CaseEnd::Stop {
                    return None;
                }
                for pattern in patterns {
                    let CasePattern::Fixed(matcher) = pattern else {
                        return None;
                    };
                    if matcher.matches_session(&value, env.session) {
                        return Some(body);
                    }
                }
            }
            Some(&[])
        }
        Command::If {
            branches,
            otherwise,
        } => {
            for (condition, body) in branches {
                let (name, args) = builtin(condition, env.session)?;
                let args = match (name, args) {
                    ("[", [args @ .., end]) if end.as_literal() == Some("]") => args,
                    ("test", args) => args,
                    _ => return None,
                };
                let [op, operand] = args else { return None };
                let op = op.as_literal()?;
                if !matches!(op, "-n" | "-z") || !scalar(operand, &|key| env.shadows(key), true) {
                    return None;
                }
                if env.scalar(operand.clone()).ok()?.is_empty() == (op == "-z") {
                    return Some(body);
                }
            }
            Some(otherwise)
        }
        _ => None,
    }
}

pub(super) fn capture(
    session: &mut Session,
    body: &[Statement],
    arguments: &[Word],
    last: u8,
) -> Option<Result<(String, u8), &'static str>> {
    if !super::can_elide_child(session)
        || session.traps.has_explicit_traps()
        || session.keyword
        || session.shopt.nocasematch
        || session.shopt.localvar_inherit
        || session.noexec
        || !arguments.iter().all(|arg| {
            scalar(
                arg,
                &|key| key != "_" && session.stored_scalar(key).is_some(),
                true,
            )
        })
    {
        return None;
    }
    let (branch, declarations) = body.split_last()?;
    if declarations.is_empty() {
        return None;
    }
    let captured = Cell::new(None);
    let mut env = Expansion::new(session, last, &captured);
    let mut args = Vec::with_capacity(arguments.len());
    for arg in arguments {
        env.append_fields(arg.clone(), &mut args).ok()?;
    }
    let outer = std::mem::replace(&mut session.positional, args.into_iter());
    let result = (|| {
        let mut locals = Vec::with_capacity(declarations.len());
        for statement in declarations {
            let ("local", [word]) = builtin(std::slice::from_ref(statement), session)? else {
                return None;
            };
            if !scalar(word, &|key| locals.iter().any(|(k, _)| k == key), false) {
                return None;
            }
            let first = match word {
                Word::Literal(text) => text,
                Word::Declaration(parts) => match parts.first()? {
                    Part::Text(text) => text,
                    _ => return None,
                },
                _ => return None,
            };
            let (key, _) = first.split_once('=')?;
            if !session.capture_local(key) || locals.iter().any(|(name, _)| name == key) {
                return None;
            }
            let mut rhs = match word {
                Word::Declaration(parts) => Word::Expanded(parts.clone()),
                _ => word.clone(),
            };
            rhs.remove_assignment_prefix(key.len() + 1);
            let value = Expansion::with_assignments(session, last, &captured, &mut locals, false)
                .scalar(rhs)
                .ok()?;
            locals.push((key.to_owned(), value));
        }
        // Check field shapes before evaluating the selected output builtin.
        let mut env = Expansion::with_assignments(session, 0, &captured, &mut locals, false);
        let body = output_arm(branch, &mut env)?;
        if body.is_empty() {
            return Some(Ok((String::new(), 0)));
        }
        let (name, operands) = builtin(body, env.session)?;
        if !matches!(name, "echo" | "printf")
            || !operands
                .iter()
                .all(|w| scalar(w, &|key| env.shadows(key), true))
            || name == "printf"
                && !matches!(
                    operands.first()?.as_literal(),
                    Some("%s" | "%s\\n" | "%s\n")
                )
        {
            return None;
        }
        let mut fields = Vec::with_capacity(operands.len());
        for operand in operands {
            env.append_fields(operand.clone(), &mut fields).ok()?;
        }
        let mut out = Vec::new();
        let status = if name == "echo" {
            crate::builtins::echo_into(&fields, &mut out, env.session);
            0
        } else {
            crate::session::printf::run(env.session, &fields, Some(&mut out))
        };
        Some(super::captured_output(out, status))
    })();
    session.positional = outer;
    result
}
