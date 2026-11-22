use std::io::{self, Write};
use std::rc::Rc;

use super::{Session, Variable, is_identifier, print_declaration};

#[derive(Default)]
struct Options {
    print: bool,
    array: bool,
    functions: bool,
    global: bool,
    integer: Option<bool>,
    exported: Option<bool>,
}

pub(super) fn run(session: &mut Session, command: &str, mut operands: &[String]) -> u8 {
    let local = command == "local";
    if local && session.locals.is_empty() {
        eprintln!("tonari: local: can only be used in a function");
        return 1;
    }
    let mut flags = Options::default();
    while let Some((option, rest)) = operands.split_first() {
        if option == "--" {
            operands = rest;
            break;
        }
        if option.len() < 2 || !option.starts_with(['-', '+']) {
            break;
        }
        let on = option.starts_with('-');
        for c in option[1..].chars() {
            match c {
                'a' if on => flags.array = true,
                'F' => flags.functions = true,
                'g' => flags.global = on,
                'i' => flags.integer = Some(on),
                'p' => flags.print = true,
                'x' => flags.exported = Some(on),
                _ => {
                    eprintln!("tonari: {command}: {option}: unsupported option");
                    return 2;
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
                if writeln!(out, "declare -f {name}").is_err() {
                    return 1;
                }
            }
        } else {
            for name in operands {
                if !session.functions.contains_key(name) {
                    status = 1;
                    continue;
                }
                let line = if flags.print { "declare -f " } else { "" };
                if writeln!(out, "{line}{name}").is_err() {
                    return 1;
                }
            }
        }
        return out.flush().map_or(1, |()| status);
    }
    if operands.is_empty() || flags.print {
        if operands.is_empty() && !local && !flags.print && flags.exported.is_none() {
            return session.set(&[]);
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
                if (flags.exported.is_none() || !operands.is_empty() || var.exported)
                    && print_declaration(&mut out, name, &var).is_err()
                {
                    return 1;
                }
            } else if !operands.is_empty() {
                eprintln!("tonari: {command}: {name}: not found");
                status = 1;
            }
        }
        return out.flush().map_or(1, |()| status);
    }
    let mut status = 0;
    for arg in operands {
        let (name, value) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(k, v)| (k, Some(v)));
        let append = value.is_some() && name.ends_with('+');
        let name = if append {
            &name[..name.len() - 1]
        } else {
            name
        };
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
            .unwrap_or(Variable {
                integer: false,
                array_assigned: false,
                elements: None,
                declared: true,
                value: None,
                exported: false,
            });
        var.declared = true;
        if !flags.global {
            if let Some(scope) = session.locals.last_mut() {
                if !scope.0.contains_key(name) {
                    if name == "OPTIND" {
                        scope.1 = Some(session.opt_offset);
                        session.opt_offset = 0;
                    }
                    scope
                        .0
                        .insert(name.into(), session.variables.get(name).cloned());
                    // A fresh function-local declaration inherits only the
                    // export attribute of the shadowed variable.
                    var.integer = false;
                    var.value = None;
                    var.elements = None;
                    var.array_assigned = false;
                }
            }
        }
        if let Some(integer) = flags.integer {
            var.integer = integer;
        }
        if flags.array {
            var.make_array();
        }
        if let Some(text) = value {
            let number;
            let (text, append) = if var.integer {
                let old = var
                    .elements
                    .as_ref()
                    .and_then(|a| a.get(&0))
                    .or(var.value.as_ref());
                match session.integer_value(old.map(String::as_str), text, append) {
                    Ok(v) => {
                        number = v;
                        (number.as_str(), false)
                    }
                    Err(e) => {
                        eprintln!("tonari: {command}: {e}");
                        status = 1;
                        continue;
                    }
                }
            } else {
                (text, append)
            };
            if name == "OPTIND" {
                session.opt_offset = 0;
            }
            if let Some(array) = var.elements.as_mut().map(Rc::make_mut) {
                var.array_assigned = true;
                if append {
                    array.append_first(text)
                } else {
                    array.insert(0, text.into());
                }
            } else if append {
                var.value.get_or_insert_default().push_str(text);
            } else {
                var.value = Some(text.into());
            }
        }
        if let Some(on) = flags.exported {
            var.exported = on;
        }
        if flags.global {
            if let Some(saved) = session
                .locals
                .iter_mut()
                .find_map(|scope| scope.0.get_mut(name))
            {
                *saved = Some(var);
                continue;
            }
        }
        session.variables.insert(name.into(), var);
    }
    status
}

impl Session {
    pub fn declare(&mut self, name: &str, args: &[String]) -> u8 {
        run(self, name, args)
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
        entry.cloned().or_else(|| {
            let value = self.inherited_value(name);
            (value.is_some() || name == "OLDPWD").then(|| Variable {
                integer: false,
                array_assigned: false,
                elements: None,
                declared: true,
                value,
                exported: matches!(name, "PWD" | "OLDPWD") || std::env::var_os(name).is_some(),
            })
        })
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
            assert_eq!(s.declare("local", &[format!("x={inner}")]), 0);
        }
        assert_eq!(s.declare("declare", &["-g".into(), "x+=d".into()]), 0);
        assert_eq!(s.get("x").as_deref(), Some("c"));
        s.leave_function();
        assert_eq!(s.get("x").as_deref(), Some("b"));
        s.leave_function();
        assert_eq!(s.get("x").as_deref(), Some("ad"));
    }
}
