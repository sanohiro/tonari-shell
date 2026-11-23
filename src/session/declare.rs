use std::io::{self, Write};
use std::rc::Rc;

use super::{Session, Variable, is_identifier, print_declaration};

#[derive(Default)]
struct Options {
    print: bool,
    nameref: Option<bool>,
    array: bool,
    associative: bool,
    functions: bool,
    traced: Option<bool>,
    function_names: bool,
    readonly: Option<bool>,
    global: bool,
    integer: Option<bool>,
    inherit: bool,
    exported: Option<bool>,
}

pub(super) fn run(
    session: &mut Session,
    command: &str,
    mut operands: &[String],
    arrays: &[crate::execute::ArrayDeclaration],
) -> crate::execute::Flow {
    let total_args = operands.len();
    let readonly = command == "readonly";
    let local = command == "local";
    if local && session.locals.is_empty() {
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
                    if writeln!(out, "{name} {} {source}", function.definition.line).is_err() {
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
        {
            return crate::execute::Flow::Continue(session.set(&[]));
        }
        let names;
        let names = if operands.is_empty() {
            names = if local {
                session.locals.last().unwrap().0.keys().cloned().collect()
            } else {
                session.variable_names()
            };
            &names[..]
        } else {
            operands
        };
        let mut out = io::stdout().lock();
        let mut status = 0;
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
        let array = arrays.next_if(|a| a.operand == index + first_operand);
        let (name, value) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(k, v)| (k, Some(v)));
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
                .locals
                .last()
                .is_some_and(|scope| !scope.0.contains_key(base));
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
        let name = target.map_or(name, |(base, _)| base);
        if !is_identifier(name) {
            eprintln!("tonari: {command}: {arg}: invalid identifier");
            status = 1;
            continue;
        }
        if name == "PATH" {
            session.hashes.clear();
        }
        let mut var = session
            .declared_variable(name, flags.global)
            .unwrap_or_default();
        let fresh_local = !readonly
            && !flags.global
            && session
                .locals
                .last()
                .is_some_and(|scope| !scope.0.contains_key(name));
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
        var.declared = true;
        if fresh_local {
            let scope = session.locals.last_mut().unwrap();
            if name == "OPTIND" {
                scope.1 = Some(session.opt_offset);
                session.opt_offset = 0;
            }
            scope
                .0
                .insert(name.into(), session.variables.get(name).cloned());
            var.nameref = false;
            if !flags.inherit && !session.shopt.localvar_inherit {
                var.readonly = false;
                var.integer = false;
                var.value = None;
                var.elements = None;
                var.array_assigned = false;
            }
        }
        if let Some(on) = flags.nameref {
            let dest = value.or(var.value.as_deref()).unwrap_or("");
            if on
                && (target.is_some()
                    || var.elements.is_some()
                    || flags.array
                    || flags.associative
                    || array.is_some()
                    || !dest.is_empty()
                        && !(is_identifier(dest)
                            || super::array::subscript(dest).is_some_and(|(base, expr)| {
                                is_identifier(base) && !expr.is_empty()
                            }))
                    || dest == name)
            {
                eprintln!("tonari: {command}: {name}: invalid name reference");
                status = 1;
                continue;
            }
            var.nameref = on;
            session.has_namerefs |= on;
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
        if flags.array && var.associative().is_some() {
            eprintln!("tonari: {command}: {name}: cannot convert associative to indexed array");
            status = 1;
            continue;
        }
        if flags.array && (!readonly || value.is_some() || array.is_some()) {
            var.make_array();
        }
        if let Some(text) = value {
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
                match session.resolve_index(name, expr) {
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
                var.associative().and_then(|map| map.get(key))
            } else {
                var.elements
                    .as_ref()
                    .and_then(|a| a.get(&index))
                    .or(var.value.as_ref())
            };
            let text = if var.integer {
                match session.integer_value(old.map(String::as_str), text, append) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("tonari: {command}: {e}");
                        status = 1;
                        continue;
                    }
                }
            } else if append {
                old.cloned().unwrap_or_default() + text
            } else {
                text.into()
            };
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
                var.value = Some(text);
            }
        }
        if let Some(on) = flags.exported {
            var.exported = on;
        }
        if let Some(d) = array
            && let Err(e) = session.array_items(&mut var, d.append, &d.items)
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
    ) -> crate::execute::Flow {
        run(self, name, args, arrays)
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
        if let Some(var) = entry {
            let mut var = var.clone();
            if matches!(name, "BASHOPTS" | "SHELLOPTS") {
                var.value = self.inherited_value(name);
            }
            return Some(var);
        }
        {
            if name == "BASH_VERSINFO" {
                return Some(Variable {
                    declared: true,
                    readonly: true,
                    array_assigned: true,
                    elements: Some(std::rc::Rc::new(super::array::Elements::Dense(
                        self.array_values(name),
                    ))),
                    ..Variable::default()
                });
            }
            let value = self.inherited_value(name);
            (value.is_some() || name == "OLDPWD").then(|| Variable {
                readonly: Self::builtin_readonly(name),
                nameref: false,
                integer: matches!(name, "PPID" | "EUID" | "UID"),
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
            s.enter_function();
            assert_eq!(
                s.declare("local", &[format!("x={inner}")], &[]),
                crate::Flow::Continue(0)
            );
        }
        assert_eq!(
            s.declare("declare", &["-g".into(), "x+=d".into()], &[]),
            crate::Flow::Continue(0)
        );
        assert_eq!(s.get("x").as_deref(), Some("c"));
        s.leave_function();
        assert_eq!(s.get("x").as_deref(), Some("b"));
        s.leave_function();
        assert_eq!(s.get("x").as_deref(), Some("ad"));
    }
}
