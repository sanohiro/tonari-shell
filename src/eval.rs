use crate::Session;
use crate::execute::{self, Flow};
use crate::parser::{CasePattern, Command, Statement};

pub fn execute_program(
    session: &mut Session,
    program: Vec<Statement>,
    mut status: u8,
    replace_last: bool,
) -> Flow {
    let mut statements = program.into_iter().peekable();
    while let Some(statement) = statements.next() {
        if !statement.condition.allows(status) {
            continue;
        }
        let negated = statement.negated;
        let tested = negated
            || statements
                .peek()
                .is_some_and(|s| s.condition != crate::parser::Condition::Always);
        session.tested_depth += u32::from(tested);
        let flow = command(
            session,
            statement.command,
            status,
            replace_last && statements.peek().is_none() && !negated,
        );
        session.tested_depth -= u32::from(tested);
        match flow {
            Flow::Continue(code)
                if code != 0 && session.errexit && !tested && session.tested_depth == 0 =>
            {
                return Flow::Exit(code);
            }
            Flow::Continue(code) => status = if negated { u8::from(code == 0) } else { code },
            other => return other,
        }
    }
    Flow::Continue(status)
}

fn condition_program(session: &mut Session, condition: Vec<Statement>, status: u8) -> Flow {
    session.tested_depth += 1;
    let result = execute_program(session, condition, status, false);
    session.tested_depth -= 1;
    result
}

pub(crate) fn command(session: &mut Session, command: Command, status: u8, tail: bool) -> Flow {
    let scope = session.substitutions.scope();
    let result = run_command(session, command, status, tail);
    session.substitutions.finish(scope);
    result
}

fn run_command(session: &mut Session, command: Command, status: u8, tail: bool) -> Flow {
    match command {
        Command::Pipeline(stages) => {
            execute::run(stages, status, tail, session).unwrap_or_else(|e| {
                eprintln!("tonari: {e}");
                Flow::Continue(1)
            })
        }
        Command::If {
            branches,
            otherwise,
        } => {
            let mut last = status;
            for (condition, body) in branches {
                match condition_program(session, condition, last) {
                    Flow::Continue(0) => return execute_program(session, body, 0, tail),
                    Flow::Continue(code) => last = code,
                    other => return other,
                }
            }
            if otherwise.is_empty() {
                Flow::Continue(0)
            } else {
                execute_program(session, otherwise, last, tail)
            }
        }
        Command::While {
            until,
            condition,
            body,
        } => repeat(session, &body, status, |s, last| {
            match condition_program(s, condition.clone(), last) {
                Flow::Continue(code) => Ok(((code == 0) != until).then_some(code)),
                other => Err(other),
            }
        }),
        Command::For { name, words, body } => {
            let items = match words {
                None => session.positional.clone(),
                Some(words) => {
                    let captured = std::cell::Cell::new(None);
                    let mut env = crate::expansion::Expansion::new(session, status, &captured);
                    let mut fields = Vec::new();
                    for word in words {
                        match env.fields(word) {
                            Ok(items) => fields.extend(items),
                            Err(error) => {
                                eprintln!("tonari: {error}");
                                return error.flow(env.session);
                            }
                        }
                    }
                    fields
                }
            };
            let mut items = items.into_iter();
            repeat(session, &body, status, |s, last| {
                Ok(items.next().map(|value| {
                    s.assign(&name, &value);
                    last
                }))
            })
        }
        Command::Test(expr) => crate::test_expression::evaluate(expr, session, status),
        Command::Arithmetic(expr) => crate::arithmetic::command(&expr, session, status),
        Command::ForArithmetic {
            init,
            condition,
            step,
            body,
        } => {
            let mut first = true;
            if let Err(e) = crate::arithmetic::expanded(&init, session, status) {
                eprintln!("tonari: {e}");
                return Flow::Continue(1);
            }
            repeat(session, &body, status, |s, last| {
                let mut arithmetic = |expr: &str| {
                    crate::arithmetic::expanded(expr, s, last).map_err(|e| {
                        eprintln!("tonari: {e}");
                        Flow::Continue(1)
                    })
                };
                if !first && !step.is_empty() {
                    arithmetic(&step)?;
                }
                first = false;
                Ok((condition.trim().is_empty() || arithmetic(&condition)? != 0).then_some(last))
            })
        }
        Command::Background(mut body) => {
            if body.len() == 1
                && !body[0].negated
                && matches!(body[0].command, Command::Pipeline(_))
            {
                let Command::Pipeline(stages) = body.pop().unwrap().command else {
                    unreachable!()
                };
                execute::run_pipeline(stages, status, false, session, true).unwrap_or_else(|e| {
                    eprintln!("tonari: {e}");
                    Flow::Continue(1)
                })
            } else {
                crate::subshell::launch_group(session, body, status, true)
            }
        }
        Command::Group(body) => execute_program(session, body, status, tail),
        Command::Subshell(body) => crate::subshell::launch_group(session, body, status, false),
        Command::Case { word, arms } => {
            let value = match expand_case(session, word, status, false) {
                Ok(v) => v,
                Err(f) => return f,
            };
            for (patterns, body) in arms.iter() {
                for pattern in patterns {
                    let matched = match pattern {
                        CasePattern::Fixed(matcher) => matcher.matches(&value),
                        CasePattern::Dynamic(word) => {
                            match expand_case(session, word.clone(), status, true) {
                                Ok(glob) => crate::pattern::matches(&value, &glob),
                                Err(f) => return f,
                            }
                        }
                    };
                    if matched {
                        return if body.is_empty() {
                            Flow::Continue(0)
                        } else {
                            execute_program(session, body.clone(), status, tail)
                        };
                    }
                }
            }
            Flow::Continue(0)
        }
        Command::Function { name, body } => {
            session.function_sources.insert(
                name.clone(),
                session
                    .source_names
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "environment".into()),
            );
            session.functions.insert(name, std::rc::Rc::new(body));
            Flow::Continue(0)
        }
    }
}

pub(crate) fn expand_case(
    session: &mut Session,
    word: crate::word::Word,
    status: u8,
    pattern: bool,
) -> Result<String, Flow> {
    let captured = std::cell::Cell::new(None);
    let mut env = crate::expansion::Expansion::new(session, status, &captured);
    let result = if pattern {
        env.pattern(word)
    } else {
        env.scalar(word)
    };
    result.map_err(|e| {
        eprintln!("tonari: {e}");
        e.flow(session)
    })
}

fn repeat(
    session: &mut Session,
    body: &[Statement],
    mut last: u8,
    mut next: impl FnMut(&mut Session, u8) -> Result<Option<u8>, Flow>,
) -> Flow {
    session.loop_depth += 1;
    let mut result = 0;
    let flow = loop {
        if crate::interrupted() {
            break Flow::Continue(130);
        }
        let step = match next(session, last) {
            Ok(None) => break Flow::Continue(result),
            Ok(Some(status)) => execute_program(session, body.to_vec(), status, false),
            Err(flow) => flow,
        };
        match step {
            Flow::Continue(code) => {
                result = code;
                last = code;
            }
            Flow::Break(1) => break Flow::Continue(0),
            Flow::LoopContinue(1) => {
                result = 0;
                last = 0;
            }
            Flow::Break(n) => break Flow::Break(n - 1),
            Flow::LoopContinue(n) => break Flow::LoopContinue(n - 1),
            flow => break flow,
        }
    };
    session.loop_depth -= 1;
    flow
}

pub fn call_function(session: &mut Session, name: &str, args: &[String], status: u8) -> Flow {
    if session.function_depth >= 128 {
        eprintln!("tonari: {name}: function nesting limit exceeded");
        return Flow::Continue(1);
    }
    let body = session.functions[name].clone();
    let source = session
        .function_sources
        .get(name)
        .cloned()
        .unwrap_or_default();
    session.source_names.push(source);
    session.function_names.push(name.to_owned());
    session.enter_function();
    let old = std::mem::replace(&mut session.positional, args[1..].to_vec());
    let flow = execute_program(session, body.as_ref().clone(), status, false);
    session.positional = old;
    session.leave_function();
    session.function_names.pop();
    session.source_names.pop();
    match flow {
        Flow::Return(code) => Flow::Continue(code),
        other => other,
    }
}
