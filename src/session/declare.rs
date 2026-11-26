use std::io::{self, Write};
use std::rc::Rc;

use super::{LetterCase, Session, Variable, is_identifier, print_declaration};

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

pub(super) fn run(
    session: &mut Session,
    command: &str,
    mut operands: &[String],
    arrays: &[crate::execute::ArrayDeclaration],
    previous_status: u8,
) -> crate::execute::Flow {
    let total_args = operands.len();
    let readonly = command == "readonly";
    let local = command == "local";
    if local && session.function_depth == 0 {
        eprintln!("tonari: local: can only be used in a function");
        return crate::execute::Flow::Continue(1);
    }
    let mut flags = Options {
        readonly: readonly.then_some(true),
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
        if readonly && !on {
            break;
        }
        for c in option[1..].chars() {
            if readonly && !matches!(c, 'a' | 'A' | 'f' | 'p') {
                eprintln!("tonari: {command}: {option}: unsupported option");
                return crate::execute::Flow::Continue(2);
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
                'n' => flags.nameref = Some(on),
                'I' => flags.inherit = on,
                'p' => flags.print = true,
                'x' => flags.exported = Some(on),
                _ => {
                    eprintln!("tonari: {command}: {option}: unsupported option");
                    return crate::execute::Flow::Continue(2);
                }
            }
        }
        operands = rest;
    }
    if flags.functions {
        let mut out = io::stdout().lock();
        let mut status = 0;
        if operands.is_empty() {
            for name in session.functions.keys() {
                if !flags.function_names && !readonly {
                    if session.print_function(&mut out, name).is_err() {
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
        } else {
            for name in operands {
                if !session.functions.contains_key(name) {
                    if readonly {
                        eprintln!("tonari: {command}: {name}: not a function");
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
                        eprintln!("tonari: {command}: {name}: readonly function");
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
                    if writeln!(out, "{name} {} {source}", function.definition.location.line)
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
                if !flags.function_names && session.print_function(&mut out, name).is_err() {
                    return crate::Flow::Continue(1);
                }
                if flags.function_names && writeln!(out, "{line}{name}").is_err() {
                    return crate::execute::Flow::Continue(1);
                }
            }
        }
        return crate::execute::Flow::Continue(out.flush().map_or(1, |()| status));
    }
    if operands.is_empty() || flags.print && !readonly {
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
        let names;
        let names = if operands.is_empty() {
            names = if local {
                session.locals.last().unwrap().0.keys().cloned().collect()
            } else {
                session.variable_names("")
            };
            &names[..]
        } else {
            operands
        };
        let mut out = io::stdout().lock();
        let mut status = 0;
        if local
            && operands.is_empty()
            && session.locals.last().is_some_and(|s| s.3.is_some())
            && writeln!(out, "local -").is_err()
        {
            return crate::Flow::Continue(1);
        }
        for name in names {
            let listed = !local || session.locals.last().unwrap().0.contains_key(name);
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
                    && print_declaration(&mut out, name, &var).is_err()
                {
                    return crate::execute::Flow::Continue(1);
                }
            } else if !operands.is_empty() {
                eprintln!("tonari: {command}: {name}: not found");
                status = 1;
            }
        }
        return crate::execute::Flow::Continue(out.flush().map_or(1, |()| status));
    }
    let mut status = 0;
    let first_operand = total_args - operands.len() + 1;
    let mut arrays = arrays.iter().peekable();
    for (index, arg) in operands.iter().enumerate() {
        if local && arg == "-" {
            session.save_local_options();
            continue;
        }
        let array = arrays.next_if(|a| a.operand == index + first_operand);
        let subscript_end = arg
            .find(['[', '='])
            .filter(|&i| arg.as_bytes()[i] == b'[')
            .and_then(|i| crate::parser::index_end(&arg[i..]).ok().map(|n| i + n));
        let separator = arg[subscript_end.unwrap_or(0)..]
            .find('=')
            .map(|i| i + subscript_end.unwrap_or(0));
        let (name, value) = separator.map_or((arg.as_str(), None), |at| {
            (&arg[..at], Some(&arg[at + 1..]))
        });
        let append = value.is_some() && name.ends_with('+');
        let name = if append {
            &name[..name.len() - 1]
        } else {
            name
        };
        let base = super::array::subscript(name).map_or(name, |(base, _)| base);
        let creates_local = !readonly
            && !flags.global
            && session
                .declaration_scope(base)
                .is_some_and(|i| !session.locals[i].0.contains_key(base));
        let resolved;
        let name = if !creates_local && flags.nameref.is_none() && session.is_nameref(base) {
            match session.reference_name(name) {
                Ok(target) => {
                    resolved = target;
                    resolved.as_ref()
                }
                Err(e) => {
                    eprintln!("tonari: {command}: {name}: {e}");
                    status = 1;
                    continue;
                }
            }
        } else {
            name
        };
        let target = super::array::subscript(name);
        if readonly && target.is_some() {
            eprintln!("tonari: readonly: {name}: not a valid identifier");
            status = 1;
            continue;
        }
        let name = target.map_or(name, |(base, _)| base);
        if !is_identifier(name) {
            eprintln!("tonari: {command}: {arg}: invalid identifier");
            status = 1;
            continue;
        }
        if name == "PATH" {
            session.hashes.clear();
        }
        if session.protected_array(name) && (local || value.is_some() || array.is_some()) {
            eprintln!("tonari: {command}: {name}: variable may not be assigned a value");
            status = 1;
            continue;
        }
        let mut var = session
            .declared_variable(name, flags.global)
            .unwrap_or_default();
        let scope_index = session.declaration_scope(name);
        let fresh_local = !readonly
            && !flags.global
            && scope_index.is_some_and(|i| !session.locals[i].0.contains_key(name));
        let outer_local = fresh_local
            && session
                .locals
                .iter()
                .any(|scope| scope.0.contains_key(name));
        let shadow = outer_local && !flags.inherit && !session.shopt.localvar_inherit;
        if var.readonly
            && !shadow
            && (fresh_local && !outer_local
                || value.is_some()
                || array.is_some()
                || flags.readonly == Some(false))
        {
            eprintln!("tonari: {command}: {name}: readonly variable");
            if array.is_some() {
                return crate::execute::Flow::Discard(1);
            }
            status = 1;
            continue;
        }
        let new_global = !fresh_local && !var.declared && !session.variables.contains_key(name);
        var.declared = true;
        if fresh_local {
            let scope = &mut session.locals[scope_index.unwrap()];
            if name == "OPTIND" {
                scope.1 = Some(session.opt_offset);
                session.opt_offset = 0;
            }
            scope
                .0
                .insert(name.into(), session.variables.get(name).cloned());
            var.dynamic = false;
            var.nameref = false;
            if !var.temporary && !flags.inherit && !session.shopt.localvar_inherit {
                var.readonly = false;
                var.integer = false;
                var.letter_case = None;
                var.value = None;
                var.elements = None;
                var.array_assigned = false;
            }
        }
        if session.function_depth != 0 && !readonly && !flags.global {
            var.temporary = false;
        }
        // +n with an assignment writes through the existing reference and then
        // removes its attribute. An unsuccessful write leaves the reference live.
        let value = if flags.nameref == Some(false)
            && var.nameref
            && let Some(rhs) = value
        {
            let write = session
                .reference_name(name)
                .map(|s| s.into_owned())
                .and_then(|dest| {
                    let rhs = if append {
                        session.get(&dest).unwrap_or_default() + rhs
                    } else {
                        rhs.to_owned()
                    };
                    session.assign_target(&dest, &rhs)
                });
            if let Err(e) = write {
                eprintln!("tonari: {command}: {name}: {e}");
                status = 1;
                continue;
            }
            None
        } else {
            value
        };
        if let Some(on) = flags.nameref {
            let dest = value.or(var.value.as_deref()).unwrap_or("");
            if on
                && (target.is_some()
                    || var.elements.is_some()
                    || flags.array
                    || flags.associative
                    || array.is_some()
                    || !dest.is_empty() && !Session::valid_reference_target(dest)
                    || dest == name)
            {
                eprintln!("tonari: {command}: {name}: invalid name reference");
                status = 1;
                continue;
            }
            var.nameref = on;
            session.has_namerefs |= on;
        }
        let case_flags = [
            (LetterCase::Capitalize, flags.capitalize),
            (LetterCase::Lower, flags.lower),
            (LetterCase::Upper, flags.upper),
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
        if let Some(integer) = flags.integer {
            var.integer = integer;
        }
        if flags.associative
            && (!readonly || value.is_some() || array.is_some())
            && let Err(e) = var.make_associative()
        {
            eprintln!("tonari: {command}: {name}: {e}");
            status = 1;
            continue;
        }
        if flags.remove_array && var.elements.is_some() && var.associative().is_none()
            || flags.remove_associative && var.associative().is_some()
        {
            eprintln!("tonari: {command}: {name}: cannot remove array attribute");
            status = 1;
            continue;
        }
        if flags.array && var.associative().is_some() {
            eprintln!("tonari: {command}: {name}: cannot convert associative to indexed array");
            status = 1;
            continue;
        }
        if flags.array && (!readonly || value.is_some() || array.is_some()) {
            var.make_array();
        }
        if let Some(text) = value
            && target.is_none()
            && array.is_none()
            && var.elements.is_some()
            && text.starts_with('(')
            && text.ends_with(')')
        {
            let parts = match crate::parser::quoted_initializer(text) {
                Ok(parts) => parts,
                Err(e) => {
                    eprintln!("tonari: {command}: {name}: {}", e.message());
                    return crate::Flow::Discard(1);
                }
            };
            let captured = std::cell::Cell::new(None);
            let items = crate::expansion::Expansion::new(session, previous_status, &captured)
                .array_items(parts, var.associative().is_some());
            match items.and_then(|items| {
                Ok(session.array_items(name, flags.global, &mut var, append, &items)?)
            }) {
                Ok(()) => {}
                Err(e) => {
                    e.report(&format!("{command}: {name}: "));
                    return e.flow(session);
                }
            }
        } else if let Some(text) = value {
            let assoc_key = if var.associative().is_some() {
                match target
                    .map(|(_, expr)| session.subscript_key(expr))
                    .transpose()
                {
                    Ok(key) => Some(key.unwrap_or_else(|| "0".into())),
                    Err(e) => {
                        eprintln!("tonari: {command}: {e}");
                        status = 1;
                        continue;
                    }
                }
            } else {
                None
            };
            let index = if let Some((_, expr)) = target
                && assoc_key.is_none()
            {
                match session.declaration_index(name, expr, flags.global, &mut var) {
                    Ok(i) => {
                        var.make_array();
                        i
                    }
                    Err(e) => {
                        eprintln!("tonari: {command}: {e}");
                        status = 1;
                        continue;
                    }
                }
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
                match session.integer_value(old, text, append) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("tonari: {command}: {e}");
                        status = 1;
                        continue;
                    }
                }
            } else if append {
                old.unwrap_or_default().to_owned() + text
            } else {
                text.into()
            };
            let text = session.convert_case(text, var.letter_case);
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
                var.value = Some(text);
            }
        }
        if session.allexport
            && (value.is_some()
                || new_global && array.is_none() && !flags.array && !flags.associative)
        {
            var.exported = true;
        } else if let Some(on) = flags.exported {
            var.exported = on;
        }
        if let Some(d) = array
            && let Err(e) = session.array_items(name, flags.global, &mut var, d.append, &d.items)
        {
            eprintln!("tonari: {command}: {name}: {e}");
            status = 1;
            continue;
        }
        if let Some(on) = flags.readonly {
            var.readonly = on;
        }
        if name == "GLOBIGNORE" && value.is_some() {
            session.shopt.glob.set_ignore(var.value.as_deref());
        }
        if value.is_some()
            && matches!(name, "HISTSIZE" | "HISTFILESIZE")
            && let Some(value) = var.value.as_deref()
        {
            session.apply_history_limit(name, value);
        }
        if name == "BASH_COMPAT" && (!flags.global || session.declaration_scope(name).is_none()) {
            session.apply_compat(var.value.as_deref().unwrap_or(""));
        }
        if name == "POSIXLY_CORRECT" && (!flags.global || session.declaration_scope(name).is_none())
        {
            session.apply_posix_mode(var.value.is_some());
        }
        session.store_variable(name, var, flags.global);
    }
    crate::execute::Flow::Continue(status)
}

impl Session {
    pub fn declare(
        &mut self,
        name: &str,
        args: &[String],
        arrays: &[crate::execute::ArrayDeclaration],
        previous_status: u8,
    ) -> crate::execute::Flow {
        run(self, name, args, arrays, previous_status)
    }

    pub(super) fn store_variable(&mut self, name: &str, var: Variable, global: bool) {
        if global
            && let Some(saved) = self
                .locals
                .iter_mut()
                .find_map(|scope| scope.0.get_mut(name))
        {
            *saved = Some(var);
        } else {
            self.variables.insert(name.into(), var);
        }
    }

    pub(super) fn declared_variable(&self, name: &str, global: bool) -> Option<Variable> {
        let entry = if global {
            self.locals
                .iter()
                .find_map(|scope| scope.0.get(name))
                .map(Option::as_ref)
                .unwrap_or_else(|| self.variables.get(name))
        } else {
            self.variables.get(name)
        };
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
                var.value = self.inherited_value(name);
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
                letter_case: None,
                temporary: false,
                dynamic: super::dynamic::is_dynamic(name),
                inherited: Default::default(),
                readonly: Self::builtin_readonly(name),
                nameref: false,
                integer: super::dynamic::integer_attribute(name)
                    || matches!(name, "PPID" | "EUID" | "UID"),
                array_assigned: false,
                elements: None,
                declared: true,
                value,
                exported: matches!(name, "PWD" | "OLDPWD") || std::env::var_os(name).is_some(),
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
                s.declare("local", &[format!("x={inner}")], &[], 0),
                crate::Flow::Continue(0)
            );
        }
        assert_eq!(
            s.declare("declare", &["-g".into(), "x+=d".into()], &[], 0),
            crate::Flow::Continue(0)
        );
        assert_eq!(s.get("x").as_deref(), Some("c"));
        s.leave_function();
        assert_eq!(s.get("x").as_deref(), Some("b"));
        s.leave_function();
        assert_eq!(s.get("x").as_deref(), Some("ad"));
    }
}
