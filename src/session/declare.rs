use std::io::Write;
use std::rc::Rc;

use super::{LetterCase, Session, Variable, is_identifier, print_attribute};

#[derive(Default)]
struct Options {
    print: bool,
    nameref: Option<bool>,
    array: bool,
    remove_array: bool,
    remove_associative: bool,
    associative: bool,
    functions: bool,
    traced: Option<bool>,
    function_names: bool,
    readonly: Option<bool>,
    global: bool,
    integer: Option<bool>,
    capitalize: Option<bool>,
    lower: Option<bool>,
    upper: Option<bool>,
    inherit: bool,
    exported: Option<bool>,
}

fn declare_functions(
    session: &mut Session,
    command: &str,
    operands: &[String],
    flags: &Options,
    readonly: bool,
) -> crate::Flow {
    if operands.is_empty() {
        let mut out = crate::builtins::listing_stdout(command);
        for name in session.functions.names("") {
            if !flags.function_names && !readonly {
                if session.print_function(&mut out, name, true).is_err() {
                    return crate::Flow::Continue(1);
                }
                continue;
            }
            if flags.readonly == Some(true) && !session.readonly_functions.contains(name) {
                continue;
            }
            let attr = session.function_attributes(name);
            if writeln!(out, "declare -{attr} {name}").is_err() {
                return crate::execute::Flow::Continue(1);
            }
        }
        return crate::Flow::Continue(u8::from(out.flush().is_err()));
    }
    let mut out = crate::builtins::stdout_for(command);
    let mut status = 0;
    for name in operands {
        if !session.functions.contains_key(name) {
            if readonly {
                diagnostic!("{command}: {name}: not a function");
            }
            status = 1;
            continue;
        }
        if let Some(on) = flags.traced {
            if on {
                session.traced_functions.insert(name.clone());
            } else {
                session.traced_functions.remove(name);
            }
            if flags.exported.is_none() && flags.readonly.is_none() {
                continue;
            }
        }
        if let Some(on) = flags.exported {
            if session.export_function(name, on).is_err() {
                status = 1;
            }
            continue;
        }
        if let Some(on) = flags.readonly {
            if on {
                session.readonly_functions.insert(name.clone());
            } else if session.readonly_functions.contains(name) {
                diagnostic!("{command}: {name}: readonly function");
                status = 1;
            }
            continue;
        }
        if flags.function_names && !flags.print && session.shopt.extdebug {
            let function = &session.functions[name];
            let source = if function.source.is_empty() {
                "environment"
            } else {
                &function.source
            };
            if writeln!(
                out,
                "{name} {} {source}",
                function.definition.declaration_line
            )
            .is_err()
            {
                return crate::Flow::Continue(1);
            }
            continue;
        }
        let line = if flags.print {
            format!("declare -{} ", session.function_attributes(name))
        } else {
            String::new()
        };
        if !flags.function_names && session.print_function(&mut out, name, flags.print).is_err() {
            return crate::Flow::Continue(1);
        }
        if flags.function_names && writeln!(out, "{line}{name}").is_err() {
            return crate::execute::Flow::Continue(1);
        }
    }
    crate::execute::Flow::Continue(out.flush().map_or(1, |()| status))
}

impl Options {
    fn changes_target(&self) -> bool {
        self.integer.is_some()
            || self.lower.is_some()
            || self.upper.is_some()
            || self.capitalize.is_some()
            || self.exported.is_some()
            || self.readonly.is_some()
            || self.traced.is_some()
            || self.array
            || self.associative
            || self.remove_array
            || self.remove_associative
    }

    fn apply_value_attributes(
        &self,
        var: &mut Variable,
        subscript: bool,
        may_create_array: bool,
    ) -> Result<(), &'static str> {
        let case_flags = [
            (LetterCase::Capitalize, self.capitalize),
            (LetterCase::Lower, self.lower),
            (LetterCase::Upper, self.upper),
        ];
        let mut enabled = case_flags
            .iter()
            .filter_map(|&(mode, on)| (on == Some(true)).then_some(mode));
        if let Some(mode) = enabled.next() {
            var.letter_case = enabled.next().is_none().then_some(mode);
        } else if case_flags
            .iter()
            .any(|&(mode, on)| on == Some(false) && var.letter_case == Some(mode))
        {
            var.letter_case = None;
        }
        if let Some(integer) = self.integer {
            var.integer = integer;
        }
        if self.associative && may_create_array {
            var.make_associative()?;
        }
        if self.remove_array && var.elements.is_some() && var.associative().is_none()
            || self.remove_associative && var.associative().is_some()
        {
            return Err("cannot remove array attribute");
        }
        if self.array && var.associative().is_some() {
            return Err("cannot convert associative to indexed array");
        }
        if (self.array || subscript && !self.associative && var.associative().is_none())
            && may_create_array
        {
            var.make_array();
        }
        Ok(())
    }

    fn parse<'a>(command: &str, mut operands: &'a [String]) -> Result<(Self, &'a [String]), ()> {
        let readonly = command == "readonly";
        let exporting = command == "export";
        let attribute = readonly || exporting;
        let mut flags = Options {
            readonly: readonly.then_some(true),
            exported: exporting.then_some(true),
            ..Options::default()
        };
        while let Some((option, rest)) = operands.split_first() {
            if option == "--" {
                operands = rest;
                break;
            }
            if option.len() < 2 || !option.starts_with(['-', '+']) {
                break;
            }
            let on = option.starts_with('-');
            if attribute && !on {
                break;
            }
            for c in option[1..].chars() {
                if attribute && !(matches!(c, 'a' | 'A' | 'f' | 'p') || exporting && c == 'n') {
                    diagnostic!("{command}: {option}: unsupported option");
                    return Err(());
                }
                match c {
                    'a' if on => flags.array = true,
                    'a' => flags.remove_array = true,
                    'A' if !on => flags.remove_associative = true,
                    'A' if on => flags.associative = true,
                    'F' => {
                        flags.functions = true;
                        flags.function_names = true;
                    }
                    't' => flags.traced = Some(on),
                    'f' => flags.functions = true,
                    'r' => flags.readonly = Some(on),
                    'g' => flags.global = on,
                    'i' => flags.integer = Some(on),
                    'c' => flags.capitalize = Some(on),
                    'l' => flags.lower = Some(on),
                    'u' => flags.upper = Some(on),
                    'n' if exporting => flags.exported = Some(false),
                    'n' => flags.nameref = Some(on),
                    'I' => flags.inherit = on,
                    'p' => flags.print = true,
                    'x' => flags.exported = Some(on),
                    _ => {
                        diagnostic!("{command}: {option}: unsupported option");
                        return Err(());
                    }
                }
            }
            operands = rest;
        }
        Ok((flags, operands))
    }
}

fn print_variables(
    session: &mut Session,
    command: &str,
    operands: &[String],
    flags: &Options,
) -> crate::Flow {
    let local = command == "local";
    let attribute = matches!(command, "readonly" | "export");
    if operands.is_empty()
        && !local
        && !flags.print
        && flags.exported.is_none()
        && flags.readonly.is_none()
        && !flags.array
        && !flags.associative
        && flags.nameref.is_none()
        && flags.integer.is_none()
        && flags.capitalize.is_none()
        && flags.lower.is_none()
        && flags.upper.is_none()
    {
        return crate::execute::Flow::Continue(session.set(&mut Vec::new()));
    }
    let local_scope = local.then(|| {
        session
            .locals
            .iter()
            .rev()
            .find(|scope| !scope.temporary)
            .expect("function scope")
    });
    let names;
    let names = if operands.is_empty() {
        names = if let Some(scope) = local_scope {
            let mut names: Vec<_> = scope.variables.keys().cloned().collect();
            names.sort_unstable();
            names
        } else {
            session.variable_names("")
        };
        &names[..]
    } else {
        operands
    };
    if let [name] = operands
        && name != "-"
        && local_scope.is_none_or(|s| s.variables.contains_key(name))
        && !matches!(name.as_str(), "BASHOPTS" | "SHELLOPTS")
        && let Some(var) = session.scoped_variable(name, flags.global)
        && var.declared
        && !var.dynamic
        && var.elements.is_none()
    {
        let mut bytes = Vec::with_capacity(64);
        let result = print_attribute(
            &mut bytes,
            name,
            var,
            (attribute && session.posix).then_some(command),
        );
        return crate::Flow::Continue(if result.is_err() {
            1
        } else {
            crate::builtins::write_stdout_for(command, &bytes)
        });
    }
    let mut out = crate::builtins::listing_stdout(command);
    let mut status = 0;
    if local
        && operands.is_empty()
        && local_scope.is_some_and(|s| s.set_options.is_some())
        && writeln!(out, "local -").is_err()
    {
        return crate::Flow::Continue(1);
    }
    for name in names {
        if name == "-"
            && session
                .locals
                .iter()
                .rev()
                .filter(|scope| !scope.temporary)
                .take(if local { 1 } else { usize::MAX })
                .any(|scope| scope.set_options.is_some())
        {
            if writeln!(out, "{}", if local { "local -" } else { "declare -- -" }).is_err() {
                return crate::Flow::Continue(1);
            }
            continue;
        }
        let listed = local_scope.is_none_or(|scope| scope.variables.contains_key(name));
        let variable = listed
            .then(|| session.declared_variable(name, flags.global))
            .flatten();
        if let Some(var) = variable.filter(|v| v.declared) {
            if operands.is_empty()
                && (flags.readonly == Some(true) && !var.readonly
                    || flags.array && (var.elements.is_none() || var.associative().is_some())
                    || flags.associative && var.associative().is_none()
                    || flags.capitalize == Some(true)
                        && var.letter_case != Some(LetterCase::Capitalize)
                    || flags.lower == Some(true) && var.letter_case != Some(LetterCase::Lower)
                    || flags.upper == Some(true) && var.letter_case != Some(LetterCase::Upper)
                    || flags.integer == Some(true) && !var.integer
                    || flags.nameref == Some(true) && !var.nameref)
            {
                continue;
            }
            if (flags.exported.is_none() || !operands.is_empty() || var.exported)
                && print_attribute(
                    &mut out,
                    name,
                    &var,
                    (attribute && session.posix).then_some(command),
                )
                .is_err()
            {
                return crate::execute::Flow::Continue(1);
            }
        } else if !operands.is_empty() {
            if out.flush().is_err() {
                return crate::Flow::Continue(1);
            }
            diagnostic!("{command}: {name}: not found");
            status = 1;
        }
    }
    crate::Flow::Continue(out.flush().map_or(1, |()| status))
}

fn declaration_operand(arg: &str) -> (&str, Option<&str>, bool) {
    let separator = arg.find(['[', '=']).and_then(|i| {
        if arg.as_bytes()[i] == b'=' {
            return Some(i);
        }
        let end = crate::parser::index_end(&arg[i..]).ok()? + i;
        arg[end..].find('=').map(|at| at + end)
    });
    let (name, value) = separator.map_or((arg, None), |at| (&arg[..at], Some(&arg[at + 1..])));
    let append = value.is_some() && name.ends_with('+');
    let name = if append {
        &name[..name.len() - 1]
    } else {
        name
    };
    (name, value, append)
}

#[inline(never)]
// Assign one expanded word. Compound initializers have separate partial-commit rules.
fn assign_value(
    session: &mut Session,
    var: &mut Variable,
    name: &str,
    subscript: Option<&str>,
    text: &str,
    append: bool,
    global: bool,
) -> Result<(), String> {
    let assoc_key = if var.associative().is_some() {
        Some(match subscript {
            Some(expr) => session.subscript_key(expr)?,
            None => "0".into(),
        })
    } else {
        None
    };
    let index = if let Some(expr) = subscript
        && assoc_key.is_none()
    {
        let index = session.declaration_index(name, expr, global, var)?;
        var.make_array();
        index
    } else {
        0
    };
    let old = if let Some(key) = &assoc_key {
        var.associative()
            .and_then(|map| map.get(key))
            .map(String::as_str)
    } else {
        var.elements
            .as_ref()
            .and_then(|a| a.get(&index))
            .or(var.value.as_deref())
    };
    let text = if var.integer {
        session.integer_value(old, text, append)?
    } else if append {
        old.unwrap_or_default().to_owned() + text
    } else {
        text.into()
    };
    let text = session.convert_case(text, var.letter_case);
    if var.nameref && !Session::valid_reference_target(&text) {
        return Err(format!("{name}: invalid name reference"));
    }
    if name == "OPTIND" {
        session.opt_offset = 0;
    }
    if let Some(key) = assoc_key {
        var.make_associative().unwrap().insert(key, text);
        var.array_assigned = true;
    } else if let Some(array) = var.elements.as_mut().map(Rc::make_mut) {
        array.insert(index, text);
        var.array_assigned = true;
    } else {
        if var.dynamic {
            session.set_dynamic(name, &text);
        }
        *var.scalar_mut() = Some(text);
    }
    Ok(())
}

impl Session {
    pub fn declare(
        &mut self,
        command: &str,
        operands: &[String],
        arrays: &[crate::execute::ArrayDeclaration],
        previous_status: u8,
        bindings: &mut super::Bindings,
    ) -> crate::execute::Flow {
        let total_args = operands.len();
        let readonly = command == "readonly";
        let exporting = command == "export";
        let attribute = readonly || exporting;
        let local = command == "local";
        if local && self.function_depth == 0 {
            diagnostic!("local: can only be used in a function");
            return crate::execute::Flow::Continue(1);
        }
        let Ok((flags, operands)) = Options::parse(command, operands) else {
            return crate::Flow::Continue(2);
        };
        if flags.functions {
            return declare_functions(self, command, operands, &flags, readonly);
        }
        if operands.is_empty() || flags.print && !attribute {
            return print_variables(self, command, operands, &flags);
        }
        let mut status = 0;
        if local
            && operands.len() >= 8
            && let Some(scope) = self.locals.last_mut().filter(|s| !s.temporary)
        {
            scope
                .variables
                .reserve(operands.len().saturating_sub(scope.variables.len()));
        }
        let first_operand = total_args - operands.len() + 1;
        let mut arrays = arrays.iter().peekable();
        for (index, arg) in operands.iter().enumerate() {
            if local && arg == "-" {
                self.save_local_options();
                continue;
            }
            let array = arrays.next_if(|a| a.operand == index + first_operand);
            let (name, value, append) = declaration_operand(arg);
            let base = super::array::subscript(name).map_or(name, |(base, _)| base);
            if flags.global
                && let Some(previous) = bindings.variables.remove(base)
            {
                if let Some(var) = previous {
                    self.variables.insert(base.into(), var);
                } else {
                    self.variables.remove(base);
                }
            }
            let scope_for = |shell: &Session, name: &str| {
                let scope = shell.declaration_scope(name);
                let fresh = !attribute
                    && !flags.global
                    && scope.is_some_and(|i| !shell.locals[i].variables.contains_key(name));
                (scope, fresh)
            };
            let (base_scope, creates_local) = scope_for(self, base);
            let mut remove_reference = None;
            let resolved;
            let name = if !creates_local
                && (flags.nameref.is_none()
                    || flags.nameref == Some(false)
                        && (value.is_some() || array.is_some() || flags.changes_target()))
                && self.has_namerefs
            {
                match self.scoped_reference_name(name, flags.global) {
                    Ok(target) => {
                        if target != name && flags.nameref == Some(false) {
                            if self.variables.get(base).is_some_and(|v| v.readonly) {
                                diagnostic!("{command}: {base}: readonly variable");
                                status = 1;
                                continue;
                            }
                            remove_reference = Some(base);
                        }
                        resolved = target;
                        resolved.as_ref()
                    }
                    Err(e) => {
                        diagnostic!("{command}: {name}: {e}");
                        status = 1;
                        continue;
                    }
                }
            } else {
                name
            };
            let target = super::array::subscript(name);
            if attribute && target.is_some() {
                diagnostic!("{command}: {name}: not a valid identifier");
                status = 1;
                continue;
            }
            let name = target.map_or(name, |(base, _)| base);
            if !is_identifier(name) {
                diagnostic!("{command}: {arg}: invalid identifier");
                status = 1;
                continue;
            }
            if name == "PATH" {
                self.hashes.clear();
            }
            if self.protected_array(name) && (local || value.is_some() || array.is_some()) {
                diagnostic!("{command}: {name}: variable may not be assigned a value");
                status = 1;
                continue;
            }
            let (scope_index, fresh_local) = if name == base {
                (base_scope, creates_local)
            } else {
                scope_for(self, name)
            };
            // Lowercase names have no implicit shell value. Repeated function
            // calls can rule out an inherited binding without scanning environ.
            let fresh_unbound = fresh_local
                && name.as_bytes()[0].is_ascii_lowercase()
                && !self.variables.contains_key(name)
                && {
                    let inherited = self.inherited_variable_names();
                    inherited.last().is_none_or(|last| last.as_str() < name)
                        || inherited
                            .binary_search_by(|key| key.as_str().cmp(name))
                            .is_err()
                };
            if fresh_unbound
                && local
                && total_args == operands.len()
                && array.is_none()
                && target.is_none()
                && !append
                && Self::ordinary_assignment(name)
                && bindings.variables.is_empty()
            {
                // A new plain local has no attributes, inherited value or prior binding to restore.
                self.locals[scope_index.unwrap()]
                    .variables
                    .insert(name.to_owned(), None);
                self.store_variable(
                    name,
                    Variable::scalar(value.map(str::to_owned), self.allexport && value.is_some()),
                    false,
                );
                continue;
            }
            let mut var = if fresh_unbound {
                Variable::default()
            } else {
                self.declared_variable(name, flags.global)
                    .unwrap_or_default()
            };
            let outer_local = fresh_local
                && self.locals[..scope_index.unwrap()]
                    .iter()
                    .any(|scope| scope.variables.contains_key(name));
            let shadow = outer_local && !flags.inherit && !self.shopt.localvar_inherit;
            if var.readonly
                && !shadow
                && (fresh_local && !outer_local
                    || value.is_some()
                    || array.is_some()
                    || flags.array && (var.elements.is_none() || var.associative().is_some())
                    || flags.associative && var.associative().is_none()
                    || flags.readonly == Some(false)
                    || flags.nameref.is_some())
            {
                diagnostic!("{command}: {name}: readonly variable");
                if array.is_some() {
                    return crate::execute::Flow::Discard(1);
                }
                status = 1;
                continue;
            }
            let new_global = !fresh_local && !var.declared && !self.variables.contains_key(name);
            var.declared = true;
            let mut previous_local = fresh_local.then(|| {
                let previous = bindings.variables.get(name).cloned().unwrap_or_else(|| {
                    if fresh_unbound {
                        None
                    } else {
                        self.variables.get(name).cloned()
                    }
                });
                (scope_index.unwrap(), previous, self.opt_offset)
            });
            let mut record_scope = |s: &mut Session| {
                if let Some((i, previous, offset)) = previous_local.take() {
                    let scope = &mut s.locals[i];
                    if name == "OPTIND" {
                        scope.opt_offset = Some(offset);
                        s.opt_offset = 0;
                    }
                    scope.variables.insert(name.into(), previous);
                }
            };
            if fresh_local {
                var.dynamic = false;
                var.nameref = false;
                if !var.temporary && !flags.inherit && !self.shopt.localvar_inherit {
                    var.readonly = false;
                    var.integer = false;
                    var.letter_case = None;
                    *var.scalar_mut() = None;
                    var.elements = None;
                    var.array_assigned = false;
                }
            }
            if self.function_depth != 0 && !attribute && !flags.global {
                var.temporary = false;
            }
            let making_array = flags.array || flags.associative || array.is_some();
            if making_array
                && (var.nameref || flags.nameref == Some(true))
                && var.elements.is_none()
            {
                if var.nameref && flags.nameref != Some(true) {
                    diagnostic!("warning: {name}: removing nameref attribute");
                }
                var.nameref = false;
            }
            if let Some(on) = flags
                .nameref
                .filter(|_| !making_array && remove_reference.is_none())
            {
                let dest = value.or(var.value.as_deref()).unwrap_or("");
                if on
                    && (target.is_some()
                        || var.elements.is_some()
                        || value.or(var.value.as_deref()).is_some()
                            && !Session::valid_reference_target(dest)
                        || dest == name)
                {
                    diagnostic!("{command}: {name}: invalid name reference");
                    status = 1;
                    continue;
                }
                if on {
                    var.integer = false;
                    var.letter_case = None;
                }
                var.nameref = on;
                self.has_namerefs |= on;
            }
            if flags.nameref == Some(true)
                && making_array
                && (array.is_some() || var.elements.is_some())
            {
                diagnostic!("{command}: {name}: reference variable cannot be an array");
                status = 1;
            }
            if let Err(e) = flags.apply_value_attributes(
                &mut var,
                target.is_some(),
                !attribute || value.is_some() || array.is_some(),
            ) {
                diagnostic!("{command}: {name}: {e}");
                status = 1;
                continue;
            }
            let mut initializer_error = array.and_then(|d| d.items.initializer_error());
            if let Some(text) = value
                && (target.is_none() || flags.array || flags.associative)
                && array.is_none()
                && var.elements.is_some()
                && (!attribute || flags.array || flags.associative)
                && text.starts_with('(')
                && text.ends_with(')')
            {
                let parts = match crate::parser::quoted_initializer(text) {
                    Ok(parts) => parts,
                    Err(e) => {
                        diagnostic!("{command}: {name}: {}", e.message());
                        return crate::Flow::Discard(1);
                    }
                };
                let captured = std::cell::Cell::new(None);
                let items = crate::expansion::Expansion::new(self, previous_status, &captured)
                    .array_items(parts, var.associative().is_some());
                match items.and_then(|items| {
                    initializer_error = items.initializer_error();
                    Ok(self.array_items(name, flags.global, &mut var, append, &items)?)
                }) {
                    Ok(()) => {}
                    Err(e) => {
                        record_scope(self);
                        self.store_variable(name, var, flags.global);
                        e.report(&format!("{command}: {name}: "));
                        return e.flow(self);
                    }
                }
            } else if let Some(text) = value
                && let Err(e) = assign_value(
                    self,
                    &mut var,
                    name,
                    target.map(|(_, expr)| expr),
                    text,
                    append,
                    flags.global,
                )
            {
                diagnostic!("{command}: {e}");
                status = 1;
                continue;
            }
            if self.allexport
                && (value.is_some()
                    || new_global && array.is_none() && !flags.array && !flags.associative)
            {
                var.exported = true;
            } else if let Some(on) = flags.exported {
                var.exported = on;
            }
            if let Some(d) = array
                && let Err(e) = self.array_items(name, flags.global, &mut var, d.append, &d.items)
            {
                record_scope(self);
                self.store_variable(name, var, flags.global);
                diagnostic!("{command}: {name}: {e}");
                return crate::Flow::Discard(1);
            }
            if let Some(on) = flags.readonly {
                var.readonly = on;
            }
            if matches!(name, "IGNOREEOF" | "ignoreeof") && (!flags.global || scope_index.is_none())
            {
                self.set_eof_variable(var.value.as_deref());
            }
            if name == "GLOBIGNORE" && value.is_some() {
                self.shopt.glob.set_ignore(var.value.as_deref());
            }
            if value.is_some()
                && matches!(name, "HISTSIZE" | "HISTFILESIZE")
                && let Some(value) = var.value.as_deref()
            {
                self.apply_history_limit(name, value);
            }
            if name == "BASH_COMPAT" && (!flags.global || self.declaration_scope(name).is_none()) {
                self.apply_compat(var.value.as_deref().unwrap_or(""));
            }
            if name == "POSIXLY_CORRECT"
                && (!flags.global || self.declaration_scope(name).is_none())
            {
                self.apply_posix_mode(var.value.is_some());
            }
            if flags.readonly == Some(true)
                || flags.exported == Some(true)
                || (!attribute && !flags.global && scope_index.is_some())
            {
                bindings.variables.remove(name);
            }
            record_scope(self);
            self.store_variable(name, var, flags.global);
            if let Some(source) = remove_reference
                && let Some(reference) = self.variables.get_mut(source)
            {
                reference.nameref = false;
            }
            if let Some(error) = initializer_error {
                let error = crate::expansion::Error::discard(error);
                error.report(&format!("{command}: {name}: "));
                return error.flow(self);
            }
        }
        crate::execute::Flow::Continue(status)
    }

    pub(super) fn store_variable(&mut self, name: &str, var: Variable, global: bool) {
        if global
            && let Some(saved) = self
                .locals
                .iter_mut()
                .find_map(|scope| scope.variables.get_mut(name))
        {
            *saved = Some(var);
        } else {
            self.variables.insert(name.into(), var);
        }
    }

    pub(super) fn scoped_variable(&self, name: &str, global: bool) -> Option<&Variable> {
        if global {
            self.locals
                .iter()
                .find_map(|scope| scope.variables.get(name))
                .map(Option::as_ref)
                .unwrap_or_else(|| self.variables.get(name))
        } else {
            self.variables.get(name)
        }
    }

    pub(super) fn declared_variable(&self, name: &str, global: bool) -> Option<Variable> {
        let entry = self.scoped_variable(name, global);
        if matches!(name, "BASH_ALIASES" | "BASH_CMDS") && entry.is_none_or(|v| v.dynamic) {
            let mut var = entry.cloned().unwrap_or_default();
            var.declared = true;
            var.dynamic = true;
            var.array_assigned = true;
            var.elements = Some(std::rc::Rc::new(super::array::Elements::Associative(
                self.table_snapshot(name),
            )));
            return Some(var);
        }
        if let Some(var) = entry {
            let mut var = var.clone();
            if var.dynamic || matches!(name, "BASHOPTS" | "SHELLOPTS") {
                *var.scalar_mut() = self.inherited_value(name);
            }
            return Some(var);
        }
        {
            if self.array_parameter(name) {
                return Some(Variable {
                    declared: true,
                    readonly: name == "BASH_VERSINFO",
                    array_assigned: name != "FUNCNAME" || self.function_depth != 0,
                    elements: Some(std::rc::Rc::new(super::array::Elements::Dense(
                        self.array_values(name),
                    ))),
                    ..Variable::default()
                });
            }
            let value = self.inherited_value(name);
            (value.is_some() || name == "OLDPWD").then(|| Variable {
                dynamic: super::dynamic::is_dynamic(name),
                readonly: Self::builtin_readonly(name),
                integer: super::dynamic::integer_attribute(name)
                    || matches!(name, "PPID" | "EUID" | "UID"),
                declared: true,
                value,
                exported: matches!(name, "PWD" | "OLDPWD") || std::env::var_os(name).is_some(),
                ..Variable::default()
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_declaration_bypasses_all_active_function_locals() {
        let mut s = Session::default();
        s.assign("x", "a");
        for inner in ["b", "c"] {
            s.enter_function(&mut Default::default());
            assert_eq!(
                s.declare(
                    "local",
                    &[format!("x={inner}")],
                    &[],
                    0,
                    &mut Default::default()
                ),
                crate::Flow::Continue(0)
            );
        }
        assert_eq!(
            s.declare(
                "declare",
                &["-g".into(), "x+=d".into()],
                &[],
                0,
                &mut Default::default()
            ),
            crate::Flow::Continue(0)
        );
        assert_eq!(s.get("x").as_deref(), Some("c"));
        s.leave_function();
        assert_eq!(s.get("x").as_deref(), Some("b"));
        s.leave_function();
        assert_eq!(s.get("x").as_deref(), Some("ad"));
    }
}
