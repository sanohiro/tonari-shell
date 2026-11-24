mod select;
mod timing;

use crate::Session;
use crate::execute::{self, Flow};
use crate::parser::{CasePattern, Command, Statement};

pub fn execute_program(
    session: &mut Session,
    program: Vec<Statement>,
    status: u8,
    replace_last: bool,
) -> Flow {
    run_statements(session, &mut program.into_iter(), status, replace_last)
}

fn run_statements(
    session: &mut Session,
    iter: &mut dyn Iterator<Item = Statement>,
    mut status: u8,
    replace_last: bool,
) -> Flow {
    let mut statements = iter.peekable();
    while let Some(statement) = statements.next() {
        if session.interactive && session.completions.in_callback() {
            crate::editor::preview::statement();
        }
        if !session.coprocesses.is_empty() {
            crate::execute::coprocess::refresh(session);
        }
        if session.traps.active() {
            match crate::traps::dispatch(session, status) {
                Flow::Continue(_) => {}
                flow => return flow,
            }
        }
        if !statement.condition.allows(status) {
            continue;
        }
        if !session.traps.in_action() {
            session.current_command = Some(statement.location);
        }
        let negated = statement.negated;
        let tested = negated
            || statements
                .peek()
                .is_some_and(|s| s.condition != crate::parser::Condition::Always);
        session.tested_depth += u32::from(tested);
        // Compound lists test each command in their body. A failed condition
        // such as { false && :; } must not fire ERR again at the closing brace.
        let subject = match &statement.command {
            Command::Timed { command, .. } => command.as_ref(),
            command => command,
        };
        let reports_failure = match subject {
            Command::Pipeline(stages) => stages.len() != 1 || stages[0].compound.is_none(),
            Command::Subshell(_) | Command::Test(_) | Command::Arithmetic(_) => true,
            _ => false,
        };
        let flow = command(
            session,
            statement.command,
            status,
            replace_last && statements.peek().is_none() && !negated && !session.traps.active(),
        );
        session.tested_depth -= u32::from(tested);
        match flow {
            Flow::Discard(code) if session.errexit && !tested && session.tested_depth == 0 => {
                return Flow::Exit(code);
            }
            Flow::Continue(code) => {
                if code != 0 && reports_failure && !tested && session.tested_depth == 0 {
                    if session.traps.active() {
                        let flow = crate::traps::error(session, code);
                        if !matches!(flow, Flow::Continue(_)) {
                            return flow;
                        }
                    }
                    if session.errexit {
                        return Flow::Exit(code);
                    }
                }
                status = if negated { u8::from(code == 0) } else { code };
            }
            other => return other,
        }
    }
    if session.traps.active() {
        crate::traps::dispatch(session, status)
    } else {
        Flow::Continue(status)
    }
}

fn condition_program(session: &mut Session, condition: Vec<Statement>, status: u8) -> Flow {
    session.tested_depth += 1;
    let result = execute_program(session, condition, status, false);
    session.tested_depth -= 1;
    result
}

pub(crate) fn command(session: &mut Session, command: Command, status: u8, tail: bool) -> Flow {
    if matches!(command, Command::Arithmetic(_) | Command::Test(_))
        && let Some(flow) = crate::traps::debug(session, status)
    {
        return flow;
    }
    let scope = session.substitutions.scope();
    let single = match &command {
        Command::Pipeline(stages) => stages.len() == 1,
        Command::Subshell(_) | Command::Test(_) | Command::Arithmetic(_) => true,
        _ => false,
    };
    let result = run_command(session, command, status, tail);
    if single {
        if let Flow::Discard(code) | Flow::Continue(code) | Flow::Exit(code) | Flow::Return(code) =
            result
        {
            session.pipe_status.single(code);
        }
    }
    session.substitutions.finish(scope);
    result
}

fn run_command(session: &mut Session, command: Command, status: u8, tail: bool) -> Flow {
    match command {
        Command::Timed {
            posix,
            command: inner,
        } => {
            let timer = timing::Timer::start();
            let flow = self::command(session, *inner, status, false);
            timer.report(session, posix);
            flow
        }
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
        Command::For {
            select,
            header,
            name,
            words,
            body,
        } => {
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
            if select {
                if session.traps.debug_handler() {
                    session.current_command = Some(header);
                    if let Some(flow) = crate::traps::debug(session, status) {
                        return flow;
                    }
                }
                return select::run(session, &name, &items, &body, status);
            }
            let mut items = items.into_iter();
            repeat(session, &body, status, |s, last| {
                let Some(value) = items.next() else {
                    return Ok(None);
                };
                if s.traps.debug_handler() {
                    s.current_command = Some(header.clone());
                    if let Some(flow) = crate::traps::debug(s, last) {
                        return Err(flow);
                    }
                }
                if s.readonly(&name) {
                    eprintln!("tonari: {name}: readonly variable");
                    return Err(Flow::Continue(1));
                }
                s.assign(&name, &value);
                Ok(Some(last))
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
            let line = session.line();
            let mut first = true;
            let arithmetic = |s: &mut Session,
                              expr: &str,
                              compiled: &crate::arithmetic::Repeated,
                              last: u8|
             -> Result<i64, Flow> {
                if s.traps.debug_handler() {
                    s.current_command = Some(crate::parser::Location::label(
                        format!("(({}))", expr.trim()),
                        line,
                    ));
                    if let Some(flow) = crate::traps::debug(s, last) {
                        return Err(flow);
                    }
                }
                compiled.run(s, last).map_err(|e| {
                    eprintln!("tonari: {e}");
                    Flow::Continue(1)
                })
            };
            if !init.trim().is_empty()
                && let Err(flow) = arithmetic(
                    session,
                    &init,
                    &crate::arithmetic::Repeated::Other(&init),
                    status,
                )
            {
                return flow;
            }
            let test = crate::arithmetic::Repeated::new(&condition);
            let update = crate::arithmetic::Repeated::new(&step);
            repeat(session, &body, status, |s, last| {
                if !first && !step.is_empty() {
                    arithmetic(s, &step, &update, last)?;
                }
                first = false;
                Ok(
                    (condition.trim().is_empty() || arithmetic(s, &condition, &test, last)? != 0)
                        .then_some(last),
                )
            })
        }
        Command::Coprocess { name, body } => {
            crate::execute::coprocess::launch(session, name, *body, status).unwrap_or_else(|e| {
                eprintln!("tonari: coproc: {e}");
                Flow::Continue(1)
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
                // The background child itself is the explicit subshell. Its PID
                // is exposed as $! and BASH_SUBSHELL increases only once.
                if body.len() == 1
                    && !body[0].negated
                    && matches!(body[0].command, Command::Subshell(_))
                {
                    let Command::Subshell(group) = body.pop().unwrap().command else {
                        unreachable!()
                    };
                    body = group;
                }
                crate::subshell::launch_group(session, body, status, true)
            }
        }
        Command::Group(body) => execute_program(session, body, status, tail),
        Command::Subshell(body) => crate::subshell::launch_group(session, body, status, false),
        Command::Case { header, word, arms } => {
            if session.traps.debug_handler() {
                session.current_command = Some(crate::parser::Location::label(
                    format!("{} ", header.text()),
                    header.line,
                ));
                if let Some(flow) = crate::traps::debug(session, status) {
                    return flow;
                }
            }
            let value = match expand_case(session, word, status, false) {
                Ok(v) => v,
                Err(f) => return f,
            };
            let mut fall_through = false;
            let mut result = 0;
            let mut last = status;
            for (patterns, body, end) in arms.iter() {
                let mut matched = fall_through;
                if !matched {
                    for pattern in patterns {
                        matched = match pattern {
                            CasePattern::Fixed(matcher) if session.shopt.nocasematch => {
                                matcher.insensitive(&value, false, session.character_locale())
                            }
                            CasePattern::Extended(_, matcher) if session.shopt.nocasematch => {
                                matcher.insensitive(
                                    &value,
                                    session.shopt.glob.extglob,
                                    session.character_locale(),
                                )
                            }
                            CasePattern::Fixed(matcher) => matcher.matches(&value),
                            CasePattern::Extended(group, basic) => {
                                if session.shopt.glob.extglob {
                                    group.accepts(&value, false)
                                } else {
                                    basic.matches(&value)
                                }
                            }
                            CasePattern::Dynamic(word) => {
                                match expand_case(session, word.clone(), last, true) {
                                    Ok(glob) if session.shopt.nocasematch => {
                                        crate::pattern::extended::insensitive_match(
                                            &value,
                                            &glob,
                                            session.shopt.glob.extglob,
                                            session.character_locale(),
                                        )
                                    }
                                    Ok(glob) if session.shopt.glob.extglob => {
                                        crate::pattern::extended::matches(&value, &glob)
                                    }
                                    Ok(glob) => crate::pattern::matches(&value, &glob),
                                    Err(f) => return f,
                                }
                            }
                        };
                        if matched {
                            break;
                        }
                    }
                }
                if matched {
                    let stop = *end == crate::parser::CaseEnd::Stop;
                    if !body.is_empty() {
                        match execute_program(session, body.clone(), last, tail && stop) {
                            Flow::Continue(code) => {
                                result = code;
                                last = code;
                            }
                            other => return other,
                        }
                    }
                    if stop {
                        break;
                    }
                    fall_through = *end == crate::parser::CaseEnd::FallThrough;
                }
            }
            Flow::Continue(result)
        }
        Command::Function { name, definition } => {
            if session.readonly_functions.contains(&name) {
                eprintln!("tonari: {name}: readonly function");
                return Flow::Continue(1);
            }
            session.define_function(name, definition);
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
            Ok(Some(status)) => run_statements(session, &mut body.iter().cloned(), status, false),
            Err(Flow::Continue(code)) => break Flow::Continue(code),
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

pub fn call_function(session: &mut Session, mut argv: Vec<String>, status: u8) -> Flow {
    let name = argv.remove(0);
    if session.function_depth >= 128 {
        eprintln!("tonari: {name}: function nesting limit exceeded");
        return Flow::Continue(1);
    }
    let function = session.functions[&name].clone();
    let traced = session.traced_functions.contains(&name);
    let callsite = session.current_command.clone();
    session.call_lines.push(session.line());
    session.source_names.push(function.source);
    session.function_names.push(name);
    session.enter_function();
    let tracked = session.shopt.extdebug;
    if tracked {
        if session.debug_args.is_empty() {
            session.debug_args.push(session.positional.clone());
        }
        session.debug_args.push(argv.clone());
    }
    let old = std::mem::replace(&mut session.positional, argv);
    let hidden = session
        .traps
        .enter_function(session.errtrace, session.functrace || traced);
    let flow = crate::traps::debug(session, status).unwrap_or_else(|| {
        run_statements(
            session,
            &mut function.definition.body.statements().iter().cloned(),
            status,
            false,
        )
    });
    let flow = crate::traps::returning(session, flow);
    session.traps.leave_function(hidden);
    session.positional = old;
    if tracked {
        session.debug_args.pop();
        if session.debug_args.len() == 1 {
            session.debug_args.clear();
        }
    }
    session.leave_function();
    session.current_command = callsite;
    session.call_lines.pop();
    session.function_names.pop();
    session.source_names.pop();
    match flow {
        Flow::Return(code) => Flow::Continue(code),
        other => other,
    }
}
