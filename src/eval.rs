mod select;
mod timing;

use crate::Session;
use crate::execute::{self, Flow};
use crate::parser::{CasePattern, Command, Statement};
use std::borrow::Cow;

pub fn execute_program(
    session: &mut Session,
    program: Vec<Statement>,
    status: u8,
    replace_last: bool,
) -> Flow {
    run_statements(
        session,
        program.into_iter().map(Cow::Owned),
        status,
        replace_last,
        &mut [],
    )
}

fn run_statements<'a>(
    session: &mut Session,
    iter: impl Iterator<Item = Cow<'a, Statement>>,
    mut status: u8,
    replace_last: bool,
    literals: &mut [Option<Box<execute::PreparedCommand>>],
) -> Flow {
    let mut literals = literals.iter_mut();
    let mut statements = iter.peekable();
    while let Some(statement) = statements.next() {
        if session.noexec {
            return Flow::Continue(0);
        }
        if session.notify && session.jobs.awaiting_notification() {
            session.jobs.poll();
            session.jobs.report_changes(session.posix);
        }
        if !session.shopt.globasciiranges {
            crate::pattern::sync_ranges(session);
        }
        let literal = literals.next().and_then(Option::as_deref_mut);
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
            session.current_command = Some(statement.location.clone());
        } else if session
            .traps
            .in_action_child(session.function_depth + session.source_depth)
            || session.signal_line.is_some() && !session.traps.in_nested_action()
        {
            session.signal_line = Some(statement.location.line);
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
        let flow = if let Some(flow) = literal.and_then(|cmd| cmd.run_literal(session, status)) {
            single_status(session, &flow);
            flow
        } else {
            dispatch(
                session,
                match statement {
                    Cow::Owned(stmt) => Cow::Owned(stmt.command),
                    Cow::Borrowed(stmt) => Cow::Borrowed(&stmt.command),
                },
                status,
                replace_last && statements.peek().is_none() && !negated && !session.traps.active(),
            )
        };
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

pub(crate) fn borrowed_program(
    session: &mut Session,
    body: &[Statement],
    status: u8,
    tail: bool,
) -> Flow {
    run_statements(
        session,
        body.iter().map(Cow::Borrowed),
        status,
        tail,
        &mut [],
    )
}

fn condition_program(session: &mut Session, condition: &[Statement], status: u8) -> Flow {
    session.tested_depth += 1;
    let result = borrowed_program(session, condition, status, false);
    session.tested_depth -= 1;
    result
}

pub(crate) fn command(session: &mut Session, command: Command, status: u8, tail: bool) -> Flow {
    dispatch(session, Cow::Owned(command), status, tail)
}

fn dispatch(session: &mut Session, command: Cow<'_, Command>, status: u8, tail: bool) -> Flow {
    if matches!(command.as_ref(), Command::Arithmetic(_) | Command::Test(_))
        && let Some(flow) = crate::traps::debug(session, status)
    {
        return flow;
    }
    let scope = session.substitutions.scope();
    let single = match command.as_ref() {
        Command::Pipeline(stages) => stages.len() == 1,
        Command::Subshell(_) | Command::Test(_) | Command::Arithmetic(_) => true,
        _ => false,
    };
    let result = match command {
        Cow::Borrowed(Command::Pipeline(stages)) if stages.len() == 1 => {
            execute::run_single(Cow::Borrowed(&stages[0]), status, tail, session)
                .unwrap_or_else(execution_error)
        }
        Cow::Borrowed(Command::Subshell(body)) => {
            crate::subshell::launch_borrowed(session, body, status)
        }
        Cow::Borrowed(Command::If {
            branches,
            otherwise,
        }) => run_if(
            session,
            branches
                .iter()
                .map(|(condition, body)| (condition.as_slice(), body.as_slice())),
            otherwise.as_slice(),
            status,
            tail,
            borrowed_program,
        ),
        Cow::Borrowed(Command::Function { name, definition })
            if !(session.allexport
                || session.readonly_functions.contains(name)
                || session.posix && posix_function_error(name).is_some())
                && session.functions.get(name).is_some_and(|old| {
                    std::rc::Rc::ptr_eq(&old.definition, definition)
                        && old.source.as_ref() == session.function_source()
                }) =>
        {
            Flow::Continue(0)
        }
        Cow::Borrowed(Command::While {
            until,
            condition,
            body,
        }) => run_while(session, *until, condition, body, status),
        Cow::Borrowed(Command::For(c)) => run_for(
            session,
            c.select,
            &c.header,
            &c.name,
            c.words.clone(),
            &c.body,
            status,
        ),
        Cow::Borrowed(Command::ForArithmetic(c)) => {
            run_arithmetic_loop(session, &c.init, &c.condition, &c.step, &c.body, status)
        }
        Cow::Borrowed(Command::Case(c)) => run_case(
            session,
            &c.header,
            Cow::Borrowed(&c.word),
            &c.arms,
            status,
            tail,
        ),
        Cow::Borrowed(Command::Group(body)) => borrowed_program(session, body, status, tail),
        Cow::Borrowed(Command::Arithmetic(expr)) => {
            crate::arithmetic::command(expr, session, status, true)
        }
        Cow::Borrowed(Command::Test(expr)) => {
            crate::test_expression::evaluate(expr, session, status)
        }
        _ => run_command(session, command.into_owned(), status, tail),
    };
    if single {
        single_status(session, &result);
    }
    session.substitutions.finish(scope);
    result
}

#[cold]
fn execution_error(error: std::io::Error) -> Flow {
    diagnostic!("{error}");
    Flow::Continue(1)
}

fn single_status(session: &mut Session, flow: &Flow) {
    if let Flow::Discard(code) | Flow::Continue(code) | Flow::Exit(code) | Flow::Return(code) = flow
    {
        session.pipe_status.single(*code);
    }
}

fn run_if<P: AsRef<[Statement]>>(
    session: &mut Session,
    branches: impl IntoIterator<Item = (P, P)>,
    otherwise: P,
    status: u8,
    tail: bool,
    run: impl Fn(&mut Session, P, u8, bool) -> Flow,
) -> Flow {
    let mut last = status;
    for (condition, body) in branches {
        session.tested_depth += 1;
        let flow = run(session, condition, last, false);
        session.tested_depth -= 1;
        match flow {
            Flow::Continue(0) => return run(session, body, 0, tail),
            Flow::Continue(code) => last = code,
            other => return other,
        }
    }
    if otherwise.as_ref().is_empty() {
        Flow::Continue(0)
    } else {
        run(session, otherwise, last, tail)
    }
}

fn run_while(
    session: &mut Session,
    until: bool,
    condition: &[Statement],
    body: &[Statement],
    status: u8,
) -> Flow {
    repeat(session, body, status, |s, last| {
        match condition_program(s, condition, last) {
            Flow::Continue(code) => Ok(((code == 0) != until).then_some(code)),
            other => Err(other),
        }
    })
}

fn run_for(
    session: &mut Session,
    select: bool,
    header: &crate::parser::Location,
    name: &str,
    words: Option<Vec<crate::word::Word>>,
    body: &[Statement],
    status: u8,
) -> Flow {
    if !crate::word::is_identifier(name) {
        diagnostic!("{name}: invalid loop variable");
        return Flow::Continue(1);
    }
    let implicit_words = words.is_none();
    let items = match words {
        None => session.positional.as_slice().to_vec(),
        Some(words) => {
            let captured = std::cell::Cell::new(None);
            let mut env = crate::expansion::Expansion::new(session, status, &captured);
            let mut fields = Vec::new();
            for word in words {
                if let Err(error) = env.append_fields(word, &mut fields) {
                    error.report("");
                    return error.flow(env.session);
                }
            }
            fields
        }
    };
    if select {
        if session.traps.debug_handler() {
            session.current_command = Some(header.clone());
            if let Some(flow) = crate::traps::debug(session, status) {
                return flow;
            }
        }
        return select::run(session, name, &items, body, status);
    }
    let mut items = items.into_iter();
    repeat(session, body, status, |s, last| {
        let Some(value) = items.next() else {
            return Ok(None);
        };
        if s.traps.debug_handler() {
            s.current_command = Some(header.clone());
            if let Some(flow) = crate::traps::debug(s, last) {
                return Err(flow);
            }
        }
        if s.xtrace {
            execute::trace_line(s, last, |line| {
                line.push_str(header.text());
                if implicit_words {
                    line.push_str(" in \"$@\"");
                }
            });
        }
        if s.assign_existing_literal(name, &value) {
            return Ok(Some(last));
        }
        let error = if s.is_nameref(name) {
            s.assign_for_reference(name, value).err()
        } else if s.readonly(name) {
            Some(crate::session::READONLY)
        } else if s.integer(name) {
            if let Err(error) = s.assign_target(name, &value) {
                diagnostic!("{name}: {error}");
                return Err(Flow::Discard(1));
            }
            None
        } else {
            s.assign(name, &value);
            None
        };
        if let Some(error) = error {
            diagnostic!("{name}: {error}");
            return Err(
                if error == crate::session::READONLY && s.posix && !s.interactive {
                    Flow::Exit(1)
                } else {
                    Flow::Continue(1)
                },
            );
        }
        Ok(Some(last))
    })
}

fn run_arithmetic_loop(
    session: &mut Session,
    init: &str,
    condition: &str,
    step: &str,
    body: &[Statement],
    status: u8,
) -> Flow {
    let header = session.current_command.clone();
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
        let result = if s.xtrace {
            let expr = expr.trim_start();
            if expr.is_empty() {
                crate::arithmetic::traced("1", s, last, None)
            } else {
                crate::arithmetic::traced(expr, s, last, Some(compiled))
            }
        } else {
            compiled.run(s, last)
        };
        result.map_err(|e| {
            diagnostic!("{e}");
            Flow::Continue(1)
        })
    };
    if (session.xtrace || !init.trim().is_empty())
        && let Err(flow) = arithmetic(
            session,
            init,
            &crate::arithmetic::Repeated::Other(init),
            status,
        )
    {
        return flow;
    }
    let test = crate::arithmetic::Repeated::new(condition);
    let update = crate::arithmetic::Repeated::new(step);
    repeat(session, body, status, |s, last| {
        if !first && !s.traps.debug_handler() {
            s.current_command.clone_from(&header);
        }
        if !first && (s.xtrace || !step.is_empty()) {
            arithmetic(s, step, &update, last)?;
        }
        first = false;
        Ok(((!s.xtrace && condition.trim().is_empty())
            || arithmetic(s, condition, &test, last)? != 0)
            .then_some(last))
    })
}

fn run_case(
    session: &mut Session,
    header: &crate::parser::Location,
    word: Cow<'_, crate::word::Word>,
    arms: &[crate::parser::CaseArm],
    status: u8,
    tail: bool,
) -> Flow {
    if session.traps.debug_handler() {
        session.current_command = Some(crate::parser::Location::label(
            format!("{} ", header.text()),
            header.line,
        ));
        if let Some(flow) = crate::traps::debug(session, status) {
            return flow;
        }
    }
    if session.xtrace {
        execute::trace_line(session, status, |line| line.push_str(header.text()));
    }
    let value = match word
        .stored_parameter(session)
        .map(|s| Ok(s.to_owned()))
        .unwrap_or_else(|| expand_case(session, word.into_owned(), status, false))
    {
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
                    CasePattern::Extended(_, matcher) if session.shopt.nocasematch => matcher
                        .insensitive(
                            &value,
                            session.shopt.glob.extglob,
                            session.character_locale(),
                        ),
                    CasePattern::Fixed(matcher) => matcher.matches_session(&value, session),
                    CasePattern::Extended(group, basic) => {
                        if session.shopt.glob.extglob {
                            group.accepts(&value, false)
                        } else {
                            basic.matches_session(&value, session)
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
                match borrowed_program(session, body, last, tail && stop) {
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
            execute::run(stages, status, tail, session).unwrap_or_else(execution_error)
        }
        Command::If {
            branches,
            otherwise,
        } => run_if(session, branches, otherwise, status, tail, execute_program),
        Command::While {
            until,
            condition,
            body,
        } => run_while(session, until, &condition, &body, status),
        Command::For(c) => run_for(
            session, c.select, &c.header, &c.name, c.words, &c.body, status,
        ),
        Command::Test(expr) => crate::test_expression::evaluate(&expr, session, status),
        Command::Arithmetic(expr) => crate::arithmetic::command(&expr, session, status, false),
        Command::ForArithmetic(c) => {
            run_arithmetic_loop(session, &c.init, &c.condition, &c.step, &c.body, status)
        }
        Command::Coprocess { name, body } => {
            crate::execute::coprocess::launch(session, name, *body, status).unwrap_or_else(|e| {
                diagnostic!("coproc: {e}");
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
                    diagnostic!("{e}");
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
        Command::Case(c) => run_case(
            session,
            &c.header,
            Cow::Owned(c.word),
            &c.arms,
            status,
            tail,
        ),
        Command::Function { name, definition } => {
            if session.posix {
                if let Some(reason) = posix_function_error(&name) {
                    diagnostic!("{name}: {reason}");
                    return if session.interactive {
                        Flow::Continue(2)
                    } else {
                        Flow::Exit(2)
                    };
                }
            } else if !crate::parser::is_function_name(&name) {
                diagnostic!("{name}: invalid function name");
                return Flow::Continue(1);
            }
            if session.readonly_functions.contains(&name) {
                diagnostic!("{name}: readonly function");
                return Flow::Continue(1);
            }
            session.define_function(name, definition);
            Flow::Continue(0)
        }
    }
}

fn posix_function_error(name: &str) -> Option<&'static str> {
    if !crate::word::is_identifier(name) {
        Some("invalid function name")
    } else if crate::execute::special_builtin(name) {
        Some("special builtin cannot be redefined")
    } else {
        None
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
        e.report("");
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
    let mut iterations = 0u8;
    let mut literals = Vec::new();
    let flow = loop {
        if session.noexec {
            break Flow::Continue(0);
        }
        if crate::interrupted() {
            break Flow::Continue(130);
        }
        let step = match next(session, last) {
            Ok(None) => break Flow::Continue(result),
            Ok(Some(status)) => {
                if iterations == 2 {
                    // Shell lookup stays live. Only immutable argument strings are reused.
                    literals = body
                        .iter()
                        .map(|stmt| execute::PreparedCommand::literal_builtin(stmt, session))
                        .collect();
                }
                iterations = iterations.saturating_add(1);
                run_statements(
                    session,
                    body.iter().map(Cow::Borrowed),
                    status,
                    false,
                    &mut literals,
                )
            }
            Err(Flow::Continue(code)) => break Flow::Continue(code),
            Err(flow) => flow,
        };
        match step {
            Flow::Continue(code) => {
                result = code;
                last = code;
            }
            Flow::Break(1, code) => break Flow::Continue(code),
            Flow::LoopContinue(1) => {
                result = 0;
                last = 0;
            }
            Flow::Break(n, code) => break Flow::Break(n - 1, code),
            Flow::LoopContinue(n) => break Flow::LoopContinue(n - 1),
            flow => break flow,
        }
    };
    session.loop_depth -= 1;
    flow
}

pub fn call_function(
    session: &mut Session,
    argv: Vec<String>,
    status: u8,
    replace_last: bool,
) -> Flow {
    call_with_bindings(session, argv, status, replace_last, &mut Default::default())
}

pub(crate) fn call_with_bindings(
    session: &mut Session,
    argv: Vec<String>,
    status: u8,
    replace_last: bool,
    bindings: &mut crate::session::Bindings,
) -> Flow {
    let mut arguments = argv.into_iter();
    let name = arguments.next().expect("function name");
    if session.function_nesting_full() {
        diagnostic!("{name}: function nesting limit exceeded");
        return Flow::Discard(1);
    }
    let function = session.functions[&name].clone();
    let traced = session.traced_functions.contains(&name);
    let callsite = session.current_command.clone();
    let caller_signal_line = session.signal_line;
    session.call_lines.push(session.line());
    session.source_names.push(function.source);
    session.function_names.push(name);
    let outer_loop = session.loop_depth;
    if session.shopt.compat > 43 {
        session.loop_depth = 0;
    }
    session.enter_function(bindings);
    let tracked = session.shopt.extdebug;
    if tracked {
        session
            .debug_base
            .get_or_init(|| session.positional.as_slice().to_vec());
        session.debug_args.push(arguments.as_slice().to_vec());
    } else if session.shopt.compat <= 44 {
        session.debug_base.get_or_init(Vec::new);
    }
    let outer_set_args = std::mem::take(&mut session.source_set_args);
    let old = std::mem::replace(&mut session.positional, arguments);
    let hidden = session
        .traps
        .enter_function(session.errtrace, session.functrace || traced);
    let entry = if session.traps.debug_handler() {
        let old = session
            .signal_line
            .replace(function.definition.location.line);
        let flow = crate::traps::debug(session, status);
        session.signal_line = old;
        flow
    } else {
        None
    };
    let flow = entry.unwrap_or_else(|| {
        run_statements(
            session,
            function
                .definition
                .body
                .statements()
                .iter()
                .map(Cow::Borrowed),
            status,
            replace_last,
            &mut [],
        )
    });
    if matches!(flow, Flow::Continue(_)) && session.traps.return_handler() {
        session.signal_line = Some(function.definition.location.line);
    }
    let flow = crate::traps::returning(session, flow);
    session.traps.leave_function(hidden);
    session.signal_line = caller_signal_line;
    session.positional = old;
    session.source_set_args = outer_set_args;
    if tracked {
        session.debug_args.pop();
    }
    session.leave_function();
    session.loop_depth = outer_loop;
    if matches!(flow, Flow::Continue(1..) | Flow::Return(1..))
        && session.traps.error_handler()
        && let (Some(inner), Some(caller)) = (&mut session.current_command, &callsite)
    {
        // ERR sees the failed command's text, with LINENO at the call site.
        inner.line = caller.line;
    } else {
        session.current_command = callsite;
    }
    session.call_lines.pop();
    session.function_names.pop();
    session.source_names.pop();
    match flow {
        Flow::Return(code) => Flow::Continue(code),
        other => other,
    }
}
