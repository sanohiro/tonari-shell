use super::*;

pub(crate) struct ArrayDeclaration {
    pub operand: usize,
    pub name: String,
    pub append: bool,
    pub items: crate::session::array::ArrayItems,
}

#[derive(Default)]
pub(crate) struct PreparedCommand {
    pub(super) missing_hook: bool,
    pub(super) deferred_redirects: Vec<Redirect<Word>>,
    pub(super) resolved: Option<PathBuf>,
    pub(super) array_declarations: Vec<ArrayDeclaration>,
    pub(super) compound: Option<std::rc::Rc<crate::parser::Command>>,
    pub(super) search: lookup::Search,
    pub(super) default_path: bool,
    pub(super) words: Vec<String>,
    pub(super) redirects: Vec<Redirect>,
    pub(super) assignments: Vec<(String, String)>,
    pub(super) empty_status: u8,
    pub(super) environment: external::ChildEnvironment,
    pub(super) literal_has_keywords: bool,
    pub(super) literal_has_braces: bool,
}
impl PreparedCommand {
    pub(crate) fn literal_builtin(
        statement: &crate::parser::Statement,
        session: &mut Session,
    ) -> Option<Box<Self>> {
        let cmd = statement.single_stage()?;
        if cmd.compound.is_some() || !cmd.assignments.is_empty() {
            return None;
        }
        let name = cmd.words.first()?.as_literal()?;
        let command_query = cmd
            .words
            .get(1)
            .and_then(Word::as_literal)
            .is_some_and(|flag| matches!(flag, "-v" | "-V"));
        let set_args = name == "set"
            && cmd.words.get(1).and_then(Word::as_literal) == Some("--")
            && cmd.redirects.is_empty();
        if name == "builtin"
            || name == "set" && !set_args
            || name == "command" && !command_query
            || !(builtin_names().contains(&name)
                || name.starts_with('/') && cmd.redirects.is_empty())
        {
            return None;
        }
        let mut words = Vec::with_capacity(cmd.words.len());
        let mut literal_has_braces = false;
        for word in &cmd.words {
            if let Some(text) = word.as_literal() {
                words.push(text.to_owned());
            } else if session.shopt.braceexpand
                && statement.location.text().len() <= 1024
                && let Word::Expanded(parts) = word
                && {
                    let room = 256usize.saturating_sub(words.len());
                    crate::word::brace::append_borrowed_product(parts, &mut words, room)
                }
            {
                literal_has_braces = true;
            } else {
                return None;
            }
        }
        let redirects = if cmd.redirects.is_empty() {
            Vec::new()
        } else {
            // A literal redirect can reuse its parsed path or fd, but its open,
            // duplicate, move or close must still happen on every iteration.
            if session.noclobber
                || cmd.redirects.iter().any(|r| {
                    r.variable.is_some()
                        || r.word.as_literal().is_none()
                        || matches!(r.target, Target::HereDoc { expand: true })
                })
            {
                return None;
            }
            let captured = std::cell::Cell::new(None);
            prepare_redirects(
                cmd.redirects.clone(),
                &mut Expansion::new(session, 0, &captured),
            )
            .ok()?
        };
        Some(Box::new(Self {
            literal_has_keywords: !cmd.keyword_args.is_empty(),
            literal_has_braces,
            words,
            redirects,
            ..Self::default()
        }))
    }

    pub(crate) fn run_literal(&mut self, session: &mut Session, status: u8) -> Option<Flow> {
        if self.literal_has_braces && !session.shopt.braceexpand
            || session.keyword && self.literal_has_keywords
            || session.xtrace
            || session.traps.observes_commands()
            || session.jobs.monitor
        {
            return None;
        }
        if session.noclobber && self.redirects.iter().any(|r| r.target == Target::Write) {
            return None;
        }
        let selected = builtin(self, status, session);
        if selected.is_none() && self.words[0].starts_with('/') && self.redirects.is_empty() {
            let scope = session.substitutions.scope();
            let result = start(self, None, None, None, status, session, Default::default())
                .and_then(|stage| match stage {
                    Stage::Child(pid) | Stage::GroupedChild(pid) => wait(pid),
                    Stage::Status(code) => Ok(code),
                });
            session.substitutions.finish(scope);
            session.remember_literal_argument(&self.words);
            return Some(Flow::Continue(result.unwrap_or_else(|e| {
                diagnostic!("{e}");
                1
            })));
        }
        let selected = selected?;
        if matches!(selected, Builtin::Set) {
            // set -- consumes its argv. Keep the loop template immutable and
            // reuse the storage of the live positional arguments on replacement.
            let args = &self.words[2..];
            if session.positional.len() == args.len() {
                session.positional.as_mut_slice().clone_from_slice(args);
            } else {
                let mut values = std::mem::take(&mut session.positional).collect();
                args.clone_into(&mut values);
                session.positional = values.into_iter();
            }
            session.source_set_args = true;
            session.remember_literal_argument(&self.words);
            return Some(Flow::Continue(0));
        }
        if matches!(selected, Builtin::Function) {
            return None;
        }
        let scope = session.substitutions.scope();
        let flow = run_builtin(selected, self, session, status, false).unwrap_or_else(|e| {
            diagnostic!("{e}");
            Flow::Continue(1)
        });
        session.substitutions.finish(scope);
        session.remember_literal_argument(&self.words);
        Some(flow)
    }

    pub(super) fn cacheable(&self) -> bool {
        !self.default_path && !self.assignments.iter().any(|(k, _)| k == "PATH")
    }
    // Bash invokes this function only for a missing PATH command. A present
    // but non-executable file and an explicit pathname keep their exec errors.
    pub(super) const MISSING_HOOK: &'static str = "command_not_found_handle";

    pub(super) fn select_missing_hook(&mut self) {
        self.words.insert(0, Self::MISSING_HOOK.to_owned());
        self.search = lookup::Search::Normal;
        self.missing_hook = true;
    }

    pub(super) fn prepare_missing_hook(&mut self, session: &Session) -> bool {
        let Some(name) = self.words.first() else {
            return false;
        };
        if !session.functions.contains_key(Self::MISSING_HOOK)
            || name.contains('/')
            || self.cached_path(session, name).is_some()
        {
            return false;
        }
        match hash::search_in(name, session, lookup::path(self, session)) {
            Err(hash::Absent) => {
                self.select_missing_hook();
                true
            }
            Ok(path) => {
                if self.cacheable() {
                    self.resolved = path.map(|entry| entry.path);
                }
                false
            }
        }
    }

    pub(super) fn cached_path<'a>(&'a self, session: &'a Session, name: &str) -> Option<&'a Path> {
        if name.contains('/') {
            return None;
        }
        self.resolved.as_deref().or_else(|| {
            if !self.cacheable() {
                return None;
            }
            session.hashes.get(name).filter(|path| {
                !(session.posix || session.shopt.checkhash) || lookup::executable(path)
            })
        })
    }
}

// A repeatedly executed command owns its final argv. Clone only words whose
// expansion needs mutable parts, retaining the ordinary splitting and globbing rules.
pub(super) fn borrowed_arguments(
    raw: &[Word],
    session: &mut Session,
    status: u8,
) -> Result<PreparedCommand, crate::expansion::Error> {
    let captured = Cell::new(None);
    let mut exp = Expansion::new(session, status, &captured);
    let mut words = Vec::with_capacity(raw.len());
    for word in raw {
        if let Some(s) = word.as_literal() {
            words.push(s.to_owned());
        } else if matches!(word, Word::Expanded(parts) if matches!(parts.as_slice(), [crate::word::Part::Parameter { quoted: true, .. }]))
            && let Some(s) = word.stored_parameter(exp.session)
        {
            words.push(s.to_owned());
        } else {
            if exp.session.shopt.braceexpand
                && let Word::Expanded(parts) = word
                && crate::word::brace::append_borrowed_product(parts, &mut words, usize::MAX)
            {
                continue;
            }
            exp.append_fields(word.clone(), &mut words)?;
        }
    }
    prepare_expanded(
        SimpleCommand::default(),
        words,
        session,
        status,
        CommandContext::Shell,
        &captured,
    )
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum CommandContext {
    Shell,
    BackgroundJobSpec,
    Child,
    Pipeline,
}

pub(super) fn prepare(
    mut raw: SimpleCommand<Word>,
    session: &mut Session,
    last_status: u8,
    context: CommandContext,
) -> Result<PreparedCommand, crate::expansion::Error> {
    let captured = Cell::new(None);
    let words = expand_command_words(&mut raw, session, last_status, &captured)?;
    prepare_expanded(raw, words, session, last_status, context, &captured)
}

// The option is evaluated at execution: a previously parsed function may
// be called both with and without -k. Only syntactic assignments qualify.
pub(super) fn separate_keyword_arguments(raw: &mut SimpleCommand<Word>) {
    let first_array = raw
        .assignments
        .iter()
        .position(|(_, a)| matches!(a, Assignment::Array(Some(_), ..)));
    if raw.keyword_args.is_empty() && first_array.is_none() {
        return;
    }
    let mut arrays = first_array
        .map(|i| raw.assignments.split_off(i))
        .unwrap_or_default()
        .into_iter()
        .peekable();
    let keywords = std::mem::take(&mut raw.keyword_args);
    let mut indices = 0..;
    raw.words.retain_mut(|word| {
        let index = indices.next().unwrap();
        if let Some(key) = keywords.key(index, word) {
            if key.contains('[') {
                diagnostic!("{key}: not a valid identifier");
                return false;
            }
            let key = key.to_owned();
            if word.remove_assignment_prefix(key.len() + 1).is_none() {
                return true;
            }
            let value = std::mem::take(word);
            raw.assignments.push((key, Assignment::Scalar(value)));
            false
        } else if let Some((key, Assignment::Array(_, parts, source))) =
            arrays.next_if(|(_, a)| matches!(a, Assignment::Array(Some(pos), ..) if *pos == index))
        {
            raw.assignments
                .push((key, Assignment::Array(None, parts, source)));
            false
        } else {
            true
        }
    });
}

pub(super) fn expand_command_words(
    raw: &mut SimpleCommand<Word>,
    session: &mut Session,
    last_status: u8,
    captured: &Cell<Option<u8>>,
) -> Result<Vec<String>, crate::expansion::Error> {
    let mut words = Vec::with_capacity(raw.words.len());
    if session.posix && !raw.keyword_args.is_empty() {
        for (index, word) in raw.words.iter_mut().enumerate() {
            if raw.keyword_args.key(index, word).is_some()
                && let Word::Expanded(parts) = word
            {
                for part in parts {
                    if let crate::word::Part::Tilde(tilde) = part {
                        *part = crate::word::Part::Text(std::mem::take(tilde));
                    }
                }
            }
        }
    }
    let mut original = Expansion::new(session, last_status, captured);
    let mut positions = raw
        .assignments
        .iter_mut()
        .filter_map(|(_, assignment)| match assignment {
            Assignment::Array(Some(position), _, _) => Some(position),
            _ => None,
        })
        .peekable();
    for (index, word) in std::mem::take(&mut raw.words).into_iter().enumerate() {
        if positions.peek().is_some_and(|p| **p == index) {
            *positions.next().unwrap() = words.len();
        }
        match word {
            Word::Literal(word) => words.push(word),
            word => original.append_fields(word, &mut words)?,
        }
    }
    Ok(words)
}

pub(super) fn prepare_expanded(
    raw: SimpleCommand<Word>,
    words: Vec<String>,
    session: &mut Session,
    last_status: u8,
    context: CommandContext,
    captured: &Cell<Option<u8>>,
) -> Result<PreparedCommand, crate::expansion::Error> {
    let standalone = context != CommandContext::Pipeline;
    // A lone assignment has no following word, redirect or trace that needs a copy.
    let direct_assignment = standalone
        && words.is_empty()
        && raw.assignments.len() == 1
        && raw.compound.is_none()
        && raw.redirects.is_empty()
        && !session.xtrace;
    let mut assignments = if direct_assignment {
        Vec::new()
    } else {
        Vec::with_capacity(raw.assignments.len())
    };
    let mut array_declarations = Vec::new();
    let mut traced_operands = Vec::new();
    // Each assignment sees its predecessors, while command arguments saw the
    // outer environment. Standalone assignments persist even on redirect failure.
    let typed_assignment = words.is_empty()
        || session.posix
            && words
                .first()
                .is_some_and(|word| super::special_builtin(word));
    for (mut key, word) in raw.assignments {
        let mut exp = Expansion::with_assignments(
            session,
            last_status,
            captured,
            &mut assignments,
            standalone && words.is_empty(),
        );
        let append = key.ends_with('+');
        if append {
            key.pop();
        }
        match word {
            Assignment::Array(operand, parts, source) => {
                if exp.session.xtrace {
                    if let Some(index) = operand {
                        traced_operands.push((index, source));
                    } else {
                        trace_line(exp.session, last_status, |out| out.push_str(source.text()));
                    }
                }
                if operand.is_none() && exp.session.readonly(&key) {
                    return Err(crate::expansion::Error::readonly(&key));
                }
                if operand.is_none() && !words.is_empty() {
                    let mut text = if append {
                        exp.lookup(&key).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    text.push('(');
                    for (i, (index, mut word)) in parts.indexed_words().enumerate() {
                        if i != 0 {
                            text.push(' ');
                        }
                        if let Some(index) = index {
                            text.push('[');
                            text.push_str(&exp.key(&index.index)?);
                            text.push_str(if index.append { "]+=" } else { "]=" });
                        }
                        // This command environment entry is a scalar. Preserve
                        // tildes, braces and globs while expanding substitutions.
                        if let Word::Expanded(parts) = &mut word {
                            for part in parts {
                                if let crate::word::Part::Tilde(s) = part {
                                    *part = crate::word::Part::Text(std::mem::take(s));
                                }
                            }
                        }
                        text.push_str(&exp.scalar(word)?);
                    }
                    text.push(')');
                    assignments.push((key, text));
                    continue;
                }
                let declared_assoc = operand.is_some()
                    && words
                        .iter()
                        .skip(1)
                        .take_while(|arg| crate::builtins::flags::cluster(arg))
                        .any(|arg| arg.contains('A'));
                let items = exp.array_items(
                    parts,
                    declared_assoc || exp.session.is_associative(&key),
                    operand.is_some(),
                )?;
                if words.is_empty() {
                    if standalone {
                        let error = items.initializer_error();
                        session
                            .assign_expanded_array(&key, append, items)
                            .map_err(crate::expansion::Error::discard_line)?;
                        if let Some(error) = error {
                            return Err(crate::expansion::Error::discard(error));
                        }
                    }
                } else if let Some(operand) = operand {
                    array_declarations.push(ArrayDeclaration {
                        operand,
                        name: key,
                        append,
                        items,
                    });
                }
            }
            Assignment::Scalar(mut word) => {
                if direct_assignment && key.ends_with(']') {
                    let suffix = if append {
                        word.as_literal()
                    } else {
                        word.literal_element_suffix(&key)
                    };
                    if suffix.is_some_and(|s| exp.session.append_literal_element(&key, s)) {
                        continue;
                    }
                }
                let append =
                    append || direct_assignment && word.take_append_suffix(&key, exp.session);
                let value = exp.scalar(word)?;
                if exp.session.xtrace {
                    trace_line(exp.session, last_status, |out| {
                        out.push_str(&key);
                        out.push_str(if append { "+=" } else { "=" });
                        out.push_str(&trace_word(&value));
                    });
                }
                let mut stored_index = false;
                match exp.session.reference_name(&key) {
                    Ok(std::borrow::Cow::Owned(resolved)) => {
                        stored_index = !key.contains('[');
                        key = resolved;
                    }
                    Ok(_) => {}
                    Err(error) if exp.session.function_depth == 0 => {
                        return Err(crate::expansion::Error::fatal(
                            format!("warning: {key}: {error}"),
                            1,
                        ));
                    }
                    Err(error) => {
                        diagnostic!("warning: {key}: {error}");
                        continue;
                    }
                }
                let target = crate::session::array::subscript(&key);
                let base = target.map_or(key.as_str(), |(base, _)| base);
                if exp.session.readonly(base) {
                    let error = crate::expansion::Error::readonly(base);
                    if words.is_empty() || exp.session.posix {
                        return Err(error);
                    }
                    diagnostic!("{error}");
                    continue;
                }
                if target.is_some_and(|(_, index)| index.is_empty()) {
                    return Err(crate::expansion::Error::discard(format!(
                        "{key}: bad array subscript"
                    )));
                }
                if target.is_some() && !words.is_empty() {
                    diagnostic!("{key}: not a valid identifier");
                    continue;
                }
                if let Some((base, subscript)) = target
                    && exp.session.is_associative(base)
                {
                    if standalone {
                        let element = if stored_index {
                            crate::arithmetic::data_key(subscript, &mut exp)?
                        } else {
                            exp.expanded_key(subscript)?
                        };
                        if element.is_empty() {
                            return Err(crate::expansion::Error::fatal(
                                format!("{key}: bad array subscript"),
                                1,
                            ));
                        }
                        let key = element;
                        if append
                            && exp.session.append_element_text(
                                base,
                                crate::session::array::Element::Key(&key),
                                &value,
                            )
                        {
                            continue;
                        }
                        let old = append
                            .then(|| exp.session.assoc_value(base, &key).map(str::to_owned))
                            .flatten();
                        let value = if typed_assignment && exp.session.integer(base) {
                            exp.session.integer_value(old.as_deref(), &value, append)?
                        } else if append {
                            old.unwrap_or_default() + &value
                        } else {
                            value
                        };
                        exp.session.assign_key(base, key, value)?;
                    }
                    continue;
                }
                let index = if let Some((_, subscript)) = target {
                    if !standalone {
                        continue;
                    }
                    let n = if let Some(n) = Session::plain_decimal(subscript) {
                        n
                    } else if stored_index {
                        crate::arithmetic::data_index(subscript, &mut exp)?
                    } else {
                        exp.arithmetic(subscript)?
                    };
                    Some(exp.session.relative_index(base, n).map_err(|_| {
                        crate::expansion::Error::discard(format!("{key}: bad array subscript"))
                    })?)
                } else {
                    None
                };
                let old = |exp: &Expansion<'_>| match index {
                    Some(i) => exp.session.element(base, i),
                    None => exp.lookup(&key),
                };
                let value = if typed_assignment && exp.session.integer(base) {
                    let old = if append {
                        let old = old(&exp).unwrap_or_default();
                        exp.arithmetic_value(&old)?
                    } else {
                        0
                    };
                    old.wrapping_add(exp.arithmetic_value(&value)?).to_string()
                } else if append {
                    if let Some(i) = index {
                        if exp.session.append_element_text(
                            base,
                            crate::session::array::Element::Indexed(i),
                            &value,
                        ) {
                            continue;
                        }
                    } else if direct_assignment && exp.session.append_plain_scalar(&key, &value) {
                        continue;
                    }
                    old(&exp).unwrap_or_default() + &value
                } else {
                    value
                };
                if let Some(i) = index {
                    session.assign_element(base, i, value);
                    continue;
                }
                if session.is_nameref(&key) && !Session::valid_reference_target(&value) {
                    return Err(crate::expansion::Error::discard("invalid name reference"));
                }
                if standalone && words.is_empty() {
                    // The assignment is already visible in the shell. Retaining
                    // it as a command prefix would incorrectly export it to
                    // substitutions in subsequent operands.
                    session.assign_value(&key, value.into());
                } else {
                    assignments.push((key, value));
                }
            }
        }
    }
    let assignment_only = words.is_empty() && raw.compound.is_none();
    let mut cmd = PreparedCommand {
        missing_hook: false,
        literal_has_keywords: false,
        literal_has_braces: false,
        deferred_redirects: Vec::new(),
        resolved: None,
        array_declarations,
        compound: raw.compound,
        search: lookup::Search::Normal,
        default_path: false,
        words,
        redirects: Vec::new(),
        assignments,
        empty_status: captured.get().unwrap_or(0),
        environment: if raw.direct_exec {
            external::ChildEnvironment::ReplaceShell
        } else {
            external::ChildEnvironment::Inherit
        },
    };
    if session.xtrace && !cmd.words.is_empty() {
        trace_line(session, last_status, |out| {
            let mut operands = traced_operands.iter().peekable();
            for (i, word) in cmd.words.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                if let Some((_, source)) = operands.next_if(|(index, _)| *index == i) {
                    out.push_str(source.text());
                } else {
                    out.push_str(&trace_word(word));
                }
            }
        });
    }
    lookup::command_prefix(&mut cmd, session);
    if matches!(
        context,
        CommandContext::Shell | CommandContext::BackgroundJobSpec
    ) && cmd.search != lookup::Search::BuiltinsOnly
        && cmd.words.first().is_some_and(|word| word.starts_with('%'))
    {
        let builtin = if context == CommandContext::BackgroundJobSpec {
            "bg"
        } else {
            "fg"
        };
        cmd.words.insert(0, builtin.into());
        cmd.search = lookup::Search::JobSpec;
        return Ok(cmd);
    }
    expand_prepared_redirects(
        &mut cmd,
        raw.redirects,
        session,
        last_status,
        captured,
        assignment_only,
        standalone,
    )?;
    if assignment_only {
        cmd.assignments.clear();
    }
    Ok(cmd)
}

fn expand_prepared_redirects(
    cmd: &mut PreparedCommand,
    redirects: Vec<Redirect<Word>>,
    session: &mut Session,
    last_status: u8,
    captured: &Cell<Option<u8>>,
    assignment_only: bool,
    standalone: bool,
) -> Result<(), crate::expansion::Error> {
    if super::deferred_redirects(&redirects, session) {
        cmd.deferred_redirects = redirects;
    } else if !redirects.is_empty() {
        // A standalone null command expands ordinary output redirects in the shell.
        // Bash isolates the whole redirect list if any redirect replaces stdin.
        let isolated_null = assignment_only && super::isolated_null_redirects(&redirects);
        let child = if assignment_only {
            !standalone || isolated_null
        } else {
            builtin(cmd, last_status, session).is_none()
        };
        let process_scope = session.substitutions.scope();
        let mut isolated = (child && redirects.iter().any(|r| redirection_observable(r, session)))
            .then(|| session.clone());
        let mut exp = Expansion::with_assignments(
            isolated.as_mut().unwrap_or(session),
            last_status,
            captured,
            if assignment_only {
                &mut cmd.assignments
            } else {
                &mut []
            },
            standalone && assignment_only,
        );
        let result = prepare_redirects(redirects, &mut exp);
        if let Some(isolated) = isolated {
            session
                .substitutions
                .adopt(isolated.substitutions, process_scope);
        }
        cmd.redirects = result.map_err(|mut e| {
            if child {
                e.child_failure();
            }
            e
        })?;
        cmd.empty_status = if isolated_null {
            0
        } else {
            captured.get().unwrap_or(0)
        };
    }
    Ok(())
}

pub(super) fn prepare_redirects(
    raw: impl IntoIterator<Item = Redirect<Word>>,
    exp: &mut Expansion<'_>,
) -> Result<Vec<Redirect>, crate::expansion::Error> {
    let raw = raw.into_iter();
    let mut redirects = Vec::with_capacity(raw.size_hint().0);
    for r in raw {
        let word = match (r.target, r.word) {
            (Target::HereDoc { expand: true }, Word::Literal(body)) => {
                if body.contains(['$', '`', '\\']) {
                    let mut expanded = String::with_capacity(body.len());
                    let plain = super::plain_here_parts(&body, |literal, name| {
                        expanded.push_str(literal);
                        if let Some(name) = name {
                            if super::stage_parameter(name, exp.session) {
                                return false;
                            }
                            if let Some(value) = exp.borrowed_scalar(name) {
                                expanded.push_str(value);
                            } else if let Some(value) = exp.lookup(name) {
                                expanded.push_str(&value);
                            } else if exp.session.nounset {
                                return false;
                            }
                        }
                        true
                    });
                    if plain {
                        expanded
                    } else {
                        exp.scalar(
                            crate::parser::here_word(exp.session, &body)
                                .map_err(|e| e.message())?,
                        )?
                    }
                } else {
                    body
                }
            }
            (Target::HereString, word) => {
                let mut text = exp.scalar(word)?;
                text.push('\n');
                text
            }
            (_, Word::Literal(word)) => word,
            (_, word) => exp.redirect_path(word)?,
        };
        let mut word = word;
        let both = r.target == Target::OutputWord
            && word != "-"
            && !word.bytes().all(|b| b.is_ascii_digit());
        let mut target = match r.target {
            Target::OutputWord if both => Target::Write,
            Target::MoveWord | Target::OutputWord => Target::Duplicate,
            target => target,
        };
        if r.target == Target::MoveWord && word.is_empty() {
            word.push('-');
        }
        let mut moved = None;
        if target == Target::Duplicate && word != "-" {
            let fd = word
                .parse::<RawFd>()
                .ok()
                .filter(|_| word.bytes().all(|b| b.is_ascii_digit()))
                .ok_or_else(|| {
                    let (label, reason) = if r.target == Target::MoveWord
                        || !word.is_empty() && !word.bytes().all(|b| b.is_ascii_digit())
                    {
                        (word.clone(), "ambiguous redirect")
                    } else if word.is_empty() {
                        ("\"\"".into(), "Bad file descriptor")
                    } else if r.target == Target::OutputWord {
                        (word.clone(), "Bad file descriptor")
                    } else {
                        (r.fd.to_string(), "Bad file descriptor")
                    };
                    crate::expansion::Error::from(reason).in_expression(&label)
                })?;
            if r.target == Target::MoveWord && (r.variable.is_some() || fd != r.fd) {
                moved = Some(fd);
            }
        }
        if target == Target::Write && exp.session.noclobber {
            target = Target::NoClobber;
        }
        redirects.push(Redirect {
            line: r.line,
            variable: r.variable,
            fd: r.fd,
            target,
            word,
        });
        if both {
            redirects.push(Redirect {
                line: r.line,
                variable: None,
                fd: 2,
                target: Target::Duplicate,
                word: "1".into(),
            });
        }
        if let Some(fd) = moved {
            redirects.push(Redirect {
                line: r.line,
                variable: None,
                fd,
                target: Target::CloseMoved,
                word: String::new(),
            });
        }
    }
    Ok(redirects)
}
